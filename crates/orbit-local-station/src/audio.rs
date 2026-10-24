//! The session's sound, played on the station's default output (through
//! PipeWire's PulseAudio server):
//!
//!   webrtcbin → rtpopusdepay → opusdec → audioconvert → audioresample → pulsesink
//!
//! Played as it comes, with a small buffer: late is worse than a click.

use anyhow::Context;
use gst::prelude::*;

/// The playing chain behind `webrtcbin`'s new sound `pad`.
pub fn play(pipeline: &gst::Pipeline, pad: &gst::Pad) -> anyhow::Result<()> {
    let depay = gst::ElementFactory::make("rtpopusdepay").build()?;
    let decoder = gst::ElementFactory::make("opusdec").build().context("no opusdec (gstreamer1.0-plugins-base)")?;
    let convert = gst::ElementFactory::make("audioconvert").build()?;
    let resample = gst::ElementFactory::make("audioresample").build()?;
    let sink = gst::ElementFactory::make("pulsesink")
        .property("buffer-time", 40_000i64)
        .property("latency-time", 10_000i64)
        .property("sync", false)
        .build()
        .context("no pulsesink (gstreamer1.0-plugins-good)")?;
    let elements = [&depay, &decoder, &convert, &resample, &sink];
    pipeline.add_many(elements)?;
    gst::Element::link_many(elements)?;
    for element in elements {
        element.sync_state_with_parent()?;
    }
    pad.link(&depay.static_pad("sink").context("the depayloader's pad")?)?;
    log::info!("station: the sound is coming");
    Ok(())
}
