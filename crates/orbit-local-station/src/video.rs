//! The session's video: WebRTC from orbit-remote-server, decoded on the
//! GPU and handed to the screen as dmabufs, with no copy on the way.
//!
//!   webrtcbin → rtph264depay → h264parse → vah264dec → vapostproc → appsink
//!
//! The server offers; `webrtcbin` here answers, and the ICE candidates go
//! both ways, all on the link's WebSocket (`Video::offer`, `Video::ice`;
//! what goes back comes out of `out`). `vapostproc` turns the decoded NV12
//! into XRGB at the monitor's size, the picture kept whole with borders
//! around it, so the screen only has to show it. A frame drawn late is
//! dropped for the next one: the newest is what counts.

use std::{
    os::fd::BorrowedFd,
    sync::mpsc,
};

use anyhow::{Context, anyhow};
use gst::prelude::*;
use serde_json::{Value, json};
use smithay::{
    backend::allocator::{
        Fourcc, Modifier,
        dmabuf::{Dmabuf, DmabufFlags},
    },
    reexports::calloop::channel,
};

use crate::link::{Event, Wake};

/// What the jitter buffer holds back, in milliseconds: none on a LAN,
/// where packets come in order and on time.
const LATENCY_MS: u32 = 0;

/// A decoded picture, ready for the screen.
pub struct Frame {
    pub dmabuf: Dmabuf,
    /// The server's picture size, before it was fitted to the monitor.
    pub source: (u32, u32),
    /// Its memory stays the decoder's until the frame is dropped.
    _sample: gst::Sample,
}

pub struct Video {
    pipeline: gst::Pipeline,
    webrtc: gst::Element,
}

impl Video {
    /// A receiver for a monitor of `size`: frames go to `events`, the
    /// answer and ICE candidates to `out` (with `wake` rung for each).
    pub fn new(size: (u32, u32), events: channel::Sender<Event>, out: mpsc::Sender<Value>, wake: Wake) -> anyhow::Result<Self> {
        gst::init()?;
        let pipeline = gst::Pipeline::new();
        let webrtc = gst::ElementFactory::make("webrtcbin")
            .property_from_str("bundle-policy", "max-bundle")
            .property("latency", LATENCY_MS)
            .build()
            .context("no webrtcbin (gstreamer1.0-plugins-bad, gstreamer1.0-nice)")?;
        pipeline.add(&webrtc)?;
        {
            webrtc.connect("on-ice-candidate", false, move |values| {
                let mline = values[1].get::<u32>().expect("an m-line");
                let candidate = values[2].get::<String>().expect("a candidate");
                let _ = out.send(json!({ "type": "ice", "candidate": candidate, "sdpMLineIndex": mline }));
                wake.ring();
                None
            });
        }
        {
            let pipeline = pipeline.downgrade();
            webrtc.connect_pad_added(move |_, pad| {
                if pad.direction() != gst::PadDirection::Src {
                    return;
                }
                let Some(pipeline) = pipeline.upgrade() else { return };
                if let Err(err) = decode(&pipeline, pad, size, events.clone()) {
                    log::error!("station: the video: {err:#}");
                    let _ = events.send(Event::Lost(format!("the video: {err:#}")));
                }
            });
        }
        pipeline.set_state(gst::State::Playing).map_err(|e| anyhow!("starting WebRTC: {e}"))?;
        Ok(Self { pipeline, webrtc })
    }

    /// The server's offer: answered, the answer sent on `out`.
    pub fn offer(&self, sdp: &str, out: mpsc::Sender<Value>, wake: Wake) -> anyhow::Result<()> {
        let sdp = gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes()).map_err(|_| anyhow!("a bad offer"))?;
        let offer = gst_webrtc::WebRTCSessionDescription::new(gst_webrtc::WebRTCSDPType::Offer, sdp);
        self.webrtc.emit_by_name::<()>("set-remote-description", &[&offer, &None::<gst::Promise>]);
        let webrtc = self.webrtc.clone();
        let promise = gst::Promise::with_change_func(move |reply| {
            let answer = match reply {
                Ok(Some(reply)) => reply.value("answer").ok().and_then(|v| v.get::<gst_webrtc::WebRTCSessionDescription>().ok()),
                _ => None,
            };
            let Some(answer) = answer else { return log::error!("station: WebRTC made no answer") };
            webrtc.emit_by_name::<()>("set-local-description", &[&answer, &None::<gst::Promise>]);
            let sdp = answer.sdp().as_text().unwrap_or_default();
            let _ = out.send(json!({ "type": "answer", "sdp": sdp }));
            wake.ring();
        });
        self.webrtc.emit_by_name::<()>("create-answer", &[&None::<gst::Structure>, &promise]);
        Ok(())
    }

    /// An ICE candidate from the server.
    pub fn ice(&self, message: &Value) {
        let mline = message["sdpMLineIndex"].as_u64().unwrap_or(0) as u32;
        let candidate = message["candidate"].as_str().unwrap_or_default();
        if !candidate.is_empty() {
            self.webrtc.emit_by_name::<()>("add-ice-candidate", &[&mline, &candidate]);
        }
    }

    /// What went wrong in the pipeline, if anything has.
    pub fn failed(&self) -> Option<String> {
        let bus = self.pipeline.bus()?;
        let message = bus.pop_filtered(&[gst::MessageType::Error])?;
        match message.view() {
            gst::MessageView::Error(err) => Some(format!("the video: {}", err.error())),
            _ => None,
        }
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// The decoding chain behind `webrtcbin`'s new `pad`.
fn decode(pipeline: &gst::Pipeline, pad: &gst::Pad, (width, height): (u32, u32), events: channel::Sender<Event>) -> anyhow::Result<()> {
    let depay = gst::ElementFactory::make("rtph264depay").build()?;
    let parse = gst::ElementFactory::make("h264parse").build()?;
    let decoder = gst::ElementFactory::make("vah264dec").build().context("no VA-API H.264 decoder (gstreamer1.0-plugins-bad, mesa-va-drivers)")?;
    let post = gst::ElementFactory::make("vapostproc").property("add-borders", true).build()?;
    let caps = gst::Caps::builder("video/x-raw")
        .features([gst_allocators::CAPS_FEATURE_MEMORY_DMABUF])
        .field("format", "DMA_DRM")
        .field("drm-format", "XR24")
        .field("width", width as i32)
        .field("height", height as i32)
        .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
        .build();
    let sink = gst_app::AppSink::builder().caps(&caps).sync(false).max_buffers(1).drop(true).build();
    // A dmabuf's planes are only known from its VideoMeta: said so to
    // `vapostproc`, which will not hand dmabufs out otherwise.
    sink.static_pad("sink").context("appsink's pad")?.add_probe(gst::PadProbeType::QUERY_DOWNSTREAM, |_, info| {
        if let Some(gst::QueryViewMut::Allocation(query)) = info.query_mut().map(|q| q.view_mut()) {
            query.add_allocation_meta::<gst_video::VideoMeta>(None);
        }
        gst::PadProbeReturn::Ok
    });
    let decoded = decoder.static_pad("src").context("the decoder's pad")?;
    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                match frame(sample, &decoded) {
                    Ok(frame) => {
                        let _ = events.send(Event::Frame(frame));
                    }
                    Err(err) => log::warn!("station: a frame: {err:#}"),
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
    let elements = [&depay, &parse, &decoder, &post, sink.upcast_ref()];
    pipeline.add_many(elements)?;
    gst::Element::link_many(elements)?;
    for element in elements {
        element.sync_state_with_parent()?;
    }
    pad.link(&depay.static_pad("sink").context("the depayloader's pad")?)?;
    log::info!("station: the video is coming, fitted to {width}x{height}");
    Ok(())
}

/// A decoded sample as a dmabuf; `decoded` is the decoder's pad, whose
/// caps give the server's size.
fn frame(sample: gst::Sample, decoded: &gst::Pad) -> anyhow::Result<Frame> {
    let caps = sample.caps().context("no caps")?;
    let info = gst_video::VideoInfoDmaDrm::from_caps(caps).map_err(|e| anyhow!("{e}"))?;
    let buffer = sample.buffer().context("no buffer")?;
    let meta = buffer.meta::<gst_video::VideoMeta>().context("no VideoMeta")?;
    let fourcc = Fourcc::try_from(info.fourcc()).map_err(|_| anyhow!("an unknown format {:#x}", info.fourcc()))?;
    let mut builder = Dmabuf::builder((meta.width() as i32, meta.height() as i32), fourcc, Modifier::from(info.modifier()), DmabufFlags::empty());
    for plane in 0..meta.n_planes() as usize {
        // Planes share a memory unless there are as many as planes.
        let memory = buffer.peek_memory(plane.min(buffer.n_memory() - 1));
        let memory = memory.downcast_memory_ref::<gst_allocators::DmaBufMemory>().context("not a dmabuf")?;
        // SAFETY: the fd is the memory's, alive while `sample` is.
        let fd = unsafe { BorrowedFd::borrow_raw(memory.fd()) }.try_clone_to_owned()?;
        let offset = meta.offset()[plane] + memory.offset();
        builder.add_plane(fd, offset as u32, meta.stride()[plane] as u32);
    }
    let dmabuf = builder.build().context("an empty dmabuf")?;
    let source = decoded
        .current_caps()
        .and_then(|caps| gst_video::VideoInfo::from_caps(&caps).ok())
        .map_or((meta.width(), meta.height()), |info| (info.width(), info.height()));
    Ok(Frame { dmabuf, source, _sample: sample })
}
