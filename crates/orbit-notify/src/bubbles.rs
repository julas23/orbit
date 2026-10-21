//! The bubbles: one surface in the screen's top-right corner holding the
//! cards one under the other, newest on top. Hidden when there are none.
//!
//! Each bubble has its time; the pointer over it holds the time back, as in
//! the dunst this replaces.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::config::Config;

const TICK: Duration = Duration::from_millis(250);

struct Shown {
    id: u32,
    widget: gtk::Widget,
    /// Set when the bubble goes, so its timer stops.
    gone: Rc<Cell<bool>>,
}

pub struct Bubbles {
    window: gtk::Window,
    column: gtk::Box,
    shown: RefCell<Vec<Shown>>,
    max: usize,
    /// The centre is open: bubbles would lie over it.
    hidden: Cell<bool>,
}

impl Bubbles {
    pub fn new(config: &Config, monitor: Option<&gdk::Monitor>) -> Bubbles {
        let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build();
        column.set_size_request(config.width, -1);
        // Not an application window: the centre must stay the app's first
        // window, the one a second launch shows or hides.
        let window = gtk::Window::builder().css_classes(["orbit", "orbit-notify-bubbles"]).decorated(false).child(&column).build();
        window.init_layer_shell();
        window.set_namespace(Some("orbit-notify"));
        window.set_layer(Layer::Overlay);
        window.set_monitor(monitor);
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Right, true);
        window.set_margin(Edge::Top, config.margin);
        window.set_margin(Edge::Right, config.margin);
        window.set_keyboard_mode(KeyboardMode::None);
        Bubbles { window, column, shown: RefCell::default(), max: config.max_popups.max(1), hidden: Cell::new(false) }
    }

    pub fn is_full(&self) -> bool {
        self.shown.borrow().len() >= self.max
    }

    pub fn has(&self, id: u32) -> bool {
        self.shown.borrow().iter().any(|s| s.id == id)
    }

    /// Shows `card` for notification `id` — in place of its bubble when it
    /// has one — for `ms` (0: until closed); `expired` is called when the
    /// time is up.
    pub fn show(&self, id: u32, card: &gtk::Box, ms: u32, expired: impl Fn() + 'static) {
        let gone = Rc::new(Cell::new(false));
        let widget: gtk::Widget = card.clone().upcast();
        {
            let mut shown = self.shown.borrow_mut();
            if let Some(old) = shown.iter_mut().find(|s| s.id == id) {
                old.gone.set(true);
                self.column.insert_child_after(&widget, Some(&old.widget));
                self.column.remove(&old.widget);
                old.widget = widget.clone();
                old.gone = gone.clone();
            } else {
                self.column.prepend(&widget);
                shown.push(Shown { id, widget: widget.clone(), gone: gone.clone() });
            }
        }
        self.window.set_visible(!self.hidden.get());
        if ms > 0 {
            countdown(&widget, ms, gone, expired);
        }
    }

    pub fn remove(&self, id: u32) {
        let mut shown = self.shown.borrow_mut();
        if let Some(i) = shown.iter().position(|s| s.id == id) {
            let s = shown.remove(i);
            s.gone.set(true);
            self.column.remove(&s.widget);
        }
        if shown.is_empty() {
            self.window.set_visible(false);
        }
    }

    /// Hides the bubbles while the centre is open, and brings back those
    /// still there when it closes.
    pub fn set_hidden(&self, hidden: bool) {
        self.hidden.set(hidden);
        self.window.set_visible(!hidden && !self.shown.borrow().is_empty());
    }

    /// Takes every bubble down (do not disturb turned on).
    pub fn clear(&self, keep: impl Fn(u32) -> bool) {
        let ids: Vec<u32> = self.shown.borrow().iter().map(|s| s.id).filter(|id| !keep(*id)).collect();
        for id in ids {
            self.remove(id);
        }
    }
}

/// Calls `expired` after `ms` of the pointer away from `widget`.
fn countdown(widget: &gtk::Widget, ms: u32, gone: Rc<Cell<bool>>, expired: impl Fn() + 'static) {
    let inside = Rc::new(Cell::new(false));
    let motion = gtk::EventControllerMotion::new();
    {
        let inside = inside.clone();
        motion.connect_enter(move |_, _, _| inside.set(true));
    }
    {
        let inside = inside.clone();
        motion.connect_leave(move |_| inside.set(false));
    }
    widget.add_controller(motion);
    let mut left = Duration::from_millis(ms as u64);
    glib::timeout_add_local(TICK, move || {
        if gone.get() {
            return glib::ControlFlow::Break;
        }
        if inside.get() {
            return glib::ControlFlow::Continue;
        }
        if left <= TICK {
            expired();
            return glib::ControlFlow::Break;
        }
        left -= TICK;
        glib::ControlFlow::Continue
    });
}
