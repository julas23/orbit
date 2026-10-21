//! Layer surfaces (`wlr-layer-shell`): the panel, the dock, the
//! notifications, menus of the panel. They sit on an edge of the output,
//! reserve their part of it, and the windows are laid out over the rest.
//!
//! The parts are reserved in the order the surfaces came, so the panel is
//! always put first: it takes its whole edge, and the dock and the others
//! only what it leaves — whether the dock came before it or not (a panel
//! restarted under a running dock).
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

/// Orbit-Panel's layer-shell namespace.
const PANEL: &str = "orbit-panel";

impl WlrLayerShellHandler for Orbit {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(&mut self, surface: WlrLayerSurface, output: Option<WlOutput>, _layer: Layer, namespace: String) {
        let Some(output) = output.as_ref().and_then(Output::from_resource).or_else(|| self.pointer_output()) else {
            tracing::warn!("layer surface {namespace}: no output");
            return;
        };
        let mut map = layer_map_for_output(&output);
        if let Err(err) = map.map_layer(&LayerSurface::new(surface, namespace.clone())) {
            tracing::warn!("layer surface {namespace}: {err}");
            return;
        }
        if namespace == PANEL {
            // The others again, now after the panel.
            let others: Vec<LayerSurface> = map.layers().filter(|l| l.namespace() != PANEL).cloned().collect();
            for layer in &others {
                map.unmap_layer(layer);
            }
            for layer in &others {
                if let Err(err) = map.map_layer(layer) {
                    tracing::warn!("layer surface {}: {err}", layer.namespace());
                }
            }
            // `arrange` takes the map again.
            drop(map);
            self.arrange();
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
        let had_keyboard = self.seat.get_keyboard().and_then(|k| k.current_focus()).is_some_and(|f| f.is(layer.wl_surface()));
        layer_map_for_output(&output).unmap_layer(&layer);
        // Its room goes back to the windows.
        self.arrange();
        if had_keyboard {
            let window = self.tiled().last().cloned();
            self.focus_window(window);
        }
    }
}
