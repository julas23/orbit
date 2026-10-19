//! Calendar module, after the calendar of the eww panel.
//!
//! Block form (vertical panel): a weekday header and the month as a 7×6 grid
//! — always six weeks, so the card never changes height — with thin lines
//! around the days, like a paper calendar. The header has no lines.
//! Line form (horizontal panel): the day of the month; the same grid opens
//! in a popover on click.
//!
//! Colour carries the meaning: plain days white, weekends grey, today orange;
//! in the header, Saturday and Sunday red.
//!
//! The lines are drawn with cairo at the real edges of the day cells instead
//! of CSS borders, which would add their width to every cell.

use std::rc::Rc;

use gtk::{glib, prelude::*};
use serde::Deserialize;

/// `modules/calendar.yaml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CalendarConfig {
    pub week_start: WeekStart,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WeekStart {
    #[default]
    Monday,
    Sunday,
}

impl WeekStart {
    /// The ISO weekday (1 = Monday … 7 = Sunday) shown in column `col`.
    fn weekday(self, col: usize) -> i32 {
        let col = col as i32;
        match self {
            WeekStart::Monday => col + 1,
            WeekStart::Sunday if col == 0 => 7,
            WeekStart::Sunday => col,
        }
    }

    fn is_weekend(self, col: usize) -> bool {
        self.weekday(col) >= 6
    }
}

pub fn new(config: &CalendarConfig, vertical: bool) -> gtk::Widget {
    let month = Rc::new(Month::new(config.week_start));

    let root: gtk::Widget = if vertical {
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["card", "calendar"])
            .build();
        card.append(&month.root);
        card.upcast()
    } else {
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["card", "calendar", "popup"])
            .build();
        card.append(&month.root);
        let popover = gtk::Popover::builder().child(&card).build();

        let button = gtk::Button::builder()
            .css_classes(["calendar-line"])
            .child(&month.today)
            .build();
        popover.set_parent(&button);
        button.connect_clicked(move |_| popover.popup());
        button.upcast()
    };

    month.fill();
    // Checked every minute rather than timed to midnight: GLib timers run on
    // the monotonic clock, which stops during suspend, so a timer aimed at
    // midnight would fire late after a night asleep.
    let day = std::cell::Cell::new(today().map(|d| d.ymd()));
    glib::timeout_add_seconds_local(60, move || {
        let now = today().map(|d| d.ymd());
        if now != day.get() {
            day.set(now);
            month.fill();
        }
        glib::ControlFlow::Continue
    });
    root
}

fn today() -> Option<glib::DateTime> {
    glib::DateTime::now_local().ok()
}

struct Month {
    root: gtk::Box,
    week_start: WeekStart,
    days: Vec<gtk::Label>,
    /// The day of the month, for the line form.
    today: gtk::Label,
}

impl Month {
    fn new(week_start: WeekStart) -> Self {
        let head = gtk::Grid::builder().column_homogeneous(true).build();
        for col in 0..7 {
            let mut classes = vec!["head"];
            if week_start.is_weekend(col) {
                classes.push("weekend");
            }
            let label = gtk::Label::builder()
                .label(initial(week_start.weekday(col)))
                .css_classes(classes)
                .hexpand(true)
                .build();
            head.attach(&label, col as i32, 0, 1, 1);
        }

        let grid = gtk::Grid::builder()
            .column_homogeneous(true)
            .row_homogeneous(true)
            .build();
        let days: Vec<gtk::Label> = (0..42)
            .map(|i| {
                let label = gtk::Label::builder().hexpand(true).build();
                grid.attach(&label, i % 7, i / 7, 1, 1);
                label
            })
            .collect();

        // The lines go behind the days, at the edges of their cells.
        let lines = gtk::DrawingArea::builder().css_classes(["lines"]).build();
        let cells = days.clone();
        lines.set_draw_func(move |area, cr, _, _| draw_lines(area, cr, &cells));
        let overlay = gtk::Overlay::builder().child(&lines).build();
        overlay.add_overlay(&grid);
        overlay.set_measure_overlay(&grid, true);

        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        root.append(&head);
        root.append(&overlay);

        Self {
            root,
            week_start,
            days,
            today: gtk::Label::new(None),
        }
    }

    /// Lays out the current month.
    fn fill(&self) {
        let Some(now) = today() else { return };
        let (year, month, day) = now.ymd();
        let Ok(first) = glib::DateTime::from_local(year, month, 1, 0, 0, 0.0) else {
            return;
        };
        let length = first
            .add_months(1)
            .and_then(|next| next.add_days(-1))
            .map_or(31, |last| last.day_of_month());
        // Empty cells before the 1st.
        let offset = (0..7)
            .find(|&col| self.week_start.weekday(col) == first.day_of_week())
            .unwrap_or(0);

        for (i, label) in self.days.iter().enumerate() {
            let n = i as i32 - offset as i32 + 1;
            let in_month = (1..=length).contains(&n);
            label.set_label(&if in_month {
                n.to_string()
            } else {
                String::new()
            });
            let mut classes = vec!["day"];
            if in_month && n == day {
                classes.push("today");
            } else if self.week_start.is_weekend(i % 7) {
                classes.push("weekend");
            }
            label.set_css_classes(&classes);
        }
        self.today.set_label(&day.to_string());
        self.root.queue_draw();
    }
}

/// First letter of a weekday's name in the current locale: S T Q Q S S D in
/// Portuguese, M T W T F S S in English.
fn initial(weekday: i32) -> String {
    // 1 January 2024 was a Monday, so day `weekday` of that week is that weekday.
    glib::DateTime::from_local(2024, 1, weekday, 12, 0, 0.0)
        .ok()
        .and_then(|d| d.format("%a").ok())
        .and_then(|name| name.chars().next())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default()
}

/// 1 px lines on the edges of the day cells, in the area's CSS colour.
fn draw_lines(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, cells: &[gtk::Label]) {
    // (x, y, width, height) of cell `i`, in the area's coordinates.
    let bounds = |i: usize| {
        cells[i].compute_bounds(area).map(|b| {
            (
                f64::from(b.x()).round(),
                f64::from(b.y()).round(),
                f64::from(b.width()),
                f64::from(b.height()),
            )
        })
    };
    let (Some(first), Some(last)) = (bounds(0), bounds(41)) else {
        return;
    };
    let (left, top) = (first.0, first.1);
    let right = (last.0 + last.2).round() - 1.0;
    let bottom = (last.1 + last.3).round() - 1.0;

    let color = area.color();
    cr.set_source_rgba(
        color.red().into(),
        color.green().into(),
        color.blue().into(),
        color.alpha().into(),
    );
    // A column's left edge, then the right edge of the last column.
    for x in (0..7)
        .filter_map(|col| bounds(col).map(|b| b.0))
        .chain([right])
    {
        cr.rectangle(x, top, 1.0, bottom - top + 1.0);
    }
    for y in (0..6)
        .filter_map(|row| bounds(row * 7).map(|b| b.1))
        .chain([bottom])
    {
        cr.rectangle(left, y, right - left + 1.0, 1.0);
    }
    let _ = cr.fill();
}
