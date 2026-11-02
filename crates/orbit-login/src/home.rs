//! The encrypted `/home` (docs/PLANO-DISTRO.md §6.4): LUKS2, opened with
//! the password of the user who logs in, and open until the machine goes
//! down. `home:` of `/etc/orbit/login.yaml` names its `/etc/crypttab`
//! entry; `/etc/fstab` mounts `/home` from `/dev/mapper/NAME`, `noauto`.
//!
//! Each user's password is a keyslot of its own, written down in
//! `KEYSLOTS` ("user slot" lines, root's only). A password changed while
//! `/home` is open (`orbit-login --home-key`, from PAM's password stack)
//! gets a new keyslot and the user's old one goes; one changed while it
//! was shut catches up at the next login opened with the old password or
//! the recovery key. Keyslots are added with the volume key, read from
//! the device mapper: no other passphrase is needed, and none is kept.
//! So the volume is opened with `--disable-keyring`, which keeps the key
//! in the mapper's table instead of the kernel's keyring.

use std::{
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const KEYSLOTS: &str = "/etc/orbit/home-keyslots";

/// The encrypted `/home`: its crypttab name and device.
#[derive(Debug, Clone)]
pub struct Home {
    pub name: String,
    device: PathBuf,
}

impl Home {
    /// The crypttab entry `name`, if there is one.
    pub fn find(name: &str) -> Option<Home> {
        let text = std::fs::read_to_string("/etc/crypttab").ok()?;
        let source = text.lines().map(str::split_whitespace).find_map(|mut f| (f.next() == Some(name)).then(|| f.next()).flatten())?;
        let device = match source.split_once('=') {
            Some(("UUID", v)) => PathBuf::from("/dev/disk/by-uuid").join(v),
            Some(("PARTUUID", v)) => PathBuf::from("/dev/disk/by-partuuid").join(v),
            Some(("LABEL", v)) => PathBuf::from("/dev/disk/by-label").join(v),
            _ => PathBuf::from(source),
        };
        Some(Home { name: name.to_owned(), device })
    }

    fn mapper(&self) -> PathBuf {
        Path::new("/dev/mapper").join(&self.name)
    }

    /// Opened and mounted on `/home`.
    pub fn is_open(&self) -> bool {
        self.mapper().exists() && mounted("/home")
    }

    /// Opens the volume with `password` (unless it is open) and mounts
    /// `/home`. A refused password says "Authentication failure", as PAM
    /// does, for the greet to put in its words.
    pub fn open(&self, password: &str) -> Result<(), String> {
        if !self.mapper().exists() {
            let device = self.device.to_string_lossy();
            let ok = run_with_input(&["cryptsetup", "open", "--type", "luks", "--disable-keyring", "--key-file=-", &device, &self.name], password.as_bytes())
                .map_err(|e| format!("cryptsetup: {e}"))?;
            if !ok.0 {
                log::warn!("{}: not opened: {}", self.name, ok.1.trim());
                return Err("Authentication failure".into());
            }
            log::info!("{}: opened", self.name);
        }
        if !mounted("/home") {
            let status = Command::new("mount").arg("/home").stdin(Stdio::null()).status().map_err(|e| format!("mount: {e}"))?;
            if !status.success() {
                return Err("/home did not mount (the journal says why)".into());
            }
            log::info!("/home mounted");
        }
        Ok(())
    }

    /// Makes `password` `user`'s keyslot: kept if it already opens the
    /// volume, otherwise added with the volume key; the user's keyslot
    /// before, if another, removed. The volume must be open.
    pub fn follow(&self, user: &str, password: &str) -> Result<(), String> {
        let device = self.device.to_string_lossy().into_owned();
        let mut slots = read_keyslots();
        let old = slots.iter().find(|(u, _)| u == user).map(|(_, s)| *s);
        let (ok, out) = run_with_input(&["cryptsetup", "open", "--test-passphrase", "-v", "--key-file=-", &device], password.as_bytes()).map_err(|e| e.to_string())?;
        let slot = if ok {
            let slot = slot_in(&out, "unlocked").ok_or("cryptsetup did not say the keyslot")?;
            if old == Some(slot) {
                return Ok(());
            }
            slot
        } else {
            let slot = self.add(&device, password)?;
            log::info!("{user}: keyslot {slot} added");
            slot
        };
        // The user's keyslot before: their old password.
        let kill = old.filter(|&o| o != slot);
        slots.retain(|(u, s)| u != user && *s != slot);
        slots.push((user.to_owned(), slot));
        write_keyslots(&slots)?;
        if let Some(old) = kill {
            // The new password, in a keyslot that stays, is the proof
            // cryptsetup asks for (on stdin: never the caller's).
            let (killed, out) = run_with_input(&["cryptsetup", "luksKillSlot", "--key-file=-", &device, &old.to_string()], password.as_bytes()).map_err(|e| e.to_string())?;
            if !killed {
                return Err(format!("keyslot {slot} added, but the old keyslot {old} stays: {}", out.trim()));
            }
            log::info!("{user}: old keyslot {old} removed");
        }
        Ok(())
    }

    /// A new keyslot for `password`, from the volume key in the mapper's
    /// table (`--disable-keyring` at open put it there).
    fn add(&self, device: &str, password: &str) -> Result<u32, String> {
        let table = Command::new("dmsetup").args(["table", "--showkeys", &self.name]).stdin(Stdio::null()).output().map_err(|e| format!("dmsetup: {e}"))?;
        let table = String::from_utf8_lossy(&table.stdout).into_owned();
        // start length crypt cipher KEY iv_offset device offset
        let hex = table.split_whitespace().nth(4).filter(|k| !k.starts_with(':')).ok_or("the volume key is not in the mapper's table (opened without --disable-keyring)")?;
        let key = decode_hex(hex).ok_or("the volume key is not hex")?;
        let bits = (key.len() * 8).to_string();
        let file = PathBuf::from("/run/orbit/home.vk");
        let _ = std::fs::create_dir_all("/run/orbit");
        let _ = std::fs::remove_file(&file);
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&file)
            .and_then(|mut f| f.write_all(&key));
        // cryptsetup does not say which keyslot it took with a volume key:
        // the first free one is chosen here.
        let slot = free_slot(device)?;
        let result = written.map_err(|e| format!("{}: {e}", file.display())).and_then(|()| {
            let vk = file.to_string_lossy();
            let args = ["cryptsetup", "luksAddKey", "--key-size", &bits, "--volume-key-file", &vk, "--new-key-slot", &slot.to_string(), "--new-keyfile=-", device];
            run_with_input(&args, password.as_bytes()).map_err(|e| e.to_string())
        });
        if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(&file) {
            let _ = f.write_all(&vec![0u8; key.len()]);
        }
        let _ = std::fs::remove_file(&file);
        let (ok, out) = result?;
        if !ok {
            return Err(format!("luksAddKey: {}", out.trim()));
        }
        Ok(slot)
    }
}

/// `orbit-login --home-key`: PAM's password stack (pam_exec with
/// expose_authtok, as root) after a password changed; the new one on
/// stdin. Nothing to do without an encrypted `/home`; with it shut, the
/// next login catches up.
pub fn home_key(config_home: Option<&str>) -> i32 {
    let Some(home) = config_home.and_then(Home::find) else { return 0 };
    // pam_exec (from passwd) gives no PATH: cryptsetup is in /usr/sbin.
    // Safety: one thread yet, nothing else reads the environment.
    unsafe { std::env::set_var("PATH", "/usr/sbin:/usr/bin:/sbin:/bin") };
    let Ok(user) = std::env::var("PAM_USER") else {
        eprintln!("orbit-login --home-key: run by pam_exec (PAM_USER)");
        return 2;
    };
    let mut password = Vec::new();
    let _ = std::io::Read::read_to_end(&mut std::io::stdin(), &mut password);
    while password.last().is_some_and(|b| *b == 0 || *b == b'\n') {
        password.pop();
    }
    let password = String::from_utf8_lossy(&password).into_owned();
    if password.is_empty() || orbit_login::user(&user).is_none() {
        return 0;
    }
    if !home.mapper().exists() {
        journal(&format!("{user}: /home is shut; its keyslot follows the new password at the next login"));
        return 0;
    }
    match home.follow(&user, &password) {
        Ok(()) => {
            journal(&format!("{user}: the /home keyslot follows the new password"));
            0
        }
        Err(err) => {
            journal(&format!("{user}: the /home keyslot did not follow the new password: {err}"));
            // Optional in PAM: the password changed anyway.
            1
        }
    }
}

/// To the journal: under pam_exec (quiet) stderr goes nowhere.
fn journal(message: &str) {
    log::info!("{message}");
    let _ = Command::new("logger").args(["-t", "orbit-login", "--", message]).stdin(Stdio::null()).status();
}

/// The lowest keyslot of `device` no key is in (LUKS2 has 32).
fn free_slot(device: &str) -> Result<u32, String> {
    let out = Command::new("cryptsetup").args(["luksDump", "--dump-json-metadata", device]).stdin(Stdio::null()).output().map_err(|e| format!("cryptsetup: {e}"))?;
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("luksDump: {e}"))?;
    let used: Vec<u32> = json["keyslots"].as_object().ok_or("luksDump: no keyslots")?.keys().filter_map(|k| k.parse().ok()).collect();
    (0..32).find(|n| !used.contains(n)).ok_or_else(|| "no free keyslot".into())
}

fn mounted(path: &str) -> bool {
    std::fs::read_to_string("/proc/self/mounts").is_ok_and(|m| m.lines().any(|l| l.split_whitespace().nth(1) == Some(path)))
}

/// Runs `args` with `input` on stdin: whether it succeeded, and what it
/// said (stdout and stderr).
fn run_with_input(args: &[&str], input: &[u8]) -> std::io::Result<(bool, String)> {
    // What cryptsetup says is read (the keyslot): never translated.
    let mut child = Command::new(args[0])
        .args(&args[1..])
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", args[0])))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input);
    }
    let out = child.wait_with_output()?;
    let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), said))
}

/// The N of cryptsetup's "Key slot N unlocked." / "Key slot N created.".
fn slot_in(text: &str, word: &str) -> Option<u32> {
    text.lines().find_map(|l| {
        let rest = l.trim().strip_prefix("Key slot ")?;
        let (n, tail) = rest.split_once(' ')?;
        tail.starts_with(word).then(|| n.parse().ok()).flatten()
    })
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    hex.len().is_multiple_of(2).then_some(())?;
    (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect()
}

fn read_keyslots() -> Vec<(String, u32)> {
    std::fs::read_to_string(KEYSLOTS)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            Some((f.next()?.to_owned(), f.next()?.parse().ok()?))
        })
        .collect()
}

fn write_keyslots(slots: &[(String, u32)]) -> Result<(), String> {
    let mut text = String::from("# The LUKS keyslot of each user's password on the encrypted /home\n# (orbit-login keeps it; docs/PLANO-DISTRO.md §6.4).\n");
    for (user, slot) in slots {
        text.push_str(&format!("{user} {slot}\n"));
    }
    let tmp = format!("{KEYSLOTS}.new");
    std::fs::write(&tmp, text).map_err(|e| format!("{tmp}: {e}"))?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).map_err(|e| format!("{tmp}: {e}"))?;
    std::fs::rename(&tmp, KEYSLOTS).map_err(|e| format!("{KEYSLOTS}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cryptsetup_says_the_keyslot() {
        assert_eq!(slot_in("No usable token is available.\nKey slot 2 unlocked.\nCommand successful.\n", "unlocked"), Some(2));
        assert_eq!(slot_in("No key available with this passphrase.\n", "unlocked"), None);
    }

    #[test]
    fn the_volume_key_is_hex() {
        assert_eq!(decode_hex("00ff10"), Some(vec![0, 255, 16]));
        assert_eq!(decode_hex("0g"), None);
        assert_eq!(decode_hex("abc"), None);
    }
}
