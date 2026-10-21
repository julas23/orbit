//! Maintenance mode: the layout grid drawn behind the panel.
//!
//! Vertical panel: the cells of the grid (see `board`), each with its name —
//! columns a–d, rows from 1, so a1 is the top-left cell — to see and name
//! where every module sits. Horizontal panel: a 5 px grid, every tenth line
//! (50 px) brighter, for checking that sizes, gaps and paddings land on it.

use gtk::prelude::*;

use crate::board;

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
pub fn wrap(content: &impl IsA<gtk::Widget>, vertical: bool) -> gtk::Overlay {
    let grid = gtk::DrawingArea::new();
    if vertical {
        grid.set_draw_func(|_, cr, width, height| cells(cr, width, height));
    } else {
        grid.set_draw_func(|_, cr, width, height| lines(cr, width, height));
    }

    let overlay = gtk::Overlay::builder()
        .css_classes(["maintenance"])
        .child(&grid)
        .build();
    overlay.add_overlay(content);
    // The grid has no size of its own; let the content size the overlay.
    overlay.set_measure_overlay(content, true);
    overlay
}

/// Every cell outlined, its name in its top-left corner.
fn cells(cr: &gtk::cairo::Context, _width: i32, height: i32) {
    let (r, g, b, a) = MAJOR;
    cr.set_line_width(1.0);
    cr.select_font_face(
        "monospace",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Normal,
    );
    cr.set_font_size(9.0);
    for row in 0..board::rows_in(height) {
        for col in 0..board::COLUMNS {
            let x = f64::from(board::SIDE + col * board::PITCH);
            let y = f64::from(board::PAD + row * board::PITCH);
            let side = f64::from(board::CELL);
            // On pixel centres (+0.5) so a 1 px line stays crisp.
            cr.rectangle(x + 0.5, y + 0.5, side - 1.0, side - 1.0);
            cr.set_source_rgba(r, g, b, a);
            let _ = cr.stroke();
            cr.move_to(x + 3.0, y + 11.0);
            cr.set_source_rgba(r, g, b, 0.7);
            if let Err(err) = cr.show_text(&board::cell_name(col, row)) {
                log::warn!("maintenance grid: {err}");
                return;
            }
        }
    }
}

fn lines(cr: &gtk::cairo::Context, width: i32, height: i32) {
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
