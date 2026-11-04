//! Orbit-Wallpaper: a video (or an animated GIF) as the wallpaper of every
//! monitor — silent, looping without a gap, on the GPU where it can
//! (VA-API through `gtk4paintablesink`'s GL path).
//!
//!   orbit-wallpaper [--mode fill|fit|center|stretch] FILE
//!
//! orbit-core starts it for `wallpaper.video` in core.yaml and talks to it
//! on its standard input, a word a line: `pause` (the wallpaper cannot be
//! seen: locked, a fullscreen window, on battery with the effects off) and
//! `play`. When the input closes — the core gone, or the video taken out
//! of the config — it ends. Under it, the core's own wallpaper (picture or
//! colour) shows around a video that does not fill the monitor, and in its
//! place if the file cannot be played.
//!
//! Each monitor gets a background layer surface showing the same picture;
//! monitors that come and go get theirs.
//!
//!   orbit-wallpaper --music
//!
//! What is playing over the spectrum sphere (`effect: scope`): the cover,
//! the song and the lyrics (`music.rs`). The core starts it too, and its
//! input closing ends it the same way.

mod music;

use std::{cell::RefCell, io::BufRead, path::PathBuf, rc::Rc};

use gst::prelude::*;
use gtk::{gdk, gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

fn main() -> glib::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut args = std::env::args().skip(1);
    let (mut mode, mut file) = ("fill".to_owned(), None);
    let mut music = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--music" => music = true,
            "--mode" => mode = args.next().unwrap_or_default(),
            "-h" | "--help" => {
                println!("Usage: orbit-wallpaper [--mode fill|fit|center|stretch] FILE");
                return glib::ExitCode::SUCCESS;
            }
            other => file = Some(PathBuf::from(other)),
        }
    }
    if music {
        let app = gtk::Application::builder()
            .application_id("io.github.julas23.Orbit.Wallpaper.Music")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.connect_activate(|app| {
            music::run(app);
            // Held until the core closes the input.
            let hold = app.hold();
            let (sender, receiver) = async_channel::bounded::<()>(1);
            std::thread::spawn(move || {
                for line in std::io::stdin().lock().lines() {
                    if line.is_err() {
                        break;
                    }
                }
                let _ = sender.send_blocking(());
            });
            let app = app.clone();
            glib::spawn_future_local(async move {
                let _ = receiver.recv().await;
                drop(hold);
                app.quit();
            });
        });
        return app.run_with_args::<&str>(&[]);
    }
    let Some(file) = file else {
        eprintln!("orbit-wallpaper: no file");
        return glib::ExitCode::FAILURE;
    };
    let fit = match mode.as_str() {
        "fit" => gtk::ContentFit::Contain,
        "center" => gtk::ContentFit::ScaleDown,
        "stretch" => gtk::ContentFit::Fill,
        _ => gtk::ContentFit::Cover,
    };
    let app = gtk::Application::builder()
        .application_id("io.github.julas23.Orbit.Wallpaper")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let keep: Rc<RefCell<Option<Player>>> = Rc::default();
    app.connect_activate(move |app| match Player::new(&file) {
        Ok(player) => {
            player.show(app, fit);
            listen(app, player.playbin.clone());
            *keep.borrow_mut() = Some(player);
        }
        Err(err) => {
            log::error!("{}: {err:#}", file.display());
            app.quit();
        }
    });
    app.run_with_args::<&str>(&[])
}

struct Player {
    playbin: gst::Element,
    paintable: gdk::Paintable,
    /// The surfaces, one per monitor.
    windows: Rc<RefCell<Vec<gtk::Window>>>,
}

impl Player {
    fn new(file: &std::path::Path) -> anyhow::Result<Self> {
        gst::init()?;
        gstgtk4::plugin_register_static()?;
        let gtk_sink = gst::ElementFactory::make("gtk4paintablesink").build()?;
        let paintable = gtk_sink.property::<gdk::Paintable>("paintable");
        let has_gl = paintable.property::<Option<gdk::GLContext>>("gl-context").is_some();
        // As Orbit-Play: through GL, frames decoded on the GPU stay there.
        let sink: gst::Element = if has_gl {
            gst::ElementFactory::make("glsinkbin")
                .property("sink", &gtk_sink)
                .build()?
        } else {
            let bin = gst::Bin::new();
            let convert = gst::ElementFactory::make("videoconvert").build()?;
            bin.add_many([&convert, &gtk_sink])?;
            convert.link(&gtk_sink)?;
            let pad = convert.static_pad("sink").expect("videoconvert has a sink");
            bin.add_pad(&gst::GhostPad::with_target(&pad)?)?;
            bin.upcast()
        };
        let path = std::fs::canonicalize(file)?;
        let uri = glib::filename_to_uri(&path, None)?.to_string();
        let playbin = gst::ElementFactory::make("playbin")
            .property("uri", &uri)
            .property("video-sink", &sink)
            .build()?;
        // The picture alone: the sound is never decoded.
        playbin.set_property_from_str("flags", "video");
        // Gapless: the same file queued again just before the end.
        playbin.connect("about-to-finish", false, move |values| {
            if let Ok(playbin) = values[0].get::<gst::Element>() {
                playbin.set_property("uri", &uri);
            }
            None
        });
        let bus = playbin.bus().expect("a pipeline has a bus");
        let watched = playbin.downgrade();
        let watch = bus.add_watch_local(move |_, message| {
            let Some(playbin) = watched.upgrade() else {
                return glib::ControlFlow::Break;
            };
            match message.view() {
                // A stream that would not queue again: back to the start.
                gst::MessageView::Eos(_) => {
                    let _ = playbin.seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT, gst::ClockTime::ZERO);
                }
                gst::MessageView::Error(err) => {
                    log::error!(
                        "{}: {}",
                        err.src().map(|s| s.path_string()).unwrap_or_default(),
                        err.error()
                    );
                    // Gone, so the core's own wallpaper shows.
                    std::process::exit(1);
                }
                _ => {}
            }
            glib::ControlFlow::Continue
        })?;
        // The watch lives as long as the program.
        std::mem::forget(watch);
        playbin.set_state(gst::State::Playing)?;
        Ok(Self {
            playbin,
            paintable,
            windows: Rc::default(),
        })
    }

    /// A surface on every monitor, made again when they change.
    fn show(&self, app: &gtk::Application, fit: gtk::ContentFit) {
        let Some(display) = gdk::Display::default() else {
            return;
        };
        let provider = gtk::CssProvider::new();
        provider.load_from_string("window.orbit-wallpaper { background: none; }");
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        let monitors = display.monitors();
        let make = {
            let (app, paintable, windows) = (app.clone(), self.paintable.clone(), self.windows.clone());
            move |monitors: &gio::ListModel| {
                for window in windows.borrow_mut().drain(..) {
                    window.destroy();
                }
                for monitor in (0..monitors.n_items()).filter_map(|i| monitors.item(i).and_downcast::<gdk::Monitor>()) {
                    let window = surface(&app, &monitor, &paintable, fit);
                    window.present();
                    windows.borrow_mut().push(window);
                }
            }
        };
        make(&monitors);
        monitors.connect_items_changed(move |monitors, _, _, _| make(monitors));
    }
}

fn surface(
    app: &gtk::Application,
    monitor: &gdk::Monitor,
    paintable: &gdk::Paintable,
    fit: gtk::ContentFit,
) -> gtk::Window {
    let window = gtk::Window::builder()
        .application(app)
        .css_classes(["orbit-wallpaper"])
        .build();
    window.init_layer_shell();
    window.set_namespace(Some("orbit-wallpaper"));
    window.set_layer(Layer::Background);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    // The whole monitor, under the panel's and the dock's zones too.
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_monitor(Some(monitor));
    let picture = gtk::Picture::builder()
        .paintable(paintable)
        .content_fit(fit)
        .can_shrink(true)
        .build();
    window.set_child(Some(&picture));
    window
}

/// The core's words on standard input; its end ends the program.
fn listen(app: &gtk::Application, playbin: gst::Element) {
    let (sender, receiver) = async_channel::unbounded::<Option<String>>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if sender.send_blocking(Some(line)).is_err() {
                return;
            }
        }
        let _ = sender.send_blocking(None);
    });
    let app = app.clone();
    glib::spawn_future_local(async move {
        while let Ok(Some(line)) = receiver.recv().await {
            let state = match line.trim() {
                "pause" => gst::State::Paused,
                "play" => gst::State::Playing,
                other => {
                    log::warn!("unknown word: {other}");
                    continue;
                }
            };
            if let Err(err) = playbin.set_state(state) {
                log::warn!("{state:?}: {err}");
            }
        }
        let _ = playbin.set_state(gst::State::Null);
        app.quit();
    });
}
