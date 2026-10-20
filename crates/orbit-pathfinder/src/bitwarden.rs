//! Bitwarden, through its official command line (`bw`): the vault is where
//! the passwords live; the browser keeps none of its own.
//!
//! The vault is unlocked with the master password, which gives a session
//! key; that key stays in memory only, for as long as the program runs, and
//! goes to each `bw` call in its environment. Nothing listens on a port
//! (`bw serve` would let any local program read the vault).
//!
//! Two uses: importing the passwords other browsers saved (only those the
//! vault does not have yet, into a folder per browser), and finding the
//! logins for a page to fill its form.

use std::{
    collections::HashSet,
    path::PathBuf,
    process::{Command, Stdio},
};

use gtk::glib;
use serde_json::{Value, json};

use crate::{import, privacy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// `bw` is not installed.
    Missing,
    /// Installed but never logged in (`bw login` in a terminal).
    LoggedOut,
    /// Logged in; unlocking takes the master password (an unlocked `bw`
    /// counts too: its key belongs to that terminal, not to this program).
    Locked,
}

/// A login of the vault, as the form filler needs it.
#[derive(Debug, Clone)]
pub struct Credential {
    pub name: String,
    pub username: String,
    pub password: String,
}

fn program() -> Option<PathBuf> {
    glib::find_program_in_path("bw").or_else(|| Some(glib::home_dir().join(".local/bin/bw")).filter(|p| p.is_file()))
}

/// Runs `bw` with `args`, the session key in its environment; its output.
fn run(args: &[&str], session: Option<&str>, env: &[(&str, &str)]) -> Result<String, String> {
    let bw = program().ok_or("bw não está instalado")?;
    let mut command = Command::new(bw);
    command.args(args).env("BW_NOINTERACTION", "true").stdin(Stdio::null());
    if let Some(key) = session {
        command.env("BW_SESSION", key);
    }
    for (k, v) in env {
        command.env(k, v);
    }
    let out = command.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        Err(if err.is_empty() { String::from_utf8_lossy(&out.stdout).trim().to_owned() } else { err })
    }
}

pub fn status() -> Status {
    if program().is_none() {
        return Status::Missing;
    }
    let text = run(&["status"], None, &[]).unwrap_or_default();
    let status = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("status").and_then(|s| s.as_str()).map(str::to_owned));
    match status.as_deref() {
        Some("unlocked") | Some("locked") => Status::Locked,
        _ => Status::LoggedOut,
    }
}

/// Unlocks the vault: the session key.
pub fn unlock(master: &str) -> Result<String, String> {
    // The password goes in the environment of that one call, never on its
    // command line (which other users can read).
    let key = run(&["unlock", "--raw", "--passwordenv", "ORBIT_BW_PASSWORD"], None, &[("ORBIT_BW_PASSWORD", master)])?;
    let key = key.trim().to_owned();
    if key.is_empty() { Err("senha mestra recusada".into()) } else { Ok(key) }
}

/// What makes two logins the same: site, user and password.
fn same(url: &str, username: &str, password: &str) -> (String, String, String) {
    let site = privacy::host(url).map(|h| privacy::site(&h)).unwrap_or_else(|| url.to_owned());
    (site, username.to_owned(), password.to_owned())
}

/// The logins already in the vault.
fn existing(session: &str) -> Result<HashSet<(String, String, String)>, String> {
    let items: Value = serde_json::from_str(&run(&["list", "items"], Some(session), &[])?).map_err(|e| e.to_string())?;
    let mut set = HashSet::new();
    for item in items.as_array().into_iter().flatten() {
        let Some(login) = item.get("login") else { continue };
        let username = login.get("username").and_then(Value::as_str).unwrap_or_default();
        let password = login.get("password").and_then(Value::as_str).unwrap_or_default();
        for uri in login.get("uris").and_then(Value::as_array).into_iter().flatten() {
            if let Some(u) = uri.get("uri").and_then(Value::as_str) {
                set.insert(same(u, username, password));
            }
        }
    }
    Ok(set)
}

/// How an import went.
#[derive(Debug, Default)]
pub struct Imported {
    pub added: usize,
    pub already: usize,
}

/// Sends `logins` to the vault — those it lacks, each browser's into a
/// folder "Importado do <browser>". The file `bw import` reads is written
/// to a private folder in memory and removed straight after.
pub fn import(session: &str, logins: &[import::Login]) -> Result<Imported, String> {
    run(&["sync"], Some(session), &[])?;
    let mut known = existing(session)?;
    let mut result = Imported::default();
    let mut folders: Vec<(import::Source, String)> = Vec::new();
    let mut items = Vec::new();
    for login in logins {
        if !known.insert(same(&login.url, &login.username, &login.password)) {
            result.already += 1;
            continue;
        }
        let folder = match folders.iter().find(|(s, _)| *s == login.source) {
            Some((_, id)) => id.clone(),
            None => {
                let id = glib::uuid_string_random().to_string();
                folders.push((login.source, id.clone()));
                id
            }
        };
        let name = privacy::host(&login.url).map(|h| h.trim_start_matches("www.").to_owned()).unwrap_or_else(|| login.url.clone());
        items.push(json!({
            "type": 1,
            "name": name,
            "folderId": folder,
            "favorite": false,
            "notes": null,
            "login": {
                "uris": [{ "match": null, "uri": login.url }],
                "username": login.username,
                "password": login.password,
                "totp": null,
            },
        }));
    }
    if items.is_empty() {
        return Ok(result);
    }
    result.added = items.len();
    let doc = json!({
        "encrypted": false,
        "folders": folders.iter().map(|(s, id)| json!({ "id": id, "name": format!("Importado do {}", s.label()) })).collect::<Vec<_>>(),
        "items": items,
    });
    let file = import::private_dir().map_err(|e| e.to_string())?.join(format!("bw-{}.json", glib::uuid_string_random()));
    let written = write_private(&file, &doc.to_string());
    let outcome = written.and_then(|()| run(&["import", "bitwardenjson", &file.to_string_lossy()], Some(session), &[]));
    let _ = std::fs::remove_file(&file);
    outcome?;
    let _ = run(&["sync"], Some(session), &[]);
    Ok(result)
}

fn write_private(file: &std::path::Path, text: &str) -> Result<(), String> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(file).map_err(|e| e.to_string())?;
    f.write_all(text.as_bytes()).map_err(|e| e.to_string())
}

/// The vault's logins for `url`.
pub fn find(session: &str, url: &str) -> Result<Vec<Credential>, String> {
    let items: Value = serde_json::from_str(&run(&["list", "items", "--url", url], Some(session), &[])?).map_err(|e| e.to_string())?;
    Ok(items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let login = item.get("login")?;
            Some(Credential {
                name: item.get("name").and_then(Value::as_str).unwrap_or_default().to_owned(),
                username: login.get("username").and_then(Value::as_str).unwrap_or_default().to_owned(),
                password: login.get("password").and_then(Value::as_str)?.to_owned(),
            })
        })
        .collect())
}
