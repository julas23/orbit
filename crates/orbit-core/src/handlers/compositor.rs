//! Surface commits: a window's or a layer surface's new content, its first
//! configure, and a new frame to draw.
//!
//! Ported from Smithay's smallvil `src/handlers/compositor.rs` @ 24647fc
//! (MIT, Copyright (c) 2017 Victor Berger and Victoria Brekenfeld); the
//! layer surfaces' first configure follows anvil `src/shell/mod.rs` @
//! 24647fc (same licence).

use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    desktop::{PopupKind, layer_map_for_output},
    reexports::wayland_server::{
        Client,
        protocol::{wl_buffer, wl_surface::WlSurface},
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState, get_parent, is_sync_subsurface, with_states},
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
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self.window_for(&root) {
                window.on_commit();
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
        // Locked, a client changing is nothing to draw.
        if !self.locked() {
            self.request_render();
        }
    }
}

impl Orbit {
    /// A new window or layer surface gets its first configure on its first
    /// commit; a layer surface that changed its size or exclusive zone moves
    /// the windows.
    fn first_configure(&mut self, surface: &WlSurface) {
        if let Some(window) = self.window_for(surface) {
            let sent = with_states(surface, |states| {
                states.data_map.get::<XdgToplevelSurfaceData>().is_some_and(|d| d.lock().unwrap().initial_configure_sent)
            });
            if !sent && let Some(toplevel) = window.toplevel() {
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
