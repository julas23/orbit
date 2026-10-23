//! Wayland backend, built on the wlr-layer-shell protocol.

use gtk4_layer_shell::{Edge as LayerEdge, Layer, LayerShell};
use orbit_common::{
    Backend, Edge, Placement,
    backend::{find_monitor, monitor},
    config::MonitorSpec,
};

use gtk::{gdk, prelude::*};

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

    /// A monitor's name (its connector) may come after the monitor itself,
    /// as the outputs are announced: so the window is placed again on each
    /// change to the list and to each name.
    fn follow(&self, window: &gtk::Window, spec: &MonitorSpec) {
        let display = WidgetExt::display(window);
        let place = {
            let window = window.downgrade();
            let display = display.clone();
            let spec = spec.clone();
            move || {
                let Some(window) = window.upgrade() else { return };
                let Ok(target) = monitor(&display, &spec) else { return };
                if window.monitor().as_ref() == Some(&target) {
                    return;
                }
                if find_monitor(&display, &spec).is_some() {
                    log::info!("back on monitor {spec:?}");
                } else {
                    log::warn!("monitor {spec:?} is gone, on {:?} meanwhile", target.connector());
                }
                window.set_monitor(Some(&target));
            }
        };
        let watch = {
            let place = place.clone();
            move |monitor: &gdk::Monitor| {
                let place = place.clone();
                monitor.connect_connector_notify(move |_| place());
            }
        };
        let monitors = display.monitors();
        for monitor in monitors.iter::<gdk::Monitor>().flatten() {
            watch(&monitor);
        }
        monitors.connect_items_changed(move |list, at, _, added| {
            for i in at..at + added {
                if let Some(monitor) = list.item(i).and_then(|o| o.downcast::<gdk::Monitor>().ok()) {
                    watch(&monitor);
                }
            }
            place();
        });
    }
}
