//! Minimal sway IPC client: commands, queries and event subscriptions.
//!
//! The protocol is the i3 one: a message is the magic `i3-ipc`, the payload
//! length and the message type (both u32, native endian), then the JSON
//! payload. Replies and events share that framing; events have the high bit
//! of the type set.
//!
//! Queries and commands run on the GTK thread — sway answers them in well
//! under a millisecond. Subscriptions block, so they run on their own thread
//! and only wake the GTK thread through a channel.

use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    time::Duration,
};

use serde::Deserialize;

const MAGIC: &[u8; 6] = b"i3-ipc";

pub const RUN_COMMAND: u32 = 0;
pub const GET_WORKSPACES: u32 = 1;
const SUBSCRIBE: u32 = 2;
pub const GET_OUTPUTS: u32 = 3;
pub const GET_TREE: u32 = 4;

/// Whether the panel runs under sway.
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

    fn receive(&mut self) -> io::Result<(u32, Vec<u8>)> {
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

/// Calls `notify` on the GTK thread after sway reports any of `events`
/// (e.g. `["workspace", "output"]`). A burst of events results in one call.
/// Reconnects if sway restarts.
pub fn subscribe(events: &'static [&'static str], notify: impl Fn() + 'static) {
    let (tx, rx) = async_channel::unbounded::<()>();
    let payload = serde_json::to_string(events).expect("a list of strings serializes");

    std::thread::spawn(move || {
        loop {
            let run = || -> io::Result<()> {
                let mut conn = Connection::open()?;
                conn.request(SUBSCRIBE, &payload)?;
                loop {
                    conn.receive()?;
                    if tx.send_blocking(()).is_err() {
                        return Ok(()); // the GTK side is gone
                    }
                }
            };
            match run() {
                Ok(()) => return,
                Err(err) => {
                    log::warn!("sway: subscription lost ({err}), retrying");
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        }
    });

    gtk::glib::spawn_future_local(async move {
        while rx.recv().await.is_ok() {
            while rx.try_recv().is_ok() {} // fold a burst into one update
            notify();
        }
    });
}

#[derive(Debug, Deserialize)]
pub struct Workspace {
    pub num: i32,
    pub focused: bool,
}

#[derive(Debug, Deserialize)]
pub struct Output {
    pub name: String,
    pub active: bool,
    pub focused: bool,
    pub rect: Rect,
}

/// One entry of a RUN_COMMAND reply.
#[derive(Debug, Deserialize)]
pub struct CommandResult {
    pub success: bool,
}

#[derive(Debug, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
}
