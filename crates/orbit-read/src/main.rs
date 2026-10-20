//! Orbit-Read: a reader for PDF, Markdown and plain text.
//!
//! PDFs scroll page after page, drawn by poppler: zoom (fitted to the width
//! by default), a sidebar with the outline and the pages, search, text
//! selection and copying, links, night mode and printing; a document with
//! a password asks for it. Markdown is laid out — headings, lists, code,
//! tables, images, links — with its source a button away. Plain text in a
//! monospaced font. Documents always open at the top.

mod actions;
mod markdown;
mod pdf;
mod reader;
mod textdoc;

use std::{cell::RefCell, path::Path, rc::Rc};

use gtk::{gio, glib, prelude::*};
use orbit_common::ConfigDir;
use reader::Reader;

/// The types it opens.
pub const MIME_TYPES: &[&str] = &["application/pdf", "text/markdown", "text/x-markdown", "text/plain"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Pdf,
    Markdown,
    Text,
}

/// What `path` is, by its name; `None` for a file that is not a document
/// this reads.
pub fn kind_of(path: &Path) -> Option<Kind> {
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "pdf" => return Some(Kind::Pdf),
        "md" | "markdown" | "mkd" => return Some(Kind::Markdown),
        _ => {}
    }
    let (kind, _) = gio::content_type_guess(Some(path), None);
    if gio::content_type_is_a(&kind, "application/pdf") {
        Some(Kind::Pdf)
    } else if gio::content_type_is_a(&kind, "text/markdown") {
        Some(Kind::Markdown)
    } else if gio::content_type_is_a(&kind, "text/plain") {
        Some(Kind::Text)
    } else {
        None
    }
}

thread_local! {
    static READER: RefCell<Option<Rc<Reader>>> = const { RefCell::new(None) };
}

fn main() -> glib::ExitCode {
    orbit_app::run_app(orbit_app::App {
        name: "orbit-read",
        app_id: "io.github.julas23.Orbit.Read",
        css: include_str!("../../../data/read.css"),
        build,
        resident: false,
        open: Some(open),
    })
}

fn build(app: &gtk::Application, _dir: &ConfigDir) -> anyhow::Result<gtk::ApplicationWindow> {
    let reader = Reader::new(app);
    let window = reader.window().clone();
    READER.with(|r| *r.borrow_mut() = Some(reader));
    Ok(window)
}

/// Opens the first of `files` (a launch with files, or a second launch).
fn open(_window: &gtk::ApplicationWindow, files: &[gio::File]) {
    let Some(path) = files.first().and_then(|f| f.path()) else {
        return;
    };
    let reader = READER.with(|r| r.borrow().clone());
    if let Some(reader) = reader {
        reader.open(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_name() {
        assert!(kind_of(Path::new("a.PDF")) == Some(Kind::Pdf));
        assert!(kind_of(Path::new("README.md")) == Some(Kind::Markdown));
        assert!(kind_of(Path::new("notes.txt")) == Some(Kind::Text));
        assert!(kind_of(Path::new("photo.jpg")).is_none());
    }
}
