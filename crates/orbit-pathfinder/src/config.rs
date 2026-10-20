//! `~/.config/orbit/pathfinder.yaml`: the services in the sidebar, in
//! groups, and how each one behaves.
//!
//! ```yaml
//! groups:
//!   - name: Comunicação
//!     services:
//!       - name: Gmail
//!         url: https://mail.google.com/
//!       - name: WhatsApp
//!         url: https://web.whatsapp.com/
//!         always_active: true
//!         media: true
//! ```
//!
//! The sidebar's "add a service" writes here too; the file is the one place
//! the services live.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub groups: Vec<Group>,
    /// The command that opens links leaving a service ("zen-browser %u");
    /// the system's default browser when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_browser: Option<String>,
    /// The user agent every service sends unless it sets its own; WebKit's
    /// own when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<Service>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub name: String,
    pub url: String,
    /// Names the service's own storage — cookies, logins, cache — so two
    /// accounts of one site stay apart. From the name when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// An icon file or icon-theme name; the site's own icon otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Loaded at start and kept loaded, so its notifications and unread
    /// count arrive; otherwise loaded on the first click.
    #[serde(default, skip_serializing_if = "is_false")]
    pub always_active: bool,
    /// Camera, microphone and calls (WebRTC).
    #[serde(default, skip_serializing_if = "is_false")]
    pub media: bool,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub notifications: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub muted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    /// "socks5://host:port" or "http://host:port", for this service only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    /// Other sites whose new windows open inside the service instead of the
    /// external browser (its login pages, its file previews).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<String>,
}

fn yes() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Service {
    pub fn new(name: &str, url: &str) -> Self {
        Self {
            name: name.to_owned(),
            url: url.to_owned(),
            id: None,
            icon: None,
            always_active: false,
            media: false,
            notifications: true,
            muted: false,
            user_agent: None,
            proxy: None,
            domains: Vec::new(),
        }
    }

    /// The name of its storage: `id`, or the name made safe for a folder.
    pub fn key(&self) -> String {
        slug(self.id.as_deref().unwrap_or(&self.name))
    }
}

/// "Gmail Trabalho" → "gmail-trabalho".
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() { "service".into() } else { out }
}

pub fn path(dir: &orbit_common::ConfigDir) -> PathBuf {
    dir.path().join("pathfinder.yaml")
}

/// Writes the config back (after "add a service" or a change from the
/// sidebar). Comments in the file are not kept.
pub fn save(path: &Path, config: &Config) -> anyhow::Result<()> {
    let text = serde_yaml_ng::to_string(config)?;
    let tmp = path.with_extension("yaml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_names() {
        assert_eq!(slug("Gmail Trabalho"), "gmail-trabalho");
        assert_eq!(slug("Comunicação"), "comunicacao");
        assert_eq!(slug("  !! "), "service");
    }

    #[test]
    fn reads_and_writes_back() {
        let text = "groups:\n  - name: A\n    services:\n      - name: WhatsApp\n        url: https://web.whatsapp.com/\n        always_active: true\n";
        let config: Config = serde_yaml_ng::from_str(text).unwrap();
        let s = &config.groups[0].services[0];
        assert!(s.always_active && s.notifications && !s.media);
        assert_eq!(s.key(), "whatsapp");
        let back = serde_yaml_ng::to_string(&config).unwrap();
        assert!(back.contains("always_active: true"));
        assert!(!back.contains("notifications"));
    }
}
