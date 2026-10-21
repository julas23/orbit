//! Ferdium recipes: how a service's unread count is read from its page
//! (and the touches of CSS it gets), for the sites whose title says
//! nothing — Outlook, GitHub — or says it wrong (WhatsApp counts muted
//! chats). A recipe is Ferdium's `package.json` and `webview.js`; the
//! `webview.js` runs unchanged on top of the subset of Ferdium's API in
//! `data/pathfinder/recipe.js`.
//!
//! Recipes come from `~/.local/share/orbit/pathfinder/recipes/` (your
//! own, first) and from the ones installed with Pathfinder
//! (`<data dir>/orbit-pathfinder/recipes`, from `data/pathfinder/recipes`).
//! A service names its recipe (`recipe: gmail`, `recipe: none` for none),
//! or gets the one whose address matches its own.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use gtk::glib;
use serde::Deserialize;

use crate::privacy;

/// The isolated world recipes run in.
pub const WORLD: &str = "orbit-recipe";
/// The message handler they talk through.
pub const HANDLER: &str = "orbitRecipe";

const SHIM: &str = include_str!("../../../data/pathfinder/recipe.js");

/// Hosts that hold several products under one name, told apart by the
/// path: a recipe for one (Google Maps, at www.google.com/maps) is not the
/// others'.
const SHARED_HOSTS: &[&str] = &["www.google.com", "google.com"];

#[derive(Debug, Clone)]
pub struct Recipe {
    pub id: String,
    url: String,
    aliases: Vec<String>,
    dir: PathBuf,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    config: PackageConfig,
}

#[derive(Default, Deserialize)]
struct PackageConfig {
    #[serde(default, rename = "serviceURL")]
    service_url: String,
}

/// The recipes found, by id; your own over the installed ones.
#[derive(Default)]
pub struct Index(HashMap<String, Recipe>);

impl Index {
    pub fn load() -> Self {
        let mut dirs = vec![glib::user_data_dir().join("orbit").join("pathfinder").join("recipes")];
        for data in std::iter::once(glib::user_data_dir()).chain(glib::system_data_dirs()) {
            dirs.push(data.join("orbit-pathfinder").join("recipes"));
        }
        // Run from the source tree (target/release/orbit-pathfinder).
        if let Ok(exe) = std::env::current_exe()
            && let Some(root) = exe.ancestors().nth(3)
        {
            dirs.push(root.join("data").join("pathfinder").join("recipes"));
        }
        Self::from_dirs(&dirs)
    }

    /// The recipes in `dirs`, the first holding an id winning.
    fn from_dirs(dirs: &[PathBuf]) -> Self {
        let mut index = HashMap::new();
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            for entry in entries.flatten() {
                let dir = entry.path();
                let Ok(text) = std::fs::read_to_string(dir.join("package.json")) else { continue };
                let Ok(package) = serde_json::from_str::<Package>(&text) else { continue };
                if !dir.join("webview.js").is_file() {
                    continue;
                }
                index.entry(package.id.clone()).or_insert(Recipe {
                    id: package.id,
                    url: package.config.service_url,
                    aliases: package.aliases.into_iter().map(|a| a.to_ascii_lowercase()).collect(),
                    dir,
                });
            }
        }
        log::debug!("{} recipes", index.len());
        Self(index)
    }

    /// The recipe of a service: the one it names, or else the one for its
    /// address. `none` names none.
    pub fn find(&self, named: Option<&str>, url: &str) -> Option<&Recipe> {
        match named {
            Some("none") | Some("") => None,
            Some(id) => self.0.get(id),
            None => matching(self.0.values(), url),
        }
    }
}

/// The recipe for `url`: the longest recipe address it starts with; else
/// one for the same host (not a shared one); else one with the site among
/// its aliases ("live.com" for Outlook).
fn matching<'a>(recipes: impl Iterator<Item = &'a Recipe> + Clone, url: &str) -> Option<&'a Recipe> {
    let host = privacy::host(url)?;
    let trimmed = |u: &str| u.trim_end_matches('/').to_ascii_lowercase();
    let url_l = url.to_ascii_lowercase();
    let prefix = recipes
        .clone()
        .filter(|r| !r.url.is_empty() && (url_l.starts_with(&trimmed(&r.url)) && boundary(&url_l, &trimmed(&r.url))))
        .max_by_key(|r| (trimmed(&r.url).len(), std::cmp::Reverse(r.id.clone())));
    if prefix.is_some() {
        return prefix;
    }
    if !SHARED_HOSTS.contains(&host.as_str()) {
        let mut same: Vec<&Recipe> = recipes.clone().filter(|r| privacy::host(&r.url).as_deref() == Some(&host)).collect();
        same.sort_by(|a, b| a.id.cmp(&b.id));
        if let Some(r) = same.first() {
            return Some(r);
        }
    }
    // By alias, the same product: "/mail/" is not "/calendar/".
    let site = privacy::site(&host);
    let same_product = |r: &Recipe| match (first_segment(url), first_segment(&r.url)) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => true,
    };
    let mut aliased: Vec<&Recipe> =
        recipes.filter(|r| r.aliases.iter().any(|a| *a == host || *a == site) && same_product(r)).collect();
    aliased.sort_by(|a, b| a.id.cmp(&b.id));
    aliased.first().copied()
}

/// "https://outlook.live.com/mail/0/" → "mail".
fn first_segment(url: &str) -> Option<&str> {
    let path = url.split_once("://")?.1.split_once('/')?.1;
    path.split(['/', '?', '#']).next().filter(|s| !s.is_empty())
}

/// Whether `prefix` ends at a boundary of `url` ("…/mail" is not a prefix
/// of "…/mailbox").
fn boundary(url: &str, prefix: &str) -> bool {
    matches!(url.as_bytes().get(prefix.len()), None | Some(b'/' | b'?' | b'#'))
}

impl Recipe {
    /// The user script that runs it: the shim with its files and settings.
    pub fn script(&self) -> Option<String> {
        let webview = std::fs::read_to_string(self.dir.join("webview.js")).ok()?;
        let mut files = serde_json::Map::new();
        for entry in std::fs::read_dir(&self.dir).ok()?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Its style sheets, for `injectCSS`; page scripts are read when
            // asked for (`page_script`).
            if name.ends_with(".css")
                && let Ok(text) = std::fs::read_to_string(entry.path())
            {
                files.insert(name, text.into());
            }
        }
        Some(
            // In this order, once each: the recipe's own text may hold
            // anything.
            SHIM.replacen("__FILES__", &serde_json::Value::Object(files).to_string(), 1)
                .replacen("__SETTINGS__", "{}", 1)
                .replacen("__WEBVIEW__", &webview, 1),
        )
    }

    /// One of its page scripts (`injectJSUnsafe`), by file name.
    pub fn page_script(&self, name: &str) -> Option<String> {
        let path = self.dir.join(name);
        // Only a file of its own folder.
        (Path::new(name).file_name().is_some_and(|n| n == name) && name.ends_with(".js")).then_some(())?;
        std::fs::read_to_string(path).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(id: &str, url: &str, aliases: &[&str]) -> Recipe {
        Recipe { id: id.into(), url: url.into(), aliases: aliases.iter().map(|a| a.to_string()).collect(), dir: PathBuf::new() }
    }

    #[test]
    fn matches_addresses() {
        let all = [
            r("gmail", "https://mail.google.com", &[]),
            r("hangouts", "https://mail.google.com/chat/", &[]),
            r("google-maps", "https://www.google.com/maps", &[]),
            r("linkedin", "https://www.linkedin.com/messaging", &[]),
            r("office365-owa", "https://outlook.cloud.microsoft/mail/", &["live.com", "outlook"]),
            r("whatsapp", "https://web.whatsapp.com", &[]),
        ];
        let find = |url: &str| matching(all.iter(), url).map(|r| r.id.as_str());
        assert_eq!(find("https://mail.google.com/"), Some("gmail"));
        assert_eq!(find("https://mail.google.com/chat/u/0"), Some("hangouts"));
        assert_eq!(find("https://www.google.com"), None);
        assert_eq!(find("https://www.google.com/maps"), Some("google-maps"));
        assert_eq!(find("https://www.google.com/mapsx"), None);
        assert_eq!(find("https://www.linkedin.com/"), Some("linkedin"));
        assert_eq!(find("https://outlook.live.com/mail/"), Some("office365-owa"));
        assert_eq!(find("https://outlook.live.com/calendar/"), None);
        assert_eq!(find("https://web.whatsapp.com/"), Some("whatsapp"));
        assert_eq!(find("https://example.org/"), None);
    }

    /// Which recipe each service of this machine's config gets:
    /// `cargo test -p orbit-pathfinder services_here -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn services_here() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/pathfinder/recipes");
        let index = Index::from_dirs(&[root]);
        let file = glib::user_config_dir().join("orbit").join("pathfinder.yaml");
        let config: crate::config::Config = serde_yaml_ng::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
        for s in config.groups.iter().flat_map(|g| &g.services) {
            eprintln!("{:<26} {:<45} {:?}", s.name, s.url, index.find(s.recipe.as_deref(), &s.url).map(|r| &r.id));
        }
    }

    #[test]
    fn page_scripts_stay_in_their_folder() {
        let recipe = r("x", "", &[]);
        assert_eq!(recipe.page_script("../../etc/passwd"), None);
        assert_eq!(recipe.page_script("a/b.js"), None);
    }
}
