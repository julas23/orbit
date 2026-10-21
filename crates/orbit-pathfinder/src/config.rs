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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// The browser's start page (the Home button); Google's when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// Where text typed in the address bar that is no address is searched:
    /// the query goes on the end. Google's when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    /// The block lists (Adblock format) every page uses; EasyList and
    /// EasyPrivacy when absent, none when empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocklists: Option<Vec<String>>,
    /// Minutes a service may go unused before its page is unloaded, to come
    /// back on the next click; 0 never. Services kept always active are
    /// never unloaded.
    #[serde(default = "hibernate_default", skip_serializing_if = "is_hibernate_default")]
    pub hibernate_after: u32,
}

fn hibernate_default() -> u32 {
    30
}

fn is_hibernate_default(n: &u32) -> bool {
    *n == hibernate_default()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            external_browser: None,
            user_agent: None,
            home: None,
            search: None,
            blocklists: None,
            hibernate_after: hibernate_default(),
        }
    }
}

/// A run of services; between groups the sidebar draws a line. A named
/// group gets a heading that folds it away.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Group {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<Service>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub name: String,
    pub url: String,
    /// Names the service (its icon, zoom and place). From the name when
    /// absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The storage — cookies, logins, cache — it uses. Services naming the
    /// same profile share one login (Gmail, Calendar and YouTube on one
    /// Google account); without one, a service has storage of its own, so
    /// two accounts of one site stay apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
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
    /// The Ferdium recipe that reads its unread count ("gmail"; "none" for
    /// none); the one matching its address when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
    /// The block lists apply to it; off for a service they break.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub block: bool,
    /// The built-in browser at the top of the sidebar (not in the file):
    /// every link opens in it, and its title counts nothing.
    #[serde(skip)]
    pub browser: bool,
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
            profile: None,
            icon: None,
            always_active: false,
            media: false,
            notifications: true,
            muted: false,
            user_agent: None,
            proxy: None,
            domains: Vec::new(),
            recipe: None,
            block: true,
            browser: false,
        }
    }

    /// The name of its storage: `id`, or the name made safe for a folder.
    pub fn key(&self) -> String {
        slug(self.id.as_deref().unwrap_or(&self.name))
    }

    /// The name of its storage: the profile, or its own key.
    pub fn profile(&self) -> String {
        self.profile.as_deref().map(slug).unwrap_or_else(|| self.key())
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
        let text = "groups:\n  - services:\n      - name: Gmail\n        url: https://mail.google.com/\n        profile: Google\n  - name: A\n    services:\n      - name: WhatsApp\n        url: https://web.whatsapp.com/\n        always_active: true\n";
        let config: Config = serde_yaml_ng::from_str(text).unwrap();
        assert_eq!(config.groups[0].name, None);
        assert_eq!(config.groups[0].services[0].profile(), "google");
        let s = &config.groups[1].services[0];
        assert!(s.always_active && s.notifications && !s.media);
        assert_eq!(s.key(), "whatsapp");
        let back = serde_yaml_ng::to_string(&config).unwrap();
        assert!(back.contains("always_active: true"));
        assert!(!back.contains("notifications"));
    }
}
