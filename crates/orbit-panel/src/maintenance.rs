//! Maintenance mode: a 5 px grid drawn behind the panel, for checking that
//! every size, gap and padding lands on the grid.
//!
//! Every tenth line (50 px) is drawn brighter so distances can be counted
//! at a glance.

use gtk::prelude::*;

/// The layout step: every measure in the panel is a multiple of this.
const STEP: i32 = 5;
/// A brighter line every this many steps.
const MAJOR_EVERY: i32 = 10;

// Over the translucent panel a 10% white line was only ~8 levels brighter
// than the background: the 5 px grid vanished and only the 50 px lines
// showed. The minor lines must be clearly visible on their own.
const MINOR: (f64, f64, f64, f64) = (1.0, 1.0, 1.0, 0.22);
const MAJOR: (f64, f64, f64, f64) = (1.0, 1.0, 1.0, 0.45);

/// Puts `content` on top of the grid.
pub fn wrap(content: &impl IsA<gtk::Widget>) -> gtk::Overlay {
    let grid = gtk::DrawingArea::new();
    grid.set_draw_func(|_, cr, width, height| draw(cr, width, height));

    let overlay = gtk::Overlay::builder()
        .css_classes(["maintenance"])
        .child(&grid)
        .build();
    overlay.add_overlay(content);
    // The grid has no size of its own; let the content size the overlay.
    overlay.set_measure_overlay(content, true);
    overlay
}

fn draw(cr: &gtk::cairo::Context, width: i32, height: i32) {
    cr.set_line_width(1.0);
    // Minor lines first, major on top. Lines sit on pixel centres (+0.5) so
    // a 1 px line stays crisp.
    for (major, (r, g, b, a)) in [(false, MINOR), (true, MAJOR)] {
        let mut x = 0;
        while x <= width {
            if ((x / STEP) % MAJOR_EVERY == 0) == major {
                cr.move_to(f64::from(x) + 0.5, 0.0);
                cr.line_to(f64::from(x) + 0.5, f64::from(height));
            }
            x += STEP;
        }
        let mut y = 0;
        while y <= height {
            if ((y / STEP) % MAJOR_EVERY == 0) == major {
                cr.move_to(0.0, f64::from(y) + 0.5);
                cr.line_to(f64::from(width), f64::from(y) + 0.5);
            }
            y += STEP;
        }
        cr.set_source_rgba(r, g, b, a);
        if let Err(err) = cr.stroke() {
            log::warn!("maintenance grid: {err}");
            return;
        }
    }
}
