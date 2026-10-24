//! Serving the session: one port on the LAN (7311) for the browser's page,
//! `GET /`, and its WebSocket, `GET /ws`, which carries only the pairing
//! and WebRTC's signalling (the offer, the answer, the ICE candidates);
//! the video goes over WebRTC itself (UDP).
//!
//! Pairing: a six-digit code shows on the server's screen (a notification)
//! and the browser must send it first; five wrong codes make a new one.
//! A client that asks to be remembered (`orbit-local-station`) gets a token
//! back and gives it instead of the code from then on; the server keeps
//! only the tokens' SHA-256, in `~/.local/state/orbit/remote-tokens`.
//! One client at a time.
//!
//! What the client sees: a mirror of a local monitor, or (`serve
//! --virtual`) a monitor of its own, made in orbit-core at the size the
//! client gives when it pairs (`screen`) and taken away when it leaves.
//!
//! While the session is locked (orbit-core's `lock` event) no new client
//! pairs: a code is answered `locked`, and the code is shown only once
//! the session is unlocked. A remembered client still comes in, to the
//! lock screen, where the password is asked as at the computer.
//!
//! A session: `webrtcbin` at the encoder's tail (`encode::Tail::WebRtc`),
//! offering the video; a thread captures and pushes frames until the
//! browser goes away. The browser's mouse and keys come back on the
//! WebSocket (`input`). The capture stopping (the user left the session,
//! its compositor gone) is told as `ended`, for the client not to come back.

use std::{
    io::ErrorKind,
    os::unix::fs::OpenOptionsExt,
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

use anyhow::{Context, anyhow};
use gst::prelude::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tungstenite::{Message, WebSocket};

use crate::{
    capture::Capture,
    encode::{Encoder, Tail},
    input::Input,
};

const PAGE: &str = include_str!("../web/index.html");
/// Wrong codes before a new one is made.
const TRIES: u32 = 5;

/// What a client is shown.
pub enum Screen {
    /// A local monitor (the first when not named).
    Mirror(Option<String>),
    /// A virtual monitor of the client's size (the mirror for a client
    /// that gives none).
    Virtual,
}

/// The virtual monitor's name in orbit-core.
const VIRTUAL: &str = "REMOTE-1";

struct Shared {
    screen: Screen,
    code: Mutex<String>,
    wrong: Mutex<u32>,
    busy: AtomicBool,
    /// The session is locked: no pairing by code.
    locked: AtomicBool,
    /// The code still to be shown, held back by the lock.
    unshown: AtomicBool,
}

pub fn serve(port: u16, screen: Screen) -> anyhow::Result<()> {
    gst::init()?;
    let listener = TcpListener::bind(("0.0.0.0", port)).with_context(|| format!("listening on port {port}"))?;
    let shared = Arc::new(Shared {
        screen,
        code: Mutex::new(new_code()),
        wrong: Mutex::new(0),
        busy: AtomicBool::new(false),
        // Behind the lock until orbit-core says otherwise (at once, on
        // subscribing); without it (sway), never.
        locked: AtomicBool::new(orbit_ipc::available()),
        unshown: AtomicBool::new(true),
    });
    {
        let shared = shared.clone();
        std::thread::spawn(move || follow_lock(&shared));
    }
    if !orbit_ipc::available() {
        shared.reveal();
    }
    log::info!("remote: open http://<this computer>:{port}/ in a browser on the LAN");
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let shared = shared.clone();
        std::thread::spawn(move || {
            if let Err(err) = connection(stream, &shared) {
                log::warn!("remote: {err:#}");
            }
        });
    }
    Ok(())
}

impl Shared {
    /// Shows the code, if it has not been since it was made.
    fn reveal(&self) {
        if self.unshown.swap(false, Ordering::SeqCst) {
            show_code(&self.code.lock().unwrap());
        }
    }

    /// A new code, shown now or once the session is unlocked.
    fn renew(&self) {
        *self.code.lock().unwrap() = new_code();
        self.unshown.store(true, Ordering::SeqCst);
        if !self.locked.load(Ordering::SeqCst) {
            self.reveal();
        }
    }
}

/// Follows orbit-core's `lock` event. Losing the core's socket (the
/// session gone) leaves the server locked: nothing pairs anew.
fn follow_lock(shared: &Shared) {
    let Ok(mut ipc) = orbit_ipc::Connection::open() else { return };
    let Some(lock) = orbit_ipc::event_type("lock") else { return };
    if ipc.request(orbit_ipc::SUBSCRIBE, r#"["lock"]"#).is_err() {
        return;
    }
    while let Ok((kind, body)) = ipc.receive() {
        if kind != lock {
            continue;
        }
        let change: Value = serde_json::from_slice(&body).unwrap_or_default();
        let locked = change["change"] == "locked";
        if shared.locked.swap(locked, Ordering::SeqCst) != locked {
            log::info!("remote: the session is {}", if locked { "locked: no pairing" } else { "unlocked" });
        }
        if !locked {
            shared.reveal();
        }
    }
    shared.locked.store(true, Ordering::SeqCst);
}

/// Where the remembered clients' tokens are kept, one SHA-256 a line.
fn tokens_path() -> std::path::PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state"));
    state.join("orbit/remote-tokens")
}

fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// A token given before (by `remember`).
fn known(token: &str) -> bool {
    let wanted = hash(token);
    std::fs::read_to_string(tokens_path()).is_ok_and(|text| text.lines().any(|line| line.trim() == wanted))
}

/// A new token for a client paired by its code, kept for next time.
fn remember() -> anyhow::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| anyhow!("random bytes: {e}"))?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let path = tokens_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    std::io::Write::write_all(&mut file, format!("{}\n", hash(&token)).as_bytes())?;
    Ok(token)
}

/// Six digits from the system's random source.
fn new_code() -> String {
    let mut bytes = [0u8; 4];
    getrandom::getrandom(&mut bytes).expect("random bytes");
    format!("{:06}", u32::from_le_bytes(bytes) % 1_000_000)
}

/// The code on the server's screen, and in the log.
fn show_code(code: &str) {
    log::info!("remote: the pairing code is {code}");
    let shown = std::process::Command::new("notify-send")
        .args(["--app-name=Orbit Remote", "--icon=network-server", "Orbit Remote", &format!("Pairing code: {code}")])
        .status();
    if let Err(err) = shown {
        log::warn!("remote: notify-send: {err}");
    }
}

/// One TCP connection: the page, or the WebSocket.
fn connection(stream: TcpStream, shared: &Shared) -> anyhow::Result<()> {
    let mut head = [0u8; 512];
    let n = stream.peek(&mut head)?;
    let request = String::from_utf8_lossy(&head[..n]);
    let path = request.split_whitespace().nth(1).unwrap_or("/");
    if path.starts_with("/ws") {
        let ws = tungstenite::accept(stream).map_err(|e| anyhow!("websocket: {e}"))?;
        return session(ws, shared);
    }
    let (status, kind, body) = match path {
        "/" | "/index.html" => ("200 OK", "text/html; charset=utf-8", PAGE),
        _ => ("404 Not Found", "text/plain", "not found"),
    };
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // Read the request away first, so closing does not reset it.
    let mut stream = stream;
    let mut sink = [0u8; 512];
    let _ = std::io::Read::read(&mut stream, &mut sink);
    std::io::Write::write_all(&mut stream, reply.as_bytes())?;
    Ok(())
}

/// A browser on the WebSocket: paired, then streamed to until it leaves.
fn session(mut ws: WebSocket<TcpStream>, shared: &Shared) -> anyhow::Result<()> {
    let peer = ws.get_ref().peer_addr()?;
    let first = read_json(&mut ws)?;
    if first["type"] != "pair" {
        ws.send(Message::text(json!({ "type": "refused" }).to_string()))?;
        return Ok(());
    }
    // A remembered client: its token, not the code.
    if let Some(token) = first["token"].as_str() {
        if !known(token) {
            log::warn!("remote: {peer} gave an unknown token");
            ws.send(Message::text(json!({ "type": "forgotten" }).to_string()))?;
            return Ok(());
        }
        return paired(ws, shared, None, size(&first));
    }
    if shared.locked.load(Ordering::SeqCst) {
        log::warn!("remote: {peer} wants to pair, but the session is locked");
        ws.send(Message::text(json!({ "type": "locked" }).to_string()))?;
        return Ok(());
    }
    let given = first["code"].as_str().unwrap_or_default();
    if given != *shared.code.lock().unwrap() {
        let mut wrong = shared.wrong.lock().unwrap();
        *wrong += 1;
        log::warn!("remote: {peer} gave a wrong code ({} of {TRIES})", *wrong);
        if *wrong >= TRIES {
            *wrong = 0;
            shared.renew();
        }
        ws.send(Message::text(json!({ "type": "refused" }).to_string()))?;
        return Ok(());
    }
    *shared.wrong.lock().unwrap() = 0;
    let token = if first["remember"] == true { Some(remember()?) } else { None };
    paired(ws, shared, token, size(&first))
}

/// The client's screen, `{"screen": {"width", "height", "rate"}}`, if it
/// gave one.
fn size(first: &Value) -> Option<(u32, u32, u32)> {
    let screen = &first["screen"];
    let side = |name: &str| screen[name].as_u64().and_then(|v| u32::try_from(v).ok()).filter(|v| (64..=7680).contains(v));
    Some((side("width")?, side("height")?, screen["rate"].as_f64().map_or(60, |r| r.round().clamp(24.0, 240.0) as u32)))
}

/// A virtual monitor of `size` in orbit-core, removed when dropped.
struct Monitor;

impl Monitor {
    fn create((width, height, rate): (u32, u32, u32)) -> anyhow::Result<Self> {
        let reply = core_command(&format!("output {VIRTUAL} create {width}x{height}@{rate}"))?;
        if reply[0]["success"] != true {
            anyhow::bail!("orbit-core made no virtual monitor: {}", reply[0]["error"].as_str().unwrap_or("no answer"));
        }
        log::info!("remote: a virtual monitor of {width}x{height}@{rate} for the client");
        Ok(Self)
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        if let Err(err) = core_command(&format!("output {VIRTUAL} remove")) {
            log::warn!("remote: removing the virtual monitor: {err:#}");
        }
    }
}

/// A command to orbit-core, its reply.
fn core_command(command: &str) -> anyhow::Result<Value> {
    let mut ipc = orbit_ipc::Connection::open().context("orbit-core's socket")?;
    let reply = ipc.request(orbit_ipc::RUN_COMMAND, command)?;
    Ok(serde_json::from_slice(&reply)?)
}

/// A client that gave the right code or a known token: told so (with its
/// new token, if it asked for one), then streamed to.
fn paired(mut ws: WebSocket<TcpStream>, shared: &Shared, token: Option<String>, screen: Option<(u32, u32, u32)>) -> anyhow::Result<()> {
    let peer = ws.get_ref().peer_addr()?;
    if shared.busy.swap(true, Ordering::SeqCst) {
        ws.send(Message::text(json!({ "type": "busy" }).to_string()))?;
        return Ok(());
    }
    let reply = match token {
        Some(token) => json!({ "type": "paired", "token": token }),
        None => json!({ "type": "paired" }),
    };
    if let Err(err) = ws.send(Message::text(reply.to_string())) {
        shared.busy.store(false, Ordering::SeqCst);
        return Err(anyhow!("websocket: {err}"));
    }
    log::info!("remote: {peer} paired");
    let result = stream(&mut ws, shared, screen);
    shared.busy.store(false, Ordering::SeqCst);
    log::info!("remote: {peer} left");
    result
}

fn read_json(ws: &mut WebSocket<TcpStream>) -> anyhow::Result<Value> {
    loop {
        match ws.read() {
            Ok(Message::Text(text)) => return Ok(serde_json::from_str(&text)?),
            Ok(Message::Close(_)) => anyhow::bail!("closed"),
            Ok(_) => {}
            Err(tungstenite::Error::Io(err)) if err.kind() == ErrorKind::WouldBlock || err.kind() == ErrorKind::TimedOut => {}
            Err(err) => return Err(anyhow!("websocket: {err}")),
        }
    }
}

/// Streams to the paired browser on `ws` until it closes.
fn stream(ws: &mut WebSocket<TcpStream>, shared: &Shared, screen: Option<(u32, u32, u32)>) -> anyhow::Result<()> {
    // Its own monitor, when served so and it said its size; kept until it leaves.
    let monitor = match (&shared.screen, screen) {
        (Screen::Virtual, Some(size)) => match Monitor::create(size) {
            Ok(monitor) => Some(monitor),
            Err(err) => {
                log::warn!("remote: {err:#}; a mirror instead");
                None
            }
        },
        _ => None,
    };
    let output = match (&shared.screen, &monitor) {
        (_, Some(_)) => Some(VIRTUAL),
        (Screen::Mirror(name), None) => name.as_deref(),
        (Screen::Virtual, None) => None,
    };
    let mut capture = Capture::open(output, true)?;
    // A compositor without virtual input (an older orbit-core) still
    // shows the session: the client only watches.
    let mut input = match Input::open(&capture.name) {
        Ok(input) => Some(input),
        Err(err) => {
            log::warn!("remote: no remote mouse and keys, the session is view only: {err:#}");
            None
        }
    };
    let webrtc = gst::ElementFactory::make("webrtcbin")
        .property_from_str("bundle-policy", "max-bundle")
        .build()
        .context("no webrtcbin (gstreamer1.0-plugins-bad, gstreamer1.0-nice)")?;
    // What goes to the browser, from GStreamer's threads.
    let (out, outbox) = mpsc::channel::<Value>();
    {
        let out = out.clone();
        webrtc.connect("on-negotiation-needed", false, move |values| {
            let webrtc = values[0].get::<gst::Element>().expect("webrtcbin");
            let out = out.clone();
            let w = webrtc.clone();
            let promise = gst::Promise::with_change_func(move |reply| {
                let Ok(Some(reply)) = reply else { return log::error!("remote: no offer") };
                let Ok(offer) = reply.value("offer").map(|v| v.get::<gst_webrtc::WebRTCSessionDescription>()) else {
                    return log::error!("remote: no offer in {reply:?}");
                };
                let Ok(offer) = offer else { return };
                w.emit_by_name::<()>("set-local-description", &[&offer, &None::<gst::Promise>]);
                let sdp = offer.sdp().as_text().unwrap_or_default();
                let _ = out.send(json!({ "type": "offer", "sdp": sdp }));
            });
            webrtc.emit_by_name::<()>("create-offer", &[&None::<gst::Structure>, &promise]);
            None
        });
    }
    {
        let out = out.clone();
        webrtc.connect("on-ice-candidate", false, move |values| {
            let mline = values[1].get::<u32>().expect("an m-line");
            let candidate = values[2].get::<String>().expect("a candidate");
            let _ = out.send(json!({ "type": "ice", "candidate": candidate, "sdpMLineIndex": mline }));
            None
        });
    }
    // Connected: the picture can reach the client from now on.
    let connected = Arc::new(AtomicBool::new(false));
    {
        let connected = connected.clone();
        webrtc.connect_notify(Some("connection-state"), move |webrtc, _| {
            let state = webrtc.property::<gst_webrtc::WebRTCPeerConnectionState>("connection-state");
            connected.store(state == gst_webrtc::WebRTCPeerConnectionState::Connected, Ordering::SeqCst);
        });
    }
    let encoder = Encoder::new(&capture.planes, Tail::WebRtc(webrtc.clone()))?;
    let encoder = Arc::new(encoder);

    // Frames, from their own thread, until told to stop. The first makes
    // the offer (its caps), but goes out before anything can arrive: once
    // connected it is sent again as a key frame, or a still screen would
    // leave the client with nothing to show.
    let stop = Arc::new(AtomicBool::new(false));
    let pusher = {
        let (stop, encoder) = (stop.clone(), encoder.clone());
        std::thread::spawn(move || -> anyhow::Result<()> {
            let first = capture.next()?;
            encoder.push(&capture.planes, &first)?;
            while !connected.load(Ordering::SeqCst) {
                if stop.load(Ordering::SeqCst) {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            encoder.key_frame();
            encoder.push(&capture.planes, &first)?;
            while !stop.load(Ordering::SeqCst) {
                let frame = capture.next()?;
                encoder.push(&capture.planes, &frame)?;
            }
            Ok(())
        })
    };

    ws.get_mut().set_read_timeout(Some(Duration::from_millis(20)))?;
    let result = loop {
        let sent: Result<(), _> = outbox.try_iter().try_for_each(|message| ws.send(Message::text(message.to_string())));
        if let Err(err) = sent {
            break Err(anyhow!("websocket: {err}"));
        }
        if pusher.is_finished() {
            // The compositor went away under it: the user left the session.
            let _ = ws.send(Message::text(json!({ "type": "ended" }).to_string()));
            break Err(anyhow!("the capture stopped"));
        }
        let message = match ws.read() {
            Ok(Message::Text(text)) => serde_json::from_str::<Value>(&text)?,
            Ok(Message::Close(_)) => break Ok(()),
            Ok(_) => continue,
            Err(tungstenite::Error::Io(err)) if err.kind() == ErrorKind::WouldBlock || err.kind() == ErrorKind::TimedOut => continue,
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => break Ok(()),
            Err(err) => break Err(anyhow!("websocket: {err}")),
        };
        let handled = match &mut input {
            Some(input) => input.handle(&message)?,
            None => matches!(message["type"].as_str(), Some("move" | "button" | "wheel" | "key")),
        };
        if handled {
            continue;
        }
        match message["type"].as_str() {
            Some("answer") => {
                let sdp = message["sdp"].as_str().unwrap_or_default();
                let sdp = gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes()).map_err(|_| anyhow!("a bad answer"))?;
                let answer = gst_webrtc::WebRTCSessionDescription::new(gst_webrtc::WebRTCSDPType::Answer, sdp);
                webrtc.emit_by_name::<()>("set-remote-description", &[&answer, &None::<gst::Promise>]);
            }
            Some("ice") => {
                let mline = message["sdpMLineIndex"].as_u64().unwrap_or(0) as u32;
                let candidate = message["candidate"].as_str().unwrap_or_default();
                if !candidate.is_empty() {
                    webrtc.emit_by_name::<()>("add-ice-candidate", &[&mline, &candidate]);
                }
            }
            _ => {}
        }
    };
    stop.store(true, Ordering::SeqCst);
    let _ = encoder.pipeline.set_state(gst::State::Null);
    result
}
