//! What the `earth` and `sky` wallpaper effects know of the world: where
//! the user is, and the weather there. Nothing is fetched here: the place
//! is `latitude`/`longitude` in the panel's `modules/weather.yaml`, else the
//! one the panel's weather found (`~/.cache/orbit/weather.json`, written
//! every `interval` minutes), and the weather is that file's, while it is
//! less than three hours old. Without them, the longitude comes from the
//! clock's time zone and the sky is clear.

use std::{
    path::PathBuf,
    time::{Duration, Instant, SystemTime},
};

use serde::Deserialize;

/// Where the user is, radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub lat: f64,
    pub lon: f64,
    /// Known (configured, or found by the panel), not guessed from the
    /// time zone.
    pub known: bool,
}

/// The weather, each 0..1.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Weather {
    /// How much of the sky the clouds cover.
    pub cover: f32,
    pub rain: f32,
    pub snow: f32,
    pub fog: f32,
    pub storm: f32,
    /// The wind: 0 still … 1 a gale (60 km/h and over), and where it blows
    /// to (radians, from north through east).
    pub wind: f32,
    pub wind_to: f32,
}

/// Read again at most once a minute.
const FRESH: Duration = Duration::from_secs(60);
/// Weather older than this is not believed.
const STALE: Duration = Duration::from_secs(3 * 3600);

#[derive(Default)]
pub struct World {
    read: Option<Instant>,
    place: Option<Place>,
    weather: Option<Weather>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Configured {
    latitude: Option<f64>,
    longitude: Option<f64>,
}

#[derive(Deserialize)]
struct Cached {
    latitude: f64,
    #[serde(default)]
    longitude: Option<f64>,
    code: i32,
    clouds: f64,
    wind_speed: f64,
    wind_direction: f64,
}

impl World {
    /// The place and the weather, read again when a minute has gone by.
    pub fn now(&mut self) -> (Place, Weather) {
        if self.read.is_none_or(|t| t.elapsed() >= FRESH) {
            self.read = Some(Instant::now());
            let (place, weather) = read();
            self.place = place;
            self.weather = weather;
        }
        (self.place.unwrap_or_else(guessed), self.weather.unwrap_or_default())
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

fn read() -> (Option<Place>, Option<Weather>) {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".config"))
        .join("orbit/modules/weather.yaml");
    let configured: Configured = std::fs::read_to_string(config)
        .ok()
        .and_then(|t| serde_yaml_ng::from_str(&t).ok())
        .unwrap_or_default();
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".cache"))
        .join("orbit/weather.json");
    let fresh = std::fs::metadata(&cache)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .is_some_and(|age| age < STALE);
    let cached: Option<Cached> = std::fs::read(&cache).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let place = match (configured.latitude, configured.longitude, &cached) {
        (Some(lat), Some(lon), _) => Some((lat, lon)),
        (_, _, Some(c)) => c.longitude.map(|lon| (c.latitude, lon)),
        _ => None,
    }
    .map(|(lat, lon)| Place {
        lat: lat.to_radians(),
        lon: lon.to_radians(),
        known: true,
    });
    let weather = cached.filter(|_| fresh).map(|c| weather(&c));
    (place, weather)
}

/// From the WMO code Open-Meteo gives, the cloud cover and the wind.
fn weather(c: &Cached) -> Weather {
    let (rain, snow, fog, storm) = match c.code {
        45 | 48 => (0.0, 0.0, 1.0, 0.0),
        51 | 56 => (0.15, 0.0, 0.2, 0.0),
        53 | 57 => (0.25, 0.0, 0.2, 0.0),
        55 => (0.35, 0.0, 0.2, 0.0),
        61 | 66 => (0.4, 0.0, 0.0, 0.0),
        63 => (0.65, 0.0, 0.0, 0.0),
        65 | 67 => (0.95, 0.0, 0.0, 0.0),
        71 => (0.0, 0.4, 0.0, 0.0),
        73 => (0.0, 0.7, 0.0, 0.0),
        75 => (0.0, 1.0, 0.0, 0.0),
        77 => (0.0, 0.3, 0.0, 0.0),
        80 => (0.45, 0.0, 0.0, 0.0),
        81 => (0.75, 0.0, 0.0, 0.0),
        82 => (1.0, 0.0, 0.0, 0.0),
        85 => (0.0, 0.6, 0.0, 0.0),
        86 => (0.0, 1.0, 0.0, 0.0),
        95 => (0.8, 0.0, 0.0, 1.0),
        96 | 99 => (1.0, 0.0, 0.0, 1.0),
        _ => (0.0, 0.0, 0.0, 0.0),
    };
    let mut cover = (c.clouds / 100.0).clamp(0.0, 1.0) as f32;
    if rain + snow + storm > 0.0 {
        cover = cover.max(0.85);
    }
    Weather {
        cover,
        rain,
        snow,
        fog,
        storm,
        wind: (c.wind_speed / 60.0).clamp(0.0, 1.0) as f32,
        // Open-Meteo gives where it blows from.
        wind_to: ((c.wind_direction + 180.0) % 360.0).to_radians() as f32,
    }
}

/// No place known: the longitude of the clock's time zone, on the equator.
fn guessed() -> Place {
    let offset = local_offset_seconds().unwrap_or(0);
    Place {
        lat: 0.0,
        lon: (offset as f64 / 3600.0 * 15.0).to_radians(),
        known: false,
    }
}

/// The local time zone's offset from UTC, from `date` (the core links no
/// time zone database).
fn local_offset_seconds() -> Option<i64> {
    let out = std::process::Command::new("date").arg("+%z").output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let text = text.trim();
    let sign = if text.starts_with('-') { -1 } else { 1 };
    let digits = text.trim_start_matches(['+', '-']);
    let hours: i64 = digits.get(..2)?.parse().ok()?;
    let minutes: i64 = digits.get(2..4)?.parse().ok()?;
    Some(sign * (hours * 3600 + minutes * 60))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cached(code: i32, clouds: f64) -> Cached {
        Cached {
            latitude: -22.8,
            longitude: Some(-43.2),
            code,
            clouds,
            wind_speed: 30.0,
            wind_direction: 90.0,
        }
    }

    #[test]
    fn rain_covers_the_sky() {
        let w = weather(&cached(63, 20.0));
        assert!(w.rain > 0.5 && w.cover >= 0.85);
    }

    #[test]
    fn clear_sky() {
        let w = weather(&cached(0, 0.0));
        assert_eq!((w.rain, w.snow, w.cover), (0.0, 0.0, 0.0));
        assert!((w.wind - 0.5).abs() < 1e-6);
        // From the east, so to the west.
        assert!((w.wind_to - 270f32.to_radians()).abs() < 1e-4);
    }
}
