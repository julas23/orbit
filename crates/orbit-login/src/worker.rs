//! The two jobs the service hands to programs of their own — itself, run
//! again — because PAM's modules fork and allocate, which a process with
//! threads should not do after a fork:
//!
//!   orbit-login --session USER VT COMMAND
//!   orbit-login --check USER          (the password on stdin)
//!
//! `--session` opens USER's session (PAM service `orbit-login`, no
//! password: the session starts locked) on console VT, says
//! `ready SESSION_ID PID` on stdout, runs COMMAND as USER through their
//! login shell, and closes the session when it ends, exiting as it did.
//! This process is the session's leader for logind.
//!
//! `--check` checks USER's password (PAM service `orbit-greet`, the one
//! the greet uses for its own user; its `pam_gnome_keyring` unlocks the
//! login keyring of USER's session, already open) and says why on stdout
//! if it is refused.

use std::{
    ffi::CString,
    io::{Read, Write},
};

use crate::pam::Pam;

pub fn session(user: &str, vt: u32, command: &str) -> i32 {
    let Some(account) = orbit_login::user(user) else {
        log::error!("{user}: no such user");
        return 1;
    };
    let mut pam = match Pam::start(c"orbit-login", user, None) {
        Ok(pam) => pam,
        Err(err) => {
            log::error!("{user}: {err}");
            return 1;
        }
    };
    pam.set_tty(&format!("tty{vt}"));
    for (name, value) in [
        ("XDG_SEAT", "seat0"),
        ("XDG_VTNR", &vt.to_string()),
        ("XDG_SESSION_TYPE", "wayland"),
        ("XDG_SESSION_CLASS", "user"),
        ("XDG_SESSION_DESKTOP", "orbit"),
    ] {
        pam.putenv(name, value);
    }
    if let Err(err) = pam.account().and_then(|()| pam.open_session()) {
        log::error!("{user}: the session did not open: {err}");
        return 1;
    }

    let mut env = pam.env();
    let mut set = |name: &str, value: &str| {
        if !env.iter().any(|(n, _)| n == name) {
            env.push((name.to_owned(), value.to_owned()));
        }
    };
    set("HOME", &account.home.to_string_lossy());
    set("USER", user);
    set("LOGNAME", user);
    set("SHELL", &account.shell);
    set("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/games");
    set("XDG_CURRENT_DESKTOP", "orbit");
    let id = env.iter().find(|(n, _)| n == "XDG_SESSION_ID").map(|(_, v)| v.clone());

    // Everything the child needs, made before the fork: after it, only
    // system calls.
    let cstr = |s: &str| CString::new(s).unwrap_or_default();
    let shell = cstr(&account.shell);
    let line = format!("exec {command}");
    let argv_owned = [cstr(&account.shell), cstr("-l"), cstr("-c"), cstr(&line)];
    let mut argv: Vec<*const libc::c_char> = argv_owned.iter().map(|s| s.as_ptr()).collect();
    argv.push(std::ptr::null());
    let env_owned: Vec<CString> = env.iter().map(|(n, v)| cstr(&format!("{n}={v}"))).collect();
    let mut envp: Vec<*const libc::c_char> = env_owned.iter().map(|s| s.as_ptr()).collect();
    envp.push(std::ptr::null());
    let name = cstr(user);
    let home = CString::new(account.home.as_os_str().as_encoded_bytes()).unwrap_or_default();
    let null = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDWR) };

    // Safety: the child only makes system calls on memory made above,
    // then execs or exits.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        log::error!("{user}: fork: {}", std::io::Error::last_os_error());
        return 1;
    }
    if pid == 0 {
        unsafe {
            libc::setsid();
            if null >= 0 {
                libc::dup2(null, 0);
                libc::dup2(null, 1);
            }
            if libc::initgroups(name.as_ptr(), account.gid) != 0 || libc::setgid(account.gid) != 0 || libc::setuid(account.uid) != 0 || libc::getuid() != account.uid {
                libc::_exit(126);
            }
            if libc::chdir(home.as_ptr()) != 0 {
                libc::chdir(c"/".as_ptr());
            }
            libc::execve(shell.as_ptr(), argv.as_ptr(), envp.as_ptr());
            libc::_exit(127);
        }
    }

    log::info!("{user}: session {} on tty{vt}, {command:?} as pid {pid}", id.as_deref().unwrap_or("?"));
    let mut out = std::io::stdout();
    let _ = writeln!(out, "ready {} {pid}", id.as_deref().unwrap_or("-"));
    let _ = out.flush();

    let mut status = 0;
    loop {
        // Safety: waiting for the child forked above.
        let rc = unsafe { libc::waitpid(pid, &mut status, 0) };
        if rc == pid || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            break;
        }
    }
    let code = if libc::WIFEXITED(status) { libc::WEXITSTATUS(status) } else { 128 + libc::WTERMSIG(status) };
    log::info!("{user}: the session ended ({code})");
    drop(pam);
    code
}

pub fn check(user: &str) -> i32 {
    let mut password = String::new();
    if std::io::stdin().read_to_string(&mut password).is_err() {
        return 1;
    }
    if password.ends_with('\n') {
        password.pop();
    }
    let result = Pam::start(c"orbit-greet", user, Some(password)).and_then(|mut pam| {
        if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
            pam.putenv("XDG_RUNTIME_DIR", &dir.to_string_lossy());
        }
        pam.authenticate()
    });
    match result {
        Ok(()) => 0,
        Err(why) => {
            print!("{why}");
            1
        }
    }
}
