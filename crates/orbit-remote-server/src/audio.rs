//! The session's sound, to the client beside the video: what the default
//! output plays (its monitor, through PipeWire's PulseAudio server), in
//! Opus over the same WebRTC connection.
//!
//!   pulsesrc (@DEFAULT_MONITOR@) → audioconvert → audioresample → opusenc → rtpopuspay → webrtcbin
//!
//! Small buffers and 10 ms frames, for a short delay. The source does not
//! give the pipeline its clock: the video's timestamps are the system's.

use anyhow::Context;
use gst::prelude::*;

/// The sound's RTP payload type; the video's is 96.
const PAYLOAD: i32 = 111;

/// Adds the sound to `pipeline`, into its `webrtc`. Before the pipeline
/// plays: the offer then has both.
pub fn attach(pipeline: &gst::Pipeline, webrtc: &gst::Element) -> anyhow::Result<()> {
    let source = gst::ElementFactory::make("pulsesrc")
        .property("device", "@DEFAULT_MONITOR@")
        .property("provide-clock", false)
        .property("buffer-time", 40_000i64)
        .property("latency-time", 10_000i64)
        .property("do-timestamp", true)
        .build()
        .context("no pulsesrc (gstreamer1.0-plugins-good)")?;
    let convert = gst::ElementFactory::make("audioconvert").build()?;
    let resample = gst::ElementFactory::make("audioresample").build()?;
    let raw = gst::Caps::builder("audio/x-raw").field("rate", 48_000i32).field("channels", 2i32).build();
    let raw = gst::ElementFactory::make("capsfilter").property("caps", &raw).build()?;
    let encoder = gst::ElementFactory::make("opusenc")
        .property("bitrate", 128_000i32)
        .property_from_str("frame-size", "10")
        .property_from_str("audio-type", "restricted-lowdelay")
        .build()
        .context("no opusenc (gstreamer1.0-plugins-base)")?;
    let pay = gst::ElementFactory::make("rtpopuspay").build()?;
    let rtp = gst::Caps::builder("application/x-rtp")
        .field("media", "audio")
        .field("encoding-name", "OPUS")
        .field("payload", PAYLOAD)
        .build();
    let filter = gst::ElementFactory::make("capsfilter").property("caps", &rtp).build()?;
    let elements = [&source, &convert, &resample, &raw, &encoder, &pay, &filter];
    pipeline.add_many(elements)?;
    gst::Element::link_many(elements)?;
    filter.link(webrtc)?;
    log::info!("remote: the sound goes along (the default output's monitor, Opus)");
    Ok(())
}
