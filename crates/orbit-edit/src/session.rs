//! The session, kept across runs in `~/.local/state/orbit/edit/`: the
//! folder in the sidebar, the tabs in order with their cursors, the tab
//! shown — and, for every tab with unsaved changes, its text, so closing
//! the editor never asks and never loses anything.

use std::path::PathBuf;

use gtk::glib;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub root: Option<PathBuf>,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub next_id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Tab {
    pub id: u64,
    pub path: Option<PathBuf>,
    /// Its text is in `unsaved/<id>.txt`.
    pub unsaved: bool,
    pub cursor: i32,
}

/// `$XDG_STATE_HOME/orbit/edit`, `~/.local/state/orbit/edit` by default.
fn dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local").join("state"))
        .join("orbit")
        .join("edit")
}

fn unsaved_dir() -> PathBuf {
    dir().join("unsaved")
}

pub fn load() -> Session {
    std::fs::read_to_string(dir().join("session.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// The unsaved text of tab `id`.
pub fn unsaved_text(id: u64) -> Option<String> {
    std::fs::read_to_string(unsaved_dir().join(format!("{id}.txt"))).ok()
}

/// Writes `session`, with `texts` — (id, text) of the tabs with unsaved
/// changes; other tabs' old copies go.
pub fn save(session: &Session, texts: &[(u64, String)]) -> std::io::Result<()> {
    let unsaved = unsaved_dir();
    std::fs::create_dir_all(&unsaved)?;
    for (id, text) in texts {
        write(&unsaved.join(format!("{id}.txt")), text.as_bytes())?;
    }
    if let Ok(entries) = std::fs::read_dir(&unsaved) {
        for entry in entries.filter_map(Result::ok) {
            let keep = entry
                .path()
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u64>().ok())
                .is_some_and(|id| texts.iter().any(|(t, _)| *t == id));
            if !keep {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    write(
        &dir().join("session.json"),
        serde_json::to_string_pretty(session)?.as_bytes(),
    )
}

/// Into a file beside it, then over it: never half written.
fn write(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}
