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
//! Paired, the server offers its video over WebRTC (`video`); the answer,
//! the ICE candidates and the station's mouse and keys go on the same
//! WebSocket. The session waits on the socket and on a wake-up (`Wake`)
//! rung whenever there is something to send, so nothing waits its turn.
//!
//! The server lost or gone: tried again every few seconds, for ever. A new
//! address typed on the station (F2) starts over with it. The session left
//! on the server (`ended`, its compositor gone) is the station's end too.

use std::{
    io::{ErrorKind, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    time::Duration,
};

use anyhow::anyhow;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use serde_json::{Value, json};
use smithay::reexports::calloop::channel;
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

use crate::video::{Frame, Video};

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
    /// The server's session is locked: no pairing until it is unlocked.
    Locked,
    /// Paired: the video comes next.
    Paired,
    /// A picture of the session.
    Frame(Frame),
    /// The session ended (the reason).
    Lost(String),
    /// The user left the session on the server: the station is done.
    Ended,
}

/// What the station tells the link.
pub enum Command {
    /// The code typed on the station.
    Code(String),
    /// Another server, as `host:port`.
    Server(String),
    /// The mouse or a key, for the server (`move`, `button`, `wheel`, `key`).
    Input(Value),
}

/// Rung to wake the session when there is something to send.
#[derive(Clone)]
pub struct Wake(Arc<UnixStream>);

impl Wake {
    pub fn ring(&self) {
        // Full means it is rung already.
        let _ = (&*self.0).write(&[1]);
    }
}

/// What the station gives the link.
pub struct Commands {
    sender: mpsc::Sender<Command>,
    wake: Wake,
}

impl Commands {
    pub fn send(&self, command: Command) {
        if self.sender.send(command).is_ok() {
            self.wake.ring();
        }
    }
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

/// Starts the link to `server` (`host:port`), for a monitor of `size`;
/// what the station types goes to it through what is returned.
pub fn start(server: String, size: (u32, u32), events: channel::Sender<Event>) -> anyhow::Result<Commands> {
    let (ring, woken) = UnixStream::pair()?;
    ring.set_nonblocking(true)?;
    woken.set_nonblocking(true)?;
    let wake = Wake(Arc::new(ring));
    let (sender, commands) = mpsc::channel();
    let link_wake = wake.clone();
    std::thread::spawn(move || {
        let mut link = Link { server, size, events, commands, wake: link_wake, woken };
        link.run();
    });
    Ok(Commands { sender, wake })
}

struct Link {
    server: String,
    size: (u32, u32),
    events: channel::Sender<Event>,
    commands: Receiver<Command>,
    wake: Wake,
    woken: UnixStream,
}

/// Why a wait ended early.
enum Woken {
    /// The station is gone.
    Quit,
    /// Another server was typed: start over.
    Server,
    /// The server said the session is over.
    Ended,
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

    /// A command when no code is wanted: a stale code is dropped, and so is
    /// input with no session to take it.
    fn command(&mut self, command: Command) -> Result<(), Woken> {
        match command {
            Command::Code(_) | Command::Input(_) => Ok(()),
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
                Err(Woken::Ended) => {
                    log::info!("station: the session was left on the server");
                    return self.tell(Event::Ended);
                }
            }
        }
    }

    /// One try: pair, then the session until it ends.
    fn attempt(&mut self) -> Result<(), Woken> {
        let mut wrong = false;
        loop {
            // Who the station is: its token, or a code typed now.
            // Its screen too: a server serving virtual monitors makes one
            // of this size.
            let screen = json!({ "width": self.size.0, "height": self.size.1, "rate": 60 });
            let pair = match load_token(&self.server) {
                Some(token) => json!({ "type": "pair", "token": token, "screen": screen }),
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
                    json!({ "type": "pair", "code": code, "remember": true, "screen": screen })
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
                Some("locked") => {
                    self.tell(Event::Locked);
                    self.sleep(RETRY)?;
                }
                // A server from before the tokens streams straight away.
                Some("offer" | "ice") => {
                    self.tell(Event::Unreachable(format!("{} is too old for the station: update orbit-remote-server", self.server)));
                    self.sleep(RETRY)?;
                }
                other => {
                    self.tell(Event::Unreachable(format!("an unexpected reply: {other:?}")));
                    self.sleep(RETRY)?;
                }
            }
        }
    }

    /// The paired session, until it ends: why it did.
    fn session(&mut self, ws: &mut Socket) -> Result<String, Woken> {
        let (out, outbox) = mpsc::channel::<Value>();
        let video = match Video::new(self.size, self.events.clone(), out.clone(), self.wake.clone()) {
            Ok(video) => video,
            Err(err) => return Ok(format!("{err:#}")),
        };
        if let MaybeTlsStream::Plain(stream) = ws.get_mut()
            && let Err(err) = stream.set_nonblocking(true)
        {
            return Ok(err.to_string());
        }
        let fd = match ws.get_ref() {
            MaybeTlsStream::Plain(stream) => stream.try_clone().map_err(|_| Woken::Quit)?,
            _ => return Ok("not a plain connection".into()),
        };
        loop {
            // Whatever rang, all is looked at below.
            while (&self.woken).read(&mut [0u8; 64]).is_ok_and(|n| n > 0) {}
            while let Ok(command) = self.commands.try_recv() {
                match command {
                    Command::Input(message) => {
                        if let Err(why) = send(ws, &message) {
                            return Ok(why);
                        }
                    }
                    command => self.command(command)?,
                }
            }
            for message in outbox.try_iter() {
                if let Err(why) = send(ws, &message) {
                    return Ok(why);
                }
            }
            if let Some(why) = video.failed() {
                return Ok(why);
            }
            loop {
                match ws.read() {
                    Ok(Message::Text(text)) => {
                        let message = serde_json::from_str::<Value>(&text).unwrap_or_default();
                        match message["type"].as_str() {
                            Some("offer") => {
                                let sdp = message["sdp"].as_str().unwrap_or_default();
                                if let Err(err) = video.offer(sdp, out.clone(), self.wake.clone()) {
                                    return Ok(format!("{err:#}"));
                                }
                            }
                            Some("ice") => video.ice(&message),
                            Some("ended") => return Err(Woken::Ended),
                            other => log::debug!("station: {other:?}"),
                        }
                    }
                    Ok(Message::Close(_)) => return Ok("the server closed the session".into()),
                    Ok(_) => {}
                    Err(tungstenite::Error::Io(err)) if err.kind() == ErrorKind::WouldBlock => break,
                    Err(err) => return Ok(err.to_string()),
                }
            }
            if let Err(why) = flush(ws) {
                return Ok(why);
            }
            let mut fds = [PollFd::new(&fd, PollFlags::IN), PollFd::new(&self.woken, PollFlags::IN)];
            let _ = poll(&mut fds, Some(&Timespec { tv_sec: 0, tv_nsec: 100_000_000 }));
        }
    }
}

/// A message to the server; a full socket keeps it for the next flush.
fn send(ws: &mut Socket, message: &Value) -> Result<(), String> {
    match ws.send(Message::text(message.to_string())) {
        Ok(()) => Ok(()),
        Err(tungstenite::Error::Io(err)) if err.kind() == ErrorKind::WouldBlock => Ok(()),
        Err(err) => Err(err.to_string()),
    }
}

fn flush(ws: &mut Socket) -> Result<(), String> {
    match ws.flush() {
        Ok(()) => Ok(()),
        Err(tungstenite::Error::Io(err)) if err.kind() == ErrorKind::WouldBlock => Ok(()),
        Err(err) => Err(err.to_string()),
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
