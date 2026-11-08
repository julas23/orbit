//! Orbit-Edit: a text and code editor.
//!
//! On GtkSourceView: highlighting for hundreds of languages, line numbers,
//! search and replace with regular expressions, undo, the minimap. On top,
//! of its own: several cursors (Ctrl+D, Ctrl+click) and column selection
//! (Alt+drag), indent guides and folding by indentation, the folder in a
//! sidebar with Ctrl+P to find a file in it, comments toggled with Ctrl+/,
//! and the session — tabs and unsaved text — kept between runs. And: the
//! file watched on disk, saved as root when it must be, language servers
//! (`lsp`), the git marks, search in every file, the command palette,
//! the view split, the Markdown preview, a terminal, spelling, the
//! folder's .editorconfig, formatting and saving by itself.
//!
//! Options in `~/.config/orbit/edit.yaml`.

mod doc;
mod editor;
mod editorconfig;
mod findbar;
mod fold;
mod gitmarks;
mod lsp;
mod multi;
mod palette;
mod preview;
mod search;
mod session;
mod sidebar;
mod terminal;

use std::{cell::RefCell, collections::HashMap, rc::Rc};

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
    /// Spaces at the ends of lines taken off on saving; a newline put at
    /// the end. A folder's .editorconfig says otherwise for its files.
    pub trim_trailing: bool,
    pub final_newline: bool,
    pub show_whitespace: bool,
    /// Files saved by themselves a moment after the typing stops, and
    /// when the window loses the focus.
    pub autosave: bool,
    /// Spelling checked as one types (comments and strings, in code).
    pub spell: bool,
    /// Formatted on saving, by the language server or by `formatters`.
    pub format_on_save: bool,
    /// A program to format a language with, by GtkSourceView's name of
    /// it: the text on its standard input, the result on its output
    /// (`rust: [rustfmt, --edition, "2024"]`). Without one, the language
    /// server formats.
    pub formatters: HashMap<String, Vec<String>>,
    /// The language servers, by GtkSourceView's name of the language, in
    /// place of the usual ones; `[]` for none.
    pub lsp: HashMap<String, Vec<String>>,
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
            trim_trailing: false,
            final_newline: false,
            show_whitespace: false,
            autosave: false,
            spell: true,
            format_on_save: false,
            formatters: HashMap::new(),
            lsp: HashMap::new(),
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
        windows: false,
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
