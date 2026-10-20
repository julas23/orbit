//! Orbit-Edit: a text and code editor.
//!
//! On GtkSourceView: highlighting for hundreds of languages, line numbers,
//! search and replace with regular expressions, undo, the minimap. On top,
//! of its own: several cursors (Ctrl+D, Ctrl+click) and column selection
//! (Alt+drag), indent guides and folding by indentation, the folder in a
//! sidebar with Ctrl+P to find a file in it, comments toggled with Ctrl+/,
//! and the session — tabs and unsaved text — kept between runs.
//!
//! Options in `~/.config/orbit/edit.yaml`.

mod doc;
mod editor;
mod findbar;
mod fold;
mod multi;
mod session;
mod sidebar;

use std::{cell::RefCell, rc::Rc};

use editor::Editor;
use gtk::{gio, glib, prelude::*};
use orbit_common::ConfigDir;
use serde::Deserialize;

/// `~/.config/orbit/edit.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub font: String,
    /// In points.
    pub font_size: f64,
    pub tab_width: u32,
    /// For new files; a file's own indentation is read from it.
    pub insert_spaces: bool,
    pub minimap: bool,
    pub wrap: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font: "CaskaydiaCove Nerd Font Mono".into(),
            font_size: 11.0,
            tab_width: 4,
            insert_spaces: true,
            minimap: true,
            wrap: false,
        }
    }
}

thread_local! {
    static EDITOR: RefCell<Option<Rc<Editor>>> = const { RefCell::new(None) };
}

fn main() -> glib::ExitCode {
    orbit_app::run_app(orbit_app::App {
        name: "orbit-edit",
        app_id: "io.github.julas23.Orbit.Edit",
        css: include_str!("../../../data/edit.css"),
        build,
        resident: false,
        open: Some(open),
    })
}

fn build(app: &gtk::Application, dir: &ConfigDir) -> anyhow::Result<gtk::ApplicationWindow> {
    let editor = Editor::new(app, dir.app("edit")?);
    let window = editor.window().clone();
    EDITOR.with(|e| *e.borrow_mut() = Some(editor));
    Ok(window)
}

/// Opens `files` (a launch with files, or a second launch).
fn open(_window: &gtk::ApplicationWindow, files: &[gio::File]) {
    let editor = EDITOR.with(|e| e.borrow().clone());
    if let Some(editor) = editor {
        for path in files.iter().filter_map(|f| f.path()) {
            editor.open(&path);
        }
    }
}
