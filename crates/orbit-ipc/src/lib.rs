//! The i3/sway IPC protocol, as Orbit speaks it: the core serves it (so
//! everything written for sway keeps working, PLANO-CORE §5) and the
//! panel, the apps and `orbitctl` use the client here.
//!
//! A message is the magic `i3-ipc`, the payload's length and the message
//! type (both u32, native endian), then the JSON payload. Replies and events
//! share that framing; events have the high bit of the type set.
//!
//! The socket is `$ORBIT_SOCK` under Orbit-Core, `$SWAYSOCK` under sway (the
//! core exports all three names).

use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
};

use serde::Deserialize;

pub const MAGIC: &[u8; 6] = b"i3-ipc";
/// Magic, length, type.
pub const HEADER: usize = 14;

pub const RUN_COMMAND: u32 = 0;
pub const GET_WORKSPACES: u32 = 1;
pub const SUBSCRIBE: u32 = 2;
pub const GET_OUTPUTS: u32 = 3;
pub const GET_TREE: u32 = 4;
pub const GET_MARKS: u32 = 5;
pub const GET_BAR_CONFIG: u32 = 6;
pub const GET_VERSION: u32 = 7;
pub const GET_BINDING_MODES: u32 = 8;
pub const GET_CONFIG: u32 = 9;
pub const SEND_TICK: u32 = 10;
pub const SYNC: u32 = 11;
pub const GET_BINDING_STATE: u32 = 12;
pub const GET_INPUTS: u32 = 100;
pub const GET_SEATS: u32 = 101;

/// The high bit marks an event.
pub const EVENT: u32 = 0x8000_0000;

/// The events a client may subscribe to, by name, and their type.
pub const EVENTS: &[(&str, u32)] = &[
    ("workspace", EVENT),
    ("output", EVENT | 1),
    ("mode", EVENT | 2),
    ("window", EVENT | 3),
    ("barconfig_update", EVENT | 4),
    ("binding", EVENT | 5),
    ("shutdown", EVENT | 6),
    ("tick", EVENT | 7),
    ("bar_state_update", EVENT | 0x14),
    ("input", EVENT | 0x15),
    // Orbit's own: the session locked or unlocked ({"change": "locked"}).
    ("lock", EVENT | 0x16),
];

/// The type of the event called `name`.
pub fn event_type(name: &str) -> Option<u32> {
    EVENTS.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

/// The message types by the names `swaymsg -t` takes.
pub fn message_type(name: &str) -> Option<u32> {
    Some(match name {
        "command" => RUN_COMMAND,
        "get_workspaces" => GET_WORKSPACES,
        "subscribe" => SUBSCRIBE,
        "get_outputs" => GET_OUTPUTS,
        "get_tree" => GET_TREE,
        "get_marks" => GET_MARKS,
        "get_bar_config" => GET_BAR_CONFIG,
        "get_version" => GET_VERSION,
        "get_binding_modes" => GET_BINDING_MODES,
        "get_config" => GET_CONFIG,
        "send_tick" => SEND_TICK,
        "get_binding_state" => GET_BINDING_STATE,
        "get_inputs" => GET_INPUTS,
        "get_seats" => GET_SEATS,
        _ => return None,
    })
}

/// One framed message.
pub fn encode(kind: u32, payload: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(HEADER + payload.len());
    message.extend_from_slice(MAGIC);
    message.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
    message.extend_from_slice(&kind.to_ne_bytes());
    message.extend_from_slice(payload);
    message
}

/// Takes complete messages off the front of `buffer`: Ok(None) while one is
/// still arriving, an error when the bytes are not this protocol.
pub fn decode(buffer: &mut Vec<u8>) -> io::Result<Option<(u32, Vec<u8>)>> {
    if buffer.len() < HEADER {
        return Ok(None);
    }
    if &buffer[..6] != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not an i3-ipc message"));
    }
    let length = u32::from_ne_bytes(buffer[6..10].try_into().expect("4 bytes")) as usize;
    let kind = u32::from_ne_bytes(buffer[10..14].try_into().expect("4 bytes"));
    if length > 16 * 1024 * 1024 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "message too large"));
    }
    if buffer.len() < HEADER + length {
        return Ok(None);
    }
    let payload = buffer[HEADER..HEADER + length].to_vec();
    buffer.drain(..HEADER + length);
    Ok(Some((kind, payload)))
}

/// The socket to talk to: Orbit-Core's, or sway's.
pub fn socket_path() -> Option<PathBuf> {
    ["ORBIT_SOCK", "SWAYSOCK", "I3SOCK"].iter().find_map(std::env::var_os).map(PathBuf::from)
}

/// Whether a compositor that speaks this protocol is there.
pub fn available() -> bool {
    socket_path().is_some()
}

/// A blocking client connection.
pub struct Connection(UnixStream);

impl Connection {
    pub fn open() -> io::Result<Self> {
        let path = socket_path().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "neither ORBIT_SOCK nor SWAYSOCK is set"))?;
        Ok(Self(UnixStream::connect(path)?))
    }

    /// Sends a message and returns the reply's payload.
    pub fn request(&mut self, kind: u32, payload: &str) -> io::Result<Vec<u8>> {
        self.0.write_all(&encode(kind, payload.as_bytes()))?;
        loop {
            let (reply, body) = self.receive()?;
            if reply == kind {
                return Ok(body);
            }
        }
    }

    /// Sends a query and parses its JSON reply.
    pub fn query<T: for<'de> Deserialize<'de>>(&mut self, kind: u32) -> io::Result<T> {
        let body = self.request(kind, "")?;
        serde_json::from_slice(&body).map_err(io::Error::other)
    }

    /// The next message: (type, payload) — a reply, or an event after
    /// subscribing.
    pub fn receive(&mut self) -> io::Result<(u32, Vec<u8>)> {
        let mut header = [0u8; HEADER];
        self.0.read_exact(&mut header)?;
        if &header[..6] != MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "not an i3-ipc message"));
        }
        let length = u32::from_ne_bytes(header[6..10].try_into().expect("4 bytes"));
        let kind = u32::from_ne_bytes(header[10..14].try_into().expect("4 bytes"));
        let mut body = vec![0; length as usize];
        self.0.read_exact(&mut body)?;
        Ok((kind, body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let mut buffer = encode(GET_TREE, b"");
        buffer.extend(encode(RUN_COMMAND, b"workspace 2"));
        buffer.extend(&encode(SUBSCRIBE, b"[\"window\"]")[..5]);
        assert_eq!(decode(&mut buffer).unwrap(), Some((GET_TREE, Vec::new())));
        assert_eq!(decode(&mut buffer).unwrap(), Some((RUN_COMMAND, b"workspace 2".to_vec())));
        // Half a message: wait for the rest.
        assert_eq!(decode(&mut buffer).unwrap(), None);
        let mut junk = b"GET / HTTP/1.1\r\n".to_vec();
        assert!(decode(&mut junk).is_err());
    }

    #[test]
    fn names() {
        assert_eq!(event_type("window"), Some(EVENT | 3));
        assert_eq!(event_type("input"), Some(EVENT | 0x15));
        assert_eq!(message_type("get_tree"), Some(GET_TREE));
        assert_eq!(message_type("nope"), None);
    }
}
