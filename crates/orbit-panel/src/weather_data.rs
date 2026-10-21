//! Weather data shared by the weather and forecast modules: one fetch serves
//! both. After `bar-clima.sh` of the eww panel:
//!
//! - wttr.in locates by IP (coordinates and place name) and gives the UV
//!   index — unless coordinates are configured;
//! - Open-Meteo gives everything else, current and daily, with the ICON
//!   model by default: the automatic model and wttr.in's own conditions both
//!   overstated rain on the coast, and ICON matched the reference forecast.
//!
//! The fetch runs on a thread every `interval` minutes; the last result is
//! cached in ~/.cache/orbit/weather.json, so the panel starts with it
//! without waiting for the network, and keeps it when the network is down.

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gtk::glib;
use orbit_app::i18n::tr;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `modules/weather.yaml`. The forecast module reads the same file, so the
/// two always show the same data and the same icons.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WeatherConfig {
    /// Open-Meteo model.
    pub model: String,
    /// Minutes between fetches.
    pub interval: u32,
    /// Fixed place instead of locating by IP.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub place: Option<String>,
    /// Where brweather's pictures are (`tema1`…`tema7`, `lua-sul`,
    /// `lua-norte`). Not in the repository: they come from third-party
    /// themes. Without them, Nerd Font glyphs are used.
    pub icon_dir: String,
    /// brweather theme, 1 to 7.
    pub icon_theme: u32,
    /// Characters of the description shown at once; longer ones scroll.
    pub marquee: usize,
}

impl Default for WeatherConfig {
    fn default() -> Self {
        Self {
            model: "icon_seamless".into(),
            interval: 15,
            latitude: None,
            longitude: None,
            place: None,
            icon_dir: "~/.local/share/orbit/weather".into(),
            icon_theme: 3,
            marquee: 21,
        }
    }
}

impl WeatherConfig {
    fn dir(&self) -> std::path::PathBuf {
        orbit_common::config::expand_home(self.icon_dir.as_ref())
    }

    /// The picture for `code`, its night version if asked and available.
    pub fn picture(&self, code: i32, night: bool) -> Option<std::path::PathBuf> {
        let theme = self.dir().join(format!("tema{}", self.icon_theme));
        let base = brweather_code(code);
        let night_file = theme.join(format!("{base}2.png"));
        if night && night_file.exists() {
            return Some(night_file);
        }
        let file = theme.join(format!("{base}.png"));
        file.exists().then_some(file)
    }

    /// One of the 36 moon pictures (0 new, 18 full); the southern set is
    /// mirrored, as the moon is seen from there.
    pub fn moon(&self, index: usize, south: bool) -> Option<std::path::PathBuf> {
        let file = self
            .dir()
            .join(if south { "lua-sul" } else { "lua-norte" })
            .join(format!("{index:02}.png"));
        file.exists().then_some(file)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Weather {
    pub place: String,
    pub latitude: f64,
    pub code: i32,
    pub temperature: f64,
    pub feels_like: f64,
    pub humidity: f64,
    pub is_day: bool,
    pub clouds: f64,
    pub pressure: f64,
    pub wind_speed: f64,
    pub wind_direction: f64,
    pub uv: Option<f64>,
    /// Today first.
    pub days: Vec<Day>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Day {
    /// `YYYY-MM-DD`
    pub date: String,
    pub code: i32,
    pub min: f64,
    pub max: f64,
    /// `HH:MM`
    pub sunrise: String,
    pub sunset: String,
}

type Listener = Box<dyn Fn(&Weather)>;

struct Service {
    source: WeatherConfig,
    last: RefCell<Option<Weather>>,
    listeners: RefCell<Vec<Listener>>,
    busy: std::cell::Cell<bool>,
}

thread_local! {
    static SERVICE: RefCell<Option<Rc<Service>>> = const { RefCell::new(None) };
}

/// Calls `listener` with the weather now (from the cache, if any) and after
/// every fetch. The first call starts the service with `source`.
pub fn subscribe(source: &WeatherConfig, listener: impl Fn(&Weather) + 'static) {
    let service = SERVICE.with(|s| {
        s.borrow_mut()
            .get_or_insert_with(|| Service::start(source.clone()))
            .clone()
    });
    if let Some(weather) = &*service.last.borrow() {
        listener(weather);
    }
    service.listeners.borrow_mut().push(Box::new(listener));
}

impl Service {
    fn start(source: WeatherConfig) -> Rc<Service> {
        let service = Rc::new(Service {
            last: RefCell::new(read_cache()),
            source,
            listeners: RefCell::default(),
            busy: std::cell::Cell::new(false),
        });
        service.fetch();
        let weak = Rc::downgrade(&service);
        glib::timeout_add_seconds_local(service.source.interval.max(5) * 60, move || {
            match weak.upgrade() {
                Some(service) => {
                    service.fetch();
                    glib::ControlFlow::Continue
                }
                None => glib::ControlFlow::Break,
            }
        });
        service
    }

    fn fetch(self: &Rc<Self>) {
        if self.busy.replace(true) {
            return;
        }
        let (tx, rx) = async_channel::bounded(1);
        let source = self.source.clone();
        std::thread::spawn(move || {
            let _ = tx.send_blocking(fetch(&source));
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = rx.recv().await;
            let Some(service) = weak.upgrade() else {
                return;
            };
            service.busy.set(false);
            match result {
                Ok(Ok(weather)) => {
                    write_cache(&weather);
                    for listener in service.listeners.borrow().iter() {
                        listener(&weather);
                    }
                    *service.last.borrow_mut() = Some(weather);
                }
                Ok(Err(err)) => log::warn!("weather: {err}; keeping the last data"),
                Err(_) => {}
            }
        });
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into()
}

fn get_json(agent: &ureq::Agent, url: &str) -> Result<Value, String> {
    let text = agent
        .get(url)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

fn fetch(source: &WeatherConfig) -> Result<Weather, String> {
    let agent = agent();
    let (lat, lon, mut place, uv) = match (source.latitude, source.longitude) {
        (Some(lat), Some(lon)) => (lat, lon, source.place.clone().unwrap_or_default(), None),
        _ => {
            let wttr = get_json(&agent, "https://wttr.in/?format=j1")?;
            let area = &wttr["nearest_area"][0];
            let number = |v: &Value| v.as_str().and_then(|s| s.parse::<f64>().ok());
            (
                number(&area["latitude"]).ok_or("wttr.in: no latitude")?,
                number(&area["longitude"]).ok_or("wttr.in: no longitude")?,
                area["areaName"][0]["value"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                number(&wttr["current_condition"][0]["uvIndex"]),
            )
        }
    };
    if let Some(name) = &source.place {
        place = name.clone();
    }

    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}\
         &models={model}&timezone=auto&forecast_days=5\
         &current=weather_code,temperature_2m,apparent_temperature,relative_humidity_2m,\
is_day,cloud_cover,pressure_msl,wind_speed_10m,wind_direction_10m\
         &daily=weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset",
        model = source.model,
    );
    let om = get_json(&agent, &url)?;
    let current = &om["current"];
    let daily = &om["daily"];
    let num = |v: &Value| v.as_f64().unwrap_or_default();
    let time = |v: &Value| {
        v.as_str()
            .and_then(|s| s.split('T').nth(1))
            .unwrap_or_default()
            .to_string()
    };
    let count = daily["time"].as_array().map_or(0, Vec::len);
    if count == 0 || current.is_null() {
        return Err("Open-Meteo: no current or daily data".into());
    }
    let days = (0..count)
        .map(|i| Day {
            date: daily["time"][i].as_str().unwrap_or_default().to_string(),
            code: daily["weather_code"][i].as_i64().unwrap_or_default() as i32,
            min: num(&daily["temperature_2m_min"][i]),
            max: num(&daily["temperature_2m_max"][i]),
            sunrise: time(&daily["sunrise"][i]),
            sunset: time(&daily["sunset"][i]),
        })
        .collect();
    Ok(Weather {
        place,
        latitude: lat,
        code: current["weather_code"].as_i64().unwrap_or_default() as i32,
        temperature: num(&current["temperature_2m"]),
        feels_like: num(&current["apparent_temperature"]),
        humidity: num(&current["relative_humidity_2m"]),
        is_day: current["is_day"].as_i64() == Some(1),
        clouds: num(&current["cloud_cover"]),
        pressure: num(&current["pressure_msl"]),
        wind_speed: num(&current["wind_speed_10m"]),
        wind_direction: num(&current["wind_direction_10m"]),
        uv,
        days,
    })
}

fn cache_path() -> std::path::PathBuf {
    glib::user_cache_dir().join("orbit").join("weather.json")
}

fn read_cache() -> Option<Weather> {
    serde_json::from_slice(&std::fs::read(cache_path()).ok()?).ok()
}

fn write_cache(weather: &Weather) {
    let path = cache_path();
    let _ = std::fs::create_dir_all(path.parent().expect("has a parent"));
    if let Ok(json) = serde_json::to_vec(weather) {
        let _ = std::fs::write(path, json);
    }
}

// ------------------------------------------------------------ the tables

/// WMO weather code -> brweather icon code (CPTEC/INPE), day version.
pub fn brweather_code(code: i32) -> &'static str {
    match code {
        0 => "cl",            // clear sky
        1 => "ps",            // mostly sunny
        2 => "pn",            // partly cloudy
        3 => "n",             // overcast
        45 | 48 => "nv",      // fog
        51 | 53 | 55 => "cv", // drizzle
        61 => "ci",           // light rain
        63 => "c",            // rain
        65 | 82 => "ch",      // heavy rain
        // showers: brweather's own "pc" has lightning; that stays for storms
        80 | 81 => "ci",
        56 | 57 | 66 | 67 => "g",            // freezing drizzle/rain
        71 | 73 | 75 | 77 | 85 | 86 => "ne", // snow
        95 | 96 | 99 => "t",                 // thunderstorm
        _ => "nd",
    }
}

/// The colour classes of temperatures, coldest first (style.css gives each
/// its colour; `temp_class` picks one).
const TEMP_CLASSES: [&str; 10] = [
    "temp-bitter", "temp-m30", "temp-m20", "temp-m10", "temp-zero", "temp-0", "temp-10", "temp-20", "temp-30", "temp-40",
];

/// The colour class of `celsius` as shown (rounded), on the user's scale:
/// −30 and below black and bold, to −20 purple, to −10 blue, below 0
/// green, 0 white, below 10 light blue, below 20 light green, below 30
/// yellow, below 40 orange, 40 and above red and bold. Fahrenheit, when
/// it comes, is converted first and takes the same colours.
pub fn temp_class(celsius: f64) -> &'static str {
    let t = celsius.round();
    let i = if t <= -30.0 {
        0
    } else if t <= -20.0 {
        1
    } else if t <= -10.0 {
        2
    } else if t < 0.0 {
        3
    } else if t == 0.0 {
        4
    } else if t < 10.0 {
        5
    } else if t < 20.0 {
        6
    } else if t < 30.0 {
        7
    } else if t < 40.0 {
        8
    } else {
        9
    };
    TEMP_CLASSES[i]
}

/// Gives `label` the colour of `celsius`, instead of the one it had.
pub fn colour_temp(label: &gtk::Label, celsius: f64) {
    use gtk::prelude::WidgetExt;

    let class = temp_class(celsius);
    for other in TEMP_CLASSES.iter().filter(|c| **c != class) {
        label.remove_css_class(other);
    }
    label.add_css_class(class);
}

/// WMO weather code -> Nerd Font glyph, when there are no picture icons.
pub fn glyph(code: i32) -> &'static str {
    match code {
        0 => "\u{f0599}",                                // 󰖙
        1 | 2 => "\u{f0595}",                            // 󰖕
        45 | 48 => "\u{f0591}",                          // 󰖑
        51 | 53 | 55 | 56 | 57 | 61 | 80 => "\u{f0597}", // 󰖗
        63 | 65 | 66 | 67 | 81 | 82 => "\u{f0596}",      // 󰖖
        71 | 73 | 75 | 77 | 85 | 86 => "\u{f0598}",      // 󰖘
        95 | 96 | 99 => "\u{f067e}",                     // 󰙾
        _ => "\u{f0590}",                                // 󰖐
    }
}

/// WMO weather code -> description, in the user's language.
pub fn description(code: i32) -> String {
    tr(&format!("weather-code-{}", description_id(code)))
}

/// The catalog message for a WMO code: codes that read the same share one.
fn description_id(code: i32) -> String {
    match code {
        0 | 1 | 2 | 3 | 45 | 48 | 51 | 53 | 55 | 56 | 61 | 63 | 65 | 66 | 71 | 73 | 75 | 80
        | 81 | 82 | 85 | 95 | 96 => code.to_string(),
        57 => "56".into(),
        67 => "66".into(),
        77 => "73".into(),
        86 => "85".into(),
        99 => "96".into(),
        _ => "unknown".into(),
    }
}

/// Degrees -> point of the 16-point compass rose (0 = north, clockwise).
fn compass_index(degrees: f64) -> usize {
    ((degrees / 22.5).round() as i64).rem_euclid(16) as usize
}

/// Degrees -> compass point in the user's language (N NNE NE… in English,
/// N NNL NL… in Portuguese, where east is L and west O).
pub fn compass(degrees: f64) -> String {
    const ENGLISH: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    let index = compass_index(degrees);
    let rose = tr("weather-compass");
    let points: Vec<&str> = rose.split_whitespace().collect();
    points
        .get(index)
        .filter(|_| points.len() == 16)
        .copied()
        .unwrap_or(ENGLISH[index])
        .to_string()
}

/// The moon now: (image index 0..36 — 0 new, 18 full —, phase name in the
/// user's language, % lit). Age from a known new moon (2000-01-06 18:14 UTC)
/// over the synodic month.
pub fn moon() -> (usize, String, u32) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    let (index, id, lit) = moon_at(now);
    (index, tr(id), lit)
}

/// (image index, phase message id, % lit) at `unix`.
fn moon_at(unix: f64) -> (usize, &'static str, u32) {
    const SYNODIC: f64 = 29.530_588_853;
    let age = ((unix - 947_182_440.0) / 86_400.0).rem_euclid(SYNODIC);
    let f = age / SYNODIC;
    let index = (f * 36.0).round() as usize % 36;
    let id = match f {
        f if !(0.02..0.98).contains(&f) => "moon-new",
        f if f < 0.23 => "moon-waxing-crescent",
        f if f < 0.27 => "moon-first-quarter",
        f if f < 0.48 => "moon-waxing-gibbous",
        f if f < 0.52 => "moon-full",
        f if f < 0.73 => "moon-waning-gibbous",
        f if f < 0.77 => "moon-last-quarter",
        _ => "moon-waning-crescent",
    };
    let lit = ((1.0 - (2.0 * std::f64::consts::PI * f).cos()) / 2.0 * 100.0).round() as u32;
    (index, id, lit)
}

/// The season, in the user's language.
pub fn season(month: i32, day: i32, lat: f64) -> String {
    tr(season_id(month, day, lat))
}

/// The season's message id by mean equinox/solstice dates, for the
/// hemisphere of `lat`.
fn season_id(month: i32, day: i32, lat: f64) -> &'static str {
    let md = month * 100 + day;
    let north = match md {
        md if !(320..1221).contains(&md) => 0, // winter
        md if md < 621 => 1,                   // spring
        md if md < 922 => 2,                   // summer
        _ => 3,                                // autumn
    };
    let season = if lat < 0.0 { (north + 2) % 4 } else { north };
    [
        "season-winter",
        "season-spring",
        "season-summer",
        "season-autumn",
    ][season]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temperatures_take_the_scale_colours() {
        let c = |t| temp_class(t);
        assert_eq!([c(-35.0), c(-30.0), c(-25.0), c(-20.0), c(-15.0), c(-10.0), c(-3.0)], ["temp-bitter", "temp-bitter", "temp-m30", "temp-m30", "temp-m20", "temp-m20", "temp-m10"]);
        // As shown, rounded: −0.4 and 0.3 are 0°.
        assert_eq!([c(-0.4), c(0.3), c(0.6), c(9.4), c(10.0)], ["temp-zero", "temp-zero", "temp-0", "temp-0", "temp-10"]);
        assert_eq!([c(19.6), c(25.0), c(30.0), c(39.4), c(39.6), c(45.0)], ["temp-20", "temp-20", "temp-30", "temp-30", "temp-40", "temp-40"]);
    }

    #[test]
    fn compass_points() {
        assert_eq!(compass_index(0.0), 0);
        assert_eq!(compass_index(90.0), 4);
        assert_eq!(compass_index(180.0), 8);
        assert_eq!(compass_index(270.0), 12);
        assert_eq!(compass_index(359.0), 0);
    }

    #[test]
    fn seasons_flip_south() {
        assert_eq!(season_id(10, 19, -23.6), "season-spring");
        assert_eq!(season_id(10, 19, 40.0), "season-autumn");
        assert_eq!(season_id(1, 10, -23.6), "season-summer");
    }

    #[test]
    fn moon_matches_known_dates() {
        // 2026-10-19 15:00 UTC: waxing gibbous, about 60% lit (as the eww script had it).
        let (index, id, lit) = moon_at(1_792_422_000.0);
        assert_eq!(id, "moon-waxing-gibbous");
        assert!((55..=65).contains(&lit), "{lit}");
        assert!((9..=12).contains(&index), "{index}");
        // The reference new moon itself.
        assert_eq!(moon_at(947_182_440.0).1, "moon-new");
    }

    #[test]
    fn every_weather_code_has_a_message_id() {
        for code in [
            0, 1, 2, 3, 45, 48, 51, 53, 55, 56, 57, 61, 63, 65, 66, 67, 71, 73, 75, 77, 80, 81, 82,
            85, 86, 95, 96, 99,
        ] {
            assert_ne!(description_id(code), "unknown", "{code}");
        }
        assert_eq!(description_id(42), "unknown");
    }
}
