//! The nested backend: the core as a window inside another Wayland session
//! (sway today), drawn with GLES. Frames are drawn only when something
//! changed (PLANO-CORE §1.3): a commit, input, a resize.
//!
//! Ported from Smithay's smallvil `src/winit.rs` @ 24647fc (MIT, Copyright
//! (c) 2017 Victor Berger and Victoria Brekenfeld), with the dmabuf setup
//! of anvil `src/winit.rs` @ 24647fc (same licence).
//!
//! linux-dmabuf goes out at version 3, with EGL's wl_drm beside it for Mesa:
//! Smithay at this commit offers version 6 whenever there is feedback, and
//! GTK 4.18 (Debian 13) crashes on it while opening the display — v6 no
//! longer sends `main_device`, which it still reads. Back to v4+ feedback
//! when the GTK in use takes it.

use std::{cell::RefCell, time::Duration};

use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        renderer::{
            ImportDma, ImportEgl, ImportMemWl,
            damage::OutputDamageTracker,
            element::{
                Kind,
                solid::{SolidColorBuffer, SolidColorRenderElement},
            },
            gles::GlesRenderer,
        },
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    desktop::{Window, layer_map_for_output, space::render_output},
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::EventLoop,
    utils::{Rectangle, Transform},
    wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
};

use crate::{config::color, state::Orbit};

/// The screen behind the windows.
const BACKGROUND: [f32; 4] = [0.106, 0.122, 0.153, 1.0];

pub struct WinitData {
    pub backend: WinitGraphicsBackend<GlesRenderer>,
    damage_tracker: OutputDamageTracker,
    dmabuf_state: DmabufState,
    _dmabuf_global: DmabufGlobal,
    output: Output,
}

impl DmabufHandler for Orbit {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.winit.as_mut().expect("the winit backend").dmabuf_state
    }

    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
        let imported = self.winit.as_mut().is_some_and(|w| w.backend.renderer().import_dmabuf(&dmabuf, None).is_ok());
        if imported {
            let _ = notifier.successful::<Orbit>();
        } else {
            notifier.failed();
        }
    }
}

pub fn init(event_loop: &mut EventLoop<'static, Orbit>, state: &mut Orbit) -> anyhow::Result<()> {
    let (mut backend, winit) = winit::init::<GlesRenderer>().map_err(|e| anyhow::anyhow!("nested window: {e}"))?;
    backend.window().set_title("Orbit");

    let mode = Mode { size: backend.window_size(), refresh: 60_000 };
    let output = Output::new(
        "winit".to_string(),
        PhysicalProperties { size: (0, 0).into(), subpixel: Subpixel::Unknown, make: "Orbit".into(), model: "Nested".into(), serial_number: String::new() },
    );
    let _global = output.create_global::<Orbit>(&state.display_handle);
    output.change_current_state(Some(mode), Some(Transform::Flipped180), None, Some((0, 0).into()));
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));

    // Buffers from the GPU (see the note at the top).
    let formats = backend.renderer().dmabuf_formats();
    let mut dmabuf_state = DmabufState::new();
    let dmabuf_global = dmabuf_state.create_global::<Orbit>(&state.display_handle, formats);
    if let Err(err) = backend.renderer().bind_wl_display(&state.display_handle) {
        tracing::warn!("no wl_drm, GL clients may draw in software: {err:?}");
    }
    state.shm_state.update_formats(backend.renderer().shm_formats());

    state.winit = Some(WinitData {
        backend,
        damage_tracker: OutputDamageTracker::from_output(&output),
        dmabuf_state,
        _dmabuf_global: dmabuf_global,
        output: output.clone(),
    });

    event_loop.handle().insert_source(winit, move |event, _, state| match event {
        WinitEvent::Resized { size, .. } => {
            output.change_current_state(Some(Mode { size, refresh: 60_000 }), None, None, None);
            state.arrange();
        }
        WinitEvent::Input(event) => state.process_input_event(event),
        WinitEvent::Redraw => render(state),
        WinitEvent::CloseRequested => state.loop_signal.stop(),
        _ => {}
    })
    .map_err(|e| anyhow::anyhow!("nested window events: {e}"))?;
    Ok(())
}

/// The four sides of a window's border, kept with the window so the
/// damage tracker sees the same elements frame after frame.
#[derive(Default)]
struct Border([SolidColorBuffer; 4]);

fn border_elements(state: &Orbit) -> Vec<SolidColorRenderElement> {
    let width = state.config.border;
    if width <= 0 {
        return Vec::new();
    }
    let focused = state.focused_window();
    let (on, off) = (color(&state.config.focused_color), color(&state.config.unfocused_color));
    let mut elements = Vec::new();
    for window in &state.tiled() {
        let Some(geo) = state.space.element_geometry(window) else { continue };
        let colour = if Some(window) == focused.as_ref() { on } else { off };
        let border = border_of(window);
        let mut border = border.borrow_mut();
        let (x, y, w, h) = (geo.loc.x - width, geo.loc.y - width, geo.size.w + 2 * width, geo.size.h + 2 * width);
        let sides = [(x, y, w, width), (x, y + h - width, w, width), (x, y + width, width, h - 2 * width), (x + w - width, y + width, width, h - 2 * width)];
        for (buffer, (sx, sy, sw, sh)) in border.0.iter_mut().zip(sides) {
            buffer.update((sw.max(0), sh.max(0)), colour);
            elements.push(SolidColorRenderElement::from_buffer(buffer, (sx, sy), 1.0, 1.0, Kind::Unspecified));
        }
    }
    elements
}

fn border_of(window: &Window) -> &RefCell<Border> {
    window.user_data().insert_if_missing(|| RefCell::new(Border::default()));
    window.user_data().get::<RefCell<Border>>().expect("just inserted")
}

/// Draws a frame (only what changed), then tells clients they may draw the
/// next one.
fn render(state: &mut Orbit) {
    tracing::debug!(windows = state.tiled().len(), "frame");
    state.needs_render = false;
    let borders = border_elements(state);
    let Some(winit) = state.winit.as_mut() else { return };
    let output = winit.output.clone();
    // Right after the nested window is resized the surface has no age yet
    // (EGL logs BAD_SURFACE): the frame is then drawn whole.
    let age = winit.backend.buffer_age().unwrap_or(0);
    let damage = match winit.backend.bind() {
        Ok((renderer, mut framebuffer)) => {
            match render_output::<_, SolidColorRenderElement, _, _>(
                &output,
                renderer,
                &mut framebuffer,
                1.0,
                age,
                [&state.space],
                &borders,
                &mut winit.damage_tracker,
                BACKGROUND,
            ) {
                Ok(result) => result.damage.cloned(),
                Err(err) => {
                    tracing::warn!("render: {err:?}");
                    None
                }
            }
        }
        Err(err) => {
            tracing::warn!("bind: {err:?}");
            None
        }
    };
    tracing::debug!(?damage, "frame drawn");
    if let Some(damage) = damage {
        let size = winit.backend.window_size();
        let damage = if damage.is_empty() { vec![Rectangle::from_size(size)] } else { damage };
        if let Err(err) = winit.backend.submit(Some(&damage)) {
            tracing::warn!("submit: {err:?}");
        }
    }

    let time = state.start_time.elapsed();
    for window in state.space.elements() {
        window.send_frame(&output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
    }
    for layer in layer_map_for_output(&output).layers() {
        layer.send_frame(&output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
    }
    state.space.refresh();
    state.popups.cleanup();
    layer_map_for_output(&output).cleanup();
    let _ = state.display_handle.flush_clients();
}
