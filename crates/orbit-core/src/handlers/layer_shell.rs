//! Layer surfaces (`wlr-layer-shell`): the panel, the dock, the
//! notifications, menus of the panel. They sit on an edge of the output,
//! reserve their part of it, and the windows are laid out over the rest.
//!
//! Ported from anvil `src/shell/mod.rs` @ 24647fc (Smithay, MIT, Copyright
//! (c) 2017 Victor Berger and Victoria Brekenfeld).

use smithay::{
    desktop::{LayerSurface, PopupKind, layer_map_for_output},
    output::Output,
    reexports::wayland_server::protocol::wl_output::WlOutput,
    wayland::shell::{
        wlr_layer::{Layer, LayerSurface as WlrLayerSurface, WlrLayerShellHandler, WlrLayerShellState},
        xdg::PopupSurface,
    },
};

use crate::state::Orbit;

impl WlrLayerShellHandler for Orbit {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(&mut self, surface: WlrLayerSurface, output: Option<WlOutput>, _layer: Layer, namespace: String) {
        let Some(output) = output.as_ref().and_then(Output::from_resource).or_else(|| self.pointer_output()) else {
            tracing::warn!("layer surface {namespace}: no output");
            return;
        };
        if let Err(err) = layer_map_for_output(&output).map_layer(&LayerSurface::new(surface, namespace.clone())) {
            tracing::warn!("layer surface {namespace}: {err}");
        }
    }

    fn new_popup(&mut self, _parent: WlrLayerSurface, popup: PopupSurface) {
        self.unconstrain_popup(&popup);
        if let Err(err) = self.popups.track_popup(PopupKind::Xdg(popup)) {
            tracing::warn!("layer popup: {err}");
        }
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let gone = self.space.outputs().find_map(|o| {
            let map = layer_map_for_output(o);
            map.layers().find(|l| l.layer_surface() == &surface).cloned().map(|l| (o.clone(), l))
        });
        let Some((output, layer)) = gone else { return };
        let had_keyboard = self.seat.get_keyboard().and_then(|k| k.current_focus()).as_ref() == Some(layer.wl_surface());
        layer_map_for_output(&output).unmap_layer(&layer);
        // Its room goes back to the windows.
        self.arrange();
        if had_keyboard {
            let window = self.tiled().last().cloned();
            self.focus_window(window);
        }
    }
}
