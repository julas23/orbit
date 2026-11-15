//! orbit-services-helper: the steps of Orbit's services that need root,
//! run by `pkexec` for Orbit-Settings and `orbitctl services`.
//!
//!   orbit-services-helper [--trial SECONDS] STEP ARG [-- STEP ARG]…
//!
//! STEP is `install` or `remove` (a package of the catalogue), or `unit-on`
//! or `unit-off` (one of its units: enabled and started, or stopped and
//! disabled). Every step is checked against the catalogue before any is
//! run; a step that fails undoes the unit steps already done. With
//! `--trial`, the unit steps done are undone after SECONDS unless a line
//! `keep` comes on the standard input first.
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
            [verb, arg] => steps.push(Privileged::parse(verb, arg)?),
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
    }
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

/// Whether `keep` comes on the standard input within `seconds`.
fn kept(seconds: u32) -> bool {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            match line {
                Ok(l) if l.trim() == "keep" => {
                    let _ = tx.send(());
                    return;
                }
                Ok(_) => continue,
                Err(_) => return,
            }
        }
    });
    rx.recv_timeout(Duration::from_secs(seconds.into())).is_ok()
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
    }
}
