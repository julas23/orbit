//! X11 backend.
//!
//! GTK4 dropped the X11 window-positioning and type-hint APIs, so this backend
//! talks to the X server directly through `x11rb`: it marks the window as a
//! dock (`_NET_WM_WINDOW_TYPE_DOCK`), reserves screen space with
//! `_NET_WM_STRUT_PARTIAL` and moves it into place. Window managers that
//! follow EWMH (bspwm included) keep dock windows on every desktop, above
//! normal windows, and lay tiled windows out around their struts.

use std::rc::Rc;

use anyhow::Context;
use gdk4_x11::X11Surface;
use gtk::{gdk, prelude::*};
use orbit_common::{Backend, Edge, Placement, backend::monitor};
use x11rb::{
    connection::Connection,
    protocol::xproto::{AtomEnum, ConfigureWindowAux, ConnectionExt as _, PropMode},
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_DOCK,
        _NET_WM_STRUT,
        _NET_WM_STRUT_PARTIAL,
    }
}

struct Inner {
    conn: RustConnection,
    screen: usize,
    atoms: Atoms,
}

pub struct X11Backend {
    inner: Rc<Inner>,
}

impl X11Backend {
    pub fn new() -> anyhow::Result<Self> {
        let (conn, screen) = x11rb::connect(None).context("connecting to the X server")?;
        let atoms = Atoms::new(&conn)?.reply()?;
        Ok(Self {
            inner: Rc::new(Inner {
                conn,
                screen,
                atoms,
            }),
        })
    }
}

impl Backend for X11Backend {
    fn name(&self) -> &'static str {
        "x11"
    }

    fn attach(&self, window: &gtk::Window, placement: Placement) -> anyhow::Result<()> {
        let monitor = monitor(&WidgetExt::display(window), &placement.monitor)?;
        let rect = placement.rect(&monitor.geometry());

        window.set_decorated(false);
        window.set_resizable(false);
        window.set_default_size(rect.width(), rect.height());

        // Hints must be on the window before it is mapped, or the window
        // manager will already have decided to manage it as a normal window.
        // `realize` is the last point where the X window exists but is unmapped.
        let inner = self.inner.clone();
        let scale = monitor.scale_factor();
        window.connect_realize(move |window| {
            let Some(xid) = xid(window) else {
                log::error!("window has no X11 surface; is GDK_BACKEND set to x11?");
                return;
            };
            if let Err(err) = inner.dock(xid, &placement, rect, scale) {
                log::error!("setting dock hints: {err:#}");
            }
        });

        // Some window managers reposition windows on map; put it back.
        let inner = self.inner.clone();
        window.connect_map(move |window| {
            if let Some(xid) = xid(window)
                && let Err(err) = inner.move_to(xid, rect, scale)
            {
                log::error!("positioning window: {err:#}");
            }
        });

        Ok(())
    }
}

fn xid(window: &gtk::Window) -> Option<u32> {
    let surface = window.surface()?.downcast::<X11Surface>().ok()?;
    Some(surface.xid() as u32)
}

impl Inner {
    fn dock(
        &self,
        xid: u32,
        placement: &Placement,
        rect: gdk::Rectangle,
        scale: i32,
    ) -> anyhow::Result<()> {
        let a = &self.atoms;
        let c = &self.conn;
        c.change_property32(
            PropMode::REPLACE,
            xid,
            a._NET_WM_WINDOW_TYPE,
            AtomEnum::ATOM,
            &[a._NET_WM_WINDOW_TYPE_DOCK],
        )?;
        if placement.reserve {
            let strut = self.strut(placement.edge, device(rect, scale));
            c.change_property32(
                PropMode::REPLACE,
                xid,
                a._NET_WM_STRUT_PARTIAL,
                AtomEnum::CARDINAL,
                &strut,
            )?;
            c.change_property32(
                PropMode::REPLACE,
                xid,
                a._NET_WM_STRUT,
                AtomEnum::CARDINAL,
                &strut[..4],
            )?;
        }

        self.move_to(xid, rect, scale)
    }

    fn move_to(&self, xid: u32, rect: gdk::Rectangle, scale: i32) -> anyhow::Result<()> {
        let r = device(rect, scale);
        let aux = ConfigureWindowAux::new()
            .x(r.x())
            .y(r.y())
            .width(r.width() as u32)
            .height(r.height() as u32);
        self.conn.configure_window(xid, &aux)?;
        self.conn.flush()?;
        Ok(())
    }

    /// `_NET_WM_STRUT_PARTIAL`: left, right, top, bottom widths, then the
    /// start/end range each one covers. Struts are measured from the edges of
    /// the whole X screen, not of the monitor.
    fn strut(&self, edge: Edge, r: gdk::Rectangle) -> [u32; 12] {
        let root = &self.conn.setup().roots[self.screen];
        let (root_w, root_h) = (root.width_in_pixels as i32, root.height_in_pixels as i32);
        let (x0, x1) = (r.x() as u32, (r.x() + r.width() - 1) as u32);
        let (y0, y1) = (r.y() as u32, (r.y() + r.height() - 1) as u32);
        let mut s = [0u32; 12];
        match edge {
            Edge::Left => (s[0], s[4], s[5]) = ((r.x() + r.width()) as u32, y0, y1),
            Edge::Right => (s[1], s[6], s[7]) = ((root_w - r.x()) as u32, y0, y1),
            Edge::Top => (s[2], s[8], s[9]) = ((r.y() + r.height()) as u32, x0, x1),
            Edge::Bottom => (s[3], s[10], s[11]) = ((root_h - r.y()) as u32, x0, x1),
        }
        s
    }
}

/// Logical (GDK) pixels to X11 device pixels.
fn device(r: gdk::Rectangle, scale: i32) -> gdk::Rectangle {
    gdk::Rectangle::new(
        r.x() * scale,
        r.y() * scale,
        r.width() * scale,
        r.height() * scale,
    )
}
