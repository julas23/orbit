//! Where the Sun and the Moon are, for the `earth` and `sky` wallpaper
//! effects: low-precision formulas (the Astronomical Almanac's, a fraction
//! of a degree off), enough for a picture.

use std::time::{SystemTime, UNIX_EPOCH};

/// Days since J2000.0 (2000-01-01 12:00 UTC) at `time`.
pub fn days(time: SystemTime) -> f64 {
    let secs = time.duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
    secs / 86_400.0 + 2_440_587.5 - 2_451_545.0
}

/// A body on the sky: right ascension and declination, radians.
#[derive(Debug, Clone, Copy)]
pub struct Equatorial {
    pub ra: f64,
    pub dec: f64,
}

fn obliquity(n: f64) -> f64 {
    (23.439 - 0.000_000_4 * n).to_radians()
}

/// From ecliptic longitude and latitude (radians).
fn equatorial(lambda: f64, beta: f64, n: f64) -> Equatorial {
    let e = obliquity(n);
    let ra = (lambda.sin() * e.cos() - beta.tan() * e.sin()).atan2(lambda.cos());
    let dec = (beta.sin() * e.cos() + beta.cos() * e.sin() * lambda.sin()).asin();
    Equatorial { ra, dec }
}

/// The Sun's ecliptic longitude, radians.
fn sun_longitude(n: f64) -> f64 {
    let l = 280.460 + 0.985_647_4 * n;
    let g = (357.528 + 0.985_600_3 * n).to_radians();
    (l + 1.915 * g.sin() + 0.020 * (2.0 * g).sin()).to_radians()
}

pub fn sun(n: f64) -> Equatorial {
    equatorial(sun_longitude(n), 0.0, n)
}

/// The Moon's ecliptic longitude and latitude, radians.
fn moon_ecliptic(n: f64) -> (f64, f64) {
    let l = 218.316 + 13.176_396 * n;
    let m = (134.963 + 13.064_993 * n).to_radians();
    let f = (93.272 + 13.229_350 * n).to_radians();
    let d = (297.850 + 12.190_749 * n).to_radians();
    let lambda = l + 6.289 * m.sin() + 1.274 * (2.0 * d - m).sin() + 0.658 * (2.0 * d).sin();
    (lambda.to_radians(), (5.128 * f.sin()).to_radians())
}

pub fn moon(n: f64) -> Equatorial {
    let (lambda, beta) = moon_ecliptic(n);
    equatorial(lambda, beta, n)
}

/// How much of the Moon is lit (0 new … 1 full), and whether it grows.
pub fn moon_phase(n: f64) -> (f64, bool) {
    let elongation = moon_ecliptic(n).0 - sun_longitude(n);
    ((1.0 - elongation.cos()) / 2.0, elongation.sin() > 0.0)
}

/// Greenwich sidereal time, radians.
fn sidereal(n: f64) -> f64 {
    (280.460_618_37 + 360.985_647_366_29 * n).to_radians()
}

/// Where `body` is straight overhead: latitude and longitude, radians.
pub fn overhead(body: Equatorial, n: f64) -> (f64, f64) {
    let lon = body.ra - sidereal(n);
    (body.dec, wrap(lon))
}

/// `body` seen from `lat`, `lon` (radians): altitude over the horizon and
/// azimuth from north through east, radians.
pub fn horizontal(body: Equatorial, n: f64, lat: f64, lon: f64) -> (f64, f64) {
    let hour = sidereal(n) + lon - body.ra;
    let alt = (lat.sin() * body.dec.sin() + lat.cos() * body.dec.cos() * hour.cos()).asin();
    let az = (-hour.sin()).atan2(body.dec.tan() * lat.cos() - lat.sin() * hour.cos());
    (alt, wrap(az))
}

/// An angle into -π..π.
pub fn wrap(a: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    (a + std::f64::consts::PI).rem_euclid(tau) - std::f64::consts::PI
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> f64 {
        days(UNIX_EPOCH + std::time::Duration::from_secs(secs))
    }

    #[test]
    fn june_solstice_sun_over_the_tropic() {
        // 2024-06-20 20:51 UTC: the Sun over the Tropic of Cancer.
        let n = at(1_718_916_660);
        let (lat, _) = overhead(sun(n), n);
        assert!((lat.to_degrees() - 23.44).abs() < 0.1, "{}", lat.to_degrees());
    }

    #[test]
    fn noon_at_greenwich() {
        // 2024-03-20 12:07 UTC (equinox, the equation of time ~ -7 min):
        // the Sun nearly over Greenwich and the equator.
        let n = at(1_710_936_420);
        let (lat, lon) = overhead(sun(n), n);
        assert!(lat.to_degrees().abs() < 0.5, "{}", lat.to_degrees());
        assert!(lon.to_degrees().abs() < 1.0, "{}", lon.to_degrees());
    }

    #[test]
    fn full_moon() {
        // 2024-04-23 23:49 UTC, a full moon.
        let (lit, _) = moon_phase(at(1_713_916_140));
        assert!(lit > 0.98, "{lit}");
    }

    #[test]
    fn sun_high_at_noon_in_sao_paulo() {
        // 2024-12-21 15:00 UTC, near noon in São Paulo (-23.5, -46.6): the
        // Sun almost overhead.
        let n = at(1_734_793_200);
        let (alt, _) = horizontal(sun(n), n, (-23.55f64).to_radians(), (-46.63f64).to_radians());
        assert!(alt.to_degrees() > 85.0, "{}", alt.to_degrees());
    }
}
