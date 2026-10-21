//! What the window remembers between runs, in
//! `~/.local/state/orbit/pathfinder/state.json`: the service last shown,
//! the groups folded away and each service's zoom.

use std::{collections::BTreeMap, path::PathBuf};

use gtk::glib;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub last: Option<String>,
    #[serde(default)]
    pub collapsed: Vec<String>,
    #[serde(default)]
    pub zoom: BTreeMap<String, f64>,
    /// The browser's open tabs, and which was shown.
    #[serde(default)]
    pub tabs: Vec<String>,
    #[serde(default)]
    pub tab: usize,
}

/// `$XDG_STATE_HOME/orbit/pathfinder`, `~/.local/state/orbit/pathfinder`
/// by default.
fn path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local").join("state"))
        .join("orbit")
        .join("pathfinder")
        .join("state.json")
}

impl State {
    pub fn load() -> Self {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(err) = std::fs::write(&path, text) {
                    log::warn!("{}: {err}", path.display());
                }
            }
            Err(err) => log::warn!("state: {err}"),
        }
    }
}
