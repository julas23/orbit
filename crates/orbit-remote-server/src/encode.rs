//! Encoding the captured frames: H.264 on the GPU (VA-API), for low
//! latency. The dmabuf the compositor drew into goes into GStreamer as it
//! is (`memory:DMABuf`), `vapostproc` turns it into NV12 on the GPU and
//! `vah264enc` encodes it: constant bitrate, no B-frames, a key frame every
//! two seconds and whenever the other end asks (a lost frame).
//!
//! The capture's buffers are taken in turn (`capture::BUFFERS`): one is
//! drawn into again three frames after it was pushed, 50 ms at 60 fps,
//! while the pipeline lets go of it within the 8 ms it takes to encode.
//!
//! The encoded frames go to an `appsink` (`measure`) or, for a client,
//! through `rtph264pay` to WebRTC (`Tail::WebRtc`).

use std::{
    os::fd::AsFd,
    time::{Duration, Instant},
};

use anyhow::{Context, anyhow};
use gst::prelude::*;
use gst_allocators::prelude::*;

use crate::capture::{Capture, Captured, Plane};

/// Bits a second for the video.
pub const BITRATE_KBPS: u32 = 20_000;
/// A key frame at least this often, in frames.
const KEY_INTERVAL: u32 = 120;

/// Where the encoded frames go.
pub enum Tail {
    /// An `appsink`, to read them (`Encoder::pull`).
    Sink,
    /// RTP into this `webrtcbin`.
    WebRtc(gst::Element),
}

/// The encoder: frames in (`push`), H.264 out of its tail.
pub struct Encoder {
    pub pipeline: gst::Pipeline,
    source: gst_app::AppSrc,
    sink: Option<gst_app::AppSink>,
    allocator: gst_allocators::DmaBufAllocator,
    start: Instant,
}

impl Encoder {
    /// A pipeline for `planes` (the capture's buffers, all the same size)
    /// into `tail`, playing.
    pub fn new(planes: &[Plane], tail: Tail) -> anyhow::Result<Self> {
        gst::init()?;
        let first = planes.first().context("no buffers")?;
        let caps = gst::Caps::builder("video/x-raw")
            .features([gst_allocators::CAPS_FEATURE_MEMORY_DMABUF])
            .field("format", "DMA_DRM")
            .field("drm-format", "XR24")
            .field("width", first.width as i32)
            .field("height", first.height as i32)
            // Nominal: frames come as the screen changes, at most this often.
            .field("framerate", gst::Fraction::new(60, 1))
            .build();
        let source = gst_app::AppSrc::builder()
            .caps(&caps)
            .is_live(true)
            .format(gst::Format::Time)
            .do_timestamp(false)
            .max_buffers(2)
            .leaky_type(gst_app::AppLeakyType::Downstream)
            .build();
        let convert = gst::ElementFactory::make("vapostproc").build().context("no vapostproc (gstreamer1.0-plugins-bad)")?;
        let nv12 = gst::Caps::builder("video/x-raw").features(["memory:VAMemory"]).field("format", "NV12").build();
        let filter = gst::ElementFactory::make("capsfilter").property("caps", &nv12).build()?;
        let encoder = gst::ElementFactory::make("vah264enc")
            .property_from_str("rate-control", "cbr")
            .property("bitrate", BITRATE_KBPS)
            .property("b-frames", 0u32)
            .property("ref-frames", 1u32)
            .property("key-int-max", KEY_INTERVAL)
            .property("target-usage", 7u32)
            .build()
            .context("no vah264enc (VA-API H.264 encoding)")?;
        let parse = gst::ElementFactory::make("h264parse").property("config-interval", -1i32).build()?;
        let pipeline = gst::Pipeline::new();
        pipeline.add_many([source.upcast_ref(), &convert, &filter, &encoder, &parse])?;
        gst::Element::link_many([source.upcast_ref(), &convert, &filter, &encoder, &parse])?;
        let sink = match tail {
            Tail::Sink => {
                let au = gst::Caps::builder("video/x-h264").field("stream-format", "byte-stream").field("alignment", "au").build();
                let sink = gst_app::AppSink::builder().caps(&au).sync(false).max_buffers(4).build();
                pipeline.add(&sink)?;
                parse.link(&sink)?;
                Some(sink)
            }
            Tail::WebRtc(webrtc) => {
                // Each frame sent as soon as it is there, not gathered.
                let pay = gst::ElementFactory::make("rtph264pay")
                    .property("config-interval", -1i32)
                    .property_from_str("aggregate-mode", "zero-latency")
                    .build()?;
                let rtp = gst::Caps::builder("application/x-rtp")
                    .field("media", "video")
                    .field("encoding-name", "H264")
                    .field("payload", 96i32)
                    .build();
                let filter = gst::ElementFactory::make("capsfilter").property("caps", &rtp).build()?;
                pipeline.add_many([&pay, &filter, &webrtc])?;
                gst::Element::link_many([&parse, &pay, &filter, &webrtc])?;
                None
            }
        };
        pipeline.set_state(gst::State::Playing)?;
        Ok(Self {
            pipeline,
            source,
            sink,
            allocator: gst_allocators::DmaBufAllocator::new(),
            start: Instant::now(),
        })
    }

    /// Hands frame `frame` (in `planes[frame.index]`) to the encoder.
    pub fn push(&self, planes: &[Plane], frame: &Captured) -> anyhow::Result<()> {
        let plane = &planes[frame.index];
        let fd = plane.fd.as_fd().try_clone_to_owned()?;
        let size = plane.offset as usize + plane.stride as usize * plane.height as usize;
        // SAFETY: the memory owns the duplicate, closed when it is freed.
        let memory = unsafe { self.allocator.alloc_dmabuf(fd, size) }.map_err(|e| anyhow!("dmabuf memory: {e}"))?;
        let mut buffer = gst::Buffer::new();
        {
            let buffer = buffer.get_mut().expect("just made");
            buffer.append_memory(memory);
            gst_video::VideoMeta::add_full(
                buffer,
                gst_video::VideoFrameFlags::empty(),
                gst_video::VideoFormat::DmaDrm,
                plane.width,
                plane.height,
                &[plane.offset as usize],
                &[plane.stride as i32],
            )?;
            let pts = gst::ClockTime::from_nseconds(self.start.elapsed().as_nanos() as u64);
            buffer.set_pts(pts);
        }
        self.source.push_buffer(buffer).map_err(|e| anyhow!("pushing a frame: {e:?}"))?;
        Ok(())
    }

    /// The next encoded frame (an H.264 access unit), within `wait`; with
    /// the `Sink` tail only.
    pub fn pull(&self, wait: Duration) -> Option<gst::Sample> {
        self.sink.as_ref()?.try_pull_sample(gst::ClockTime::from_nseconds(wait.as_nanos() as u64))
    }

}

impl Drop for Encoder {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// `orbit-remote-server encode [OUTPUT]`: captures and encodes for 10 s,
/// says how long a frame takes from the copy to H.264 and how big the
/// stream is, and writes it to `$XDG_RUNTIME_DIR/orbit-remote-test.h264`.
pub fn measure(output: Option<&str>) -> anyhow::Result<()> {
    let mut capture = Capture::open(output, true)?;
    let encoder = Encoder::new(&capture.planes, Tail::Sink)?;
    let out_path = std::env::temp_dir().join("orbit-remote-test.h264");
    let path = std::env::var_os("XDG_RUNTIME_DIR").map_or(out_path, |d| std::path::PathBuf::from(d).join("orbit-remote-test.h264"));
    let mut file = std::fs::File::create(&path)?;
    let start = Instant::now();
    let (mut frames, mut bytes, mut latencies) = (0u32, 0usize, Vec::new());
    while start.elapsed() < Duration::from_secs(10) {
        let frame = capture.next()?;
        let pushed = Instant::now();
        encoder.push(&capture.planes, &frame)?;
        let sample = encoder.pull(Duration::from_millis(500)).context("no encoded frame within 500 ms")?;
        latencies.push(pushed.elapsed());
        let buffer = sample.buffer().context("an empty sample")?;
        let map = buffer.map_readable()?;
        std::io::Write::write_all(&mut file, &map)?;
        bytes += map.len();
        frames += 1;
    }
    drop(encoder);
    latencies.sort();
    let at = |q: f64| latencies[((latencies.len() - 1) as f64 * q) as usize];
    let seconds = start.elapsed().as_secs_f64();
    println!(
        "{}: {frames} frames, encode {:?} median, {:?} at 95%, {:?} worst; {:.0} kbit/s; {}",
        capture.name,
        at(0.5),
        at(0.95),
        latencies.last().copied().unwrap_or_default(),
        bytes as f64 * 8.0 / 1000.0 / seconds,
        path.display()
    );
    Ok(())
}
