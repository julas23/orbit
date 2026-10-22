//! The nested backend: the core as a window inside another Wayland session
//! (sway today), drawn with GLES. Frames are drawn only when something
//! changed (PLANO-CORE §1.3): a commit, input, a resize. The host draws the
//! pointer.
//!
//! Ported from Smithay's smallvil `src/winit.rs` @ 24647fc (MIT, Copyright
//! (c) 2017 Victor Berger and Victoria Brekenfeld), with the dmabuf setup
//! of anvil `src/winit.rs` @ 24647fc (same licence).
//!
//! linux-dmabuf goes out at version 5 with feedback naming the render node
//! (Mesa 26 finds the GPU only that way), with EGL's wl_drm beside it for
//! older Mesa. Version 5, not 6, through the vendored Smithay: GTK 4.18
//! (Debian 13) crashes on 6, which no longer sends `main_device`. Without
//! a render node it falls back to version 3.

use std::time::Duration;

use smithay::{
    backend::{
        egl::EGLDevice,
        renderer::{ImportDma, ImportEgl, ImportMemWl, damage::OutputDamageTracker, gles::GlesRenderer},
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    desktop::layer_map_for_output,
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::EventLoop,
    utils::{Rectangle, Transform},
    wayland::dmabuf::{DmabufFeedbackBuilder, DmabufGlobal, DmabufState},
};

use crate::{
    render::{background, output_elements},
    state::{Backend, Orbit},
};

pub struct WinitData {
    pub backend: WinitGraphicsBackend<GlesRenderer>,
    damage_tracker: OutputDamageTracker,
    pub dmabuf_state: DmabufState,
    _dmabuf_global: DmabufGlobal,
    output: Output,
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
    state.add_output(&output);

    // Buffers from the GPU (see the note at the top).
    let mut dmabuf_state = DmabufState::new();
    let formats = backend.renderer().dmabuf_formats();
    let render_node = EGLDevice::device_for_display(backend.renderer().egl_context().display())
        .ok()
        .and_then(|device| device.try_get_render_node().ok().flatten());
    let feedback = render_node.and_then(|node| DmabufFeedbackBuilder::new(node.dev_id(), formats.clone()).build().ok());
    let dmabuf_global = match &feedback {
        Some(feedback) => dmabuf_state.create_global_with_default_feedback::<Orbit>(&state.display_handle, feedback),
        None => dmabuf_state.create_global::<Orbit>(&state.display_handle, formats),
    };
    if let Err(err) = backend.renderer().bind_wl_display(&state.display_handle) {
        tracing::warn!("no wl_drm, GL clients may draw in software: {err:?}");
    }
    state.shm_state.update_formats(backend.renderer().shm_formats());

    state.backend = Backend::Winit(Box::new(WinitData {
        backend,
        damage_tracker: OutputDamageTracker::from_output(&output),
        dmabuf_state,
        _dmabuf_global: dmabuf_global,
        output: output.clone(),
    }));

    event_loop
        .handle()
        .insert_source(winit, move |event, _, state| match event {
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

/// Draws a frame (only what changed), then tells clients they may draw the
/// next one.
fn render(state: &mut Orbit) {
    state.needs_render = false;
    // Taken out while drawing: the elements are built from the state.
    let Backend::Winit(mut winit) = std::mem::replace(&mut state.backend, Backend::None) else { return };
    let output = winit.output.clone();
    // Right after the nested window is resized the surface has no age yet
    // (EGL logs BAD_SURFACE): the frame is then drawn whole.
    let age = winit.backend.buffer_age().unwrap_or(0);
    let elements = output_elements(state, &output, winit.backend.renderer(), false);
    let clear = background(state);
    let damage = match winit.backend.bind() {
        Ok((renderer, mut framebuffer)) => match winit.damage_tracker.render_output(renderer, &mut framebuffer, age, &elements, clear) {
            Ok(result) => result.damage.cloned(),
            Err(err) => {
                tracing::warn!("render: {err:?}");
                None
            }
        },
        Err(err) => {
            tracing::warn!("bind: {err:?}");
            None
        }
    };
    if let Some(damage) = damage {
        let size = winit.backend.window_size();
        let damage = if damage.is_empty() { vec![Rectangle::from_size(size)] } else { damage };
        if let Err(err) = winit.backend.submit(Some(&damage)) {
            tracing::warn!("submit: {err:?}");
        }
    }
    // The eyes moving: another frame.
    if state.animating() {
        winit.backend.window().request_redraw();
    }
    state.backend = Backend::Winit(winit);
    state.confirm_lock();

    // Locked, clients are not drawn, so they are not asked to draw; the
    // greet is.
    let time = state.start_time.elapsed();
    for window in state.space.elements().filter(|_| state.session_shown()) {
        window.send_frame(&output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
    }
    for layer in layer_map_for_output(&output).layers().filter(|_| state.session_shown()) {
        layer.send_frame(&output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
    }
    state.face_send_frame(&output, time);
    state.space.refresh();
    state.popups.cleanup();
    layer_map_for_output(&output).cleanup();
    let _ = state.display_handle.flush_clients();
}
