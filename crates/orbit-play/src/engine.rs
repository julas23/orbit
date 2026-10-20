//! The playback engine: GStreamer's player, its picture drawn into GTK by
//! `gtk4paintablesink` — through OpenGL when GTK has a GL context, so the
//! frames decoded on the GPU (VA-API) stay there. FFmpeg's decoders
//! (`gstreamer1.0-libav`) give it practically every audio and video format.

use gst::prelude::*;
use gtk::gdk;

pub struct Engine {
    pub play: gst_play::Play,
    pub signals: gst_play::PlaySignalAdapter,
    /// Where the video appears.
    pub paintable: gdk::Paintable,
}

impl Engine {
    pub fn new() -> anyhow::Result<Self> {
        gst::init()?;
        gstgtk4::plugin_register_static()?;
        let gtk_sink = gst::ElementFactory::make("gtk4paintablesink").build()?;
        let paintable = gtk_sink.property::<gdk::Paintable>("paintable");
        let has_gl = paintable
            .property::<Option<gdk::GLContext>>("gl-context")
            .is_some();
        let sink: gst::Element = if has_gl {
            gst::ElementFactory::make("glsinkbin")
                .property("sink", &gtk_sink)
                .build()?
        } else {
            // Without GL, frames are converted to what the sink takes.
            let bin = gst::Bin::new();
            let convert = gst::ElementFactory::make("videoconvert").build()?;
            bin.add_many([&convert, &gtk_sink])?;
            convert.link(&gtk_sink)?;
            let pad = convert.static_pad("sink").expect("videoconvert has a sink");
            bin.add_pad(&gst::GhostPad::with_target(&pad)?)?;
            bin.upcast()
        };
        let renderer = gst_play::PlayVideoOverlayVideoRenderer::with_sink(&sink);
        let play = gst_play::Play::new(Some(renderer));
        // Exact seeks, so a frame back lands on the frame before.
        let mut config = play.config();
        config.set_seek_accurate(true);
        // The position, often enough for a smooth time bar.
        config.set_position_update_interval(250);
        if let Err(err) = play.set_config(config) {
            log::warn!("player config: {err}");
        }
        let signals = gst_play::PlaySignalAdapter::new(&play);
        Ok(Self {
            play,
            signals,
            paintable,
        })
    }

    /// Advances one frame (paused).
    pub fn frame_forward(&self) {
        let step = gst::event::Step::new(gst::format::Buffers::ONE, 1.0, true, false);
        self.play.pipeline().send_event(step);
    }

    /// Goes back one frame (paused): a seek to a frame's length before.
    pub fn frame_back(&self) {
        let frame = self
            .play
            .current_video_track()
            .map(|v| v.framerate())
            .filter(|f| f.numer() > 0)
            .map(|f| {
                gst::ClockTime::from_nseconds(
                    1_000_000_000u64 * f.denom() as u64 / f.numer() as u64,
                )
            })
            .unwrap_or(gst::ClockTime::from_mseconds(40));
        if let Some(position) = self.play.position() {
            self.play.seek(position.saturating_sub(frame));
        }
    }
}
