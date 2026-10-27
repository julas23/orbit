//! The client's input, on lanes of its own beside the video, in the same
//! WebRTC connection (UDP, DTLS): two data channels, so that a lost
//! packet holds back nothing behind it, as TCP (the WebSocket) would.
//!
//! - `input`, reliable and in order: keys, buttons, the wheel, a pad's
//!   buttons, the pads' coming and going. Nothing there may be lost.
//! - `motion`, neither: the mouse's moves and the pads' axes. Each is a
//!   whole position, so a lost one is made good by the next; one that
//!   comes late, after a newer one, is dropped by its number (`seq`, one
//!   count a pointer or a pad). Once still, the client sends where it
//!   stopped again on `input`, so the last position always arrives.
//!
//! The WebSocket still takes input, until the channels are open or from a
//! client without them.
//!
//! What comes in goes to a thread of its own, blocked on its inbox: no
//! wait between a message and the session. `ping` is answered (`pong`) on
//! its own channel at once, for the client to measure the way there and
//! back.

use std::{
    collections::HashMap,
    sync::mpsc,
    thread::JoinHandle,
};

use anyhow::Context;
use gst::prelude::*;
use serde_json::{Value, json};

use crate::{input::Input, pads::Pads};

/// What the input thread takes.
const INPUT: [&str; 7] = ["move", "button", "wheel", "key", "pad", "pad-input", "pad-gone"];

pub struct Lanes {
    inbox: mpsc::Sender<Value>,
    thread: Option<JoinHandle<()>>,
    /// Kept for as long as the session: dropped, they would close.
    _channels: [gst_webrtc::WebRTCDataChannel; 2],
}

impl Lanes {
    /// The channels in `webrtc` (ready, before its offer), and the thread
    /// that gives their input to the session on `output`.
    pub fn open(webrtc: &gst::Element, output: String) -> anyhow::Result<Self> {
        let (inbox, messages) = mpsc::channel::<Value>();
        let thread = std::thread::Builder::new().name("remote-input".into()).spawn(move || steer(&output, &messages))?;
        let make = |label: &str, config: Option<gst::Structure>| -> anyhow::Result<gst_webrtc::WebRTCDataChannel> {
            let channel = webrtc
                .emit_by_name::<Option<gst_webrtc::WebRTCDataChannel>>("create-data-channel", &[&label, &config])
                .with_context(|| format!("no data channel {label} (gstreamer1.0-plugins-bad's sctp)"))?;
            let inbox = inbox.clone();
            channel.connect_on_message_string(move |channel, text| {
                let Some(message) = text.and_then(|t| serde_json::from_str::<Value>(t).ok()) else { return };
                if message["type"] == "ping" {
                    let _ = channel.send_string_full(Some(&json!({ "type": "pong", "t": message["t"] }).to_string()));
                } else {
                    let _ = inbox.send(message);
                }
            });
            let name = label.to_string();
            channel.connect_on_open(move |_| log::info!("remote: the {name} channel is open"));
            Ok(channel)
        };
        let input = make("input", None)?;
        let motion = make("motion", Some(gst::Structure::builder("config").field("ordered", false).field("max-retransmits", 0i32).build()))?;
        Ok(Self { inbox, thread: Some(thread), _channels: [input, motion] })
    }

    /// A message from the WebSocket: true if it was input, now given on.
    pub fn take(&self, message: &Value) -> bool {
        let input = message["type"].as_str().is_some_and(|kind| INPUT.contains(&kind));
        if input {
            let _ = self.inbox.send(message.clone());
        }
        input
    }
}

impl Drop for Lanes {
    fn drop(&mut self) {
        // Null ends the thread; the pads and the virtual devices go with it.
        let _ = self.inbox.send(Value::Null);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The input thread: until told to stop (Null) or no one is left to send.
fn steer(output: &str, messages: &mpsc::Receiver<Value>) {
    // A compositor without virtual input (an older orbit-core) still
    // shows the session: the client only watches, its pads still work.
    let mut input = match Input::open(output) {
        Ok(input) => Some(input),
        Err(err) => {
            log::warn!("remote: no remote mouse and keys, the session is view only: {err:#}");
            None
        }
    };
    let mut pads = Pads::default();
    let mut newest = Newest::default();
    while let Ok(message) = messages.recv() {
        if message.is_null() {
            break;
        }
        if !newest.fresh(&message) || pads.handle(&message) {
            continue;
        }
        if let Some(handler) = &mut input
            && let Err(err) = handler.handle(&message)
        {
            log::warn!("remote: the remote input: {err:#}; view only from now on");
            input = None;
        }
    }
}

/// The last number seen of each pointer and pad: what is older is late.
#[derive(Default)]
struct Newest(HashMap<String, u64>);

impl Newest {
    fn fresh(&mut self, message: &Value) -> bool {
        let stream = match message["type"].as_str() {
            Some("move" | "button") => "pointer".to_string(),
            Some("pad-input") => format!("pad {}", message["id"]),
            // A pad described again counts from the start.
            Some("pad") => {
                self.0.remove(&format!("pad {}", message["id"]));
                return true;
            }
            _ => return true,
        };
        let Some(seq) = message["seq"].as_u64() else { return true };
        let last = self.0.entry(stream).or_insert(0);
        // The same number again is the position sent once more, at rest.
        if seq < *last {
            return false;
        }
        *last = seq;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_positions_are_dropped() {
        let mut newest = Newest::default();
        assert!(newest.fresh(&json!({ "type": "move", "seq": 2 })));
        assert!(!newest.fresh(&json!({ "type": "move", "seq": 1 })));
        // At rest, the last again.
        assert!(newest.fresh(&json!({ "type": "move", "seq": 2 })));
        // Each pad counts on its own; keys are never late.
        assert!(newest.fresh(&json!({ "type": "pad-input", "id": 1, "seq": 1 })));
        assert!(newest.fresh(&json!({ "type": "key", "seq": 0 })));
        assert!(newest.fresh(&json!({ "type": "pad", "id": 1 })));
        assert!(newest.fresh(&json!({ "type": "pad-input", "id": 1, "seq": 0 })));
        // A client without numbers (an older one) is taken as it comes.
        assert!(newest.fresh(&json!({ "type": "move" })));
    }
}
