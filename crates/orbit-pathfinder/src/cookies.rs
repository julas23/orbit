//! Staying logged in. Each service keeps its cookies in `cookies.sqlite`
//! in its own folder; WebKit writes there only when told to, so it is told
//! when the service's session is made.
//!
//! WebKit also drops the cookies that have no expiry date — "session"
//! cookies — when the program closes, and many sites keep the login in one
//! of those. Chromium (so Wavebox) keeps them across restarts, and so does
//! this: they are written to `session-cookies.json` in the service's folder
//! when the window closes and every few minutes (for a power cut or a
//! crash), and put back before the service's page loads. They go back as
//! session cookies, with the same host, path and flags.

use std::{cell::Cell, path::Path, rc::Rc};

use gtk::gio;
use serde::{Deserialize, Serialize};
use webkit6::soup;

const SESSION_FILE: &str = "session-cookies.json";

#[derive(Debug, Serialize, Deserialize)]
struct SessionCookie {
    name: String,
    value: String,
    domain: String,
    path: String,
    secure: bool,
    http_only: bool,
    /// "none", "lax" or "strict".
    same_site: String,
}

/// Where the service's cookies live from now on.
pub fn persist(manager: &webkit6::CookieManager, dir: &Path) {
    let file = dir.join("cookies.sqlite");
    manager.set_persistent_storage(&file.to_string_lossy(), webkit6::CookiePersistentStorage::Sqlite);
}

/// Writes the service's session cookies, then calls `done`.
pub fn save(manager: &webkit6::CookieManager, dir: &Path, done: impl FnOnce() + 'static) {
    let file = dir.join(SESSION_FILE);
    manager.all_cookies(gio::Cancellable::NONE, move |result| {
        match result {
            Ok(cookies) => {
                let session: Vec<SessionCookie> = cookies.into_iter().filter_map(to_saved).collect();
                if let Err(err) = write_private(&file, &session) {
                    log::warn!("{}: {err}", file.display());
                }
            }
            Err(err) => log::warn!("reading cookies: {err}"),
        }
        done();
    });
}

/// Puts the saved session cookies back, then calls `then` (which loads the
/// page — after, so the page is asked for with them).
pub fn restore(manager: &webkit6::CookieManager, dir: &Path, then: impl FnOnce() + 'static) {
    let saved: Vec<SessionCookie> = std::fs::read_to_string(dir.join(SESSION_FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    if saved.is_empty() {
        then();
        return;
    }
    let left = Rc::new(Cell::new(saved.len()));
    let then = Rc::new(Cell::new(Some(Box::new(then) as Box<dyn FnOnce()>)));
    for saved in saved {
        let cookie = from_saved(&saved);
        let (left, then) = (left.clone(), then.clone());
        manager.add_cookie(&cookie, gio::Cancellable::NONE, move |result| {
            if let Err(err) = result {
                log::debug!("cookie {}: {err}", saved.name);
            }
            left.set(left.get() - 1);
            if left.get() == 0
                && let Some(then) = then.take()
            {
                then();
            }
        });
    }
}

/// Drops the saved session cookies (clearing a service's data).
pub fn forget(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(SESSION_FILE));
}

fn to_saved(mut cookie: soup::Cookie) -> Option<SessionCookie> {
    if cookie.expires().is_some() {
        // Persistent: already in cookies.sqlite.
        return None;
    }
    Some(SessionCookie {
        name: cookie.name()?.to_string(),
        value: cookie.value().map(|v| v.to_string()).unwrap_or_default(),
        domain: cookie.domain()?.to_string(),
        path: cookie.path().map(|p| p.to_string()).unwrap_or_else(|| "/".into()),
        secure: cookie.is_secure(),
        http_only: cookie.is_http_only(),
        same_site: match cookie.same_site_policy() {
            soup::SameSitePolicy::Strict => "strict",
            soup::SameSitePolicy::None => "none",
            _ => "lax",
        }
        .into(),
    })
}

fn from_saved(saved: &SessionCookie) -> soup::Cookie {
    // Max age -1: a session cookie again.
    let mut cookie = soup::Cookie::new(&saved.name, &saved.value, &saved.domain, &saved.path, -1);
    cookie.set_secure(saved.secure);
    cookie.set_http_only(saved.http_only);
    cookie.set_same_site_policy(match saved.same_site.as_str() {
        "strict" => soup::SameSitePolicy::Strict,
        "none" => soup::SameSitePolicy::None,
        _ => soup::SameSitePolicy::Lax,
    });
    cookie
}

/// Logins are secrets: the file is readable by its owner only.
fn write_private(file: &Path, cookies: &[SessionCookie]) -> std::io::Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let text = serde_json::to_string(cookies).map_err(std::io::Error::other)?;
    let tmp = file.with_extension("json.tmp");
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    std::fs::rename(&tmp, file)
}

/// How often the session cookies are written while the window is open.
pub const SAVE_EVERY: std::time::Duration = std::time::Duration::from_secs(300);
