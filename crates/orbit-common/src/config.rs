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
use serde::{Deserialize, Serialize, de::DeserializeOwned};

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

    /// Writes the panel's module lists into `panel.yaml`, in place of the
    /// `start:`, `center:` and `end:` it has; every other line, comments
    /// included, stays. The file before the first write of a run is kept
    /// as `panel.yaml.bak`.
    pub fn save_panel_modules(
        &self,
        start: &[ModuleEntry],
        center: &[ModuleEntry],
        end: &[ModuleEntry],
        first: bool,
    ) -> anyhow::Result<()> {
        let lists = module_lists(start, center, end);
        self.rewrite("panel.yaml", &["start", "center", "end"], &lists, first)
    }

    /// Writes the dock's launchers into `dock.yaml`, in place of its
    /// `apps:`; the rest stays, and the first write of a run keeps the old
    /// file as `dock.yaml.bak`.
    pub fn save_dock_apps(&self, apps: &[DockApp], first: bool) -> anyhow::Result<()> {
        let list = if apps.is_empty() {
            "apps: []\n".to_owned()
        } else {
            format!("apps:\n{}", serde_yaml_ng::to_string(apps)?)
        };
        self.rewrite("dock.yaml", &["apps"], &list, first)
    }

    /// File `name` with its top-level `keys` replaced by `blocks`.
    fn rewrite(&self, name: &str, keys: &[&str], blocks: &str, first: bool) -> anyhow::Result<()> {
        let path = self.path.join(name);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
        };
        if first && !text.is_empty() {
            std::fs::write(path.with_extension("yaml.bak"), &text)
                .with_context(|| format!("backing up {}", path.display()))?;
        }
        std::fs::create_dir_all(&self.path)?;
        let temp = path.with_extension("yaml.new");
        std::fs::write(&temp, with_blocks(&text, keys, blocks))
            .and_then(|()| std::fs::rename(&temp, &path))
            .with_context(|| format!("writing {}", path.display()))
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

impl Size {
    /// Its name in `panel.yaml`.
    pub fn name(self) -> &'static str {
        match self {
            Size::Mini => "mini",
            Size::Normal => "normal",
            Size::Full => "full",
        }
    }
}

/// `panel.yaml`'s `start:`, `center:` and `end:`, as YAML.
fn module_lists(start: &[ModuleEntry], center: &[ModuleEntry], end: &[ModuleEntry]) -> String {
    let mut lists = String::new();
    for (key, entries) in [("start", start), ("center", center), ("end", end)] {
        if entries.is_empty() {
            lists.push_str(&format!("{key}: []\n"));
            continue;
        }
        lists.push_str(&format!("{key}:\n"));
        for entry in entries {
            match entry.size {
                Some(size) => lists.push_str(&format!("  - {}: {}\n", entry.name, size.name())),
                None => lists.push_str(&format!("  - {}\n", entry.name)),
            }
        }
    }
    lists
}

/// `text` (a YAML file) with its top-level `keys` taken out and `blocks`
/// written where the first of them was (or at the end). A key's block runs
/// to the first line that is blank or starts in the first column with
/// anything but `- `.
fn with_blocks(text: &str, keys: &[&str], blocks: &str) -> String {
    let is_key = |line: &str| {
        keys.iter()
            .any(|key| line.strip_prefix(key).is_some_and(|rest| rest.trim_start().starts_with(':')))
    };
    let mut kept: Vec<&str> = Vec::new();
    let mut at = None;
    let mut inside = false;
    for line in text.lines() {
        if is_key(line) {
            at.get_or_insert(kept.len());
            inside = true;
            continue;
        }
        if inside && !line.trim().is_empty() && (line.starts_with([' ', '\t']) || line.starts_with('-')) {
            continue;
        }
        inside = false;
        kept.push(line);
    }
    let at = at.unwrap_or(kept.len());
    let mut out = String::new();
    for line in &kept[..at] {
        out.push_str(line);
        out.push('\n');
    }
    if at == kept.len() && !out.is_empty() && !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(blocks);
    for line in &kept[at..] {
        out.push_str(line);
        out.push('\n');
    }
    out
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DockApp {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Neighbouring apps with different groups get a separator between them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub show: bool,
}

fn is_true(value: &bool) -> bool {
    *value
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

    #[test]
    fn module_lists_are_replaced_in_place() {
        const KEYS: &[&str] = &["start", "center", "end"];
        let lists = |start: &[&str], end: &[ModuleEntry]| {
            let start: Vec<ModuleEntry> = start.iter().map(|&n| n.into()).collect();
            module_lists(&start, &[], end)
        };
        let text = "# Panel\nedge: left\n# The modules:\nstart:\n  - menu\n  # old\n  - windows\nend: [clock]\n\nsize: 154\n";
        let out = with_blocks(
            text,
            KEYS,
            &lists(&["windows"], &["clock".into(), ModuleEntry { name: "volume".into(), size: Some(Size::Mini) }]),
        );
        assert_eq!(
            out,
            "# Panel\nedge: left\n# The modules:\nstart:\n  - windows\ncenter: []\nend:\n  - clock\n  - volume: mini\n\nsize: 154\n"
        );
        let panel: PanelConfig = serde_yaml_ng::from_str(&out).unwrap();
        assert_eq!(panel.end[1].size, Some(Size::Mini));
        // Lists at column 0, and a file without any.
        let out = with_blocks("start:\n- menu\nedge: left\n", KEYS, &lists(&["clock"], &[]));
        assert_eq!(out, "start:\n  - clock\ncenter: []\nend: []\nedge: left\n");
        assert_eq!(
            with_blocks("edge: left\n", KEYS, &lists(&[], &[])),
            "edge: left\n\nstart: []\ncenter: []\nend: []\n"
        );
    }

    #[test]
    fn dock_apps_round_trip() {
        let text = "# Dock\nedge: top\napps:\n- desktop: a.desktop\n  group: web\n- desktop: b.desktop\n  show: false\nsize: 36\n";
        let dock: DockConfig = serde_yaml_ng::from_str(text).unwrap();
        let list = format!("apps:\n{}", serde_yaml_ng::to_string(&dock.apps).unwrap());
        let out = with_blocks(text, &["apps"], &list);
        assert_eq!(out, text);
    }
}
