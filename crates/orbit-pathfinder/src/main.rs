//! Orbit-Pathfinder: a web browser without telemetry, built around a
//! sidebar of services — mail, chat, work — each in storage of its own, as
//! Wavebox does. The engine is WebKitGTK 6.0 from Debian.
//!
//! The services are in `~/.config/orbit/pathfinder.yaml` (see `config`);
//! the plan is `docs/PLANO-PATHFINDER.md`.

mod config;
mod cookies;
mod privacy;
mod service;
mod state;
mod window;

use std::{cell::RefCell, rc::Rc};

use gtk::glib;
use orbit_common::ConfigDir;

thread_local! {
    static WINDOW: RefCell<Option<Rc<window::Window>>> = const { RefCell::new(None) };
}

fn main() -> glib::ExitCode {
    orbit_app::run_app(orbit_app::App {
        name: "orbit-pathfinder",
        app_id: "io.github.julas23.Orbit.Pathfinder",
        css: include_str!("../../../data/pathfinder.css"),
        build,
        resident: false,
        open: None,
    })
}

fn build(app: &gtk::Application, dir: &ConfigDir) -> anyhow::Result<gtk::ApplicationWindow> {
    let window = window::Window::new(app, dir)?;
    let w = window.window().clone();
    WINDOW.with(|cell| *cell.borrow_mut() = Some(window));
    Ok(w)
}
