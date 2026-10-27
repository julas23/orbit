//! The station's input on the server's data channels (orbit-remote-server's
//! `lanes`), in the WebRTC connection beside the video: `input`, reliable,
//! for keys, buttons, the wheel and a pad's buttons; `motion`, unordered
//! and never sent twice, for the mouse's moves and the pads' axes, where
//! a lost position is made good by the next and a late one is dropped by
//! its number (`seq`).
//!
//! Once a pointer or a pad has been still for `REST`, where it stopped is
//! sent again on `input`: the last position always arrives, even when the
//! packet that carried it was lost.
//!
//! Until the server's channels are open (or with a server without them)
//! everything goes on the WebSocket, as before.
//!
//! Every few seconds a `ping` on `motion` measures the way to the server
//! and back, in the log.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use gst::prelude::*;
use serde_json::{Value, json};

/// Still for this long: the last position again, on `input`.
const REST: Duration = Duration::from_millis(50);
const PING: Duration = Duration::from_secs(2);
/// The round trips told in the log, every this many pings.
const TELL: u32 = 15;

const EV_ABS: u64 = 3;

fn start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

/// The round trips measured since they were last told, in milliseconds.
#[derive(Default)]
struct Trips(Vec<f64>);

/// The server's channels, as `webrtcbin` hands them over.
#[derive(Clone, Default)]
pub struct Channels {
    input: Arc<Mutex<Option<gst_webrtc::WebRTCDataChannel>>>,
    motion: Arc<Mutex<Option<gst_webrtc::WebRTCDataChannel>>>,
    trips: Arc<Mutex<Trips>>,
}

impl Channels {
    /// Takes the channels the server's offer brings into `webrtc`.
    pub fn attach(&self, webrtc: &gst::Element) {
        let channels = self.clone();
        webrtc.connect("on-data-channel", false, move |values| {
            let channel = values[1].get::<gst_webrtc::WebRTCDataChannel>().expect("a data channel");
            let label = channel.label().unwrap_or_default();
            let slot = match label.as_str() {
                "input" => &channels.input,
                "motion" => &channels.motion,
                other => {
                    log::debug!("station: a data channel {other:?}, not taken");
                    return None;
                }
            };
            let trips = channels.trips.clone();
            channel.connect_on_message_string(move |_, text| {
                let Some(message) = text.and_then(|t| serde_json::from_str::<Value>(t).ok()) else { return };
                if message["type"] == "pong"
                    && let Some(sent) = message["t"].as_f64()
                {
                    let now = start().elapsed().as_secs_f64() * 1000.0;
                    trips.lock().unwrap().0.push(now - sent);
                }
            });
            log::info!("station: the server's {label} channel is here");
            *slot.lock().unwrap() = Some(channel);
            None
        });
    }

    /// The channel in `slot`, if it is open.
    fn open(slot: &Mutex<Option<gst_webrtc::WebRTCDataChannel>>) -> Option<gst_webrtc::WebRTCDataChannel> {
        slot.lock().unwrap().clone().filter(|c| c.ready_state() == gst_webrtc::WebRTCDataChannelState::Open)
    }
}

/// A pointer's or a pad's last position, to send again once still.
struct Resting {
    seq: u64,
    /// When it is due, if a position went unreliably since the last time.
    due: Option<Instant>,
    /// The pointer's last move, or a pad's axes as they last were.
    last: Option<Value>,
    axes: BTreeMap<u64, Value>,
}

/// The input of one session, on its way to the server.
pub struct Lanes {
    channels: Channels,
    /// By stream: `pointer`, or a pad's id.
    streams: HashMap<String, Resting>,
    pinged: Option<Instant>,
    pings: u32,
}

impl Lanes {
    pub fn new(channels: Channels) -> Self {
        Self { channels, streams: HashMap::new(), pinged: None, pings: 0 }
    }

    /// Sends `message` on its channel; if that is not open yet, gives it
    /// back for the WebSocket.
    pub fn send(&mut self, mut message: Value) -> Option<Value> {
        let kind = message["type"].as_str().unwrap_or_default().to_string();
        let mut fast = false;
        match kind.as_str() {
            "move" => {
                let stream = self.stream("pointer".into());
                stream.seq += 1;
                message["seq"] = stream.seq.into();
                stream.last = Some(message.clone());
                fast = true;
            }
            // The click where the pointer is: the same number as its move.
            "button" => message["seq"] = self.stream("pointer".into()).seq.into(),
            "pad-input" => {
                let stream = self.stream(format!("pad {}", message["id"]));
                stream.seq += 1;
                message["seq"] = stream.seq.into();
                let events = message["events"].as_array().cloned().unwrap_or_default();
                for event in &events {
                    if event[0].as_u64() == Some(EV_ABS)
                        && let Some(code) = event[1].as_u64()
                    {
                        stream.axes.insert(code, event[2].clone());
                    }
                }
                // Only axes: a position. With a button: input that must arrive.
                fast = events.iter().all(|event| event[0].as_u64() == Some(EV_ABS));
            }
            "pad-gone" => {
                self.streams.remove(&format!("pad {}", message["id"]));
            }
            _ => {}
        }
        let channel = if fast { Channels::open(&self.channels.motion).or_else(|| Channels::open(&self.channels.input)) } else { Channels::open(&self.channels.input) };
        let Some(channel) = channel else { return Some(message) };
        if channel.send_string_full(Some(&message.to_string())).is_err() {
            return Some(message);
        }
        if fast && channel.label().as_deref() == Some("motion") {
            let key = if kind == "move" { "pointer".to_string() } else { format!("pad {}", message["id"]) };
            if let Some(stream) = self.streams.get_mut(&key) {
                stream.due = Some(Instant::now() + REST);
            }
        }
        None
    }

    fn stream(&mut self, key: String) -> &mut Resting {
        self.streams.entry(key).or_insert(Resting { seq: 0, due: None, last: None, axes: BTreeMap::new() })
    }

    /// The positions now still, sent again on `input`; a ping when it is
    /// time.
    pub fn settle(&mut self) {
        let now = Instant::now();
        if let Some(input) = Channels::open(&self.channels.input) {
            for (key, stream) in &mut self.streams {
                if stream.due.is_none_or(|due| due > now) {
                    continue;
                }
                stream.due = None;
                let again = if key == "pointer" {
                    stream.last.clone()
                } else {
                    let id: u64 = key.trim_start_matches("pad ").parse().unwrap_or(0);
                    let events: Vec<Value> = stream.axes.iter().map(|(code, value)| json!([EV_ABS, code, value])).collect();
                    Some(json!({ "type": "pad-input", "id": id, "events": events, "seq": stream.seq }))
                };
                if let Some(again) = again {
                    let _ = input.send_string_full(Some(&again.to_string()));
                }
            }
        }
        if self.pinged.is_none_or(|at| at.elapsed() >= PING)
            && let Some(motion) = Channels::open(&self.channels.motion)
        {
            self.pinged = Some(now);
            let t = start().elapsed().as_secs_f64() * 1000.0;
            let _ = motion.send_string_full(Some(&json!({ "type": "ping", "t": t }).to_string()));
            self.pings += 1;
            if self.pings.is_multiple_of(TELL) {
                let trips = std::mem::take(&mut self.channels.trips.lock().unwrap().0);
                if !trips.is_empty() {
                    let least = trips.iter().copied().fold(f64::INFINITY, f64::min);
                    let mean = trips.iter().sum::<f64>() / trips.len() as f64;
                    let lost = TELL as usize - trips.len().min(TELL as usize);
                    log::info!("station: to the server and back {mean:.1} ms (least {least:.1}), {lost} of {TELL} pings lost");
                }
            }
        }
    }

    /// How long until `settle` has something to do.
    pub fn wait(&self) -> Option<Duration> {
        let now = Instant::now();
        let rest = self.streams.values().filter_map(|s| s.due).min().map(|due| due.saturating_duration_since(now));
        let ping = self.pinged.map(|at| PING.saturating_sub(at.elapsed()));
        [rest, ping].into_iter().flatten().min()
    }
}
