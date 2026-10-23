//! Moving the panel or the dock to another edge or monitor, from its
//! editor's menu.
//!
//! Every monitor is covered, dimmed, with the edges the surface may take
//! drawn on it, its current place among them. The edge nearest the pointer
//! lights up; a click puts the surface there, and so does a drag let go
//! over it — on any monitor, as the drag carries on past the one it began
//! on. Escape or a right click leaves it where it was.
//!
//! The choice is written to the surface's file (`edge:`, `monitor:`) and
//! the panel and the dock are started again (`orbit-session`), so each
//! comes up laid out for its new edge, the panel first.

use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge as LayerEdge, KeyboardMode, Layer, LayerShell};
use orbit_common::{ConfigDir, Edge};

use crate::i18n::tr;

/// The band each edge is drawn as, at least this thick.
const BAND: f64 = 48.0;

/// What is being moved.
pub struct Surface {
    /// Its file in the config directory (`panel.yaml`).
    pub file: &'static str,
    /// The edges it may take.
    pub edges: &'static [Edge],
    /// Its thickness, for the bands drawn.
    pub size: i32,
    /// Its edge now.
    pub edge: Edge,
    /// Its monitor's name now, found by [`start`].
    pub monitor: Option<String>,
}

/// Whether surfaces can be moved here: on Wayland, with layer-shell.
pub fn supported() -> bool {
    gdk::Display::default().is_some_and(|d| d.is::<gdk4_wayland::WaylandDisplay>())
        && gtk4_layer_shell::is_supported()
}

struct Mover {
    surface: Surface,
    dir: ConfigDir,
    /// One per monitor, with the monitor.
    covers: RefCell<Vec<(gtk::Window, gdk::Monitor, gtk::DrawingArea)>>,
    /// The monitor's name and the edge under the pointer.
    target: RefCell<Option<(String, Edge)>>,
    /// Where a drag began, in global coordinates.
    from: RefCell<Option<(f64, f64)>>,
}

/// Covers every monitor and lets the user pick where `surface`, shown by
/// `widget`, goes.
pub fn start(mut surface: Surface, widget: &impl IsA<gtk::Widget>, dir: &ConfigDir) {
    let Some(display) = gdk::Display::default() else { return };
    surface.monitor = widget
        .root()
        .and_then(|root| root.surface())
        .and_then(|s| display.monitor_at_surface(&s))
        .and_then(|m| m.connector())
        .map(|c| c.to_string());
    let mover = Rc::new(Mover {
        surface,
        dir: dir.clone(),
        covers: RefCell::new(Vec::new()),
        target: RefCell::new(None),
        from: RefCell::new(None),
    });
    for monitor in display.monitors().iter::<gdk::Monitor>().flatten() {
        cover(&mover, &monitor);
    }
}

fn cover(mover: &Rc<Mover>, monitor: &gdk::Monitor) {
    let name = monitor.connector().map(|c| c.to_string()).unwrap_or_default();
    let area = gtk::DrawingArea::builder().hexpand(true).vexpand(true).build();
    {
        let weak = Rc::downgrade(mover);
        let name = name.clone();
        area.set_draw_func(move |area, cr, w, h| {
            if let Some(mover) = weak.upgrade() {
                draw(&mover, area, cr, &name, w as f64, h as f64);
            }
        });
    }

    let hint = gtk::Label::builder()
        .label(tr("mover-hint"))
        .css_classes(["mover-hint"])
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .can_target(false)
        .build();
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&area));
    overlay.add_overlay(&hint);

    let window = gtk::Window::builder()
        .title("Orbit-Mover")
        .css_classes(["orbit", "orbit-mover"])
        .child(&overlay)
        .build();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("orbit-mover"));
    window.set_monitor(Some(monitor));
    for edge in [LayerEdge::Top, LayerEdge::Bottom, LayerEdge::Left, LayerEdge::Right] {
        window.set_anchor(edge, true);
    }
    window.set_exclusive_zone(-1);
    if mover.surface.monitor.as_deref() == Some(name.as_str()) {
        window.set_keyboard_mode(KeyboardMode::Exclusive);
    }

    let origin = monitor.geometry();
    let (ox, oy) = (origin.x() as f64, origin.y() as f64);

    let motion = gtk::EventControllerMotion::new();
    {
        let weak = Rc::downgrade(mover);
        motion.connect_motion(move |_, x, y| {
            if let Some(mover) = weak.upgrade()
                && mover.from.borrow().is_none()
            {
                mover.aim(ox + x, oy + y);
            }
        });
    }
    area.add_controller(motion);

    // A press, dragged or not, and let go: the place under it.
    let drag = gtk::GestureDrag::new();
    drag.set_button(gdk::BUTTON_PRIMARY);
    {
        let weak = Rc::downgrade(mover);
        drag.connect_drag_begin(move |_, x, y| {
            let Some(mover) = weak.upgrade() else { return };
            *mover.from.borrow_mut() = Some((ox + x, oy + y));
            mover.aim(ox + x, oy + y);
        });
    }
    {
        let weak = Rc::downgrade(mover);
        drag.connect_drag_update(move |_, dx, dy| {
            let Some(mover) = weak.upgrade() else { return };
            let Some((x, y)) = *mover.from.borrow() else { return };
            mover.aim(x + dx, y + dy);
        });
    }
    {
        let weak = Rc::downgrade(mover);
        drag.connect_drag_end(move |_, dx, dy| {
            let Some(mover) = weak.upgrade() else { return };
            let Some((x, y)) = mover.from.borrow_mut().take() else { return };
            mover.aim(x + dx, y + dy);
            mover.pick();
        });
    }
    area.add_controller(drag);

    let cancel = gtk::GestureClick::new();
    cancel.set_button(gdk::BUTTON_SECONDARY);
    {
        let weak = Rc::downgrade(mover);
        cancel.connect_pressed(move |_, _, _, _| {
            if let Some(mover) = weak.upgrade() {
                mover.close();
            }
        });
    }
    area.add_controller(cancel);

    let keys = gtk::EventControllerKey::new();
    {
        let weak = Rc::downgrade(mover);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key != gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            if let Some(mover) = weak.upgrade() {
                mover.close();
            }
            glib::Propagation::Stop
        });
    }
    window.add_controller(keys);

    // The covers hold the mover until they are closed.
    let keep = mover.clone();
    window.connect_destroy(move |_| {
        let _ = &keep;
    });
    window.present();
    mover.covers.borrow_mut().push((window, monitor.clone(), area));
}

impl Mover {
    /// The monitor and edge under (`x`, `y`), global coordinates.
    fn aim(&self, x: f64, y: f64) {
        let target = self.covers.borrow().iter().find_map(|(_, monitor, _)| {
            let g = monitor.geometry();
            let (lx, ly) = (x - g.x() as f64, y - g.y() as f64);
            let (w, h) = (g.width() as f64, g.height() as f64);
            if lx < 0.0 || ly < 0.0 || lx >= w || ly >= h {
                return None;
            }
            let edge = nearest(self.surface.edges, lx, ly, w, h)?;
            Some((monitor.connector()?.to_string(), edge))
        });
        // Off every monitor (a drag past the last), the last edge stays.
        if target.is_some() && *self.target.borrow() != target {
            *self.target.borrow_mut() = target;
            for (_, _, area) in self.covers.borrow().iter() {
                area.queue_draw();
            }
        }
    }

    /// Puts the surface where the target is.
    fn pick(&self) {
        let target = self.target.borrow().clone();
        self.close();
        let Some((monitor, edge)) = target else { return };
        if self.surface.monitor.as_deref() == Some(monitor.as_str()) && self.surface.edge == edge {
            return;
        }
        log::info!("moving to the {} edge of {monitor}", edge.name());
        if let Err(err) = self.dir.save_place(self.surface.file, edge, &monitor) {
            log::error!("{err:#}");
            return;
        }
        restart();
    }

    fn close(&self) {
        for (window, _, _) in self.covers.borrow_mut().drain(..) {
            window.close();
        }
    }
}

/// The edge of `edges` nearest (`x`, `y`) on a `w` × `h` monitor.
fn nearest(edges: &[Edge], x: f64, y: f64, w: f64, h: f64) -> Option<Edge> {
    let distance = |edge: &Edge| match edge {
        Edge::Left => x,
        Edge::Right => w - x,
        Edge::Top => y,
        Edge::Bottom => h - y,
    };
    edges.iter().copied().min_by(|a, b| distance(a).total_cmp(&distance(b)))
}

/// The rectangle edge `edge`'s band takes on a `w` × `h` monitor.
fn band(edge: Edge, thick: f64, w: f64, h: f64) -> (f64, f64, f64, f64) {
    match edge {
        Edge::Left => (0.0, 0.0, thick, h),
        Edge::Right => (w - thick, 0.0, thick, h),
        Edge::Top => (0.0, 0.0, w, thick),
        Edge::Bottom => (0.0, h - thick, w, thick),
    }
}

fn draw(mover: &Mover, area: &gtk::DrawingArea, cr: &gtk::cairo::Context, name: &str, w: f64, h: f64) {
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.45);
    let _ = cr.paint();
    let accent = area.color();
    let thick = (mover.surface.size as f64).max(BAND);
    let target = mover.target.borrow();
    for &edge in mover.surface.edges {
        let (x, y, bw, bh) = band(edge, thick, w, h);
        let current = mover.surface.monitor.as_deref() == Some(name) && mover.surface.edge == edge;
        let aimed = target.as_ref().is_some_and(|(m, e)| m == name && *e == edge);
        let fill = if aimed {
            0.45
        } else if current {
            0.2
        } else {
            0.06
        };
        cr.set_source_rgba(accent.red() as f64, accent.green() as f64, accent.blue() as f64, fill);
        cr.rectangle(x + 4.0, y + 4.0, bw - 8.0, bh - 8.0);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(accent.red() as f64, accent.green() as f64, accent.blue() as f64, if aimed { 0.9 } else { 0.35 });
        cr.set_line_width(2.0);
        if current && !aimed {
            cr.set_dash(&[8.0, 6.0], 0.0);
        } else {
            cr.set_dash(&[], 0.0);
        }
        let _ = cr.stroke();
    }
}

/// Starts the panel and the dock again, from their files: `orbit-session`
/// keeps their order; without it, this program alone, in place.
fn restart() {
    let session = std::process::Command::new("orbit-session")
        .stdin(std::process::Stdio::null())
        .spawn();
    if session.is_ok() {
        return;
    }
    use std::os::unix::process::CommandExt;
    let Ok(exe) = std::env::current_exe() else { return };
    let err = std::process::Command::new(exe).args(std::env::args_os().skip(1)).exec();
    log::error!("restarting: {err}");
}
