//! Calendar module, after the calendar of the eww panel.
//!
//! Full (4 × 4 cells): the season and the year on top — the season of the
//! hemisphere the weather module's place is in, or of `latitude` in
//! `calendar.yaml` — then a weekday header and the month as a 7×6 grid
//! — always six weeks, so the card never changes height — with thin lines
//! around the days, like a paper calendar. The header has no lines.
//! Normal (4 × 2): the same title and header over two weeks, this one and
//! the next. Mini (3 × 1): weekday, day and month, "qua 21 out". Line form
//! (horizontal panel): the day of the month. Mini, Normal and the line form
//! open the whole month in a popover on click.
//!
//! Colour carries the meaning: plain days white, weekends grey, today orange;
//! in the header, Saturday and Sunday red.
//!
//! The lines are drawn with cairo at the real edges of the day cells instead
//! of CSS borders, which would add their width to every cell.

use std::{cell::Cell, rc::Rc};

use gtk::{glib, prelude::*};
use orbit_app::i18n::{text, tr_args};
use serde::Deserialize;

use crate::{
    board::Form,
    weather_data::{self, WeatherConfig},
};

/// `modules/calendar.yaml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CalendarConfig {
    pub week_start: WeekStart,
    /// Where the season is reckoned (only its sign counts: south of the
    /// equator the seasons are swapped). By default, the weather module's
    /// place.
    pub latitude: Option<f64>,
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

pub fn new(config: &CalendarConfig, weather: Option<&WeatherConfig>, form: Form) -> gtk::Widget {
    let weeks = if form == Form::Normal { 2 } else { 6 };
    let month = Rc::new(Month::new(config.week_start, weeks));
    // The whole month, in the popover of the smaller forms.
    let whole = (form != Form::Full).then(|| Rc::new(Month::new(config.week_start, 6)));
    let months: Vec<Rc<Month>> = [Some(month.clone()), whole.clone()].into_iter().flatten().collect();
    for m in &months {
        m.latitude.set(config.latitude);
    }
    if config.latitude.is_none()
        && let Some(weather) = weather
    {
        let months = months.clone();
        weather_data::subscribe(weather, move |w| {
            for month in &months {
                if month.latitude.replace(Some(w.latitude)) != Some(w.latitude) {
                    month.fill();
                }
            }
        });
    }

    let card = |classes: &[&str], child: &gtk::Box| {
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(classes)
            .build();
        card.append(child);
        card
    };
    // Mini and Normal are cards, not buttons: the panel's buttons have no
    // border, and the card's outline is part of its size on the grid.
    let root: gtk::Widget = match form {
        Form::Full => card(&["card", "calendar", "full"], &month.root).upcast(),
        Form::Normal => {
            let card = card(&["card", "calendar", "normal"], &month.root);
            month_popover(&card, whole.as_deref().expect("made for this form"));
            card.upcast()
        }
        Form::Mini => {
            let card = gtk::Box::builder()
                .css_classes(["card", "calendar-mini", "mini"])
                .build();
            month.short.set_hexpand(true);
            card.append(&month.short);
            month_popover(&card, whole.as_deref().expect("made for this form"));
            card.upcast()
        }
        Form::Line => {
            let button = gtk::Button::builder()
                .css_classes(["calendar-line"])
                .child(&month.today)
                .build();
            month_popover(&button, whole.as_deref().expect("made for this form"));
            button.upcast()
        }
    };

    let tooltip_month = month.clone();
    root.set_has_tooltip(form != Form::Full);
    root.connect_query_tooltip(move |_, _, _, _, tooltip| {
        tooltip.set_text(Some(&tooltip_month.tooltip()));
        true
    });

    for m in &months {
        m.fill();
    }
    // Checked every minute rather than timed to midnight: GLib timers run on
    // the monotonic clock, which stops during suspend, so a timer aimed at
    // midnight would fire late after a night asleep.
    let day = std::cell::Cell::new(today().map(|d| d.ymd()));
    glib::timeout_add_seconds_local(60, move || {
        let now = today().map(|d| d.ymd());
        if now != day.get() {
            day.set(now);
            for m in &months {
                m.fill();
            }
        }
        glib::ControlFlow::Continue
    });
    root
}

fn today() -> Option<glib::DateTime> {
    glib::DateTime::now_local().ok()
}

/// Opens `month` in a popover under `anchor` on a left click.
fn month_popover(anchor: &impl IsA<gtk::Widget>, month: &Month) {
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["card", "calendar", "popup"])
        .build();
    card.append(&month.root);
    let popover = gtk::Popover::builder().child(&card).build();
    popover.set_parent(anchor);
    let click = gtk::GestureClick::builder().button(1).build();
    click.connect_released(move |_, _, _, _| popover.popup());
    anchor.add_controller(click);
    anchor.set_cursor_from_name(Some("pointer"));
}

struct Month {
    root: gtk::Box,
    /// Season and year.
    title: gtk::Label,
    latitude: Cell<Option<f64>>,
    week_start: WeekStart,
    /// 6: the month's weeks; fewer: from this week on.
    weeks: usize,
    days: Vec<gtk::Label>,
    /// The day of the month, for the line form.
    today: gtk::Label,
    /// Weekday, day and month, for the mini form.
    short: gtk::Label,
}

impl Month {
    fn new(week_start: WeekStart, weeks: usize) -> Self {
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
        let days: Vec<gtk::Label> = (0..weeks as i32 * 7)
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
        // The weeks take the height the card has left.
        let overlay = gtk::Overlay::builder().child(&lines).vexpand(true).build();
        overlay.add_overlay(&grid);
        overlay.set_measure_overlay(&grid, true);

        let title = gtk::Label::builder().css_classes(["title"]).build();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        root.append(&title);
        root.append(&head);
        root.append(&overlay);

        Self {
            root,
            title,
            latitude: Cell::new(None),
            week_start,
            weeks,
            days,
            today: gtk::Label::new(None),
            short: gtk::Label::new(None),
        }
    }

    /// Lays out the current month, or the weeks from this one on.
    fn fill(&self) {
        let Some(now) = today() else { return };
        let (year, month, day) = now.ymd();
        let Ok(first) = glib::DateTime::from_local(year, month, 1, 0, 0, 0.0) else {
            return;
        };
        // The first day shown and how far today is from the start of its
        // week.
        let column = |date: &glib::DateTime| {
            (0..7)
                .find(|&col| self.week_start.weekday(col) == date.day_of_week())
                .unwrap_or(0) as i32
        };
        let start = if self.weeks >= 6 {
            first.add_days(-column(&first))
        } else {
            now.add_days(-column(&now))
        };
        let Ok(start) = start else { return };

        for (i, label) in self.days.iter().enumerate() {
            let Ok(date) = start.add_days(i as i32) else { continue };
            let in_month = date.month() == month;
            // The month's grid leaves other months' days empty; the weeks
            // from today on show them all.
            let shown = in_month || self.weeks < 6;
            label.set_label(&if shown {
                date.day_of_month().to_string()
            } else {
                String::new()
            });
            let mut classes = vec!["day"];
            if date.ymd() == (year, month, day) {
                classes.push("today");
            } else if self.week_start.is_weekend(i % 7) {
                classes.push("weekend");
            }
            if !in_month && shown {
                classes.push("other");
            }
            label.set_css_classes(&classes);
        }
        self.today.set_label(&day.to_string());
        self.short.set_label(
            &now.format("%a %-d %b")
                .map(|s| s.replace('.', ""))
                .unwrap_or_default(),
        );
        // Until the place is known, the year alone.
        self.title.set_label(&match self.latitude.get() {
            Some(lat) => tr_args(
                "calendar-title",
                &[
                    ("season", text(capitalize(&weather_data::season(month, day, lat)))),
                    ("year", text(year.to_string())),
                ],
            ),
            None => year.to_string(),
        });
        self.root.queue_draw();
    }

    /// The full date and the season, for the forms that leave them out.
    fn tooltip(&self) -> String {
        let date = today()
            .and_then(|now| now.format("%A, %-d %B %Y").ok())
            .map(|s| capitalize(&s))
            .unwrap_or_default();
        format!("{date}\n{}", self.title.label())
    }
}

/// "primavera" → "Primavera".
fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
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
    let (Some(first), Some(last)) = (bounds(0), bounds(cells.len().saturating_sub(1))) else {
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
    for y in (0..cells.len() / 7)
        .filter_map(|row| bounds(row * 7).map(|b| b.1))
        .chain([bottom])
    {
        cr.rectangle(left, y, right - left + 1.0, 1.0);
    }
    let _ = cr.fill();
}
