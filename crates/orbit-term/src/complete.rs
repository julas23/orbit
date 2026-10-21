//! Tab completion in the prompt, for the word before the cursor:
//!
//!   first word (or after `|`, `;`, `&&`, `sudo`, `time`…)
//!               the session's commands: aliases, functions, builtins,
//!               keywords and programs in PATH (`compgen`, read once at
//!               start and again now and then); with a `/`, files
//!   cd, pushd   directories
//!   git X       git's commands; after checkout, switch, merge, rebase,
//!               branch, push, pull, log, diff, reset, show, cherry-pick,
//!               the branches and tags too
//!   anything    files and directories (hidden ones when the word starts
//!               with a dot), `~` understood
//!
//! One candidate is put in at once (a directory with `/`, the rest with a
//! space); several put in what they share and, when that adds nothing,
//! are listed under the prompt, Tab and ↓ ↑ going through them.

use std::path::{Path, PathBuf};

/// Words after which a command comes.
const RUNNERS: &[&str] = &["sudo", "time", "nice", "env", "command", "exec", "nohup", "xargs", "watch", "doas"];
const GIT_COMMANDS: &[&str] = &[
    "add", "am", "bisect", "blame", "branch", "checkout", "cherry-pick", "clean", "clone", "commit", "config", "describe",
    "diff", "fetch", "grep", "init", "log", "merge", "mv", "pull", "push", "rebase", "reflog", "remote", "reset", "restore",
    "revert", "rm", "show", "stash", "status", "submodule", "switch", "tag", "worktree",
];
/// git commands that take a branch.
const GIT_REFS: &[&str] = &["checkout", "switch", "merge", "rebase", "branch", "push", "pull", "log", "diff", "reset", "show", "cherry-pick"];

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// What goes in the prompt in place of the word.
    pub insert: String,
    /// What the list shows.
    pub show: String,
    /// A directory: no space after it, so the path can go on.
    pub dir: bool,
}

/// Where the word under completion starts (a byte index into `before`) and
/// the candidates for it. `before` is the prompt up to the cursor.
pub fn complete(before: &str, cwd: &Path, commands: &[String]) -> (usize, Vec<Candidate>) {
    let start = word_start(before);
    let word = &before[start..];
    let previous: Vec<&str> = before[..start]
        .rsplit(['|', ';', '&', '(', '\n'])
        .next()
        .unwrap_or("")
        .split_whitespace()
        .skip_while(|w| RUNNERS.contains(w) || (w.contains('=') && !w.starts_with('-')))
        .collect();
    let candidates = match previous.as_slice() {
        [] if !word.contains('/') && !word.starts_with('~') => commands
            .iter()
            .filter(|c| c.starts_with(word))
            .map(|c| Candidate { insert: c.clone(), show: c.clone(), dir: false })
            .collect(),
        [] => files(word, cwd, false),
        ["cd" | "pushd"] => files(word, cwd, true),
        ["git"] => GIT_COMMANDS
            .iter()
            .filter(|c| c.starts_with(word))
            .map(|c| Candidate { insert: c.to_string(), show: c.to_string(), dir: false })
            .collect(),
        ["git", sub, ..] if GIT_REFS.contains(sub) && !word.starts_with('-') => {
            let mut all: Vec<Candidate> = git_refs(cwd)
                .into_iter()
                .filter(|r| r.starts_with(word))
                .map(|r| Candidate { insert: r.clone(), show: r, dir: false })
                .collect();
            all.extend(files(word, cwd, false));
            all
        }
        _ => files(word, cwd, false),
    };
    (start, candidates)
}

/// The start of the last word: after the last unescaped space or operator.
fn word_start(before: &str) -> usize {
    let mut start = 0;
    let mut escaped = false;
    for (i, c) in before.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c.is_whitespace() || matches!(c, '|' | ';' | '&' | '(' | ')' | '<' | '>' | '=') {
            start = i + c.len_utf8();
        }
    }
    start
}

/// Files and directories whose path starts with `word` (only directories
/// with `dirs`).
fn files(word: &str, cwd: &Path, dirs: bool) -> Vec<Candidate> {
    let unescaped = unescape(word);
    let (dir_part, prefix) = match unescaped.rfind('/') {
        Some(i) => (&unescaped[..=i], &unescaped[i + 1..]),
        None => ("", unescaped.as_str()),
    };
    let base: PathBuf = if let Some(rest) = dir_part.strip_prefix('~') {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest.trim_start_matches('/'))
    } else if dir_part.starts_with('/') {
        PathBuf::from(dir_part)
    } else {
        cwd.join(dir_part)
    };
    let Ok(entries) = std::fs::read_dir(&base) else { return Vec::new() };
    let mut out: Vec<Candidate> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with(prefix) || (name.starts_with('.') && !prefix.starts_with('.')) {
                return None;
            }
            // Following links: a link to a directory is one.
            let dir = e.path().is_dir();
            if dirs && !dir {
                return None;
            }
            let show = if dir { format!("{name}/") } else { name.clone() };
            Some(Candidate { insert: format!("{}{}", escape(dir_part), escape(&name)), show, dir })
        })
        .collect();
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.show.to_lowercase().cmp(&b.show.to_lowercase())));
    out
}

fn git_refs(cwd: &Path) -> Vec<String> {
    let out = std::process::Command::new("git")
        .args(["for-each-ref", "--format=%(refname:short)", "refs/heads", "refs/remotes", "refs/tags"])
        .current_dir(cwd)
        .stderr(std::process::Stdio::null())
        .output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.ends_with("/HEAD")).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// A name as a shell word: spaces and the shell's own characters escaped.
fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_whitespace() || "'\"\\$`!&|;()<>*?[]{}#".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn unescape(word: &str) -> String {
    let mut out = String::new();
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// What every candidate starts with.
pub fn shared(candidates: &[Candidate]) -> String {
    let Some(first) = candidates.first() else { return String::new() };
    let mut end = first.insert.len();
    for c in &candidates[1..] {
        end = first.insert.char_indices().zip(c.insert.chars()).take_while(|((_, a), b)| a == b).map(|((i, a), _)| i + a.len_utf8()).last().unwrap_or(0).min(end);
    }
    first.insert[..end].to_string()
}

/// The session's commands, as `compgen` lists them (run with the user's
/// files, for their aliases and functions).
pub fn commands() -> Vec<String> {
    let script = format!("{}compgen -A alias -A function -A builtin -A keyword -A command\n", crate::session::INIT);
    let out = std::process::Command::new("bash")
        .args(["--noprofile", "--norc", "-c", &script])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let mut list: Vec<String> = out
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.is_empty() && !l.starts_with('_')).map(str::to_owned).collect())
        .unwrap_or_default();
    list.sort();
    list.dedup();
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_and_contexts() {
        assert_eq!(word_start("ls -la my\\ fi"), 7);
        assert_eq!(word_start("cat a|gre"), 6);
        let commands: Vec<String> = ["grep", "git", "gzip", "ls"].map(String::from).to_vec();
        let dir = std::env::temp_dir().join(format!("orbit-complete-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub dir")).unwrap();
        std::fs::write(dir.join("notes.txt"), "").unwrap();
        std::fs::write(dir.join(".hidden"), "").unwrap();
        let names = |line: &str| complete(line, &dir, &commands).1.into_iter().map(|c| c.insert).collect::<Vec<_>>();
        assert_eq!(names("g"), ["grep", "git", "gzip"]);
        assert_eq!(names("sudo gr"), ["grep"]);
        assert_eq!(names("cat n"), ["notes.txt"]);
        assert_eq!(names("cd "), ["sub\\ dir"]);
        assert_eq!(names("ls ."), [".hidden"]);
        assert_eq!(names("git chec"), ["checkout"]);
        assert_eq!(names("ls sub\\ "), ["sub\\ dir"]);
        std::fs::remove_dir_all(dir).unwrap();
        let c = |s: &str| Candidate { insert: s.into(), show: s.into(), dir: false };
        assert_eq!(shared(&[c("checkout"), c("cherry-pick")]), "che");
        assert_eq!(shared(&[c("ação"), c("açúcar")]), "aç");
    }
}
