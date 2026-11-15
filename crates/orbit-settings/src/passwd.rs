//! One's own password, changed by `passwd` on a pseudo-terminal, as typed
//! in a terminal: through PAM, so its rules (pwquality) apply and the
//! encrypted `/home`'s keyslot follows (orbit-login --home-key). Its
//! prompts are read in English (`LC_ALL=C`).

use std::{
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::process::CommandExt,
    },
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// The current password was not the one.
    Wrong,
    /// `passwd` said why (a password too short, too simple…).
    Said(String),
}

/// What `passwd` printed besides its prompts, last line last.
fn said(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.to_lowercase().ends_with("password:") && !l.starts_with("Changing password for"))
        .map(str::to_owned)
        .collect()
}

/// Why it failed, from what it printed and how many prompts it answered.
fn failure(output: &str, answered: usize) -> Failure {
    let lines = said(output);
    if let Some(bad) = lines.iter().find(|l| l.starts_with("BAD PASSWORD")) {
        return Failure::Said(bad.trim_start_matches("BAD PASSWORD:").trim().to_owned());
    }
    if answered <= 1 && lines.iter().any(|l| l.contains("Authentication token manipulation error") || l.contains("Authentication failure")) {
        return Failure::Wrong;
    }
    Failure::Said(lines.last().cloned().unwrap_or_else(|| "passwd failed".into()))
}

/// Changes the password from `current` to `new`.
pub fn change(current: &str, new: &str) -> Result<(), Failure> {
    let io = |e: std::io::Error| Failure::Said(e.to_string());
    let (mut master, mut slave) = (-1, -1);
    // SAFETY: two out-pointers to ints; no name, settings or size given.
    if unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), std::ptr::null()) } != 0 {
        return Err(io(std::io::Error::last_os_error()));
    }
    // SAFETY: openpty gave both, open, and nothing else owns them.
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    let mut command = Command::new("passwd");
    command
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::from(slave.try_clone().map_err(io)?))
        .stdout(Stdio::from(slave.try_clone().map_err(io)?))
        .stderr(Stdio::from(slave));
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(io)?;
    // The terminal's other end closes with passwd, once ours are gone.
    drop(command);
    let mut terminal = File::from(master);
    let deadline = Instant::now() + Duration::from_secs(30);
    let (mut output, mut pending) = (String::new(), String::new());
    let mut answered = 0;
    loop {
        if Instant::now() > deadline || answered > 3 {
            let _ = child.kill();
            break;
        }
        let mut poll = libc::pollfd { fd: terminal.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: one pollfd, ours.
        if unsafe { libc::poll(&mut poll, 1, 500) } <= 0 {
            continue;
        }
        let mut buf = [0u8; 512];
        match terminal.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let text = String::from_utf8_lossy(&buf[..n]);
                output.push_str(&text);
                pending.push_str(&text);
            }
        }
        let prompt = pending.trim_end().to_lowercase();
        if prompt.ends_with("password:") {
            let answer = if prompt.contains("current") { current } else { new };
            let _ = terminal.write_all(format!("{answer}\n").as_bytes());
            answered += 1;
            pending.clear();
        }
    }
    let status = child.wait().map_err(io)?;
    if status.success() { Ok(()) } else { Err(failure(&output, answered)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wrong current password changes nothing and is told as such
    /// (runs the real passwd: `cargo test -- --ignored`).
    #[test]
    #[ignore]
    fn a_wrong_current_password_is_refused() {
        assert_eq!(change("not-the-password-0", "Another-one-123!"), Err(Failure::Wrong));
    }

    #[test]
    fn failures_are_told_apart() {
        let wrong = "Changing password for ana.\nCurrent password: \npasswd: Authentication token manipulation error\npasswd: password unchanged\n";
        assert_eq!(failure(wrong, 1), Failure::Wrong);
        let short = "Current password: \nNew password: \nBAD PASSWORD: The password is shorter than 8 characters\nNew password: ";
        assert_eq!(failure(short, 4), Failure::Said("The password is shorter than 8 characters".into()));
        let other = "Current password: \nNew password: \nRetype new password: \nSorry, passwords do not match.\npasswd: Authentication token manipulation error\npasswd: password unchanged\n";
        assert_eq!(failure(other, 3), Failure::Said("passwd: password unchanged".into()));
    }
}
