//! The window: the video — or, for audio, the album's cover with the title
//! and artist — with the title bar above it and the controls below: the
//! time bar, play, previous and next in the folder, frame steps, volume,
//! speed, audio track and subtitles. Playing a video, the bars slide away a
//! moment after the pointer stops and the video grows to the whole window;
//! they slide back, and the video makes room for them, when it moves. The
//! bars never cover the picture, nor its subtitles.
//!
//! A Flash movie is handed to Ruffle, in a window of its own.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use gst::prelude::*;
use gtk::{gdk, gio, glib, prelude::*};
use orbit_app::i18n::{number, text, tr, tr_args};

use crate::{
    engine::Engine,
    media::{self, Kind},
};

const CONTROLS_FOR: Duration = Duration::from_secs(2);
const SPEEDS: &[f64] = &[0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

/// What the player reports, carried to GTK's thread.
enum Signal {
    State(gst_play::PlayState),
    Duration(f64),
    Position(f64),
    MediaInfo(gst_play::PlayMediaInfo),
    End,
    Error(String),
    Volume(f64),
}

pub struct Player {
    window: gtk::ApplicationWindow,
    engine: Engine,
    stack: gtk::Stack,
    cover: gtk::Picture,
    cover_icon: gtk::Image,
    song: gtk::Label,
    artist: gtk::Label,
    message: gtk::Label,
    top: gtk::Revealer,
    bottom: gtk::Revealer,
    title: gtk::Label,
    time: gtk::Scale,
    position: gtk::Label,
    duration: gtk::Label,
    play_button: gtk::Button,
    fullscreen: gtk::Button,
    previous: gtk::Button,
    next: gtk::Button,
    volume: gtk::Scale,
    mute: gtk::ToggleButton,
    speed: gtk::MenuButton,
    audio: gtk::MenuButton,
    subtitles: gtk::MenuButton,

    files: RefCell<Vec<PathBuf>>,
    index: Cell<Option<usize>>,
    playing: Cell<bool>,
    has_video: Cell<bool>,
    on_controls: Cell<bool>,
    hide: RefCell<Option<glib::SourceId>>,
}

impl Player {
    pub fn new(app: &gtk::Application) -> anyhow::Result<Rc<Self>> {
        let engine = Engine::new()?;
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title(tr("play-title"))
            .default_width(960)
            .default_height(600)
            .css_classes(["orbit-play"])
            .build();
        window.set_titlebar(Some(&gtk::Box::builder().visible(false).build()));

        // ---- what is shown
        let picture = gtk::Picture::builder()
            .paintable(&engine.paintable)
            .content_fit(gtk::ContentFit::Contain)
            .hexpand(true)
            .vexpand(true)
            .css_classes(["video"])
            .build();
        let cover = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .width_request(300)
            .height_request(300)
            .css_classes(["cover"])
            .build();
        let cover_icon = gtk::Image::builder()
            .icon_name("audio-x-generic-symbolic")
            .pixel_size(150)
            .css_classes(["cover-icon"])
            .build();
        let song = gtk::Label::builder().wrap(true).justify(gtk::Justification::Center).css_classes(["song"]).build();
        let artist = gtk::Label::builder().wrap(true).justify(gtk::Justification::Center).css_classes(["artist"]).build();
        let audio_page = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(15)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();
        audio_page.append(&cover);
        audio_page.append(&cover_icon);
        audio_page.append(&song);
        audio_page.append(&artist);
        let message = gtk::Label::builder()
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["message"])
            .build();
        let open_button = gtk::Button::builder()
            .label(tr("play-open"))
            .halign(gtk::Align::Center)
            .build();
        let message_page = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(15)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();
        message_page.append(&gtk::Image::builder().icon_name("video-x-generic-symbolic").pixel_size(60).css_classes(["message-icon"]).build());
        message_page.append(&message);
        message_page.append(&open_button);
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        stack.add_named(&picture, Some("video"));
        stack.add_named(&audio_page, Some("audio"));
        stack.add_named(&message_page, Some("message"));

        // ---- the bar along the top: the title, the window's buttons
        let title = gtk::Label::builder()
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .hexpand(true)
            .xalign(0.0)
            .css_classes(["title"])
            .build();
        let top_row = gtk::Box::builder().spacing(10).css_classes(["bar", "top"]).build();
        top_row.append(&title);
        top_row.append(
            &gtk::WindowControls::builder()
                .side(gtk::PackType::End)
                .decoration_layout(":minimize,maximize,close")
                .build(),
        );
        let top = gtk::Revealer::builder()
            .child(&gtk::WindowHandle::builder().child(&top_row).build())
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .reveal_child(true)
            .build();

        // ---- the controls along the bottom
        let button = |icon: &str, tip: &str, key: &str| {
            gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(shortcut(tip, key))
                .focus_on_click(false)
                .build()
        };
        let position = gtk::Label::builder().width_chars(8).css_classes(["time-label"]).build();
        let duration = gtk::Label::builder().width_chars(8).css_classes(["time-label"]).build();
        let time = gtk::Scale::builder()
            .orientation(gtk::Orientation::Horizontal)
            .adjustment(&gtk::Adjustment::new(0.0, 0.0, 1.0, 1.0, 10.0, 0.0))
            .hexpand(true)
            .focus_on_click(false)
            .css_classes(["time"])
            .build();
        let time_row = gtk::Box::builder().spacing(10).build();
        time_row.append(&position);
        time_row.append(&time);
        time_row.append(&duration);

        let previous = button("media-skip-backward-symbolic", "play-previous", "PgUp");
        let play_button = button("media-playback-start-symbolic", "play-play", &tr("play-key-space"));
        play_button.add_css_class("main");
        let next = button("media-skip-forward-symbolic", "play-next", "PgDn");
        let frame_back = button("go-previous-symbolic", "play-frame-back", ",");
        let frame_forward = button("go-next-symbolic", "play-frame-forward", ".");
        let mute = gtk::ToggleButton::builder()
            .icon_name("audio-volume-high-symbolic")
            .tooltip_text(shortcut("play-mute", "M"))
            .focus_on_click(false)
            .build();
        let volume = gtk::Scale::builder()
            .orientation(gtk::Orientation::Horizontal)
            .adjustment(&gtk::Adjustment::new(1.0, 0.0, 1.0, 0.05, 0.1, 0.0))
            .width_request(100)
            .focus_on_click(false)
            .tooltip_text(shortcut("play-volume", "↑ ↓"))
            .build();
        let menu = |icon: &str, tip: &str| {
            gtk::MenuButton::builder()
                .icon_name(icon)
                .tooltip_text(tr(tip))
                .focus_on_click(false)
                .direction(gtk::ArrowType::Up)
                .build()
        };
        let speed = gtk::MenuButton::builder()
            .label("1×")
            .tooltip_text(shortcut("play-speed", "[ ]"))
            .focus_on_click(false)
            .direction(gtk::ArrowType::Up)
            .build();
        let audio = menu("audio-x-generic-symbolic", "play-audio-track");
        let subtitles = menu("media-view-subtitles-symbolic", "play-subtitles");
        let open = button("document-open-symbolic", "play-open", "Ctrl+O");
        let fullscreen = button("view-fullscreen-symbolic", "play-fullscreen", "F");

        let controls_row = gtk::Box::builder().spacing(5).build();
        for w in [
            previous.upcast_ref::<gtk::Widget>(),
            play_button.upcast_ref(),
            next.upcast_ref(),
            frame_back.upcast_ref(),
            frame_forward.upcast_ref(),
        ] {
            controls_row.append(w);
        }
        controls_row.append(&gtk::Box::builder().hexpand(true).build());
        for w in [
            mute.upcast_ref::<gtk::Widget>(),
            volume.upcast_ref(),
            speed.upcast_ref(),
            audio.upcast_ref(),
            subtitles.upcast_ref(),
            open.upcast_ref(),
            fullscreen.upcast_ref(),
        ] {
            controls_row.append(w);
        }
        let controls = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(5)
            .css_classes(["bar", "controls"])
            .build();
        controls.append(&time_row);
        controls.append(&controls_row);
        let bottom = gtk::Revealer::builder()
            .child(&controls)
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .reveal_child(true)
            .build();

        // One above the other: the bars take their room from the video.
        stack.set_vexpand(true);
        let layout = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        layout.append(&top);
        layout.append(&stack);
        layout.append(&bottom);
        window.set_child(Some(&layout));

        let player = Rc::new(Self {
            window,
            engine,
            stack,
            cover,
            cover_icon,
            song,
            artist,
            message,
            top,
            bottom,
            title,
            time,
            position,
            duration,
            play_button: play_button.clone(),
            fullscreen: fullscreen.clone(),
            previous: previous.clone(),
            next: next.clone(),
            volume,
            mute,
            speed,
            audio,
            subtitles,
            files: RefCell::default(),
            index: Cell::new(None),
            playing: Cell::new(false),
            has_video: Cell::new(false),
            on_controls: Cell::new(false),
            hide: RefCell::default(),
        });

        let act = |button: &gtk::Button, f: fn(&Rc<Player>)| {
            let weak = Rc::downgrade(&player);
            button.connect_clicked(move |_| {
                if let Some(p) = weak.upgrade() {
                    f(&p);
                }
            });
        };
        act(&play_button, |p| p.toggle());
        act(&previous, |p| p.step(-1));
        act(&next, |p| p.step(1));
        act(&frame_back, |p| p.frame(false));
        act(&frame_forward, |p| p.frame(true));
        act(&open, |p| p.choose());
        act(&open_button, |p| p.choose());
        act(&fullscreen, |p| p.toggle_fullscreen());
        player.build_speed_menu();
        player.connect_engine();
        player.connect_widgets(&layout);
        player.show_message(&tr("play-empty"));
        Ok(player)
    }

    pub fn window(&self) -> &gtk::ApplicationWindow {
        &self.window
    }

    // ------------------------------------------------------------ files

    /// Opens `path`, and lists its folder. A Flash movie goes to Ruffle;
    /// when nothing else plays here, this window closes.
    pub fn open(self: &Rc<Self>, path: &Path) {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if let Some(dir) = path.parent() {
            let files = media::folder(dir, &path);
            self.index.set(files.iter().position(|p| *p == path));
            *self.files.borrow_mut() = files;
        }
        self.load(&path);
    }

    fn load(self: &Rc<Self>, path: &Path) {
        self.update_nav();
        if media::kind_of(path) == Some(Kind::Flash) {
            self.flash(path);
            return;
        }
        let name = file_name(path);
        self.title.set_label(&name);
        self.window.set_title(Some(&name));
        self.has_video.set(false);
        self.time.set_value(0.0);
        self.position.set_label(&clock(0.0));
        self.duration.set_label("");
        let play = &self.engine.play;
        play.stop();
        // A subtitle file beside it, of the same name, comes along.
        let subtitle = media::subtitle_for(path).map(|s| gio::File::for_path(s).uri());
        play.set_uri(Some(&gio::File::for_path(path).uri()));
        play.set_subtitle_uri(subtitle.as_deref());
        play.set_rate(self.rate());
        play.play();
        self.reveal(true);
    }

    fn flash(self: &Rc<Self>, path: &Path) {
        let ruffle = glib::home_dir().join(".local/bin/ruffle");
        let program = if ruffle.is_file() {
            ruffle
        } else {
            match glib::find_program_in_path("ruffle") {
                Some(p) => p,
                None => {
                    self.show_message(&tr("play-no-ruffle"));
                    return;
                }
            }
        };
        let launched = gio::Subprocess::newv(
            &[program.as_os_str(), path.as_os_str()],
            gio::SubprocessFlags::NONE,
        );
        match launched {
            Ok(_) => {
                // Nothing playing here: the window is not needed.
                if self.engine.play.uri().is_none() {
                    self.window.close();
                }
            }
            Err(err) => self.show_message(&tr_args("play-failed", &[("name", text(file_name(path))), ("reason", text(err.message()))])),
        }
    }

    fn step(self: &Rc<Self>, by: isize) {
        let len = self.files.borrow().len();
        let Some(i) = self.index.get() else { return };
        let next = i as isize + by;
        if (0..len as isize).contains(&next) {
            self.index.set(Some(next as usize));
            let path = self.files.borrow()[next as usize].clone();
            self.load(&path);
        }
    }

    fn update_nav(&self) {
        let len = self.files.borrow().len();
        let i = self.index.get();
        self.previous.set_sensitive(i.is_some_and(|i| i > 0));
        self.next.set_sensitive(i.is_some_and(|i| i + 1 < len));
    }

    fn choose(self: &Rc<Self>) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(&tr("play-filter")));
        filter.add_mime_type("video/*");
        filter.add_mime_type("audio/*");
        filter.add_mime_type("application/x-shockwave-flash");
        filter.add_pattern("*.swf");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title(tr("play-open"))
            .filters(&filters)
            .default_filter(&filter)
            .modal(true)
            .build();
        let current = self.index.get().and_then(|i| self.files.borrow().get(i).cloned());
        if let Some(dir) = current.as_ref().and_then(|p| p.parent()) {
            dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
        }
        let weak = Rc::downgrade(self);
        dialog.open(Some(&self.window), gio::Cancellable::NONE, move |result| {
            if let (Some(p), Ok(file)) = (weak.upgrade(), result)
                && let Some(path) = file.path()
            {
                p.open(&path);
            }
        });
    }

    fn show_message(&self, message: &str) {
        self.message.set_label(message);
        self.stack.set_visible_child_name("message");
        self.reveal(true);
    }

    // ---------------------------------------------------------- playback

    fn toggle(&self) {
        if self.engine.play.uri().is_none() {
            return;
        }
        if self.playing.get() {
            self.engine.play.pause();
        } else {
            self.engine.play.play();
        }
    }

    fn seek_by(&self, seconds: i64) {
        let play = &self.engine.play;
        let Some(now) = play.position() else { return };
        let target = if seconds < 0 {
            now.saturating_sub(gst::ClockTime::from_seconds(seconds.unsigned_abs()))
        } else {
            now + gst::ClockTime::from_seconds(seconds as u64)
        };
        let target = play.duration().map_or(target, |d| target.min(d));
        play.seek(target);
    }

    fn frame(&self, forward: bool) {
        if self.playing.get() {
            self.engine.play.pause();
        }
        if forward {
            self.engine.frame_forward();
        } else {
            self.engine.frame_back();
        }
    }

    fn rate(&self) -> f64 {
        self.speed
            .label()
            .and_then(|l| l.trim_end_matches('×').replace(',', ".").parse().ok())
            .unwrap_or(1.0)
    }

    fn set_rate(&self, rate: f64) {
        self.engine.play.set_rate(rate);
        self.speed.set_label(&format!("{}×", decimal(rate)));
    }

    fn change_speed(&self, by: isize) {
        let now = self.rate();
        let at = SPEEDS.iter().position(|s| (*s - now).abs() < 1e-6).unwrap_or(3);
        let next = (at as isize + by).clamp(0, SPEEDS.len() as isize - 1) as usize;
        self.set_rate(SPEEDS[next]);
    }

    fn build_speed_menu(self: &Rc<Self>) {
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["menu-list"]).build();
        let popover = gtk::Popover::builder().child(&list).build();
        for &rate in SPEEDS {
            let item = gtk::Button::builder()
                .label(format!("{}×", decimal(rate)))
                .css_classes(["flat"])
                .build();
            let weak = Rc::downgrade(self);
            let pop = popover.clone();
            item.connect_clicked(move |_| {
                if let Some(p) = weak.upgrade() {
                    p.set_rate(rate);
                }
                pop.popdown();
            });
            list.append(&item);
        }
        self.speed.set_popover(Some(&popover));
    }

    /// The audio and subtitle menus, from the tracks the file has.
    fn build_track_menus(self: &Rc<Self>, info: &gst_play::PlayMediaInfo) {
        let play = &self.engine.play;
        // Audio.
        let streams = info.audio_streams();
        let current = play.current_audio_track().map(|t| track_index(&t));
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["menu-list"]).build();
        let popover = gtk::Popover::builder().child(&list).build();
        let mut group: Option<gtk::CheckButton> = None;
        for (n, stream) in streams.iter().enumerate() {
            let mut label = stream
                .language()
                .map(|l| language_name(&l))
                .unwrap_or_else(|| tr_args("play-track", &[("n", number((n + 1) as f64))]));
            if stream.channels() > 0 {
                label = format!("{label} · {}", channels(stream.channels()));
            }
            let check = gtk::CheckButton::builder().label(label).active(current == Some(track_index(stream))).build();
            check.set_group(group.as_ref());
            group.get_or_insert_with(|| check.clone());
            let weak = Rc::downgrade(self);
            let index = track_index(stream);
            check.connect_toggled(move |c| {
                if c.is_active()
                    && let Some(p) = weak.upgrade()
                    && let Err(err) = p.engine.play.set_audio_track(index)
                {
                    log::warn!("audio track {index}: {err}");
                }
            });
            list.append(&check);
        }
        self.audio.set_popover(Some(&popover));
        self.audio.set_visible(streams.len() > 1);

        // Subtitles: off, each track, and a file to load.
        let streams = info.subtitle_streams();
        let current = play.current_subtitle_track().map(|t| track_index(&t));
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["menu-list"]).build();
        let popover = gtk::Popover::builder().child(&list).build();
        let off = gtk::CheckButton::builder().label(tr("play-subtitles-off")).active(current.is_none()).build();
        let weak = Rc::downgrade(self);
        off.connect_toggled(move |c| {
            if c.is_active()
                && let Some(p) = weak.upgrade()
            {
                p.engine.play.set_subtitle_track_enabled(false);
            }
        });
        list.append(&off);
        for (n, stream) in streams.iter().enumerate() {
            let label = stream
                .language()
                .map(|l| language_name(&l))
                .unwrap_or_else(|| tr_args("play-track", &[("n", number((n + 1) as f64))]));
            let check = gtk::CheckButton::builder().label(label).active(current == Some(track_index(stream))).build();
            check.set_group(Some(&off));
            let weak = Rc::downgrade(self);
            let index = track_index(stream);
            check.connect_toggled(move |c| {
                if c.is_active()
                    && let Some(p) = weak.upgrade()
                {
                    p.engine.play.set_subtitle_track_enabled(true);
                    if let Err(err) = p.engine.play.set_subtitle_track(index) {
                        log::warn!("subtitle track {index}: {err}");
                    }
                }
            });
            list.append(&check);
        }
        let load = gtk::Button::builder().label(tr("play-subtitles-load")).css_classes(["flat"]).build();
        let weak = Rc::downgrade(self);
        let pop = popover.clone();
        load.connect_clicked(move |_| {
            pop.popdown();
            if let Some(p) = weak.upgrade() {
                p.choose_subtitle();
            }
        });
        list.append(&load);
        self.subtitles.set_popover(Some(&popover));
        self.subtitles.set_visible(info.number_of_video_streams() > 0);
    }

    fn choose_subtitle(self: &Rc<Self>) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(&tr("play-subtitles")));
        for ext in ["srt", "ass", "ssa", "vtt", "sub"] {
            filter.add_pattern(&format!("*.{ext}"));
        }
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title(tr("play-subtitles-load"))
            .filters(&filters)
            .default_filter(&filter)
            .modal(true)
            .build();
        let weak = Rc::downgrade(self);
        dialog.open(Some(&self.window), gio::Cancellable::NONE, move |result| {
            let (Some(p), Ok(file)) = (weak.upgrade(), result) else { return };
            // A new subtitle file takes effect from the start: carry on
            // from where it was.
            let play = &p.engine.play;
            let at = play.position();
            let uri = play.uri();
            play.stop();
            play.set_uri(uri.as_deref());
            play.set_subtitle_uri(Some(&file.uri()));
            play.play();
            if let Some(at) = at {
                let play = play.clone();
                glib::timeout_add_local_once(Duration::from_millis(300), move || play.seek(at));
            }
        });
    }

    /// The player's signals come from its own thread: through a channel,
    /// they are handled here, on GTK's.
    fn connect_engine(self: &Rc<Self>) {
        let (tx, rx) = async_channel::unbounded::<Signal>();
        let signals = &self.engine.signals;
        let t = tx.clone();
        signals.connect_state_changed(move |_, state| {
            let _ = t.send_blocking(Signal::State(state));
        });
        let t = tx.clone();
        signals.connect_duration_changed(move |_, duration| {
            let _ = t.send_blocking(Signal::Duration(duration.map_or(0.0, |d| d.seconds_f64())));
        });
        let t = tx.clone();
        signals.connect_position_updated(move |_, position| {
            let _ = t.send_blocking(Signal::Position(position.map_or(0.0, |d| d.seconds_f64())));
        });
        let t = tx.clone();
        signals.connect_media_info_updated(move |_, info| {
            let _ = t.send_blocking(Signal::MediaInfo(info.clone()));
        });
        let t = tx.clone();
        signals.connect_end_of_stream(move |_| {
            let _ = t.send_blocking(Signal::End);
        });
        let t = tx.clone();
        signals.connect_error(move |_, err, _| {
            let _ = t.send_blocking(Signal::Error(err.message().to_owned()));
        });
        signals.connect_volume_changed(move |_, volume| {
            let _ = tx.send_blocking(Signal::Volume(volume));
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(signal) = rx.recv().await {
                let Some(p) = weak.upgrade() else { break };
                p.signal(signal);
            }
        });
    }

    fn signal(self: &Rc<Self>, signal: Signal) {
        match signal {
            Signal::State(state) => {
                let playing = state == gst_play::PlayState::Playing;
                self.playing.set(playing);
                self.play_button.set_icon_name(if playing {
                    "media-playback-pause-symbolic"
                } else {
                    "media-playback-start-symbolic"
                });
                self.play_button.set_tooltip_text(Some(&shortcut(
                    if playing { "play-pause" } else { "play-play" },
                    &tr("play-key-space"),
                )));
                if playing {
                    self.hide_later();
                } else {
                    self.reveal(true);
                }
            }
            Signal::Duration(seconds) => {
                self.time.adjustment().set_upper(seconds.max(1.0));
                self.duration.set_label(&clock(seconds));
            }
            Signal::Position(seconds) => {
                self.time.set_value(seconds);
                self.position.set_label(&clock(seconds));
            }
            Signal::MediaInfo(info) => self.media_info(&info),
            Signal::End => {
                // On to the next in the folder that plays here — a Flash
                // movie opens only when asked for; after the last, stop.
                let next = self.index.get().and_then(|i| {
                    let files = self.files.borrow();
                    (i + 1..files.len()).find(|&n| media::kind_of(&files[n]) == Some(Kind::Media))
                });
                match next {
                    Some(n) => self.step(n as isize - self.index.get().unwrap_or(0) as isize),
                    None => {
                        self.engine.play.pause();
                        self.reveal(true);
                    }
                }
            }
            Signal::Error(reason) => {
                let name = self
                    .index
                    .get()
                    .and_then(|i| self.files.borrow().get(i).map(|f| file_name(f)))
                    .unwrap_or_default();
                self.show_message(&tr_args(
                    "play-failed",
                    &[("name", text(name)), ("reason", text(reason))],
                ));
            }
            Signal::Volume(volume) => {
                if (self.volume.value() - volume).abs() > 1e-3 {
                    self.volume.set_value(volume);
                }
            }
        }
    }

    fn media_info(self: &Rc<Self>, info: &gst_play::PlayMediaInfo) {
        let current = self.index.get().and_then(|i| self.files.borrow().get(i).cloned());
        let video = info.number_of_video_streams() > 0
            && !current.as_deref().is_some_and(media::is_audio);
        if video != self.has_video.get() || self.stack.visible_child_name().as_deref() == Some("message") {
            self.has_video.set(video);
            self.stack.set_visible_child_name(if video { "video" } else { "audio" });
        }
        // The file's tags, then each stream's: an MP3's ID3 tags come with
        // its audio stream.
        let tags: Vec<gst::TagList> = info
            .tags()
            .into_iter()
            .chain(
                info.stream_list()
                    .iter()
                    .filter_map(gst_play::prelude::PlayStreamInfoExt::tags),
            )
            .collect();
        let tag = |f: fn(&gst::TagList) -> Option<String>| tags.iter().find_map(f);
        let title = info.title().map(|t| t.to_string()).or_else(|| tag(|t| t.get::<gst::tags::Title>().map(|v| v.get().to_owned())));
        let artist = tag(|t| t.get::<gst::tags::Artist>().map(|v| v.get().to_owned()));
        let album = tag(|t| t.get::<gst::tags::Album>().map(|v| v.get().to_owned()));
        if let Some(title) = title.as_ref().filter(|t| !t.trim().is_empty()) {
            self.title.set_label(title);
            self.window.set_title(Some(title));
        }
        if !video {
            self.song.set_label(&title.unwrap_or_else(|| current.as_deref().map(file_name).unwrap_or_default()));
            self.artist.set_label(&[artist, album].into_iter().flatten().collect::<Vec<_>>().join(" — "));
            let texture = info
                .image_sample()
                .and_then(|s| s.buffer_owned())
                .and_then(|b| b.into_mapped_buffer_readable().ok())
                .and_then(|m| gdk::Texture::from_bytes(&glib::Bytes::from(m.as_slice())).ok());
            self.cover.set_paintable(texture.as_ref());
            self.cover.set_visible(texture.is_some());
            self.cover_icon.set_visible(texture.is_none());
        }
        self.build_track_menus(info);
    }

    // ------------------------------------------------- screen and controls

    fn toggle_fullscreen(&self) {
        if self.window.is_fullscreen() {
            self.window.unfullscreen();
        } else {
            self.window.fullscreen();
        }
    }

    fn reveal(&self, shown: bool) {
        // Audio keeps its controls; so does a paused video.
        let shown = shown || !self.has_video.get() || !self.playing.get();
        self.top.set_reveal_child(shown);
        self.top.set_can_target(shown);
        self.bottom.set_reveal_child(shown);
        self.bottom.set_can_target(shown);
        self.window.set_cursor_from_name(if shown { None } else { Some("none") });
    }

    fn hide_later(self: &Rc<Self>) {
        if let Some(id) = self.hide.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(CONTROLS_FOR, move || {
            if let Some(p) = weak.upgrade() {
                p.hide.borrow_mut().take();
                // Not while a menu is open or the pointer is on them.
                let menu_open = [&p.speed, &p.audio, &p.subtitles]
                    .iter()
                    .any(|m| m.popover().is_some_and(|pop| pop.is_visible()));
                if !p.on_controls.get() && !menu_open {
                    p.reveal(false);
                }
            }
        });
        *self.hide.borrow_mut() = Some(id);
    }

    fn connect_widgets(self: &Rc<Self>, layout: &gtk::Box) {
        // Seeking: only what the user drags or clicks on the bar.
        let weak = Rc::downgrade(self);
        self.time.connect_change_value(move |_, _, value| {
            if let Some(p) = weak.upgrade() {
                p.engine.play.seek(gst::ClockTime::from_seconds_f64(value.max(0.0)));
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        self.volume.connect_value_changed(move |s| {
            if let Some(p) = weak.upgrade() {
                p.engine.play.set_volume(s.value());
                p.update_volume_icon();
            }
        });
        let weak = Rc::downgrade(self);
        self.mute.connect_toggled(move |b| {
            if let Some(p) = weak.upgrade() {
                p.engine.play.set_mute(b.is_active());
                p.update_volume_icon();
            }
        });

        // The pointer moving brings the controls back.
        let motion = gtk::EventControllerMotion::new();
        let last = Cell::new((f64::NAN, f64::NAN));
        let weak = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            if last.get() == (x, y) {
                return;
            }
            last.set((x, y));
            if let Some(p) = weak.upgrade() {
                p.reveal(true);
                if p.playing.get() {
                    p.hide_later();
                }
            }
        });
        layout.add_controller(motion);
        for revealer in [&self.top, &self.bottom] {
            let motion = gtk::EventControllerMotion::new();
            let weak = Rc::downgrade(self);
            motion.connect_enter(move |_, _, _| {
                if let Some(p) = weak.upgrade() {
                    p.on_controls.set(true);
                }
            });
            let weak = Rc::downgrade(self);
            motion.connect_leave(move |_| {
                if let Some(p) = weak.upgrade() {
                    p.on_controls.set(false);
                    if p.playing.get() {
                        p.hide_later();
                    }
                }
            });
            revealer.add_controller(motion);
        }

        // Double-click: full screen.
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |_, n, _, _| {
            if n == 2
                && let Some(p) = weak.upgrade()
            {
                p.toggle_fullscreen();
            }
        });
        self.stack.add_controller(click);

        // Keys, ahead of the buttons and the bar, which would take some.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(p) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
            use gdk::Key;
            match key {
                Key::space | Key::k => p.toggle(),
                Key::Right => p.seek_by(if shift { 60 } else { 5 }),
                Key::Left => p.seek_by(if shift { -60 } else { -5 }),
                Key::Up => p.volume.set_value((p.volume.value() + 0.05).min(1.0)),
                Key::Down => p.volume.set_value((p.volume.value() - 0.05).max(0.0)),
                Key::m => p.mute.set_active(!p.mute.is_active()),
                Key::f | Key::F11 if !ctrl => p.toggle_fullscreen(),
                Key::Escape if p.window.is_fullscreen() => p.window.unfullscreen(),
                Key::comma => p.frame(false),
                Key::period => p.frame(true),
                Key::bracketleft => p.change_speed(-1),
                Key::bracketright => p.change_speed(1),
                Key::BackSpace => p.set_rate(1.0),
                Key::Page_Down | Key::n if !ctrl => p.step(1),
                Key::Page_Up | Key::p if !ctrl => p.step(-1),
                Key::o if ctrl => p.choose(),
                Key::w | Key::q if ctrl => p.window.close(),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.window.add_controller(keys);

        // A file dropped on the window plays.
        let target = gtk::DropTarget::new(gio::File::static_type(), gdk::DragAction::COPY);
        let weak = Rc::downgrade(self);
        target.connect_drop(move |_, value, _, _| {
            let (Some(p), Ok(file)) = (weak.upgrade(), value.get::<gio::File>()) else {
                return false;
            };
            file.path().map(|path| p.open(&path)).is_some()
        });
        self.window.add_controller(target);

        // The full-screen button shows the way back while in full screen.
        let weak = Rc::downgrade(self);
        self.window.connect_fullscreened_notify(move |w| {
            if let Some(p) = weak.upgrade() {
                let (icon, tip) = if w.is_fullscreen() {
                    ("view-restore-symbolic", "play-leave-fullscreen")
                } else {
                    ("view-fullscreen-symbolic", "play-fullscreen")
                };
                p.fullscreen.set_icon_name(icon);
                p.fullscreen.set_tooltip_text(Some(&shortcut(tip, "F")));
            }
        });

        // Closing stops the sound at once.
        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(p) = weak.upgrade() {
                p.engine.play.stop();
            }
            glib::Propagation::Proceed
        });
    }

    fn update_volume_icon(&self) {
        let v = self.volume.value();
        self.mute.set_icon_name(if self.mute.is_active() || v <= 0.0 {
            "audio-volume-muted-symbolic"
        } else if v < 0.34 {
            "audio-volume-low-symbolic"
        } else if v < 0.67 {
            "audio-volume-medium-symbolic"
        } else {
            "audio-volume-high-symbolic"
        });
    }
}

/// A track's index among the streams (GTK has an `index` of its own).
fn track_index(track: &impl IsA<gst_play::PlayStreamInfo>) -> i32 {
    gst_play::prelude::PlayStreamInfoExt::index(track)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn shortcut(id: &str, key: &str) -> String {
    format!("{} ({key})", tr(id))
}

/// 3:07, 1:02:45.
fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// 1.25 → "1,25" in Portuguese; whole numbers bare.
fn decimal(x: f64) -> String {
    let s = format!("{x}");
    s.replace('.', &tr("calc-decimal-separator"))
}

/// "por" → "Português", from GStreamer's language tables; the code itself
/// when it knows none.
fn language_name(code: &str) -> String {
    gst_tag_language_name(code).unwrap_or_else(|| code.to_owned())
}

fn gst_tag_language_name(code: &str) -> Option<String> {
    // libgsttag knows ISO 639 names; reached through its C interface to
    // spare a crate for one call.
    #[link(name = "gsttag-1.0")]
    unsafe extern "C" {
        fn gst_tag_get_language_name(code: *const std::ffi::c_char) -> *const std::ffi::c_char;
    }
    let code = std::ffi::CString::new(code).ok()?;
    let name = unsafe { gst_tag_get_language_name(code.as_ptr()) };
    if name.is_null() {
        return None;
    }
    Some(unsafe { std::ffi::CStr::from_ptr(name) }.to_string_lossy().into_owned())
}

/// 1 → "mono", 2 → "estéreo", 6 → "5.1".
fn channels(n: i32) -> String {
    match n {
        1 => tr("play-mono"),
        2 => tr("play-stereo"),
        6 => "5.1".into(),
        8 => "7.1".into(),
        n => tr_args("play-channels", &[("n", number(n as f64))]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_format() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(187.4), "3:07");
        assert_eq!(clock(3765.0), "1:02:45");
    }
}
