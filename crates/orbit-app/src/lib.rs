//! Startup shared by the Orbit binaries: the `orbit-panel` and `orbit-dock`
//! surfaces ([`run`]) and the applications, such as `orbit-calc`
//! ([`run_app`]).
//!
//! Each surface runs as its own process, so a crash in one does not take the
//! other down. All read the same config directory and stylesheet; this crate
//! does the common work (arguments, logging, config, CSS, display backend)
//! and hands each binary a ready window to fill.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

use anyhow::Context;
use gtk::{gdk, gio, glib, prelude::*};
use orbit_common::{Backend, ConfigDir, Placement, config::expand_home, style::Style};

pub mod i18n;
pub mod popup;
pub mod theme;

const BUILTIN_CSS: &str = include_str!("../../../data/style.css");

/// What a binary's `build` returns: its window and where it goes, or `None`
/// when the surface is disabled in its config.
pub type Built = anyhow::Result<Option<(gtk::Window, Placement)>>;

/// What a binary contributes: its identity and how to build its surface.
pub struct Surface {
    /// Program name, used in messages (`orbit-panel`).
    pub name: &'static str,
    /// GTK application id; distinct per binary so both can run at once.
    pub app_id: &'static str,
    /// Builds the window from the config directory (each binary reads its own
    /// file there), or `None` when the surface is disabled.
    pub build: fn(&gtk::Application, &ConfigDir) -> Built,
}

/// Runs `surface` until its window is closed.
pub fn run(surface: Surface) -> glib::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let config_dir = match parse_args(surface.name, false, false) {
        Ok((path, _, _)) => path,
        Err(err) => {
            eprintln!("{}: {err}", surface.name);
            return glib::ExitCode::FAILURE;
        }
    };

    let app = gtk::Application::builder()
        .application_id(surface.app_id)
        .build();
    // Holds the stylesheet watcher and the theme for the lifetime of the
    // application.
    let style = RefCell::new(None);
    app.connect_activate(move |app| {
        // A second invocation re-activates the running instance.
        if !app.windows().is_empty() {
            return;
        }
        match start(app, &config_dir, &surface) {
            Ok(s) => *style.borrow_mut() = s,
            Err(err) => {
                log::error!("{err:#}");
                app.quit();
            }
        }
    });
    // Arguments are already parsed; don't let GApplication reject them.
    app.run_with_args::<&str>(&[])
}

/// What an application — an ordinary window, not a surface on a screen edge
/// — contributes: its identity, its stylesheet and how to build its window.
pub struct App {
    /// Program name, used in messages (`orbit-calc`).
    pub name: &'static str,
    pub app_id: &'static str,
    /// The built-in stylesheet; the user's style.css goes on top of it.
    pub css: &'static str,
    pub build: fn(&gtk::Application, &ConfigDir) -> anyhow::Result<gtk::ApplicationWindow>,
    /// Keeps running in the background, like a clipboard manager: started
    /// with `--background` it shows no window, closing the window only hides
    /// it, and launching the program again shows or hides it.
    pub resident: bool,
    /// Takes files on the command line, like an image viewer: they are
    /// handed to this after the window is built, and a second launch with
    /// files hands them to the running window.
    pub open: Option<fn(&gtk::ApplicationWindow, &[gio::File])>,
}

/// Runs `spec` until its window is closed. Logging, arguments, the
/// stylesheet and the system's colours are set up as for the panel; a second
/// launch brings the running window forward (or, for a resident app, shows
/// or hides it).
pub fn run_app(spec: App) -> glib::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let (config_dir, background, files) =
        match parse_args(spec.name, spec.resident, spec.open.is_some()) {
            Ok(parsed) => parsed,
            Err(err) => {
                eprintln!("{}: {err}", spec.name);
                return glib::ExitCode::FAILURE;
            }
        };

    let mut app = gtk::Application::builder().application_id(spec.app_id);
    if spec.open.is_some() {
        app = app.flags(gio::ApplicationFlags::HANDLES_OPEN);
    }
    let app = app.build();
    // The window, built on the first activation or open.
    let keep = std::rc::Rc::new(RefCell::new(None));
    let window = {
        let keep = keep.clone();
        move |app: &gtk::Application| -> Option<gtk::ApplicationWindow> {
            if let Some(window) = app.windows().first() {
                return window.clone().downcast().ok();
            }
            let Some(display) = gdk::Display::default() else {
                log::error!("no display");
                app.quit();
                return None;
            };
            let dir = ConfigDir::new(&config_dir);
            let style = Style::install(&display, spec.css, &dir.style_path());
            let theme = theme::install(&display);
            match (spec.build)(app, &dir) {
                Ok(window) => {
                    if spec.resident {
                        window.set_hide_on_close(true);
                    }
                    *keep.borrow_mut() = Some((style, theme));
                    if !background {
                        window.present();
                    }
                    Some(window)
                }
                Err(err) => {
                    log::error!("{err:#}");
                    app.quit();
                    None
                }
            }
        }
    };
    {
        let window = window.clone();
        app.connect_activate(move |app| {
            let built = !app.windows().is_empty();
            let Some(window) = window(app) else { return };
            if built {
                if spec.resident && window.is_visible() {
                    window.set_visible(false);
                } else {
                    window.present();
                }
            }
        });
    }
    if let Some(open) = spec.open {
        app.connect_open(move |app, files, _| {
            if let Some(window) = window(app) {
                open(&window, files);
                window.present();
            }
        });
    }
    // Arguments are already parsed; GApplication only gets the files, which
    // it opens here or hands to the instance already running.
    let mut args = vec![spec.name.to_owned()];
    args.extend(files);
    app.run_with_args(&args)
}

/// The config directory, whether `--background` was given (when
/// `background` is accepted) and the files given (when `files` are).
fn parse_args(
    name: &str,
    background: bool,
    files: bool,
) -> anyhow::Result<(PathBuf, bool, Vec<String>)> {
    let mut args = std::env::args().skip(1);
    let mut config = None;
    let mut hidden = false;
    let mut paths = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-c" | "--config" => {
                config = Some(PathBuf::from(
                    args.next().context("--config needs a directory")?,
                ));
            }
            "--background" if background => hidden = true,
            "-h" | "--help" => {
                let extra = if background { " [--background]" } else { "" };
                let extra = if files { " [FILE…]" } else { extra };
                println!("Usage: {name} [-c|--config DIR]{extra}  (default: ~/.config/orbit)");
                std::process::exit(0);
            }
            "--" if files => paths.extend(args.by_ref()),
            other if files && !other.starts_with('-') => paths.push(other.to_owned()),
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    Ok((config.unwrap_or_else(ConfigDir::default_path), hidden, paths))
}

fn start(
    app: &gtk::Application,
    config_dir: &Path,
    surface: &Surface,
) -> anyhow::Result<Option<(Style, theme::Theme)>> {
    let dir = ConfigDir::new(config_dir);

    let Some((window, placement)) = (surface.build)(app, &dir)? else {
        log::info!("{} is disabled in the config, exiting", surface.name);
        app.quit();
        return Ok(None);
    };

    let display = gdk::Display::default().context("no display")?;
    let backend = backend_for(&display)?;
    log::info!("using the {} backend", backend.name());

    let style = Style::install(&display, BUILTIN_CSS, &dir.style_path());
    // The system's light/dark choice and accent colour, followed live.
    let theme = theme::install(&display);
    backend.attach(&window, placement)?;
    window.present();
    Ok(Some((style, theme)))
}

fn backend_for(display: &gdk::Display) -> anyhow::Result<Box<dyn Backend>> {
    if display.is::<gdk4_x11::X11Display>() {
        return Ok(Box::new(orbit_backend_x11::X11Backend::new()?));
    }
    if display.is::<gdk4_wayland::WaylandDisplay>() {
        return Ok(Box::new(orbit_backend_wayland::WaylandBackend::new()?));
    }
    anyhow::bail!("unsupported display backend: {}", display.type_().name())
}

/// An image from either a file path or an icon-theme name.
pub fn icon(spec: &str, size: i32) -> gtk::Image {
    let image = gtk::Image::new();
    if spec.contains('/') {
        set_file(&image, &expand_home(spec.as_ref()), size);
    } else {
        image.set_icon_name(Some(spec));
    }
    image.set_pixel_size(size);
    image
}

/// Shows `gicon` in `image` at `size`. Icons given as files are loaded
/// already scaled down: GTK would otherwise keep each one decoded at its full
/// size — AppImage icons are often 1024 px or more (one was 3840 px, 59 MB),
/// and the dock's ~40 file icons took ~100 MB to show at 24 px.
pub fn set_gicon(image: &gtk::Image, gicon: &gio::Icon, size: i32) {
    let path = gicon
        .downcast_ref::<gio::FileIcon>()
        .and_then(|icon| icon.file().path());
    match path {
        Some(path) => set_file(image, &path, size),
        None => image.set_from_gicon(gicon),
    }
    image.set_pixel_size(size);
}

/// Loads `path` at twice `size` (sharp on HiDPI, still tiny in memory).
fn set_file(image: &gtk::Image, path: &Path, size: i32) {
    let side = size.max(1) * 2;
    match gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(path, side, side, true) {
        Ok(pixbuf) => image.set_paintable(Some(&gdk::Texture::for_pixbuf(&pixbuf))),
        Err(err) => {
            log::warn!("icon {}: {err}", path.display());
            image.set_icon_name(Some("application-x-executable"));
        }
    }
}
