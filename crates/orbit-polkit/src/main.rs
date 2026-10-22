//! Orbit-Polkit: the polkit authentication agent of the Orbit session.
//! When a program needs an administrator's rights (pkexec, a disk to
//! mount, a package to install), polkitd asks the session's agent; this
//! one shows the request over everything, with the look of the greet, and
//! lets polkit's own helper check the password (`agent`, `dialog`,
//! `helper`). One per session: it quits if another agent holds it.
//!
//!   orbit-polkit [-c|--config DIR]

mod agent;
mod dialog;
mod helper;
mod identity;

use std::{path::PathBuf, rc::Rc};

use gtk::{gdk, glib, prelude::*};
use orbit_common::{ConfigDir, style::Style};

fn main() -> glib::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let config = match config_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("orbit-polkit: {err}");
            return glib::ExitCode::FAILURE;
        }
    };
    // Unique: a second one started hands over to the first and quits.
    let app = gtk::Application::builder().application_id("io.github.julas23.Orbit.Polkit").build();
    let keep = Rc::new(std::cell::RefCell::new(None));
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
        let style = Style::install(&display, include_str!("../../../data/polkit.css"), &dir.style_path());
        let theme = orbit_app::theme::install(&display);
        match agent::Agent::start(app) {
            Ok(agent) => *keep.borrow_mut() = Some((style, theme, agent, app.hold())),
            Err(err) => {
                log::error!("no agent: {err}");
                app.quit();
            }
        }
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
                println!("Usage: orbit-polkit [-c|--config DIR]  (the session's polkit agent)");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(dir)
}

