//! The real backend: the core owns the screen and the input devices, as a
//! session started from a text console (or the greeter). The session
//! (libseat, through logind) hands over the GPU and the input devices and
//! takes them back on a VT switch; libinput reads keyboard, mouse and
//! touchpad; each connected monitor is a DRM output with its own
//! swapchain.
//!
//! One GPU (the hp's and the think's have one). Frames are drawn on demand
//! (PLANO-CORE §1.3): a request draws on the next idle turn unless a frame
//! is still waiting for its vblank, and the vblank draws again only when
//! something changed on that output since.
//!
//! Ported from anvil `src/udev.rs` @ 24647fc (Smithay, MIT, Copyright (c)
//! 2017 Victor Berger and Victoria Brekenfeld), without its multi-GPU
//! manager, leases and per-frame repaint.

use std::{collections::HashMap, time::Duration};

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
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{ImportDma, ImportEgl, ImportMemWl, gles::GlesRenderer},
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{UdevBackend, UdevEvent, all_gpus, primary_gpu},
    },
    desktop::layer_map_for_output,
    input::pointer::CursorImageStatus,
    output::{Mode as WlMode, Output, PhysicalProperties},
    reexports::{
        calloop::EventLoop,
        drm::control::{ModeTypeFlags, connector, crtc},
        input::Libinput,
        rustix::fs::OFlags,
        wayland_server::backend::GlobalId,
    },
    utils::DeviceFd,
    wayland::{
        compositor::{SurfaceAttributes, TraversalAction, with_surface_tree_downward},
        dmabuf::{DmabufFeedbackBuilder, DmabufGlobal, DmabufState},
    },
};
use smithay_drm_extras::{display_info, drm_scanner::{DrmScanEvent, DrmScanner}};

use crate::{
    render::{OutputElement, background, output_elements},
    state::{Backend, Orbit},
};

/// 8-bit colour: what every monitor takes.
const FORMATS: &[Fourcc] = &[Fourcc::Abgr8888, Fourcc::Argb8888];

type Manager = DrmOutputManager<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;
type Element = OutputElement;

pub struct DrmData {
    pub session: LibSeatSession,
    pub renderer: GlesRenderer,
    manager: Manager,
    scanner: DrmScanner,
    surfaces: HashMap<crtc::Handle, Surface>,
    pub dmabuf_state: DmabufState,
    _dmabuf_global: DmabufGlobal,
}

struct Surface {
    output: Output,
    drm_output: DrmOutput<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>,
    global: GlobalId,
    /// A frame is queued and its vblank has not come.
    waiting: bool,
    /// A redraw is already set for the next idle turn.
    scheduled: bool,
    /// Something changed since its last frame. Kept per output: drawing
    /// one must not clear what another, still waiting for its vblank,
    /// has yet to show.
    dirty: bool,
}

pub fn init(event_loop: &mut EventLoop<'static, Orbit>, state: &mut Orbit) -> anyhow::Result<()> {
    let (mut session, notifier) = LibSeatSession::new().map_err(|e| anyhow::anyhow!("no seat session (run it from a text console): {e}"))?;
    let seat = session.seat();

    let path = primary_gpu(&seat)?
        .or_else(|| all_gpus(&seat).ok()?.into_iter().next())
        .ok_or_else(|| anyhow::anyhow!("no GPU"))?;
    let node = DrmNode::from_path(&path)?;
    tracing::info!("GPU {} ({})", path.display(), node);
    let fd = session
        .open(&path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)
        .map_err(|e| anyhow::anyhow!("opening {}: {e}", path.display()))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, drm_notifier) = DrmDevice::new(fd.clone(), true)?;
    let gbm = GbmDevice::new(fd)?;

    let display = unsafe { EGLDisplay::new(gbm.clone())? };
    let context = EGLContext::new(&display)?;
    let mut renderer = unsafe { GlesRenderer::new(context)? };
    let render_formats = renderer.egl_context().dmabuf_render_formats().clone();
    let render_node = node.node_with_type(NodeType::Render).and_then(Result::ok).unwrap_or(node);
    let manager = DrmOutputManager::new(
        drm,
        GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT),
        GbmFramebufferExporter::new(gbm.clone(), Some(render_node).into()),
        Some(gbm),
        FORMATS.iter().copied(),
        render_formats,
    );

    // Client buffers: dmabuf v5 with feedback, and wl_drm, as nested (see `winit`).
    let mut dmabuf_state = DmabufState::new();
    let formats = renderer.dmabuf_formats();
    let dmabuf_global = match DmabufFeedbackBuilder::new(render_node.dev_id(), formats.clone()).build() {
        Ok(feedback) => dmabuf_state.create_global_with_default_feedback::<Orbit>(&state.display_handle, &feedback),
        Err(_) => dmabuf_state.create_global::<Orbit>(&state.display_handle, formats),
    };
    if let Err(err) = renderer.bind_wl_display(&state.display_handle) {
        tracing::warn!("no wl_drm, GL clients may draw in software: {err:?}");
    }
    state.shm_state.update_formats(renderer.shm_formats());

    // Keyboard, mouse, touchpad.
    let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput.udev_assign_seat(&seat).map_err(|()| anyhow::anyhow!("libinput: no seat {seat}"))?;
    event_loop
        .handle()
        .insert_source(LibinputInputBackend::new(libinput.clone()), |event, _, state| state.process_input_event(event))
        .map_err(|e| anyhow::anyhow!("libinput in the event loop: {e}"))?;

    // Leaving for another VT, and coming back.
    event_loop
        .handle()
        .insert_source(notifier, move |event, _, state| match event {
            SessionEvent::PauseSession => {
                tracing::info!("session paused");
                libinput.suspend();
                if let Backend::Drm(drm) = &mut state.backend {
                    drm.manager.pause();
                }
            }
            SessionEvent::ActivateSession => {
                tracing::info!("session back");
                if let Err(err) = libinput.resume() {
                    tracing::error!("libinput resume: {err:?}");
                }
                if let Backend::Drm(drm) = &mut state.backend {
                    if let Err(err) = drm.manager.lock().activate(false) {
                        tracing::error!("DRM activate: {err}");
                    }
                    for surface in drm.surfaces.values_mut() {
                        surface.waiting = false;
                    }
                }
                state.request_render();
            }
        })
        .map_err(|e| anyhow::anyhow!("session in the event loop: {e}"))?;

    event_loop
        .handle()
        .insert_source(drm_notifier, move |event, _, state| match event {
            DrmEvent::VBlank(crtc) => vblank(state, crtc),
            DrmEvent::Error(err) => tracing::error!("DRM: {err:?}"),
        })
        .map_err(|e| anyhow::anyhow!("DRM events in the event loop: {e}"))?;

    // A monitor plugged in or out.
    let udev = UdevBackend::new(&seat)?;
    let dev_id = node.dev_id();
    event_loop
        .handle()
        .insert_source(udev, move |event, _, state| {
            if let UdevEvent::Changed { device_id } = event
                && device_id == dev_id
            {
                scan(state);
            }
        })
        .map_err(|e| anyhow::anyhow!("udev in the event loop: {e}"))?;

    state.backend = Backend::Drm(Box::new(DrmData {
        session,
        renderer,
        manager,
        scanner: DrmScanner::new(),
        surfaces: HashMap::new(),
        dmabuf_state,
        _dmabuf_global: dmabuf_global,
    }));
    scan(state);
    Ok(())
}

/// Looks at the connectors: outputs for the monitors that came, none for
/// those that went.
fn scan(state: &mut Orbit) {
    let Backend::Drm(drm) = &mut state.backend else { return };
    let events = match drm.scanner.scan_connectors(drm.manager.device()) {
        Ok(result) => result.into_iter().collect::<Vec<_>>(),
        Err(err) => {
            tracing::warn!("scanning connectors: {err}");
            return;
        }
    };
    for event in events {
        match event {
            DrmScanEvent::Connected { connector, crtc: Some(crtc) } => connected(state, connector, crtc),
            DrmScanEvent::Disconnected { crtc: Some(crtc), .. } => disconnected(state, crtc),
            _ => {}
        }
    }
    state.arrange();
}

fn connected(state: &mut Orbit, connector: connector::Info, crtc: crtc::Handle) {
    let name = format!("{}-{}", connector.interface().as_str(), connector.interface_id());
    let Backend::Drm(drm) = &mut state.backend else { return };
    let Some(mode) = connector.modes().iter().find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED)).or_else(|| connector.modes().first()).copied() else {
        tracing::warn!("{name}: no modes");
        return;
    };
    let info = display_info::for_connector(drm.manager.device(), connector.handle());
    let (w, h) = connector.size().unwrap_or((0, 0));
    let output = Output::new(
        name.clone(),
        PhysicalProperties {
            size: (w as i32, h as i32).into(),
            subpixel: connector.subpixel().into(),
            make: info.as_ref().and_then(|i| i.make()).unwrap_or_else(|| "Unknown".into()),
            model: info.as_ref().and_then(|i| i.model()).unwrap_or_else(|| "Unknown".into()),
            serial_number: info.as_ref().and_then(|i| i.serial()).unwrap_or_default(),
        },
    );
    let global = output.create_global::<Orbit>(&state.display_handle);
    let wl_mode = WlMode::from(mode);
    output.set_preferred(wl_mode);
    // Its place comes with the others' (`output::place`).
    output.change_current_state(Some(wl_mode), None, None, Some((0, 0).into()));
    let Backend::Drm(drm) = &mut state.backend else { return };
    let drm_output = match drm.manager.lock().initialize_output::<_, Element>(
        crtc,
        mode,
        &[connector.handle()],
        &output,
        None,
        &mut drm.renderer,
        &DrmOutputRenderElements::default(),
    ) {
        Ok(o) => o,
        Err(err) => {
            tracing::error!("{name}: {err}");
            state.display_handle.remove_global::<Orbit>(global);
            return;
        }
    };
    tracing::info!("{name}: {}x{}@{}", wl_mode.size.w, wl_mode.size.h, wl_mode.refresh);
    drm.surfaces.insert(crtc, Surface { output: output.clone(), drm_output, global, waiting: false, scheduled: false, dirty: true });
    state.add_output(&output);
    state.request_render();
}

fn disconnected(state: &mut Orbit, crtc: crtc::Handle) {
    let Backend::Drm(drm) = &mut state.backend else { return };
    let Some(surface) = drm.surfaces.remove(&crtc) else { return };
    tracing::info!("{}: gone", surface.output.name());
    state.display_handle.disable_global::<Orbit>(surface.global.clone());
    state.remove_output(&surface.output);
}

/// Marks every output changed and draws those not waiting for their
/// vblank on the next idle turn of the loop; the others draw at their
/// vblank.
pub fn schedule(state: &mut Orbit) {
    let Backend::Drm(drm) = &mut state.backend else { return };
    for (crtc, surface) in drm.surfaces.iter_mut() {
        surface.dirty = true;
        if surface.waiting || surface.scheduled {
            continue;
        }
        surface.scheduled = true;
        let crtc = *crtc;
        state.loop_handle.insert_idle(move |state| render(state, crtc));
    }
}

fn vblank(state: &mut Orbit, crtc: crtc::Handle) {
    let Backend::Drm(drm) = &mut state.backend else { return };
    let Some(surface) = drm.surfaces.get_mut(&crtc) else { return };
    if let Err(err) = surface.drm_output.frame_submitted() {
        tracing::warn!("frame submitted: {err:?}");
    }
    surface.waiting = false;
    if surface.dirty && !surface.scheduled {
        surface.scheduled = true;
        state.loop_handle.insert_idle(move |state| render(state, crtc));
    }
    state.lock_shown();
    state.confirm_lock();
}

fn render(state: &mut Orbit, crtc: crtc::Handle) {
    // Taken out while drawing: the elements are built from the state.
    let Backend::Drm(mut drm) = std::mem::replace(&mut state.backend, Backend::None) else { return };
    let Some(output) = drm.surfaces.get(&crtc).map(|s| s.output.clone()) else {
        state.backend = Backend::Drm(drm);
        return;
    };
    if let Some(surface) = drm.surfaces.get_mut(&crtc) {
        surface.scheduled = false;
        surface.dirty = false;
    }
    let elements = output_elements(state, &output, &mut drm.renderer, true);
    let clear = background(state);
    if let Some(surface) = drm.surfaces.get_mut(&crtc) {
        match surface.drm_output.render_frame(&mut drm.renderer, &elements, clear, FrameFlags::DEFAULT) {
            Ok(result) if !result.is_empty => match surface.drm_output.queue_frame(()) {
                Ok(()) => surface.waiting = true,
                Err(err) => tracing::warn!("queue frame: {err:?}"),
            },
            Ok(_) => {}
            Err(err) => tracing::warn!("render: {err:?}"),
        }
    }
    // The eyes moving: the next frame is drawn at this one's vblank.
    if state.animating()
        && let Some(surface) = drm.surfaces.get_mut(&crtc)
    {
        surface.dirty = true;
    }
    state.backend = Backend::Drm(drm);
    send_frames(state, &output);
    // Nothing new to show: no vblank comes.
    state.lock_shown();
    state.confirm_lock();
}

/// Tells the clients on `output` they may draw their next frame — only
/// the greet while locked: the others are not drawn.
fn send_frames(state: &mut Orbit, output: &Output) {
    let time = state.start_time.elapsed();
    for window in state.space.elements_for_output(output).filter(|_| state.session_shown()) {
        window.send_frame(output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
    }
    for layer in layer_map_for_output(output).layers().filter(|_| state.session_shown()) {
        layer.send_frame(output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
    }
    state.face_send_frame(output, time);
    // The pointer's own surface, when a client gave one.
    if let CursorImageStatus::Surface(surface) = &state.cursor_status {
        with_surface_tree_downward(
            surface,
            (),
            |_, _, &()| TraversalAction::DoChildren(()),
            |_, states, &()| {
                for callback in states.cached_state.get::<SurfaceAttributes>().current().frame_callbacks.drain(..) {
                    callback.done(time.as_millis() as u32);
                }
            },
            |_, _, &()| true,
        );
    }
    state.space.refresh();
    state.popups.cleanup();
    layer_map_for_output(output).cleanup();
    let _ = state.display_handle.flush_clients();
}

impl DrmData {
    /// Every screen shows the last frame drawn, with nothing left to draw.
    pub fn settled(&self) -> bool {
        self.surfaces.values().all(|s| !s.dirty && !s.scheduled && !s.waiting)
    }

    /// Switches to virtual terminal `vt` (Ctrl+Alt+F1…F12).
    pub fn change_vt(&mut self, vt: i32) {
        if let Err(err) = self.session.change_vt(vt) {
            tracing::warn!("VT {vt}: {err}");
        }
    }
}
