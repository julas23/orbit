//! The screen and the keyboard, owned straight from a text console: no
//! compositor runs under the station. The session (libseat, through
//! logind) hands over the GPU and the input devices and takes them back on
//! a VT switch; libinput reads the keyboard; one monitor shows the
//! station, the first connected one or the one named in station.yaml.
//! The session's video fills it; the station's own screens show over it
//! when there is no video.
//!
//! The same pieces of Smithay as orbit-core's `udev.rs`, without its
//! Wayland side: frames are drawn when something changed and the last one
//! has had its vblank.

use std::path::PathBuf;

use anyhow::{Context, anyhow};
use smithay::{
    backend::{
        allocator::{
            Fourcc,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, NodeType,
            compositor::FrameFlags,
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
        },
        egl::{EGLContext, EGLDisplay},
        input::{
            AbsolutePositionEvent, Axis, AxisSource, ButtonState, InputEvent, KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
            PointerMotionEvent,
        },
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            ImportDma, Renderer,
            element::{
                Id, Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                texture::TextureRenderElement,
            },
            gles::{GlesRenderer, GlesTexture},
        },
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{all_gpus, primary_gpu},
    },
    output::OutputModeSource,
    reexports::{
        calloop::EventLoop,
        drm::control::{Device, ModeTypeFlags, connector, crtc},
        input::Libinput,
        rustix::fs::OFlags,
    },
    render_elements,
    utils::{DeviceFd, Rectangle, Transform},
};

use crate::{
    Station,
    video::{Frame, Picture},
};

/// 8-bit colour: what every monitor takes.
const FORMATS: &[Fourcc] = &[Fourcc::Abgr8888, Fourcc::Argb8888];

type Output = DrmOutput<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

render_elements! {
    Element<=GlesRenderer>;
    /// The station's screens, and the video decoded in software.
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Video=TextureRenderElement<GlesTexture>,
}

pub struct Screen {
    pub session: LibSeatSession,
    renderer: GlesRenderer,
    manager: DrmOutputManager<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>,
    output: Output,
    /// The monitor's size in pixels.
    pub size: (u32, u32),
    /// A frame is queued and its vblank has not come.
    waiting: bool,
    /// Something changed since the last frame.
    dirty: bool,
    /// The video's picture when it comes in memory: one buffer, so one
    /// texture, written over for each picture.
    memory: Option<(Fourcc, MemoryRenderBuffer)>,
}

/// A key pressed or let go, by its evdev code.
pub struct Key {
    pub code: u32,
    pub down: bool,
}

/// The mouse.
pub enum Pointer {
    /// Moved by this much, in pixels.
    By(f64, f64),
    /// Moved to here, as fractions of the screen.
    To(f64, f64),
    /// A button (evdev's code) pressed or let go.
    Button { code: u32, down: bool },
    /// The wheel, in the browser's pixels (about 100 a notch).
    Wheel { dx: f64, dy: f64 },
}

/// A touchpad's scroll (Wayland's units, about 15 a notch) in the
/// browser's pixels, which the server takes.
const SCROLL_TO_WHEEL: f64 = 100.0 / 15.0;

/// The GPU to draw with: `$ORBIT_DRM_DEVICE` when set, otherwise the one the
/// firmware booted with when it can render, otherwise the first that can
/// (as orbit-core's `udev::gpu`).
fn gpu(seat: &str) -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("ORBIT_DRM_DEVICE") {
        return Ok(path.into());
    }
    let renders = |path: &std::path::Path| {
        let Some(card) = path.file_name() else { return false };
        let dir = std::path::Path::new("/sys/class/drm").join(card).join("device/drm");
        std::fs::read_dir(dir).is_ok_and(|entries| entries.flatten().any(|e| e.file_name().to_string_lossy().starts_with("renderD")))
    };
    let primary = primary_gpu(seat)?;
    let all = all_gpus(seat).unwrap_or_default();
    primary
        .iter()
        .chain(&all)
        .find(|path| renders(path))
        .or(primary.as_ref())
        .or(all.first())
        .cloned()
        .ok_or_else(|| anyhow!("no GPU"))
}

impl Screen {
    /// Takes the seat, the GPU and one monitor (`wanted`, as `HDMI-A-1`, or
    /// the first connected), and puts libinput's keys into `event_loop`.
    pub fn open(event_loop: &mut EventLoop<'static, Station>, wanted: Option<&str>) -> anyhow::Result<Self> {
        let (mut session, notifier) = LibSeatSession::new().map_err(|e| anyhow!("no seat session (run it from a text console): {e}"))?;
        let seat = session.seat();

        let path = gpu(&seat)?;
        let node = DrmNode::from_path(&path)?;
        log::info!("GPU {} ({node})", path.display());
        let fd = session
            .open(&path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)
            .map_err(|e| anyhow!("opening {}: {e}", path.display()))?;
        let fd = DrmDeviceFd::new(DeviceFd::from(fd));
        let (drm, drm_notifier) = DrmDevice::new(fd.clone(), true)?;
        let gbm = GbmDevice::new(fd)?;

        let display = unsafe { EGLDisplay::new(gbm.clone())? };
        let context = EGLContext::new(&display)?;
        let mut renderer = unsafe { GlesRenderer::new(context)? };
        let render_formats = renderer.egl_context().dmabuf_render_formats().clone();
        let render_node = node.node_with_type(NodeType::Render).and_then(Result::ok).unwrap_or(node);
        if let Some(name) = render_node.dev_path().and_then(|path| path.file_name().map(|n| n.to_string_lossy().into_owned())) {
            crate::video::set_render_node(name);
        }

        let (connector, crtc, mode) = monitor(&drm, wanted)?;
        let mut manager = DrmOutputManager::new(
            drm,
            GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT),
            GbmFramebufferExporter::new(gbm.clone(), Some(render_node).into()),
            Some(gbm),
            FORMATS.iter().copied(),
            render_formats,
        );
        let (w, h) = mode.size();
        let source = OutputModeSource::Static { size: (i32::from(w), i32::from(h)).into(), scale: 1.0.into(), transform: Transform::Normal };
        let output = manager
            .lock()
            .initialize_output::<_, Element>(crtc, mode, &[connector], source, None, &mut renderer, &DrmOutputRenderElements::default())
            .map_err(|e| anyhow!("setting up the monitor: {e}"))?;

        // The keyboard.
        let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
        libinput.udev_assign_seat(&seat).map_err(|()| anyhow!("libinput: no seat {seat}"))?;
        event_loop
            .handle()
            .insert_source(LibinputInputBackend::new(libinput.clone()), |event, _, station| match event {
                InputEvent::Keyboard { event } => {
                    // xkb's numbers are evdev's plus 8.
                    let code = event.key_code().raw().saturating_sub(8);
                    station.key(Key { code, down: event.state() == KeyState::Pressed });
                }
                InputEvent::PointerMotion { event } => station.pointer(Pointer::By(event.delta_x(), event.delta_y())),
                InputEvent::PointerMotionAbsolute { event } => station.pointer(Pointer::To(event.x_transformed(1), event.y_transformed(1))),
                InputEvent::PointerButton { event } => {
                    station.pointer(Pointer::Button { code: event.button_code(), down: event.state() == ButtonState::Pressed });
                }
                InputEvent::PointerAxis { event } => {
                    let amount = |axis| match event.source() {
                        AxisSource::Wheel | AxisSource::WheelTilt => event.amount_v120(axis).map_or(0.0, |v| v / 120.0 * 100.0),
                        _ => event.amount(axis).unwrap_or(0.0) * SCROLL_TO_WHEEL,
                    };
                    station.pointer(Pointer::Wheel { dx: amount(Axis::Horizontal), dy: amount(Axis::Vertical) });
                }
                _ => {}
            })
            .map_err(|e| anyhow!("libinput in the event loop: {e}"))?;

        // Leaving for another VT, and coming back.
        event_loop
            .handle()
            .insert_source(notifier, move |event, _, station| match event {
                SessionEvent::PauseSession => {
                    log::info!("session paused");
                    station.paused();
                    libinput.suspend();
                    station.screen.manager.pause();
                }
                SessionEvent::ActivateSession => {
                    log::info!("session back");
                    if let Err(err) = libinput.resume() {
                        log::error!("libinput resume: {err:?}");
                    }
                    // Everything off first: the console may have left the
                    // monitor on another crtc (orbit-core's lesson).
                    if let Err(err) = station.screen.manager.lock().activate(true) {
                        log::error!("DRM activate: {err}");
                    }
                    station.screen.waiting = false;
                    station.screen.dirty = true;
                }
            })
            .map_err(|e| anyhow!("session in the event loop: {e}"))?;

        event_loop
            .handle()
            .insert_source(drm_notifier, |event, _, station| match event {
                DrmEvent::VBlank(_) => {
                    if let Err(err) = station.screen.output.frame_submitted() {
                        log::warn!("frame submitted: {err:?}");
                    }
                    station.screen.waiting = false;
                }
                DrmEvent::Error(err) => log::error!("DRM: {err:?}"),
            })
            .map_err(|e| anyhow!("DRM events in the event loop: {e}"))?;

        Ok(Self { session, renderer, manager, output, size: (u32::from(w), u32::from(h)), waiting: false, dirty: true, memory: None })
    }

    /// Something to show: drawn on the next `draw`.
    pub fn changed(&mut self) {
        self.dirty = true;
    }

    /// Draws the `video`, with `ui` over it if given, if something changed
    /// and the screen can take a frame.
    pub fn draw(&mut self, ui: Option<&MemoryRenderBuffer>, video: Option<&Frame>) {
        if !self.dirty || self.waiting || !self.session.is_active() {
            return;
        }
        let mut elements = Vec::with_capacity(2);
        if let Some(ui) = ui {
            match MemoryRenderBufferRenderElement::from_buffer(&mut self.renderer, (0.0, 0.0), ui, None, None, None, Kind::Unspecified) {
                Ok(element) => elements.push(Element::Memory(element)),
                Err(err) => return log::warn!("the station's screen: {err:?}"),
            }
        }
        if let Some(Frame { picture: Picture::Memory { size, stride, fourcc }, .. }) = video {
            match self.memory_picture(video.expect("a frame"), *size, *stride, *fourcc) {
                Ok(element) => elements.push(Element::Memory(element)),
                Err(err) => log::warn!("the video's picture: {err:#}"),
            }
        }
        if let Some(Frame { picture: Picture::Dmabuf(video), .. }) = video {
            match self.renderer.import_dmabuf(video, None) {
                // A new id each picture: all of it is new.
                Ok(texture) => elements.push(Element::Video(TextureRenderElement::from_static_texture(
                    Id::new(),
                    self.renderer.context_id(),
                    (0.0, 0.0),
                    texture,
                    1,
                    Transform::Normal,
                    None,
                    None,
                    None,
                    None,
                    Kind::Unspecified,
                ))),
                Err(err) => log::warn!("the video's picture: {err:?}"),
            }
        }
        match self.output.render_frame(&mut self.renderer, &elements, [0.0, 0.0, 0.0, 1.0], FrameFlags::DEFAULT) {
            Ok(result) if !result.is_empty => match self.output.queue_frame(()) {
                Ok(()) => {
                    self.waiting = true;
                    self.dirty = false;
                }
                Err(err) => log::warn!("queue frame: {err:?}"),
            },
            Ok(_) => self.dirty = false,
            Err(err) => log::warn!("render: {err:?}"),
        }
    }
}

impl Screen {
    /// A picture in memory, copied row by row into the buffer
    /// kept for it (the decoder's rows may be padded), and drawn fitted to
    /// the monitor, whole, the rest left black.
    fn memory_picture(
        &mut self,
        frame: &Frame,
        (width, height): (u32, u32),
        stride: usize,
        fourcc: Fourcc,
    ) -> anyhow::Result<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let pixels = frame.pixels().context("the picture's memory")?;
        let size = (width as i32, height as i32);
        if self.memory.as_ref().is_some_and(|(format, _)| *format != fourcc) {
            self.memory = None;
        }
        let (_, buffer) = self.memory.get_or_insert_with(|| (fourcc, MemoryRenderBuffer::new(fourcc, size, 1, Transform::Normal, None)));
        {
            let mut context = buffer.render();
            context.resize(size);
            let row = width as usize * 4;
            context.draw(|memory| {
                if pixels.len() < stride * (height as usize - 1) + row {
                    return Err(anyhow!("a picture shorter than its size"));
                }
                for (y, out) in memory.chunks_exact_mut(row).take(height as usize).enumerate() {
                    out.copy_from_slice(&pixels[y * stride..y * stride + row]);
                }
                Ok(vec![Rectangle::from_size(size.into())])
            })?;
        }
        let (at, fitted) = fit((width, height), self.size);
        MemoryRenderBufferRenderElement::from_buffer(&mut self.renderer, at, buffer, None, None, Some(fitted.into()), Kind::Unspecified)
            .map_err(|e| anyhow!("{e:?}"))
    }
}

/// Where a picture of `size` goes on a monitor of `monitor`, and at what
/// size: as large as fits, its proportions kept, centred.
fn fit((width, height): (u32, u32), (monitor_w, monitor_h): (u32, u32)) -> ((f64, f64), (i32, i32)) {
    let scale = (f64::from(monitor_w) / f64::from(width)).min(f64::from(monitor_h) / f64::from(height));
    let (w, h) = ((f64::from(width) * scale).round(), (f64::from(height) * scale).round());
    (((f64::from(monitor_w) - w) / 2.0, (f64::from(monitor_h) - h) / 2.0), (w as i32, h as i32))
}

/// The monitor to use, its crtc and mode: `wanted` (a connector's name) or
/// the first connected, at its preferred mode.
fn monitor(drm: &DrmDevice, wanted: Option<&str>) -> anyhow::Result<(connector::Handle, crtc::Handle, smithay::reexports::drm::control::Mode)> {
    let resources = drm.resource_handles().context("the GPU's connectors")?;
    let mut connected = Vec::new();
    for handle in resources.connectors() {
        let info = drm.get_connector(*handle, false)?;
        if info.state() != connector::State::Connected || info.modes().is_empty() {
            continue;
        }
        let name = format!("{}-{}", info.interface().as_str(), info.interface_id());
        connected.push((name, info));
    }
    let names = connected.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(", ");
    let found = match wanted {
        Some(wanted) => connected.iter().find(|(name, _)| name == wanted).ok_or_else(|| anyhow!("no monitor {wanted} (connected: {names})"))?,
        None => connected.first().ok_or_else(|| anyhow!("no monitor connected"))?,
    };
    let (name, info) = found;
    let mode = info.modes().iter().find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED)).copied().unwrap_or(info.modes()[0]);
    // A crtc its encoders can drive.
    let crtc = info
        .encoders()
        .iter()
        .filter_map(|e| drm.get_encoder(*e).ok())
        .flat_map(|e| resources.filter_crtcs(e.possible_crtcs()))
        .next()
        .ok_or_else(|| anyhow!("{name}: no crtc"))?;
    let (w, h) = mode.size();
    log::info!("{name}: {w}x{h}@{}", mode.vrefresh());
    Ok((info.handle(), crtc, mode))
}

#[cfg(test)]
mod tests {
    use super::fit;

    #[test]
    fn pictures_fit_whole() {
        assert_eq!(fit((1920, 1080), (2560, 1440)), ((0.0, 0.0), (2560, 1440)));
        assert_eq!(fit((1920, 1080), (1280, 1024)), ((0.0, 152.0), (1280, 720)));
        assert_eq!(fit((1080, 1920), (1920, 1080)), ((656.0, 0.0), (608, 1080)));
    }
}
