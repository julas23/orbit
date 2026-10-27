//! `aigent.yaml`, in `$XDG_CONFIG_HOME/orbit` (`~/.config/orbit`):
//!
//! ```yaml
//! agent: claude            # the one in use at start
//! agents:                  # every one that can be chosen
//!   claude: {}             # the name is the kind: claude, codex, gemini, opencode, kimi
//!   work:                  # or a name of one's own, with its kind
//!     kind: claude
//!     program: /opt/claude/bin/claude
//!     dir: ~/Vault/GIT     # where it works; the home by default
//!     args: [--model, opus]
//! wake: [orbit, jarvis]    # the words that wake it, in any language
//! wake_model: small-q5_1   # Whisper, hearing for the wake word
//! model: large-v3-turbo-q5_0   # and hearing the request
//! microphone: alsa_input.usb-…   # a source's node name (`wpctl inspect`);
//!                          # the default one when left out
//! hints: "Que horas são? …"  # Whisper's prompt for a request: requests as
//!                          # they are often said, so short ones are heard
//!                          # right ("que horas são", not "que oração")
//! voices:                  # Piper's, in ~/.local/share/orbit/aigent/voices
//!   pt: pt_BR-faber-medium
//!   en: en_US-ryan-medium
//! chime: true              # two short notes when woken: it listens now
//! voice:                   # the local voice (voice.rs): left out, none
//!   model: gemma-3-4b-it-Q4_K_M   # models/<name>.gguf, or a file's path
//!   server: llama-server   # llama.cpp's server, on the PATH or a path
//!   env:                   # for the server, as where its GPU backend is
//!     GGML_BACKEND_PATH: /usr/local/lib/ollama/vulkan/libggml-vulkan.so
//!   gpu_layers: 99         # all on the GPU
//!   context: 8192
//! ```
//!
//! A Bluetooth headset's microphone is a poor choice: it records as a phone
//! call does, which Whisper hears badly, and while it is open the headset
//! plays everything else as a phone call too. With speakers, the best is
//! `orbit-aigent-mic`, the microphone with the speakers' sound taken out
//! (data/pipewire/60-orbit-aigent-aec.conf): otherwise it hears itself.
//!
//! The models are `ggml-<name>.bin` in `~/.local/share/orbit/aigent/models`.
//! Their defaults, above, are for Whisper on the GPU (the `vulkan`
//! feature): small in 0.4 s and large-v3-turbo in 1.7 s on the hp's
//! integrated Radeon. On the CPU they would take 1.4 and over 5 s, so
//! without the feature they are base and small.
//!
//! Without the file, the five kinds are offered under their own names and
//! those whose program is not installed are left out of the panel's list.
//! A program is looked for on the `PATH`, then where its installer puts it
//! (`~/.kimi-code/bin`, `~/.opencode/bin`, …), which is often only on the
//! `PATH` of a shell.

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
    pub hints: String,
    /// Piper's voice for each language, `voices/<name>.onnx`.
    pub voices: std::collections::HashMap<String, String>,
    /// A short sound when woken.
    pub chime: bool,
    /// The local voice, when there is one.
    pub voice: Option<Voice>,
}

/// The local model that talks (`voice.rs`).
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Voice {
    pub model: String,
    pub server: PathBuf,
    pub env: BTreeMap<String, String>,
    pub gpu_layers: u32,
    pub context: u32,
}

impl Default for Voice {
    fn default() -> Self {
        Self { model: "gemma-3-4b-it-Q4_K_M".into(), server: "llama-server".into(), env: BTreeMap::new(), gpu_layers: 99, context: 8192 }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            agent: "claude".into(),
            agents: Kind::ALL.iter().map(|kind| (kind.name().to_owned(), Agent::default())).collect(),
            wake: vec!["orbit".into(), "jarvis".into()],
            wake_model: if cfg!(feature = "vulkan") { "small-q5_1" } else { "base-q5_1" }.into(),
            model: if cfg!(feature = "vulkan") { "large-v3-turbo-q5_0" } else { "small-q5_1" }.into(),
            microphone: None,
            voices: Default::default(),
            chime: true,
            voice: None,
            hints: "Que horas são? Abra o terminal. Quanto espaço livre tem no disco? \
                    What time is it? Open the browser."
                .into(),
        }
    }
}

/// The coding agents Orbit-Aigent knows how to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Claude,
    /// OpenAI's Codex CLI.
    Codex,
    Gemini,
    Opencode,
    Kimi,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Claude, Kind::Codex, Kind::Gemini, Kind::Opencode, Kind::Kimi];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Claude => "claude",
            Kind::Codex => "codex",
            Kind::Gemini => "gemini",
            Kind::Opencode => "opencode",
            Kind::Kimi => "kimi",
        }
    }

    /// As it is said: "vou pedir ao Claude".
    pub fn said(self) -> &'static str {
        match self {
            Kind::Claude => "Claude",
            Kind::Codex => "Codex",
            Kind::Gemini => "Gemini",
            Kind::Opencode => "OpenCode",
            Kind::Kimi => "Kimi",
        }
    }

    /// Where the kind's own installer puts it, under the home, when that is
    /// not on the session's `PATH` (it only adds it in `~/.bashrc`).
    fn home_dirs(self) -> &'static [&'static str] {
        match self {
            Kind::Claude => &[".claude/local"],
            Kind::Codex => &[".npm-global/bin", ".local/bin"],
            Kind::Gemini => &[".npm-global/bin"],
            Kind::Opencode => &[".opencode/bin"],
            Kind::Kimi => &[".kimi-code/bin"],
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
    /// `PATH` and then where its installer puts it.
    pub fn find_program(&self, kind: Kind) -> Option<PathBuf> {
        if let Some(program) = &self.program {
            return is_executable(program).then(|| program.clone());
        }
        let path = std::env::var_os("PATH").unwrap_or_default();
        let home = std::env::var_os("HOME").map(PathBuf::from).filter(|home| home.is_absolute());
        std::env::split_paths(&path)
            .chain(home.iter().flat_map(|home| kind.home_dirs().iter().map(move |dir| home.join(dir))))
            .map(|dir| dir.join(kind.name()))
            .find(|candidate| is_executable(candidate))
    }
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// `~/x` as the home's `x`.
pub fn expand(path: &Path) -> PathBuf {
    match path.strip_prefix("~") {
        Ok(rest) => PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into())).join(rest),
        Err(_) => path.to_owned(),
    }
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
    fn the_voice_takes_its_defaults() {
        let config: Config = serde_yaml_ng::from_str("voice: {server: /opt/llama-server}\n").unwrap();
        let voice = config.voice.unwrap();
        assert_eq!(voice.model, "gemma-3-4b-it-Q4_K_M");
        assert_eq!(voice.server, PathBuf::from("/opt/llama-server"));
        assert!(Config::default().voice.is_none());
    }

    #[test]
    fn the_defaults_offer_the_five_kinds() {
        let config = Config::default();
        let names: Vec<&str> = config.agents.keys().map(String::as_str).collect();
        assert_eq!(names, ["claude", "codex", "gemini", "kimi", "opencode"]);
    }
}
