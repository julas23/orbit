//! A document: a tab's buffer and view, the minimap beside it, the layer
//! with the extra cursors and indent guides over it, and its file.
//!
//! Files are read as UTF-8 or, failing that, Latin-1, and their line
//! endings (LF or CRLF) noted; saving writes them back the same way, into a
//! file beside the original that then takes its place. The indentation
//! (tabs, or spaces and how many) is read from the file, unless an
//! `.editorconfig` says (see `editorconfig`).
//!
//! The file is watched: changed by another program, it is read again —
//! or, with changes of its own here, a bar over the text asks which to
//! keep. A file the user may not write is saved as root (pkexec).
//!
//! Besides: the view split in two over the same text (Ctrl+\), the
//! Markdown preview beside it (`preview`), the git marks in the margin
//! (`gitmarks`), words of the open files completed as one types, and the
//! line commands (duplicate, delete, move, join, sort, case).

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, SystemTime},
};

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::tr;
use sourceview5::prelude::*;

use crate::{Config, fold::Folds, gitmarks::GitMarks, multi::Cursors, preview::Preview};

thread_local! {
    /// The words of every open document, for completion.
    static WORDS: sourceview5::CompletionWords = {
        let words = sourceview5::CompletionWords::new(Some(&tr("edit-complete-words")));
        words.set_minimum_word_size(3);
        words
    };
}

/// What is done to the text on saving (`trim_trailing`, `final_newline`
/// in edit.yaml, or the folder's .editorconfig).
#[derive(Clone, Copy, Default)]
pub struct SaveRules {
    pub trim_trailing: bool,
    pub final_newline: bool,
}

/// When the file was last seen as written here: its time and size.
type Stamp = (SystemTime, u64);

type Hook = RefCell<Option<Box<dyn Fn()>>>;

pub struct Doc {
    /// Names the backup of its unsaved text, across sessions.
    pub id: u64,
    pub widget: gtk::Box,
    pub view: sourceview5::View,
    pub buffer: sourceview5::Buffer,
    /// Held here: its handlers hold it only weakly.
    _cursors: Rc<Cursors>,
    pub folds: Rc<Folds>,
    pub git: Rc<GitMarks>,
    pub search: sourceview5::SearchContext,
    path: RefCell<Option<PathBuf>>,
    latin1: Cell<bool>,
    crlf: Cell<bool>,
    pub rules: Cell<SaveRules>,
    /// The tab's label.
    pub tab: gtk::Box,
    label: gtk::Label,
    pub close: gtk::Button,

    /// The two views over the text, when split.
    split: gtk::Paned,
    second: RefCell<Option<sourceview5::View>>,
    preview: RefCell<Option<Rc<Preview>>>,
    body: gtk::Box,
    map: sourceview5::Map,

    /// The bar over the text: the file changed on disk.
    banner: gtk::Revealer,
    banner_text: gtk::Label,
    banner_reload: gtk::Button,
    stamp: Cell<Option<Stamp>>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    check_soon: RefCell<Option<glib::SourceId>>,
    /// Called after the file is read again from disk or saved.
    pub on_reloaded: Hook,
    /// Spelling: misspelt words underlined (in code, only comments and
    /// strings), suggestions in the right-click menu.
    spell: libspelling::TextBufferAdapter,
}

impl Doc {
    pub fn new(id: u64, config: &Config) -> Rc<Self> {
        let buffer = sourceview5::Buffer::new(None);
        buffer.set_highlight_matching_brackets(true);
        let view = code_view(&buffer, config);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&view)
            .hexpand(true)
            .vexpand(true)
            .build();
        let cursors = Cursors::new(&view);
        let folds = Folds::new(&view);
        let git = GitMarks::new(&view);
        let overlay = gtk::Overlay::builder().child(&scrolled).build();
        overlay.add_overlay(&cursors.layer);
        let map = sourceview5::Map::builder().view(&view).visible(config.minimap).build();
        let split = gtk::Paned::builder()
            .start_child(&overlay)
            .shrink_start_child(false)
            .hexpand(true)
            .build();
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.append(&split);
        body.append(&map);

        let banner_text = gtk::Label::builder().xalign(0.0).hexpand(true).wrap(true).build();
        let banner_reload = gtk::Button::builder().label(tr("edit-disk-reload")).build();
        let banner_keep = gtk::Button::builder().label(tr("edit-disk-keep")).build();
        let banner_row = gtk::Box::builder().spacing(10).css_classes(["disk-banner"]).build();
        banner_row.append(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
        banner_row.append(&banner_text);
        banner_row.append(&banner_reload);
        banner_row.append(&banner_keep);
        let banner = gtk::Revealer::builder().child(&banner_row).build();

        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.append(&banner);
        widget.append(&body);

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

        let checker = libspelling::Checker::default();
        let spell = libspelling::TextBufferAdapter::new(&buffer, &checker);
        spell.set_enabled(config.spell);

        let doc = Rc::new(Self {
            id,
            widget,
            view,
            buffer,
            _cursors: cursors,
            folds,
            git,
            search,
            path: RefCell::default(),
            latin1: Cell::new(false),
            crlf: Cell::new(false),
            rules: Cell::new(SaveRules {
                trim_trailing: config.trim_trailing,
                final_newline: config.final_newline,
            }),
            tab,
            label,
            close,
            split,
            second: RefCell::default(),
            preview: RefCell::default(),
            body,
            map,
            banner,
            banner_text,
            banner_reload,
            stamp: Cell::new(None),
            monitor: RefCell::default(),
            check_soon: RefCell::default(),
            on_reloaded: RefCell::default(),
            spell,
        });
        doc.spell_menu(&doc.view);
        doc.update_label();
        doc.set_whitespace(config.show_whitespace);
        let weak = Rc::downgrade(&doc);
        doc.buffer.connect_modified_changed(move |_| {
            if let Some(d) = weak.upgrade() {
                d.update_label();
            }
        });
        WORDS.with(|w| {
            w.register(&doc.buffer);
            doc.view.completion().add_provider(w);
        });
        doc.view
            .completion()
            .add_provider(&sourceview5::CompletionSnippets::new());
        let weak = Rc::downgrade(&doc);
        doc.banner_reload.connect_clicked(move |_| {
            if let Some(d) = weak.upgrade() {
                d.reload();
            }
        });
        let weak = Rc::downgrade(&doc);
        banner_keep.connect_clicked(move |_| {
            if let Some(d) = weak.upgrade() {
                // What is here wins: saving then writes it without asking.
                d.stamp.set(d.path().and_then(|p| stamp(&p)));
                d.banner.set_reveal_child(false);
                d.buffer.set_modified(true);
            }
        });
        let weak = Rc::downgrade(&doc);
        doc.buffer.connect_changed(move |_| {
            if let Some(d) = weak.upgrade()
                && let Some(p) = d.preview.borrow().as_ref()
            {
                p.update_soon(&d.text(), d.path().as_deref());
            }
        });
        doc
    }

    /// The tab closes: the words of its text are no longer offered.
    pub fn dispose(&self) {
        WORDS.with(|w| w.unregister(&self.buffer));
        if let Some(m) = self.monitor.borrow_mut().take() {
            m.cancel();
        }
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
    pub fn load(self: &Rc<Self>, path: Option<&Path>, unsaved: Option<String>) -> std::io::Result<()> {
        let text = match (&unsaved, path) {
            (Some(text), _) => text.clone(),
            (None, Some(path)) => self.read(path)?,
            (None, None) => String::new(),
        };
        let text = self.note_line_endings(text);
        self.buffer.begin_irreversible_action();
        self.buffer.set_text(&text);
        self.buffer.end_irreversible_action();
        self.buffer.place_cursor(&self.buffer.start_iter());
        *self.path.borrow_mut() = path.map(Path::to_path_buf);
        self.detect_indentation(&text);
        self.guess_language();
        self.buffer.set_modified(unsaved.is_some());
        self.update_label();
        self.watch();
        self.git.refresh(self.path().as_deref());
        Ok(())
    }

    fn read(&self, path: &Path) -> std::io::Result<String> {
        let bytes = std::fs::read(path)?;
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
        Ok(match std::str::from_utf8(bytes) {
            Ok(text) => {
                self.latin1.set(false);
                text.to_owned()
            }
            Err(_) => {
                self.latin1.set(true);
                bytes.iter().map(|&b| b as char).collect()
            }
        })
    }

    fn note_line_endings(&self, text: String) -> String {
        self.crlf.set(text.contains("\r\n"));
        if self.crlf.get() {
            text.replace("\r\n", "\n")
        } else {
            text
        }
    }

    /// Reads the file again, the cursor kept on its line.
    pub fn reload(self: &Rc<Self>) {
        self.banner.set_reveal_child(false);
        let Some(path) = self.path() else { return };
        let Ok(text) = self.read(&path) else { return };
        let text = self.note_line_endings(text);
        let at = self.buffer.iter_at_mark(&self.buffer.get_insert());
        let (line, column) = (at.line(), at.line_offset());
        // Undoable, as any edit: what was here can come back with Ctrl+Z.
        self.buffer.begin_user_action();
        let (mut start, mut end) = self.buffer.bounds();
        self.buffer.delete(&mut start, &mut end);
        self.buffer.insert(&mut start, &text);
        self.buffer.end_user_action();
        if let Some(mut iter) = self.buffer.iter_at_line(line) {
            let len = iter.chars_in_line();
            iter.set_line_offset(column.min((len - 1).max(0)));
            self.buffer.place_cursor(&iter);
        }
        self.buffer.set_modified(false);
        self.stamp.set(stamp(&path));
        self.git.refresh(Some(&path));
        if let Some(f) = self.on_reloaded.borrow().as_ref() {
            f();
        }
    }

    /// Whether the file was changed on disk since it was read or saved
    /// here (and that was not accepted with "Keep mine").
    pub fn changed_on_disk(&self) -> bool {
        match (self.path(), self.stamp.get()) {
            (Some(p), Some(seen)) => stamp(&p).is_some_and(|now| now != seen),
            _ => false,
        }
    }

    /// Watches the file for changes made by other programs.
    fn watch(self: &Rc<Self>) {
        if let Some(m) = self.monitor.borrow_mut().take() {
            m.cancel();
        }
        let Some(path) = self.path() else {
            self.stamp.set(None);
            return;
        };
        self.stamp.set(stamp(&path));
        let Ok(monitor) =
            gio::File::for_path(&path).monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        else {
            return;
        };
        let weak = Rc::downgrade(self);
        monitor.connect_changed(move |_, _, _, _| {
            // Programs write in several steps: looked at once they settle.
            let Some(d) = weak.upgrade() else { return };
            if let Some(id) = d.check_soon.borrow_mut().take() {
                id.remove();
            }
            let weak = Rc::downgrade(&d);
            let id = glib::timeout_add_local_once(Duration::from_millis(300), move || {
                if let Some(d) = weak.upgrade() {
                    d.check_soon.borrow_mut().take();
                    d.check_disk();
                }
            });
            *d.check_soon.borrow_mut() = Some(id);
        });
        *self.monitor.borrow_mut() = Some(monitor);
    }

    fn check_disk(self: &Rc<Self>) {
        let Some(path) = self.path() else { return };
        let now = stamp(&path);
        if now == self.stamp.get() {
            return;
        }
        match now {
            None => {
                self.banner_text.set_label(&tr("edit-disk-deleted"));
                self.banner_reload.set_visible(false);
                self.banner.set_reveal_child(true);
            }
            Some(_) if !self.is_dirty() => self.reload(),
            Some(_) => {
                self.banner_text.set_label(&tr("edit-disk-changed"));
                self.banner_reload.set_visible(true);
                self.banner.set_reveal_child(true);
            }
        }
    }

    /// The bytes the file gets: the line endings and encoding it had,
    /// after the save rules.
    fn bytes(&self) -> Vec<u8> {
        self.apply_rules();
        let mut text = self.text();
        if self.crlf.get() {
            text = text.replace('\n', "\r\n");
        }
        if self.latin1.get() && text.chars().all(|c| (c as u32) < 256) {
            text.chars().map(|c| c as u8).collect()
        } else {
            text.into_bytes()
        }
    }

    /// Spaces at the line ends taken off, a newline at the end: as one
    /// edit, undone with Ctrl+Z.
    fn apply_rules(&self) {
        let rules = self.rules.get();
        if !rules.trim_trailing && !rules.final_newline {
            return;
        }
        self.buffer.begin_user_action();
        if rules.trim_trailing {
            for line in 0..self.buffer.line_count() {
                let Some(mut end) = self.buffer.iter_at_line(line) else {
                    continue;
                };
                if !end.ends_line() {
                    end.forward_to_line_end();
                }
                let mut start = end;
                while start.line_offset() > 0 {
                    let mut before = start;
                    before.backward_char();
                    if before.char() == ' ' || before.char() == '\t' {
                        start = before;
                    } else {
                        break;
                    }
                }
                if start.offset() < end.offset() {
                    self.buffer.delete(&mut start, &mut end);
                }
            }
        }
        if rules.final_newline {
            let mut end = self.buffer.end_iter();
            let mut before = end;
            if before.backward_char() && before.char() != '\n' {
                self.buffer.insert(&mut end, "\n");
            }
        }
        self.buffer.end_user_action();
    }

    /// Writes the text to `path` (the document's own file by default).
    pub fn save(self: &Rc<Self>, path: Option<&Path>) -> std::io::Result<()> {
        let path = match path.map(Path::to_path_buf).or_else(|| self.path()) {
            Some(p) => p,
            None => return Err(std::io::Error::other("no file")),
        };
        let bytes = self.bytes();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let temporary = path.with_file_name(format!(".{name}.orbit-edit"));
        std::fs::write(&temporary, &bytes)?;
        if let Ok(meta) = std::fs::metadata(&path) {
            let _ = std::fs::set_permissions(&temporary, meta.permissions());
        }
        if let Err(err) = std::fs::rename(&temporary, &path) {
            let _ = std::fs::remove_file(&temporary);
            return Err(err);
        }
        self.saved(path);
        Ok(())
    }

    /// Writes the text to `path` as root: into a file of the user's,
    /// copied over with pkexec (polkit asks the password). The file keeps
    /// its owner and permissions.
    pub fn save_as_root(self: &Rc<Self>, path: &Path, done: impl FnOnce(Result<(), String>) + 'static) {
        let bytes = self.bytes();
        let temporary = glib::user_runtime_dir().join(format!("orbit-edit-{}-{}", std::process::id(), self.id));
        if let Err(err) = std::fs::write(&temporary, &bytes) {
            done(Err(err.to_string()));
            return;
        }
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o644));
        }
        let args = [
            std::ffi::OsStr::new("pkexec"),
            std::ffi::OsStr::new("cp"),
            std::ffi::OsStr::new("--"),
            temporary.as_os_str(),
            path.as_os_str(),
        ];
        let process = match gio::Subprocess::newv(&args, gio::SubprocessFlags::STDERR_PIPE) {
            Ok(p) => p,
            Err(err) => {
                let _ = std::fs::remove_file(&temporary);
                done(Err(err.to_string()));
                return;
            }
        };
        let weak = Rc::downgrade(self);
        let path = path.to_path_buf();
        process.communicate_utf8_async(None, gio::Cancellable::NONE, move |result| {
            let _ = std::fs::remove_file(&temporary);
            let Some(d) = weak.upgrade() else { return };
            match result {
                Ok((_, err)) if d.exit_ok(&err) => {
                    d.saved(path);
                    done(Ok(()));
                }
                Ok((_, err)) => done(Err(err.map(|e| e.trim().to_string()).unwrap_or_default())),
                Err(err) => done(Err(err.to_string())),
            }
        });
    }

    /// pkexec said nothing on its error output: the copy was made (a
    /// cancelled password says "Not authorized" or "dismissed").
    fn exit_ok(&self, err: &Option<glib::GString>) -> bool {
        err.as_ref().is_none_or(|e| e.trim().is_empty())
    }

    fn saved(self: &Rc<Self>, path: PathBuf) {
        let renamed = self.path().as_deref() != Some(path.as_path());
        *self.path.borrow_mut() = Some(path.clone());
        if renamed {
            self.guess_language();
            self.watch();
        }
        self.stamp.set(stamp(&path));
        self.banner.set_reveal_child(false);
        self.buffer.set_modified(false);
        self.update_label();
        self.git.refresh(Some(&path));
        if let Some(f) = self.on_reloaded.borrow().as_ref() {
            f();
        }
    }

    /// The file was moved or renamed (in the sidebar): the tab follows.
    pub fn save_as_moved(self: &Rc<Self>, path: &Path) -> std::io::Result<()> {
        *self.path.borrow_mut() = Some(path.to_path_buf());
        self.guess_language();
        self.watch();
        self.update_label();
        self.git.refresh(Some(path));
        Ok(())
    }

    /// The spaces at the ends of the lines taken off now.
    pub fn trim_now(&self) {
        let rules = self.rules.get();
        self.rules.set(SaveRules {
            trim_trailing: true,
            final_newline: false,
        });
        self.apply_rules();
        self.rules.set(rules);
    }

    /// Replaces the whole text (a formatter's output), as one edit, the
    /// cursor kept on its line.
    pub fn replace_text(&self, text: &str) {
        if self.text() == text {
            return;
        }
        let at = self.buffer.iter_at_mark(&self.buffer.get_insert());
        let (line, column) = (at.line(), at.line_offset());
        self.buffer.begin_user_action();
        let (mut start, mut end) = self.buffer.bounds();
        self.buffer.delete(&mut start, &mut end);
        self.buffer.insert(&mut start, text);
        self.buffer.end_user_action();
        if let Some(mut iter) = self.buffer.iter_at_line(line) {
            iter.set_line_offset(column.min((iter.chars_in_line() - 1).max(0)));
            self.buffer.place_cursor(&iter);
        }
    }

    fn guess_language(&self) {
        let manager = sourceview5::LanguageManager::default();
        let path = self.path();
        let name = path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        let content = path.as_deref().map(|p| gio::content_type_guess(Some(p), None).0);
        let language = manager.guess_language(name.as_deref(), content.as_deref());
        self.buffer.set_language(language.as_ref());
    }

    pub fn set_language(&self, id: Option<&str>) {
        let language = id.and_then(|id| sourceview5::LanguageManager::default().language(id));
        self.buffer.set_language(language.as_ref());
    }

    pub fn language_id(&self) -> Option<String> {
        self.buffer.language().map(|l| l.id().to_string())
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
                self.set_indent(true, width as u32);
            }
        }
    }

    pub fn set_indent(&self, spaces: bool, width: u32) {
        for view in self.views() {
            view.set_insert_spaces_instead_of_tabs(spaces);
            view.set_indent_width(width as i32);
            view.set_tab_width(width);
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

    pub fn set_crlf(&self, crlf: bool) {
        self.crlf.set(crlf);
    }

    pub fn set_dark(&self, dark: bool) {
        let manager = sourceview5::StyleSchemeManager::default();
        let scheme = manager
            .scheme(if dark { "Adwaita-dark" } else { "Adwaita" })
            .or_else(|| manager.scheme(if dark { "oblivion" } else { "classic" }));
        self.buffer.set_style_scheme(scheme.as_ref());
        if let Some(p) = self.preview.borrow().as_ref() {
            p.update_soon(&self.text(), self.path().as_deref());
        }
    }

    pub fn cursor_offset(&self) -> i32 {
        self.buffer.iter_at_mark(&self.buffer.get_insert()).offset()
    }

    pub fn place_cursor(&self, offset: i32) {
        let iter = self.buffer.iter_at_offset(offset);
        self.buffer.place_cursor(&iter);
        self.scroll_to_cursor();
    }

    /// Puts the cursor on `line` (from 0), at `column` characters.
    pub fn go_to(&self, line: i32, column: i32) {
        let Some(mut iter) = self.buffer.iter_at_line(line.max(0)) else {
            self.buffer.place_cursor(&self.buffer.end_iter());
            self.scroll_to_cursor();
            return;
        };
        iter.set_line_offset(column.clamp(0, (iter.chars_in_line() - 1).max(0)));
        self.buffer.place_cursor(&iter);
        self.scroll_to_cursor();
    }

    fn scroll_to_cursor(&self) {
        let view = self.view.clone();
        let mark = self.buffer.get_insert();
        // After the first layout, or it scrolls nowhere.
        glib::idle_add_local_once(move || {
            view.scroll_to_mark(&mark, 0.1, true, 0.0, 0.3);
        });
    }

    // ------------------------------------------------------------ views

    /// The views over the text: one, or two when split.
    pub fn views(&self) -> Vec<sourceview5::View> {
        let mut views = vec![self.view.clone()];
        views.extend(self.second.borrow().clone());
        views
    }

    /// Ctrl+\: the text in two views side by side, or back to one.
    pub fn toggle_split(&self, config: &Config) {
        if self.second.borrow_mut().take().is_some() {
            self.split.set_end_child(None::<&gtk::Widget>);
            self.view.grab_focus();
            return;
        }
        let view = code_view(&self.buffer, config);
        view.set_insert_spaces_instead_of_tabs(self.view.is_insert_spaces_instead_of_tabs());
        view.set_tab_width(self.view.tab_width());
        view.set_indent_width(self.view.indent_width());
        view.set_wrap_mode(self.view.wrap_mode());
        let types = self
            .view
            .space_drawer()
            .types_for_locations(sourceview5::SpaceLocationFlags::ALL);
        view.space_drawer()
            .set_types_for_locations(sourceview5::SpaceLocationFlags::ALL, types);
        view.space_drawer()
            .set_enable_matrix(self.view.space_drawer().enables_matrix());
        WORDS.with(|w| view.completion().add_provider(w));
        self.spell_menu(&view);
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&view)
            .hexpand(true)
            .vexpand(true)
            .build();
        self.split.set_end_child(Some(&scrolled));
        let width = self.split.width();
        self.split.set_position(if width > 0 { width / 2 } else { 400 });
        let mark = self.buffer.get_insert();
        let v = view.clone();
        glib::idle_add_local_once(move || {
            v.scroll_to_mark(&mark, 0.1, true, 0.0, 0.3);
            v.grab_focus();
        });
        *self.second.borrow_mut() = Some(view);
    }

    /// The Markdown laid out beside the text, following it.
    pub fn toggle_preview(&self) {
        if let Some(p) = self.preview.borrow_mut().take() {
            self.body.remove(&p.widget);
            self.map.set_visible(p.map_was);
            return;
        }
        let preview = Preview::new(self.map.is_visible());
        self.map.set_visible(false);
        self.body.append(&preview.widget);
        preview.update_soon(&self.text(), self.path().as_deref());
        *self.preview.borrow_mut() = Some(preview);
    }

    pub fn set_wrap(&self, wrap: bool) {
        for view in self.views() {
            view.set_wrap_mode(if wrap {
                gtk::WrapMode::WordChar
            } else {
                gtk::WrapMode::None
            });
        }
    }

    pub fn wraps(&self) -> bool {
        self.view.wrap_mode() != gtk::WrapMode::None
    }

    pub fn set_minimap(&self, shown: bool) {
        self.map.set_visible(shown);
    }

    pub fn minimap(&self) -> bool {
        self.map.is_visible()
    }

    /// Spaces and tabs drawn as dots and arrows.
    pub fn set_whitespace(&self, shown: bool) {
        for view in self.views() {
            let drawer = view.space_drawer();
            drawer.set_types_for_locations(
                sourceview5::SpaceLocationFlags::ALL,
                if shown {
                    sourceview5::SpaceTypeFlags::SPACE | sourceview5::SpaceTypeFlags::TAB
                } else {
                    sourceview5::SpaceTypeFlags::NONE
                },
            );
            drawer.set_enable_matrix(shown);
        }
    }

    pub fn whitespace(&self) -> bool {
        self.view.space_drawer().enables_matrix()
    }

    fn spell_menu(&self, view: &sourceview5::View) {
        view.set_extra_menu(Some(&self.spell.menu_model()));
        view.insert_action_group("spelling", Some(&self.spell));
    }

    pub fn set_spell(&self, on: bool) {
        self.spell.set_enabled(on);
    }

    pub fn spells(&self) -> bool {
        self.spell.is_enabled()
    }

    // ------------------------------------------------------------ lines

    /// The lines the selection (or the cursor) is on: first and last.
    fn selected_lines(&self) -> (i32, i32) {
        let (start, end) = self.buffer.selection_bounds().unwrap_or_else(|| {
            let at = self.buffer.iter_at_mark(&self.buffer.get_insert());
            (at, at)
        });
        let (first, mut last) = (start.line(), end.line());
        // A selection ending at a line's start leaves that line alone.
        if end.line_offset() == 0 && last > first {
            last -= 1;
        }
        (first, last)
    }

    /// Where `line` starts, or the end of the text past the last line.
    fn line_start(&self, line: i32) -> gtk::TextIter {
        if line >= self.buffer.line_count() {
            return self.buffer.end_iter();
        }
        self.buffer.iter_at_line(line).unwrap_or_else(|| self.buffer.end_iter())
    }

    /// Ctrl+Shift+D: the lines copied under themselves.
    pub fn duplicate_lines(&self) {
        let (first, last) = self.selected_lines();
        let start = self.line_start(first);
        let end = self.line_start(last + 1);
        let mut text = self.buffer.text(&start, &end, true).to_string();
        let at_end = last + 1 >= self.buffer.line_count();
        self.buffer.begin_user_action();
        let mut at = self.line_start(last + 1);
        if at_end {
            // The last line has no newline to carry.
            text.insert(0, '\n');
            if text.ends_with('\n') {
                text.pop();
            }
        }
        self.buffer.insert(&mut at, &text);
        self.buffer.end_user_action();
    }

    /// Ctrl+Shift+K: the lines taken out.
    pub fn delete_lines(&self) {
        let (first, last) = self.selected_lines();
        let mut start = self.line_start(first);
        let mut end = self.line_start(last + 1);
        if last + 1 >= self.buffer.line_count() && first > 0 {
            // The last lines: the newline before them goes instead.
            start.backward_char();
            end = self.buffer.end_iter();
        }
        self.buffer.begin_user_action();
        self.buffer.delete(&mut start, &mut end);
        self.buffer.end_user_action();
    }

    /// Ctrl+Shift+↑ and ↓: the lines moved up or down.
    pub fn move_lines(&self, down: bool) {
        self.view.emit_move_lines(down);
    }

    /// Ctrl+J: the lines joined into one.
    pub fn join_lines(&self) {
        self.view.emit_join_lines();
    }

    pub fn sort_lines(&self, reverse: bool) {
        let (first, last) = self.selected_lines();
        let mut start = self.line_start(first);
        let mut end = self.line_start(last);
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        let flags = if reverse {
            sourceview5::SortFlags::REVERSE_ORDER
        } else {
            sourceview5::SortFlags::NONE
        };
        self.buffer.sort_lines(&mut start, &mut end, flags, 0);
    }

    pub fn change_case(&self, case: sourceview5::ChangeCaseType) {
        self.view.emit_change_case(case);
    }

    /// Ctrl+/: comments the selected lines out with the language's line
    /// comment, or back in when they all have it.
    pub fn toggle_comment(&self) {
        let Some(comment) = self.buffer.language().and_then(|l| l.metadata("line-comment-start")) else {
            return;
        };
        let comment = comment.to_string();
        let (first, last) = self.selected_lines();
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

    // ---------------------------------------------------------- printing

    pub fn print(&self, parent: &gtk::Window) {
        let compositor = sourceview5::PrintCompositor::new(&self.buffer);
        compositor.set_print_line_numbers(1);
        compositor.set_wrap_mode(gtk::WrapMode::WordChar);
        compositor.set_body_font_name("Monospace 9");
        compositor.set_header_format(true, Some(&self.title()), None, Some("%N / %Q"));
        compositor.set_print_header(true);
        let operation = gtk::PrintOperation::new();
        operation.set_job_name(&self.title());
        let c = compositor.clone();
        operation.connect_begin_print(move |op, context| {
            while !c.paginate(context) {}
            op.set_n_pages(c.n_pages());
        });
        operation.connect_draw_page(move |_, context, page| {
            compositor.draw_page(context, page);
        });
        if let Err(err) = operation.run(gtk::PrintOperationAction::PrintDialog, Some(parent)) {
            log::warn!("printing: {err}");
        }
    }
}

/// A view over `buffer` as the editor's are.
fn code_view(buffer: &sourceview5::Buffer, config: &Config) -> sourceview5::View {
    sourceview5::View::builder()
        .buffer(buffer)
        .monospace(true)
        .show_line_numbers(true)
        .show_line_marks(true)
        .highlight_current_line(true)
        .auto_indent(true)
        .indent_on_tab(true)
        .smart_backspace(true)
        .smart_home_end(sourceview5::SmartHomeEndType::Before)
        .tab_width(config.tab_width)
        .insert_spaces_instead_of_tabs(config.insert_spaces)
        .wrap_mode(if config.wrap {
            gtk::WrapMode::WordChar
        } else {
            gtk::WrapMode::None
        })
        .top_margin(5)
        .bottom_margin(200)
        .left_margin(5)
        .css_classes(["code"])
        .build()
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}
