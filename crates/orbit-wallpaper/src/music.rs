//! `orbit-wallpaper --music`: what is playing, over the wallpaper's
//! spectrum sphere (the core starts it with `effect: scope`). The cover,
//! the title, the artist, the album and how far it has gone in the bottom
//! right corner; in the middle the lyrics, the line being sung brighter,
//! when there are lyrics.
//!
//! The music comes from any MPRIS player (Feishin, Spotify, a browser,
//! Orbit-Play): the one playing, else the first. The lyrics, in order:
//! what the player hands over (`xesam:asText`), an `.lrc` beside the file
//! played, and the Navidrome (or any OpenSubsonic) server of
//! `~/.config/orbit/music.yaml`:
//!
//! ```yaml
//! navidrome:
//!   url: https://music.example.com
//!   user: juliano
//!   password_command: secret-tool lookup service navidrome   # or password:
//! ```
//!
//! Nothing here takes input: clicks go through to the desktop.

use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{gdk, gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde::Deserialize;
use serde_json::Value;

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct Config {
    navidrome: Option<Server>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct Server {
    url: String,
    user: String,
    password: Option<String>,
    /// A command whose output is the password (a keyring lookup).
    password_command: Option<String>,
}

fn config() -> Config {
    let path = glib::user_config_dir().join("orbit").join("music.yaml");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| match serde_yaml_ng::from_str(&text) {
            Ok(config) => Some(config),
            Err(err) => {
                log::warn!("{}: {err}", path.display());
                None
            }
        })
        .unwrap_or_default()
}

/// The track as the player tells it.
#[derive(Debug, Clone, Default, PartialEq)]
struct Track {
    title: String,
    artist: String,
    album: String,
    art: String,
    url: String,
    trackid: String,
    lyrics: String,
    /// Microseconds.
    length: i64,
}

impl Track {
    fn key(&self) -> String {
        format!("{}\u{1}{}\u{1}{}", self.title, self.artist, self.album)
    }
}

/// What a poll found.
#[derive(Debug, Clone, Default)]
struct Now {
    track: Option<Track>,
    playing: bool,
    /// Microseconds, when polled.
    position: i64,
}

/// Lines of lyrics: when each starts (ms; `None` for lyrics that are not
/// timed) and its text.
type Lines = Vec<(Option<i64>, String)>;

// ------------------------------------------------------------- the player

async fn call(bus: &gio::DBusConnection, name: &str, property: &str) -> Option<glib::Variant> {
    bus.call_future(
        Some(name),
        PATH,
        "org.freedesktop.DBus.Properties",
        "Get",
        Some(&(PLAYER, property).to_variant()),
        Some(glib::VariantTy::new("(v)").expect("a type")),
        gio::DBusCallFlags::NONE,
        1000,
    )
    .await
    .ok()
    .and_then(|v| v.child_value(0).as_variant())
}

async fn players(bus: &gio::DBusConnection) -> Vec<String> {
    bus.call_future(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "ListNames",
        None,
        Some(glib::VariantTy::new("(as)").expect("a type")),
        gio::DBusCallFlags::NONE,
        1000,
    )
    .await
    .ok()
    .and_then(|v| v.child_value(0).get::<Vec<String>>())
    .unwrap_or_default()
    .into_iter()
    .filter(|n| n.starts_with(PREFIX))
    .collect()
}

fn text(meta: &HashMap<String, glib::Variant>, key: &str) -> String {
    let Some(v) = meta.get(key) else { return String::new() };
    if let Some(s) = v.str() {
        return s.to_string();
    }
    // Artists are a list.
    v.get::<Vec<String>>().map(|l| l.join(", ")).unwrap_or_default()
}

fn number(meta: &HashMap<String, glib::Variant>, key: &str) -> i64 {
    meta.get(key)
        .and_then(|v| v.get::<i64>().or_else(|| v.get::<u64>().map(|u| u as i64)).or_else(|| v.get::<i32>().map(i64::from)))
        .unwrap_or(0)
}

/// The player playing (else the first) and its track.
async fn poll(bus: &gio::DBusConnection) -> Now {
    let mut first: Option<Now> = None;
    for name in players(bus).await {
        let playing = call(bus, &name, "PlaybackStatus").await.and_then(|v| v.get::<String>()).as_deref() == Some("Playing");
        let Some(meta) = call(bus, &name, "Metadata").await.and_then(|v| v.get::<HashMap<String, glib::Variant>>()) else {
            continue;
        };
        let track = Track {
            title: text(&meta, "xesam:title"),
            artist: text(&meta, "xesam:artist"),
            album: text(&meta, "xesam:album"),
            art: text(&meta, "mpris:artUrl"),
            url: text(&meta, "xesam:url"),
            trackid: meta.get("mpris:trackid").and_then(|v| v.str().map(str::to_owned)).unwrap_or_default(),
            lyrics: text(&meta, "xesam:asText"),
            length: number(&meta, "mpris:length"),
        };
        if track.title.is_empty() {
            continue;
        }
        let position = call(bus, &name, "Position")
            .await
            .and_then(|v| v.get::<i64>())
            .unwrap_or(0);
        let now = Now {
            track: Some(track),
            playing,
            position,
        };
        if playing {
            return now;
        }
        first.get_or_insert(now);
    }
    first.unwrap_or_default()
}

// ------------------------------------------------------------- the lyrics

/// `[mm:ss.xx]text` lines (several stamps on one line, `[offset:ms]`);
/// lyrics without stamps come back untimed.
fn parse_lrc(text: &str) -> Lines {
    let mut offset = 0i64;
    let mut timed: Vec<(i64, String)> = Vec::new();
    let mut plain = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim();
        let mut stamps = Vec::new();
        while let Some(inner) = rest.strip_prefix('[') {
            let Some(end) = inner.find(']') else { break };
            let tag = &inner[..end];
            rest = inner[end + 1..].trim_start();
            if let Some(ms) = tag.strip_prefix("offset:") {
                offset = ms.trim().parse().unwrap_or(0);
            } else if let Some((m, s)) = tag.split_once(':')
                && let (Ok(m), Ok(s)) = (m.trim().parse::<i64>(), s.trim().parse::<f64>())
            {
                stamps.push(m * 60_000 + (s * 1000.0) as i64);
            }
        }
        if stamps.is_empty() {
            if !line.trim().is_empty() && !line.trim_start().starts_with('[') {
                plain.push(line.trim().to_string());
            }
        } else {
            for at in stamps {
                timed.push((at, rest.to_string()));
            }
        }
    }
    if timed.is_empty() {
        return plain.into_iter().map(|l| (None, l)).collect();
    }
    timed.sort_by_key(|(at, _)| *at);
    // A positive offset shows the lines sooner.
    timed.into_iter().map(|(at, l)| (Some(at - offset), l)).collect()
}

/// The `.lrc` beside a file played (`file://…/song.flac` → `song.lrc`).
fn beside(url: &str) -> Option<String> {
    let path = glib::filename_from_uri(url).ok()?.0;
    std::fs::read_to_string(path.with_extension("lrc")).ok()
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into()
}

fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

impl Server {
    fn password(&self) -> Option<String> {
        if let Some(command) = &self.password_command {
            let out = std::process::Command::new("sh").arg("-c").arg(command).output().ok()?;
            return Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|p| !p.is_empty());
        }
        self.password.clone()
    }

    /// `method` of the Subsonic API with `query`, its `subsonic-response`.
    fn get(&self, method: &str, query: &str) -> Result<Value, String> {
        let password = self.password().ok_or("no password")?;
        use md5::Digest;
        let salt = format!(
            "{:x}{:x}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
        );
        let token = format!("{:x}", md5::Md5::digest(format!("{password}{salt}").as_bytes()));
        let url = format!(
            "{}/rest/{method}?u={}&t={token}&s={salt}&v=1.16.1&c=orbit&f=json&{query}",
            self.url.trim_end_matches('/'),
            encode(&self.user),
        );
        let text = agent()
            .get(&url)
            .call()
            .map_err(|e| e.to_string())?
            .body_mut()
            .read_to_string()
            .map_err(|e| e.to_string())?;
        let json: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let reply = json["subsonic-response"].clone();
        if reply["status"] != "ok" {
            return Err(reply["error"]["message"].as_str().unwrap_or("failed").to_string());
        }
        Ok(reply)
    }

    /// The song's id on the server: in the stream's address (Feishin
    /// plays `…/rest/stream?id=…`), in the track id, or searched.
    fn song_id(&self, track: &Track) -> Option<String> {
        let from_query = |url: &str| {
            url.split(['?', '&'])
                .find_map(|part| part.strip_prefix("id="))
                .map(str::to_owned)
        };
        if let Some(id) = from_query(&track.url) {
            return Some(id);
        }
        let found = self
            .get("search3", &format!("query={}&songCount=20&albumCount=0&artistCount=0", encode(&track.title)))
            .ok()?;
        let songs = found["searchResult3"]["song"].as_array()?.clone();
        let fold = |s: &str| s.to_lowercase();
        songs
            .iter()
            .find(|s| {
                fold(s["title"].as_str().unwrap_or_default()) == fold(&track.title)
                    && (track.artist.is_empty() || fold(&track.artist).contains(&fold(s["artist"].as_str().unwrap_or_default())))
            })
            .or_else(|| songs.first())
            .and_then(|s| s["id"].as_str().map(str::to_owned))
    }

    fn lyrics(&self, track: &Track) -> Option<Lines> {
        if let Some(id) = self.song_id(track)
            && let Ok(reply) = self.get("getLyricsBySongId", &format!("id={}", encode(&id)))
        {
            let list = reply["lyricsList"]["structuredLyrics"].as_array().cloned().unwrap_or_default();
            let best = list.iter().find(|l| l["synced"] == true).or(list.first());
            if let Some(lyrics) = best {
                let synced = lyrics["synced"] == true;
                let offset = lyrics["offset"].as_i64().unwrap_or(0);
                let lines: Lines = lyrics["line"]
                    .as_array()
                    .map(|lines| {
                        lines
                            .iter()
                            .map(|l| {
                                (
                                    synced.then(|| l["start"].as_i64().unwrap_or(0) - offset),
                                    l["value"].as_str().unwrap_or_default().to_string(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if !lines.is_empty() {
                    return Some(lines);
                }
            }
        }
        let reply = self
            .get("getLyrics", &format!("artist={}&title={}", encode(&track.artist), encode(&track.title)))
            .ok()?;
        let text = reply["lyrics"]["value"].as_str()?.to_string();
        Some(parse_lrc(&text)).filter(|l| !l.is_empty())
    }
}

/// The track's lyrics, from wherever they are (blocking: on a thread).
fn find_lyrics(track: &Track, config: &Config) -> Lines {
    if !track.lyrics.trim().is_empty() {
        return parse_lrc(&track.lyrics);
    }
    if let Some(text) = beside(&track.url) {
        return parse_lrc(&text);
    }
    if let Some(server) = config.navidrome.as_ref().filter(|s| !s.url.is_empty())
        && let Some(lines) = server.lyrics(track)
    {
        return lines;
    }
    Vec::new()
}

/// The cover's bytes (blocking: on a thread).
fn find_cover(url: &str) -> Option<Vec<u8>> {
    if url.starts_with("file://") {
        let path = glib::filename_from_uri(url).ok()?.0;
        return std::fs::read(path).ok();
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        let mut reply = agent().get(url).call().ok()?;
        return reply.body_mut().with_config().limit(16 * 1024 * 1024).read_to_vec().ok();
    }
    None
}

/// The line being sung at `ms` of a track `length_ms` long: by its stamp,
/// or, for untimed lyrics, spread evenly over the track.
fn line_at(lines: &Lines, ms: i64, length_ms: i64) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    if lines[0].0.is_none() {
        if length_ms <= 0 {
            return Some(0);
        }
        let i = (ms.max(0) as f64 / length_ms as f64 * lines.len() as f64) as usize;
        return Some(i.min(lines.len() - 1));
    }
    lines.iter().rposition(|(at, _)| at.is_some_and(|at| at <= ms))
}

// --------------------------------------------------------------- the view

struct View {
    window: gtk::Window,
    card: gtk::Box,
    cover: gtk::Picture,
    title: gtk::Label,
    artist: gtk::Label,
    album: gtk::Label,
    progress: gtk::ProgressBar,
    time: gtk::Label,
    lyrics: gtk::Box,
    before: gtk::Label,
    current: gtk::Label,
    after: gtk::Label,
}

const CSS: &str = "
window.orbit-music { background: none; }
.music-card {
  background-color: rgba(8, 10, 18, 0.55); border-radius: 14px; padding: 14px;
  border: 1px solid rgba(255, 255, 255, 0.08); color: #e5e9f0;
}
.music-cover { border-radius: 8px; }
.music-title { font-size: 18px; font-weight: bold; }
.music-artist { font-size: 14px; color: #c8d0dc; }
.music-album { font-size: 12px; color: #8a93a3; }
.music-time { font-size: 11px; color: #8a93a3; }
.music-card progressbar trough { border: none; min-height: 3px; background-color: rgba(255,255,255,0.12); border-radius: 2px; }
.music-card progressbar progress { min-height: 3px; background-color: #22d3ee; border-radius: 2px; }
.lyrics label { color: #e5e9f0; text-shadow: 0 0 12px rgba(0, 0, 0, 0.9), 0 0 3px rgba(0, 0, 0, 0.9); }
.lyrics .current { font-size: 34px; font-weight: bold; color: #ffffff; }
.lyrics .near { font-size: 20px; opacity: 0.45; }
";

fn label(class: &str) -> gtk::Label {
    gtk::Label::builder()
        .css_classes([class])
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(34)
        .build()
}

fn view(app: &gtk::Application, monitor: &gdk::Monitor) -> View {
    let window = gtk::Window::builder().application(app).css_classes(["orbit-music"]).build();
    window.init_layer_shell();
    window.set_namespace(Some("orbit-wallpaper"));
    window.set_layer(Layer::Background);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    // Clear of the panel and the dock: the card is not hidden under them.
    window.set_exclusive_zone(0);
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_monitor(Some(monitor));

    let cover = gtk::Picture::builder()
        .css_classes(["music-cover"])
        .content_fit(gtk::ContentFit::Cover)
        .width_request(128)
        .height_request(128)
        .build();
    let (title, artist, album) = (label("music-title"), label("music-artist"), label("music-album"));
    let progress = gtk::ProgressBar::builder().valign(gtk::Align::End).vexpand(true).build();
    let time = label("music-time");
    let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(3).width_request(260).build();
    text.append(&title);
    text.append(&artist);
    text.append(&album);
    text.append(&progress);
    text.append(&time);
    let card = gtk::Box::builder()
        .css_classes(["music-card"])
        .spacing(14)
        .halign(gtk::Align::End)
        .valign(gtk::Align::End)
        .margin_end(28)
        .margin_bottom(28)
        .visible(false)
        .build();
    card.append(&cover);
    card.append(&text);

    let line = |class: &str| {
        gtk::Label::builder()
            .css_classes([class])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .max_width_chars(48)
            .build()
    };
    let (before, current, after) = (line("near"), line("current"), line("near"));
    let lyrics = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(["lyrics"])
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .visible(false)
        .build();
    lyrics.append(&before);
    lyrics.append(&current);
    lyrics.append(&after);

    let overlay = gtk::Overlay::builder().child(&gtk::Box::new(gtk::Orientation::Vertical, 0)).build();
    overlay.add_overlay(&lyrics);
    overlay.add_overlay(&card);
    window.set_child(Some(&overlay));
    // Nothing here takes a click: they reach the desktop.
    window.connect_realize(|window| {
        if let Some(surface) = window.surface() {
            surface.set_input_region(Some(&gtk::cairo::Region::create()));
        }
    });
    View {
        window,
        card,
        cover,
        title,
        artist,
        album,
        progress,
        time,
        lyrics,
        before,
        current,
        after,
    }
}

fn clock(us: i64) -> String {
    let s = us.max(0) / 1_000_000;
    format!("{}:{:02}", s / 60, s % 60)
}

#[derive(Default)]
struct State {
    now: Now,
    polled: Option<Instant>,
    key: String,
    lines: Lines,
    cover: Option<gdk::Texture>,
    shown_line: Option<Option<usize>>,
}

pub fn run(app: &gtk::Application) {
    let Some(display) = gdk::Display::default() else { return };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        log::error!("music: no session bus");
        return;
    };
    let views: Rc<RefCell<Vec<View>>> = Rc::default();
    let state: Rc<RefCell<State>> = Rc::default();
    let make = {
        let (app, views, state) = (app.clone(), views.clone(), state.clone());
        move |monitors: &gio::ListModel| {
            for v in views.borrow_mut().drain(..) {
                v.window.destroy();
            }
            for monitor in (0..monitors.n_items()).filter_map(|i| monitors.item(i).and_downcast::<gdk::Monitor>()) {
                let v = view(&app, &monitor);
                v.window.present();
                views.borrow_mut().push(v);
            }
            let mut s = state.borrow_mut();
            s.key.clear();
            s.shown_line = None;
        }
    };
    let monitors = display.monitors();
    make(&monitors);
    monitors.connect_items_changed(move |monitors, _, _, _| make(monitors));

    // The player, twice a second.
    {
        let (views, state) = (views.clone(), state.clone());
        glib::timeout_add_local(Duration::from_millis(500), move || {
            let (bus, views, state) = (bus.clone(), views.clone(), state.clone());
            glib::spawn_future_local(async move {
                let now = poll(&bus).await;
                let changed = {
                    let mut s = state.borrow_mut();
                    let key = now.track.as_ref().map(Track::key).unwrap_or_default();
                    s.now = now;
                    s.polled = Some(Instant::now());
                    (s.key != key).then(|| {
                        s.key = key;
                        s.lines.clear();
                        s.cover = None;
                        s.shown_line = None;
                        s.now.track.clone()
                    })
                };
                if let Some(track) = changed {
                    show_track(&views, track.as_ref());
                    if let Some(track) = track {
                        fetch(&views, &state, track);
                    }
                }
            });
            glib::ControlFlow::Continue
        });
    }
    // The lyrics and the progress, ten times a second.
    glib::timeout_add_local(Duration::from_millis(100), move || {
        tick(&views, &state);
        glib::ControlFlow::Continue
    });
}

fn show_track(views: &Rc<RefCell<Vec<View>>>, track: Option<&Track>) {
    for v in views.borrow().iter() {
        match track {
            Some(t) => {
                v.title.set_label(&t.title);
                v.artist.set_label(&t.artist);
                v.album.set_label(&t.album);
                v.album.set_visible(!t.album.is_empty());
                v.cover.set_paintable(gdk::Paintable::NONE);
                v.cover.set_visible(false);
                v.card.set_visible(true);
            }
            None => v.card.set_visible(false),
        }
        v.lyrics.set_visible(false);
    }
}

/// The cover and the lyrics of `track`, on threads.
fn fetch(views: &Rc<RefCell<Vec<View>>>, state: &Rc<RefCell<State>>, track: Track) {
    let key = track.key();
    let (tx, rx) = async_channel::bounded::<(Option<Vec<u8>>, Lines)>(2);
    {
        let (track, tx) = (track.clone(), tx.clone());
        std::thread::spawn(move || {
            let _ = tx.send_blocking((find_cover(&track.art), Vec::new()));
        });
    }
    std::thread::spawn(move || {
        let lines = find_lyrics(&track, &config());
        let _ = tx.send_blocking((None, lines));
    });
    let (views, state) = (views.clone(), state.clone());
    glib::spawn_future_local(async move {
        while let Ok((cover, lines)) = rx.recv().await {
            let mut s = state.borrow_mut();
            if s.key != key {
                return;
            }
            if let Some(bytes) = cover
                && let Ok(texture) = gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes))
            {
                for v in views.borrow().iter() {
                    v.cover.set_paintable(Some(&texture));
                    v.cover.set_visible(true);
                }
                s.cover = Some(texture);
            }
            if !lines.is_empty() {
                log::info!("music: {} lines of lyrics", lines.len());
                s.lines = lines;
                s.shown_line = None;
            }
        }
    });
}

fn tick(views: &Rc<RefCell<Vec<View>>>, state: &Rc<RefCell<State>>) {
    let mut s = state.borrow_mut();
    let Some(track) = s.now.track.clone() else { return };
    let mut position = s.now.position;
    if s.now.playing
        && let Some(at) = s.polled
    {
        position += at.elapsed().as_micros() as i64;
    }
    let length = track.length;
    let line = line_at(&s.lines, position / 1000, length / 1000);
    for v in views.borrow().iter() {
        if length > 0 {
            v.progress.set_fraction((position as f64 / length as f64).clamp(0.0, 1.0));
            v.time.set_label(&format!("{} / {}", clock(position), clock(length)));
        }
        v.progress.set_visible(length > 0);
    }
    if s.shown_line == Some(line) {
        return;
    }
    s.shown_line = Some(line);
    let text = |i: Option<usize>| i.and_then(|i| s.lines.get(i)).map(|(_, l)| l.clone()).unwrap_or_default();
    for v in views.borrow().iter() {
        v.lyrics.set_visible(!s.lines.is_empty());
        v.before.set_label(&text(line.and_then(|i| i.checked_sub(1))));
        v.current.set_label(&text(line));
        v.after.set_label(&text(Some(line.map_or(0, |i| i + 1))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_lines() {
        let lines = parse_lrc("[ar:x]\n[offset:500]\n[00:01.50]One\n[00:10.00][01:00.00]Two\n");
        assert_eq!(
            lines,
            vec![
                (Some(1000), "One".to_string()),
                (Some(9500), "Two".to_string()),
                (Some(59500), "Two".to_string())
            ]
        );
        assert_eq!(line_at(&lines, 0, 0), None);
        assert_eq!(line_at(&lines, 9600, 0), Some(1));
        let plain = parse_lrc("a\nb\n");
        assert_eq!(line_at(&plain, 900, 1000), Some(1));
    }
}

#[cfg(test)]
mod look {
    use super::*;

    /// The card and the lyrics with made-up data, drawn to
    /// `$ORBIT_MUSIC_LOOK` (PNG), on a display of its own; `$ORBIT_MUSIC_COVER`
    /// is a picture for the cover.
    #[test]
    #[ignore]
    fn music_look() {
        let Ok(path) = std::env::var("ORBIT_MUSIC_LOOK") else { return };
        gtk::init().expect("a display");
        let display = gdk::Display::default().unwrap();
        let provider = gtk::CssProvider::new();
        provider.load_from_string(&format!("{CSS}\nwindow.orbit-music {{ background: #05070d; }}"));
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        let app = gtk::Application::new(None::<&str>, gio::ApplicationFlags::NON_UNIQUE);
        app.register(gio::Cancellable::NONE).unwrap();
        let monitor = display.monitors().item(0).and_downcast::<gdk::Monitor>().unwrap();
        let v = view(&app, &monitor);
        v.window.set_default_size(1280, 720);
        let track = Track {
            title: "Wish You Were Here".into(),
            artist: "Pink Floyd".into(),
            album: "Wish You Were Here (1975)".into(),
            ..Default::default()
        };
        let views = Rc::new(RefCell::new(vec![v]));
        show_track(&views, Some(&track));
        if let Ok(cover) = std::env::var("ORBIT_MUSIC_COVER") {
            let texture = gdk::Texture::from_filename(cover).unwrap();
            views.borrow()[0].cover.set_paintable(Some(&texture));
            views.borrow()[0].cover.set_visible(true);
        }
        let state = Rc::new(RefCell::new(State {
            now: Now { track: Some(Track { length: 334_000_000, ..track }), playing: false, position: 95_000_000 },
            lines: parse_lrc("[01:20.00]So, so you think you can tell\n[01:30.00]Heaven from hell, blue skies from pain\n[01:40.00]Can you tell a green field from a cold steel rail?"),
            ..Default::default()
        }));
        tick(&views, &state);
        let window = views.borrow()[0].window.clone();
        window.present();
        let until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < until {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(10));
        }
        let (w, h) = (window.width(), window.height());
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(w), f64::from(h));
        let node = snapshot.to_node().unwrap();
        window.renderer().unwrap().render_texture(&node, None).save_to_png(&path).unwrap();
    }
}
