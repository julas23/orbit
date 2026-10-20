//! Orbit-Play: a video and audio player — and, through Ruffle, old Flash
//! movies.
//!
//! GStreamer plays the video and the audio, with FFmpeg's decoders behind
//! it, so practically every format opens; the picture is drawn into the
//! window by GTK. Audio shows the album's cover. The folder is a playlist:
//! previous and next, and on to the next when one ends. Audio tracks,
//! subtitles (embedded, the file of the same name beside the video, or one
//! chosen), speed and frame steps. Everything starts from the beginning.

mod engine;
mod media;
mod player;

use std::{cell::RefCell, rc::Rc};

use gtk::{gio, glib, prelude::*};
use orbit_common::ConfigDir;
use player::Player;

thread_local! {
    static PLAYER: RefCell<Option<Rc<Player>>> = const { RefCell::new(None) };
}

fn main() -> glib::ExitCode {
    orbit_app::run_app(orbit_app::App {
        name: "orbit-play",
        app_id: "io.github.julas23.Orbit.Play",
        css: include_str!("../../../data/play.css"),
        build,
        resident: false,
        open: Some(open),
    })
}

fn build(app: &gtk::Application, _dir: &ConfigDir) -> anyhow::Result<gtk::ApplicationWindow> {
    let player = Player::new(app)?;
    let window = player.window().clone();
    PLAYER.with(|p| *p.borrow_mut() = Some(player));
    Ok(window)
}

/// Plays the first of `files` (a launch with files, or a second launch).
fn open(_window: &gtk::ApplicationWindow, files: &[gio::File]) {
    let Some(path) = files.first().and_then(|f| f.path()) else {
        return;
    };
    let player = PLAYER.with(|p| p.borrow().clone());
    if let Some(player) = player {
        player.open(&path);
    }
}
