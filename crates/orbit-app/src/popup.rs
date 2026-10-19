//! Opening popover menus where the user clicked.
//!
//! GTK points a popover at the middle of its parent widget by default; a
//! context menu should instead open with its top-left corner at the click.

use gtk::{gdk, glib, prelude::*};

/// A click position, in the anchor widget's coordinates.
pub type Point = Option<(f64, f64)>;

/// Parents `popover` to `anchor` and opens it with its top-left corner at
/// `at` (or under the anchor's left edge without a point). The popover is
/// unparented once closed, so it can be opened afresh each time.
pub fn open(popover: &impl IsA<gtk::Popover>, anchor: &impl IsA<gtk::Widget>, at: Point) {
    let popover = popover.as_ref();
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Bottom);
    // With the popover aligned to the start, GTK places its left edge at the
    // left of the pointed rectangle instead of centring it there.
    popover.set_halign(gtk::Align::Start);
    popover.set_parent(anchor);
    if let Some((x, y)) = at {
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    }
    popover.connect_closed(|popover| {
        // After the click that closed it has been handled.
        let popover = popover.clone();
        glib::idle_add_local_once(move || popover.unparent());
    });
    popover.popup();
}
