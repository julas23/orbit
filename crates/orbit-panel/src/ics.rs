//! Calendars read from iCalendar links (`.ics`): Google's "secret address
//! in iCal format", Outlook's "publish a calendar" ICS link, or any other.
//! Read only: the agenda shows the events, it does not change them.
//!
//! The parser keeps to what calendars publish: VEVENTs with DTSTART,
//! DTEND or DURATION, SUMMARY, LOCATION, RRULE, EXDATE, RECURRENCE-ID and
//! STATUS. Recurrences are expanded by the `rrule` crate. Time zones are
//! IANA names (Google) or Windows names (Outlook), mapped to IANA.

use std::{collections::HashSet, str::FromStr};

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// A calendar of `modules/calendar.yaml`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Source {
    pub name: String,
    pub url: String,
    /// `#rrggbb`; by default one of the agenda's own.
    pub colour: Option<String>,
}

/// One event as it happens: a single one, or one time of a recurring one.
#[derive(Debug, Clone)]
pub struct Occurrence {
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    pub all_day: bool,
    pub summary: String,
    pub location: String,
    /// Index of its calendar.
    pub calendar: usize,
}

impl Occurrence {
    /// Whether it takes any part of `day`.
    pub fn on(&self, day: NaiveDate) -> bool {
        let first = self.start.date_naive();
        // An event ending at midnight does not take the day after.
        let last = (self.end - Duration::seconds(1)).date_naive().max(first);
        first <= day && day <= last
    }
}

/// A moment as written in the file.
#[derive(Debug, Clone, Copy)]
enum When {
    Date(NaiveDate),
    Utc(NaiveDateTime),
    Zoned(NaiveDateTime, chrono_tz::Tz),
    Floating(NaiveDateTime),
}

impl When {
    fn local(self) -> DateTime<Local> {
        let pick = |dt: chrono::LocalResult<DateTime<Local>>| dt.earliest();
        match self {
            When::Date(d) => pick(Local.from_local_datetime(&d.and_time(NaiveTime::MIN))),
            When::Floating(dt) => pick(Local.from_local_datetime(&dt)),
            When::Utc(dt) => Some(Utc.from_utc_datetime(&dt).with_timezone(&Local)),
            When::Zoned(dt, tz) => tz.from_local_datetime(&dt).earliest().map(|t| t.with_timezone(&Local)),
        }
        .unwrap_or_else(Local::now)
    }

    fn rrule_start(self) -> DateTime<rrule::Tz> {
        match self {
            When::Zoned(dt, tz) => rrule::Tz::Tz(tz)
                .from_local_datetime(&dt)
                .earliest()
                .unwrap_or_else(|| self.local().with_timezone(&rrule::Tz::LOCAL)),
            When::Utc(dt) => rrule::Tz::UTC.from_utc_datetime(&dt),
            _ => self.local().with_timezone(&rrule::Tz::LOCAL),
        }
    }
}

#[derive(Debug, Default)]
struct Event {
    uid: String,
    start: Option<When>,
    end: Option<When>,
    duration: Option<Duration>,
    summary: String,
    location: String,
    rrule: Option<String>,
    exdates: Vec<When>,
    recurrence_id: Option<When>,
    cancelled: bool,
}

/// The lines of `text` with the folded ones joined (RFC 5545 §3.1).
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix([' ', '\t'])
            && let Some(last) = lines.last_mut()
        {
            last.push_str(rest);
        } else {
            lines.push(line.to_string());
        }
    }
    lines
}

/// A content line: its name, its parameters and its value.
type Property = (String, Vec<(String, String)>, String);

/// `NAME;PARAM=V;…:value` → (NAME, params, value).
fn split(line: &str) -> Option<Property> {
    // The colon outside quotes ends the name and parameters.
    let mut quoted = false;
    let at = line.char_indices().find(|&(_, c)| {
        if c == '"' {
            quoted = !quoted;
        }
        c == ':' && !quoted
    })?;
    let (head, value) = (&line[..at.0], &line[at.0 + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.to_ascii_uppercase();
    let params = parts
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_ascii_uppercase(), v.trim_matches('"').to_string()))
        .collect();
    Some((name, params, value.to_string()))
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A zone named in a TZID: IANA, or Windows (Outlook).
fn zone(name: &str) -> Option<chrono_tz::Tz> {
    let name = name.trim_matches('/');
    chrono_tz::Tz::from_str(name)
        .ok()
        .or_else(|| windows_zone(name).and_then(|iana| chrono_tz::Tz::from_str(iana).ok()))
}

/// The Windows time-zone names Outlook writes, for the IANA ones.
fn windows_zone(name: &str) -> Option<&'static str> {
    Some(match name {
        "E. South America Standard Time" => "America/Sao_Paulo",
        "SA Eastern Standard Time" => "America/Cayenne",
        "Bahia Standard Time" => "America/Bahia",
        "Tocantins Standard Time" => "America/Araguaina",
        "Central Brazilian Standard Time" => "America/Cuiaba",
        "SA Western Standard Time" => "America/La_Paz",
        "SA Pacific Standard Time" => "America/Bogota",
        "Argentina Standard Time" => "America/Buenos_Aires",
        "Pacific SA Standard Time" => "America/Santiago",
        "Montevideo Standard Time" => "America/Montevideo",
        "Paraguay Standard Time" => "America/Asuncion",
        "Venezuela Standard Time" => "America/Caracas",
        "UTC-02" | "Mid-Atlantic Standard Time" => "Etc/GMT+2",
        "Eastern Standard Time" => "America/New_York",
        "Central Standard Time" => "America/Chicago",
        "Mountain Standard Time" => "America/Denver",
        "US Mountain Standard Time" => "America/Phoenix",
        "Pacific Standard Time" => "America/Los_Angeles",
        "Alaskan Standard Time" => "America/Anchorage",
        "Hawaiian Standard Time" => "Pacific/Honolulu",
        "Atlantic Standard Time" => "America/Halifax",
        "Newfoundland Standard Time" => "America/St_Johns",
        "Central Standard Time (Mexico)" => "America/Mexico_City",
        "Canada Central Standard Time" => "America/Regina",
        "UTC" | "Coordinated Universal Time" => "UTC",
        "GMT Standard Time" => "Europe/London",
        "Greenwich Standard Time" => "Atlantic/Reykjavik",
        "W. Europe Standard Time" => "Europe/Berlin",
        "Romance Standard Time" => "Europe/Paris",
        "Central Europe Standard Time" => "Europe/Budapest",
        "Central European Standard Time" => "Europe/Warsaw",
        "E. Europe Standard Time" => "Europe/Chisinau",
        "FLE Standard Time" => "Europe/Kiev",
        "GTB Standard Time" => "Europe/Bucharest",
        "Russian Standard Time" => "Europe/Moscow",
        "Turkey Standard Time" => "Europe/Istanbul",
        "Israel Standard Time" => "Asia/Jerusalem",
        "South Africa Standard Time" => "Africa/Johannesburg",
        "Egypt Standard Time" => "Africa/Cairo",
        "Arabian Standard Time" => "Asia/Dubai",
        "Arab Standard Time" => "Asia/Riyadh",
        "India Standard Time" => "Asia/Kolkata",
        "China Standard Time" => "Asia/Shanghai",
        "Singapore Standard Time" => "Asia/Singapore",
        "Tokyo Standard Time" => "Asia/Tokyo",
        "Korea Standard Time" => "Asia/Seoul",
        "AUS Eastern Standard Time" => "Australia/Sydney",
        "E. Australia Standard Time" => "Australia/Brisbane",
        "W. Australia Standard Time" => "Australia/Perth",
        "New Zealand Standard Time" => "Pacific/Auckland",
        "Portugal Standard Time" => "Europe/Lisbon",
        _ => return None,
    })
}

fn when(value: &str, params: &[(String, String)]) -> Option<When> {
    let value = value.trim();
    let param = |k: &str| params.iter().find(|(name, _)| name == k).map(|(_, v)| v.as_str());
    if param("VALUE") == Some("DATE") || value.len() == 8 {
        return NaiveDate::parse_from_str(value, "%Y%m%d").ok().map(When::Date);
    }
    let (text, utc) = match value.strip_suffix('Z') {
        Some(t) => (t, true),
        None => (value, false),
    };
    let dt = NaiveDateTime::parse_from_str(text, "%Y%m%dT%H%M%S").ok()?;
    Some(if utc {
        When::Utc(dt)
    } else if let Some(tz) = param("TZID").and_then(zone) {
        When::Zoned(dt, tz)
    } else {
        When::Floating(dt)
    })
}

/// An ISO 8601 duration as iCalendar has it: `P1D`, `PT1H30M`, `P2W`.
fn duration(text: &str) -> Option<Duration> {
    let (negative, text) = match text.strip_prefix('-') {
        Some(t) => (true, t),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let mut rest = text.strip_prefix('P')?;
    let mut total = Duration::zero();
    let mut in_time = false;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('T') {
            in_time = true;
            rest = r;
            continue;
        }
        let digits = rest.find(|c: char| !c.is_ascii_digit())?;
        let n: i64 = rest[..digits].parse().ok()?;
        let unit = rest[digits..].chars().next()?;
        total += match (unit, in_time) {
            ('W', _) => Duration::weeks(n),
            ('D', _) => Duration::days(n),
            ('H', true) => Duration::hours(n),
            ('M', true) => Duration::minutes(n),
            ('S', true) => Duration::seconds(n),
            _ => return None,
        };
        rest = &rest[digits + 1..];
    }
    Some(if negative { -total } else { total })
}

fn parse(text: &str) -> Vec<Event> {
    let mut events = Vec::new();
    let mut current: Option<Event> = None;
    // Nested components (VALARM) inside an event are skipped.
    let mut depth = 0;
    for line in unfold(text) {
        let Some((name, params, value)) = split(&line) else { continue };
        match (name.as_str(), value.to_ascii_uppercase().as_str()) {
            ("BEGIN", "VEVENT") if current.is_none() => {
                current = Some(Event::default());
                depth = 0;
                continue;
            }
            ("END", "VEVENT") if depth == 0 => {
                if let Some(event) = current.take() {
                    events.push(event);
                }
                continue;
            }
            ("BEGIN", _) if current.is_some() => {
                depth += 1;
                continue;
            }
            ("END", _) if current.is_some() => {
                depth -= 1;
                continue;
            }
            _ => {}
        }
        let Some(event) = current.as_mut().filter(|_| depth == 0) else {
            continue;
        };
        match name.as_str() {
            "UID" => event.uid = value,
            "DTSTART" => event.start = when(&value, &params),
            "DTEND" => event.end = when(&value, &params),
            "DURATION" => event.duration = duration(&value),
            "SUMMARY" => event.summary = unescape(&value),
            "LOCATION" => event.location = unescape(&value),
            "RRULE" => event.rrule = Some(value),
            "EXDATE" => event.exdates.extend(value.split(',').filter_map(|v| when(v, &params))),
            "RECURRENCE-ID" => event.recurrence_id = when(&value, &params),
            "STATUS" => event.cancelled = value.eq_ignore_ascii_case("CANCELLED"),
            _ => {}
        }
    }
    events
}

/// The occurrences of calendar `index`'s `text` that touch [`from`, `to`).
pub fn occurrences(text: &str, index: usize, from: DateTime<Local>, to: DateTime<Local>) -> Vec<Occurrence> {
    let events = parse(text);
    // Times of a recurring event moved or cancelled one by one.
    let moved: HashSet<(String, i64)> = events
        .iter()
        .filter_map(|e| Some((e.uid.clone(), e.recurrence_id?.local().timestamp())))
        .collect();
    let mut out = Vec::new();
    for event in &events {
        let Some(start) = event.start else { continue };
        let all_day = matches!(start, When::Date(_));
        let first = start.local();
        let length = match (event.end, event.duration) {
            (Some(end), _) => end.local() - first,
            (None, Some(d)) => d,
            (None, None) if all_day => Duration::days(1),
            (None, None) => Duration::zero(),
        }
        .max(Duration::zero());
        let mut push = |start: DateTime<Local>| {
            let end = start + length;
            if end.max(start + Duration::seconds(1)) > from && start < to && !event.cancelled {
                out.push(Occurrence {
                    start,
                    end: end.max(start),
                    all_day,
                    summary: event.summary.clone(),
                    location: event.location.clone(),
                    calendar: index,
                });
            }
        };
        let Some(rule) = event.rrule.as_deref().filter(|_| event.recurrence_id.is_none()) else {
            push(first);
            continue;
        };
        let set = rrule::RRule::from_str(rule)
            .map_err(|e| e.to_string())
            .and_then(|r| r.build(start.rrule_start()).map_err(|e| e.to_string()));
        let set = match set {
            Ok(set) => event
                .exdates
                .iter()
                .fold(set, |set, ex| set.exdate(ex.local().with_timezone(&rrule::Tz::LOCAL))),
            Err(err) => {
                log::warn!("calendar: {:?}: {err}; only its first time is shown", event.summary);
                push(first);
                continue;
            }
        };
        let after = (from - length - Duration::days(1)).with_timezone(&rrule::Tz::LOCAL);
        let before = to.with_timezone(&rrule::Tz::LOCAL);
        for time in set.after(after).before(before).all(2000).dates {
            let time = time.with_timezone(&Local);
            if !moved.contains(&(event.uid.clone(), time.timestamp())) {
                push(time);
            }
        }
    }
    out
}

/// `webcal://` is the same address over HTTPS.
pub fn http_url(url: &str) -> String {
    match url.strip_prefix("webcal://") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_string(),
    }
}

/// Where calendar `url`'s last copy is kept.
pub fn cache_path(url: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    gtk::glib::user_cache_dir()
        .join("orbit")
        .join("calendars")
        .join(format!("{:016x}.ics", hasher.finish()))
}

/// Downloads `url` (blocking: call it on a thread) and keeps a copy.
pub fn fetch(url: &str) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build()
        .into();
    let text = agent
        .get(&http_url(url))
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .with_config()
        .limit(32 * 1024 * 1024)
        .read_to_string()
        .map_err(|e| e.to_string())?;
    if !text.contains("BEGIN:VCALENDAR") {
        return Err("not an iCalendar file".into());
    }
    let path = cache_path(url);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, &text);
    Ok(text)
}

/// The copy kept by the last [`fetch`] of `url`.
pub fn cached(url: &str) -> Option<String> {
    std::fs::read_to_string(cache_path(url)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nDTSTART;TZID=America/Sao_Paulo:20261102T090000\r\nDTEND;TZID=America/Sao_Paulo:20261102T100000\r\nRRULE:FREQ=WEEKLY;COUNT=3\r\nEXDATE;TZID=America/Sao_Paulo:20261109T090000\r\nSUMMARY:Weekly\\, team\r\n sync\r\nBEGIN:VALARM\r\nSUMMARY:alarm\r\nEND:VALARM\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:b\r\nDTSTART;VALUE=DATE:20261105\r\nDTEND;VALUE=DATE:20261107\r\nSUMMARY:Trip\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:c\r\nDTSTART;TZID=E. South America Standard Time:20261104T150000\r\nDURATION:PT30M\r\nSUMMARY:Outlook\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn parses_and_expands() {
        let from = Local.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap();
        let to = Local.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap();
        let mut found = occurrences(SAMPLE, 0, from, to);
        found.sort_by_key(|o| o.start);
        let names: Vec<&str> = found.iter().map(|o| o.summary.as_str()).collect();
        // Three weeks, the second excluded.
        assert_eq!(names, ["Weekly, teamsync", "Outlook", "Trip", "Weekly, teamsync"]);
        let trip = found.iter().find(|o| o.summary == "Trip").unwrap();
        assert!(trip.all_day);
        assert!(trip.on(NaiveDate::from_ymd_opt(2026, 11, 6).unwrap()));
        assert!(!trip.on(NaiveDate::from_ymd_opt(2026, 11, 7).unwrap()));
        let outlook = found.iter().find(|o| o.summary == "Outlook").unwrap();
        assert_eq!(outlook.end - outlook.start, Duration::minutes(30));
    }

    #[test]
    fn durations() {
        assert_eq!(duration("PT1H30M"), Some(Duration::minutes(90)));
        assert_eq!(duration("P1W"), Some(Duration::weeks(1)));
        assert_eq!(duration("P1DT2H"), Some(Duration::hours(26)));
    }
}
