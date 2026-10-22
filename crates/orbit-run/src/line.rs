//! What the typed line means, apart from GTK: the program it starts with,
//! the word under completion (a program from $PATH first, then files), and
//! the history kept between runs.

use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

/// The program `line` runs: its first word after any `NAME=value`.
pub fn program(line: &str) -> Option<&str> {
    line.split_whitespace().find(|word| !is_assignment(word))
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !name.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// Every program on $PATH, sorted, each once.
pub fn programs() -> Vec<String> {
    let mut names: Vec<String> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default()
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .flatten()
        .filter(|entry| executable(&entry.path()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    names.dedup();
    names
}

fn executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Whether `program` would run: a path to an executable, a program on
/// $PATH, or what the shell knows by itself (`cd`, `exec`, a function).
pub fn exists(program: &str, programs: &[String]) -> bool {
    if program.contains('/') {
        return executable(&expand(program));
    }
    programs.binary_search_by(|p| p.as_str().cmp(program)).is_ok()
        || std::process::Command::new("sh")
            .args(["-c", "command -v -- \"$1\" >/dev/null", "sh", program])
            .status()
            .is_ok_and(|s| s.success())
}

/// `~` and `~/…` as the home folder.
fn expand(path: &str) -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    match path.strip_prefix('~') {
        Some("") => home(),
        Some(rest) if rest.starts_with('/') => home().join(&rest[1..]),
        _ => PathBuf::from(path),
    }
}

/// Completing the last word of `line`: the line with it extended as far
/// as every match agrees (a space after a single program, a `/` after a
/// single folder), and the matches when there are several.
pub fn complete(line: &str, programs: &[String]) -> (String, Vec<String>) {
    let start = line.rfind(char::is_whitespace).map_or(0, |i| i + 1);
    let (head, word) = line.split_at(start);
    let first = program(head).is_none() && !word.contains('/') && !is_assignment(word);
    let matches: Vec<String> = if first {
        programs.iter().filter(|p| p.starts_with(word)).cloned().collect()
    } else {
        files(word)
    };
    let Some(common) = common_prefix(&matches) else { return (line.to_owned(), Vec::new()) };
    let mut done = format!("{head}{common}");
    if let [one] = matches.as_slice() {
        if first {
            done.push(' ');
        } else if expand(one).is_dir() && !one.ends_with('/') {
            done.push('/');
        }
        return (done, Vec::new());
    }
    let shown = matches.iter().map(|m| m.rsplit('/').find(|s| !s.is_empty()).unwrap_or(m).to_owned()).collect();
    (done, shown)
}

/// The files and folders whose path starts with `word` (`~` kept as typed);
/// hidden ones only when the name starts with a dot.
fn files(word: &str) -> Vec<String> {
    let (dir, name) = match word.rfind('/') {
        Some(i) => (&word[..=i], &word[i + 1..]),
        None => ("", word),
    };
    let read = if dir.is_empty() { PathBuf::from(".") } else { expand(dir) };
    let Ok(entries) = std::fs::read_dir(read) else { return Vec::new() };
    let mut found: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with(name) && (name.starts_with('.') || !n.starts_with('.')))
        .map(|n| format!("{dir}{n}"))
        .collect();
    found.sort();
    found
}

fn common_prefix(words: &[String]) -> Option<String> {
    let first = words.first()?;
    let mut end = first.len();
    for word in &words[1..] {
        end = first.char_indices().zip(word.chars()).take_while(|((_, a), b)| a == b).last().map_or(0, |((i, a), _)| i + a.len_utf8()).min(end);
    }
    Some(first[..end].to_owned())
}

/// The lines run before, oldest first, kept in
/// `$XDG_STATE_HOME/orbit/run-history`.
pub struct History {
    path: PathBuf,
    pub lines: Vec<String>,
}

/// How many lines the history keeps.
const KEPT: usize = 200;

impl History {
    pub fn load() -> Self {
        let path = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state"))
            .join("orbit/run-history");
        let lines = std::fs::read_to_string(&path).map(|t| t.lines().map(str::to_owned).collect()).unwrap_or_default();
        Self { path, lines }
    }

    /// `line` becomes the newest (once) and the history is saved.
    pub fn push(&mut self, line: &str) {
        self.lines.retain(|l| l != line);
        self.lines.push(line.to_owned());
        let excess = self.lines.len().saturating_sub(KEPT);
        self.lines.drain(..excess);
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(err) = std::fs::write(&self.path, self.lines.join("\n") + "\n") {
            log::warn!("{}: {err}", self.path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_skips_assignments() {
        assert_eq!(program("GDK_BACKEND=x11 firefox --new-window"), Some("firefox"));
        assert_eq!(program("  kitty -e htop"), Some("kitty"));
        assert_eq!(program("a=b"), None);
        assert_eq!(program("x --opt=1"), Some("x"));
    }

    #[test]
    fn programs_complete_to_what_they_share() {
        let names: Vec<String> = ["firefox", "firefox-esr", "fish", "kitty"].map(String::from).to_vec();
        assert_eq!(complete("fir", &names), ("firefox".into(), vec!["firefox".into(), "firefox-esr".into()]));
        assert_eq!(complete("ki", &names), ("kitty ".into(), vec![]));
        assert_eq!(complete("zz", &names), ("zz".into(), vec![]));
        assert_eq!(complete("FOO=1 ki", &names), ("FOO=1 kitty ".into(), vec![]));
    }

    #[test]
    fn later_words_complete_files() {
        let dir = std::env::temp_dir().join(format!("orbit-run-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("dog.txt"), "").unwrap();
        let base = dir.display().to_string();
        assert_eq!(complete(&format!("ls {base}/doc"), &[]), (format!("ls {base}/docs/"), vec![]));
        assert_eq!(complete(&format!("ls {base}/do"), &[]), (format!("ls {base}/do"), vec!["docs".into(), "dog.txt".into()]));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_shell_knows_its_own() {
        assert!(exists("cd", &[]));
        assert!(exists("/bin/sh", &[]));
        assert!(!exists("no-such-program-anywhere", &[]));
    }
}
