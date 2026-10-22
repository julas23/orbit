//! The shortcuts: `keys.yaml` next to `core.yaml`, each key combination
//! with the command it runs — the same commands as the IPC's (`command`),
//! as sway's `bindsym`. `Mod` is the `modifier` of `core.yaml`; `$terminal`,
//! `$launcher` and `$run` are its commands. The file adds to the defaults
//! below or changes them; `none` takes one away.
//!
//! ```yaml
//! Mod+R: exec $run
//! Mod+B: exec firefox
//! Mod+Shift+Return: none
//! ```
//!
//! Two stay out of the file, always the core's: Ctrl+Alt+Backspace (out,
//! whatever the config says) and Ctrl+Alt+F1…F12 (another console).

use std::{collections::HashMap, path::Path};

use smithay::input::keyboard::{Keysym, ModifiersState, xkb};

use crate::config::{Config, Modifier};

/// The shortcuts of a core with no `keys.yaml`.
pub const DEFAULT: &[(&str, &str)] = &[
    ("Mod+Return", "exec $terminal"),
    ("Mod+Space", "exec $launcher"),
    ("Mod+R", "exec $run"),
    ("Mod+Q", "kill"),
    ("Mod+Shift+Q", "kill --force"),
    ("Mod+Shift+E", "exit"),
    ("Mod+L", "lock"),
    ("Mod+F", "fullscreen toggle"),
    ("Mod+Shift+Space", "floating toggle"),
    ("Mod+Escape", "floating toggle"),
    ("Alt+Tab", "switch workspace"),
    ("Alt+Shift+Tab", "switch all"),
    ("Mod+Tab", "switch cards"),
    ("Mod+Shift+Tab", "switch cards back"),
    ("Ctrl+Alt+Left", "workspace left"),
    ("Ctrl+Alt+Right", "workspace right"),
    ("Ctrl+Alt+Up", "workspace up"),
    ("Ctrl+Alt+Down", "workspace down"),
    // Mod+arrows: the window's size; Alt+arrows: the keyboard that way;
    // Mod+Ctrl+arrows: the window that way (`direction`).
    ("Mod+Up", "maximize"),
    ("Mod+Down", "restore"),
    ("Mod+Left", "snap left"),
    ("Mod+Right", "snap right"),
    ("Alt+Left", "focus left"),
    ("Alt+Right", "focus right"),
    ("Alt+Up", "focus up"),
    ("Alt+Down", "focus down"),
    ("Mod+Ctrl+Left", "move left"),
    ("Mod+Ctrl+Right", "move right"),
    ("Mod+Ctrl+Up", "move up"),
    ("Mod+Ctrl+Down", "move down"),
    ("Mod+J", "focus next"),
    ("Mod+K", "focus prev"),
    ("Mod+H", "focus prev"),
    ("Mod+Shift+H", "move left"),
    ("Mod+Shift+J", "move down"),
    ("Mod+Shift+K", "move up"),
    ("Mod+Shift+L", "move right"),
    ("Mod+1", "workspace number 1"),
    ("Mod+2", "workspace number 2"),
    ("Mod+3", "workspace number 3"),
    ("Mod+4", "workspace number 4"),
    ("Mod+5", "workspace number 5"),
    ("Mod+6", "workspace number 6"),
    ("Mod+7", "workspace number 7"),
    ("Mod+8", "workspace number 8"),
    ("Mod+9", "workspace number 9"),
    ("Mod+0", "workspace number 10"),
    ("Mod+Shift+1", "move container to workspace number 1"),
    ("Mod+Shift+2", "move container to workspace number 2"),
    ("Mod+Shift+3", "move container to workspace number 3"),
    ("Mod+Shift+4", "move container to workspace number 4"),
    ("Mod+Shift+5", "move container to workspace number 5"),
    ("Mod+Shift+6", "move container to workspace number 6"),
    ("Mod+Shift+7", "move container to workspace number 7"),
    ("Mod+Shift+8", "move container to workspace number 8"),
    ("Mod+Shift+9", "move container to workspace number 9"),
    ("Mod+Shift+0", "move container to workspace number 10"),
];

/// The modifiers of a combination; all four must match, no more held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
struct Held {
    logo: bool,
    alt: bool,
    ctrl: bool,
    shift: bool,
}

impl Held {
    fn of(mods: &ModifiersState) -> Self {
        Self { logo: mods.logo, alt: mods.alt, ctrl: mods.ctrl, shift: mods.shift }
    }
}

/// The shortcuts in force: combination → command.
#[derive(Debug, Default)]
pub struct Keys(HashMap<(Held, Keysym), String>);

impl Keys {
    /// The defaults, then `keys.yaml` beside `core` over them.
    pub fn load(core: &Path, config: &Config) -> Self {
        let path = core.with_file_name("keys.yaml");
        let file: Vec<(String, String)> = match std::fs::read_to_string(&path) {
            Ok(text) if text.trim().is_empty() => Vec::new(),
            Ok(text) => match serde_yaml_ng::from_str::<serde_yaml_ng::Mapping>(&text) {
                Ok(map) => map
                    .into_iter()
                    .filter_map(|(k, v)| match (k.as_str(), v.as_str()) {
                        (Some(k), Some(v)) => Some((k.to_owned(), v.to_owned())),
                        _ => {
                            tracing::error!("{}: {k:?}: {v:?}: a key and a command, both text", path.display());
                            None
                        }
                    })
                    .collect(),
                Err(err) => {
                    tracing::error!("{}: {err}; only the default shortcuts", path.display());
                    Vec::new()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                tracing::error!("{}: {err}; only the default shortcuts", path.display());
                Vec::new()
            }
        };
        let defaults = DEFAULT.iter().map(|&(k, v)| (k.to_owned(), v.to_owned()));
        let mut keys = Self::default();
        for (combo, command) in defaults.chain(file) {
            match parse(&combo, config.modifier) {
                Ok(key) if command.trim() == "none" => {
                    keys.0.remove(&key);
                }
                Ok(key) => {
                    keys.0.insert(key, expand(&command, config));
                }
                Err(err) => tracing::error!("{}: {combo}: {err}", path.display()),
            }
        }
        keys
    }

    /// The command of `sym` with `mods` held, if it is a shortcut.
    pub fn command(&self, mods: &ModifiersState, sym: Keysym) -> Option<&str> {
        let sym = Keysym::from(xkb_lower(sym.raw()));
        self.0.get(&(Held::of(mods), sym)).map(String::as_str)
    }
}

/// `Mod+Shift+Q` → the modifiers and the key (letters by their small one).
fn parse(combo: &str, modifier: Modifier) -> Result<(Held, Keysym), String> {
    let mut held = Held::default();
    let mut parts: Vec<&str> = combo.split('+').map(str::trim).collect();
    let key = parts.pop().filter(|k| !k.is_empty()).ok_or("no key")?;
    for part in parts {
        match part.to_ascii_lowercase().as_str() {
            "mod" => match modifier {
                Modifier::Super => held.logo = true,
                Modifier::Alt => held.alt = true,
                Modifier::Ctrl => held.ctrl = true,
            },
            "super" | "logo" | "mod4" => held.logo = true,
            "alt" | "mod1" => held.alt = true,
            "ctrl" | "control" => held.ctrl = true,
            "shift" => held.shift = true,
            other => return Err(format!("not a modifier: {other}")),
        }
    }
    let mut sym = xkb::keysym_from_name(key, xkb::KEYSYM_NO_FLAGS);
    if sym.raw() == 0 {
        sym = xkb::keysym_from_name(key, xkb::KEYSYM_CASE_INSENSITIVE);
    }
    if sym.raw() == 0 {
        return Err(format!("not a key: {key}"));
    }
    Ok((held, Keysym::from(xkb_lower(sym.raw()))))
}

/// `$terminal`, `$launcher` and `$run` as `core.yaml` says.
fn expand(command: &str, config: &Config) -> String {
    command.replace("$terminal", &config.terminal).replace("$launcher", &config.launcher).replace("$run", &config.run)
}

/// A Latin capital's small letter ("Q" with Shift is still Q).
fn xkb_lower(raw: u32) -> u32 {
    if (u32::from(b'A')..=u32::from(b'Z')).contains(&raw) { raw + 32 } else { raw }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(logo: bool, shift: bool) -> ModifiersState {
        ModifiersState { logo, shift, ..Default::default() }
    }

    fn keys(file: &str, config: &Config) -> Keys {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("orbit-keys-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("keys.yaml"), file).unwrap();
        let keys = Keys::load(&dir.join("core.yaml"), config);
        std::fs::remove_dir_all(dir).unwrap();
        keys
    }

    #[test]
    fn the_defaults() {
        let config = Config::default();
        let k = keys("", &config);
        assert_eq!(k.command(&mods(true, false), Keysym::Return), Some("exec kitty"));
        assert_eq!(k.command(&mods(true, false), Keysym::r), Some("exec orbit-run"));
        assert_eq!(k.command(&mods(true, false), Keysym::q), Some("kill"));
        assert_eq!(k.command(&mods(true, true), Keysym::Q), Some("kill --force"));
        assert_eq!(k.command(&mods(false, true), Keysym::Q), None);
        assert_eq!(k.command(&mods(true, false), Keysym::x), None);
        assert_eq!(k.command(&mods(true, false), Keysym::_3), Some("workspace number 3"));
        assert_eq!(k.command(&mods(true, true), Keysym::_0), Some("move container to workspace number 10"));
        // No more modifiers held than the combination's.
        let ctrl = ModifiersState { logo: true, ctrl: true, ..Default::default() };
        assert_eq!(k.command(&ctrl, Keysym::q), None);
    }

    #[test]
    fn the_file_adds_changes_and_removes() {
        let config = Config::default();
        let k = keys("Mod+B: exec firefox\nMod+Return: exec foot\nMod+Shift+E: none\nCtrl+Alt+t: exec $terminal\n", &config);
        assert_eq!(k.command(&mods(true, false), Keysym::b), Some("exec firefox"));
        assert_eq!(k.command(&mods(true, false), Keysym::Return), Some("exec foot"));
        assert_eq!(k.command(&mods(true, true), Keysym::e), None);
        let ctrl_alt = ModifiersState { ctrl: true, alt: true, ..Default::default() };
        assert_eq!(k.command(&ctrl_alt, Keysym::t), Some("exec kitty"));
    }

    #[test]
    fn mod_follows_the_modifier() {
        let config = Config { modifier: Modifier::Alt, ..Config::default() };
        let k = keys("", &config);
        let alt = ModifiersState { alt: true, ..Default::default() };
        assert_eq!(k.command(&alt, Keysym::Return), Some("exec kitty"));
        assert_eq!(k.command(&mods(true, false), Keysym::Return), None);
    }

    #[test]
    fn a_bad_combination_is_skipped() {
        assert!(parse("Hyper+Q", Modifier::Super).is_err());
        assert!(parse("Mod+NoSuchKey", Modifier::Super).is_err());
        assert!(parse("Mod+", Modifier::Super).is_err());
        assert_eq!(parse("mod+shift+q", Modifier::Super), Ok((Held { logo: true, shift: true, ..Default::default() }, Keysym::q)));
    }
}
