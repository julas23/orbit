//! Wayland backend, built on the wlr-layer-shell protocol.

use gtk4_layer_shell::{Edge as LayerEdge, Layer, LayerShell};
use orbit_common::{Backend, Edge, Placement, backend::monitor};

use gtk::prelude::*;

pub struct WaylandBackend;

impl WaylandBackend {
    pub fn new() -> anyhow::Result<Self> {
        anyhow::ensure!(
            gtk4_layer_shell::is_supported(),
            "the compositor does not support the layer-shell protocol"
        );
        Ok(Self)
    }
}

impl Backend for WaylandBackend {
    fn name(&self) -> &'static str {
        "wayland"
    }

    fn attach(&self, window: &gtk::Window, placement: Placement) -> anyhow::Result<()> {
        let monitor = monitor(&WidgetExt::display(window), &placement.monitor)?;

        window.init_layer_shell();
        window.set_layer(Layer::Top);
        window.set_monitor(Some(&monitor));
        window.set_namespace(Some(placement.namespace));

        // Anchor to the chosen edge and both adjacent ones, so the surface
        // stretches along the whole edge.
        let (edge, sides) = match placement.edge {
            Edge::Top => (LayerEdge::Top, [LayerEdge::Left, LayerEdge::Right]),
            Edge::Bottom => (LayerEdge::Bottom, [LayerEdge::Left, LayerEdge::Right]),
            Edge::Left => (LayerEdge::Left, [LayerEdge::Top, LayerEdge::Bottom]),
            Edge::Right => (LayerEdge::Right, [LayerEdge::Top, LayerEdge::Bottom]),
        };
        window.set_anchor(edge, true);
        for side in sides {
            window.set_anchor(side, true);
        }

        if placement.edge.is_vertical() {
            window.set_default_size(placement.size, -1);
        } else {
            window.set_default_size(-1, placement.size);
        }
        if placement.reserve {
            window.set_exclusive_zone(placement.size);
        }
        Ok(())
    }
}
