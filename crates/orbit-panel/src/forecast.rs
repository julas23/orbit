//! Forecast module, after the forecast of the eww panel: the next days —
//! tomorrow and the ones after, not today, which the weather card shows —
//! each a small card with the weekday, the picture and the maximum. The
//! tooltip has the date, description, minimum and maximum.
//!
//! Same data and icons as the weather module (`weather_data`, read from
//! modules/weather.yaml); this module's own file only says how many days.
//! Line form: one row of icon + maximum per day.

use std::rc::Rc;

use gtk::{glib, prelude::*};
use serde::Deserialize;

use orbit_app::i18n::{text, tr_args};

use crate::weather_data::{self, Weather, WeatherConfig};

/// `modules/forecast.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ForecastConfig {
    /// Days shown, from tomorrow on (the data covers up to 4).
    pub days: usize,
}

impl Default for ForecastConfig {
    fn default() -> Self {
        Self { days: 4 }
    }
}

struct DayCard {
    root: gtk::Box,
    name: gtk::Label,
    icon: gtk::Image,
    glyph: gtk::Label,
    max: gtk::Label,
}

pub fn new(config: &ForecastConfig, weather: &WeatherConfig, vertical: bool) -> gtk::Widget {
    // Four 31–32 px columns: four cards and three 5 px gaps cannot all be
    // multiples of 5 in 140 px, so — as with the calendar's columns — only
    // the module's box is on the grid.
    let row = gtk::Box::builder()
        .spacing(5)
        .homogeneous(vertical)
        .css_classes(["forecast"])
        .build();
    let cards: Vec<DayCard> = (0..config.days.clamp(1, 4))
        .map(|_| {
            let card = DayCard {
                root: gtk::Box::builder()
                    .orientation(if vertical {
                        gtk::Orientation::Vertical
                    } else {
                        gtk::Orientation::Horizontal
                    })
                    .css_classes(if vertical {
                        vec!["card", "day"]
                    } else {
                        vec!["day"]
                    })
                    .build(),
                name: gtk::Label::builder().css_classes(["name"]).build(),
                icon: gtk::Image::builder().pixel_size(20).build(),
                glyph: gtk::Label::builder()
                    .css_classes(["glyph"])
                    .visible(false)
                    .build(),
                max: gtk::Label::builder().css_classes(["max"]).build(),
            };
            if vertical {
                card.root.append(&card.name);
            }
            card.root.append(&card.icon);
            card.root.append(&card.glyph);
            card.root.append(&card.max);
            row.append(&card.root);
            card
        })
        .collect();

    let cards = Rc::new(cards);
    let config = weather.clone();
    weather_data::subscribe(weather, move |w| show(&cards, &config, w));
    row.upcast()
}

fn show(cards: &[DayCard], config: &WeatherConfig, w: &Weather) {
    for (card, day) in cards.iter().zip(w.days.iter().skip(1)) {
        let weekday = weekday(day);
        card.name.set_label(&weekday);
        match config.picture(day.code, false) {
            Some(file) => {
                card.icon.set_from_file(Some(file));
                card.icon.set_visible(true);
                card.glyph.set_visible(false);
            }
            None => {
                card.glyph.set_label(weather_data::glyph(day.code));
                card.glyph.set_visible(true);
                card.icon.set_visible(false);
            }
        }
        card.max.set_label(&format!("{:.0}°", day.max));
        card.root.set_tooltip_text(Some(&tooltip(day)));
    }
}

/// The day's short weekday name.
fn weekday(day: &weather_data::Day) -> String {
    date(&day.date)
        .and_then(|d| d.format("%a").ok())
        .map(|s| s.trim_end_matches('.').to_string())
        .unwrap_or_default()
}

/// Weekday, date, description, minimum and maximum of `day`.
pub fn tooltip(day: &weather_data::Day) -> String {
    let short = date(&day.date)
        .map(|d| format!("{}/{}", d.day_of_month(), d.month()))
        .unwrap_or_default();
    tr_args(
        "forecast-tooltip",
        &[
            ("weekday", text(weekday(day))),
            ("date", text(short)),
            ("description", text(weather_data::description(day.code))),
            ("min", text(format!("{:.0}", day.min))),
            ("max", text(format!("{:.0}", day.max))),
        ],
    )
}

/// `YYYY-MM-DD` at local noon.
fn date(text: &str) -> Option<glib::DateTime> {
    let mut parts = text.split('-').map(|p| p.parse::<i32>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    glib::DateTime::from_local(y, m, d, 12, 0, 0.0).ok()
}
