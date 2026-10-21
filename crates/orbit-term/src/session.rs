//! The shell behind the prompt: one bash that lives as long as the window,
//! so `cd`, `export` and the user's aliases (`~/.shell/*.sh`) hold from one
//! command to the next.
//!
//! bash reads commands from a pipe. Its stdout is a pseudo-terminal, so
//! programs colour their output as in a terminal (`ls`, `grep`, `git`
//! colour only for one), sized to the output box, with no `\n` → `\r\n`
//! and no pager; its stderr is a pipe of its own, so the window knows a
//! command wrote errors. Each command goes through `eval` (a syntax error
//! fails the command, not the session) with stdin from /dev/null, followed
//! by an end mark on both streams carrying the exit code and the directory
//! it left — the stdout mark says the command is over, the stderr one that
//! every error it wrote has come. The session's exported variables are
//! saved after each command for the interactive programs (`interactive`),
//! which run outside it.
//!
//! A command is interrupted by sending SIGINT to bash's children (bash
//! itself is not touched, so the session goes on); `exit` ends bash, and a
//! new session starts where the old one was.

use std::{
    io::{Read, Write},
    os::fd::{AsFd, OwnedFd},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use rustix::{
    fs::{Mode, OFlags},
    pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt},
    termios::{OptionalActions, OutputModes, Winsize, tcgetattr, tcsetattr, tcsetwinsize},
};

/// What the session tells the window.
#[derive(Debug)]
pub enum Event {
    /// Bytes the running command wrote to stdout or stderr.
    Out(Vec<u8>),
    Err(Vec<u8>),
    /// The command ended with `code`, leaving the shell in `cwd`.
    Done { code: i32, cwd: PathBuf },
    /// Everything the command wrote to stderr has come.
    ErrDone,
    /// bash itself is gone (`exit`, killed).
    Gone,
}

pub struct Session {
    child: Child,
    stdin: ChildStdin,
    master: OwnedFd,
    token: String,
    /// Where the session's exported variables are saved.
    pub env_file: PathBuf,
}

/// Sources the user's shell files: what makes `ll` work here as in a
/// terminal. Their complaints about not being interactive go nowhere.
pub const INIT: &str = r#"shopt -s expand_aliases
for __orbit_rc in "$HOME"/.shell/*.sh; do [ -r "$__orbit_rc" ] && . "$__orbit_rc"; done 2>/dev/null
[ -d "$HOME/.shell" ] || { [ -r "$HOME/.bash_aliases" ] && . "$HOME/.bash_aliases"; } 2>/dev/null
unset __orbit_rc PROMPT_COMMAND
"#;

impl Session {
    /// Starts bash in `cwd`; its events go to `events`.
    pub fn start(cwd: &Path, state: &Path, events: async_channel::Sender<Event>) -> anyhow::Result<Self> {
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC)?;
        grantpt(&master)?;
        unlockpt(&master)?;
        let name = ptsname(&master, Vec::new())?;
        let slave = rustix::fs::open(name.as_c_str(), OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC, Mode::empty())?;
        // Lines end in "\n" only, as a pipe's would.
        let mut termios = tcgetattr(&slave)?;
        termios.output_modes.remove(OutputModes::OPOST);
        tcsetattr(&slave, OptionalActions::Now, &termios)?;
        set_size(&master, 200, 50);

        let (err_read, err_write) = std::io::pipe()?;
        let token = format!("orbit-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.subsec_nanos()));
        let env_file = state.join(format!("env-{}", std::process::id()));
        let mut child = Command::new("bash")
            .args(["--noprofile", "--norc"])
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::from(slave))
            .stderr(err_write)
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            // Nothing pages here: the output box scrolls.
            .env("PAGER", "cat")
            .env("GIT_PAGER", "cat")
            .env("SYSTEMD_PAGER", "")
            .env("MANPAGER", "cat")
            .env("ORBIT_TERM", "1")
            .spawn()?;
        let mut stdin = child.stdin.take().expect("piped");
        stdin.write_all(INIT.as_bytes())?;
        // The first mark ends the start, whatever the files printed.
        stdin.write_all(epilogue(&token, &env_file).as_bytes())?;
        stdin.flush()?;

        let out = std::fs::File::from(master.try_clone()?);
        spawn_reader(out, token.clone(), events.clone(), true);
        spawn_reader(std::fs::File::from(OwnedFd::from(err_read)), token.clone(), events, false);
        Ok(Self { child, stdin, master, token, env_file })
    }

    /// Runs `line`, `cols` × `rows` characters being what the output box
    /// shows.
    pub fn run(&mut self, line: &str, cols: u16, rows: u16) -> std::io::Result<()> {
        set_size(&self.master, cols, rows);
        let script = format!("eval {} </dev/null\n{}", quote(line), epilogue(&self.token, &self.env_file));
        self.stdin.write_all(script.as_bytes())?;
        self.stdin.flush()
    }

    /// Interrupts the running command (SIGINT, or SIGKILL when `hard`).
    pub fn interrupt(&self, hard: bool) {
        use rustix::process::{Pid, Signal, kill_process};
        let pid = self.child.id();
        let children = std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")).unwrap_or_default();
        let signal = if hard { Signal::KILL } else { Signal::INT };
        for child in children.split_whitespace().filter_map(|p| p.parse::<i32>().ok()) {
            if let Some(pid) = Pid::from_raw(child) {
                let _ = kill_process(pid, signal);
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.env_file);
    }
}

/// After every command: the end marks (exit code and directory on stdout,
/// a bare one on stderr) and the exported variables.
fn epilogue(token: &str, env_file: &Path) -> String {
    format!(
        "__orbit_rc=$?; export -p >{env} 2>/dev/null; printf '\\036{token} %d %s\\036' \"$__orbit_rc\" \"$PWD\"; printf '\\036{token}\\036' >&2\n",
        env = quote(&env_file.to_string_lossy()),
    )
}

/// `text` as one word for bash.
pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

fn set_size(master: &impl AsFd, cols: u16, rows: u16) {
    let size = Winsize { ws_row: rows.max(1), ws_col: cols.max(1), ws_xpixel: 0, ws_ypixel: 0 };
    if let Err(err) = tcsetwinsize(master, size) {
        log::warn!("term: window size: {err}");
    }
}

/// Reads one stream, handing on its bytes and taking the end marks out.
fn spawn_reader(mut stream: std::fs::File, token: String, events: async_channel::Sender<Event>, stdout: bool) {
    std::thread::spawn(move || {
        let mut scanner = Scanner::new(&token);
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            for piece in scanner.feed(&buf[..n]) {
                let event = match piece {
                    Piece::Text(bytes) if stdout => Event::Out(bytes),
                    Piece::Text(bytes) => Event::Err(bytes),
                    Piece::Mark(rest) if stdout => {
                        let rest = String::from_utf8_lossy(&rest).into_owned();
                        let (code, cwd) = rest.split_once(' ').unwrap_or((&rest, ""));
                        Event::Done { code: code.trim().parse().unwrap_or(-1), cwd: PathBuf::from(cwd) }
                    }
                    // The stderr mark only says stderr has caught up.
                    Piece::Mark(_) => Event::ErrDone,
                };
                if events.send_blocking(event).is_err() {
                    return;
                }
            }
        }
        if stdout {
            let _ = events.send_blocking(Event::Gone);
        }
    });
}

/// A piece of a stream: text, or an end mark with what follows the token.
#[derive(Debug, PartialEq)]
enum Piece {
    Text(Vec<u8>),
    Mark(Vec<u8>),
}

/// Finds `\x1e<token>…\x1e` marks in a stream that comes in chunks.
struct Scanner {
    start: Vec<u8>,
    pending: Vec<u8>,
}

impl Scanner {
    fn new(token: &str) -> Self {
        let mut start = vec![0x1e];
        start.extend_from_slice(token.as_bytes());
        Self { start, pending: Vec::new() }
    }

    fn feed(&mut self, chunk: &[u8]) -> Vec<Piece> {
        self.pending.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            match find(&self.pending, &self.start) {
                Some(at) => {
                    let body = at + self.start.len();
                    let Some(end) = self.pending[body..].iter().position(|b| *b == 0x1e) else {
                        // The mark goes on in the next chunk.
                        if at > 0 {
                            out.push(Piece::Text(self.pending.drain(..at).collect()));
                        }
                        return out;
                    };
                    if at > 0 {
                        out.push(Piece::Text(self.pending[..at].to_vec()));
                    }
                    let mark = self.pending[body..body + end].strip_prefix(b" ").unwrap_or(&self.pending[body..body + end]).to_vec();
                    out.push(Piece::Mark(mark));
                    self.pending.drain(..body + end + 1);
                }
                None => {
                    // Keep what may be the start of a mark cut in two.
                    let keep = (1..self.start.len().min(self.pending.len() + 1))
                        .rev()
                        .find(|&k| self.pending.ends_with(&self.start[..k]))
                        .unwrap_or(0);
                    let cut = self.pending.len() - keep;
                    if cut > 0 {
                        out.push(Piece::Text(self.pending.drain(..cut).collect()));
                    }
                    return out;
                }
            }
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The path with the home directory as `~`.
pub fn tilde(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_are_found_across_chunks() {
        let mut s = Scanner::new("T");
        assert_eq!(s.feed(b"hello\n"), vec![Piece::Text(b"hello\n".to_vec())]);
        assert_eq!(s.feed(b"ab\x1e"), vec![Piece::Text(b"ab".to_vec())]);
        assert_eq!(s.feed(b"T 0 /h"), vec![]);
        assert_eq!(s.feed(b"ome\x1enext"), vec![Piece::Mark(b"0 /home".to_vec()), Piece::Text(b"next".to_vec())]);
        // A lone \x1e that is not a mark is text.
        assert_eq!(s.feed(b"\x1ex"), vec![Piece::Text(b"\x1ex".to_vec())]);
    }

    #[test]
    fn a_command_runs_in_the_session() {
        let (tx, rx) = async_channel::unbounded();
        let dir = std::env::temp_dir();
        let mut session = Session::start(Path::new("/"), &dir, tx).unwrap();
        let collect = |rx: &async_channel::Receiver<Event>| {
            let (mut out, mut err, mut done, mut err_done) = (Vec::new(), Vec::new(), None, false);
            loop {
                match rx.recv_blocking().unwrap() {
                    Event::Out(b) => out.extend(b),
                    Event::Err(b) => err.extend(b),
                    Event::Done { code, cwd } => done = Some((code, cwd)),
                    Event::ErrDone => err_done = true,
                    Event::Gone => panic!("bash went"),
                }
                if err_done && let Some((code, cwd)) = done.take() {
                    return (out, err, code, cwd);
                }
            }
        };
        collect(&rx); // the start
        session.run("cd /tmp && echo hi; [ -t 1 ] && echo tty", 80, 24).unwrap();
        let (out, _, code, cwd) = collect(&rx);
        assert_eq!(String::from_utf8_lossy(&out), "hi\ntty\n");
        assert_eq!((code, cwd), (0, PathBuf::from("/tmp")));
        session.run("pwd; ls /nonexistent", 80, 24).unwrap();
        let (out, err, code, _) = collect(&rx);
        assert_eq!(String::from_utf8_lossy(&out), "/tmp\n");
        assert!(!err.is_empty());
        assert_ne!(code, 0);
        // A syntax error fails the whole line, as in a terminal.
        session.run("echo no; if then", 80, 24).unwrap();
        let (out, err, code, _) = collect(&rx);
        assert!(out.is_empty() && !err.is_empty() && code != 0);
        // Still alive after the syntax error.
        session.run("x=1; echo $((x+1))", 80, 24).unwrap();
        assert_eq!(String::from_utf8_lossy(&collect(&rx).0), "2\n");
    }
}
