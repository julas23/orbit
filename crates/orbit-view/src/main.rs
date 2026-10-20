//! Orbit-View: an image viewer.
//!
//! The image fills the window; the controls show while the pointer moves.
//! Opening an image lists its folder, browsed with the arrow keys or the
//! strip of thumbnails. It zooms around the pointer, turns and mirrors
//! (saving that into JPEG and PNG files), shows the file's and the camera's
//! details, plays animations and slideshows, and sends images to the
//! trash. Formats are decoded by glycin, each in a sandbox: PNG, JPEG, GIF,
//! WebP, AVIF, HEIC, JPEG XL, SVG, TIFF and more, as installed.
//!
//! Options in `~/.config/orbit/view.yaml`.

mod canvas;
mod decode;
mod edit;
mod folder;
mod info;
mod pixels;
mod thumbs;
mod viewer;

use std::{cell::RefCell, rc::Rc};

use gtk::{gio, glib, prelude::*};
use orbit_common::ConfigDir;
use viewer::Viewer;

thread_local! {
    static VIEWER: RefCell<Option<Rc<Viewer>>> = const { RefCell::new(None) };
}

fn main() -> glib::ExitCode {
    orbit_app::run_app(orbit_app::App {
        name: "orbit-view",
        app_id: "io.github.julas23.Orbit.View",
        css: include_str!("../../../data/view.css"),
        build,
        resident: false,
        open: Some(open),
    })
}

fn build(app: &gtk::Application, dir: &ConfigDir) -> anyhow::Result<gtk::ApplicationWindow> {
    let viewer = Viewer::new(app, dir.app("view")?);
    let window = viewer.window().clone();
    VIEWER.with(|v| *v.borrow_mut() = Some(viewer));
    Ok(window)
}

/// Shows the first of `files` (a launch with files, or a second launch).
fn open(_window: &gtk::ApplicationWindow, files: &[gio::File]) {
    let Some(path) = files.first().and_then(|f| f.path()) else {
        return;
    };
    let viewer = VIEWER.with(|v| v.borrow().clone());
    if let Some(viewer) = viewer {
        viewer.open(&path);
    }
}
