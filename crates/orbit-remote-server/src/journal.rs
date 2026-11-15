//! Warnings and errors also go to the systemd journal, as
//! `orbit-remote-server` (SYSLOG_IDENTIFIER): Fail2Ban's `orbit-remote` jail
//! reads the wrong pairing codes there (docs/PLANO-DISTRO.md §12.5), and
//! the session's log file (`~/.local/state/orbit/remote.log`) stays as it
//! was.
//!
//! The journal's native protocol: one datagram a message, `KEY=value`
//! lines, to /run/systemd/journal/socket. Without a journal (a container,
//! a machine without systemd) nothing is sent and nothing fails.

use std::{os::unix::net::UnixDatagram, sync::OnceLock};

const SOCKET: &str = "/run/systemd/journal/socket";
const IDENTIFIER: &str = "orbit-remote-server";

fn socket() -> Option<&'static UnixDatagram> {
    static SOCKET_FD: OnceLock<Option<UnixDatagram>> = OnceLock::new();
    SOCKET_FD.get_or_init(|| UnixDatagram::unbound().ok()).as_ref()
}

/// The datagram for `message` at `level`: one line each field, the message
/// on one line too (the simple form of the protocol has no newlines in a
/// value).
fn datagram(level: log::Level, message: &str) -> String {
    let priority = match level {
        log::Level::Error => 3,
        log::Level::Warn => 4,
        log::Level::Info => 6,
        log::Level::Debug | log::Level::Trace => 7,
    };
    let message = message.replace('\n', " ");
    format!("MESSAGE={message}\nPRIORITY={priority}\nSYSLOG_IDENTIFIER={IDENTIFIER}\n")
}

/// Sends `message` to the journal, if there is one.
pub fn send(level: log::Level, message: &str) {
    if let Some(socket) = socket() {
        let _ = socket.send_to(datagram(level, message).as_bytes(), SOCKET);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_one_datagram_of_fields() {
        assert_eq!(
            datagram(log::Level::Warn, "remote: 10.0.0.5:51234 gave a wrong code (1 of 5)"),
            "MESSAGE=remote: 10.0.0.5:51234 gave a wrong code (1 of 5)\nPRIORITY=4\nSYSLOG_IDENTIFIER=orbit-remote-server\n"
        );
        assert!(!datagram(log::Level::Error, "two\nlines").contains("two\nlines"));
    }
}
