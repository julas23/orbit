//! Screen capture for clients (`wlr-screencopy-unstable-v1`): Orbit-Blink,
//! grim and the like ask for a monitor, or a rectangle of one, and get it
//! copied into a buffer of theirs — shared memory or a dmabuf.
//!
//! Each copy is drawn off screen from the same elements the monitor shows
//! (`render::output_elements`), the pointer only when asked for. `copy` is
//! answered at once. `copy_with_damage`, what a screencast uses
//! (orbit-remote-server, the portal), waits until something has changed on
//! that monitor since the client's last copy of it, and says what: each
//! client has a damage tracker per monitor and region, and the queue is
//! looked at again after every frame drawn for that monitor. Locked, every
//! copy fails, so the lock screen never shows the session.
//!
//! The protocol's handling follows niri `src/protocols/screencopy.rs` and
//! `render_helpers/mod.rs` (`render_to_shm`, `render_to_dmabuf`) @ aee25c9
//! (GPL-3.0-or-later, Copyright (c) Ivan Molodetskikh), without the queue
//! of frames waiting for damage.

use std::{
    ptr,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use anyhow::{Context, bail, ensure};
use smithay::{
    backend::{
        allocator::{Buffer as _, Fourcc, dmabuf::Dmabuf},
        session::Session as _,
        renderer::{
            Bind, ExportMem, Offscreen,
            damage::OutputDamageTracker,
            element::utils::{Relocate, RelocateRenderElement},
            gles::{GlesRenderer, GlesTexture},
        },
    },
    output::Output,
    reexports::{
        wayland_protocols_wlr::screencopy::v1::server::{
            zwlr_screencopy_frame_v1::{self, Flags, ZwlrScreencopyFrameV1},
            zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
        },
        wayland_server::{
            Client, DataInit, DisplayHandle, New, Resource,
            backend::ClientId,
            protocol::{wl_buffer::WlBuffer, wl_shm::Format},
        },
    },
    utils::{Clock, Monotonic, Physical, Point, Rectangle, Size, Transform},
    wayland::{Dispatch2, GlobalDispatch2, dmabuf, shm},
};

use crate::{
    render::{background, output_elements},
    state::{Backend, Orbit},
};

const VERSION: u32 = 3;

/// The `zwlr_screencopy_manager_v1` global.
pub struct ScreencopyGlobal;

/// A bound manager.
pub struct ManagerData;

/// What a frame copies.
#[derive(Clone)]
pub struct FrameInfo {
    output: Output,
    /// Untransformed, in the output's buffer pixels.
    size: Size<i32, Physical>,
    /// The rectangle's corner on the output, transformed.
    region_loc: Point<i32, Physical>,
    cursor: bool,
}

/// The copies with damage waiting for a change, and the trackers that tell
/// one: per client, monitor, region and pointer, what that client was
/// last given.
#[derive(Default)]
pub struct Queue {
    waiting: Vec<Waiting>,
    trackers: Vec<(Key, OutputDamageTracker)>,
}

struct Waiting {
    frame: ZwlrScreencopyFrameV1,
    info: FrameInfo,
    target: Target,
    key: Key,
}

#[derive(Clone, PartialEq)]
struct Key {
    client: ClientId,
    output: String,
    region: (Point<i32, Physical>, Size<i32, Physical>),
    cursor: bool,
}

pub enum FrameData {
    Failed,
    Pending { info: FrameInfo, copied: AtomicBool },
}

/// The buffer a frame is copied into.
#[derive(Clone)]
enum Target {
    Shm(WlBuffer),
    Dmabuf(Dmabuf),
}

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Orbit, ZwlrScreencopyManagerV1, ScreencopyGlobal>(VERSION, ScreencopyGlobal);
}

fn stride(size: Size<i32, Physical>) -> i32 {
    size.w * 4
}

impl GlobalDispatch2<ZwlrScreencopyManagerV1, Orbit> for ScreencopyGlobal {
    fn bind(&self, _state: &mut Orbit, _dh: &DisplayHandle, _client: &Client, manager: New<ZwlrScreencopyManagerV1>, data_init: &mut DataInit<'_, Orbit>) {
        data_init.init(manager, ManagerData);
    }
}

impl Dispatch2<ZwlrScreencopyManagerV1, Orbit> for ManagerData {
    fn request(&self, _state: &mut Orbit, _client: &Client, _manager: &ZwlrScreencopyManagerV1, request: zwlr_screencopy_manager_v1::Request, _dh: &DisplayHandle, data_init: &mut DataInit<'_, Orbit>) {
        use zwlr_screencopy_manager_v1::Request;
        let (frame, cursor, output, region) = match request {
            Request::CaptureOutput { frame, overlay_cursor, output } => (frame, overlay_cursor, output, None),
            Request::CaptureOutputRegion { frame, overlay_cursor, x, y, width, height, output } => {
                (frame, overlay_cursor, output, Some(Rectangle::<i32, smithay::utils::Logical>::new((x, y).into(), (width, height).into())))
            }
            _ => return,
        };
        let failed = |data_init: &mut DataInit<'_, Orbit>, frame| data_init.init(frame, FrameData::Failed).failed();
        let Some(output) = Output::from_resource(&output) else { return failed(data_init, frame) };
        let Some(mode) = output.current_mode() else { return failed(data_init, frame) };
        let (size, region_loc) = match region {
            None => (mode.size, Point::from((0, 0))),
            Some(rect) => {
                if rect.size.w <= 0 || rect.size.h <= 0 {
                    return failed(data_init, frame);
                }
                // Clamped to the output, in transformed pixels, then back
                // to the buffer's.
                let transform = output.current_transform();
                let whole = transform.transform_size(mode.size);
                let physical = rect.to_physical_precise_round(output.current_scale().fractional_scale());
                let Some(clamped) = physical.intersection(Rectangle::from_size(whole)) else {
                    return failed(data_init, frame);
                };
                (transform.invert().transform_rect_in(clamped, &whole).size, clamped.loc)
            }
        };
        let frame = data_init.init(
            frame,
            FrameData::Pending { info: FrameInfo { output, size, region_loc, cursor: cursor != 0 }, copied: AtomicBool::new(false) },
        );
        frame.buffer(Format::Xrgb8888, size.w as u32, size.h as u32, stride(size) as u32);
        if frame.version() >= 3 {
            frame.linux_dmabuf(Fourcc::Xrgb8888 as u32, size.w as u32, size.h as u32);
            frame.buffer_done();
        }
    }
}

impl Dispatch2<ZwlrScreencopyFrameV1, Orbit> for FrameData {
    fn request(&self, state: &mut Orbit, _client: &Client, frame: &ZwlrScreencopyFrameV1, request: zwlr_screencopy_frame_v1::Request, _dh: &DisplayHandle, _data_init: &mut DataInit<'_, Orbit>) {
        use zwlr_screencopy_frame_v1::Request;
        let FrameData::Pending { info, copied } = self else { return };
        let (buffer, with_damage) = match request {
            Request::Copy { buffer } => (buffer, false),
            Request::CopyWithDamage { buffer } => (buffer, true),
            _ => return,
        };
        if copied.swap(true, Ordering::SeqCst) {
            frame.post_error(zwlr_screencopy_frame_v1::Error::AlreadyUsed, "copy was already requested");
            return;
        }
        let size = info.size;
        let target = if let Ok(dmabuf) = dmabuf::get_dmabuf(&buffer) {
            if dmabuf.format().code != Fourcc::Xrgb8888 || dmabuf.width() != size.w as u32 || dmabuf.height() != size.h as u32 {
                frame.post_error(zwlr_screencopy_frame_v1::Error::InvalidBuffer, "invalid dmabuf parameters");
                return;
            }
            Target::Dmabuf(dmabuf.clone())
        } else if shm::with_buffer_contents(&buffer, |_, _, data| {
            data.format == Format::Xrgb8888 && data.width == size.w && data.height == size.h && data.stride == stride(size)
        })
        .unwrap_or(false)
        {
            Target::Shm(buffer)
        } else {
            frame.post_error(zwlr_screencopy_frame_v1::Error::InvalidBuffer, "invalid buffer");
            return;
        };
        if state.locked() {
            frame.failed();
            return;
        }
        if !with_damage {
            match state.capture(info, &target, None) {
                Ok(_) => ready(frame, None),
                Err(err) => {
                    tracing::warn!("screencopy: {err:?}");
                    frame.failed();
                }
            }
            return;
        }
        let Some(client) = frame.client() else { return };
        let key = Key {
            client: client.id(),
            output: info.output.name(),
            region: (info.region_loc, info.size),
            cursor: info.cursor,
        };
        state.screencopy.waiting.push(Waiting { frame: frame.clone(), info: info.clone(), target, key });
        // Answered now if something changed since the last one.
        state.screencopy_after_frame(&info.output);
    }

    fn destroyed(&self, state: &mut Orbit, _client: ClientId, frame: &ZwlrScreencopyFrameV1) {
        state.screencopy.waiting.retain(|w| w.frame != *frame);
    }
}

/// Tells the client its copy is there, with what changed when it asked.
fn ready(frame: &ZwlrScreencopyFrameV1, damage: Option<&[Rectangle<i32, Physical>]>) {
    frame.flags(Flags::empty());
    for rect in damage.into_iter().flatten() {
        frame.damage(rect.loc.x.max(0) as u32, rect.loc.y.max(0) as u32, rect.size.w as u32, rect.size.h as u32);
    }
    let now: Duration = Clock::<Monotonic>::new().now().into();
    frame.ready((now.as_secs() >> 32) as u32, now.as_secs() as u32, now.subsec_nanos());
}

impl Orbit {
    /// Answers the copies with damage waiting on `output` whose picture has
    /// changed since their client's last one; called after each frame drawn
    /// for it, and when one is asked for. Locked, they fail.
    pub fn screencopy_after_frame(&mut self, output: &Output) {
        let name = output.name();
        if !self.screencopy.waiting.iter().any(|w| w.key.output == name) {
            return;
        }
        let waiting = std::mem::take(&mut self.screencopy.waiting);
        let mut kept = Vec::new();
        for job in waiting {
            if job.key.output != name || !job.frame.is_alive() {
                if job.frame.is_alive() {
                    kept.push(job);
                }
                continue;
            }
            if self.locked() {
                job.frame.failed();
                continue;
            }
            let mut tracker = match self.screencopy.trackers.iter().position(|(k, _)| *k == job.key) {
                Some(i) => self.screencopy.trackers.swap_remove(i).1,
                // First copy for this client: all of it is new.
                None => OutputDamageTracker::new(job.info.size, output.current_scale().fractional_scale(), output.current_transform()),
            };
            let captured = self.capture(&job.info, &job.target, Some(&mut tracker));
            self.screencopy.trackers.push((job.key.clone(), tracker));
            match captured {
                Ok(Some(damage)) => ready(&job.frame, Some(&damage)),
                Ok(None) => kept.push(job),
                Err(err) => {
                    tracing::warn!("screencopy: {err:?}");
                    job.frame.failed();
                }
            }
        }
        // Clients gone take their trackers with them.
        let handle = self.display_handle.backend_handle();
        self.screencopy.trackers.retain(|(k, _)| handle.get_client_data(k.client.clone()).is_ok());
        kept.append(&mut self.screencopy.waiting);
        self.screencopy.waiting = kept;
    }

    /// Draws what `info` asks for into `target`, off screen. With a
    /// `tracker`, only when something changed since its last frame, and
    /// what did; without, always (`Some` then holds nothing).
    fn capture(&mut self, info: &FrameInfo, target: &Target, tracker: Option<&mut OutputDamageTracker>) -> anyhow::Result<Option<Vec<Rectangle<i32, Physical>>>> {
        // Taken out while drawing, as for a frame on screen.
        let mut backend = std::mem::replace(&mut self.backend, Backend::None);
        let result = match &mut backend {
            Backend::Drm(drm) if !drm.session.is_active() => Err(anyhow::anyhow!("on another console")),
            Backend::Drm(drm) => self.capture_with(&mut drm.renderer, info, target, tracker),
            Backend::Winit(winit) => self.capture_with(winit.backend.renderer(), info, target, tracker),
            Backend::None => Err(anyhow::anyhow!("no backend")),
        };
        self.backend = backend;
        result
    }

    fn capture_with(
        &mut self,
        renderer: &mut GlesRenderer,
        info: &FrameInfo,
        target: &Target,
        tracker: Option<&mut OutputDamageTracker>,
    ) -> anyhow::Result<Option<Vec<Rectangle<i32, Physical>>>> {
        let output = &info.output;
        let offset = info.region_loc.upscale(-1);
        let elements: Vec<_> = output_elements(self, output, renderer, info.cursor)
            .into_iter()
            .map(|e| RelocateRenderElement::from_element(e, offset, Relocate::Relative))
            .collect();
        // The change since this client's last copy, told by its tracker
        // (a new tracker: all of it). The client's buffers come and go, so
        // the picture itself is always drawn whole.
        let damage = match tracker {
            None => Vec::new(),
            Some(tracker) => match tracker.damage_output(1, &elements).context("damage")?.0 {
                Some(damage) if !damage.is_empty() => damage.clone(),
                _ => return Ok(None),
            },
        };
        let clear = background(self);
        let mut tracker = OutputDamageTracker::new(info.size, output.current_scale().fractional_scale(), output.current_transform());
        match target {
            Target::Dmabuf(dmabuf) => {
                let mut dmabuf = dmabuf.clone();
                let mut frame = renderer.bind(&mut dmabuf).context("binding the dmabuf")?;
                tracker.render_output(renderer, &mut frame, 0, &elements, clear).context("drawing into the dmabuf")?;
                Ok(Some(damage))
            }
            Target::Shm(buffer) => {
                let buffer_size = info.size.to_logical(1).to_buffer(1, Transform::Normal);
                let mut texture: GlesTexture = renderer.create_buffer(Fourcc::Xrgb8888, buffer_size).context("creating the texture")?;
                let mut frame = renderer.bind(&mut texture).context("binding the texture")?;
                tracker.render_output(renderer, &mut frame, 0, &elements, clear).context("drawing")?;
                let mapping = renderer.copy_framebuffer(&frame, Rectangle::from_size(buffer_size), Fourcc::Xrgb8888).context("copying")?;
                drop(frame);
                let bytes = renderer.map_texture(&mapping).context("mapping")?;
                write_shm(buffer, info.size, bytes)?;
                Ok(Some(damage))
            }
        }
    }
}

/// Copies `bytes`, rows of `size.w` pixels, into the client's buffer.
fn write_shm(buffer: &WlBuffer, size: Size<i32, Physical>, bytes: &[u8]) -> anyhow::Result<()> {
    shm::with_buffer_contents_mut(buffer, |pool, pool_len, data| {
        let row = size.w as usize * 4;
        let height = size.h as usize;
        let offset = usize::try_from(data.offset).context("negative offset")?;
        let stride = usize::try_from(data.stride).context("negative stride")?;
        // Checked by wl_shm already, and a pool only grows; again anyway.
        let end = stride.checked_mul(height.saturating_sub(1)).and_then(|n| n.checked_add(row)).and_then(|n| n.checked_add(offset));
        ensure!(stride >= row && end.is_some_and(|end| end <= pool_len), "the buffer does not fit in its pool");
        if bytes.len() < row * height {
            bail!("short copy");
        }
        // SAFETY: the destination was just checked to lie inside the pool,
        // and the source holds `row * height` bytes.
        unsafe {
            let dst = pool.add(offset);
            for y in 0..height {
                ptr::copy_nonoverlapping(bytes.as_ptr().add(y * row), dst.add(y * stride), row);
            }
        }
        Ok(())
    })
    .context("not a shared-memory buffer")?
}
