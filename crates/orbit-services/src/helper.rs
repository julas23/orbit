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
//! off|security|all` (unattended-upgrades, installed when turned on). Every step is checked against the catalogue before any is
//! run; a step that fails undoes the unit steps already done. With
//! `--trial`, the unit steps done are undone after SECONDS unless a line
//! `keep` comes on the standard input first (`undo` undoes them at once).
//!
//! It says what it does on the standard output, a line each: `step TEXT`,
//! `progress N TEXT` (apt's, 0–100), `trial SECONDS`, `kept`, `undone`,
//! `error TEXT`, `done`.

use std::{
    io::{BufRead, BufReader},
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
