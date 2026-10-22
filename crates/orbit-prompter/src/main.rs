//! Orbit-Prompter: the keyring's password prompts — opening a locked
//! keyring, creating a new one — in the look of the greet, in place of
//! gcr's `gcr-prompter` (PLANO-CORE §7.3 step 7b). The keyring itself
//! stays gnome-keyring; this is only what it shows.
//!
//! gnome-keyring calls `org.gnome.keyring.SystemPrompter` on the session
//! bus, and the bus starts whoever its service file names: a user's file
//! (`data/dbus/org.gnome.keyring.SystemPrompter.service` copied to
//! `~/.local/share/dbus-1/services/`) wins over the system's. Started that
//! way it takes the name (`prompter`), shows the prompts (`dialog`), and
//! quits a while after the last one.
//!
//!   orbit-prompter [-c|--config DIR]

mod dialog;
mod exchange;
mod prompter;

use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Duration};

use gtk::{gdk, gio, glib, prelude::*};
use orbit_common::{ConfigDir, style::Style};

/// How long it stays without a prompt.
const IDLE: Duration = Duration::from_secs(30);

fn main() -> glib::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let config = match config_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("orbit-prompter: {err}");
            return glib::ExitCode::FAILURE;
        }
    };
    let app = gtk::Application::builder().application_id("io.github.julas23.Orbit.Prompter").build();
    app.set_inactivity_timeout(IDLE.as_millis() as u32);
    let keep = Rc::new(RefCell::new(None));
    app.connect_activate(move |app| {
        if keep.borrow().is_some() {
            return;
        }
        let Some(display) = gdk::Display::default() else {
            log::error!("no display");
            app.quit();
            return;
        };
        let dir = ConfigDir::new(&config);
        let style = Style::install(&display, include_str!("../../../data/prompter.css"), &dir.style_path());
        let theme = orbit_app::theme::install(&display);
        let Some(conn) = app.dbus_connection() else {
            log::error!("no session bus");
            app.quit();
            return;
        };
        let prompter = match prompter::Prompter::start(app, conn.clone()) {
            Ok(p) => p,
            Err(err) => {
                log::error!("no prompter: {err}");
                app.quit();
                return;
            }
        };
        // gcr-prompter, if it runs, lets the name go to us.
        let quit = app.clone();
        let name = gio::bus_own_name_on_connection(
            &conn,
            prompter::NAME,
            gio::BusNameOwnerFlags::REPLACE | gio::BusNameOwnerFlags::ALLOW_REPLACEMENT,
            |_, name| log::info!("prompting as {name}"),
            move |_, name| {
                log::warn!("{name} taken by another prompter");
                quit.quit();
            },
        );
        // Held a moment, so the inactivity time starts counting.
        drop(app.hold());
        *keep.borrow_mut() = Some((style, theme, prompter, name));
    });
    app.run_with_args::<&str>(&[])
}

fn config_dir() -> Result<PathBuf, String> {
    let mut args = std::env::args().skip(1);
    let mut dir = ConfigDir::default_path();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-c" | "--config" => dir = args.next().ok_or("--config needs a directory")?.into(),
            "-h" | "--help" => {
                println!("Usage: orbit-prompter [-c|--config DIR]  (started by the session bus for gnome-keyring)");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(dir)
}
