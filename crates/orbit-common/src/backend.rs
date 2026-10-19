//! Display-server abstraction.
//!
//! GTK4 has no portable API for docking a window to a screen edge: Wayland
//! needs the layer-shell protocol, and X11 needs EWMH hints set directly on
//! the window. Each backend implements [`Backend`] to hide that difference.

use gtk::{gdk, prelude::*};

use crate::Edge;

/// Where and how a surface sits on screen.
#[derive(Debug, Clone, Copy)]
pub struct Placement {
    pub edge: Edge,
    /// Thickness in logical pixels, perpendicular to `edge`.
    pub size: i32,
    pub monitor: usize,
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
}

/// Looks up monitor number `index`, falling back to the first one.
pub fn monitor(display: &gdk::Display, index: usize) -> anyhow::Result<gdk::Monitor> {
    let monitors = display.monitors();
    let pick = |i: usize| {
        monitors
            .item(i as u32)
            .and_then(|obj| obj.downcast::<gdk::Monitor>().ok())
    };
    if let Some(monitor) = pick(index) {
        return Ok(monitor);
    }
    log::warn!("monitor {index} not found, using monitor 0");
    pick(0).ok_or_else(|| anyhow::anyhow!("no monitors found"))
}
