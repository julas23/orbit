//! `~/.config/orbit/core.yaml`. Every field has a default, so the core
//! comes up usable with no file at all (PLANO-CORE §1.4); the defaults are
//! those of the sway configuration Orbit grew on.
//!
//! ```yaml
//! keyboard: { layout: us, variant: alt-intl }
//! modifier: super        # or alt (nested in a session that takes Super)
//! terminal: kitty
//! launcher: gapplication action io.github.julas23.Orbit.Panel menu
//! gaps: 6
//! border: 2
//! corner_radius: 10
//! startup: [orbit-session]   # at once, behind the lock if locked
//! login: [nm-applet]          # at the first unlock (at once if not locked)
//! greet: orbit-greet     # shown while locked; the only client that unlocks
//! stride: 10             # area N: workspace N, 10+N, 20+N… down the monitors
//! outputs:               # where each monitor sits; unnamed ones stack
//!   HDMI-A-1: { position: [0, 1080] }   # below the laptop's, centred
//! ```

use std::{collections::HashMap, path::PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub keyboard: Keyboard,
    /// The key the core's own shortcuts are held with.
    pub modifier: Modifier,
    /// What Mod+Return opens.
    pub terminal: String,
    /// What Mod+Space runs: the panel's application menu, at its search.
    pub launcher: String,
    /// Space between windows, and between them and the screen's edges.
    pub gaps: i32,
    /// Width of the border drawn around each window.
    pub border: i32,
    /// Radius of the windows' corners (0: square).
    pub corner_radius: i32,
    /// RGB colours of the border: the focused window's and the others'.
    pub focused_color: String,
    pub unfocused_color: String,
    /// The window the pointer enters takes the keyboard.
    pub focus_follows_mouse: bool,
    /// Commands run once the core is up (through `sh -c`).
    pub startup: Vec<String>,
    /// Commands run once the user is in: at the first unlock of a core
    /// started `--locked`, at once otherwise. What needs the user's
    /// secrets (the keyring) goes here.
    pub login: Vec<String>,
    /// The greet, launched by the core at each lock (through `sh -c`).
    pub greet: String,
    /// The pointer's theme and size (sway's `seat * xcursor_theme`); also
    /// handed to clients as XCURSOR_THEME and XCURSOR_SIZE.
    pub cursor_theme: String,
    pub cursor_size: u32,
    /// Monitors by connector name (`eDP-1`, `HDMI-A-1`).
    pub outputs: HashMap<String, OutputConfig>,
    /// Workspace-number distance between the same area on two monitors
    /// (the panel's `stride`; see `workspace`).
    pub stride: i32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OutputConfig {
    /// Top-left corner, in the layout's logical pixels (sway's `position`).
    pub position: Option<(i32, i32)>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keyboard {
    pub layout: String,
    pub variant: String,
    pub model: String,
    pub options: String,
    /// Milliseconds before a held key repeats, and repeats per second.
    pub repeat_delay: i32,
    pub repeat_rate: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modifier {
    Super,
    Alt,
    Ctrl,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            keyboard: Keyboard::default(),
            modifier: Modifier::Super,
            terminal: "kitty".into(),
            launcher: "gapplication action io.github.julas23.Orbit.Panel menu".into(),
            gaps: 6,
            border: 2,
            corner_radius: 10,
            focused_color: "#4682b4".into(),
            unfocused_color: "#4c566a".into(),
            focus_follows_mouse: true,
            startup: Vec::new(),
            login: Vec::new(),
            greet: "orbit-greet".into(),
            cursor_theme: "Breeze_Light".into(),
            cursor_size: 24,
            outputs: HashMap::new(),
            stride: 10,
        }
    }
}

impl Default for Keyboard {
    fn default() -> Self {
        Self {
            layout: "us".into(),
            variant: "alt-intl".into(),
            model: "pc105".into(),
            options: String::new(),
            repeat_delay: 200,
            repeat_rate: 25,
        }
    }
}

impl Config {
    /// `$XDG_CONFIG_HOME/orbit/core.yaml`, or `~/.config/orbit/core.yaml`.
    pub fn path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
        base.join("orbit").join("core.yaml")
    }

    /// The file at `path`, with the monitors where `output … pos` last put
    /// them (`placed`); the defaults when it is missing. An error in it is
    /// reported and the defaults are used: a bad file never stops the
    /// session from coming up (§1.5).
    pub fn load(path: &std::path::Path) -> Self {
        let mut config = Self::read(path);
        for (name, at) in placed() {
            config.outputs.entry(name).or_default().position = Some(at);
        }
        config
    }

    fn read(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) if text.trim().is_empty() => Self::default(),
            Ok(text) => serde_yaml_ng::from_str(&text).unwrap_or_else(|err| {
                tracing::error!("{}: {err}; using the defaults", path.display());
                Self::default()
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(err) => {
                tracing::error!("{}: {err}; using the defaults", path.display());
                Self::default()
            }
        }
    }
}

/// `$XDG_STATE_HOME/orbit/core`, or `~/.local/state/orbit/core`: the log
/// and what the core remembers between sessions.
pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state"))
        .join("orbit/core")
}

/// Where `output … pos` put each monitor (the panel's monitors module, or
/// orbitctl): kept in the state directory, so the next session comes up
/// the same, and over `outputs:` in core.yaml.
pub fn placed() -> HashMap<String, (i32, i32)> {
    std::fs::read_to_string(state_dir().join("outputs.yaml"))
        .ok()
        .and_then(|text| serde_yaml_ng::from_str(&text).ok())
        .unwrap_or_default()
}

/// Remembers that monitor `name` is at `at`.
pub fn place(name: &str, at: (i32, i32)) {
    let mut all = placed();
    all.insert(name.to_owned(), at);
    let dir = state_dir();
    let written = std::fs::create_dir_all(&dir)
        .and_then(|()| serde_yaml_ng::to_string(&all).map_err(std::io::Error::other))
        .and_then(|text| std::fs::write(dir.join("outputs.yaml"), text));
    if let Err(err) = written {
        tracing::warn!("remembering where {name} is: {err}");
    }
}

/// "#4682b4" → [r, g, b, 1.0]; a grey for anything else.
pub fn color(text: &str) -> [f32; 4] {
    let hex = text.trim().trim_start_matches('#');
    let channel = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).map(|v| v as f32 / 255.0);
    match (hex.len(), channel(0), channel(2), channel(4)) {
        (6, Some(r), Some(g), Some(b)) => [r, g, b, 1.0],
        _ => [0.3, 0.34, 0.42, 1.0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_partial_files() {
        let config: Config = serde_yaml_ng::from_str("gaps: 10\nkeyboard: { variant: intl }").unwrap();
        assert_eq!(config.gaps, 10);
        assert_eq!(config.keyboard.layout, "us");
        assert_eq!(config.keyboard.variant, "intl");
        assert_eq!(config.modifier, Modifier::Super);
        assert!(serde_yaml_ng::from_str::<Config>("gapz: 1").is_err());
        let config: Config = serde_yaml_ng::from_str("outputs: { HDMI-A-1: { position: [0, 1080] } }").unwrap();
        assert_eq!(config.outputs["HDMI-A-1"].position, Some((0, 1080)));
    }

    #[test]
    fn colours() {
        assert_eq!(color("#ff0000"), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(color("nope")[3], 1.0);
    }
}
