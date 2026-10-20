//! Decoding, with glycin: each image on a thread of its own, its format's
//! loader in a sandbox (bubblewrap) as glycin runs it, so a malformed file
//! can crash nothing but the loader.
//!
//! The thread sends what it decodes over a channel: first what is known of
//! the file, then the frames — one for a still image, one after another for
//! an animation, at the pace they are taken. Dropping the receiver ends it.

use std::{path::PathBuf, time::Duration};

use async_channel::Sender;
use gtk::{gdk, glib, prelude::*};

/// Up to this much of an animation is kept, to replay it without decoding
/// again; a longer one is decoded over and over.
pub const ANIMATION_CACHE: usize = 256 << 20;

/// What is known of the file once it is open.
pub struct Info {
    /// "JPEG", "WebP"…
    pub format: Option<String>,
    pub mime: String,
    /// The raw EXIF block, if any.
    pub exif: Option<Vec<u8>>,
}

/// A decoded frame, as plain memory: GDK textures stay on the GTK thread.
pub struct RawFrame {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: gdk::MemoryFormat,
    pub data: Vec<u8>,
    /// How long to show it, for a frame of an animation.
    pub delay: Option<Duration>,
}

impl RawFrame {
    pub fn texture(self) -> gdk::Texture {
        gdk::MemoryTexture::new(
            self.width as i32,
            self.height as i32,
            self.format,
            &glib::Bytes::from_owned(self.data),
            self.stride,
        )
        .upcast()
    }
}

pub enum Event {
    Opened(Info),
    Frame(RawFrame),
    /// The animation came back to its first frame, and every frame so far
    /// fit in [`ANIMATION_CACHE`]: replay those.
    Looped,
    Failed(Failure),
}

pub enum Failure {
    /// No loader for this kind of file.
    UnknownFormat,
    Other(String),
}

/// Starts decoding `path`.
pub fn load(path: PathBuf) -> async_channel::Receiver<Event> {
    let (tx, rx) = async_channel::bounded(2);
    let spawned = std::thread::Builder::new()
        .name("orbit-view-decode".into())
        .spawn(move || async_io::block_on(run(path, tx)));
    if let Err(err) = spawned {
        log::error!("starting the decoder: {err}");
    }
    rx
}

async fn run(path: PathBuf, tx: Sender<Event>) {
    let image = match glycin::Loader::new(gio20::File::for_path(&path)).load().await {
        Ok(image) => image,
        Err(err) => {
            let _ = tx.send(Event::Failed(failure(&err))).await;
            return;
        }
    };
    let info = Info {
        format: image.format_name(),
        mime: image.mime_type().as_str().to_owned(),
        exif: image
            .info()
            .details
            .exif
            .as_ref()
            .and_then(|data| data.get_full().ok()),
    };
    if tx.send(Event::Opened(info)).await.is_err() {
        return;
    }
    let mut kept = 0usize;
    loop {
        let frame = match image.next_frame().await {
            Ok(frame) => frame,
            Err(err) => {
                let _ = tx.send(Event::Failed(failure(&err))).await;
                return;
            }
        };
        let delay = frame.delay();
        if delay.is_some()
            && frame.details().n_frame == Some(0)
            && kept > 0
            && kept <= ANIMATION_CACHE
        {
            let _ = tx.send(Event::Looped).await;
            return;
        }
        let Some(format) = memory_format(frame.memory_format()) else {
            let _ = tx
                .send(Event::Failed(Failure::Other(format!(
                    "pixel format {:?}",
                    frame.memory_format()
                ))))
                .await;
            return;
        };
        let raw = RawFrame {
            width: frame.width(),
            height: frame.height(),
            stride: frame.stride() as usize,
            format,
            data: frame.buf_slice().to_vec(),
            delay,
        };
        kept += raw.data.len();
        if tx.send(Event::Frame(raw)).await.is_err() || delay.is_none() {
            return;
        }
    }
}

fn failure(err: &glycin::ErrorCtx) -> Failure {
    match err.error() {
        glycin::Error::UnknownImageFormat(..) | glycin::Error::UnknownContentType(..) => {
            Failure::UnknownFormat
        }
        // The first line: some errors go on with a dump of the config.
        other => Failure::Other(other.to_string().lines().next().unwrap_or("").to_owned()),
    }
}

/// glycin's pixel formats are GDK's, by another name.
fn memory_format(format: glycin::MemoryFormat) -> Option<gdk::MemoryFormat> {
    use gdk::MemoryFormat as G;
    use glycin::MemoryFormat as M;
    Some(match format {
        M::B8g8r8a8Premultiplied => G::B8g8r8a8Premultiplied,
        M::A8r8g8b8Premultiplied => G::A8r8g8b8Premultiplied,
        M::R8g8b8a8Premultiplied => G::R8g8b8a8Premultiplied,
        M::B8g8r8a8 => G::B8g8r8a8,
        M::A8r8g8b8 => G::A8r8g8b8,
        M::R8g8b8a8 => G::R8g8b8a8,
        M::A8b8g8r8 => G::A8b8g8r8,
        M::R8g8b8 => G::R8g8b8,
        M::B8g8r8 => G::B8g8r8,
        M::R16g16b16 => G::R16g16b16,
        M::R16g16b16a16Premultiplied => G::R16g16b16a16Premultiplied,
        M::R16g16b16a16 => G::R16g16b16a16,
        M::R16g16b16Float => G::R16g16b16Float,
        M::R16g16b16a16Float => G::R16g16b16a16Float,
        M::R32g32b32Float => G::R32g32b32Float,
        M::R32g32b32a32FloatPremultiplied => G::R32g32b32a32FloatPremultiplied,
        M::R32g32b32a32Float => G::R32g32b32a32Float,
        M::G8a8Premultiplied => G::G8a8Premultiplied,
        M::G8a8 => G::G8a8,
        M::G8 => G::G8,
        M::G16a16Premultiplied => G::G16a16Premultiplied,
        M::G16a16 => G::G16a16,
        M::G16 => G::G16,
        #[allow(unreachable_patterns)]
        _ => return None,
    })
}

/// The MIME types glycin has loaders for.
pub fn supported_types() -> std::collections::HashSet<String> {
    async_io::block_on(glycin::Loader::supported_mime_types())
        .into_iter()
        .map(|m| m.as_str().to_owned())
        .collect()
}
