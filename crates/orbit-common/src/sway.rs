//! Minimal sway IPC client: commands and queries.
//!
//! The protocol is the i3 one: a message is the magic `i3-ipc`, the payload
//! length and the message type (both u32, native endian), then the JSON
//! payload. Replies and events share that framing; events have the high bit
//! of the type set. Shared by the panel (which adds event subscriptions) and
//! the applications that need to know about outputs and windows.

use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
};

use serde::Deserialize;

const MAGIC: &[u8; 6] = b"i3-ipc";

pub const RUN_COMMAND: u32 = 0;
pub const GET_WORKSPACES: u32 = 1;
pub const SUBSCRIBE: u32 = 2;
pub const GET_OUTPUTS: u32 = 3;
pub const GET_TREE: u32 = 4;
pub const GET_INPUTS: u32 = 100;

/// Whether we run under sway.
pub fn available() -> bool {
    std::env::var_os("SWAYSOCK").is_some()
}

pub struct Connection(UnixStream);

impl Connection {
    pub fn open() -> io::Result<Self> {
        let path = std::env::var_os("SWAYSOCK")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "SWAYSOCK is not set"))?;
        Ok(Self(UnixStream::connect(path)?))
    }

    /// Sends a message and returns the reply's payload.
    pub fn request(&mut self, kind: u32, payload: &str) -> io::Result<Vec<u8>> {
        self.send(kind, payload)?;
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

    fn send(&mut self, kind: u32, payload: &str) -> io::Result<()> {
        let mut message = Vec::with_capacity(14 + payload.len());
        message.extend_from_slice(MAGIC);
        message.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
        message.extend_from_slice(&kind.to_ne_bytes());
        message.extend_from_slice(payload.as_bytes());
        self.0.write_all(&message)
    }

    /// The next message: (type, payload) — a reply, or an event after
    /// subscribing.
    pub fn receive(&mut self) -> io::Result<(u32, Vec<u8>)> {
        let mut header = [0u8; 14];
        self.0.read_exact(&mut header)?;
        if &header[..6] != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not an i3-ipc message",
            ));
        }
        let length = u32::from_ne_bytes(header[6..10].try_into().expect("4 bytes"));
        let kind = u32::from_ne_bytes(header[10..14].try_into().expect("4 bytes"));
        let mut body = vec![0; length as usize];
        self.0.read_exact(&mut body)?;
        Ok((kind, body))
    }
}
