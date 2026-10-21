//! What orbit-term keeps between runs, in `$XDG_STATE_HOME/orbit/term/`
//! (`~/.local/state/orbit/term/`):
//!
//!   outputs.json  the last 50 outputs, newest first — command, directory,
//!                 time and stdout with its colours. A command that failed
//!                 (exit code other than 0, or anything on stderr) is not
//!                 kept, so the list holds only what worked.
//!   commands      every command run, one per line (the prompt's ↑ and ↓),
//!                 the last 1000.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Outputs kept.
pub const OUTPUTS: usize = 50;
/// Commands kept for ↑ and ↓.
const COMMANDS: usize = 1000;
/// An output larger than this is cut (the file stays small to rewrite).
const MAX_OUTPUT: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Output {
    pub command: String,
    pub cwd: String,
    /// Local time, `2026-10-21 18:02`.
    pub time: String,
    pub seconds: f64,
    /// stdout, escape sequences and all.
    pub text: String,
}

pub struct History {
    dir: PathBuf,
    pub outputs: Vec<Output>,
    pub commands: Vec<String>,
}

pub fn state_dir() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state"));
    base.join("orbit").join("term")
}

impl History {
    pub fn load(dir: &Path) -> Self {
        let outputs = std::fs::read_to_string(dir.join("outputs.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).inspect_err(|e| log::warn!("term: outputs.json: {e}")).ok())
            .unwrap_or_default();
        let commands = std::fs::read_to_string(dir.join("commands"))
            .map(|text| text.lines().map(str::to_owned).collect())
            .unwrap_or_default();
        Self { dir: dir.to_path_buf(), outputs, commands }
    }

    /// Notes a command for ↑ and ↓ (not twice in a row).
    pub fn add_command(&mut self, command: &str) {
        if command.trim().is_empty() || self.commands.last().is_some_and(|c| c == command) {
            return;
        }
        self.commands.push(command.to_owned());
        let excess = self.commands.len().saturating_sub(COMMANDS);
        self.commands.drain(..excess);
        let mut text = self.commands.join("\n");
        text.push('\n');
        self.write("commands", &text);
    }

    /// Keeps an output that worked, as the newest.
    pub fn add_output(&mut self, mut output: Output) {
        if output.text.len() > MAX_OUTPUT {
            let mut cut = MAX_OUTPUT;
            while !output.text.is_char_boundary(cut) {
                cut -= 1;
            }
            output.text.truncate(cut);
            output.text.push_str("\n…\n");
        }
        self.outputs.insert(0, output);
        self.outputs.truncate(OUTPUTS);
        match serde_json::to_string(&self.outputs) {
            Ok(text) => self.write("outputs.json", &text),
            Err(err) => log::warn!("term: saving the outputs: {err}"),
        }
    }

    fn write(&self, name: &str, text: &str) {
        let path = self.dir.join(name);
        let temp = path.with_extension("new");
        let written = std::fs::create_dir_all(&self.dir)
            .and_then(|()| std::fs::write(&temp, text))
            .and_then(|()| std::fs::rename(&temp, &path));
        if let Err(err) = written {
            log::warn!("term: writing {}: {err}", path.display());
        }
    }
}

/// Whether an output is kept: the user's rule for "ERR".
pub fn worked(code: i32, wrote_errors: bool) -> bool {
    code == 0 && !wrote_errors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(n: usize) -> Output {
        Output { command: format!("echo {n}"), cwd: "~".into(), time: "t".into(), seconds: 0.0, text: n.to_string() }
    }

    #[test]
    fn fifty_newest_first_and_reloaded() {
        let dir = std::env::temp_dir().join(format!("orbit-term-test-{}", std::process::id()));
        let mut h = History::load(&dir);
        for n in 0..60 {
            h.add_output(output(n));
        }
        h.add_command("ls");
        h.add_command("ls");
        h.add_command("pwd");
        let h = History::load(&dir);
        assert_eq!(h.outputs.len(), OUTPUTS);
        assert_eq!(h.outputs[0].command, "echo 59");
        assert_eq!(h.commands, ["ls", "pwd"]);
        std::fs::remove_dir_all(dir).unwrap();
        assert!(worked(0, false) && !worked(1, false) && !worked(0, true));
    }
}
