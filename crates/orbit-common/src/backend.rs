//! Display-server abstraction.
//!
//! GTK4 has no portable API for docking a window to a screen edge: Wayland
//! needs the layer-shell protocol, and X11 needs EWMH hints set directly on
//! the window. Each backend implements [`Backend`] to hide that difference.

use gtk::{gdk, prelude::*};

use crate::{Edge, config::MonitorSpec};

/// Where and how a surface sits on screen.
#[derive(Debug, Clone)]
pub struct Placement {
    pub edge: Edge,
    /// Thickness in logical pixels, perpendicular to `edge`.
    pub size: i32,
    pub monitor: MonitorSpec,
    /// Reserve the space so other windows are laid out around the surface.
    pub reserve: bool,
    /// Identifies the surface to the compositor (the layer-shell namespace on
    /// Wayland), so rules can target the panel and the dock separately.
    pub namespace: &'static str,
}

impl Placement {
    /// The surface rectangle, in logical pixels, on a monitor with `geometry`.
    pub fn rect(&self, geometry: &gdk::Rectangle) -> gdk::Rectangle {
        let (x, y, w, h) = (
            geometry.x(),
            geometry.y(),
            geometry.width(),
            geometry.height(),
        );
        match self.edge {
            Edge::Top => gdk::Rectangle::new(x, y, w, self.size),
            Edge::Bottom => gdk::Rectangle::new(x, y + h - self.size, w, self.size),
            Edge::Left => gdk::Rectangle::new(x, y, self.size, h),
            Edge::Right => gdk::Rectangle::new(x + w - self.size, y, self.size, h),
        }
    }
}

pub trait Backend {
    fn name(&self) -> &'static str;

    /// Turns `window` into an edge-anchored surface. Must be called before the
    /// window is realized (that is, before `present()`).
    fn attach(&self, window: &gtk::Window, placement: Placement) -> anyhow::Result<()>;

    /// Keeps `window`, once attached, on the monitor `spec` names: back on
    /// it when it shows up (after the surface, or plugged in again), on
    /// another while it is gone. By default it stays where it was put.
    fn follow(&self, _window: &gtk::Window, _spec: &MonitorSpec) {}
}

/// The monitor `spec` names, if it is there.
pub fn find_monitor(display: &gdk::Display, spec: &MonitorSpec) -> Option<gdk::Monitor> {
    let monitors = display.monitors();
    let pick = |i: u32| {
        monitors
            .item(i)
            .and_then(|obj| obj.downcast::<gdk::Monitor>().ok())
    };
    match spec {
        MonitorSpec::Index(index) => pick(*index as u32),
        MonitorSpec::Name(name) => (0..monitors.n_items())
            .filter_map(pick)
            .find(|m| m.connector().as_deref() == Some(name.as_str())),
    }
}

/// Looks up a monitor by index or output name, falling back to the first one.
pub fn monitor(display: &gdk::Display, spec: &MonitorSpec) -> anyhow::Result<gdk::Monitor> {
    if let Some(monitor) = find_monitor(display, spec) {
        return Ok(monitor);
    }
    log::warn!("monitor {spec:?} not found, using monitor 0");
    display
        .monitors()
        .item(0)
        .and_then(|obj| obj.downcast::<gdk::Monitor>().ok())
        .ok_or_else(|| anyhow::anyhow!("no monitors found"))
}
