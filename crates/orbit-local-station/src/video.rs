//! The session's video: WebRTC from orbit-remote-server, decoded on the
//! GPU and handed to the screen as dmabufs, with no copy on the way; on a
//! Raspberry Pi by its V4L2 decoder, and on a machine with neither by the
//! CPU, into memory the screen uploads. The sound comes in the same
//! connection and goes to `audio`.
//!
//!   webrtcbin → rtph264depay → h264parse → vah264dec → vapostproc → appsink
//!   webrtcbin → rtph264depay → h264parse → v4l2h264dec → v4l2convert → appsink
//!   webrtcbin → rtph264depay → h264parse → avdec_h264 → videoconvert → appsink
//!
//! The server offers; `webrtcbin` here answers, and the ICE candidates go
//! both ways, all on the link's WebSocket (`Video::offer`, `Video::ice`;
//! what goes back comes out of `out`). The offer brings the server's data
//! channels for the input too (`lanes`). `vapostproc` turns the decoded NV12
//! into XRGB at the monitor's size, the picture kept whole with borders
//! around it, so the screen only has to show it. Otherwise `v4l2convert`
//! (the Pi's ISP) or `videoconvert` makes XRGB at the server's size and
//! the screen scales it as it draws: the GPU does that for nothing, the CPU would take
//! longer than the decoding. A frame drawn late is dropped for the next
//! one: the newest is what counts.

use std::{
    os::fd::BorrowedFd,
    sync::{OnceLock, mpsc},
};

use anyhow::{Context, anyhow};
use gst::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use smithay::{
    backend::allocator::{
        Fourcc, Modifier,
        dmabuf::{Dmabuf, DmabufFlags},
    },
    reexports::calloop::channel,
};

use crate::{
    lanes::Channels,
    link::{Event, Wake},
};

/// What the jitter buffer holds back, in milliseconds: none on a LAN,
/// where packets come in order and on time.
const LATENCY_MS: u32 = 0;

/// How the video is decoded: `decoder:` in station.yaml.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Decoder {
    /// VA-API when the machine has a decoder for H.264, else V4L2's (the
    /// Raspberry Pi's), else software.
    #[default]
    Auto,
    Vaapi,
    V4l2,
    Software,
}

static DECODER: OnceLock<Decoder> = OnceLock::new();
static RENDER_NODE: OnceLock<String> = OnceLock::new();

/// The decoder asked for, before the first video.
pub fn set_decoder(decoder: Decoder) {
    let _ = DECODER.set(decoder);
}

/// The render node of the GPU that drives the monitor (as `renderD129`):
/// VA-API decodes there.
pub fn set_render_node(node: String) {
    let _ = RENDER_NODE.set(node);
}

/// A VA element (`h264dec`, `postproc`) of the monitor's GPU. GStreamer
/// names those of its first VA device `vah264dec`, `vapostproc`, the
/// others' after their node (`varenderD129h264dec`); with two GPUs, the
/// first one's could not hand its pictures to the other's screen (the
/// think: an Intel first, the monitor on an AMD).
fn va_element(kind: &str) -> String {
    RENDER_NODE
        .get()
        .map(|node| format!("va{node}{kind}"))
        .filter(|name| gst::ElementFactory::find(name).is_some())
        .unwrap_or_else(|| format!("va{kind}"))
}

/// The decoder to use. GStreamer's VA and V4L2 plugins register
/// `vah264dec` and `v4l2h264dec` only when a device can decode H.264, so
/// finding them is the test.
fn decoder() -> Decoder {
    let has = |name| gst::ElementFactory::find(name).is_some();
    match DECODER.get().copied().unwrap_or_default() {
        Decoder::Auto if has("vah264dec") => Decoder::Vaapi,
        Decoder::Auto if has("v4l2h264dec") => Decoder::V4l2,
        Decoder::Auto => Decoder::Software,
        decoder => decoder,
    }
}

/// A decoded picture, ready for the screen.
pub struct Frame {
    pub picture: Picture,
    /// The server's picture size, before it was fitted to the monitor.
    pub source: (u32, u32),
    /// Its memory stays the decoder's until the frame is dropped.
    sample: gst::Sample,
}

/// Where a decoded picture is.
pub enum Picture {
    /// On the GPU, as the VA-API decoder left it.
    Dmabuf(Dmabuf),
    /// In memory, rows of `stride` bytes (`Frame::pixels`), XRGB or XBGR.
    Memory { size: (u32, u32), stride: usize, fourcc: Fourcc },
}

impl Frame {
    /// A `Picture::Memory`'s bytes.
    pub fn pixels(&self) -> Option<gst::BufferMap<'_, gst::buffer::Readable>> {
        self.sample.buffer()?.map_readable().ok()
    }
}

pub struct Video {
    pipeline: gst::Pipeline,
    webrtc: gst::Element,
    channels: Channels,
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
        let channels = Channels::default();
        channels.attach(&webrtc);
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
                // The sound comes on its own pad; losing it leaves the video.
                let media = pad.current_caps().and_then(|caps| caps.structure(0).and_then(|s| s.get::<String>("media").ok()));
                if media.as_deref() == Some("audio") {
                    if let Err(err) = crate::audio::play(&pipeline, pad) {
                        log::warn!("station: no sound: {err:#}");
                    }
                    return;
                }
                if let Err(err) = decode(&pipeline, pad, size, events.clone()) {
                    log::error!("station: the video: {err:#}");
                    let _ = events.send(Event::Lost(format!("the video: {err:#}")));
                }
            });
        }
        pipeline.set_state(gst::State::Playing).map_err(|e| anyhow!("starting WebRTC: {e}"))?;
        Ok(Self { pipeline, webrtc, channels })
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

    /// The server's data channels, as they come.
    pub fn channels(&self) -> Channels {
        self.channels.clone()
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
    let chosen = decoder();
    let memory = chosen != Decoder::Vaapi;
    let (decoder, post, caps) = if memory {
        let decoder = if chosen == Decoder::V4l2 {
            gst::ElementFactory::make("v4l2h264dec").build().context("no V4L2 H.264 decoder")?
        } else {
            // Slices, not frames, in threads: frame threads hold pictures back.
            gst::ElementFactory::make("avdec_h264")
                .property_from_str("thread-type", "slice")
                .build()
                .context("no software H.264 decoder (gstreamer1.0-libav)")?
        };
        // The Pi's ISP converts for nothing; the CPU where there is none.
        let post = match gst::ElementFactory::make("v4l2convert").build() {
            Ok(post) if chosen == Decoder::V4l2 => post,
            _ => gst::ElementFactory::make("videoconvert").property("n-threads", 0u32).build()?,
        };
        let caps = gst::Caps::builder("video/x-raw").field("format", gst::List::new(["BGRx", "RGBx"])).build();
        (decoder, post, caps)
    } else {
        let decoder = gst::ElementFactory::make(&va_element("h264dec"))
            .build()
            .context("no VA-API H.264 decoder (gstreamer1.0-plugins-bad, mesa-va-drivers)")?;
        let post = gst::ElementFactory::make(&va_element("postproc")).property("add-borders", true).build()?;
        let caps = gst::Caps::builder("video/x-raw")
            .features([gst_allocators::CAPS_FEATURE_MEMORY_DMABUF])
            .field("format", "DMA_DRM")
            .field("drm-format", "XR24")
            .field("width", width as i32)
            .field("height", height as i32)
            .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
            .build();
        (decoder, post, caps)
    };
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
                let frame = if memory { in_memory(sample, &decoded) } else { frame(sample, &decoded) };
                match frame {
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
    let how = match chosen {
        Decoder::Vaapi => "decoded by VA-API",
        Decoder::V4l2 => "decoded by V4L2",
        _ => "decoded in software",
    };
    log::info!("station: the video is coming, {how}, fitted to {width}x{height}");
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
    let source = source(decoded, (meta.width(), meta.height()));
    Ok(Frame { picture: Picture::Dmabuf(dmabuf), source, sample })
}

/// A sample in memory, its rows as they are in its buffer.
fn in_memory(sample: gst::Sample, decoded: &gst::Pad) -> anyhow::Result<Frame> {
    let caps = sample.caps().context("no caps")?;
    let info = gst_video::VideoInfo::from_caps(caps).map_err(|e| anyhow!("{e}"))?;
    // In memory order: GStreamer's BGRx is DRM's XRGB (little-endian).
    let fourcc = match info.format() {
        gst_video::VideoFormat::Bgrx => Fourcc::Xrgb8888,
        gst_video::VideoFormat::Rgbx => Fourcc::Xbgr8888,
        format => return Err(anyhow!("an unexpected format {format:?}")),
    };
    let buffer = sample.buffer().context("no buffer")?;
    let (size, stride) = match buffer.meta::<gst_video::VideoMeta>() {
        Some(meta) => ((meta.width(), meta.height()), meta.stride()[0] as usize),
        None => ((info.width(), info.height()), info.stride()[0] as usize),
    };
    let source = source(decoded, size);
    Ok(Frame { picture: Picture::Memory { size, stride, fourcc }, source, sample })
}

/// The server's picture size, from the decoder's caps (`fallback` until
/// they are known).
fn source(decoded: &gst::Pad, fallback: (u32, u32)) -> (u32, u32) {
    decoded.current_caps().and_then(|caps| gst_video::VideoInfo::from_caps(&caps).ok()).map_or(fallback, |info| (info.width(), info.height()))
}
