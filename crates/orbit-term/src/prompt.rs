//! What the prompt shows above the command line, after the user's bash
//! prompt (`~/.shell/30-prompt.sh`):
//!
//!   [ date time ][ {exit} ][ <<git>> ][ user@host ][ directory ]
//!
//! The git part is git's own `__git_ps1` with the user's settings: branch,
//! `*` unstaged, `+` staged, `$` stash, `%` untracked, and `=` `<` `>` `<>`
//! against the upstream — read off the GTK thread after each command.

use std::path::Path;

/// `__git_ps1` for `cwd`, empty outside a repository.
pub fn git(cwd: &Path) -> String {
    const SCRIPT: &str = r#"[ -r /usr/lib/git-core/git-sh-prompt ] || exit
. /usr/lib/git-core/git-sh-prompt
GIT_PS1_SHOWDIRTYSTATE=1 GIT_PS1_SHOWSTASHSTATE=1 GIT_PS1_SHOWUNTRACKEDFILES=1 GIT_PS1_SHOWUPSTREAM=auto
__git_ps1 '%s'
"#;
    let out = std::process::Command::new("bash")
        .args(["--noprofile", "--norc", "-c", SCRIPT])
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    match out {
        Ok(out) if !out.stdout.is_empty() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        // Without git's prompt script: the branch alone.
        _ => std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(cwd)
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default(),
    }
}

/// `user@host`, as `\u@\h`.
pub fn user_host() -> String {
    let user = std::env::var("USER").unwrap_or_default();
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
    let host = host.trim().split('.').next().unwrap_or_default().to_string();
    format!("{user}@{host}")
}

/// Whether the user is root (the part turns red, as in the bash prompt).
pub fn root() -> bool {
    rustix::process::geteuid().is_root()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_in_and_out_of_a_repository() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(!git(repo).is_empty());
        assert_eq!(git(Path::new("/")), "");
        assert!(user_host().contains('@'));
    }
}
