//! orbit-services-helper: the steps of Orbit's services that need root,
//! run by `pkexec` for Orbit-Settings and `orbitctl services`.
//!
//!   orbit-services-helper [--trial SECONDS] STEP ARG [-- STEP ARG]…
//!
//! STEP is `install` or `remove` (a package of the catalogue), `unit-on`
//! or `unit-off` (one of its units: enabled and started, or stopped and
//! disabled), or `logind KEY VALUE` (what the lid and the power key do,
//! in /etc/systemd/logind.conf.d/50-orbit.conf, logind reloaded),
//! `refresh` and `upgrade` (apt's lists, every update), or `auto-updates
//! off|security|all` (unattended-upgrades, installed when turned on),
//! `boot-info`, `boot-timeout N`, `boot-default ID`, `snapshot TEXT`,
//! `rollback N|current`, `uki-sync N` and `boot-theme NAME` (systemd-boot,
//! snapper, Plymouth), or `system-drive
//! ADDRESS` and `system-drive-remove ADDRESS` (a network drive for every
//! user, mounted at boot; an SMB share's password is the first line of the
//! standard input). Every step is checked against the catalogue before any is
//! run; a step that fails undoes the unit steps already done. With
//! `--trial`, the unit steps done are undone after SECONDS unless a line
//! `keep` comes on the standard input first (`undo` undoes them at once).
//!
//! It says what it does on the standard output, a line each: `step TEXT`,
//! `progress N TEXT` (apt's, 0–100), `trial SECONDS`, `kept`, `undone`,
//! `error TEXT`, `done`.

use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

use orbit_services::Privileged;

const PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

fn main() -> ExitCode {
    // SAFETY: geteuid has no preconditions and cannot fail.
    if unsafe { libc::geteuid() } != 0 {
        println!("error orbit-services-helper runs as root, by pkexec");
        return ExitCode::from(2);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (trial, steps) = match parse(&args) {
        Ok(parsed) => parsed,
        Err(e) => {
            println!("error {e}");
            return ExitCode::from(2);
        }
    };
    let mut done: Vec<Privileged> = Vec::new();
    for step in &steps {
        println!("step {}", describe(step));
        if let Err(e) = run(step) {
            println!("error {e}");
            undo(&done);
            return ExitCode::FAILURE;
        }
        done.push(step.clone());
    }
    if let Some(seconds) = trial
        && done.iter().any(|s| s.undo().is_some())
    {
        println!("trial {seconds}");
        if kept(seconds) {
            println!("kept");
        } else {
            undo(&done);
            println!("undone");
        }
    }
    println!("done");
    ExitCode::SUCCESS
}

/// `--trial SECONDS`, then the steps between `--`, each checked.
fn parse(args: &[String]) -> Result<(Option<u32>, Vec<Privileged>), String> {
    let mut rest = args;
    let mut trial = None;
    if rest.first().is_some_and(|a| a == "--trial") {
        let seconds = rest.get(1).and_then(|s| s.parse::<u32>().ok()).filter(|s| (5..=300).contains(s));
        trial = Some(seconds.ok_or("--trial needs a number of seconds, 5 to 300")?);
        rest = &rest[2..];
    }
    let mut steps = Vec::new();
    for words in rest.split(|a| a == "--") {
        match words {
            [verb, key, value] if verb == "logind" => steps.push(Privileged::logind(key, value)?),
            [verb, arg] => steps.push(Privileged::parse(verb, arg)?),
            [verb] => steps.push(Privileged::word(verb)?),
            _ => return Err(format!("a step is a verb and its argument, not {words:?}")),
        }
    }
    if steps.is_empty() || steps.len() > 8 {
        return Err("from one to eight steps".into());
    }
    Ok((trial, steps))
}

fn describe(step: &Privileged) -> String {
    match step {
        Privileged::Install(p) => format!("Installing {p}"),
        Privileged::Remove(p) => format!("Removing {p}"),
        Privileged::UnitOn(u) => format!("Starting {u}"),
        Privileged::UnitOff(u) => format!("Stopping {u}"),
        Privileged::Logind(k, v) => format!("Setting {k} to {v}"),
        Privileged::Refresh => "Fetching the package lists".into(),
        Privileged::Upgrade => "Installing the updates".into(),
        Privileged::AutoUpdates(m) => format!("Automatic updates: {m}"),
        Privileged::BootInfo => "Reading the boot settings".into(),
        Privileged::BootTimeout(n) => format!("Boot menu: {n} s"),
        Privileged::BootDefault(id) => format!("Booting {id} by default"),
        Privileged::Snapshot(_) => "Taking a snapshot".into(),
        Privileged::Rollback(n) if n == "current" => "Keeping this system as the one that starts".into(),
        Privileged::Rollback(n) => format!("Going back to snapshot {n}"),
        Privileged::UkiSync(n) => format!("Making the boot images of snapshot {n}"),
        Privileged::BootTheme(t) => format!("Boot theme: {t}"),
        Privileged::SystemDrive(a) => format!("Adding {a} for every user"),
        Privileged::SystemDriveRemove(a) => format!("Removing {a}"),
    }
}

fn run(step: &Privileged) -> Result<(), String> {
    match step {
        Privileged::Install(package) => {
            // The lists may be old enough not to know the package yet.
            let _ = apt(&["update"]);
            apt(&["install", package])
        }
        Privileged::Remove(package) => apt(&["remove", package]),
        Privileged::UnitOn(unit) => systemctl(&["enable", "--now", unit]),
        Privileged::UnitOff(unit) => systemctl(&["disable", "--now", unit]),
        Privileged::Logind(key, value) => logind(key, value),
        Privileged::Refresh => apt(&["update"]),
        Privileged::Upgrade => apt(&["dist-upgrade"]),
        Privileged::AutoUpdates(mode) => auto_updates(mode),
        Privileged::BootInfo => boot_info(),
        Privileged::BootTimeout(seconds) => boot_timeout(*seconds),
        Privileged::BootDefault(id) => tool("bootctl", &["set-default", id]).map(|_| ()),
        Privileged::Snapshot(description) => {
            tool("snapper", &["-c", "root", "create", "-c", "number", "-d", description]).map(|_| ())
        }
        Privileged::Rollback(target) => rollback(target),
        Privileged::UkiSync(number) => after_rollback(number),
        Privileged::BootTheme(theme) => {
            let file = format!("/usr/share/plymouth/themes/{theme}/{theme}.plymouth");
            if !std::path::Path::new(&file).is_file() {
                return Err(format!("no boot theme {theme} installed"));
            }
            // -R makes the initramfs again, where the theme is read from.
            tool("plymouth-set-default-theme", &["-R", theme]).map(|_| ())
        }
        Privileged::SystemDrive(address) => system_drive(address),
        Privileged::SystemDriveRemove(address) => system_drive_remove(address),
    }
}

/// A drive for every user: the tools for its kind installed, its password
/// kept (root's only), its units written and the automount started.
fn system_drive(address: &str) -> Result<(), String> {
    use orbit_services::drives::{self, Kind, SystemDrive};
    let drive = SystemDrive::parse(address)?;
    let (tool_path, package) = match drive.kind {
        Kind::Smb => ("/usr/sbin/mount.cifs", "cifs-utils"),
        Kind::Nfs => ("/usr/sbin/mount.nfs", "nfs-common"),
    };
    if !std::path::Path::new(tool_path).exists() {
        let _ = apt(&["update"]);
        apt(&["install", package])?;
    }
    if let Some(user) = &drive.user {
        let mut password = String::new();
        std::io::stdin().lock().read_line(&mut password).map_err(|e| format!("the password: {e}"))?;
        let password = password.trim_end_matches(['\r', '\n']);
        if password.contains('\n') {
            return Err("the password: one line".into());
        }
        std::fs::create_dir_all(drives::CRED_DIR).map_err(|e| format!("{}: {e}", drives::CRED_DIR))?;
        write_private(&drive.credentials(), &drives::credentials_text(user, password))?;
    }
    std::fs::create_dir_all(drive.mount_point()).map_err(|e| format!("{}: {e}", drive.mount_point().display()))?;
    let unit = drive.unit();
    let dir = std::path::Path::new(drives::UNIT_DIR);
    for (suffix, text) in [("mount", drive.mount_unit()), ("automount", drive.automount_unit())] {
        let path = dir.join(format!("{unit}.{suffix}"));
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--now", &format!("{unit}.automount")])
}

fn system_drive_remove(address: &str) -> Result<(), String> {
    use orbit_services::drives::{self, SystemDrive};
    let drive = SystemDrive::parse(address)?;
    let unit = drive.unit();
    let _ = systemctl(&["disable", "--now", &format!("{unit}.automount")]);
    let _ = systemctl(&["stop", &format!("{unit}.mount")]);
    let dir = std::path::Path::new(drives::UNIT_DIR);
    for suffix in ["mount", "automount"] {
        let _ = std::fs::remove_file(dir.join(format!("{unit}.{suffix}")));
    }
    let _ = std::fs::remove_file(drive.credentials());
    // Only an empty folder: never what someone left in it unmounted.
    let _ = std::fs::remove_dir(drive.mount_point());
    systemctl(&["daemon-reload"])
}

/// A file only root reads, written whole.
fn write_private(path: &std::path::Path, text: &str) -> Result<(), String> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path).map_err(err)?;
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).map_err(err)?;
    file.write_all(text.as_bytes()).map_err(err)
}

/// systemd-boot's loader.conf, where bootctl finds the ESP.
fn loader_conf() -> Result<String, String> {
    let esp = tool("bootctl", &["--print-esp-path"])?;
    Ok(format!("{}/loader/loader.conf", esp.trim()))
}

/// The boot loader's entries and the root's snapshots, as one `info`
/// line of JSON: {"timeout", "entries", "snapshots"}; what cannot be read
/// is null.
fn boot_info() -> Result<(), String> {
    let json = |text: Result<String, String>| text.ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    let timeout = loader_conf()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.trim().strip_prefix("timeout").map(|v| v.trim().to_owned()))
        });
    let info = serde_json::json!({
        "timeout": timeout,
        "entries": json(tool("bootctl", &["list", "--json=short", "--no-pager"])),
        "snapshots": json(tool("snapper", &["--jsonout", "-c", "root", "list"])),
        "root": root_info(),
    });
    println!("info {info}");
    Ok(())
}

/// `snapper rollback` to a snapshot (or, `current`, of the system running
/// now), then the UKIs made again for the new default. Done by snapper
/// itself in the client (`--no-dbus`), so its plugin sees
/// ORBIT_SERVICES_HELPER and leaves the UKIs to this. A failure after the
/// default changed puts the old default back and the UKIs with it.
fn rollback(target: &str) -> Result<(), String> {
    use orbit_services::boot;
    if !Path::new("/etc/snapper/configs/root").is_file() {
        return Err("snapper has no config for the root".into());
    }
    if target != "current" {
        let root = PathBuf::from(format!("/.snapshots/{target}/snapshot"));
        if !root.is_dir() {
            return Err(format!("there is no snapshot {target}"));
        }
        if boot::kernels(&root).is_empty() {
            return Err(format!("snapshot {target} holds no kernel to start"));
        }
    }
    // The UKIs' place, asked before anything changes: no UKI layout, no
    // going back.
    uki_place(Path::new("/"))?;
    let old = default_id()?;
    let mut args = vec!["--no-dbus", "--ambit", "classic", "rollback", "--print-number"];
    if target != "current" {
        args.push(target);
    }
    let out = Command::new("snapper")
        .args(&args)
        .env("PATH", PATH)
        .env("LC_ALL", "C.UTF-8")
        .env("ORBIT_SERVICES_HELPER", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("snapper: {e}"))?;
    if !out.status.success() {
        return Err(last_line(&out.stderr).unwrap_or_else(|| "snapper rollback failed".into()));
    }
    let new = String::from_utf8_lossy(&out.stdout)
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_digit()))
        .map(str::to_owned)
        .ok_or("snapper did not say the new snapshot's number")?;
    if let Err(e) = after_rollback(&new) {
        println!("error {e}");
        println!("step Putting the system that started back");
        tool("btrfs", &["subvolume", "set-default", &old, "/"])?;
        sync_ukis(Path::new("/"))?;
        let _ = std::fs::remove_file(previous_path()?);
        return Err(format!("the system stays as it was: {e}"));
    }
    Ok(())
}

/// The subvolume btrfs mounts as the root, by ID.
fn default_id() -> Result<String, String> {
    // "ID 256 gen 5165 top level 5 path @"
    let text = tool("btrfs", &["subvolume", "get-default", "/"])?;
    text.split_whitespace().nth(1).map(str::to_owned).ok_or_else(|| format!("btrfs: {}", text.trim()))
}

/// A subvolume's ID and UUIDs (its own and its parent's), from `btrfs
/// subvolume show`.
fn subvolume(path: &Path) -> Result<(String, String, String), String> {
    let text = tool("btrfs", &["subvolume", "show", &path.to_string_lossy()])?;
    let field = |name: &str| {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(name).map(|v| v.trim().to_owned()))
            .unwrap_or_default()
    };
    Ok((field("Subvolume ID:"), field("UUID:"), field("Parent UUID:")))
}

/// After the default became snapshot `number`: the system running now
/// gets its UKI (unless the new default is a copy of it), and the ESP's
/// UKIs are made from the new default's kernels.
fn after_rollback(number: &str) -> Result<(), String> {
    let root = PathBuf::from(format!("/.snapshots/{number}/snapshot"));
    let (id, _, parent) = subvolume(&root)?;
    if default_id()? != id {
        return Err(format!("snapshot {number} is not the one that starts"));
    }
    let (_, running, _) = subvolume(Path::new("/"))?;
    if parent == running {
        // A copy of this system: no other to go back to.
        let _ = std::fs::remove_file(previous_path()?);
    } else {
        make_previous()?;
    }
    sync_ukis(&root)
}

/// Where kernel-install puts UKIs, and its entry token, for a root's
/// newest kernel.
fn uki_place(root: &Path) -> Result<(PathBuf, String), String> {
    let kernels = orbit_services::boot::kernels(root);
    let version = kernels.last().ok_or("no kernel to start in /boot")?;
    let image = root.join(format!("boot/vmlinuz-{version}"));
    let text = tool("kernel-install", &["inspect", "--json=short", version, &image.to_string_lossy()])?;
    let info: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("kernel-install inspect: {e}"))?;
    if info["Layout"].as_str() != Some("uki") {
        return Err("going back to a snapshot needs systemd-boot with UKIs (layout=uki)".into());
    }
    let (Some(boot), Some(token)) = (info["BootRoot"].as_str(), info["EntryToken"].as_str()) else {
        return Err("kernel-install did not say where the UKIs go".into());
    };
    Ok((Path::new(boot).join("EFI/Linux"), token.to_owned()))
}

fn previous_path() -> Result<PathBuf, String> {
    let (dir, token) = uki_place(Path::new("/"))?;
    Ok(dir.join(format!("{token}-{}.efi", orbit_services::boot::PREVIOUS)))
}

/// The UKI that starts the system running now from its subvolume, by its
/// running kernel (or its newest one, if that was removed since).
fn make_previous() -> Result<(), String> {
    use orbit_services::boot;
    let (dir, token) = uki_place(Path::new("/"))?;
    let kernels = boot::kernels(Path::new("/"));
    let running = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim().to_owned();
    let version = kernels.iter().find(|v| **v == running).or(kernels.last()).ok_or("no kernel to start in /boot")?;
    let fsroot = tool("findmnt", &["-no", "FSROOT", "/"])?.trim().to_owned();
    let cmdline = std::fs::read_to_string("/etc/kernel/cmdline").map_err(|e| format!("/etc/kernel/cmdline: {e}"))?;
    let cmdline = boot::cmdline_with_subvol(cmdline.trim(), &fsroot);
    let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let release = Path::new("/run/orbit-previous-os-release");
    std::fs::write(release, boot::previous_os_release(&os_release, &token)).map_err(|e| e.to_string())?;
    let target = dir.join(format!("{token}-{}.efi", boot::PREVIOUS));
    let partial = dir.join(format!("{token}-{}.efi.new", boot::PREVIOUS));
    let made = tool(
        "ukify",
        &[
            "build",
            &format!("--linux=/boot/vmlinuz-{version}"),
            &format!("--initrd=/boot/initrd.img-{version}"),
            &format!("--cmdline={cmdline}"),
            &format!("--os-release=@{}", release.display()),
            &format!("--uname={version}"),
            &format!("--output={}", partial.display()),
        ],
    );
    let _ = std::fs::remove_file(release);
    if let Err(e) = made {
        let _ = std::fs::remove_file(&partial);
        return Err(format!("ukify: {e}"));
    }
    std::fs::rename(&partial, &target).map_err(|e| format!("{}: {e}", target.display()))
}

/// The ESP's UKIs made again from `root`'s kernels (kernel-install, as an
/// upgrade makes them): one per kernel it holds, the others removed. The
/// previous system's UKI is left alone.
fn sync_ukis(root: &Path) -> Result<(), String> {
    use orbit_services::boot;
    let kernels = boot::kernels(root);
    if kernels.is_empty() {
        return Err(format!("{} holds no kernel to start", root.display()));
    }
    let (dir, token) = uki_place(root)?;
    // The versions the root has not go first, so the ESP (a UKI is some
    // 150 MB) has room; meanwhile the previous system's UKI still starts.
    remove_ukis(&dir, &token, |version, _| !kernels.iter().any(|k| k == version))?;
    let started = std::time::SystemTime::now() - Duration::from_secs(2);
    for version in &kernels {
        let image = root.join(format!("boot/vmlinuz-{version}"));
        let initrd = root.join(format!("boot/initrd.img-{version}"));
        tool("kernel-install", &["add", version, &image.to_string_lossy(), &initrd.to_string_lossy()])
            .map_err(|e| format!("kernel-install {version}: {e}"))?;
    }
    // Older names of a version just made (a boot counter that changed).
    remove_ukis(&dir, &token, |_, made| made < started)
}

/// Removes the UKIs in `dir` (the previous system's aside) that `gone`
/// says, by version and time made.
fn remove_ukis(dir: &Path, token: &str, gone: impl Fn(&str, std::time::SystemTime) -> bool) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten() {
        let Some(version) = entry.file_name().to_str().and_then(|n| orbit_services::boot::uki_version(n, token)) else {
            continue;
        };
        let made = entry.metadata().and_then(|m| m.modified()).map_err(|e| e.to_string())?;
        if gone(&version, made) {
            let path = entry.path();
            std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// Which subvolume runs and which starts (they differ after starting the
/// system from before going back), whether that one's UKI is there, and
/// whether going back can be done here at all.
fn root_info() -> serde_json::Value {
    let running = tool("findmnt", &["-no", "FSROOT", "/"]).map(|t| t.trim().to_owned()).ok();
    // "ID 256 gen 5165 top level 5 path @"
    let default = tool("btrfs", &["subvolume", "get-default", "/"])
        .ok()
        .and_then(|t| t.split_once(" path ").map(|(_, p)| format!("/{}", p.trim())));
    let rollback = Path::new("/etc/snapper/configs/root").is_file() && uki_place(Path::new("/")).is_ok();
    serde_json::json!({
        "running": running,
        "default": default,
        "previous": previous_path().is_ok_and(|p| p.is_file()),
        "rollback": rollback,
    })
}

/// loader.conf's `timeout` set, its other lines kept.
fn boot_timeout(seconds: u32) -> Result<(), String> {
    let path = loader_conf()?;
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::write(&path, with_timeout(&old, seconds)).map_err(|e| format!("{path}: {e}"))
}

fn with_timeout(old: &str, seconds: u32) -> String {
    let mut found = false;
    let mut lines: Vec<String> = old
        .lines()
        .map(|l| {
            if l.trim_start().starts_with("timeout") {
                found = true;
                format!("timeout {seconds}")
            } else {
                l.to_owned()
            }
        })
        .collect();
    if !found {
        lines.push(format!("timeout {seconds}"));
    }
    lines.join("\n") + "\n"
}

/// A tool's output, or its last error line.
fn tool(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .env("PATH", PATH)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(last_line(&out.stderr).unwrap_or_else(|| format!("{program} failed")))
    }
}

/// unattended-upgrades on (installed first) or off, and which updates.
fn auto_updates(mode: &str) -> Result<(), String> {
    use orbit_services::updates;
    if mode != "off" && !updates::installed("unattended-upgrades") {
        apt(&["install", "unattended-upgrades"])?;
    }
    let (periodic, origins) = updates::auto_files(mode);
    std::fs::write(updates::PERIODIC, periodic).map_err(|e| format!("{}: {e}", updates::PERIODIC))?;
    if let Some(origins) = origins {
        std::fs::write(updates::ORIGINS, origins).map_err(|e| format!("{}: {e}", updates::ORIGINS))?;
    }
    Ok(())
}

/// Orbit's logind drop-in: `key=value` set, the others it holds kept.
const LOGIND_FILE: &str = "/etc/systemd/logind.conf.d/50-orbit.conf";

fn logind(key: &str, value: &str) -> Result<(), String> {
    let old = std::fs::read_to_string(LOGIND_FILE).unwrap_or_default();
    std::fs::create_dir_all("/etc/systemd/logind.conf.d").map_err(|e| e.to_string())?;
    std::fs::write(LOGIND_FILE, logind_text(&old, key, value)).map_err(|e| e.to_string())?;
    systemctl(&["reload", "systemd-logind"])
}

/// The drop-in with `key=value`: its other settings kept, the comment and
/// the section written again.
fn logind_text(old: &str, key: &str, value: &str) -> String {
    let mut settings: Vec<(String, String)> = old
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .filter(|(k, _)| orbit_services::LOGIND_KEYS.contains(&k.as_str()))
        .collect();
    match settings.iter_mut().find(|(k, _)| k == key) {
        Some(entry) => entry.1 = value.to_owned(),
        None => settings.push((key.to_owned(), value.to_owned())),
    }
    let mut text = String::from("# Orbit-Settings' Power page (orbit-services-helper): what the lid and\n# the power key do.\n[Login]\n");
    for (k, v) in settings {
        text.push_str(&format!("{k}={v}\n"));
    }
    text
}

/// Undoes the unit steps of `done`, the last first.
fn undo(done: &[Privileged]) {
    for step in done.iter().rev().filter_map(Privileged::undo) {
        println!("step {}", describe(&step));
        if let Err(e) = run(&step) {
            println!("error {e}");
        }
    }
}

/// Whether `keep` comes on the standard input within `seconds`; `undo`,
/// or the input closing, is no at once.
fn kept(seconds: u32) -> bool {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            match line.as_deref().map(str::trim) {
                Ok("keep") => {
                    let _ = tx.send(true);
                    return;
                }
                Ok("undo") | Err(_) => break,
                Ok(_) => continue,
            }
        }
        let _ = tx.send(false);
    });
    rx.recv_timeout(Duration::from_secs(seconds.into())).unwrap_or(false)
}

fn systemctl(args: &[&str]) -> Result<(), String> {
    let out = Command::new("systemctl")
        .args(args)
        .env("PATH", PATH)
        .output()
        .map_err(|e| format!("systemctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(last_line(&out.stderr).unwrap_or_else(|| format!("systemctl {} failed", args.join(" "))))
    }
}

/// apt-get, with its progress said as `progress` lines.
fn apt(args: &[&str]) -> Result<(), String> {
    let mut child = Command::new("apt-get")
        .args(["-y", "-qq", "-o", "APT::Status-Fd=1"])
        .args(["-o", "Dpkg::Options::=--force-confdef", "-o", "Dpkg::Options::=--force-confold"])
        .args(args)
        .env("PATH", PATH)
        .env("DEBIAN_FRONTEND", "noninteractive")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("apt-get: {e}"))?;
    let stderr = child.stderr.take().expect("piped");
    let errors = thread::spawn(move || {
        let mut text = Vec::new();
        let _ = std::io::Read::read_to_end(&mut BufReader::new(stderr), &mut text);
        text
    });
    for line in BufReader::new(child.stdout.take().expect("piped")).lines().map_while(Result::ok) {
        // pmstatus:PACKAGE:PERCENT:TEXT and dlstatus:…: apt's status lines.
        let mut parts = line.splitn(4, ':');
        if let (Some("pmstatus" | "dlstatus"), Some(_), Some(percent), Some(text)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
            && let Ok(p) = percent.parse::<f32>()
        {
            println!("progress {} {}", p.clamp(0.0, 100.0) as u8, text);
        }
    }
    let status = child.wait().map_err(|e| format!("apt-get: {e}"))?;
    let errors = errors.join().unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        Err(last_line(&errors).unwrap_or_else(|| format!("apt-get {} failed", args.join(" "))))
    }
}

fn last_line(bytes: &[u8]) -> Option<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn steps_are_parsed_and_checked() {
        let (trial, steps) = parse(&words("--trial 30 unit-on orbit-firewall.service")).unwrap();
        assert_eq!(trial, Some(30));
        assert_eq!(steps.len(), 1);
        let (_, steps) = parse(&words("install orbit-remote-server -- remove orbit-remote-server")).unwrap();
        assert_eq!(steps.len(), 2);
        assert!(parse(&words("install bash")).is_err());
        assert!(parse(&words("--trial 9999 unit-on orbit-firewall.service")).is_err());
        assert!(parse(&words("install")).is_err());
        assert!(parse(&[]).is_err());
        let (_, steps) = parse(&words("logind HandleLidSwitch lock")).unwrap();
        assert_eq!(steps, vec![Privileged::Logind("HandleLidSwitch".into(), "lock".into())]);
        assert!(parse(&words("logind HandleLidSwitch reboot")).is_err());
        let (_, steps) = parse(&words("refresh -- upgrade -- auto-updates security")).unwrap();
        assert_eq!(steps, vec![Privileged::Refresh, Privileged::Upgrade, Privileged::AutoUpdates("security".into())]);
        assert!(parse(&words("auto-updates sometimes")).is_err());
        assert!(parse(&words("reboot")).is_err());
    }

    #[test]
    fn boot_steps() {
        let (_, steps) = parse(&words("boot-info -- boot-timeout 5 -- boot-default orbit-6.12.uki.efi -- snapshot before")).unwrap();
        assert_eq!(steps[1], Privileged::BootTimeout(5));
        assert_eq!(steps[1].args(), vec!["boot-timeout", "5"]);
        assert!(parse(&words("boot-timeout 7")).is_err());
        assert!(parse(&words("boot-default ../../x")).is_err());
        assert_eq!(with_timeout("default @saved\ntimeout 0\n", 5), "default @saved\ntimeout 5\n");
        assert_eq!(with_timeout("", 3), "timeout 3\n");
        let (_, steps) = parse(&words("boot-theme orbit -- system-drive nfs://nas/export")).unwrap();
        assert_eq!(steps[0], Privileged::BootTheme("orbit".into()));
        assert_eq!(steps[1], Privileged::SystemDrive("nfs://nas/export".into()));
        assert!(parse(&words("boot-theme ../x")).is_err());
        let (_, steps) = parse(&words("rollback 91 -- rollback current -- uki-sync 93")).unwrap();
        assert_eq!(steps[0], Privileged::Rollback("91".into()));
        assert_eq!(steps[1].args(), vec!["rollback", "current"]);
        assert_eq!(steps[2], Privileged::UkiSync("93".into()));
        for bad in ["rollback 0", "rollback ../1", "rollback 091", "uki-sync current", "rollback 1234567"] {
            assert!(parse(&words(bad)).is_err(), "{bad}");
        }
        assert!(parse(&words("system-drive smb://nas/../etc")).is_err());
    }

    #[test]
    fn logind_drop_in() {
        let first = logind_text("", "HandleLidSwitch", "suspend");
        assert_eq!(first.lines().last(), Some("HandleLidSwitch=suspend"));
        let second = logind_text(&first, "HandlePowerKey", "poweroff");
        assert!(second.contains("HandleLidSwitch=suspend\nHandlePowerKey=poweroff\n"));
        let third = logind_text(&second, "HandleLidSwitch", "lock");
        assert!(third.contains("HandleLidSwitch=lock"));
        assert!(!third.contains("HandleLidSwitch=suspend"));
        // A setting not Orbit's, put there by hand, is not carried over.
        assert!(!logind_text("[Login]\nKillUserProcesses=yes\n", "HandlePowerKey", "ignore").contains("Kill"));
    }
}
