//! `aigent.yaml`, in `$XDG_CONFIG_HOME/orbit` (`~/.config/orbit`):
//!
//! ```yaml
//! agent: claude            # the one in use at start
//! agents:                  # every one that can be chosen
//!   claude: {}             # the name is the kind: claude, gemini, opencode, kimi
//!   work:                  # or a name of one's own, with its kind
//!     kind: claude
//!     program: /opt/claude/bin/claude
//!     dir: ~/Vault/GIT     # where it works; the home by default
//!     args: [--model, opus]
//! wake: [orbit, jarvis]    # the words that wake it, in any language
//! wake_model: base-q5_1    # Whisper, hearing for the wake word
//! model: small-q5_1        # and hearing the request
//! microphone: alsa_input.usb-…   # a source's node name (`wpctl inspect`);
//!                          # the default one when left out
//! ```
//!
//! A Bluetooth headset's microphone is a poor choice: it records as a phone
//! call does, which Whisper hears badly, and while it is open the headset
//! plays everything else as a phone call too.
//!
//! The models are `ggml-<name>.bin` in `~/.local/share/orbit/aigent/models`.
//!
//! Without the file, the four kinds are offered under their own names and
//! those whose program is not installed are left out of the panel's list.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub agent: String,
    pub agents: BTreeMap<String, Agent>,
    pub wake: Vec<String>,
    pub wake_model: String,
    pub model: String,
    pub microphone: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            agent: "claude".into(),
            agents: Kind::ALL.iter().map(|kind| (kind.name().to_owned(), Agent::default())).collect(),
            wake: vec!["orbit".into(), "jarvis".into()],
            wake_model: "base-q5_1".into(),
            model: "small-q5_1".into(),
            microphone: None,
        }
    }
}

/// The coding agents Orbit-Aigent knows how to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Claude,
    Gemini,
    Opencode,
    Kimi,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Claude, Kind::Gemini, Kind::Opencode, Kind::Kimi];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Claude => "claude",
            Kind::Gemini => "gemini",
            Kind::Opencode => "opencode",
            Kind::Kimi => "kimi",
        }
    }

    fn from_name(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Agent {
    /// The kind, when the name is not one.
    pub kind: Option<Kind>,
    /// The program, when it is not the kind's name on the `PATH`.
    pub program: Option<PathBuf>,
    pub dir: Option<PathBuf>,
    /// Added to the kind's own arguments.
    pub args: Vec<String>,
}

impl Config {
    /// Reads the file; a missing one gives the defaults, a broken one too,
    /// with an error in the log.
    pub fn load() -> Self {
        let path = dir().join("aigent.yaml");
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                if err.kind() != std::io::ErrorKind::NotFound {
                    log::error!("aigent: {}: {err}, using defaults", path.display());
                }
                return Self::default();
            }
        };
        match serde_yaml_ng::from_str::<Option<Self>>(&text) {
            Ok(config) => config.unwrap_or_default(),
            Err(err) => {
                log::error!("aigent: {}: {err}, using defaults", path.display());
                Self::default()
            }
        }
    }

    /// The agents whose kind is known and whose program is installed, by
    /// name.
    pub fn usable(&self) -> Vec<String> {
        self.agents
            .iter()
            .filter(|(name, agent)| match agent.kind(name) {
                Some(kind) => agent.find_program(kind).is_some(),
                None => {
                    log::warn!("aigent: agent {name} has no kind, leaving it out");
                    false
                }
            })
            .map(|(name, _)| name.clone())
            .collect()
    }
}

impl Agent {
    pub fn kind(&self, name: &str) -> Option<Kind> {
        self.kind.or_else(|| Kind::from_name(name))
    }

    /// The program to run: `program`, or the kind's name looked up on the
    /// `PATH`.
    pub fn find_program(&self, kind: Kind) -> Option<PathBuf> {
        if let Some(program) = &self.program {
            return is_executable(program).then(|| program.clone());
        }
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|dir| dir.join(kind.name()))
            .find(|candidate| is_executable(candidate))
    }
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// `$XDG_CONFIG_HOME/orbit`, falling back to `~/.config/orbit`.
fn dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"))
        .join("orbit")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_kinds_unless_told() {
        let config: Config = serde_yaml_ng::from_str(
            "agent: work\nagents:\n  gemini: {}\n  work: {kind: claude, args: [--model, opus]}\n  odd: {}\n",
        )
        .unwrap();
        assert_eq!(config.agents["gemini"].kind("gemini"), Some(Kind::Gemini));
        assert_eq!(config.agents["work"].kind("work"), Some(Kind::Claude));
        assert_eq!(config.agents["odd"].kind("odd"), None);
        assert_eq!(config.wake, ["orbit", "jarvis"]);
    }

    #[test]
    fn the_defaults_offer_the_four_kinds() {
        let config = Config::default();
        let names: Vec<&str> = config.agents.keys().map(String::as_str).collect();
        assert_eq!(names, ["claude", "gemini", "kimi", "opencode"]);
    }
}
