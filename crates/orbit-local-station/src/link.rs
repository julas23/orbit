//! The link to orbit-remote-server, on its own thread: the WebSocket on the
//! server's port (7311), the same one the browser's page uses.
//!
//! Pairing: the first time the station asks for the six-digit code shown
//! on the server's screen and sends it with `remember`; the server answers
//! with a token, kept in `~/.local/state/orbit/station-token`, which the
//! station gives instead of the code from then on. A token the server no
//! longer knows (`forgotten`) is dropped, and the code asked again.
//!
//! The server lost or gone: tried again every few seconds, for ever.

use std::{
    net::TcpStream,
    path::PathBuf,
    sync::mpsc::Receiver,
    time::Duration,
};

use anyhow::anyhow;
use serde_json::{Value, json};
use smithay::reexports::calloop::channel;
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

/// What the link tells the station.
pub enum Event {
    /// Trying to reach the server.
    Connecting,
    /// The server is not answering (the reason).
    Unreachable(String),
    /// The code on the server's screen is wanted; the bool says a code
    /// was given before and was wrong.
    Code { wrong: bool },
    /// Another client is using the server.
    Busy,
    /// Paired: the session comes next.
    Paired,
    /// The session ended (the reason).
    Lost(String),
}

/// Between tries to reach the server.
const RETRY: Duration = Duration::from_secs(3);

fn token_path() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state"));
    state.join("orbit/station-token")
}

fn load_token() -> Option<String> {
    std::fs::read_to_string(token_path()).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

fn save_token(token: &str) -> anyhow::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = token_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&path)?;
    std::io::Write::write_all(&mut file, format!("{token}\n").as_bytes())?;
    Ok(())
}

/// Starts the link to `server` (`host:port`). Codes typed on the station
/// come in on `codes`.
pub fn start(server: String, events: channel::Sender<Event>, codes: Receiver<String>) {
    std::thread::spawn(move || run(&server, &events, &codes));
}

fn run(server: &str, events: &channel::Sender<Event>, codes: &Receiver<String>) {
    let mut wrong = false;
    loop {
        // Who the station is: its token, or a code typed now.
        let pair = match load_token() {
            Some(token) => json!({ "type": "pair", "token": token }),
            None => {
                let _ = events.send(Event::Code { wrong });
                // Codes typed while the server was down are stale.
                while codes.try_recv().is_ok() {}
                let Ok(code) = codes.recv() else { return };
                json!({ "type": "pair", "code": code, "remember": true })
            }
        };
        let _ = events.send(Event::Connecting);
        let mut ws = match connect(server) {
            Ok(ws) => ws,
            Err(err) => {
                let _ = events.send(Event::Unreachable(format!("{err:#}")));
                std::thread::sleep(RETRY);
                continue;
            }
        };
        let reply = ws.send(Message::text(pair.to_string())).map_err(|e| anyhow!("{e}")).and_then(|()| read_json(&mut ws));
        let reply = match reply {
            Ok(reply) => reply,
            Err(err) => {
                let _ = events.send(Event::Unreachable(format!("{err:#}")));
                std::thread::sleep(RETRY);
                continue;
            }
        };
        wrong = false;
        match reply["type"].as_str() {
            Some("paired") => {
                if let Some(token) = reply["token"].as_str() {
                    match save_token(token) {
                        Ok(()) => log::info!("station: paired, the token kept in {}", token_path().display()),
                        Err(err) => log::warn!("station: keeping the token: {err:#}"),
                    }
                }
                let _ = events.send(Event::Paired);
                let why = session(&mut ws).err().map_or_else(|| "the server closed the session".to_string(), |e| format!("{e:#}"));
                log::info!("station: {why}");
                let _ = events.send(Event::Lost(why));
                std::thread::sleep(RETRY);
            }
            Some("refused") => wrong = true,
            Some("forgotten") => {
                log::warn!("station: the server no longer knows this station; asking for the code");
                let _ = std::fs::remove_file(token_path());
            }
            Some("busy") => {
                let _ = events.send(Event::Busy);
                std::thread::sleep(RETRY);
            }
            other => {
                let _ = events.send(Event::Unreachable(format!("an unexpected reply: {other:?}")));
                std::thread::sleep(RETRY);
            }
        }
    }
}

fn connect(server: &str) -> anyhow::Result<WebSocket<MaybeTlsStream<TcpStream>>> {
    let stream = TcpStream::connect(server).map_err(|e| anyhow!("{server}: {e}"))?;
    stream.set_nodelay(true)?;
    let (ws, _) = tungstenite::client(format!("ws://{server}/ws"), MaybeTlsStream::Plain(stream)).map_err(|e| anyhow!("{server}: {e}"))?;
    Ok(ws)
}

fn read_json(ws: &mut WebSocket<MaybeTlsStream<TcpStream>>) -> anyhow::Result<Value> {
    loop {
        match ws.read() {
            Ok(Message::Text(text)) => return Ok(serde_json::from_str(&text)?),
            Ok(Message::Close(_)) => anyhow::bail!("the server closed the connection"),
            Ok(_) => {}
            Err(err) => anyhow::bail!("{err}"),
        }
    }
}

/// The paired session, until it ends. For now only its signalling is
/// read (the video comes with WebRTC next).
fn session(ws: &mut WebSocket<MaybeTlsStream<TcpStream>>) -> anyhow::Result<()> {
    loop {
        let message = read_json(ws)?;
        log::debug!("station: {}", message["type"]);
    }
}
