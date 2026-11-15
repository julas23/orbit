//! This machine's updates, as apt sees them (Orbit-Settings' Updates
//! page, docs/PLANO-SETTINGS.md §3.4): what an upgrade would bring, read
//! without root (apt's simulation); whether unattended-upgrades installs
//! them alone, and which; the repositories; what apt did lately. Fetching
//! the lists, upgrading and choosing the automatic mode are
//! [`crate::Privileged`] steps of orbit-services-helper.

use std::process::Command;

/// The file Orbit writes the automatic mode to; the other is the one
/// `dpkg-reconfigure unattended-upgrades` writes.
pub const PERIODIC: &str = "/etc/apt/apt.conf.d/20auto-upgrades";
pub const ORIGINS: &str = "/etc/apt/apt.conf.d/52orbit-unattended-upgrades";

/// What unattended-upgrades does: nothing, the security updates, or every
/// update of every repository.
pub const AUTO_MODES: [&str; 3] = ["off", "security", "all"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upgrade {
    pub package: String,
    /// The version installed (empty for a new package an upgrade brings).
    pub from: String,
    pub to: String,
}

/// An `Inst` line of `apt-get -s`: `Inst name [old] (new origin [arch])`.
fn upgrade(line: &str) -> Option<Upgrade> {
    let rest = line.strip_prefix("Inst ")?;
    let (package, rest) = rest.split_once(' ')?;
    let from = rest.strip_prefix('[').and_then(|r| r.split_once(']')).map(|(v, _)| v.to_owned()).unwrap_or_default();
    let to = rest.split_once('(')?.1.split_whitespace().next()?.to_owned();
    Some(Upgrade { package: package.to_owned(), from, to })
}

/// What a full upgrade would install now, from the lists apt has.
pub fn pending() -> Result<Vec<Upgrade>, String> {
    let out = Command::new("apt-get")
        .args(["-s", "-o", "Debug::NoLocking=1", "dist-upgrade"])
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| format!("apt-get: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("apt-get -s failed").to_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().filter_map(upgrade).collect())
}

/// The automatic mode, from apt's files.
pub fn auto_mode() -> &'static str {
    let periodic = std::fs::read_to_string(PERIODIC).unwrap_or_default();
    auto_mode_in(&periodic, &std::fs::read_to_string(ORIGINS).unwrap_or_default(), installed("unattended-upgrades"))
}

fn auto_mode_in(periodic: &str, origins: &str, installed: bool) -> &'static str {
    let on = periodic
        .lines()
        .any(|l| l.contains("APT::Periodic::Unattended-Upgrade") && l.contains("\"1\""));
    match (installed && on, origins.contains("\"origin=*\"")) {
        (false, _) => "off",
        (true, true) => "all",
        (true, false) => "security",
    }
}

pub fn installed(package: &str) -> bool {
    Command::new("dpkg-query")
        .args(["-W", "-f", "${db:Status-Status}", package])
        .output()
        .is_ok_and(|o| o.stdout == b"installed")
}

/// The two files for `mode`: the periodic one, and the origins (`None`:
/// left as it is).
pub fn auto_files(mode: &str) -> (String, Option<String>) {
    let on = if mode == "off" { "0" } else { "1" };
    let periodic = format!(
        "// Orbit-Settings' Updates page (orbit-services-helper).\nAPT::Periodic::Update-Package-Lists \"{on}\";\nAPT::Periodic::Unattended-Upgrade \"{on}\";\n"
    );
    let origins = match mode {
        "security" => Some(
            "// Orbit-Settings' Updates page: the security updates only.\n#clear Unattended-Upgrade::Origins-Pattern;\nUnattended-Upgrade::Origins-Pattern {\n        \"origin=Debian,codename=${distro_codename}-security,label=Debian-Security\";\n        \"origin=Debian,codename=${distro_codename},label=Debian-Security\";\n};\n"
                .to_owned(),
        ),
        "all" => Some(
            "// Orbit-Settings' Updates page: every update of every repository.\n#clear Unattended-Upgrade::Origins-Pattern;\nUnattended-Upgrade::Origins-Pattern {\n        \"origin=*\";\n};\n"
                .to_owned(),
        ),
        _ => None,
    };
    (periodic, origins)
}

/// A repository apt reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub file: String,
    pub uri: String,
    /// Suites and components: "trixie main contrib".
    pub what: String,
    pub enabled: bool,
}

/// The repositories of /etc/apt: one-line `.list` files and deb822
/// `.sources` files.
pub fn sources() -> Vec<Source> {
    let mut files: Vec<std::path::PathBuf> = vec!["/etc/apt/sources.list".into()];
    if let Ok(dir) = std::fs::read_dir("/etc/apt/sources.list.d") {
        let mut more: Vec<_> = dir.flatten().map(|e| e.path()).collect();
        more.sort();
        files.extend(more);
    }
    let mut out = Vec::new();
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let file = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        match path.extension().and_then(|e| e.to_str()) {
            Some("sources") => out.extend(deb822(&file, &text)),
            _ if file.ends_with(".list") || file == "sources.list" => out.extend(one_line(&file, &text)),
            _ => {}
        }
    }
    out
}

fn one_line(file: &str, text: &str) -> Vec<Source> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let (enabled, l) = match l.strip_prefix('#') {
                Some(rest) => (false, rest.trim()),
                None => (true, l),
            };
            let rest = l.strip_prefix("deb ")?;
            // Options in brackets: [arch=amd64 signed-by=…].
            let rest = match rest.trim_start().strip_prefix('[') {
                Some(r) => r.split_once(']')?.1,
                None => rest,
            };
            let mut words = rest.split_whitespace();
            let uri = words.next()?.to_owned();
            Some(Source { file: file.to_owned(), uri, what: words.collect::<Vec<_>>().join(" "), enabled })
        })
        .collect()
}

fn deb822(file: &str, text: &str) -> Vec<Source> {
    text.split("\n\n")
        .filter_map(|stanza| {
            let field = |name: &str| {
                stanza
                    .lines()
                    .find_map(|l| l.strip_prefix(name).and_then(|r| r.strip_prefix(':')))
                    .map(|v| v.trim().to_owned())
            };
            if !field("Types").is_some_and(|t| t.split_whitespace().any(|t| t == "deb")) {
                return None;
            }
            let what = [field("Suites"), field("Components")].into_iter().flatten().collect::<Vec<_>>().join(" ");
            Some(Source {
                file: file.to_owned(),
                uri: field("URIs")?,
                what,
                enabled: field("Enabled").is_none_or(|e| e != "no"),
            })
        })
        .collect()
}

/// One run of apt, from its history.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Transaction {
    pub date: String,
    pub command: String,
    /// "Upgrade: 3", "Install: 1"…
    pub changes: Vec<(String, usize)>,
}

/// The last `count` runs of apt, the latest first.
pub fn history(count: usize) -> Vec<Transaction> {
    history_in(&std::fs::read_to_string("/var/log/apt/history.log").unwrap_or_default(), count)
}

fn history_in(text: &str, count: usize) -> Vec<Transaction> {
    let mut out: Vec<Transaction> = text
        .split("\n\n")
        .filter_map(|block| {
            let mut t = Transaction::default();
            for line in block.lines() {
                let Some((key, value)) = line.split_once(": ") else { continue };
                match key {
                    "Start-Date" => t.date = value.split_whitespace().collect::<Vec<_>>().join(" "),
                    "Commandline" => t.command = value.to_owned(),
                    "Install" | "Upgrade" | "Remove" | "Purge" | "Downgrade" | "Reinstall" => {
                        // Packages are separated by "), ".
                        t.changes.push((key.to_owned(), value.matches("), ").count() + 1));
                    }
                    _ => {}
                }
            }
            (!t.date.is_empty()).then_some(t)
        })
        .collect();
    out.reverse();
    out.truncate(count);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrades_are_read() {
        let u = upgrade("Inst orbit-panel [0.1.0+git1] (0.1.0+git2 Orbit:trixie [amd64])").unwrap();
        assert_eq!((u.package.as_str(), u.from.as_str(), u.to.as_str()), ("orbit-panel", "0.1.0+git1", "0.1.0+git2"));
        let new = upgrade("Inst libfoo1 (2.0 Debian:13.2/stable [amd64])").unwrap();
        assert_eq!(new.from, "");
        assert!(upgrade("Conf orbit-panel (0.1.0+git2 Orbit:trixie [amd64])").is_none());
    }

    #[test]
    fn the_automatic_mode_is_read() {
        let (on, all) = auto_files("all");
        assert_eq!(auto_mode_in(&on, all.as_deref().unwrap(), true), "all");
        let (on, security) = auto_files("security");
        assert_eq!(auto_mode_in(&on, security.as_deref().unwrap(), true), "security");
        assert_eq!(auto_mode_in(&on, "", false), "off");
        let (off, none) = auto_files("off");
        assert!(none.is_none());
        assert_eq!(auto_mode_in(&off, "", true), "off");
        // As Debian's own dpkg-reconfigure writes it.
        assert_eq!(auto_mode_in("APT::Periodic::Update-Package-Lists \"1\";\nAPT::Periodic::Unattended-Upgrade \"1\";\n", "", true), "security");
    }

    #[test]
    fn sources_are_read() {
        let list = one_line("docker.list", "deb [arch=amd64 signed-by=/k.gpg] https://download.docker.com/linux/debian trixie stable\n# deb http://x/ y z\n");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].uri, "https://download.docker.com/linux/debian");
        assert_eq!(list[0].what, "trixie stable");
        assert!(!list[1].enabled);
        let d = deb822("debian.sources", "Types: deb deb-src\nURIs: http://deb.debian.org/debian\nSuites: trixie trixie-updates\nComponents: main\n\nTypes: deb-src\nURIs: http://x\nSuites: y\n");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].what, "trixie trixie-updates main");
    }

    #[test]
    fn history_is_read_latest_first() {
        let text = "\nStart-Date: 2026-11-04  21:40:17\nCommandline: apt-get purge -y ifupdown\nPurge: ifupdown:amd64 (0.8.44+deb13u1)\nEnd-Date: 2026-11-04  21:40:17\n\nStart-Date: 2026-11-10  19:14:43\nCommandline: apt install -y kitty\nInstall: a:amd64 (1, automatic), kitty:amd64 (2)\nEnd-Date: 2026-11-10  19:14:45\n";
        let h = history_in(text, 10);
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].date, "2026-11-10 19:14:43");
        assert_eq!(h[0].changes, vec![("Install".to_owned(), 2)]);
        assert_eq!(h[1].command, "apt-get purge -y ifupdown");
    }
}
