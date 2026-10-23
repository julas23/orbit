//! Serving the session: one port on the LAN (7311) for the browser's page,
//! `GET /`, and its WebSocket, `GET /ws`, which carries only the pairing
//! and WebRTC's signalling (the offer, the answer, the ICE candidates);
//! the video goes over WebRTC itself (UDP).
//!
//! Pairing: a six-digit code shows on the server's screen (a notification)
//! and the browser must send it first; five wrong codes make a new one.
//! One client at a time.
//!
//! A session: `webrtcbin` at the encoder's tail (`encode::Tail::WebRtc`),
//! offering the video; a thread captures and pushes frames until the
//! browser goes away.

use std::{
    io::ErrorKind,
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
use tungstenite::{Message, WebSocket};

use crate::{
    capture::Capture,
    encode::{Encoder, Tail},
};

const PAGE: &str = include_str!("../web/index.html");
/// Wrong codes before a new one is made.
const TRIES: u32 = 5;

struct Shared {
    output: Option<String>,
    code: Mutex<String>,
    wrong: Mutex<u32>,
    busy: AtomicBool,
}

pub fn serve(port: u16, output: Option<String>) -> anyhow::Result<()> {
    gst::init()?;
    let listener = TcpListener::bind(("0.0.0.0", port)).with_context(|| format!("listening on port {port}"))?;
    let shared = Arc::new(Shared {
        output,
        code: Mutex::new(new_code()),
        wrong: Mutex::new(0),
        busy: AtomicBool::new(false),
    });
    show_code(&shared.code.lock().unwrap());
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
    let given = first["code"].as_str().unwrap_or_default();
    let right = first["type"] == "pair" && given == *shared.code.lock().unwrap();
    if !right {
        let mut wrong = shared.wrong.lock().unwrap();
        *wrong += 1;
        log::warn!("remote: {peer} gave a wrong code ({} of {TRIES})", *wrong);
        if *wrong >= TRIES {
            *wrong = 0;
            let code = new_code();
            show_code(&code);
            *shared.code.lock().unwrap() = code;
        }
        ws.send(Message::text(json!({ "type": "refused" }).to_string()))?;
        return Ok(());
    }
    if shared.busy.swap(true, Ordering::SeqCst) {
        ws.send(Message::text(json!({ "type": "busy" }).to_string()))?;
        return Ok(());
    }
    *shared.wrong.lock().unwrap() = 0;
    log::info!("remote: {peer} paired");
    let result = stream(&mut ws, shared);
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
fn stream(ws: &mut WebSocket<TcpStream>, shared: &Shared) -> anyhow::Result<()> {
    let mut capture = Capture::open(shared.output.as_deref(), true)?;
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
    let encoder = Encoder::new(&capture.planes, Tail::WebRtc(webrtc.clone()))?;
    let encoder = Arc::new(encoder);

    // Frames, from their own thread, until told to stop.
    let stop = Arc::new(AtomicBool::new(false));
    let pusher = {
        let (stop, encoder) = (stop.clone(), encoder.clone());
        std::thread::spawn(move || -> anyhow::Result<()> {
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
