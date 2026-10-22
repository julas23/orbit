//! One try at authenticating, through polkit's own helper,
//! `polkit-agent-helper-1`: setuid root, it runs PAM for the identity and,
//! on success, tells polkitd itself (the agent never sees a secret it
//! could forge). It is spoken to by lines: the cookie first, then PAM's
//! questions on its stdout ("PAM_PROMPT_ECHO_OFF Password:") answered on
//! its stdin, and at the end "SUCCESS" or "FAILURE".

use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
};

/// Where Debian (and most) put it; the first one found is used.
const HELPERS: [&str; 2] = ["/usr/lib/polkit-1/polkit-agent-helper-1", "/usr/libexec/polkit-agent-helper-1"];

/// What the helper says.
#[derive(Debug)]
pub enum Said {
    /// PAM asks; the answer is secret unless `echo`.
    Prompt { text: String, echo: bool },
    Info(String),
    Error(String),
    /// The end: authenticated or not.
    Done(bool),
}

pub struct Helper {
    child: Child,
    stdin: ChildStdin,
}

impl Helper {
    /// Starts a try for `user` (the identity's login name). What the
    /// helper says arrives through `said`, from a thread; the last thing
    /// is always `Done`.
    pub fn start(user: &str, cookie: &str, said: async_channel::Sender<Said>) -> std::io::Result<Self> {
        let path = HELPERS.into_iter().find(|p| Path::new(p).exists()).unwrap_or(HELPERS[0]);
        let mut child = Command::new(path)
            .arg(user)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let mut stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        writeln!(stdin, "{cookie}")?;
        std::thread::spawn(move || {
            let mut done = false;
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Some(said_now) = parse(&line) else {
                    log::debug!("helper: {line}");
                    continue;
                };
                done = matches!(said_now, Said::Done(_));
                if said.send_blocking(said_now).is_err() || done {
                    break;
                }
            }
            if !done {
                let _ = said.send_blocking(Said::Done(false));
            }
        });
        Ok(Self { child, stdin })
    }

    /// Answers the question asked, then forgets the answer.
    pub fn answer(&mut self, mut text: String) {
        text.push('\n');
        if let Err(err) = self.stdin.write_all(text.as_bytes()).and_then(|()| self.stdin.flush()) {
            log::warn!("helper: {err}");
        }
        wipe(text);
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        // A try given up: the helper goes, and polkitd hears nothing.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn parse(line: &str) -> Option<Said> {
    let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
    let text = || unescape(rest.trim_end());
    Some(match word {
        "PAM_PROMPT_ECHO_OFF" => Said::Prompt { text: text(), echo: false },
        "PAM_PROMPT_ECHO_ON" => Said::Prompt { text: text(), echo: true },
        "PAM_TEXT_INFO" => Said::Info(text()),
        "PAM_ERROR_MSG" => Said::Error(text()),
        "SUCCESS" => Said::Done(true),
        "FAILURE" => Said::Done(false),
        _ => return None,
    })
}

/// The helper escapes its messages as `g_strescape` does; the common
/// escapes are undone (`g_strcompress`).
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Overwrites a secret before its memory is freed.
pub fn wipe(text: String) {
    let mut bytes = text.into_bytes();
    for b in bytes.iter_mut() {
        // Safety: a valid, exclusive pointer into the vector.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_helper_lines_are_read() {
        assert!(matches!(parse("PAM_PROMPT_ECHO_OFF Senha: "), Some(Said::Prompt { echo: false, text }) if text == "Senha:"));
        assert!(matches!(parse("PAM_ERROR_MSG a\\nb"), Some(Said::Error(text)) if text == "a\nb"));
        assert!(matches!(parse("SUCCESS"), Some(Said::Done(true))));
        assert!(matches!(parse("FAILURE"), Some(Said::Done(false))));
        assert!(parse("something else").is_none());
    }
}
