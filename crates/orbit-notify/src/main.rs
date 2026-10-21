//! Orbit-Notify: the desktop's notification server, in place of dunst.
//!
//! Started with `--background` when the session starts, it serves
//! `org.freedesktop.Notifications`: bubbles in the screen's corner, and a
//! centre on the right edge that keeps them, grouped by app. Launching it
//! again (or clicking the panel's bell) shows or hides the centre.
//!
//!   orbit-notify [--background]    the server (a second launch: the centre)
//!   orbit-notify toggle|dnd|clear  tell the running server
//!
//! Options in `~/.config/orbit/notify.yaml`; the history in
//! `~/.local/state/orbit/notify/history.json`. See docs/PLANO-NOTIFY.md.

mod bubbles;
mod card;
mod centre;
mod config;
mod daemon;
mod note;

use std::{cell::RefCell, rc::Rc};

use gtk::{gio, glib, prelude::*};
use orbit_common::ConfigDir;

thread_local! {
    static DAEMON: RefCell<Option<Rc<daemon::Daemon>>> = const { RefCell::new(None) };
}

fn main() -> glib::ExitCode {
    if let Some(command) = std::env::args().nth(1).filter(|a| matches!(a.as_str(), "toggle" | "dnd" | "clear")) {
        return match tell(&command) {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("orbit-notify: {err}");
                glib::ExitCode::FAILURE
            }
        };
    }
    orbit_app::run_app(orbit_app::App {
        name: "orbit-notify",
        app_id: "io.github.julas23.Orbit.Notify",
        css: include_str!("../../../data/notify.css"),
        build,
        resident: true,
        open: None,
    })
}

fn build(app: &gtk::Application, dir: &ConfigDir) -> anyhow::Result<gtk::ApplicationWindow> {
    let config: config::Config = dir.app("notify")?;
    let spec = match &config.monitor {
        Some(spec) => Some(spec.clone()),
        None => dir.panel().ok().map(|p| p.monitor),
    };
    let monitor = match (spec, gtk::gdk::Display::default()) {
        (Some(spec), Some(display)) => orbit_common::backend::monitor(&display, &spec).ok(),
        _ => None,
    };
    let daemon = daemon::Daemon::start(app, config, monitor)?;
    let window = daemon.centre().map(|c| c.window.clone()).ok_or_else(|| anyhow::anyhow!("no centre"))?;
    app.connect_shutdown(|_| DAEMON.with(|d| d.borrow().as_ref().map(|d| d.save_now())).unwrap_or(()));
    DAEMON.with(|d| *d.borrow_mut() = Some(daemon));
    Ok(window)
}

/// Sends `command` to the running server.
fn tell(command: &str) -> anyhow::Result<()> {
    let conn = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)?;
    let call = |method: &str, args: Option<glib::Variant>| {
        conn.call_sync(
            Some(daemon::OWN),
            daemon::OWN_PATH,
            daemon::OWN,
            method,
            args.as_ref(),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            2000,
            gio::Cancellable::NONE,
        )
    };
    match command {
        "toggle" => call("Toggle", None)?,
        "clear" => call("Clear", None)?,
        _ => {
            let on = conn
                .call_sync(
                    Some(daemon::OWN),
                    daemon::OWN_PATH,
                    "org.freedesktop.DBus.Properties",
                    "Get",
                    Some(&(daemon::OWN, "Dnd").to_variant()),
                    None,
                    gio::DBusCallFlags::NO_AUTO_START,
                    2000,
                    gio::Cancellable::NONE,
                )?
                .child_value(0)
                .as_variant()
                .and_then(|v| v.get::<bool>())
                .unwrap_or(false);
            call("SetDnd", Some((!on,).to_variant()))?
        }
    };
    Ok(())
}
