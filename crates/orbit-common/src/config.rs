//! YAML configuration.
//!
//! Everything lives in one directory, `$XDG_CONFIG_HOME/orbit` by default:
//!
//! ```text
//! ~/.config/orbit/
//! ├── panel.yaml        position, size, monitor and the list of modules
//! ├── dock.yaml         the dock
//! ├── style.css         user stylesheet, on top of the built-in theme
//! └── modules/
//!     └── <module>.yaml one file per panel module, e.g. clock.yaml
//! ```
//!
//! Every file and every field is optional; anything left out falls back to
//! the defaults below. A module is turned on by listing it in `panel.yaml`;
//! its own file only holds its options.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, de::DeserializeOwned};

/// The configuration directory.
#[derive(Debug, Clone)]
pub struct ConfigDir {
    path: PathBuf,
}

impl ConfigDir {
    /// `$XDG_CONFIG_HOME/orbit`, falling back to `~/.config/orbit`.
    pub fn default_path() -> PathBuf {
        gtk::glib::user_config_dir().join("orbit")
    }

    pub fn new(path: &Path) -> Self {
        let dir = Self {
            path: expand_home(path),
        };
        if dir.path.join("config.yaml").exists() && !dir.path.join("panel.yaml").exists() {
            log::warn!(
                "{} is no longer read: the config is now split into panel.yaml, \
                 dock.yaml and modules/*.yaml (see examples/ in the repository)",
                dir.path.join("config.yaml").display()
            );
        }
        dir
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn style_path(&self) -> PathBuf {
        self.path.join("style.css")
    }

    pub fn panel(&self) -> anyhow::Result<PanelConfig> {
        self.load(&self.path.join("panel.yaml"))
    }

    pub fn dock(&self) -> anyhow::Result<DockConfig> {
        self.load(&self.path.join("dock.yaml"))
    }

    /// The options of panel module `name`, from `modules/<name>.yaml`.
    pub fn module<T: DeserializeOwned + Default>(&self, name: &str) -> anyhow::Result<T> {
        self.load(&self.path.join("modules").join(format!("{name}.yaml")))
    }

    /// The options of application `name` (`clip`), from `<name>.yaml`.
    pub fn app<T: DeserializeOwned + Default>(&self, name: &str) -> anyhow::Result<T> {
        self.load(&self.path.join(format!("{name}.yaml")))
    }

    /// Loads one file. A missing (or empty) file yields the defaults.
    fn load<T: DeserializeOwned + Default>(&self, path: &Path) -> anyhow::Result<T> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                log::info!("no {}, using defaults", path.display());
                return Ok(T::default());
            }
            Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
        };
        if text.trim().is_empty() {
            return Ok(T::default());
        }
        serde_yaml_ng::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }
}

/// Screen edge a surface is attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    pub fn is_vertical(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }
}

/// Which monitor a surface goes on: an index (`1`) or an output name
/// (`HDMI-A-1`). Names are stable across reboots and hotplug; indices follow
/// whatever order the display server reports.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum MonitorSpec {
    Index(usize),
    Name(String),
}

impl Default for MonitorSpec {
    fn default() -> Self {
        MonitorSpec::Index(0)
    }
}

/// Alignment of the content along the surface's long axis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    Start,
    #[default]
    Center,
    End,
}

/// `panel.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PanelConfig {
    pub enabled: bool,
    pub edge: Edge,
    /// Thickness in logical pixels (width for a vertical panel).
    pub size: i32,
    pub monitor: MonitorSpec,
    /// Reserve screen space so tiled windows do not overlap the panel.
    pub reserve: bool,
    /// Draws the 5 px layout grid behind the panel, for checking alignment.
    pub maintenance: bool,
    /// Modules at the start (top of a vertical panel, left of a horizontal
    /// one), in order. Listing a module turns it on.
    pub start: Vec<ModuleEntry>,
    pub center: Vec<ModuleEntry>,
    /// Modules at the end (bottom / right), in order.
    pub end: Vec<ModuleEntry>,
}

/// A module in `panel.yaml`'s lists: `- clock`, or with a size,
/// `- clock: mini`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "RawEntry")]
pub struct ModuleEntry {
    pub name: String,
    /// The module's own default when left out.
    pub size: Option<Size>,
}

/// How much of a vertical panel's grid a module takes; each module says
/// which of the three it comes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Size {
    Mini,
    Normal,
    Full,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawEntry {
    Name(String),
    Sized(std::collections::BTreeMap<String, Size>),
}

impl TryFrom<RawEntry> for ModuleEntry {
    type Error = String;

    fn try_from(raw: RawEntry) -> Result<Self, String> {
        match raw {
            RawEntry::Name(name) => Ok(name.into()),
            RawEntry::Sized(map) if map.len() == 1 => {
                let (name, size) = map.into_iter().next().expect("one entry");
                Ok(Self { name, size: Some(size) })
            }
            RawEntry::Sized(_) => Err("a module entry is `name` or `name: size`".into()),
        }
    }
}

impl From<&str> for ModuleEntry {
    fn from(name: &str) -> Self {
        Self { name: name.into(), size: None }
    }
}

impl From<String> for ModuleEntry {
    fn from(name: String) -> Self {
        Self { name, size: None }
    }
}

impl Default for PanelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            edge: Edge::Left,
            // The grid's 150 px and 2 px on each side (orbit-panel's board).
            size: 154,
            monitor: MonitorSpec::default(),
            reserve: true,
            maintenance: false,
            start: vec!["menu".into()],
            center: Vec::new(),
            end: vec!["clock".into()],
        }
    }
}

/// `dock.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DockConfig {
    pub enabled: bool,
    pub edge: Edge,
    pub size: i32,
    pub monitor: MonitorSpec,
    pub reserve: bool,
    pub align: Align,
    pub icon_size: i32,
    pub apps: Vec<DockApp>,
}

impl Default for DockConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            edge: Edge::Top,
            size: 36,
            monitor: MonitorSpec::default(),
            reserve: true,
            align: Align::End,
            icon_size: 24,
            apps: Vec::new(),
        }
    }
}

/// A dock launcher.
///
/// Either point `desktop` at a `.desktop` file (a path, or an id such as
/// `firefox.desktop` looked up in the XDG application directories), or give
/// `name`/`exec`/`icon` directly. Explicit fields override the desktop file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DockApp {
    pub desktop: Option<String>,
    pub name: Option<String>,
    pub exec: Option<String>,
    pub icon: Option<String>,
    /// Neighbouring apps with different groups get a separator between them.
    pub group: Option<String>,
    #[serde(default = "default_true")]
    pub show: bool,
}

fn default_true() -> bool {
    true
}

/// Expands a leading `~/` to the home directory.
pub fn expand_home(path: &Path) -> PathBuf {
    match path.strip_prefix("~") {
        Ok(rest) => gtk::glib::home_dir().join(rest),
        Err(_) => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_entries_with_and_without_size() {
        let panel: PanelConfig =
            serde_yaml_ng::from_str("end: [clock, {volume: mini}, {calendar: normal}]").unwrap();
        assert_eq!(
            panel.end,
            vec![
                ModuleEntry::from("clock"),
                ModuleEntry { name: "volume".into(), size: Some(Size::Mini) },
                ModuleEntry { name: "calendar".into(), size: Some(Size::Normal) },
            ]
        );
        assert!(serde_yaml_ng::from_str::<PanelConfig>("end: [{clock: huge}]").is_err());
        assert!(serde_yaml_ng::from_str::<PanelConfig>("end: [{clock: mini, volume: full}]").is_err());
    }
}
