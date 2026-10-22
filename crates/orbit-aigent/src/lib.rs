//! Orbit-Aigent's protocol, spoken by the daemon (`orbit-aigent`), the
//! panel's aigent module and `orbit-aigent <command>`.
//!
//! One Unix socket, `$XDG_RUNTIME_DIR/orbit-aigent.sock`, one JSON object a
//! line both ways. A client that connects is sent a `Frame` at once and
//! then whenever something changes: `frame_rate` times a second while the
//! sphere moves (listening, speaking), once per change otherwise. A line a
//! client writes is a `Command`.

use std::{
    io::{self, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
};

use serde::{Deserialize, Serialize};

/// Bands in a frame, around the sphere's edge and along its spectrum.
pub const BANDS: usize = 24;

/// Frames a second while the sphere moves.
pub const FRAME_RATE: u32 = 30;

/// What the aigent is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Waiting for the wake word: the sphere is white and still.
    #[default]
    Idle,
    /// Hearing a request: red, the edge moving with the microphone.
    Listening,
    /// The agent is working: white, a slow sweep.
    Thinking,
    /// Saying the answer: yellow, the edge moving with the voice.
    Speaking,
    /// An action that changes something waits for a spoken "yes" or a
    /// click: red, as it listens for the answer.
    Confirm,
}

/// The daemon to its clients.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Frame {
    pub state: State,
    /// The agent in use (its name in `aigent.yaml`), and every one set up.
    pub agent: String,
    pub agents: Vec<String>,
    /// `BANDS` values, 0 to 1; empty while nothing moves.
    pub bands: Vec<f32>,
    /// The last thing heard or said, for the tooltip.
    pub text: String,
}

/// A client to the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Command {
    /// Start listening as if the wake word was heard; stop if already.
    Toggle,
    /// Start listening as if the wake word was heard.
    Listen,
    /// Stop whatever it is doing and go back to waiting.
    Cancel,
    /// The answer to a confirmation.
    Yes,
    No,
    /// Use another agent from now on.
    Agent { name: String },
    /// Listen, think and speak with made-up sound, to see the sphere.
    Demo,
}

/// `$XDG_RUNTIME_DIR/orbit-aigent.sock`.
pub fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    dir.join("orbit-aigent.sock")
}

/// Sends `command` to the daemon and hangs up.
pub fn send(command: &Command) -> io::Result<()> {
    let mut sock = UnixStream::connect(socket_path())?;
    let mut line = serde_json::to_vec(command).map_err(io::Error::other)?;
    line.push(b'\n');
    sock.write_all(&line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_tagged() {
        assert_eq!(serde_json::to_string(&Command::Toggle).unwrap(), r#"{"cmd":"toggle"}"#);
        let agent: Command = serde_json::from_str(r#"{"cmd":"agent","name":"gemini"}"#).unwrap();
        assert_eq!(agent, Command::Agent { name: "gemini".into() });
    }

    #[test]
    fn frames_read_with_fields_missing() {
        let frame: Frame = serde_json::from_str(r#"{"state":"speaking"}"#).unwrap();
        assert_eq!(frame.state, State::Speaking);
        assert!(frame.bands.is_empty());
    }
}
