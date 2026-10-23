//! The link to orbit-remote-server, on its own thread: the WebSocket on the
//! server's port (7311), the same one the browser's page uses.
//!
//! Pairing: the first time the station asks for the six-digit code shown
//! on the server's screen and sends it with `remember`; the server answers
//! with a token, kept with the server's address in
//! `~/.local/state/orbit/station-token`, which the station gives instead
//! of the code from then on. A token the server no longer knows
//! (`forgotten`) is dropped, and the code asked again; another server
//! asks for its own.
//!
//! The server lost or gone: tried again every few seconds, for ever. A new
//! address typed on the station (F2) starts over with it.

use std::{
    io::ErrorKind,
    net::{TcpStream, ToSocketAddrs},
    path::PathBuf,
    sync::mpsc::{Receiver, RecvTimeoutError},
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

/// What the station tells the link.
pub enum Command {
    /// The code typed on the station.
    Code(String),
    /// Another server, as `host:port`.
    Server(String),
}

/// Between tries to reach the server.
const RETRY: Duration = Duration::from_secs(3);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

fn token_path() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state"));
    state.join("orbit/station-token")
}

/// The token kept for `server`: the file holds `SERVER TOKEN`.
fn load_token(server: &str) -> Option<String> {
    let text = std::fs::read_to_string(token_path()).ok()?;
    let (kept, token) = text.trim().split_once(' ')?;
    (kept == server && !token.is_empty()).then(|| token.to_string())
}

fn save_token(server: &str, token: &str) -> anyhow::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = token_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&path)?;
    std::io::Write::write_all(&mut file, format!("{server} {token}\n").as_bytes())?;
    Ok(())
}

/// Starts the link to `server` (`host:port`); what the station types comes
/// in on `commands`.
pub fn start(server: String, events: channel::Sender<Event>, commands: Receiver<Command>) {
    std::thread::spawn(move || {
        let mut link = Link { server, events, commands };
        link.run();
    });
}

struct Link {
    server: String,
    events: channel::Sender<Event>,
    commands: Receiver<Command>,
}

/// Why a wait ended early.
enum Woken {
    /// The station is gone.
    Quit,
    /// Another server was typed: start over.
    Server,
}

impl Link {
    fn tell(&self, event: Event) {
        let _ = self.events.send(event);
    }

    /// Waits `time`, or less if the server changes.
    fn sleep(&mut self, time: Duration) -> Result<(), Woken> {
        match self.commands.recv_timeout(time) {
            Ok(command) => self.command(command),
            Err(RecvTimeoutError::Timeout) => Ok(()),
            Err(RecvTimeoutError::Disconnected) => Err(Woken::Quit),
        }
    }

    /// A command when no code is wanted: a stale code is dropped.
    fn command(&mut self, command: Command) -> Result<(), Woken> {
        match command {
            Command::Code(_) => Ok(()),
            Command::Server(server) => {
                log::info!("station: the server is now {server}");
                self.server = server;
                Err(Woken::Server)
            }
        }
    }

    fn run(&mut self) {
        loop {
            match self.attempt() {
                Ok(()) | Err(Woken::Server) => {}
                Err(Woken::Quit) => return,
            }
        }
    }

    /// One try: pair, then the session until it ends.
    fn attempt(&mut self) -> Result<(), Woken> {
        let mut wrong = false;
        loop {
            // Who the station is: its token, or a code typed now.
            let pair = match load_token(&self.server) {
                Some(token) => json!({ "type": "pair", "token": token }),
                None => {
                    self.tell(Event::Code { wrong });
                    // Codes typed while the server was down are stale.
                    while let Ok(command) = self.commands.try_recv() {
                        self.command(command)?;
                    }
                    let code = loop {
                        match self.commands.recv() {
                            Ok(Command::Code(code)) => break code,
                            Ok(command) => self.command(command)?,
                            Err(_) => return Err(Woken::Quit),
                        }
                    };
                    json!({ "type": "pair", "code": code, "remember": true })
                }
            };
            self.tell(Event::Connecting);
            let reply = connect(&self.server).and_then(|mut ws| {
                ws.send(Message::text(pair.to_string())).map_err(|e| anyhow!("{e}"))?;
                let reply = read_json(&mut ws)?;
                Ok((ws, reply))
            });
            let (mut ws, reply) = match reply {
                Ok(it) => it,
                Err(err) => {
                    self.tell(Event::Unreachable(format!("{err:#}")));
                    self.sleep(RETRY)?;
                    continue;
                }
            };
            wrong = false;
            match reply["type"].as_str() {
                Some("paired") => {
                    if let Some(token) = reply["token"].as_str() {
                        match save_token(&self.server, token) {
                            Ok(()) => log::info!("station: paired, the token kept in {}", token_path().display()),
                            Err(err) => log::warn!("station: keeping the token: {err:#}"),
                        }
                    }
                    self.tell(Event::Paired);
                    let ended = self.session(&mut ws);
                    let _ = ws.close(None);
                    let why = ended?;
                    log::info!("station: {why}");
                    self.tell(Event::Lost(why));
                    return self.sleep(RETRY);
                }
                Some("refused") => wrong = true,
                Some("forgotten") => {
                    log::warn!("station: {} no longer knows this station; asking for the code", self.server);
                    let _ = std::fs::remove_file(token_path());
                }
                Some("busy") => {
                    self.tell(Event::Busy);
                    self.sleep(RETRY)?;
                }
                other => {
                    self.tell(Event::Unreachable(format!("an unexpected reply: {other:?}")));
                    self.sleep(RETRY)?;
                }
            }
        }
    }

    /// The paired session, until it ends: why it did. For now only its
    /// signalling is read (the video comes with WebRTC next).
    fn session(&mut self, ws: &mut Socket) -> Result<String, Woken> {
        if let MaybeTlsStream::Plain(stream) = ws.get_mut() {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
        }
        loop {
            while let Ok(command) = self.commands.try_recv() {
                self.command(command)?;
            }
            match ws.read() {
                Ok(Message::Text(text)) => {
                    let message = serde_json::from_str::<Value>(&text).unwrap_or_default();
                    log::debug!("station: {}", message["type"]);
                }
                Ok(Message::Close(_)) => return Ok("the server closed the session".into()),
                Ok(_) => {}
                Err(tungstenite::Error::Io(err)) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(err) => return Ok(err.to_string()),
            }
        }
    }
}

fn connect(server: &str) -> anyhow::Result<Socket> {
    let address = server
        .to_socket_addrs()
        .map_err(|e| anyhow!("{server}: {e}"))?
        .next()
        .ok_or_else(|| anyhow!("{server}: no address"))?;
    let stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).map_err(|e| anyhow!("{server}: {e}"))?;
    stream.set_nodelay(true)?;
    let (ws, _) = tungstenite::client(format!("ws://{server}/ws"), MaybeTlsStream::Plain(stream)).map_err(|e| anyhow!("{server}: {e}"))?;
    Ok(ws)
}

fn read_json(ws: &mut Socket) -> anyhow::Result<Value> {
    loop {
        match ws.read() {
            Ok(Message::Text(text)) => return Ok(serde_json::from_str(&text)?),
            Ok(Message::Close(_)) => anyhow::bail!("the server closed the connection"),
            Ok(_) => {}
            Err(err) => anyhow::bail!("{err}"),
        }
    }
}
