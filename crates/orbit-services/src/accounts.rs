//! This machine's people and groups (Orbit-Settings' Users and groups
//! page, docs/PLANO-SETTINGS.md §3.3): what `/etc/passwd` and
//! `/etc/group` say, and the [`Step`]s that change them, run as root by
//! `orbit-accounts-helper` under `pkexec` (polkit asks for an
//! administrator's password, once for the whole list).
//!
//! A person's own name and picture are not here: AccountsService changes
//! those, without a password for one's own. One's own password is not
//! either: `passwd` changes it, through PAM, as typed in a terminal.

use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
};

/// The helper, run as root by `pkexec`.
pub const HELPER: &str = "/usr/libexec/orbit-accounts-helper";
/// The group whose members are administrators (sudo, polkit).
pub const ADMIN_GROUP: &str = "sudo";
/// Orbit-Login's configuration: `user:` is the session opened at boot.
pub const LOGIN_CONFIG: &str = "/etc/orbit/login.yaml";

/// An account a person logs in with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    /// The GECOS' first field: the full name, maybe empty.
    pub full_name: String,
    pub home: String,
}

impl Person {
    /// The name to show: the full one, or the login.
    pub fn shown(&self) -> &str {
        if self.full_name.is_empty() { &self.name } else { &self.full_name }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub gid: u32,
    pub members: Vec<String>,
}

/// A line of `/etc/passwd`, if it is a person's: a uid from 1000, not
/// `nobody`'s, with a shell to log in with.
fn person(line: &str) -> Option<Person> {
    let f: Vec<&str> = line.split(':').collect();
    let [name, _, uid, gid, gecos, home, shell] = f[..] else { return None };
    let uid: u32 = uid.parse().ok()?;
    if !(1000..60000).contains(&uid) || shell.ends_with("nologin") || shell.ends_with("false") || shell.is_empty() {
        return None;
    }
    Some(Person {
        name: name.to_owned(),
        uid,
        gid: gid.parse().ok()?,
        full_name: gecos.split(',').next().unwrap_or("").to_owned(),
        home: home.to_owned(),
    })
}

fn group(line: &str) -> Option<Group> {
    let f: Vec<&str> = line.split(':').collect();
    let [name, _, gid, members] = f[..] else { return None };
    Some(Group {
        name: name.to_owned(),
        gid: gid.parse().ok()?,
        members: members.split(',').filter(|m| !m.is_empty()).map(str::to_owned).collect(),
    })
}

/// The people of this machine, by uid.
pub fn people() -> Vec<Person> {
    let text = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let mut out: Vec<Person> = text.lines().filter_map(person).collect();
    out.sort_by_key(|p| p.uid);
    out
}

/// Every group of this machine.
pub fn groups() -> Vec<Group> {
    let text = std::fs::read_to_string("/etc/group").unwrap_or_default();
    text.lines().filter_map(group).collect()
}

/// Whether `user` is an administrator.
pub fn is_admin(user: &str) -> bool {
    groups().iter().any(|g| g.name == ADMIN_GROUP && g.members.iter().any(|m| m == user))
}

/// The groups worth showing: those a person is in, and those made for
/// people (a gid from 1000) that are not someone's own (a person's
/// primary group, of their name).
pub fn shown_groups() -> Vec<Group> {
    let people = people();
    let own = |g: &Group| people.iter().any(|p| p.gid == g.gid && p.name == g.name);
    let mut out: Vec<Group> = groups()
        .into_iter()
        .filter(|g| g.name != ADMIN_GROUP)
        .filter(|g| {
            !own(g) && (g.members.iter().any(|m| people.iter().any(|p| &p.name == m)) || (1000..60000).contains(&g.gid))
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Whether `name` is a group a person may make, or remove: one made for
/// people, not someone's own.
pub fn removable_group(name: &str) -> bool {
    let people = people();
    groups()
        .iter()
        .any(|g| g.name == name && (1000..60000).contains(&g.gid) && !people.iter().any(|p| p.gid == g.gid))
}

/// The user whose session opens at boot (`user:` of Orbit-Login's file).
pub fn boot_user() -> Option<String> {
    let text = std::fs::read_to_string(LOGIN_CONFIG).ok()?;
    boot_user_in(&text)
}

fn boot_user_in(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let rest = l.strip_prefix("user:")?;
        let value = rest.split('#').next()?.trim().trim_matches(['"', '\'']);
        (!value.is_empty()).then(|| value.to_owned())
    })
}

/// Orbit-Login's file with `user:` set to `user`, the rest kept.
pub fn with_boot_user(text: &str, user: &str) -> String {
    let mut found = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|l| {
            if !found && l.starts_with("user:") {
                found = true;
                format!("user: {user}")
            } else {
                l.to_owned()
            }
        })
        .collect();
    if !found {
        out.push(format!("user: {user}"));
    }
    out.join("\n") + "\n"
}

/// A login or group name as `adduser` takes it.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 32
        && chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// A full name GECOS can hold.
pub fn valid_full_name(name: &str) -> bool {
    name.chars().count() <= 64 && !name.chars().any(|c| matches!(c, ':' | ',' | '=' | '\\') || c.is_control())
}

/// A login made from a full name: "José da Silva" → "jose".
pub fn login_from(full_name: &str) -> String {
    let first = full_name.split_whitespace().next().unwrap_or("");
    let mut out = String::new();
    for c in first.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            c => c,
        };
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
            out.push(c);
        }
    }
    while out.starts_with(|c: char| c.is_ascii_digit() || c == '-') {
        out.remove(0);
    }
    out.truncate(32);
    out
}

/// A step the helper takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// A new person: login, full name, administrator or not.
    UserAdd(String, String, bool),
    /// A person removed: their files too, or kept.
    UserDel(String, bool),
    /// A person's password, from the helper's standard input (one line).
    Password(String),
    GroupAdd(String),
    GroupDel(String),
    GroupJoin(String, String),
    GroupLeave(String, String),
    /// The person whose session opens at boot.
    BootUser(String),
}

impl Step {
    /// Its words on the helper's command line.
    pub fn args(&self) -> Vec<String> {
        let yes = |b: &bool| if *b { "yes" } else { "no" }.to_owned();
        match self {
            Step::UserAdd(n, f, a) => vec!["user-add".into(), n.clone(), f.clone(), yes(a)],
            Step::UserDel(n, files) => vec!["user-del".into(), n.clone(), yes(files)],
            Step::Password(n) => vec!["password".into(), n.clone()],
            Step::GroupAdd(g) => vec!["group-add".into(), g.clone()],
            Step::GroupDel(g) => vec!["group-del".into(), g.clone()],
            Step::GroupJoin(u, g) => vec!["group-join".into(), u.clone(), g.clone()],
            Step::GroupLeave(u, g) => vec!["group-leave".into(), u.clone(), g.clone()],
            Step::BootUser(u) => vec!["boot-user".into(), u.clone()],
        }
    }

    /// The step from its words, its names checked.
    pub fn parse(words: &[String]) -> Result<Step, String> {
        let name = |n: &String| if valid_name(n) { Ok(n.clone()) } else { Err(format!("not a valid name: {n:?}")) };
        let yes = |w: &String| match w.as_str() {
            "yes" => Ok(true),
            "no" => Ok(false),
            _ => Err(format!("yes or no, not {w:?}")),
        };
        match words {
            [v, n, f, a] if v == "user-add" => {
                if !valid_full_name(f) {
                    return Err(format!("not a valid full name: {f:?}"));
                }
                Ok(Step::UserAdd(name(n)?, f.clone(), yes(a)?))
            }
            [v, n, files] if v == "user-del" => Ok(Step::UserDel(name(n)?, yes(files)?)),
            [v, n] if v == "password" => Ok(Step::Password(name(n)?)),
            [v, g] if v == "group-add" => Ok(Step::GroupAdd(name(g)?)),
            [v, g] if v == "group-del" => Ok(Step::GroupDel(name(g)?)),
            [v, u, g] if v == "group-join" => Ok(Step::GroupJoin(name(u)?, name(g)?)),
            [v, u, g] if v == "group-leave" => Ok(Step::GroupLeave(name(u)?, name(g)?)),
            [v, u] if v == "boot-user" => Ok(Step::BootUser(name(u)?)),
            _ => Err(format!("unknown step: {words:?}")),
        }
    }
}

/// Runs `steps` with the helper (by `pkexec` unless root already), giving
/// it `secret` (a password) on its standard input; waits for it. The
/// error is the helper's, or says the password was refused.
pub fn run(steps: &[Step], secret: Option<&str>) -> Result<(), String> {
    // SAFETY: getuid has no preconditions and cannot fail.
    let mut command = if unsafe { libc::getuid() } == 0 {
        Command::new(HELPER)
    } else {
        let mut c = Command::new("pkexec");
        c.arg(HELPER);
        c
    };
    for (i, step) in steps.iter().enumerate() {
        if i > 0 {
            command.arg("--");
        }
        command.args(step.args());
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("pkexec: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{}", secret.unwrap_or(""));
    }
    let mut error = None;
    for line in BufReader::new(child.stdout.take().expect("piped")).lines().map_while(Result::ok) {
        if let Some(e) = line.strip_prefix("error ") {
            error = Some(e.to_owned());
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    match (status.code(), error) {
        (_, Some(e)) => Err(e),
        (Some(0), None) => Ok(()),
        (Some(126), None) => Err("not authorised: the password was refused or not given".into()),
        (Some(127), None) => Err("not authorised".into()),
        _ => Err("orbit-accounts-helper failed".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn people_and_groups_are_read() {
        let p = person("juliano:x:1000:1000:Juliano Alves,,,:/home/juliano:/bin/bash").unwrap();
        assert_eq!(p.shown(), "Juliano Alves");
        assert!(person("nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin").is_none());
        assert!(person("daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin").is_none());
        let g = group("sudo:x:27:juliano,merilin").unwrap();
        assert_eq!(g.members, vec!["juliano", "merilin"]);
        assert!(group("audio:x:29:").unwrap().members.is_empty());
    }

    #[test]
    fn names_are_checked() {
        assert!(valid_name("merilin"));
        assert!(valid_name("_x-1"));
        assert!(!valid_name("Merilin"));
        assert!(!valid_name("1abc"));
        assert!(!valid_name("a b"));
        assert!(!valid_name(""));
        assert!(valid_full_name("José da Silva"));
        assert!(!valid_full_name("a:b"));
        assert!(!valid_full_name("a\nb"));
        assert_eq!(login_from("José da Silva"), "jose");
        assert_eq!(login_from("  Ana-Clara  Souza"), "ana-clara");
        assert_eq!(login_from("1Zé"), "ze");
    }

    #[test]
    fn steps_round_trip() {
        for step in [
            Step::UserAdd("ana".into(), "Ana Souza".into(), true),
            Step::UserDel("ana".into(), false),
            Step::Password("ana".into()),
            Step::GroupAdd("family".into()),
            Step::GroupDel("family".into()),
            Step::GroupJoin("ana".into(), "family".into()),
            Step::GroupLeave("ana".into(), "family".into()),
            Step::BootUser("ana".into()),
        ] {
            assert_eq!(Step::parse(&step.args()).unwrap(), step);
        }
        assert!(Step::parse(&w(&["user-add", "ana", "A:B", "no"])).is_err());
        assert!(Step::parse(&w(&["password", "../etc"])).is_err());
        assert!(Step::parse(&w(&["group-join", "ana"])).is_err());
        assert!(Step::parse(&w(&["rm", "-rf"])).is_err());
    }

    #[test]
    fn boot_user_is_rewritten_in_place() {
        let text = "# Orbit-Login\nuser: juliano   # at boot\ncommand: orbit-core --locked\nvt: 1\n";
        assert_eq!(boot_user_in(text).as_deref(), Some("juliano"));
        let new = with_boot_user(text, "ana");
        assert_eq!(new, "# Orbit-Login\nuser: ana\ncommand: orbit-core --locked\nvt: 1\n");
        assert_eq!(boot_user_in(&with_boot_user("vt: 1\n", "ana")).as_deref(), Some("ana"));
    }
}
