//! Who may answer: polkit sends identities — users (`unix-user`, by uid)
//! and groups (`unix-group`, by gid: the `sudo` group of Debian's admin
//! rule) — and the agent turns them into login names, read from
//! `/etc/passwd` and `/etc/group`.

use std::collections::HashMap;

use gtk::glib;

/// A login name and its full name, if the account has one.
#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub name: String,
    pub real_name: Option<String>,
}

/// The users behind `identities`, the one running the agent first if it is
/// among them (the usual case: an admin asked for their own password),
/// then in polkit's order, each once.
pub fn users(identities: &[(String, HashMap<String, glib::Variant>)]) -> Vec<User> {
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let group = std::fs::read_to_string("/etc/group").unwrap_or_default();
    let me = glib::user_name().to_string_lossy().into_owned();
    resolve(identities, &passwd, &group, &me)
}

fn resolve(identities: &[(String, HashMap<String, glib::Variant>)], passwd: &str, group: &str, me: &str) -> Vec<User> {
    // name, uid, gid, GECOS
    let accounts: Vec<(&str, u32, u32, &str)> = passwd
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            Some((*f.first()?, f.get(2)?.parse().ok()?, f.get(3)?.parse().ok()?, f.get(4).copied().unwrap_or("")))
        })
        .collect();
    let mut names: Vec<&str> = Vec::new();
    for (kind, details) in identities {
        match kind.as_str() {
            "unix-user" => {
                let Some(uid) = details.get("uid").and_then(|v| v.get::<u32>()) else { continue };
                names.extend(accounts.iter().filter(|a| a.1 == uid).map(|a| a.0));
            }
            "unix-group" => {
                let Some(gid) = details.get("gid").and_then(|v| v.get::<u32>()) else { continue };
                let members = group.lines().find_map(|line| {
                    let f: Vec<&str> = line.split(':').collect();
                    (f.get(2)?.parse::<u32>().ok()? == gid).then(|| f.get(3).copied().unwrap_or(""))
                });
                names.extend(members.into_iter().flat_map(|m| m.split(',')).filter(|m| !m.is_empty()));
                names.extend(accounts.iter().filter(|a| a.2 == gid).map(|a| a.0));
            }
            other => log::warn!("identity of an unknown kind: {other}"),
        }
    }
    let mut seen = Vec::new();
    for name in names {
        if !seen.contains(&name) {
            seen.push(name);
        }
    }
    if let Some(i) = seen.iter().position(|n| *n == me) {
        let mine = seen.remove(i);
        seen.insert(0, mine);
    }
    seen.into_iter()
        .map(|name| {
            let gecos = accounts.iter().find(|a| a.0 == name).map(|a| a.3).unwrap_or("");
            let real = gecos.split(',').next().unwrap_or("").trim();
            User { name: name.to_owned(), real_name: (!real.is_empty() && real != name).then(|| real.to_owned()) }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::prelude::ToVariant;

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\njuliano:x:1000:1000:Juliano,,,:/home/juliano:/bin/bash\nana:x:1001:1001::/home/ana:/bin/bash\n";
    const GROUP: &str = "root:x:0:\nsudo:x:27:ana,juliano\n";

    fn identity(kind: &str, key: &str, id: u32) -> (String, HashMap<String, glib::Variant>) {
        (kind.into(), HashMap::from([(key.to_owned(), id.to_variant())]))
    }

    #[test]
    fn the_sudo_group_gives_its_members_me_first() {
        let users = resolve(&[identity("unix-group", "gid", 27)], PASSWD, GROUP, "juliano");
        let names: Vec<&str> = users.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["juliano", "ana"]);
        assert_eq!(users[0].real_name.as_deref(), Some("Juliano"));
        assert_eq!(users[1].real_name, None);
    }

    #[test]
    fn a_user_by_uid_and_no_repeats() {
        let ids = [identity("unix-user", "uid", 0), identity("unix-group", "gid", 0)];
        let users = resolve(&ids, PASSWD, GROUP, "juliano");
        assert_eq!(users.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(), ["root"]);
    }
}
