//! Orbit-Blink: screen capture — the whole screen, the focused window, or an
//! area chosen on the frozen screen.
//!
//! ```text
//! orbit-blink screen|window|area [--delay SECONDS] [--cursor]
//! orbit-blink                        # the options window
//! ```
//!
//! Each capture is saved as PNG in the home directory, put on the clipboard
//! (the process stays, invisible, serving it until something else is
//! copied) and announced by a card with a thumbnail and buttons to open it
//! or save a copy elsewhere.

mod area;
mod capture;
mod card;
mod clipboard;
mod image;
mod options;

use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Duration};

use anyhow::Context;
use gtk::{gdk, gio, glib, prelude::*};
use orbit_app::i18n::tr;
use orbit_common::{ConfigDir, style::Style};

use crate::{capture::Screen, image::Image};

const CSS: &str = include_str!("../../../data/blink.css");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Screen,
    Window,
    Area,
}

#[derive(Debug, Clone, Copy)]
pub struct Request {
    pub mode: Mode,
    pub delay: Duration,
    pub cursor: bool,
}

struct Args {
    config: PathBuf,
    request: Option<Request>,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut args = std::env::args().skip(1);
    let mut config = None;
    let mut mode = None;
    let mut delay = Duration::ZERO;
    let mut cursor = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "screen" => mode = Some(Mode::Screen),
            "window" => mode = Some(Mode::Window),
            "area" => mode = Some(Mode::Area),
            "--cursor" => cursor = true,
            "--delay" => {
                let s: f64 = args
                    .next()
                    .context("--delay needs a number of seconds")?
                    .parse()
                    .context("--delay needs a number of seconds")?;
                delay = Duration::from_secs_f64(s.clamp(0.0, 60.0));
            }
            "-c" | "--config" => {
                config = Some(PathBuf::from(
                    args.next().context("--config needs a directory")?,
                ));
            }
            "-h" | "--help" => {
                println!(
                    "Usage: orbit-blink [screen|window|area] [--delay SECONDS] [--cursor] \
                     [-c|--config DIR]\n\
                     Without a mode, opens the options window."
                );
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    Ok(Args {
        config: config.unwrap_or_else(ConfigDir::default_path),
        request: mode.map(|mode| Request {
            mode,
            delay,
            cursor,
        }),
    })
}

fn main() -> glib::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = match parse_args() {
        Ok(args) => args,
        Err(err) => {
            eprintln!("orbit-blink: {err}");
            return glib::ExitCode::FAILURE;
        }
    };
    // Every capture its own process: one may still be serving the clipboard
    // when the next key is pressed.
    let app = gtk::Application::builder()
        .application_id("io.github.julas23.Orbit.Blink")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let keep = RefCell::new(None);
    app.connect_activate(move |app| {
        let Some(display) = gdk::Display::default() else {
            log::error!("no display");
            return;
        };
        let dir = ConfigDir::new(&args.config);
        let style = Style::install(&display, CSS, &dir.style_path());
        let theme = orbit_app::theme::install(&display);
        *keep.borrow_mut() = Some((style, theme));
        match args.request {
            Some(request) => start(app, request),
            None => options::show(app),
        }
    });
    app.run_with_args::<&str>(&[])
}

/// Takes a capture, after the delay asked for.
pub fn start(app: &gtk::Application, request: Request) {
    let hold = app.hold();
    let app = app.clone();
    glib::timeout_add_local_once(request.delay, move || {
        let _hold = hold;
        let screens = match capture::screens(request.cursor) {
            Ok(screens) => Rc::new(screens),
            Err(err) => return fail(&app, &format!("{err:#}")),
        };
        let focused = screens.iter().position(|s| s.focused).unwrap_or(0);
        match request.mode {
            Mode::Screen => {
                let image = screens[focused].image.clone();
                finish(&app, &screens, image);
            }
            Mode::Window => match capture::focused_window().and_then(|w| crop(&screens, w)) {
                Some(image) => finish(&app, &screens, image),
                None => fail(&app, &tr("blink-no-window")),
            },
            Mode::Area => {
                let windows = capture::windows();
                let screens2 = screens.clone();
                let app2 = app.clone();
                let hold = app.hold();
                area::choose(&app, screens, windows, move |rect| {
                    let _hold = hold;
                    if let Some(image) = rect.and_then(|r| crop(&screens2, r)) {
                        finish(&app2, &screens2, image);
                    }
                });
            }
        }
    });
}

/// `rect` (global logical pixels) from the screen it overlaps most.
fn crop(screens: &[Screen], rect: image::Rect) -> Option<Image> {
    screens
        .iter()
        .filter_map(|s| {
            let part = rect.intersect(s.logical)?;
            Some((part.width * part.height, s))
        })
        .max_by_key(|(area, _)| *area)
        .and_then(|(_, s)| s.crop(rect))
}

/// The monitor of the focused screen, for the card.
fn focused_monitor(screens: &[Screen]) -> Option<gdk::Monitor> {
    let name = screens.iter().find(|s| s.focused).map(|s| s.name.clone())?;
    let monitors = gdk::Display::default()?.monitors();
    (0..monitors.n_items())
        .filter_map(|i| monitors.item(i).and_downcast::<gdk::Monitor>())
        .find(|m| m.connector().as_deref() == Some(name.as_str()))
}

/// Saves, copies and announces the capture.
fn finish(app: &gtk::Application, screens: &[Screen], image: Image) {
    let monitor = focused_monitor(screens);
    let result = (|| -> anyhow::Result<PathBuf> {
        let png = image.png()?;
        let path = file_path();
        std::fs::write(&path, &png).with_context(|| format!("writing {}", path.display()))?;
        // On the clipboard until something else is copied.
        let (done, copied_over) = async_channel::bounded(1);
        clipboard::offer(png, done)?;
        let hold = app.hold();
        glib::spawn_future_local(async move {
            let _ = copied_over.recv().await;
            drop(hold);
        });
        Ok(path)
    })();
    match result {
        Ok(path) => {
            log::info!("saved {}", path.display());
            card::saved(app, monitor.as_ref(), &image, path);
        }
        Err(err) => card::failed(app, monitor.as_ref(), &format!("{err:#}")),
    }
}

fn fail(app: &gtk::Application, message: &str) {
    log::error!("{message}");
    card::failed(app, None, message);
}

/// `~/<prefix>-2026-10-19_22-50-12.png`, made unique if taken.
fn file_path() -> PathBuf {
    let prefix = tr("blink-file-prefix");
    let stamp = glib::DateTime::now_local()
        .and_then(|d| d.format("%Y-%m-%d_%H-%M-%S"))
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "now".into());
    let home = glib::home_dir();
    let mut path = home.join(format!("{prefix}-{stamp}.png"));
    let mut n = 2;
    while path.exists() {
        path = home.join(format!("{prefix}-{stamp}-{n}.png"));
        n += 1;
    }
    path
}
