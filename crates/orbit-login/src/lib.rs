//! Orbit-Login's side for its clients (PLANO-CORE §7.4): the people who
//! can log in, and the requests the greet sends to the service on
//! `SOCKET` — one JSON line asked, one answered, per connection.
//!
//! The service is the only one that can open another user's session or
//! check another user's password; it answers only programs of the
//! sessions it opened.

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};

pub const SOCKET: &str = "/run/orbit/login.sock";

/// A switch waits for PAM (a wrong password, a couple of seconds) and
/// for the other session to open.
const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    /// Open `user`'s session behind this one, locked, so it is ready when
    /// the password is.
    Prepare { user: String },
    /// Check `user`'s password and, if right, show their session unlocked.
    Switch { user: String, password: String },
    /// Check the asker's own password and, if right, open a session of
    /// theirs running orbit-local-station (the remote one, pending §9) and
    /// show it; their local session stays locked behind.
    Station { password: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reply {
    Ok,
    /// The password was not accepted: PAM's words.
    Refused(String),
    Error(String),
}

/// Whether the service is there to ask (the system was started by it).
pub fn available() -> bool {
    Path::new(SOCKET).exists()
}

/// Asks the service; blocks until it answers.
pub fn call(request: &Request) -> std::io::Result<Reply> {
    let mut stream = UnixStream::connect(SOCKET)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    let mut line = serde_json::to_vec(request)?;
    line.push(b'\n');
    stream.write_all(&line)?;
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer)?;
    Ok(serde_json::from_str(&answer)?)
}

/// Someone who can log in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    /// The full name, if the account has one (GECOS's first part).
    pub real_name: Option<String>,
    pub home: PathBuf,
    pub shell: String,
}

impl User {
    /// The name to show.
    pub fn shown(&self) -> &str {
        self.real_name.as_deref().unwrap_or(&self.name)
    }
}

/// The people of `/etc/passwd`: the accounts of uid 1000 to 59999 with a
/// shell to log in with, in uid order.
pub fn users() -> Vec<User> {
    let text = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let mut users: Vec<User> = text.lines().filter_map(parse).collect();
    users.sort_by_key(|u| u.uid);
    users
}

pub fn user(name: &str) -> Option<User> {
    users().into_iter().find(|u| u.name == name)
}

pub fn user_by_uid(uid: u32) -> Option<User> {
    users().into_iter().find(|u| u.uid == uid)
}

fn parse(line: &str) -> Option<User> {
    let fields: Vec<&str> = line.split(':').collect();
    let [name, _, uid, gid, gecos, home, shell] = fields[..] else { return None };
    let uid: u32 = uid.parse().ok()?;
    if !(1000..60000).contains(&uid) || shell.ends_with("/nologin") || shell.ends_with("/false") || shell.is_empty() {
        return None;
    }
    let real = gecos.split(',').next().unwrap_or_default().trim();
    Some(User {
        name: name.to_owned(),
        uid,
        gid: gid.parse().ok()?,
        real_name: (!real.is_empty() && real != name).then(|| real.to_owned()),
        home: home.into(),
        shell: shell.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn people_are_told_from_system_accounts() {
        let juliano = parse("juliano:x:1000:1000:Juliano Alves dos Santos,,,:/home/juliano:/bin/bash").unwrap();
        assert_eq!(juliano.shown(), "Juliano Alves dos Santos");
        assert_eq!(parse("merilin:x:1001:1002::/home/merilin:/bin/bash").unwrap().shown(), "merilin");
        assert!(parse("daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin").is_none());
        assert!(parse("nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin").is_none());
        assert!(parse("ghost:x:1002:1002::/home/ghost:/bin/false").is_none());
    }

    #[test]
    fn requests_are_one_json_line() {
        let request = Request::Switch { user: "merilin".into(), password: "x".into() };
        let text = serde_json::to_string(&request).unwrap();
        assert!(!text.contains('\n'));
        assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), request);
        assert_eq!(serde_json::to_string(&Reply::Ok).unwrap(), "\"ok\"");
        let station = Request::Station { password: "x".into() };
        assert_eq!(serde_json::to_string(&station).unwrap(), r#"{"station":{"password":"x"}}"#);
    }
}
