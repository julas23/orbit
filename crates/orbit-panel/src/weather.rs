//! Weather module, after the weather card of the eww panel, which took its
//! look from brweather (a conky script, ~2014):
//!
//!   picture icon (day/night)    temperature
//!                               min  max
//!   description, scrolling when it does not fit
//!   sunrise   moon of the day   sunset
//!   humidity                    wind
//!
//!   the next days (the forecast module's row)
//!
//! That is Full (4 × 4 cells). Normal (4 × 2) keeps the picture, the
//! temperatures and the description; Mini (3 × 1) and the line form only a
//! small picture and the temperature. Pressure, clouds, UV, the moon's name
//! and the season — and in the smaller forms everything they leave out — go
//! to the tooltip.
//!
//! Icons: brweather's pictures (`icon_dir`/tema<N>/<code>.png, night versions
//! with a "2" suffix where the theme has one) and its 36 moon phases
//! (`icon_dir`/lua-sul or lua-norte — the southern ones are mirrored). They
//! are not in the repository (third-party themes); without them, Nerd Font
//! glyphs are used.

use std::{cell::Cell, rc::Rc};

use gtk::{glib, prelude::*};

pub use crate::weather_data::WeatherConfig;
use orbit_app::i18n::{text, tr_args};

use crate::{
    board::Form,
    forecast::{self, ForecastConfig},
    weather_data::{self, Weather},
};

struct Card {
    root: gtk::Widget,
    icon: gtk::Image,
    glyph: gtk::Label,
    temp: gtk::Label,
    min: Option<gtk::Label>,
    max: Option<gtk::Label>,
    desc: Option<Marquee>,
    sunrise: Option<gtk::Label>,
    sunset: Option<gtk::Label>,
    moon: Option<gtk::Image>,
    humidity: Option<gtk::Label>,
    wind: Option<gtk::Label>,
}

pub fn new(config: &WeatherConfig, form: Form) -> gtk::Widget {
    let card = Rc::new(build(config, form));
    let root = card.root.clone();
    let shown = config.clone();
    weather_data::subscribe(config, move |weather| show(&card, &shown, weather));
    root
}

fn label(class: &str) -> gtk::Label {
    gtk::Label::builder().css_classes([class]).build()
}

fn build(config: &WeatherConfig, form: Form) -> Card {
    let icon = gtk::Image::builder()
        .pixel_size(match form {
            Form::Full | Form::Normal => 40,
            Form::Mini => 25,
            Form::Line => 30,
        })
        .build();
    let glyph = label("glyph");
    glyph.set_visible(false);
    let temp = label("temp");

    if matches!(form, Form::Mini | Form::Line) {
        let line = gtk::Box::builder()
            .spacing(5)
            .css_classes(if form == Form::Mini {
                vec!["card", "weather-line", "mini"]
            } else {
                vec!["weather-line"]
            })
            .build();
        if form == Form::Mini {
            line.set_halign(gtk::Align::Fill);
            temp.set_hexpand(true);
        }
        line.append(&icon);
        line.append(&glyph);
        line.append(&temp);
        return Card {
            root: line.upcast(),
            icon,
            glyph,
            temp,
            min: None,
            max: None,
            desc: None,
            sunrise: None,
            sunset: None,
            moon: None,
            humidity: None,
            wind: None,
        };
    }

    // Row 1: picture on the left; temperature, min and max on the right.
    let (min, max) = (label("min"), label("max"));
    let range = gtk::Box::builder()
        .spacing(5)
        .halign(gtk::Align::End)
        .build();
    range.append(&min);
    range.append(&max);
    let numbers = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .halign(gtk::Align::End)
        .valign(gtk::Align::Center)
        .build();
    temp.set_halign(gtk::Align::End);
    numbers.append(&temp);
    numbers.append(&range);
    let top = gtk::Box::builder().spacing(5).build();
    top.append(&icon);
    top.append(&glyph);
    top.append(&numbers);

    let desc = Marquee::new(config.marquee);
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["card", "weather", form.class()])
        .build();
    card.append(&top);
    card.append(&desc.label);
    if form == Form::Normal {
        return Card {
            root: card.upcast(),
            icon,
            glyph,
            temp,
            min: Some(min),
            max: Some(max),
            desc: Some(desc),
            sunrise: None,
            sunset: None,
            moon: None,
            humidity: None,
            wind: None,
        };
    }

    let (sunrise, sunset) = (label("sun"), label("sun"));
    let moon = gtk::Image::builder().pixel_size(20).build();
    let sun = gtk::CenterBox::builder()
        .css_classes(["line"])
        .start_widget(&sunrise)
        .center_widget(&moon)
        .end_widget(&sunset)
        .build();

    let (humidity, wind) = (label("info"), label("info"));
    let info = gtk::CenterBox::builder()
        .css_classes(["line"])
        .start_widget(&humidity)
        .end_widget(&wind)
        .build();

    card.append(&sun);
    card.append(&info);
    let days = forecast::new(&ForecastConfig::default(), config, true);
    days.set_vexpand(true);
    days.set_valign(gtk::Align::End);
    card.append(&days);

    Card {
        root: card.upcast(),
        icon,
        glyph,
        temp,
        min: Some(min),
        max: Some(max),
        desc: Some(desc),
        sunrise: Some(sunrise),
        sunset: Some(sunset),
        moon: Some(moon),
        humidity: Some(humidity),
        wind: Some(wind),
    }
}

fn show(card: &Card, config: &WeatherConfig, w: &Weather) {
    let today = w.days.first().cloned().unwrap_or_default();
    match config.picture(w.code, !w.is_day) {
        Some(file) => {
            card.icon.set_from_file(Some(file));
            card.icon.set_visible(true);
            card.glyph.set_visible(false);
        }
        None => {
            card.glyph.set_label(weather_data::glyph(w.code));
            card.glyph.set_visible(true);
            card.icon.set_visible(false);
        }
    }
    card.temp.set_label(&format!("{:.0}°C", w.temperature));
    weather_data::colour_temp(&card.temp, w.temperature);
    if let (Some(min), Some(max)) = (&card.min, &card.max) {
        min.set_label(&format!("{:.0}°", today.min));
        max.set_label(&format!("{:.0}°", today.max));
    }
    let description = weather_data::description(w.code);
    if let Some(desc) = &card.desc {
        desc.set(&description);
    }
    if let (Some(rise), Some(set)) = (&card.sunrise, &card.sunset) {
        rise.set_label(&format!("\u{f059c} {}", today.sunrise)); // 󰖜
        set.set_label(&format!("{} \u{f059b}", today.sunset)); // 󰖛
    }
    let (moon_index, moon_name, moon_lit) = weather_data::moon();
    if let Some(moon) = &card.moon {
        match config.moon(moon_index, w.latitude < 0.0) {
            Some(file) => moon.set_from_file(Some(file)),
            None => moon.set_visible(false),
        }
    }
    let compass = weather_data::compass(w.wind_direction);
    if let (Some(humidity), Some(wind)) = (&card.humidity, &card.wind) {
        humidity.set_label(&format!("\u{f058e} {:.0}%", w.humidity)); // 󰖎
        wind.set_label(&format!("\u{f059d} {:.0} {compass}", w.wind_speed)); // 󰖝
    }

    let season = glib::DateTime::now_local()
        .map(|now| weather_data::season(now.month(), now.day_of_month(), w.latitude))
        .unwrap_or_default();
    let uv = w.uv.map_or("—".to_string(), |uv| format!("{uv:.0}"));
    // Numbers go in already rounded: Fluent would print 22.3 as "22.3".
    let n = |v: f64| text(format!("{v:.0}"));
    let mut tooltip = tr_args(
        "weather-tooltip",
        &[
            ("description", text(description.clone())),
            ("place", text(w.place.clone())),
            ("min", n(today.min)),
            ("max", n(today.max)),
            ("sunrise", text(today.sunrise.clone())),
            ("sunset", text(today.sunset.clone())),
            ("feels", n(w.feels_like)),
            ("humidity", n(w.humidity)),
            ("wind", n(w.wind_speed)),
            ("direction", text(compass.clone())),
            ("pressure", n(w.pressure)),
            ("clouds", n(w.clouds)),
            ("uv", text(uv)),
            ("moon", text(moon_name)),
            ("lit", text(moon_lit.to_string())),
            ("season", text(season)),
        ],
    );
    // The forms without the next days have them in the tooltip.
    if card.sunrise.is_none() {
        for day in w.days.iter().skip(1) {
            tooltip.push('\n');
            tooltip.push_str(&forecast::tooltip(day));
        }
    }
    card.root.set_tooltip_text(Some(&tooltip));
}

/// A label that scrolls its text like HTML's <marquee> when it is longer
/// than `width` characters, one character every 300 ms; a text that fits
/// stays still and the timer stops. Relies on the monospace panel font.
struct Marquee {
    label: gtk::Label,
    width: usize,
    text: Rc<std::cell::RefCell<String>>,
    running: Rc<Cell<bool>>,
}

const MARQUEE_GAP: &str = "   ·   ";

impl Marquee {
    fn new(width: usize) -> Self {
        Self {
            label: gtk::Label::builder()
                .css_classes(["desc"])
                .halign(gtk::Align::Center)
                .build(),
            width: width.max(1),
            text: Rc::default(),
            running: Rc::default(),
        }
    }

    fn set(&self, text: &str) {
        if *self.text.borrow() == text {
            return;
        }
        *self.text.borrow_mut() = text.to_string();
        if text.chars().count() <= self.width {
            self.label.set_label(text);
            return;
        }
        if self.running.replace(true) {
            return; // the running timer picks the new text up
        }
        let (label, text, running, width) = (
            self.label.clone(),
            self.text.clone(),
            self.running.clone(),
            self.width,
        );
        let step = Cell::new(0usize);
        glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
            let full = text.borrow().clone();
            if full.chars().count() <= width {
                label.set_label(&full);
                running.set(false);
                return glib::ControlFlow::Break;
            }
            let lap: Vec<char> = format!("{full}{MARQUEE_GAP}").chars().collect();
            let window: String = lap
                .iter()
                .cycle()
                .skip(step.get() % lap.len())
                .take(width)
                .collect();
            label.set_label(&window);
            step.set(step.get() + 1);
            glib::ControlFlow::Continue
        });
    }
}
