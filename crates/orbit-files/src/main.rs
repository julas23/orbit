//! Orbit-Files: a file manager — Yazi's speed, in a window, with the best
//! of Thunar, Caja, Nautilus, Konqueror and Nemo.
//!
//! Folders list on a thread and fill in as they are read, so a hundred
//! thousand files never freeze it. Three views of one folder — a list
//! with columns, icons with thumbnails, and Yazi's columns with a preview
//! — tabs, two folders side by side, the places on the left (disks,
//! bookmarks, the trash, network shares through GVfs), copying and moving
//! on threads with progress, a bulk renamer, the user's own actions, and
//! archives. Keys both ways: the usual ones and Vim's.
//!
//! Options in `~/.config/orbit/files.yaml`; actions in
//! `~/.config/orbit/files-actions.yaml`.

mod actions;
mod clipboard;
mod entry;
mod help;
mod ops;
mod pane;
mod places;
mod preview;
mod properties;
mod rename;
mod thumbs;
mod window;

use std::{cell::RefCell, rc::Rc};

use entry::Location;
use gtk::{gio, glib, prelude::*};
use orbit_common::ConfigDir;
use serde::Deserialize;
use window::Window;

/// `~/.config/orbit/files.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Under "Devices": `volumes` — disks, partitions, pendrives and
    /// network shares — or `all`, every mount (folders mounted into home
    /// from a server too).
    pub devices: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            devices: "volumes".into(),
        }
    }
}

thread_local! {
    /// Every window open (launching it again opens another).
    static WINDOWS: RefCell<Vec<Rc<Window>>> = const { RefCell::new(Vec::new()) };
}

fn main() -> glib::ExitCode {
    orbit_app::run_app(orbit_app::App {
        name: "orbit-files",
        app_id: "io.github.julas23.Orbit.Files",
        css: include_str!("../../../data/files.css"),
        build,
        resident: false,
        windows: true,
        open: Some(open),
    })
}

fn build(app: &gtk::Application, dir: &ConfigDir) -> anyhow::Result<gtk::ApplicationWindow> {
    let window = Window::new(app, dir.app("files")?, actions::load(dir));
    let w = window.window().clone();
    WINDOWS.with(|cell| cell.borrow_mut().push(window.clone()));
    w.connect_destroy(|gone| {
        WINDOWS.with(|cell| cell.borrow_mut().retain(|w| w.window() != gone));
    });
    // Launched without a folder: home. (A folder given arrives first.)
    glib::idle_add_local_once(move || {
        if !window.has_tabs() {
            window.show(Location::Path(glib::home_dir()));
        }
    });
    Ok(w)
}

/// Shows the folders given — a file's folder, for a file.
fn open(window: &gtk::ApplicationWindow, files: &[gio::File]) {
    let Some(window) = WINDOWS.with(|cell| cell.borrow().iter().find(|w| w.window() == window).cloned()) else {
        return;
    };
    for file in files {
        let location = Location::of(file);
        let location = match &location {
            Location::Path(p) if p.is_file() => location.parent().unwrap_or(location),
            _ => location,
        };
        window.show(location);
    }
}
