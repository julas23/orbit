//! Interactive programs — editors, monitors, pagers, remote shells, sudo's
//! password — run in a real terminal (VTE) that takes the output box's
//! place and the keyboard until they end.
//!
//! They run outside the session (a terminal program needs a terminal of
//! its own), but as if in it: in its directory, with its exported variables
//! (saved after each command, `session::Session::env_file`) and the
//! user's aliases.
//!
//! Which commands: any part of the line (`a | b && c`) whose first word —
//! past `VAR=value`, `time`, `nice`, `env`, `command` — is in the list
//! below or in `term.yaml`'s `interactive`; the shells and interpreters
//! only without arguments (`python script.py` just runs); and git when it
//! would open an editor or ask (`commit` without `-m`, `rebase -i`,
//! `add -p`). Anything else can be sent there with Ctrl+Enter.

use std::path::Path;

use gtk::{gdk, gio, glib};
use vte4::prelude::*;

const ALWAYS: &[&str] = &[
    "vi", "vim", "nvim", "nano", "emacs", "micro", "hx", "mc", "ranger", "yazi", "lf", "nnn", "htop", "btop", "top", "atop",
    "iotop", "nvtop", "less", "more", "most", "man", "info", "ssh", "mosh", "telnet", "ftp", "sftp", "sudo", "su", "passwd",
    "watch", "tmux", "screen", "ncdu", "nmtui", "alsamixer", "pulsemixer", "fzf", "crontab", "visudo", "vipw", "psql",
    "mysql", "mariadb", "sqlite3", "tig", "lazygit",
];
/// Interactive when run with no arguments.
const BARE: &[&str] = &["bash", "sh", "zsh", "fish", "python", "python3", "ipython", "node", "irb", "bc", "lua", "ghci"];
/// Words that run the next one.
const PREFIXES: &[&str] = &["time", "nice", "env", "command", "exec", "nohup"];

/// Whether `line` needs a terminal.
pub fn wanted(line: &str, extra: &[String]) -> bool {
    line.split(['|', ';', '&', '(', ')', '\n']).any(|part| {
        let words: Vec<&str> = part
            .split_whitespace()
            .skip_while(|w| w.contains('=') && !w.starts_with('-') || PREFIXES.contains(w))
            .collect();
        let Some(first) = words.first() else { return false };
        let name = first.rsplit('/').next().unwrap_or(first);
        let args = &words[1..];
        ALWAYS.contains(&name)
            || extra.iter().any(|e| e == name)
            || (BARE.contains(&name) && args.is_empty())
            || (name == "git" && git_asks(args))
    })
}

/// git subcommands that open an editor or ask.
fn git_asks(args: &[&str]) -> bool {
    let has = |flag: &str| args.iter().any(|a| *a == flag || a.starts_with(&format!("{flag}=")));
    match args.first().copied() {
        Some("commit") => !(has("-m") || has("--message") || has("-F") || has("--file") || has("--no-edit") || args.iter().any(|a| a.starts_with("-m"))),
        Some("rebase") => has("-i") || has("--interactive"),
        Some("add" | "checkout" | "reset" | "restore" | "stash") => has("-p") || has("--patch") || has("-i") || has("--interactive"),
        Some("tag") => has("-a") && !has("-m"),
        Some("merge") => has("--edit") || has("-e"),
        _ => false,
    }
}

/// The terminal in the output box's colours and font.
pub fn style(terminal: &vte4::Terminal, font: &gtk::pango::FontDescription) {
    let rgba = |s: &str| gdk::RGBA::parse(s).unwrap_or(gdk::RGBA::BLACK);
    let palette: Vec<gdk::RGBA> = crate::ansi::PALETTE.iter().map(|c| rgba(c)).collect();
    let palette: Vec<&gdk::RGBA> = palette.iter().collect();
    terminal.set_colors(Some(&rgba("#d8dee9")), Some(&rgba("#1b1f27")), &palette);
    terminal.set_font(Some(font));
    terminal.set_scrollback_lines(10_000);
    terminal.set_mouse_autohide(true);
}

/// Runs `command` in `terminal`, in `cwd`, with the session's variables.
pub fn spawn(terminal: &vte4::Terminal, command: &str, cwd: &Path, env_file: Option<&Path>) {
    terminal.reset(true, true);
    let mut script = String::from(crate::session::INIT);
    if let Some(env) = env_file {
        script.push_str(&format!(". {} 2>/dev/null\n", crate::session::quote(&env.to_string_lossy())));
    }
    script.push_str(&format!("cd {} 2>/dev/null\neval {}\n", crate::session::quote(&cwd.to_string_lossy()), crate::session::quote(command)));
    let cwd = cwd.to_string_lossy().into_owned();
    let weak = terminal.downgrade();
    terminal.spawn_async(
        vte4::PtyFlags::DEFAULT,
        Some(&cwd),
        &["bash", "--noprofile", "--norc", "-c", &script],
        &[],
        glib::SpawnFlags::SEARCH_PATH,
        || {},
        -1,
        None::<&gio::Cancellable>,
        move |result| {
            if let Err(err) = result {
                log::error!("term: starting the terminal: {err}");
                if let Some(terminal) = weak.upgrade() {
                    terminal.feed(format!("{err}\r\n").as_bytes());
                    // As if it ended at once, with an error.
                    terminal.emit_by_name::<()>("child-exited", &[&(127 << 8)]);
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_commands_take_the_terminal() {
        let w = |l: &str| wanted(l, &["myrepl".to_string()]);
        assert!(w("vim notes.txt") && w("sudo apt update") && w("ls | less") && w("FOO=1 htop") && w("time ssh host"));
        assert!(w("python") && !w("python3 script.py") && !w("bash -c 'echo hi'"));
        assert!(w("git commit") && !w("git commit -m 'x'") && !w("git commit -mfix") && w("git rebase -i HEAD~3") && !w("git log"));
        assert!(w("/usr/bin/nano x") && w("myrepl") && !w("ls -la") && !w("echo vim"));
    }
}
