//! YAML configuration.
//!
//! The file lives at `$XDG_CONFIG_HOME/orbit/config.yaml` by default. Every
//! field is optional; anything left out falls back to the defaults below.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

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

/// Alignment of the content along the surface's long axis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    Start,
    #[default]
    Center,
    End,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// User stylesheet, relative to the config directory unless absolute.
    pub style: PathBuf,
    pub panel: PanelConfig,
    pub dock: DockConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            style: PathBuf::from("style.css"),
            panel: PanelConfig::default(),
            dock: DockConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PanelConfig {
    pub enabled: bool,
    pub edge: Edge,
    /// Thickness in logical pixels (width for a vertical panel).
    pub size: i32,
    pub monitor: usize,
    /// Reserve screen space so tiled windows do not overlap the panel.
    pub reserve: bool,
    pub menu: Option<MenuConfig>,
}

impl Default for PanelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            edge: Edge::Left,
            size: 150,
            monitor: 0,
            reserve: true,
            menu: None,
        }
    }
}

/// The application-menu button at the start of the panel.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuConfig {
    pub command: String,
    /// Icon name from the theme, or a path to an image file.
    pub icon: Option<String>,
    pub tooltip: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DockConfig {
    pub enabled: bool,
    pub edge: Edge,
    pub size: i32,
    pub monitor: usize,
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
            monitor: 0,
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

impl Config {
    /// `$XDG_CONFIG_HOME/orbit`, falling back to `~/.config/orbit`.
    pub fn dir() -> PathBuf {
        gtk::glib::user_config_dir().join("orbit")
    }

    pub fn default_path() -> PathBuf {
        Self::dir().join("config.yaml")
    }

    /// Loads the config at `path`. A missing file yields the defaults.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                log::info!("no config at {}, using defaults", path.display());
                return Ok(Self::default());
            }
            Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
        };
        serde_yaml_ng::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// The stylesheet path, resolved against the directory of the config file.
    pub fn style_path(&self, config_path: &Path) -> PathBuf {
        let base = config_path.parent().unwrap_or(Path::new("."));
        expand_home(&base.join(&self.style))
    }
}

/// Expands a leading `~/` to the home directory.
pub fn expand_home(path: &Path) -> PathBuf {
    match path.strip_prefix("~") {
        Ok(rest) => gtk::glib::home_dir().join(rest),
        Err(_) => path.to_path_buf(),
    }
}
