//! Encoding the captured frames: H.264 on the GPU, for low latency. The
//! dmabuf the compositor drew into goes into GStreamer as it is
//! (`memory:DMABuf`) and is turned into NV12 and encoded on the GPU:
//! constant bitrate (by the picture's size: 20 Mbit/s for 1080p, 80 for
//! 4K), no B-frames, a key frame every two seconds and whenever the other
//! end asks (a lost frame).
//!
//!   VA-API (AMD, Intel)   vapostproc → vah264enc
//!   NVENC (NVIDIA)        glupload → glcolorscale → glcolorconvert → nvh264enc
//!
//! A picture larger than the client's screen is made smaller on the GPU
//! to fit it, its proportions kept (`fitted`): a 4K session on an 800×600
//! monitor sends no more than it shows, and the Raspberry Pi 4's decoder
//! takes no H.264 above 1080p.
//!
//! VA-API when its encoder is there (GStreamer's `va` plugin makes it only
//! for a GPU that encodes), NVENC otherwise; `$ORBIT_REMOTE_ENCODER`
//! (`va`, `nvenc`) chooses.
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

/// Bits a second for a 1080p picture; others in proportion to their pixels.
const BITRATE_1080P_KBPS: u32 = 20_000;
/// Nor less nor more than this, whatever the size.
const BITRATE_KBPS: std::ops::RangeInclusive<u32> = 8_000..=80_000;

/// The bitrate for a picture of `width` × `height`.
fn bitrate_kbps(width: u32, height: u32) -> u32 {
    let pixels = u64::from(width) * u64::from(height);
    let kbps = u64::from(BITRATE_1080P_KBPS) * pixels / (1920 * 1080);
    (kbps as u32).clamp(*BITRATE_KBPS.start(), *BITRATE_KBPS.end())
}
/// The size to encode a `width` × `height` picture at for a client whose
/// screen is `screen`: as large as fits, never larger than it is, its
/// proportions kept, even (as NV12 wants).
pub fn fitted((width, height): (u32, u32), screen: Option<(u32, u32)>) -> (u32, u32) {
    let Some((screen_w, screen_h)) = screen else { return (width, height) };
    let scale = (f64::from(screen_w) / f64::from(width)).min(f64::from(screen_h) / f64::from(height)).min(1.0);
    let even = |side: u32| ((f64::from(side) * scale) as u32 & !1).max(2);
    (even(width), even(height))
}

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
    /// Behind the encoder: asked for key frames.
    parse: gst::Element,
    allocator: gst_allocators::DmaBufAllocator,
    start: Instant,
}

impl Encoder {
    /// A pipeline for `planes` (the capture's buffers, all the same size)
    /// into `tail`, playing, the picture fitted to `screen` (the client's).
    pub fn new(planes: &[Plane], tail: Tail, screen: Option<(u32, u32)>) -> anyhow::Result<Self> {
        gst::init()?;
        let first = planes.first().context("no buffers")?;
        let caps = gst::Caps::builder("video/x-raw")
            .features([gst_allocators::CAPS_FEATURE_MEMORY_DMABUF])
            .field("format", "DMA_DRM")
            // Linear: the format alone; otherwise with its modifier.
            .field("drm-format", if first.modifier == 0 { "XR24".to_owned() } else { format!("XR24:{:#018x}", first.modifier) })
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
        let size = fitted((first.width, first.height), screen);
        if size != (first.width, first.height) {
            log::info!("remote: {}×{} sent as {}×{}, the client's screen", first.width, first.height, size.0, size.1);
        }
        let kbps = bitrate_kbps(size.0, size.1);
        let chain = match std::env::var("ORBIT_REMOTE_ENCODER").as_deref() {
            Ok("nvenc") => nvenc(kbps, size)?,
            Ok("va") => va(kbps, size)?,
            _ if gst::ElementFactory::find("vah264enc").is_some() => va(kbps, size)?,
            _ => nvenc(kbps, size).context("no H.264 encoder on the GPU: neither VA-API's nor NVENC")?,
        };
        let parse = gst::ElementFactory::make("h264parse").property("config-interval", -1i32).build()?;
        let pipeline = gst::Pipeline::new();
        let mut elements: Vec<gst::Element> = vec![source.clone().upcast()];
        elements.extend(chain);
        elements.push(parse.clone());
        pipeline.add_many(&elements)?;
        gst::Element::link_many(&elements)?;
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
                // The sound in the same connection; without it, the video alone.
                if let Err(err) = crate::audio::attach(&pipeline, &webrtc) {
                    log::warn!("remote: no sound: {err:#}");
                }
                None
            }
        };
        // For WebRTC, ready only: its data channels are made before the
        // offer (`play` then).
        pipeline.set_state(if sink.is_some() { gst::State::Playing } else { gst::State::Ready })?;
        Ok(Self {
            pipeline,
            source,
            sink,
            parse,
            allocator: gst_allocators::DmaBufAllocator::new(),
            start: Instant::now(),
        })
    }

    /// Starts a WebRTC encoder, once its data channels are made.
    pub fn play(&self) -> anyhow::Result<()> {
        self.pipeline.set_state(gst::State::Playing)?;
        Ok(())
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

    /// Makes the next frame a key frame, with its headers: one a client
    /// can start from.
    pub fn key_frame(&self) {
        let event = gst_video::UpstreamForceKeyUnitEvent::builder().all_headers(true).build();
        if !self.parse.send_event(event) {
            log::debug!("remote: the encoder took no key frame request");
        }
    }

    /// The next encoded frame (an H.264 access unit), within `wait`; with
    /// the `Sink` tail only.
    pub fn pull(&self, wait: Duration) -> Option<gst::Sample> {
        self.sink.as_ref()?.try_pull_sample(gst::ClockTime::from_nseconds(wait.as_nanos() as u64))
    }

}

/// VA-API: NV12 at `(width, height)` by the GPU's video processor, then
/// its encoder.
fn va(kbps: u32, (width, height): (u32, u32)) -> anyhow::Result<Vec<gst::Element>> {
    log::info!("remote: encoding with VA-API, {kbps} kbit/s");
    let convert = gst::ElementFactory::make("vapostproc").build().context("no vapostproc (gstreamer1.0-plugins-bad)")?;
    let nv12 = gst::Caps::builder("video/x-raw")
        .features(["memory:VAMemory"])
        .field("format", "NV12")
        .field("width", width as i32)
        .field("height", height as i32)
        .build();
    let filter = gst::ElementFactory::make("capsfilter").property("caps", &nv12).build()?;
    let encoder = gst::ElementFactory::make("vah264enc")
        .property_from_str("rate-control", "cbr")
        .property("bitrate", kbps)
        .property("b-frames", 0u32)
        .property("ref-frames", 1u32)
        .property("key-int-max", KEY_INTERVAL)
        .property("target-usage", 7u32)
        .build()
        .context("no vah264enc (VA-API H.264 encoding)")?;
    Ok(vec![convert, filter, encoder])
}

/// NVENC: the dmabuf into OpenGL, scaled to `(width, height)` and turned
/// NV12 by shaders, then NVIDIA's encoder (which takes OpenGL memory),
/// tuned for the least latency.
fn nvenc(kbps: u32, (width, height): (u32, u32)) -> anyhow::Result<Vec<gst::Element>> {
    log::info!("remote: encoding with NVENC, {kbps} kbit/s");
    let upload = gst::ElementFactory::make("glupload").build().context("no glupload (gstreamer1.0-gl)")?;
    let scale = gst::ElementFactory::make("glcolorscale").build()?;
    let convert = gst::ElementFactory::make("glcolorconvert").build()?;
    let nv12 = gst::Caps::builder("video/x-raw")
        .features(["memory:GLMemory"])
        .field("format", "NV12")
        .field("width", width as i32)
        .field("height", height as i32)
        .build();
    let filter = gst::ElementFactory::make("capsfilter").property("caps", &nv12).build()?;
    let encoder = gst::ElementFactory::make("nvh264enc").build().context("no nvh264enc (NVENC)")?;
    // Its properties differ between versions: each set only if it is there.
    let set = |name: &str, value: &str| {
        if encoder.find_property(name).is_some() {
            encoder.set_property_from_str(name, value);
        }
    };
    set("preset", "low-latency-hp");
    set("rc-mode", "cbr");
    set("bitrate", &kbps.to_string());
    set("gop-size", &KEY_INTERVAL.to_string());
    set("bframes", "0");
    set("zerolatency", "true");
    set("rc-lookahead", "0");
    Ok(vec![upload, scale, convert, filter, encoder])
}

impl Drop for Encoder {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// `orbit-remote-server encode [OUTPUT [WxH]]`: captures and encodes for
/// 10 s (fitted to a client's screen of `fit`), says how long a frame
/// takes from the copy to H.264 and how big the stream is, and writes it
/// to `$XDG_RUNTIME_DIR/orbit-remote-test.h264`.
pub fn measure(output: Option<&str>, fit: Option<(u32, u32)>) -> anyhow::Result<()> {
    let mut capture = Capture::open(output, true)?;
    let encoder = Encoder::new(&capture.planes, Tail::Sink, fit)?;
    let out_path = std::env::temp_dir().join("orbit-remote-test.h264");
    let path = std::env::var_os("XDG_RUNTIME_DIR").map_or(out_path, |d| std::path::PathBuf::from(d).join("orbit-remote-test.h264"));
    let mut file = std::fs::File::create(&path)?;
    let start = Instant::now();
    let (mut frames, mut bytes, mut latencies) = (0u32, 0usize, Vec::new());
    while start.elapsed() < Duration::from_secs(10) {
        let frame = capture.next()?;
        let pushed = Instant::now();
        encoder.push(&capture.planes, &frame)?;
        // The first takes long: NVENC makes its CUDA context and encoder.
        let wait = if frames == 0 { Duration::from_secs(5) } else { Duration::from_millis(500) };
        let sample = encoder.pull(wait).with_context(|| format!("no encoded frame within {wait:?}"))?;
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

#[cfg(test)]
mod tests {
    use super::bitrate_kbps;

    #[test]
    fn pictures_fit_the_client() {
        use super::fitted;
        assert_eq!(fitted((3840, 2160), None), (3840, 2160));
        assert_eq!(fitted((3840, 2160), Some((800, 600))), (800, 450));
        assert_eq!(fitted((3840, 2160), Some((1920, 1080))), (1920, 1080));
        assert_eq!(fitted((1920, 1080), Some((3840, 2160))), (1920, 1080));
        assert_eq!(fitted((1366, 768), Some((1280, 1024))), (1280, 718));
    }

    #[test]
    fn the_bitrate_follows_the_size() {
        assert_eq!(bitrate_kbps(1920, 1080), 20_000);
        assert_eq!(bitrate_kbps(3840, 2160), 80_000);
        assert_eq!(bitrate_kbps(2560, 1440), 35_555);
        assert_eq!(bitrate_kbps(1280, 720), 8_888);
        assert_eq!(bitrate_kbps(640, 480), 8_000);
        assert_eq!(bitrate_kbps(7680, 4320), 80_000);
    }
}
