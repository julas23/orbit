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
use orbit_app::i18n::{text, tr, tr_args};
use serde_json::Value;

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
    let details = config.clone();
    crate::settings::on_click(&root, 1, move |anchor, x, y| {
        if let Some(weather) = weather_data::last() {
            open_details(anchor, x, y, &details, &weather);
        }
    });
    let options = config.clone();
    crate::settings::on_click(&root, 3, move |anchor, x, y| menu(anchor, x, y, &options));
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
    let range = gtk::Box::builder().spacing(5).halign(gtk::Align::End).build();
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
        let (label, text, running, width) = (self.label.clone(), self.text.clone(), self.running.clone(), self.width);
        let step = Cell::new(0usize);
        glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
            let full = text.borrow().clone();
            if full.chars().count() <= width {
                label.set_label(&full);
                running.set(false);
                return glib::ControlFlow::Break;
            }
            let lap: Vec<char> = format!("{full}{MARQUEE_GAP}").chars().collect();
            let window: String = lap.iter().cycle().skip(step.get() % lap.len()).take(width).collect();
            label.set_label(&window);
            step.set(step.get() + 1);
            glib::ControlFlow::Continue
        });
    }
}

// ------------------------------------------------------- the details card

/// The weather's picture for `code`, or its glyph, at `size` px.
fn picture(config: &WeatherConfig, code: i32, night: bool, size: i32) -> gtk::Widget {
    match config.picture(code, night) {
        Some(file) => gtk::Image::builder()
            .file(file.to_string_lossy().as_ref())
            .pixel_size(size)
            .build()
            .upcast(),
        None => {
            let glyph = label("glyph");
            glyph.set_label(weather_data::glyph(code));
            glyph.set_attributes(Some(&{
                let attrs = gtk::pango::AttrList::new();
                attrs.insert(gtk::pango::AttrSize::new_size_absolute(size * gtk::pango::SCALE * 3 / 4));
                attrs
            }));
            glyph.set_size_request(size, size);
            glyph.upcast()
        }
    }
}

/// A fact of the details card: an icon, what it is and its value.
fn fact(grid: &gtk::Grid, at: i32, glyph: &str, name: &str, value: &str) {
    let (col, row) = ((at % 2) * 3, at / 2);
    grid.attach(&gtk::Label::builder().label(glyph).css_classes(["fact-glyph"]).build(), col, row, 1, 1);
    grid.attach(&gtk::Label::builder().label(name).css_classes(["fact-name"]).xalign(0.0).build(), col + 1, row, 1, 1);
    grid.attach(&gtk::Label::builder().label(value).css_classes(["fact-value"]).xalign(1.0).build(), col + 2, row, 1, 1);
}

/// Everything the weather has, in a card that closes at any click.
fn open_details(anchor: &gtk::Widget, x: f64, y: f64, config: &WeatherConfig, w: &Weather) {
    let card = details(config, w);
    let popover = crate::settings::popover(anchor, x, y, &card);
    popover.add_css_class("weather-popover");
    // Any click closes it, inside as well as outside.
    let click = gtk::GestureClick::builder().button(0).build();
    let weak = popover.downgrade();
    click.connect_released(move |_, _, _, _| {
        if let Some(popover) = weak.upgrade() {
            popover.popdown();
        }
    });
    card.add_controller(click);
}

/// The details card itself.
fn details(config: &WeatherConfig, w: &Weather) -> gtk::Box {
    let today = w.days.first().cloned().unwrap_or_default();
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(["weather-details"])
        .build();

    let description = weather_data::description(w.code);
    card.append(&gtk::Label::builder().label(&w.place).css_classes(["place"]).xalign(0.0).build());

    // The picture, the temperature and the day's range.
    let top = gtk::Box::builder().spacing(12).build();
    top.append(&picture(config, w.code, !w.is_day, 72));
    let numbers = gtk::Box::builder().orientation(gtk::Orientation::Vertical).valign(gtk::Align::Center).build();
    let temp = gtk::Label::builder().label(format!("{:.0}°C", w.temperature)).css_classes(["big-temp"]).xalign(0.0).build();
    weather_data::colour_temp(&temp, w.temperature);
    numbers.append(&temp);
    numbers.append(&gtk::Label::builder().label(&description).css_classes(["desc"]).xalign(0.0).build());
    numbers.append(
        &gtk::Label::builder()
            .label(tr_args(
                "weather-details-range",
                &[
                    ("min", text(format!("{:.0}", today.min))),
                    ("max", text(format!("{:.0}", today.max))),
                    ("feels", text(format!("{:.0}", w.feels_like))),
                ],
            ))
            .css_classes(["range"])
            .xalign(0.0)
            .build(),
    );
    top.append(&numbers);
    card.append(&top);

    // The facts, two columns.
    let grid = gtk::Grid::builder().column_spacing(10).row_spacing(4).css_classes(["facts"]).build();
    let (_, moon_name, moon_lit) = weather_data::moon();
    let season = glib::DateTime::now_local()
        .map(|now| weather_data::season(now.month(), now.day_of_month(), w.latitude))
        .unwrap_or_default();
    let facts = [
        ("\u{f058e}", tr("weather-humidity"), format!("{:.0}%", w.humidity)),
        ("\u{f059d}", tr("weather-wind"), format!("{:.0} km/h {}", w.wind_speed, weather_data::compass(w.wind_direction))),
        ("\u{f029a}", tr("weather-pressure"), format!("{:.0} hPa", w.pressure)),
        ("\u{f0590}", tr("weather-clouds"), format!("{:.0}%", w.clouds)),
        ("\u{f0599}", tr("weather-uv"), w.uv.map_or("—".into(), |uv| format!("{uv:.0}"))),
        ("\u{f0597}", tr("weather-rain"), today.rain.map_or("—".into(), |r| format!("{r:.0}%"))),
        ("\u{f059c}", tr("weather-sunrise"), today.sunrise.clone()),
        ("\u{f059b}", tr("weather-sunset"), today.sunset.clone()),
        ("\u{f0f64}", tr("weather-moon"), format!("{moon_name} · {moon_lit}%")),
        ("\u{f0d43}", tr("weather-season"), season),
    ];
    for (i, (glyph, name, value)) in facts.iter().enumerate() {
        fact(&grid, i as i32, glyph, name, value);
    }
    grid.attach(&gtk::Box::builder().width_request(16).build(), 2, 0, 1, 1);
    card.append(&grid);

    // The next hours, every third one.
    if !w.hours.is_empty() {
        card.append(&gtk::Label::builder().label(tr("weather-next-hours")).css_classes(["section"]).xalign(0.0).build());
        let hours = gtk::Box::builder().homogeneous(true).css_classes(["hours"]).build();
        for hour in w.hours.iter().step_by(3).take(8) {
            let cell = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
            cell.append(&gtk::Label::builder().label(&hour.time).css_classes(["hour"]).build());
            let pic = picture(config, hour.code, !hour.is_day, 28);
            pic.set_halign(gtk::Align::Center);
            cell.append(&pic);
            let t = gtk::Label::builder().label(format!("{:.0}°", hour.temperature)).css_classes(["hour-temp"]).build();
            weather_data::colour_temp(&t, hour.temperature);
            cell.append(&t);
            cell.append(&gtk::Label::builder().label(format!("{:.0}%", hour.rain)).css_classes(["hour-rain"]).build());
            hours.append(&cell);
        }
        card.append(&hours);
    }

    // The days.
    card.append(&gtk::Label::builder().label(tr("weather-next-days")).css_classes(["section"]).xalign(0.0).build());
    let days = gtk::Grid::builder().column_spacing(10).row_spacing(2).css_classes(["days"]).build();
    for (row, day) in w.days.iter().enumerate() {
        let row = row as i32;
        let when = crate::forecast::date(&day.date)
            .map(|d| format!("{} {}/{}", crate::forecast::weekday(day), d.day_of_month(), d.month()))
            .unwrap_or_default();
        days.attach(&gtk::Label::builder().label(when).css_classes(["day-name"]).xalign(0.0).build(), 0, row, 1, 1);
        days.attach(&picture(config, day.code, false, 22), 1, row, 1, 1);
        days.attach(
            &gtk::Label::builder()
                .label(weather_data::description(day.code))
                .css_classes(["day-desc"])
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
            2,
            row,
            1,
            1,
        );
        let rain = day.rain.map_or(String::new(), |r| format!("\u{f058c} {r:.0}%"));
        days.attach(&gtk::Label::builder().label(rain).css_classes(["day-rain"]).xalign(1.0).build(), 3, row, 1, 1);
        let min = gtk::Label::builder().label(format!("{:.0}°", day.min)).css_classes(["min"]).xalign(1.0).build();
        let max = gtk::Label::builder().label(format!("{:.0}°", day.max)).css_classes(["max"]).xalign(1.0).build();
        days.attach(&min, 4, row, 1, 1);
        days.attach(&max, 5, row, 1, 1);
    }
    card.append(&days);
    card
}

// ------------------------------------------------------------ the menu

/// Where the weather is, how often it is fetched and its pictures.
fn menu(anchor: &gtk::Widget, x: f64, y: f64, config: &WeatherConfig) {
    use crate::settings::{Menu, save, section, submenu};
    let menu = Menu::new();
    let first = section(&menu, Some(&tr("panel-module-weather")));
    menu.item(&first, &tr("weather-refresh"), weather_data::refresh);
    {
        let anchor = anchor.clone();
        menu.item(&first, &tr("weather-location"), move || location(&anchor, x, y));
    }

    let more = section(&menu, None);
    let every = submenu(&more, &tr("weather-interval"));
    let choices: Vec<(String, String)> = [15u32, 30, 60, 120]
        .iter()
        .map(|m| (m.to_string(), tr_args("weather-every", &[("minutes", text(m.to_string()))])))
        .collect();
    menu.radio(&every, &choices, &config.interval.to_string(), |value| {
        if let Ok(minutes) = value.parse::<u32>() {
            save("weather", &[("interval", minutes.into())], &["weather", "forecast", "calendar"]);
        }
    });

    let icons = submenu(&more, &tr("weather-icons"));
    let mut choices = vec![("0".to_string(), tr("weather-icons-glyphs"))];
    for theme in 1..=7u32 {
        let available = config.dir().join(format!("tema{theme}")).is_dir();
        if available {
            choices.push((theme.to_string(), tr_args("weather-icons-theme", &[("n", text(theme.to_string()))])));
        }
    }
    menu.radio(&icons, &choices, &config.icon_theme.to_string(), |value| {
        if let Ok(theme) = value.parse::<u32>() {
            save("weather", &[("icon_theme", theme.into())], &["weather", "forecast", "calendar"]);
        }
    });
    menu.open(anchor, x, y);
}

/// The language for place names: `pt` of `pt_BR.UTF-8`.
fn language() -> String {
    std::env::var("LANG")
        .ok()
        .and_then(|l| l.get(..2).map(str::to_owned))
        .filter(|l| l.chars().all(|c| c.is_ascii_alphabetic()))
        .unwrap_or_else(|| "en".into())
}

/// Picking the place: by the network's address, or one found by name.
fn location(anchor: &gtk::Widget, x: f64, y: f64) {
    use crate::settings::{form_box, save};
    let content = form_box(&tr("weather-location"));
    let automatic = gtk::Button::builder().label(tr("weather-location-auto")).build();
    let search = gtk::SearchEntry::builder().placeholder_text(tr("weather-location-search")).build();
    let status = gtk::Label::builder().css_classes(["dim"]).xalign(0.0).build();
    let list = gtk::ListBox::builder().css_classes(["places"]).build();
    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .min_content_height(220)
        .min_content_width(300)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    content.append(&automatic);
    content.append(&search);
    content.append(&status);
    content.append(&scroll);
    let popover = crate::settings::popover(anchor, x, y, &content);
    search.grab_focus();

    let rebuild = ["weather", "forecast", "calendar"];
    {
        let popover = popover.downgrade();
        automatic.connect_clicked(move |_| {
            save(
                "weather",
                &[("latitude", Value::Null), ("longitude", Value::Null), ("place", Value::Null)],
                &rebuild,
            );
            if let Some(p) = popover.upgrade() {
                p.popdown();
            }
        });
    }

    let found: Rc<std::cell::RefCell<Vec<weather_data::Place>>> = Rc::default();
    {
        let found = found.clone();
        let popover = popover.downgrade();
        list.connect_row_activated(move |_, row| {
            let Some(place) = found.borrow().get(row.index() as usize).cloned() else {
                return;
            };
            save(
                "weather",
                &[
                    ("latitude", place.latitude.into()),
                    ("longitude", place.longitude.into()),
                    ("place", place.name.clone().into()),
                ],
                &rebuild,
            );
            if let Some(p) = popover.upgrade() {
                p.popdown();
            }
        });
    }
    search.connect_activate(move |search| {
        let name = search.text().trim().to_string();
        if name.chars().count() < 2 {
            return;
        }
        status.set_label(&tr("weather-location-searching"));
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(weather_data::search_places(&name, &language()));
        });
        let (list, found, status) = (list.clone(), found.clone(), status.clone());
        glib::spawn_future_local(async move {
            let Ok(result) = rx.recv().await else { return };
            list.remove_all();
            match result {
                Ok(places) => {
                    status.set_label(&if places.is_empty() { tr("weather-location-none") } else { String::new() });
                    for place in &places {
                        let row = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
                        row.append(&gtk::Label::builder().label(&place.name).xalign(0.0).css_classes(["place-name"]).build());
                        row.append(
                            &gtk::Label::builder()
                                .label(format!("{} · {:.2}, {:.2}", place.region, place.latitude, place.longitude))
                                .xalign(0.0)
                                .css_classes(["dim"])
                                .build(),
                        );
                        list.append(&row);
                    }
                    *found.borrow_mut() = places;
                }
                Err(err) => status.set_label(&err),
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The details card from the cached weather, drawn to
    /// `$ORBIT_WEATHER_LOOK` (PNG), on a display of its own.
    #[test]
    #[ignore]
    fn details_look() {
        let Ok(path) = std::env::var("ORBIT_WEATHER_LOOK") else {
            return;
        };
        gtk::init().expect("a display");
        let display = gtk::gdk::Display::default().unwrap();
        let _theme = orbit_app::theme::install(&display);
        let css = gtk::CssProvider::new();
        css.load_from_string(&format!(
            "{}\n{}\nwindow {{ background: #1b1f27; color: #d8dee9; }}",
            include_str!("../../../data/palette-dark.css"),
            include_str!("../../../data/style.css")
        ));
        gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        let config = WeatherConfig::default();
        let weather = weather_data::fetch_now(&config).expect("the weather");
        let window = gtk::Window::builder().css_classes(["orbit"]).child(&details(&config, &weather)).build();
        window.present();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < until {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let (w, h) = (window.width(), window.height());
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(w), f64::from(h));
        let node = snapshot.to_node().unwrap();
        window.renderer().unwrap().render_texture(&node, None).save_to_png(&path).unwrap();
    }
}
