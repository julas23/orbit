//! Orbit-Pathfinder: a web browser without telemetry, built around a
//! sidebar of services — mail, chat, work — each in storage of its own, as
//! Wavebox does. The engine is WebKitGTK 6.0 from Debian.
//!
//! The services are in `~/.config/orbit/pathfinder.yaml` (see `config`);
//! the plan is `docs/PLANO-PATHFINDER.md`.

mod bitwarden;
mod config;
mod cookies;
mod import;
mod passwords;
mod privacy;
mod service;
mod state;
mod ui;
mod window;

use std::{cell::RefCell, rc::Rc};

use gtk::{gio, glib, prelude::*};
use orbit_common::ConfigDir;

thread_local! {
    static WINDOW: RefCell<Option<Rc<window::Window>>> = const { RefCell::new(None) };
}

fn main() -> glib::ExitCode {
    if std::env::args().nth(1).as_deref() == Some(import::FIREFOX_HELPER) {
        return import::firefox_helper();
    }
    orbit_app::run_app(orbit_app::App {
        name: "orbit-pathfinder",
        app_id: "io.github.julas23.Orbit.Pathfinder",
        css: include_str!("../../../data/pathfinder.css"),
        build,
        resident: false,
        open: Some(open),
    })
}

fn build(app: &gtk::Application, dir: &ConfigDir) -> anyhow::Result<gtk::ApplicationWindow> {
    let window = window::Window::new(app, dir)?;
    let w = window.window().clone();
    WINDOW.with(|cell| *cell.borrow_mut() = Some(window));
    Ok(w)
}

/// Links from other programs (the default browser): they open in the
/// built-in browser, which comes forward.
fn open(_window: &gtk::ApplicationWindow, files: &[gio::File]) {
    let Some(window) = WINDOW.with(|cell| cell.borrow().clone()) else { return };
    for file in files {
        window.open_link(&file.uri());
    }
}
