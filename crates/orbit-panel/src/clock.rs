//! Clock module, after the clock of the eww panel.
//!
//! Block form (vertical panel), three lines:
//!   1. the date, centred;
//!   2. local zone + `HH:MM`, centred, with the hourglass at the right edge;
//!   3. one world clock at a time — code left, time centred, difference right.
//!
//! Line form (horizontal panel): zone + `HH:MM` + hourglass; the date and the
//! world clocks go to the tooltip.
//!
//! The hourglass is a bar that EMPTIES over the minute — full at :00, empty
//! at :59 — because time runs out, it does not add up. It is drawn against
//! the ink of the digits, read from the time label's layout, so it lines up
//! with the numbers in any font.

use std::{cell::Cell, rc::Rc, time::Duration};

use gtk::{glib, graphene, prelude::*};
use orbit_app::i18n::{number, tr_args};
use serde::Deserialize;

/// How many other time zones the clock can show.
const MAX_WORLD_CLOCKS: usize = 4;

/// `modules/clock.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClockConfig {
    /// Replaces the local zone abbreviation shown before the time.
    pub label: Option<String>,
    /// Other time zones, in display order. Entries past the fourth are ignored.
    pub world: Vec<WorldClock>,
    /// Seconds each world clock stays on screen.
    pub rotate: u32,
}

impl Default for ClockConfig {
    fn default() -> Self {
        Self {
            label: None,
            world: Vec::new(),
            rotate: 5,
        }
    }
}

/// A time zone for the clock.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldClock {
    /// Short code shown on the panel, e.g. an airport code (`LHR`).
    pub code: String,
    /// Full name for the tooltip; defaults to `code`.
    pub name: Option<String>,
    /// IANA time-zone identifier, e.g. `Europe/London`.
    pub tz: String,
}

/// Opacity of the hourglass trough — the part of the minute already gone —
/// drawn in the time's text colour, so it shows on light and dark themes.
const TROUGH_ALPHA: f64 = 0.15;

struct Zone {
    code: String,
    name: String,
    tz: glib::TimeZone,
}

struct Labels {
    date: Option<gtk::Label>,
    zone: gtk::Label,
    time: gtk::Label,
    world: Option<(gtk::Label, gtk::Label, gtk::Label)>,
    hourglass: gtk::DrawingArea,
    root: gtk::Widget,
}

pub fn new(config: &ClockConfig, vertical: bool) -> gtk::Widget {
    let zones = zones(config);
    let second = Rc::new(Cell::new(0));

    let zone = gtk::Label::builder()
        .css_classes(["zone"])
        .valign(gtk::Align::BaselineFill)
        .build();
    let time = gtk::Label::builder()
        .css_classes(["time"])
        .valign(gtk::Align::BaselineFill)
        .build();
    let hourglass = hourglass(&time, second.clone());

    // Zone and time travel together; the hourglass keeps to the edge.
    let now = gtk::Box::builder().spacing(5).build();
    now.append(&zone);
    now.append(&time);

    let labels = if vertical {
        now.set_hexpand(true);
        now.set_halign(gtk::Align::Center);
        let row = gtk::Box::builder().spacing(5).build();
        row.append(&now);
        row.append(&hourglass);

        let date = gtk::Label::builder()
            .css_classes(["date"])
            .halign(gtk::Align::Center)
            .build();
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["card", "clock"])
            .build();
        card.append(&date);
        card.append(&row);

        let world = (!zones.is_empty()).then(|| {
            let code = label("world-code", gtk::Align::Start);
            let wtime = label("world-time", gtk::Align::Center);
            let diff = label("world-diff", gtk::Align::End);
            let line = gtk::CenterBox::builder()
                .start_widget(&code)
                .center_widget(&wtime)
                .end_widget(&diff)
                .build();
            card.append(&line);
            (code, wtime, diff)
        });
        Labels {
            date: Some(date),
            zone,
            time,
            world,
            hourglass,
            root: card.upcast(),
        }
    } else {
        now.append(&hourglass);
        now.add_css_class("clock");
        Labels {
            date: None,
            zone,
            time,
            world: None,
            hourglass,
            root: now.upcast(),
        }
    };

    let state = Rc::new(State {
        labels,
        zones,
        label: config.label.clone(),
        rotate: i64::from(config.rotate.max(1)),
        second,
    });
    state.update();
    schedule(state.clone());
    state.labels.root.clone()
}

fn label(class: &str, halign: gtk::Align) -> gtk::Label {
    gtk::Label::builder()
        .css_classes([class])
        .halign(halign)
        .build()
}

/// The configured world clocks, at most [`MAX_WORLD_CLOCKS`], skipping
/// unknown time zones.
fn zones(config: &ClockConfig) -> Vec<Zone> {
    if config.world.len() > MAX_WORLD_CLOCKS {
        log::warn!(
            "clock: {} world clocks configured, showing the first {MAX_WORLD_CLOCKS}",
            config.world.len()
        );
    }
    config
        .world
        .iter()
        .take(MAX_WORLD_CLOCKS)
        .filter_map(|w| {
            let Some(tz) = glib::TimeZone::from_identifier(Some(&w.tz)) else {
                log::warn!("clock: unknown time zone {:?} for {}", w.tz, w.code);
                return None;
            };
            Some(Zone {
                code: w.code.clone(),
                name: w.name.clone().unwrap_or_else(|| w.code.clone()),
                tz,
            })
        })
        .collect()
}

fn hourglass(time: &gtk::Label, second: Rc<Cell<i32>>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .css_classes(["hourglass"])
        .content_width(5)
        .build();
    let time = time.downgrade();
    area.set_draw_func(move |area, cr, width, height| {
        // Vertical extent of the digits, in the area's coordinates.
        let time = time.upgrade();
        let (top, bottom) = time
            .as_ref()
            .and_then(|time| digits(time, area))
            .unwrap_or((0.0, f64::from(height)));
        let span = bottom - top;
        let left = f64::from(59 - second.get().clamp(0, 59)) / 59.0;

        let text = time.as_ref().map_or_else(|| area.color(), |t| t.color());
        cr.set_source_rgba(
            text.red().into(),
            text.green().into(),
            text.blue().into(),
            TROUGH_ALPHA,
        );
        cr.rectangle(0.0, top, f64::from(width), span);
        let _ = cr.fill();

        let color = area.color();
        cr.set_source_rgba(
            color.red().into(),
            color.green().into(),
            color.blue().into(),
            color.alpha().into(),
        );
        cr.rectangle(0.0, bottom - span * left, f64::from(width), span * left);
        let _ = cr.fill();
    });
    area
}

/// Top and bottom of the ink of `time`'s text, in `area` coordinates.
fn digits(time: &gtk::Label, area: &gtk::DrawingArea) -> Option<(f64, f64)> {
    let (ink, _) = time.layout().pixel_extents();
    let (_, y) = time.layout_offsets();
    let at = |dy: i32| {
        time.compute_point(area, &graphene::Point::new(0.0, (y + dy) as f32))
            .map(|p| f64::from(p.y()))
    };
    Some((at(ink.y())?, at(ink.y() + ink.height())?))
}

struct State {
    labels: Labels,
    zones: Vec<Zone>,
    label: Option<String>,
    rotate: i64,
    second: Rc<Cell<i32>>,
}

impl State {
    fn update(&self) {
        let Ok(now) = glib::DateTime::now_local() else {
            return;
        };
        let l = &self.labels;
        let fmt =
            |dt: &glib::DateTime, f: &str| dt.format(f).map(|s| s.to_string()).unwrap_or_default();

        l.zone
            .set_label(&self.label.clone().unwrap_or_else(|| abbreviation(&now)));
        l.time.set_label(&fmt(&now, "%H:%M"));
        let date = fmt(&now, "%a %b %d");
        if let Some(d) = &l.date {
            d.set_label(&date);
        }

        if let Some((code, time, diff)) = &l.world {
            let i = (now.to_unix() / self.rotate) as usize % self.zones.len();
            let zone = &self.zones[i];
            if let Ok(there) = now.to_timezone(&zone.tz) {
                code.set_label(&zone.code);
                time.set_label(&fmt(&there, "%H:%M"));
                diff.set_label(&difference(&now, &there));
            }
        }

        l.root.set_tooltip_markup(Some(&self.tooltip(&now, &date)));

        self.second.set(now.second());
        l.hourglass.queue_draw();
    }

    fn tooltip(&self, now: &glib::DateTime, date: &str) -> String {
        let mut lines = vec![glib::markup_escape_text(date).to_string()];
        let width = self
            .zones
            .iter()
            .map(|z| z.name.chars().count())
            .max()
            .unwrap_or(0);
        for zone in &self.zones {
            if let Ok(there) = now.to_timezone(&zone.tz) {
                let time = there
                    .format("%H:%M")
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                lines.push(format!(
                    "<tt>{:width$}  {time}  {}</tt>",
                    glib::markup_escape_text(&zone.name),
                    difference(now, &there),
                ));
            }
        }
        let week = now.week_of_year();
        let day = now.day_of_year();
        lines.push(tr_args(
            "clock-tooltip-year",
            &[("week", number(week)), ("day", number(day))],
        ));
        lines.join("\n")
    }
}

/// Re-runs `update` just after every second boundary.
fn schedule(state: Rc<State>) {
    let wait = glib::DateTime::now_local()
        .map(|now| 1000 - u64::try_from(now.microsecond() / 1000).unwrap_or(0))
        .unwrap_or(1000);
    glib::timeout_add_local_once(Duration::from_millis(wait + 5), move || {
        state.update();
        schedule(state);
    });
}

/// `there`'s offset from `here`: `+4h`, `-1h`, `+5:30` or `=`.
fn difference(here: &glib::DateTime, there: &glib::DateTime) -> String {
    let minutes = there.utc_offset().as_minutes() - here.utc_offset().as_minutes();
    let sign = if minutes < 0 { '-' } else { '+' };
    match (minutes.abs() / 60, minutes.abs() % 60) {
        (0, 0) => "=".into(),
        (h, 0) => format!("{sign}{h}h"),
        (h, m) => format!("{sign}{h}:{m:02}"),
    }
}

/// The local zone's abbreviation. tzdata has none for Brazil's zones — `%Z`
/// gives only `-03` — so those come from this table.
fn abbreviation(now: &glib::DateTime) -> String {
    let z = now.format("%Z").map(|s| s.to_string()).unwrap_or_default();
    if !z.starts_with(['+', '-']) {
        return z;
    }
    let brazil = match glib::TimeZone::local().identifier().as_str() {
        "America/Noronha" => Some("FNT"),
        "America/Sao_Paulo" | "America/Bahia" | "America/Fortaleza" | "America/Recife"
        | "America/Belem" | "America/Maceio" | "America/Araguaina" | "America/Santarem" => {
            Some("BRT")
        }
        "America/Manaus"
        | "America/Cuiaba"
        | "America/Campo_Grande"
        | "America/Porto_Velho"
        | "America/Boa_Vista" => Some("AMT"),
        "America/Rio_Branco" | "America/Eirunepe" => Some("ACT"),
        _ => None,
    };
    brazil.map_or(z, String::from)
}
