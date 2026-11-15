//! Network drives for every user, mounted at boot (docs/PLANO-SETTINGS.md
//! §3.1, "Unidades de rede"): an SMB share or an NFS export as a systemd
//! mount and automount of its own under /mnt, written by
//! orbit-services-helper. The automount mounts it the first time someone
//! opens it, so a boot without the network does not wait for it.
//!
//! The units carry their address in a comment (`# orbit-system-drive
//! ADDRESS`), which is how the drives are found again without root. An SMB
//! share's user and password are kept in a file only root reads,
//! /etc/orbit/drives/NAME.cred.

use std::path::{Path, PathBuf};

/// Where the units are written.
pub const UNIT_DIR: &str = "/etc/systemd/system";
/// Where an SMB share's user and password are kept.
pub const CRED_DIR: &str = "/etc/orbit/drives";
/// The comment that marks a unit as one of these drives.
const MARK: &str = "# orbit-system-drive ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Smb,
    Nfs,
}

/// A drive for every user: `smb://[user@]server/share[/folder]` or
/// `nfs://server/path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemDrive {
    pub kind: Kind,
    pub server: String,
    /// The share and folder (SMB) or the exported path (NFS), without the
    /// leading slash.
    pub path: String,
    pub user: Option<String>,
}

impl SystemDrive {
    /// The drive from its address, refused unless every part is plain:
    /// a host name or address, a path without `..`, quotes, backslashes
    /// or control characters, a user of letters, digits and `._-`.
    pub fn parse(address: &str) -> Result<Self, String> {
        let bad = || format!("not an address of a drive for every user: {address}");
        let (kind, rest) = if let Some(r) = address.strip_prefix("smb://") {
            (Kind::Smb, r)
        } else if let Some(r) = address.strip_prefix("nfs://") {
            (Kind::Nfs, r)
        } else {
            return Err(bad());
        };
        if address.len() > 200 {
            return Err(bad());
        }
        let (host, path) = rest.split_once('/').ok_or_else(bad)?;
        let (user, server) = match host.split_once('@') {
            Some((u, s)) if kind == Kind::Smb => (Some(u.to_owned()), s),
            Some(_) => return Err(bad()),
            None => (None, host),
        };
        let plain_host = !server.is_empty() && server.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c));
        let plain_user = user
            .as_deref()
            .is_none_or(|u| !u.is_empty() && u.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)));
        let path = path.trim_matches('/');
        let plain_path = !path.is_empty()
            && !path.split('/').any(|p| p.is_empty() || p == "." || p == "..")
            && !path.chars().any(|c| c.is_control() || "\"'\\`$;".contains(c));
        if !(plain_host && plain_user && plain_path) {
            return Err(bad());
        }
        Ok(SystemDrive { kind, server: server.to_owned(), path: path.to_owned(), user })
    }

    pub fn address(&self) -> String {
        let scheme = if self.kind == Kind::Smb { "smb" } else { "nfs" };
        let user = self.user.as_ref().map(|u| format!("{u}@")).unwrap_or_default();
        format!("{scheme}://{user}{}/{}", self.server, self.path)
    }

    /// Its name: the server and the path, every other character `_` (so
    /// systemd's name for the mount point needs no escapes).
    pub fn name(&self) -> String {
        format!("{}_{}", self.server, self.path)
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect()
    }

    pub fn mount_point(&self) -> PathBuf {
        Path::new("/mnt").join(self.name())
    }

    /// The units' file name without the suffix (`mnt-NAME`).
    pub fn unit(&self) -> String {
        format!("mnt-{}", self.name())
    }

    pub fn credentials(&self) -> PathBuf {
        Path::new(CRED_DIR).join(format!("{}.cred", self.name()))
    }

    /// The mount unit's text.
    pub fn mount_unit(&self) -> String {
        let (what, kind, options) = match self.kind {
            Kind::Smb => {
                let login = if self.user.is_some() {
                    format!("credentials={}", self.credentials().display())
                } else {
                    "guest".to_owned()
                };
                (
                    format!("//{}/{}", self.server, self.path),
                    "cifs",
                    format!("_netdev,{login},iocharset=utf8,file_mode=0666,dir_mode=0777"),
                )
            }
            Kind::Nfs => (format!("{}:/{}", self.server, self.path), "nfs", "_netdev".to_owned()),
        };
        format!(
            "{MARK}{address}\n[Unit]\nDescription=Network drive {address}\nWants=network-online.target\nAfter=network-online.target\n\n[Mount]\nWhat={what}\nWhere={where_}\nType={kind}\nOptions={options}\nTimeoutSec=30\n",
            address = self.address(),
            where_ = self.mount_point().display(),
        )
    }

    /// The automount unit's text: enabled, it is what boot starts.
    pub fn automount_unit(&self) -> String {
        format!(
            "{MARK}{address}\n[Unit]\nDescription=Network drive {address}\n\n[Automount]\nWhere={where_}\nTimeoutIdleSec=600\n\n[Install]\nWantedBy=remote-fs.target\n",
            address = self.address(),
            where_ = self.mount_point().display(),
        )
    }
}

/// The credentials file of an SMB share.
pub fn credentials_text(user: &str, password: &str) -> String {
    format!("username={user}\npassword={password}\n")
}

/// The drives for every user written in `dir` (their automount units).
pub fn list(dir: &Path) -> Vec<SystemDrive> {
    let mut out: Vec<SystemDrive> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".automount"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| t.lines().next().and_then(|l| l.strip_prefix(MARK)).and_then(|a| SystemDrive::parse(a.trim()).ok()))
        .collect();
    out.sort_by_key(SystemDrive::address);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        let d = SystemDrive::parse("smb://ana@nas.home.lan/media/Músicas").unwrap();
        assert_eq!(d.kind, Kind::Smb);
        assert_eq!(d.user.as_deref(), Some("ana"));
        assert_eq!(d.path, "media/Músicas");
        assert_eq!(d.name(), "nas_home_lan_media_M_sicas");
        assert_eq!(d.unit(), "mnt-nas_home_lan_media_M_sicas");
        assert_eq!(d.address(), "smb://ana@nas.home.lan/media/Músicas");
        let n = SystemDrive::parse("nfs://10.0.0.250/datastore/jas/").unwrap();
        assert_eq!(n.path, "datastore/jas");
        assert!(n.mount_unit().contains("What=10.0.0.250:/datastore/jas\n"));
        assert!(n.mount_unit().contains("Type=nfs\n"));
        assert!(d.mount_unit().contains("credentials=/etc/orbit/drives/nas_home_lan_media_M_sicas.cred"));
        assert!(SystemDrive::parse("smb://nas/share").unwrap().mount_unit().contains(",guest,"));
        for bad in [
            "sftp://nas/x",
            "smb://nas/",
            "smb://nas/../etc",
            "smb://na s/x",
            "nfs://ana@nas/x",
            "smb://a;b@nas/x",
            "smb://nas/x\ny",
            "smb://nas/x'y",
        ] {
            assert!(SystemDrive::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn listed_from_units() {
        let dir = std::env::temp_dir().join(format!("orbit-drives-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let d = SystemDrive::parse("nfs://nas/export").unwrap();
        std::fs::write(dir.join(format!("{}.automount", d.unit())), d.automount_unit()).unwrap();
        std::fs::write(dir.join("other.automount"), "[Automount]\nWhere=/x\n").unwrap();
        assert_eq!(list(&dir), vec![d]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
