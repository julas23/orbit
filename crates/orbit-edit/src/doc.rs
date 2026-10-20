//! A document: a tab's buffer and view, the minimap beside it, the layer
//! with the extra cursors and indent guides over it, and its file.
//!
//! Files are read as UTF-8 or, failing that, Latin-1, and their line
//! endings (LF or CRLF) noted; saving writes them back the same way, into a
//! file beside the original that then takes its place. The indentation
//! (tabs, or spaces and how many) is read from the file.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::tr;
use sourceview5::prelude::*;

use crate::{Config, fold::Folds, multi::Cursors};

pub struct Doc {
    /// Names the backup of its unsaved text, across sessions.
    pub id: u64,
    pub widget: gtk::Box,
    pub view: sourceview5::View,
    pub buffer: sourceview5::Buffer,
    /// Held here: its handlers hold it only weakly.
    _cursors: Rc<Cursors>,
    pub folds: Rc<Folds>,
    pub search: sourceview5::SearchContext,
    path: RefCell<Option<PathBuf>>,
    latin1: Cell<bool>,
    crlf: Cell<bool>,
    /// The tab's label.
    pub tab: gtk::Box,
    label: gtk::Label,
    pub close: gtk::Button,
}

impl Doc {
    pub fn new(id: u64, config: &Config) -> Rc<Self> {
        let buffer = sourceview5::Buffer::new(None);
        buffer.set_highlight_matching_brackets(true);
        let view = sourceview5::View::builder()
            .buffer(&buffer)
            .monospace(true)
            .show_line_numbers(true)
            .highlight_current_line(true)
            .auto_indent(true)
            .indent_on_tab(true)
            .smart_backspace(true)
            .smart_home_end(sourceview5::SmartHomeEndType::Before)
            .tab_width(config.tab_width)
            .insert_spaces_instead_of_tabs(config.insert_spaces)
            .wrap_mode(if config.wrap { gtk::WrapMode::WordChar } else { gtk::WrapMode::None })
            .top_margin(5)
            .bottom_margin(200)
            .left_margin(5)
            .css_classes(["code"])
            .build();
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&view)
            .hexpand(true)
            .vexpand(true)
            .build();
        let cursors = Cursors::new(&view);
        let folds = Folds::new(&view);
        let overlay = gtk::Overlay::builder().child(&scrolled).build();
        overlay.add_overlay(&cursors.layer);
        let map = sourceview5::Map::builder().view(&view).visible(config.minimap).build();
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        widget.append(&overlay);
        widget.append(&map);

        let settings = sourceview5::SearchSettings::builder().wrap_around(true).build();
        let search = sourceview5::SearchContext::new(&buffer, Some(&settings));
        search.set_highlight(true);

        let label = gtk::Label::builder()
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .max_width_chars(25)
            .build();
        let close = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .css_classes(["flat", "tab-close"])
            .focus_on_click(false)
            .tooltip_text(tr("edit-close-tab"))
            .build();
        let tab = gtk::Box::builder().spacing(5).build();
        tab.append(&label);
        tab.append(&close);

        let doc = Rc::new(Self {
            id,
            widget,
            view,
            buffer,
            _cursors: cursors,
            folds,
            search,
            path: RefCell::default(),
            latin1: Cell::new(false),
            crlf: Cell::new(false),
            tab,
            label,
            close,
        });
        doc.update_label();
        let weak = Rc::downgrade(&doc);
        doc.buffer.connect_modified_changed(move |_| {
            if let Some(d) = weak.upgrade() {
                d.update_label();
            }
        });
        doc
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.path.borrow().clone()
    }

    pub fn title(&self) -> String {
        self.path()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| tr("edit-untitled"))
    }

    pub fn is_dirty(&self) -> bool {
        self.buffer.is_modified()
    }

    fn update_label(&self) {
        let title = self.title();
        let shown = if self.is_dirty() { format!("● {title}") } else { title };
        // An ellipsizing label asks only for "…": it asks for its text.
        self.label.set_width_chars(shown.chars().count().min(25) as i32);
        self.label.set_label(&shown);
        self.tab
            .set_tooltip_text(self.path().map(|p| p.display().to_string()).as_deref());
    }

    pub fn text(&self) -> String {
        let (start, end) = self.buffer.bounds();
        self.buffer.text(&start, &end, true).to_string()
    }

    /// Loads `path`. With `unsaved`, that text instead, still to be saved.
    pub fn load(&self, path: Option<&Path>, unsaved: Option<String>) -> std::io::Result<()> {
        let text = match (&unsaved, path) {
            (Some(text), _) => text.clone(),
            (None, Some(path)) => {
                let bytes = std::fs::read(path)?;
                let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
                match std::str::from_utf8(bytes) {
                    Ok(text) => text.to_owned(),
                    Err(_) => {
                        self.latin1.set(true);
                        bytes.iter().map(|&b| b as char).collect()
                    }
                }
            }
            (None, None) => String::new(),
        };
        self.crlf.set(text.contains("\r\n"));
        let text = if self.crlf.get() { text.replace("\r\n", "\n") } else { text };
        self.buffer.begin_irreversible_action();
        self.buffer.set_text(&text);
        self.buffer.end_irreversible_action();
        self.buffer.place_cursor(&self.buffer.start_iter());
        *self.path.borrow_mut() = path.map(Path::to_path_buf);
        self.detect_indentation(&text);
        self.guess_language();
        self.buffer.set_modified(unsaved.is_some());
        self.update_label();
        Ok(())
    }

    /// Writes the text to `path` (the document's own file by default).
    pub fn save(&self, path: Option<&Path>) -> std::io::Result<()> {
        let path = match path.map(Path::to_path_buf).or_else(|| self.path()) {
            Some(p) => p,
            None => return Err(std::io::Error::other("no file")),
        };
        let mut text = self.text();
        if self.crlf.get() {
            text = text.replace('\n', "\r\n");
        }
        let bytes: Vec<u8> = if self.latin1.get() && text.chars().all(|c| (c as u32) < 256) {
            text.chars().map(|c| c as u8).collect()
        } else {
            text.into_bytes()
        };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let temporary = path.with_file_name(format!(".{name}.orbit-edit"));
        std::fs::write(&temporary, &bytes)?;
        if let Ok(meta) = std::fs::metadata(&path) {
            let _ = std::fs::set_permissions(&temporary, meta.permissions());
        }
        if let Err(err) = std::fs::rename(&temporary, &path) {
            let _ = std::fs::remove_file(&temporary);
            return Err(err);
        }
        let renamed = self.path().as_deref() != Some(path.as_path());
        *self.path.borrow_mut() = Some(path);
        if renamed {
            self.guess_language();
        }
        self.buffer.set_modified(false);
        self.update_label();
        Ok(())
    }

    fn guess_language(&self) {
        let manager = sourceview5::LanguageManager::default();
        let path = self.path();
        let name = path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
        let content = path.as_deref().map(|p| gio::content_type_guess(Some(p), None).0);
        let language = manager.guess_language(name.as_deref(), content.as_deref());
        self.buffer.set_language(language.as_ref());
    }

    pub fn set_language(&self, id: Option<&str>) {
        let language = id.and_then(|id| sourceview5::LanguageManager::default().language(id));
        self.buffer.set_language(language.as_ref());
    }

    /// Tabs, or spaces and how many, as the file is indented.
    fn detect_indentation(&self, text: &str) {
        let (mut tabs, mut spaces) = (0, 0);
        let mut widths = [0usize; 9];
        let mut previous = 0usize;
        for line in text.lines().take(1000) {
            if line.trim().is_empty() {
                continue;
            }
            if line.starts_with('\t') {
                tabs += 1;
                continue;
            }
            let indent = line.len() - line.trim_start_matches(' ').len();
            if indent > 0 {
                spaces += 1;
            }
            let step = indent.abs_diff(previous);
            if (2..=8).contains(&step) {
                widths[step] += 1;
            }
            previous = indent;
        }
        if tabs + spaces < 3 {
            return;
        }
        if tabs > spaces {
            self.view.set_insert_spaces_instead_of_tabs(false);
        } else {
            self.view.set_insert_spaces_instead_of_tabs(true);
            let width = [2, 4, 8].into_iter().max_by_key(|w| widths[*w]).unwrap_or(4);
            if widths[width] > 0 {
                self.view.set_indent_width(width as i32);
                self.view.set_tab_width(width as u32);
            }
        }
    }

    /// "UTF-8 · LF", "Latin-1 · CRLF".
    pub fn encoding_label(&self) -> String {
        format!(
            "{} · {}",
            if self.latin1.get() { "Latin-1" } else { "UTF-8" },
            if self.crlf.get() { "CRLF" } else { "LF" }
        )
    }

    pub fn set_dark(&self, dark: bool) {
        let manager = sourceview5::StyleSchemeManager::default();
        let scheme = manager
            .scheme(if dark { "Adwaita-dark" } else { "Adwaita" })
            .or_else(|| manager.scheme(if dark { "oblivion" } else { "classic" }));
        self.buffer.set_style_scheme(scheme.as_ref());
    }

    pub fn cursor_offset(&self) -> i32 {
        self.buffer.iter_at_mark(&self.buffer.get_insert()).offset()
    }

    pub fn place_cursor(&self, offset: i32) {
        let iter = self.buffer.iter_at_offset(offset);
        self.buffer.place_cursor(&iter);
        let view = self.view.clone();
        let mark = self.buffer.get_insert();
        // After the first layout, or it scrolls nowhere.
        glib::idle_add_local_once(move || {
            view.scroll_to_mark(&mark, 0.1, true, 0.0, 0.3);
        });
    }

    /// Ctrl+/: comments the selected lines out with the language's line
    /// comment, or back in when they all have it.
    pub fn toggle_comment(&self) {
        let Some(comment) = self
            .buffer
            .language()
            .and_then(|l| l.metadata("line-comment-start"))
        else {
            return;
        };
        let comment = comment.to_string();
        let (start, end) = self.buffer.selection_bounds().unwrap_or_else(|| {
            let at = self.buffer.iter_at_mark(&self.buffer.get_insert());
            (at, at)
        });
        let (first, mut last) = (start.line(), end.line());
        // A selection ending at a line's start leaves that line alone.
        if end.line_offset() == 0 && last > first {
            last -= 1;
        }
        let lines: Vec<i32> = (first..=last)
            .filter(|&l| crate::multi::indentation(self.buffer.upcast_ref(), l, 1).is_some())
            .collect();
        let starts_with = |line: i32| {
            let mut at = self.buffer.iter_at_line(line).expect("a line");
            while at.char().is_whitespace() && !at.ends_line() {
                at.forward_char();
            }
            let mut stop = at;
            stop.forward_chars(comment.chars().count() as i32);
            (self.buffer.text(&at, &stop, false) == comment, at, stop)
        };
        let uncomment = !lines.is_empty() && lines.iter().all(|&l| starts_with(l).0);
        self.buffer.begin_user_action();
        for &line in &lines {
            let (_, mut at, mut stop) = starts_with(line);
            if uncomment {
                // The space after the comment mark goes too.
                if stop.char() == ' ' {
                    stop.forward_char();
                }
                self.buffer.delete(&mut at, &mut stop);
            } else {
                self.buffer.insert(&mut at, &format!("{comment} "));
            }
        }
        self.buffer.end_user_action();
    }
}
