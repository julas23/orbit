//! orbit-accounts-helper: the changes to this machine's people and groups
//! that need root, run by `pkexec` for Orbit-Settings' Users and groups
//! page (orbit_services::accounts).
//!
//!   orbit-accounts-helper STEP ARG… [-- STEP ARG…]…
//!
//!   user-add NAME "FULL NAME" yes|no   a person (an administrator or not)
//!   user-del NAME yes|no               a person removed (their files too?)
//!   password NAME                      their password: a line of stdin
//!   group-add NAME | group-del NAME    a group made for people
//!   group-join USER GROUP | group-leave USER GROUP
//!   boot-user NAME                     whose session opens at boot
//!
//! Every step is checked before any is run, and none undoes what the one
//! who asked needs: not removing oneself, nor the person opened at boot,
//! nor the last administrator; no group but one made for people removed.
//! It says what it does on the standard output: `step TEXT`, `error
//! TEXT`, `done`.

use std::{
    io::{BufRead, Write},
    process::{Command, ExitCode, Stdio},
};

use orbit_services::accounts::{self, ADMIN_GROUP, LOGIN_CONFIG, Step};

const PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

fn main() -> ExitCode {
    // SAFETY: geteuid has no preconditions and cannot fail.
    if unsafe { libc::geteuid() } != 0 {
        println!("error orbit-accounts-helper runs as root, by pkexec");
        return ExitCode::from(2);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let steps = match parse(&args).and_then(|s| check(&s).map(|()| s)) {
        Ok(steps) => steps,
        Err(e) => {
            println!("error {e}");
            return ExitCode::from(2);
        }
    };
    for step in &steps {
        println!("step {}", describe(step));
        if let Err(e) = run(step) {
            println!("error {e}");
            return ExitCode::FAILURE;
        }
    }
    println!("done");
    ExitCode::SUCCESS
}

fn parse(args: &[String]) -> Result<Vec<Step>, String> {
    let steps = args.split(|a| a == "--").map(Step::parse).collect::<Result<Vec<_>, _>>()?;
    if steps.is_empty() || steps.len() > 16 {
        return Err("from one to sixteen steps".into());
    }
    if steps.iter().filter(|s| matches!(s, Step::Password(_))).count() > 1 {
        return Err("one password at a time".into());
    }
    Ok(steps)
}

/// The one who asked (pkexec's caller), if a person.
fn caller() -> Option<String> {
    let uid: u32 = std::env::var("PKEXEC_UID").ok()?.parse().ok()?;
    accounts::people().into_iter().find(|p| p.uid == uid).map(|p| p.name)
}

/// The steps against this machine as it is (and as the earlier steps
/// leave it, for the people they add).
fn check(steps: &[Step]) -> Result<(), String> {
    let people = accounts::people();
    let groups = accounts::groups();
    let added: Vec<&String> = steps
        .iter()
        .filter_map(|s| if let Step::UserAdd(n, _, _) = s { Some(n) } else { None })
        .collect();
    let person = |n: &String| {
        if people.iter().any(|p| &p.name == n) || added.contains(&n) {
            Ok(())
        } else {
            Err(format!("not a person of this machine: {n}"))
        }
    };
    let group = |g: &String| {
        if groups.iter().any(|x| &x.name == g) || steps.contains(&Step::GroupAdd(g.clone())) {
            Ok(())
        } else {
            Err(format!("no such group: {g}"))
        }
    };
    let me = caller();
    let boot = accounts::boot_user();
    let mut admins: Vec<String> = groups
        .iter()
        .find(|g| g.name == ADMIN_GROUP)
        .map(|g| g.members.clone())
        .unwrap_or_default();
    for step in steps {
        match step {
            Step::UserAdd(n, _, admin) => {
                if people.iter().any(|p| &p.name == n) || groups.iter().any(|g| &g.name == n) {
                    return Err(format!("{n} is taken"));
                }
                if *admin {
                    admins.push(n.clone());
                }
            }
            Step::UserDel(n, _) => {
                person(n)?;
                if me.as_ref() == Some(n) {
                    return Err("one does not remove oneself".into());
                }
                if boot.as_ref() == Some(n) {
                    return Err(format!("{n}'s session opens at boot: choose another one first"));
                }
                admins.retain(|a| a != n);
            }
            Step::Password(n) => person(n)?,
            Step::GroupAdd(g) => {
                if groups.iter().any(|x| &x.name == g) {
                    return Err(format!("the group {g} is there already"));
                }
            }
            Step::GroupDel(g) => {
                if !accounts::removable_group(g) {
                    return Err(format!("{g} is not a group made for people"));
                }
            }
            Step::GroupJoin(u, g) => {
                person(u)?;
                group(g)?;
                if g == "root" {
                    return Err("not the root group".into());
                }
                if g == ADMIN_GROUP {
                    admins.push(u.clone());
                }
            }
            Step::GroupLeave(u, g) => {
                person(u)?;
                group(g)?;
                if g == ADMIN_GROUP {
                    admins.retain(|a| a != u);
                }
            }
            Step::BootUser(u) => person(u)?,
        }
    }
    let had_admins = groups.iter().any(|g| g.name == ADMIN_GROUP && g.members.iter().any(|m| people.iter().any(|p| &p.name == m)));
    if had_admins && !admins.iter().any(|a| people.iter().any(|p| &p.name == a) || added.contains(&a)) {
        return Err("someone has to stay an administrator".into());
    }
    Ok(())
}

fn describe(step: &Step) -> String {
    match step {
        Step::UserAdd(n, _, _) => format!("Adding {n}"),
        Step::UserDel(n, _) => format!("Removing {n}"),
        Step::Password(n) => format!("Setting {n}'s password"),
        Step::GroupAdd(g) => format!("Adding the group {g}"),
        Step::GroupDel(g) => format!("Removing the group {g}"),
        Step::GroupJoin(u, g) => format!("Putting {u} in {g}"),
        Step::GroupLeave(u, g) => format!("Taking {u} out of {g}"),
        Step::BootUser(u) => format!("Opening {u}'s session at boot"),
    }
}

fn run(step: &Step) -> Result<(), String> {
    match step {
        Step::UserAdd(n, full, admin) => {
            tool("adduser", &["--disabled-password", "--comment", full, n], None)?;
            if *admin {
                tool("gpasswd", &["-a", n, ADMIN_GROUP], None)?;
            }
            Ok(())
        }
        Step::UserDel(n, files) => {
            if *files {
                tool("deluser", &["--remove-home", n], None)
            } else {
                tool("deluser", &[n], None)
            }
        }
        Step::Password(n) => {
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line).map_err(|e| e.to_string())?;
            let password = line.trim_end_matches(['\n', '\r']);
            if password.is_empty() {
                return Err("an empty password".into());
            }
            // chpasswd goes through PAM: the encrypted /home's keyslot
            // follows the password (orbit-login --home-key).
            tool("chpasswd", &[], Some(&format!("{n}:{password}\n")))
        }
        Step::GroupAdd(g) => tool("addgroup", &[g], None),
        Step::GroupDel(g) => tool("delgroup", &[g], None),
        Step::GroupJoin(u, g) => tool("gpasswd", &["-a", u, g], None),
        Step::GroupLeave(u, g) => tool("gpasswd", &["-d", u, g], None),
        Step::BootUser(u) => {
            let text = std::fs::read_to_string(LOGIN_CONFIG).map_err(|e| format!("{LOGIN_CONFIG}: {e}"))?;
            let temp = format!("{LOGIN_CONFIG}.new");
            std::fs::write(&temp, accounts::with_boot_user(&text, u))
                .and_then(|()| std::fs::rename(&temp, LOGIN_CONFIG))
                .map_err(|e| format!("{LOGIN_CONFIG}: {e}"))
        }
    }
}

/// Runs `program` with `args` (and `input` on its standard input); its
/// last line of errors when it fails.
fn tool(program: &str, args: &[&str], input: Option<&str>) -> Result<(), String> {
    let mut child = Command::new(program)
        .args(args)
        .env("PATH", PATH)
        .env("LC_ALL", "C.UTF-8")
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin.write_all(text.as_bytes()).map_err(|e| format!("{program}: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr)
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .map(|l| l.trim().to_owned())
            .unwrap_or_else(|| format!("{program} failed")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn steps_are_split_and_limited() {
        let steps = parse(&words(&["user-add", "ana", "Ana Souza", "no", "--", "password", "ana"])).unwrap();
        assert_eq!(steps.len(), 2);
        assert!(parse(&words(&["password", "ana", "--", "password", "bia"])).is_err());
        assert!(parse(&[]).is_err());
    }
}
