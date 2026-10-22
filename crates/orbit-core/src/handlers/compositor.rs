//! Surface commits: a window's or a layer surface's new content, its first
//! configure (when a dialog starts floating), and a new frame to draw.
//!
//! Ported from Smithay's smallvil `src/handlers/compositor.rs` @ 24647fc
//! (MIT, Copyright (c) 2017 Victor Berger and Victoria Brekenfeld); the
//! layer surfaces' first configure follows anvil `src/shell/mod.rs` @
//! 24647fc (same licence).

use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    desktop::{PopupKind, layer_map_for_output},
    input::pointer::{CursorImageStatus, CursorImageSurfaceData},
    reexports::wayland_server::{
        Client,
        protocol::{wl_buffer, wl_surface::WlSurface},
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState, SurfaceAttributes, get_parent, is_sync_subsurface, with_states},
        shell::{wlr_layer::LayerSurfaceData, xdg::XdgToplevelSurfaceData},
        shm::{ShmHandler, ShmState},
    },
};

use crate::state::{ClientState, Orbit};

impl CompositorHandler for Orbit {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        // XWayland is a client with its own data.
        if let Some(xwayland) = client.get_data::<smithay::xwayland::XWaylandClientData>() {
            return &xwayland.compositor_state;
        }
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.move_hotspot(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self.window_for(&root).or_else(|| self.greet_window(&root)) {
                window.on_commit();
                self.floating_committed(&window);
            }
        }
        self.first_configure(surface);
        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface)
            && !popup.is_initial_configure_sent()
        {
            // The first configure is always allowed.
            popup.send_configure().expect("initial popup configure");
        }
        // Locked, a client changing is nothing to draw, unless it is the
        // greet.
        if self.session_shown() || self.is_face(surface) {
            self.request_render();
        }
    }
}

impl Orbit {
    /// The pointer's own surface moved its image (`wl_surface.offset`, or
    /// `attach` with x and y): the hotspot moves the other way. GTK swaps
    /// the image so, and a hotspot left behind clicks beside the arrow.
    fn move_hotspot(&mut self, surface: &WlSurface) {
        if !matches!(&self.cursor_status, CursorImageStatus::Surface(s) if s == surface) {
            return;
        }
        with_states(surface, |states| {
            let delta = states.cached_state.get::<SurfaceAttributes>().current().buffer_delta.take();
            if let (Some(delta), Some(data)) = (delta, states.data_map.get::<CursorImageSurfaceData>()) {
                data.lock().unwrap().hotspot -= delta;
            }
        });
    }

    /// A new window or layer surface gets its first configure on its first
    /// commit; a layer surface that changed its size or exclusive zone moves
    /// the windows.
    fn first_configure(&mut self, surface: &WlSurface) {
        if let Some(window) = self.window_for(surface).or_else(|| self.greet_window(surface)) {
            let sent = with_states(surface, |states| {
                states.data_map.get::<XdgToplevelSurfaceData>().is_some_and(|d| d.lock().unwrap().initial_configure_sent)
            });
            if !sent && let Some(toplevel) = window.toplevel() {
                // Its parent and size limits are known now: a dialog floats.
                self.float_if_dialog(&window);
                toplevel.send_configure();
            }
            return;
        }
        let Some(output) = self.space.outputs().find(|o| layer_map_for_output(o).layer_for_surface(surface, smithay::desktop::WindowSurfaceType::TOPLEVEL).is_some()).cloned() else {
            return;
        };
        let sent = with_states(surface, |states| {
            states.data_map.get::<LayerSurfaceData>().is_some_and(|d| d.lock().unwrap().initial_configure_sent)
        });
        let changed = {
            let mut map = layer_map_for_output(&output);
            let changed = map.arrange();
            if !sent && let Some(layer) = map.layer_for_surface(surface, smithay::desktop::WindowSurfaceType::TOPLEVEL) {
                layer.layer_surface().send_configure();
            }
            changed
        };
        if changed {
            self.arrange();
        }
        // A surface that wants the keyboard while it is up gets it.
        if let Some(layer) = self.exclusive_layer()
            && layer.wl_surface() == surface
        {
            self.focus_layer(&layer);
        }
    }
}

impl BufferHandler for Orbit {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for Orbit {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}
