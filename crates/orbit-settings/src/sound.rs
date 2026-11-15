//! The Sound page (docs/PLANO-SETTINGS.md §3.1), over PipeWire's
//! PulseAudio protocol in pure Rust (the `pulseaudio` crate, as the panel's
//! volume module; no pactl):
//!
//! - the output: which one, its volume, muted or not;
//! - the input (the microphone; outputs' monitors left out), the same;
//! - the applications playing now, a volume each;
//! - the cards' profiles (HDMI or analogue, and the like).
//!
//! The page follows the server's events. It builds its rows again only when
//! a device or a stream comes or goes; otherwise it sets the values in
//! place, and leaves alone a slider the user moved a moment ago, so a drag
//! is never pulled from under the pointer.

use std::{
    cell::{Cell, RefCell},
    ffi::CString,
    io::{self, BufReader},
    os::unix::net::UnixStream,
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{glib, prelude::*};
use orbit_app::i18n::tr;
use pulseaudio::protocol::{self, ChannelVolume, Command, Prop, Volume};

/// The raw value of 100% (`PA_VOLUME_NORM`).
const NORM: u32 = 0x1_0000;
/// How long after the user moves a slider its value is left alone.
const HANDS_OFF: Duration = Duration::from_millis(800);

fn other(err: impl std::fmt::Display) -> io::Error {
    io::Error::other(err.to_string())
}

/// A device (an output or an input) or a stream, as the page shows it.
#[derive(Debug, Clone, PartialEq)]
struct Level {
    index: u32,
    name: CString,
    label: String,
    percent: u32,
    muted: bool,
    channels: u8,
}

#[derive(Debug, Clone, PartialEq)]
struct Card {
    index: u32,
    label: String,
    /// Name and label of each profile that can be used now.
    profiles: Vec<(CString, String)>,
    active: Option<CString>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct State {
    default_sink: Option<CString>,
    default_source: Option<CString>,
    sinks: Vec<Level>,
    sources: Vec<Level>,
    streams: Vec<Level>,
    cards: Vec<Card>,
}

impl State {
    /// What, if it changes, means building the rows again.
    fn shape(&self) -> Vec<String> {
        let mut shape: Vec<String> = Vec::new();
        shape.extend(self.sinks.iter().map(|l| format!("o{}", l.index)));
        shape.extend(self.sources.iter().map(|l| format!("i{}", l.index)));
        shape.extend(self.streams.iter().map(|l| format!("s{}", l.index)));
        shape.extend(self.cards.iter().map(|c| format!("c{}:{}", c.index, c.profiles.len())));
        shape
    }
}

fn text(bytes: Option<&[u8]>) -> Option<String> {
    let s = String::from_utf8_lossy(bytes?).trim_end_matches('\0').to_owned();
    (!s.is_empty()).then_some(s)
}

fn percent(volume: &ChannelVolume) -> u32 {
    let channels = volume.channels();
    let average = channels.iter().map(|v| u64::from(v.as_u32())).sum::<u64>() / channels.len().max(1) as u64;
    ((average * 100 + u64::from(NORM) / 2) / u64::from(NORM)) as u32
}

fn volume(channels: u8, percent: u32) -> ChannelVolume {
    let raw = (u64::from(percent) * u64::from(NORM) / 100) as u32;
    let mut v = ChannelVolume::empty();
    for _ in 0..channels.max(1) {
        v.push(Volume::from_u32_clamped(raw));
    }
    v
}

struct Connection {
    sock: BufReader<UnixStream>,
    version: u16,
    seq: u32,
}

impl Connection {
    fn open() -> io::Result<Self> {
        let path = pulseaudio::socket_path_from_env()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no PulseAudio socket"))?;
        let mut sock = BufReader::new(UnixStream::connect(path)?);
        let cookie = pulseaudio::cookie_path_from_env()
            .and_then(|path| std::fs::read(path).ok())
            .unwrap_or_default();
        let auth = protocol::AuthParams {
            version: protocol::MAX_VERSION,
            supports_shm: false,
            supports_memfd: false,
            cookie,
        };
        protocol::write_command_message(sock.get_mut(), 0, &Command::Auth(auth), protocol::MAX_VERSION)
            .map_err(other)?;
        let (_, reply) =
            protocol::read_reply_message::<protocol::AuthReply>(&mut sock, protocol::MAX_VERSION).map_err(other)?;
        let version = protocol::MAX_VERSION.min(reply.version);
        let mut props = protocol::Props::new();
        props.set(Prop::ApplicationName, CString::new("orbit-settings").expect("no NUL"));
        protocol::write_command_message(sock.get_mut(), 1, &Command::SetClientName(props), version).map_err(other)?;
        protocol::read_reply_message::<protocol::SetClientNameReply>(&mut sock, version).map_err(other)?;
        Ok(Self { sock, version, seq: 2 })
    }

    fn send(&mut self, command: &Command) -> io::Result<()> {
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        protocol::write_command_message(self.sock.get_mut(), seq, command, self.version).map_err(other)
    }

    fn reply<T: protocol::CommandReply>(&mut self, command: &Command) -> io::Result<T> {
        self.send(command)?;
        let (_, reply) = protocol::read_reply_message::<T>(&mut self.sock, self.version).map_err(other)?;
        Ok(reply)
    }

    fn ack(&mut self, command: &Command) -> io::Result<()> {
        self.send(command)?;
        protocol::read_ack_message(&mut self.sock).map(|_| ()).map_err(other)
    }

    fn state(&mut self) -> io::Result<State> {
        let server: protocol::ServerInfo = self.reply(&Command::GetServerInfo)?;
        let sinks: protocol::SinkInfoList = self.reply(&Command::GetSinkInfoList)?;
        let sources: protocol::SourceInfoList = self.reply(&Command::GetSourceInfoList)?;
        let streams: protocol::SinkInputInfoList = self.reply(&Command::GetSinkInputInfoList)?;
        let cards: protocol::CardInfoList = self.reply(&Command::GetCardInfoList)?;
        let level = |index, name: &CString, description: Option<&CString>, cvolume: &ChannelVolume, muted| Level {
            index,
            name: name.clone(),
            label: description
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_else(|| name.to_string_lossy().into_owned()),
            percent: percent(cvolume),
            muted,
            channels: cvolume.channels().len().max(1) as u8,
        };
        Ok(State {
            default_sink: server.default_sink_name,
            default_source: server.default_source_name,
            sinks: sinks
                .iter()
                .map(|s| level(s.index, &s.name, s.description.as_ref(), &s.cvolume, s.muted))
                .collect(),
            sources: sources
                .iter()
                .filter(|s| s.monitor_of_sink_index.is_none())
                .map(|s| level(s.index, &s.name, s.description.as_ref(), &s.cvolume, s.muted))
                .collect(),
            streams: streams
                .iter()
                .filter(|s| s.has_volume)
                .map(|s| {
                    let app = text(s.props.get(Prop::ApplicationName));
                    let media = text(s.props.get(Prop::MediaName)).unwrap_or_else(|| s.name.to_string_lossy().into_owned());
                    let label = match app {
                        Some(app) if app != media => format!("{app} — {media}"),
                        Some(app) => app,
                        None => media,
                    };
                    Level {
                        index: s.index,
                        name: s.name.clone(),
                        label,
                        percent: percent(&s.cvolume),
                        muted: s.muted,
                        channels: s.cvolume.channels().len().max(1) as u8,
                    }
                })
                .collect(),
            cards: cards
                .iter()
                .map(|c| Card {
                    index: c.index,
                    label: text(c.props.get(Prop::DeviceDescription)).unwrap_or_else(|| c.name.to_string_lossy().into_owned()),
                    profiles: c
                        .profiles
                        .iter()
                        .filter(|p| p.available != 0)
                        .map(|p| {
                            let label = p.description.as_ref().unwrap_or(&p.name).to_string_lossy().into_owned();
                            (p.name.clone(), label)
                        })
                        .collect(),
                    active: c.active_profile.clone(),
                })
                .collect(),
        })
    }
}

/// Calls `notify` on the GTK thread after the server's events (a burst,
/// once), reconnecting if the server restarts.
fn subscribe(notify: impl Fn() + 'static) {
    let (tx, rx) = async_channel::unbounded::<()>();
    std::thread::spawn(move || {
        loop {
            let run = || -> io::Result<()> {
                let mut conn = Connection::open()?;
                let mask = protocol::SubscriptionMask::SINK
                    | protocol::SubscriptionMask::SOURCE
                    | protocol::SubscriptionMask::SINK_INPUT
                    | protocol::SubscriptionMask::CARD
                    | protocol::SubscriptionMask::SERVER;
                conn.ack(&Command::Subscribe(mask))?;
                loop {
                    protocol::read_command_message(&mut conn.sock, conn.version).map_err(other)?;
                    if tx.send_blocking(()).is_err() {
                        return Ok(());
                    }
                }
            };
            match run() {
                Ok(()) => return,
                Err(err) => {
                    log::warn!("sound: events lost ({err}), retrying");
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        }
    });
    glib::spawn_future_local(async move {
        while rx.recv().await.is_ok() {
            glib::timeout_future(Duration::from_millis(100)).await;
            while rx.try_recv().is_ok() {}
            notify();
        }
    });
}

/// A level's widgets: its slider and mute button, and when the user last
/// moved the slider.
struct LevelRow {
    scale: gtk::Scale,
    mute: gtk::ToggleButton,
    touched: Rc<Cell<Option<Instant>>>,
}

struct Page {
    conn: RefCell<Option<Connection>>,
    state: RefCell<State>,
    output: gtk::Box,
    input: gtk::Box,
    apps: gtk::Box,
    cards: gtk::Box,
    message: gtk::Label,
    /// Being set to what was read, not by the user.
    updating: Cell<bool>,
    rows: RefCell<Vec<(String, LevelRow)>>,
    pickers: RefCell<Vec<(String, gtk::DropDown)>>,
}

pub fn page() -> gtk::Widget {
    let page = Rc::new(Page {
        conn: RefCell::default(),
        state: RefCell::default(),
        output: card(&tr("settings-sound-output")),
        input: card(&tr("settings-sound-input")),
        apps: card(&tr("settings-sound-apps")),
        cards: card(&tr("settings-sound-cards")),
        message: crate::message(),
        updating: Cell::new(false),
        rows: RefCell::default(),
        pickers: RefCell::default(),
    });
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();
    for card in [&page.output, &page.input, &page.apps, &page.cards] {
        body.append(card);
    }
    body.append(&page.message);
    page.refresh();
    let weak = Rc::downgrade(&page);
    subscribe(move || {
        if let Some(page) = weak.upgrade() {
            page.refresh();
        }
    });
    // The page lives as long as its widgets (the subscription holds it
    // weakly): the body's destroy signal keeps it until then.
    body.connect_destroy(move |_| {
        let _ = &page;
    });
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build()
        .upcast()
}

fn card(title: &str) -> gtk::Box {
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(["card"])
        .build();
    card.append(&gtk::Label::builder().label(title).xalign(0.0).css_classes(["heading"]).build());
    card
}

/// Everything in `card` but its title.
fn clear(card: &gtk::Box) {
    let Some(title) = card.first_child() else { return };
    while let Some(child) = title.next_sibling() {
        card.remove(&child);
    }
}

impl Page {
    /// Runs `f` on the connection, opening it (again) as needed.
    fn with<T>(&self, f: impl FnOnce(&mut Connection) -> io::Result<T>) -> Option<T> {
        let mut conn = self.conn.borrow_mut();
        if conn.is_none() {
            match Connection::open() {
                Ok(c) => *conn = Some(c),
                Err(e) => {
                    crate::say(&self.message, &format!("{}: {e}", tr("settings-sound-no-server")));
                    return None;
                }
            }
        }
        match f(conn.as_mut().expect("opened")) {
            Ok(v) => Some(v),
            Err(e) => {
                *conn = None;
                crate::say(&self.message, &e.to_string());
                None
            }
        }
    }

    fn refresh(self: &Rc<Self>) {
        let Some(state) = self.with(Connection::state) else { return };
        crate::say(&self.message, "");
        let rebuild = state.shape() != self.state.borrow().shape() || self.rows.borrow().is_empty();
        *self.state.borrow_mut() = state;
        if rebuild {
            self.build();
        } else {
            self.update();
        }
    }

    /// The rows again, from the state.
    fn build(self: &Rc<Self>) {
        let state = self.state.borrow().clone();
        self.rows.borrow_mut().clear();
        self.pickers.borrow_mut().clear();
        for card in [&self.output, &self.input, &self.apps, &self.cards] {
            clear(card);
        }
        self.updating.set(true);
        self.device_part(&self.output, "o", &state.sinks, state.default_sink.as_ref());
        self.device_part(&self.input, "i", &state.sources, state.default_source.as_ref());
        if state.streams.is_empty() {
            self.apps.append(&dim(&tr("settings-sound-no-apps")));
        }
        for stream in &state.streams {
            self.apps.append(&gtk::Label::builder().label(&stream.label).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build());
            self.apps.append(&self.level_row("s", stream));
        }
        self.cards.set_visible(state.cards.iter().any(|c| c.profiles.len() > 1));
        for card in &state.cards {
            if card.profiles.len() < 2 {
                continue;
            }
            let labels: Vec<&str> = card.profiles.iter().map(|(_, l)| l.as_str()).collect();
            let picker = gtk::DropDown::from_strings(&labels);
            let active = card.profiles.iter().position(|(n, _)| Some(n) == card.active.as_ref());
            picker.set_selected(active.map_or(gtk::INVALID_LIST_POSITION, |i| i as u32));
            let page = self.clone();
            let index = card.index;
            let profiles = card.profiles.clone();
            picker.connect_selected_notify(move |p| {
                if page.updating.get() {
                    return;
                }
                let Some((name, _)) = profiles.get(p.selected() as usize).cloned() else { return };
                page.with(|c| {
                    c.ack(&Command::SetCardProfile(protocol::SetCardProfileParams {
                        card_index: Some(index),
                        card_name: None,
                        profile_name: name,
                    }))
                });
            });
            self.cards.append(&crate::row(&card.label, &picker));
            self.pickers.borrow_mut().push((format!("c{index}"), picker));
        }
        self.updating.set(false);
    }

    /// The output's or input's part: which device, and its level.
    fn device_part(self: &Rc<Self>, card: &gtk::Box, kind: &'static str, devices: &[Level], default: Option<&CString>) {
        if devices.is_empty() {
            card.append(&dim(&tr("settings-sound-none")));
            return;
        }
        let labels: Vec<&str> = devices.iter().map(|d| d.label.as_str()).collect();
        let picker = gtk::DropDown::from_strings(&labels);
        let current = devices.iter().position(|d| Some(&d.name) == default).unwrap_or(0);
        picker.set_selected(current as u32);
        let names: Vec<CString> = devices.iter().map(|d| d.name.clone()).collect();
        let page = self.clone();
        picker.connect_selected_notify(move |p| {
            if page.updating.get() {
                return;
            }
            let Some(name) = names.get(p.selected() as usize).cloned() else { return };
            let command = if kind == "o" { Command::SetDefaultSink(name) } else { Command::SetDefaultSource(name) };
            page.with(|c| c.ack(&command));
        });
        card.append(&crate::row(&tr("settings-sound-device"), &picker));
        self.pickers.borrow_mut().push((kind.to_owned(), picker));
        if let Some(device) = devices.get(current) {
            card.append(&self.level_row(kind, device));
        }
    }

    /// A slider and a mute button for an output (`o`), input (`i`) or
    /// stream (`s`).
    fn level_row(self: &Rc<Self>, kind: &'static str, level: &Level) -> gtk::Box {
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
        scale.set_hexpand(true);
        scale.set_value(f64::from(level.percent.min(100)));
        let mute = gtk::ToggleButton::builder()
            .icon_name("audio-volume-muted-symbolic")
            .active(level.muted)
            .tooltip_text(tr("settings-sound-mute"))
            .css_classes(["flat"])
            .build();
        let touched: Rc<Cell<Option<Instant>>> = Rc::default();
        let (index, channels) = (level.index, level.channels);
        scale.connect_value_changed({
            let page = self.clone();
            let touched = touched.clone();
            move |s| {
                if page.updating.get() {
                    return;
                }
                touched.set(Some(Instant::now()));
                let v = volume(channels, s.value().round() as u32);
                let command = match kind {
                    "o" => Command::SetSinkVolume(protocol::SetDeviceVolumeParams { device_index: Some(index), device_name: None, volume: v }),
                    "i" => Command::SetSourceVolume(protocol::SetDeviceVolumeParams { device_index: Some(index), device_name: None, volume: v }),
                    _ => Command::SetSinkInputVolume(protocol::SetStreamVolumeParams { index, volume: v }),
                };
                page.with(|c| c.ack(&command));
            }
        });
        mute.connect_toggled({
            let page = self.clone();
            move |m| {
                if page.updating.get() {
                    return;
                }
                let on = m.is_active();
                let command = match kind {
                    "o" => Command::SetSinkMute(protocol::SetDeviceMuteParams { device_index: Some(index), device_name: None, mute: on }),
                    "i" => Command::SetSourceMute(protocol::SetDeviceMuteParams { device_index: Some(index), device_name: None, mute: on }),
                    _ => Command::SetSinkInputMute(protocol::SetStreamMuteParams { index, mute: on }),
                };
                page.with(|c| c.ack(&command));
            }
        });
        let line = gtk::Box::builder().spacing(10).build();
        line.append(&scale);
        line.append(&mute);
        self.rows.borrow_mut().push((format!("{kind}{index}"), LevelRow { scale, mute, touched }));
        line
    }

    /// The values in place, the rows kept.
    fn update(self: &Rc<Self>) {
        let state = self.state.borrow().clone();
        // The default device changed: its level row is another device's,
        // so build again.
        let shown = |kind: &str, devices: &[Level], default: Option<&CString>| {
            let index = devices.iter().find(|d| Some(&d.name) == default).or(devices.first()).map(|d| d.index);
            index.is_none_or(|i| self.rows.borrow().iter().any(|(k, _)| *k == format!("{kind}{i}")))
        };
        if !shown("o", &state.sinks, state.default_sink.as_ref()) || !shown("i", &state.sources, state.default_source.as_ref()) {
            self.build();
            return;
        }
        self.updating.set(true);
        let levels = state.sinks.iter().map(|l| ("o", l)).chain(state.sources.iter().map(|l| ("i", l))).chain(state.streams.iter().map(|l| ("s", l)));
        for (kind, level) in levels {
            let key = format!("{kind}{}", level.index);
            if let Some((_, row)) = self.rows.borrow().iter().find(|(k, _)| *k == key) {
                let recent = row.touched.get().is_some_and(|t| t.elapsed() < HANDS_OFF);
                if !recent {
                    row.scale.set_value(f64::from(level.percent.min(100)));
                }
                row.mute.set_active(level.muted);
            }
        }
        for (key, picker) in self.pickers.borrow().iter() {
            let selected = match key.as_str() {
                "o" => state.sinks.iter().position(|d| Some(&d.name) == state.default_sink.as_ref()),
                "i" => state.sources.iter().position(|d| Some(&d.name) == state.default_source.as_ref()),
                card => state
                    .cards
                    .iter()
                    .find(|c| format!("c{}", c.index) == card)
                    .and_then(|c| c.profiles.iter().position(|(n, _)| Some(n) == c.active.as_ref())),
            };
            if let Some(i) = selected {
                picker.set_selected(i as u32);
            }
        }
        self.updating.set(false);
    }
}

fn dim(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["dim"]).build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volumes() {
        let v = volume(2, 50);
        assert_eq!(v.channels().len(), 2);
        assert_eq!(percent(&v), 50);
        assert_eq!(percent(&volume(1, 100)), 100);
    }

    #[test]
    fn props_text() {
        assert_eq!(text(Some(b"Firefox\0")), Some("Firefox".to_owned()));
        assert_eq!(text(Some(b"\0")), None);
        assert_eq!(text(None), None);
    }
}
