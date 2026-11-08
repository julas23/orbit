//! The window: the bar along the top, the folder on the left, the tabs,
//! the find bar over them, the terminal under them, and the status line
//! at the bottom — where the cursor is, the language server's errors and
//! warnings, the indentation, the language, the encoding and the line
//! endings.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use gtk::{gdk, gio, glib, prelude::*};
use orbit_app::i18n::{number, text, tr, tr_args};
use sourceview5::prelude::*;

use crate::{
    Config,
    doc::{Doc, SaveRules},
    editorconfig,
    findbar::FindBar,
    lsp::Lsp,
    palette::{self, Command},
    search,
    session::{self, Session, Tab},
    sidebar::{self, Sidebar},
    terminal::Terminal,
};

/// Recent files kept, and closed tabs that Ctrl+Shift+T can bring back.
const MOST_RECENT: usize = 30;
const MOST_CLOSED: usize = 20;

pub struct Editor {
    window: gtk::ApplicationWindow,
    config: Config,
    notebook: gtk::Notebook,
    sidebar: Rc<Sidebar>,
    findbar: Rc<FindBar>,
    terminal: Rc<Terminal>,
    lsp: Rc<Lsp>,
    docs: RefCell<Vec<Rc<Doc>>>,
    next_id: Cell<u64>,
    position: gtk::MenuButton,
    goto: gtk::Entry,
    diagnostics: gtk::Button,
    indent: gtk::MenuButton,
    spaces: gtk::CheckButton,
    width: gtk::SpinButton,
    language: gtk::DropDown,
    languages: Vec<Option<String>>,
    encoding: gtk::Label,
    sidebar_toggle: gtk::ToggleButton,
    css: gtk::CssProvider,
    font_size: Cell<f64>,
    save_soon: RefCell<Option<glib::SourceId>>,
    autosave_soon: RefCell<Option<glib::SourceId>>,
    recent: RefCell<Vec<PathBuf>>,
    /// Tabs closed, last first: the file and where the cursor was.
    closed: RefCell<Vec<(PathBuf, i32)>>,
    /// Set while the status line is filled, so its widgets' signals do
    /// not change the document back.
    updating: Cell<bool>,
}

impl Editor {
    pub fn new(app: &gtk::Application, config: Config) -> Rc<Self> {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title(tr("edit-title"))
            .default_width(1100)
            .default_height(750)
            .css_classes(["orbit-edit"])
            .build();

        // ---- the bar along the top
        let button = |icon: &str, tip: &str, key: &str| {
            gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(shortcut(tip, key))
                .focus_on_click(false)
                .build()
        };
        let sidebar_toggle = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text(shortcut("edit-sidebar", "Ctrl+B"))
            .focus_on_click(false)
            .build();
        let new = button("document-new-symbolic", "edit-new", "Ctrl+N");
        let open = button("document-open-symbolic", "edit-open", "Ctrl+O");
        let open_folder = button("folder-open-symbolic", "edit-open-folder", "Ctrl+Shift+O");
        let save = button("document-save-symbolic", "edit-save", "Ctrl+S");
        let find = button("edit-find-symbolic", "edit-find", "Ctrl+F");
        let find_files = button("system-search-symbolic", "edit-search-files", "Ctrl+Shift+F");
        let quick = button("go-jump-symbolic", "edit-quick-open", "Ctrl+P");
        let commands = button("view-more-symbolic", "edit-commands", "Ctrl+Shift+P");
        let row = gtk::Box::builder().spacing(5).css_classes(["bar"]).build();
        for w in [
            sidebar_toggle.upcast_ref::<gtk::Widget>(),
            new.upcast_ref(),
            open.upcast_ref(),
            open_folder.upcast_ref(),
            save.upcast_ref(),
        ] {
            row.append(w);
        }
        row.append(&gtk::Box::builder().hexpand(true).build());
        row.append(&quick);
        row.append(&find_files);
        row.append(&find);
        row.append(&commands);
        row.append(
            &gtk::WindowControls::builder()
                .side(gtk::PackType::End)
                .decoration_layout(":minimize,maximize,close")
                .build(),
        );
        window.set_titlebar(Some(&gtk::WindowHandle::builder().child(&row).build()));

        // ---- the tabs, the find bar, the status line
        let notebook = gtk::Notebook::builder()
            .scrollable(true)
            .show_border(false)
            .vexpand(true)
            .build();
        let findbar = FindBar::new();
        let terminal = Terminal::new();

        let position = gtk::MenuButton::builder()
            .tooltip_text(shortcut("edit-goto", "Ctrl+G"))
            .css_classes(["flat", "status-item"])
            .build();
        let goto = gtk::Entry::builder()
            .placeholder_text(tr("edit-goto-placeholder"))
            .input_purpose(gtk::InputPurpose::Digits)
            .width_chars(12)
            .build();
        position.set_popover(Some(&gtk::Popover::builder().child(&goto).build()));
        let diagnostics = gtk::Button::builder()
            .css_classes(["flat", "status-item", "diagnostics"])
            .visible(false)
            .tooltip_text(shortcut("edit-next-problem", "F8"))
            .build();

        let spaces = gtk::CheckButton::builder().label(tr("edit-indent-spaces")).build();
        let width = gtk::SpinButton::with_range(1.0, 16.0, 1.0);
        let indent_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["indent-menu"])
            .build();
        indent_box.append(&spaces);
        let width_row = gtk::Box::builder().spacing(10).build();
        width_row.append(&gtk::Label::new(Some(&tr("edit-indent-width"))));
        width_row.append(&width);
        indent_box.append(&width_row);
        let indent = gtk::MenuButton::builder()
            .css_classes(["flat", "status-item"])
            .popover(&gtk::Popover::builder().child(&indent_box).build())
            .build();

        // Every language GtkSourceView knows, by name; "Plain text" first.
        let manager = sourceview5::LanguageManager::default();
        let mut pairs: Vec<(String, String)> = manager
            .language_ids()
            .iter()
            .filter_map(|id| manager.language(id).map(|l| (l.name().to_string(), id.to_string())))
            .collect();
        pairs.sort_by_key(|a| a.0.to_lowercase());
        let mut names = vec![tr("edit-plain-text")];
        let mut languages: Vec<Option<String>> = vec![None];
        for (name, id) in pairs {
            names.push(name);
            languages.push(Some(id));
        }
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let language = gtk::DropDown::builder()
            .model(&gtk::StringList::new(&name_refs))
            .enable_search(true)
            .css_classes(["status-item"])
            .build();
        language.set_expression(Some(gtk::PropertyExpression::new(
            gtk::StringObject::static_type(),
            None::<&gtk::Expression>,
            "string",
        )));
        let encoding = gtk::Label::builder().css_classes(["status-text"]).build();
        let status = gtk::Box::builder().spacing(5).css_classes(["status"]).build();
        status.append(&diagnostics);
        status.append(&gtk::Box::builder().hexpand(true).build());
        for w in [
            position.upcast_ref::<gtk::Widget>(),
            indent.upcast_ref(),
            encoding.upcast_ref(),
            language.upcast_ref(),
        ] {
            status.append(w);
        }

        let main = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        main.append(&findbar.widget);
        main.append(&notebook);
        main.append(&terminal.widget);
        main.append(&status);
        let sidebar = Sidebar::new();
        let paned = gtk::Paned::builder()
            .start_child(&sidebar.widget)
            .end_child(&main)
            .shrink_start_child(false)
            .resize_start_child(false)
            .position(230)
            .build();
        window.set_child(Some(&paned));

        let css = gtk::CssProvider::new();
        gtk::style_context_add_provider_for_display(
            &gtk::prelude::WidgetExt::display(&window),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
        );

        // The language servers open files and find open ones through the
        // editor, which is made after them.
        let me: Rc<RefCell<std::rc::Weak<Editor>>> = Rc::default();
        let (m1, m2) = (me.clone(), me.clone());
        let lsp = Lsp::new(
            config.lsp.clone(),
            move |path, line, column| {
                if let Some(e) = m1.borrow().upgrade() {
                    e.open_at(path, line, column);
                }
            },
            move |path| m2.borrow().upgrade().and_then(|e| e.doc_of(path)),
        );

        let editor = Rc::new(Self {
            window,
            font_size: Cell::new(config.font_size),
            config,
            notebook,
            sidebar,
            findbar,
            terminal,
            lsp,
            docs: RefCell::default(),
            next_id: Cell::new(1),
            position,
            goto,
            diagnostics,
            indent,
            spaces,
            width,
            language,
            languages,
            encoding,
            sidebar_toggle,
            css,
            save_soon: RefCell::default(),
            autosave_soon: RefCell::default(),
            recent: RefCell::default(),
            closed: RefCell::default(),
            updating: Cell::new(false),
        });
        *me.borrow_mut() = Rc::downgrade(&editor);
        editor.apply_font();

        let act = |button: &gtk::Button, f: fn(&Rc<Editor>)| {
            let weak = Rc::downgrade(&editor);
            button.connect_clicked(move |_| {
                if let Some(e) = weak.upgrade() {
                    f(&e);
                }
            });
        };
        act(&new, |e| {
            e.new_tab();
        });
        act(&open, |e| e.choose());
        act(&open_folder, |e| e.choose_folder());
        act(&save, |e| e.save_current(false));
        act(&find, |e| e.find(false));
        act(&find_files, |e| e.search_files());
        act(&quick, |e| e.quick_open());
        act(&commands, |e| e.palette());
        act(&editor.diagnostics, |e| e.next_problem());
        editor.connect();
        editor.restore();
        editor
    }

    pub fn window(&self) -> &gtk::ApplicationWindow {
        &self.window
    }

    fn connect(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.sidebar_toggle.connect_toggled(move |b| {
            if let Some(e) = weak.upgrade() {
                e.sidebar.widget.set_visible(b.is_active());
            }
        });
        let weak = Rc::downgrade(self);
        self.sidebar.connect_open(move |path| {
            if let Some(e) = weak.upgrade() {
                e.open(path);
            }
        });
        let weak = Rc::downgrade(self);
        self.sidebar.connect_renamed(move |from, to| {
            let Some(e) = weak.upgrade() else { return };
            let docs: Vec<Rc<Doc>> = e.docs.borrow().clone();
            for doc in docs {
                if let Some(path) = doc.path()
                    && let Ok(rest) = path.strip_prefix(from)
                {
                    let moved = if rest.as_os_str().is_empty() {
                        to.to_path_buf()
                    } else {
                        to.join(rest)
                    };
                    let _ = doc.save_as_moved(&moved);
                    e.lsp.attach(&doc);
                }
            }
            if let Some(doc) = e.current() {
                e.show(&doc);
            }
        });
        let weak = Rc::downgrade(self);
        self.sidebar.connect_terminal(move |dir| {
            if let Some(e) = weak.upgrade() {
                if e.terminal.shown() {
                    e.terminal.toggle(dir, &e.config.font, e.font_size.get());
                }
                e.terminal.toggle(dir, &e.config.font, e.font_size.get());
            }
        });
        let weak = Rc::downgrade(self);
        self.notebook.connect_switch_page(move |_, page, _| {
            let Some(e) = weak.upgrade() else { return };
            let doc = e
                .docs
                .borrow()
                .iter()
                .find(|d| d.widget.upcast_ref::<gtk::Widget>() == page)
                .cloned();
            if let Some(doc) = doc {
                e.findbar.set_doc(Some(&doc));
                e.status(&doc);
                e.window.set_title(Some(&format!("{} — Orbit-Edit", doc.title())));
                let view = doc.view.clone();
                glib::idle_add_local_once(move || {
                    view.grab_focus();
                });
            }
        });
        let weak = Rc::downgrade(self);
        *self.lsp.on_diagnostics.borrow_mut() = Some(Rc::new(move |id| {
            if let Some(e) = weak.upgrade()
                && let Some(doc) = e.current().filter(|d| d.id == id)
            {
                e.status_diagnostics(&doc);
            }
        }));

        // The status line changes the document shown.
        let weak = Rc::downgrade(self);
        self.language.connect_selected_notify(move |dd| {
            let Some(e) = weak.upgrade() else { return };
            if e.updating.get() {
                return;
            }
            if let Some(doc) = e.current() {
                let id = e.languages.get(dd.selected() as usize).cloned().flatten();
                doc.set_language(id.as_deref());
            }
        });
        let weak = Rc::downgrade(self);
        self.spaces.connect_toggled(move |b| {
            if let Some(e) = weak.upgrade()
                && !e.updating.get()
                && let Some(doc) = e.current()
            {
                doc.set_indent(b.is_active(), doc.view.tab_width());
                e.status(&doc);
            }
        });
        let weak = Rc::downgrade(self);
        self.width.connect_value_changed(move |s| {
            if let Some(e) = weak.upgrade()
                && !e.updating.get()
                && let Some(doc) = e.current()
            {
                doc.set_indent(doc.view.is_insert_spaces_instead_of_tabs(), s.value() as u32);
                e.status(&doc);
            }
        });
        let weak = Rc::downgrade(self);
        self.goto.connect_activate(move |entry| {
            let Some(e) = weak.upgrade() else { return };
            if let (Some(doc), Ok(line)) = (e.current(), entry.text().trim().parse::<i32>()) {
                doc.go_to(line - 1, 0);
            }
            entry.set_text("");
            e.position.popdown();
            if let Some(doc) = e.current() {
                doc.view.grab_focus();
            }
        });

        // Light or dark, as the system turns.
        if let Some(settings) = gtk::Settings::default() {
            let weak = Rc::downgrade(self);
            settings.connect_gtk_application_prefer_dark_theme_notify(move |s| {
                if let Some(e) = weak.upgrade() {
                    let dark = s.is_gtk_application_prefer_dark_theme();
                    for doc in e.docs.borrow().iter() {
                        doc.set_dark(dark);
                    }
                }
            });
        }

        // Keys.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(e) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if e.key(key, state) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.window.add_controller(keys);

        // A file dropped on the window opens.
        let target = gtk::DropTarget::new(gio::File::static_type(), gdk::DragAction::COPY);
        let weak = Rc::downgrade(self);
        target.connect_drop(move |_, value, _, _| {
            let (Some(e), Ok(file)) = (weak.upgrade(), value.get::<gio::File>()) else {
                return false;
            };
            match file.path() {
                Some(p) if p.is_dir() => e.set_root(Some(&p)),
                Some(p) => e.open(&p),
                None => return false,
            }
            true
        });
        self.window.add_controller(target);

        // Saving by itself: also when the window loses the focus.
        let weak = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |w| {
            if let Some(e) = weak.upgrade()
                && e.config.autosave
                && !w.is_active()
            {
                e.autosave();
            }
        });

        // Closing keeps everything, unsaved text included, for next time.
        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(e) = weak.upgrade() {
                if e.config.autosave {
                    e.autosave();
                }
                e.save_session();
            }
            glib::Propagation::Proceed
        });
    }

    /// A key of the editor's; whether it was one.
    fn key(self: &Rc<Self>, key: gdk::Key, state: gdk::ModifierType) -> bool {
        use gdk::Key;
        let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = state.contains(gdk::ModifierType::ALT_MASK);
        let doc = self.current();
        // Keys of their own, with no Ctrl.
        if !ctrl && !alt {
            match key {
                Key::F12 => {
                    if let Some(d) = &doc {
                        self.lsp.definition(d);
                    }
                }
                Key::F2 => self.rename_symbol(),
                Key::F8 => self.next_problem(),
                _ => return false,
            }
            return true;
        }
        if !ctrl || alt {
            return false;
        }
        let with_doc = |f: &dyn Fn(&Rc<Doc>)| {
            if let Some(d) = &doc {
                f(d);
            }
        };
        match key {
            Key::n | Key::N if !shift => {
                self.new_tab();
            }
            Key::o | Key::O if shift => self.choose_folder(),
            Key::o | Key::O => self.choose(),
            Key::s | Key::S => self.save_current(shift),
            Key::w | Key::W => self.close_current(),
            Key::q | Key::Q => self.window.close(),
            Key::f | Key::F if shift => self.search_files(),
            Key::f | Key::F => self.find(false),
            Key::h | Key::H => self.find(true),
            Key::g | Key::G => {
                self.position.popup();
                self.goto.grab_focus();
            }
            Key::p | Key::P if shift => self.palette(),
            Key::p | Key::P => self.quick_open(),
            Key::r | Key::R if !shift => self.open_recent(),
            Key::t | Key::T if shift => self.reopen_closed(),
            Key::b | Key::B => self.sidebar_toggle.set_active(!self.sidebar_toggle.is_active()),
            Key::d | Key::D if shift => with_doc(&|d| d.duplicate_lines()),
            Key::k | Key::K if shift => with_doc(&|d| d.delete_lines()),
            Key::Up if shift => with_doc(&|d| d.move_lines(false)),
            Key::Down if shift => with_doc(&|d| d.move_lines(true)),
            Key::j | Key::J if !shift => with_doc(&|d| d.join_lines()),
            Key::i | Key::I if shift => self.format_current(),
            Key::m | Key::M if shift => with_doc(&|d| d.toggle_preview()),
            Key::backslash => self.toggle_split(),
            Key::grave | Key::dead_grave | Key::apostrophe => self.toggle_terminal(),
            Key::slash | Key::KP_Divide => with_doc(&|d| d.toggle_comment()),
            Key::bracketleft | Key::braceleft if shift => with_doc(&|d| d.folds.fold_here()),
            Key::bracketright | Key::braceright if shift => with_doc(&|d| d.folds.unfold_here()),
            Key::Tab | Key::Page_Down => self.cycle(1),
            Key::ISO_Left_Tab | Key::Page_Up => self.cycle(-1),
            Key::plus | Key::equal | Key::KP_Add => self.zoom(1.0),
            Key::minus | Key::KP_Subtract => self.zoom(-1.0),
            Key::_0 | Key::KP_0 => {
                self.font_size.set(self.config.font_size);
                self.apply_font();
            }
            _ => return false,
        }
        true
    }

    // -------------------------------------------------------------- tabs

    fn current(&self) -> Option<Rc<Doc>> {
        let page = self.notebook.nth_page(self.notebook.current_page())?;
        self.docs
            .borrow()
            .iter()
            .find(|d| d.widget.upcast_ref::<gtk::Widget>() == &page)
            .cloned()
    }

    /// The open document of `path`.
    fn doc_of(&self, path: &Path) -> Option<Rc<Doc>> {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.docs
            .borrow()
            .iter()
            .find(|d| d.path().as_deref() == Some(path.as_path()))
            .cloned()
    }

    fn new_tab(self: &Rc<Self>) -> Rc<Doc> {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let doc = Doc::new(id, &self.config);
        doc.set_dark(gtk::Settings::default().is_some_and(|s| s.is_gtk_application_prefer_dark_theme()));
        let page = self.notebook.append_page(&doc.widget, Some(&doc.tab));
        self.notebook.set_tab_reorderable(&doc.widget, true);
        self.docs.borrow_mut().push(doc.clone());

        let weak = Rc::downgrade(self);
        let target = Rc::downgrade(&doc);
        doc.close.connect_clicked(move |_| {
            if let (Some(e), Some(d)) = (weak.upgrade(), target.upgrade()) {
                e.close(&d);
            }
        });
        let weak = Rc::downgrade(self);
        let target = Rc::downgrade(&doc);
        doc.buffer.connect_mark_set(move |buffer, _, mark| {
            if mark == &buffer.get_insert()
                && let (Some(e), Some(d)) = (weak.upgrade(), target.upgrade())
                && e.current().is_some_and(|c| Rc::ptr_eq(&c, &d))
            {
                e.status_position(&d);
            }
        });
        let weak = Rc::downgrade(self);
        doc.buffer.connect_changed(move |_| {
            if let Some(e) = weak.upgrade() {
                e.save_session_soon();
                if e.config.autosave {
                    e.autosave_soon();
                }
            }
        });
        let weak = Rc::downgrade(self);
        let target = Rc::downgrade(&doc);
        doc.buffer.connect_language_notify(move |_| {
            let (Some(e), Some(d)) = (weak.upgrade(), target.upgrade()) else {
                return;
            };
            if d.path().is_some() {
                e.lsp.attach(&d);
            }
            if e.current().is_some_and(|c| Rc::ptr_eq(&c, &d)) {
                e.status(&d);
            }
        });
        self.notebook.set_current_page(Some(page));
        doc
    }

    /// Opens `path` — in its tab if open already, or in place of an empty
    /// untitled tab.
    pub fn open(self: &Rc<Self>, path: &Path) {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if path.is_dir() {
            self.set_root(Some(&path));
            return;
        }
        if let Some(doc) = self.doc_of(&path) {
            self.show(&doc);
            return;
        }
        let blank = self
            .current()
            .filter(|d| d.path().is_none() && !d.is_dirty() && d.text().is_empty());
        let doc = blank.unwrap_or_else(|| self.new_tab());
        if let Err(err) = doc.load(Some(&path), None) {
            self.error(&tr_args(
                "edit-open-failed",
                &[
                    ("name", text(path.display().to_string())),
                    ("reason", text(err.to_string())),
                ],
            ));
            return;
        }
        self.opened(&doc);
        if self.sidebar.root().is_none()
            && let Some(dir) = path.parent()
        {
            self.set_root(Some(dir));
        }
        self.show(&doc);
        self.save_session_soon();
    }

    /// Opens `path` with the cursor on `line`, `column` (from 0).
    fn open_at(self: &Rc<Self>, path: &Path, line: i32, column: i32) {
        self.open(path);
        if let Some(doc) = self.doc_of(path) {
            self.show(&doc);
            doc.go_to(line, column);
            doc.view.grab_focus();
        }
    }

    /// A file just read into `doc`: its .editorconfig, its server, the
    /// recent list.
    fn opened(self: &Rc<Self>, doc: &Rc<Doc>) {
        let Some(path) = doc.path() else { return };
        let ec = editorconfig::for_file(&path);
        if ec.spaces.is_some() || ec.width.is_some() {
            doc.set_indent(
                ec.spaces.unwrap_or_else(|| doc.view.is_insert_spaces_instead_of_tabs()),
                ec.width.unwrap_or_else(|| doc.view.tab_width()),
            );
        }
        if let Some(crlf) = ec.crlf
            && doc.text().is_empty()
        {
            doc.set_crlf(crlf);
        }
        doc.rules.set(SaveRules {
            trim_trailing: ec.trim_trailing.unwrap_or(self.config.trim_trailing),
            final_newline: ec.final_newline.unwrap_or(self.config.final_newline),
        });
        let weak = Rc::downgrade(self);
        let target = Rc::downgrade(doc);
        *doc.on_reloaded.borrow_mut() = Some(Box::new(move || {
            if let (Some(e), Some(d)) = (weak.upgrade(), target.upgrade()) {
                e.lsp.saved(&d);
                if e.current().is_some_and(|c| Rc::ptr_eq(&c, &d)) {
                    e.status(&d);
                }
            }
        }));
        self.lsp.attach(doc);
        let mut recent = self.recent.borrow_mut();
        recent.retain(|p| p != &path);
        recent.insert(0, path);
        recent.truncate(MOST_RECENT);
    }

    fn show(&self, doc: &Rc<Doc>) {
        if let Some(n) = self.notebook.page_num(&doc.widget) {
            self.notebook.set_current_page(Some(n));
        }
        self.status(doc);
        self.window.set_title(Some(&format!("{} — Orbit-Edit", doc.title())));
    }

    fn cycle(&self, by: i32) {
        let n = self.notebook.n_pages() as i32;
        if n > 0 {
            let next = (self.notebook.current_page().map_or(0, |p| p as i32) + by).rem_euclid(n);
            self.notebook.set_current_page(Some(next as u32));
        }
    }

    fn close_current(self: &Rc<Self>) {
        if let Some(doc) = self.current() {
            self.close(&doc);
        }
    }

    /// Closes `doc`'s tab, asking first if it has unsaved changes.
    fn close(self: &Rc<Self>, doc: &Rc<Doc>) {
        if !doc.is_dirty() {
            self.remove(doc);
            return;
        }
        let dialog = gtk::AlertDialog::builder()
            .modal(true)
            .message(tr_args("edit-close-question", &[("name", text(doc.title()))]))
            .detail(tr("edit-close-detail"))
            .buttons([tr("edit-cancel"), tr("edit-discard"), tr("edit-save")])
            .cancel_button(0)
            .default_button(2)
            .build();
        let weak = Rc::downgrade(self);
        let doc = doc.clone();
        dialog.choose(Some(&self.window), gio::Cancellable::NONE, move |answer| {
            let Some(e) = weak.upgrade() else { return };
            match answer {
                Ok(1) => e.remove(&doc),
                Ok(2) => {
                    let e2 = e.clone();
                    let d2 = doc.clone();
                    e.save(&doc, false, move || e2.remove(&d2));
                }
                _ => {}
            }
        });
    }

    fn remove(self: &Rc<Self>, doc: &Rc<Doc>) {
        if let Some(path) = doc.path() {
            let mut closed = self.closed.borrow_mut();
            closed.retain(|(p, _)| p != &path);
            closed.insert(0, (path, doc.cursor_offset()));
            closed.truncate(MOST_CLOSED);
        }
        self.lsp.detach(doc);
        doc.dispose();
        if let Some(n) = self.notebook.page_num(&doc.widget) {
            self.notebook.remove_page(Some(n));
        }
        self.docs.borrow_mut().retain(|d| !Rc::ptr_eq(d, doc));
        if self.docs.borrow().is_empty() {
            self.new_tab();
        }
        self.save_session_soon();
    }

    /// Ctrl+Shift+T: the last tab closed, open again where it was.
    fn reopen_closed(self: &Rc<Self>) {
        loop {
            let Some((path, cursor)) = ({
                let mut closed = self.closed.borrow_mut();
                (!closed.is_empty()).then(|| closed.remove(0))
            }) else {
                return;
            };
            if path.is_file() {
                self.open(&path);
                if let Some(doc) = self.doc_of(&path) {
                    doc.place_cursor(cursor);
                }
                return;
            }
        }
    }

    // ---------------------------------------------------- files, folders

    fn choose(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder().title(tr("edit-open")).modal(true).build();
        if let Some(dir) = self
            .current()
            .and_then(|d| d.path())
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or_else(|| self.sidebar.root())
        {
            dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
        }
        let weak = Rc::downgrade(self);
        dialog.open_multiple(Some(&self.window), gio::Cancellable::NONE, move |result| {
            let (Some(e), Ok(files)) = (weak.upgrade(), result) else {
                return;
            };
            for file in files.iter::<gio::File>().filter_map(Result::ok) {
                if let Some(path) = file.path() {
                    e.open(&path);
                }
            }
        });
    }

    fn choose_folder(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder()
            .title(tr("edit-open-folder"))
            .modal(true)
            .build();
        let weak = Rc::downgrade(self);
        dialog.select_folder(Some(&self.window), gio::Cancellable::NONE, move |result| {
            if let (Some(e), Ok(folder)) = (weak.upgrade(), result)
                && let Some(path) = folder.path()
            {
                e.set_root(Some(&path));
            }
        });
    }

    fn set_root(self: &Rc<Self>, root: Option<&Path>) {
        self.sidebar.set_root(root);
        if root.is_some() {
            self.sidebar_toggle.set_active(true);
        }
        self.save_session_soon();
    }

    /// The folder searches and the terminal start in: the sidebar's, or
    /// the file's.
    fn folder(&self) -> Option<PathBuf> {
        self.sidebar.root().or_else(|| {
            self.current()
                .and_then(|d| d.path())
                .and_then(|p| p.parent().map(Path::to_path_buf))
        })
    }

    fn quick_open(self: &Rc<Self>) {
        let Some(root) = self.folder() else {
            self.choose_folder();
            return;
        };
        let weak = Rc::downgrade(self);
        sidebar::quick_open(self.window.upcast_ref(), &root, move |path| {
            if let Some(e) = weak.upgrade() {
                e.open(path);
            }
        });
    }

    fn search_files(self: &Rc<Self>) {
        let Some(root) = self.folder() else {
            self.choose_folder();
            return;
        };
        let selected = self.current().and_then(|d| {
            d.buffer
                .selection_bounds()
                .map(|(s, e)| d.buffer.text(&s, &e, false).to_string())
        });
        let weak = Rc::downgrade(self);
        search::open(self.window.upcast_ref(), &root, selected, move |path, line, column| {
            if let Some(e) = weak.upgrade() {
                e.open_at(path, line, column);
            }
        });
    }

    /// Ctrl+R: a recent file.
    fn open_recent(self: &Rc<Self>) {
        let home = glib::home_dir();
        let commands = self
            .recent
            .borrow()
            .iter()
            .filter(|p| p.is_file())
            .map(|p| {
                let shown = match p.strip_prefix(&home) {
                    Ok(rest) => format!("~/{}", rest.display()),
                    Err(_) => p.display().to_string(),
                };
                let weak = Rc::downgrade(self);
                let p = p.clone();
                Command::new(shown, "", move || {
                    if let Some(e) = weak.upgrade() {
                        e.open(&p);
                    }
                })
            })
            .collect();
        palette::open(self.window.upcast_ref(), &tr("edit-recent"), commands);
    }

    fn save_current(self: &Rc<Self>, choose: bool) {
        if let Some(doc) = self.current() {
            self.save(&doc, choose, || {});
        }
    }

    /// Saves `doc` — asking where if it has no file yet, or if `choose` —
    /// then runs `then`. A file changed on disk since asks first; one the
    /// user may not write offers to be saved as root.
    fn save(self: &Rc<Self>, doc: &Rc<Doc>, choose: bool, then: impl FnOnce() + 'static) {
        if !choose && doc.path().is_some() {
            if doc.changed_on_disk() {
                let dialog = gtk::AlertDialog::builder()
                    .modal(true)
                    .message(tr_args("edit-disk-overwrite", &[("name", text(doc.title()))]))
                    .detail(tr("edit-disk-overwrite-detail"))
                    .buttons([
                        tr("edit-cancel"),
                        tr("edit-disk-reload"),
                        tr("edit-disk-overwrite-button"),
                    ])
                    .cancel_button(0)
                    .default_button(0)
                    .build();
                let weak = Rc::downgrade(self);
                let doc = doc.clone();
                dialog.choose(Some(&self.window), gio::Cancellable::NONE, move |answer| {
                    let Some(e) = weak.upgrade() else { return };
                    match answer {
                        Ok(1) => doc.reload(),
                        Ok(2) => e.write(&doc, then),
                        _ => {}
                    }
                });
                return;
            }
            if self.config.format_on_save {
                let weak = Rc::downgrade(self);
                let target = doc.clone();
                self.format(doc, move || {
                    if let Some(e) = weak.upgrade() {
                        e.write(&target, then);
                    }
                });
            } else {
                self.write(doc, then);
            }
            return;
        }
        let dialog = gtk::FileDialog::builder()
            .title(tr("edit-save-as"))
            .modal(true)
            .initial_name(
                doc.path()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| tr("edit-untitled-file")),
            )
            .build();
        if let Some(dir) = doc
            .path()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or_else(|| self.sidebar.root())
        {
            dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
        }
        let weak = Rc::downgrade(self);
        let doc = doc.clone();
        dialog.save(Some(&self.window), gio::Cancellable::NONE, move |result| {
            let (Some(e), Ok(file)) = (weak.upgrade(), result) else {
                return;
            };
            let Some(path) = file.path() else { return };
            match doc.save(Some(&path)) {
                Ok(()) => {
                    e.opened(&doc);
                    e.show(&doc);
                    then();
                }
                Err(err) => e.error(&tr_args("edit-save-failed", &[("reason", text(err.to_string()))])),
            }
            e.save_session_soon();
        });
    }

    /// Writes `doc` to its file; as root, if the user agrees, when it may
    /// not be written otherwise.
    fn write(self: &Rc<Self>, doc: &Rc<Doc>, then: impl FnOnce() + 'static) {
        match doc.save(None) {
            Ok(()) => then(),
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                let dialog = gtk::AlertDialog::builder()
                    .modal(true)
                    .message(tr_args("edit-save-root-question", &[("name", text(doc.title()))]))
                    .detail(tr("edit-save-root-detail"))
                    .buttons([tr("edit-cancel"), tr("edit-save-root")])
                    .cancel_button(0)
                    .default_button(1)
                    .build();
                let weak = Rc::downgrade(self);
                let doc = doc.clone();
                dialog.choose(Some(&self.window), gio::Cancellable::NONE, move |answer| {
                    let (Some(e), Ok(1), Some(path)) = (weak.upgrade(), answer, doc.path()) else {
                        return;
                    };
                    let weak = Rc::downgrade(&e);
                    doc.save_as_root(&path, move |result| {
                        let Some(e) = weak.upgrade() else { return };
                        match result {
                            Ok(()) => then(),
                            Err(reason) => e.error(&tr_args("edit-save-failed", &[("reason", text(reason))])),
                        }
                    });
                });
            }
            Err(err) => self.error(&tr_args("edit-save-failed", &[("reason", text(err.to_string()))])),
        }
        self.save_session_soon();
    }

    /// Formats `doc`: with the program edit.yaml names for its language,
    /// or else with its language server.
    fn format(&self, doc: &Rc<Doc>, done: impl FnOnce() + 'static) {
        let program = doc
            .language_id()
            .and_then(|l| self.config.formatters.get(&l).cloned())
            .filter(|p| !p.is_empty());
        let Some(program) = program else {
            self.lsp.format(doc, done);
            return;
        };
        let cwd = doc.path().and_then(|p| p.parent().map(Path::to_path_buf));
        let result = run_formatter(&program, &doc.text(), cwd.as_deref());
        match result {
            Ok(text) => doc.replace_text(&text),
            Err(err) => log::warn!("{}: {err}", program[0]),
        }
        done();
    }

    fn format_current(self: &Rc<Self>) {
        if let Some(doc) = self.current() {
            self.format(&doc, || {});
        }
    }

    /// Saves the documents with a file and changes, quietly.
    fn autosave(self: &Rc<Self>) {
        let docs: Vec<Rc<Doc>> = self.docs.borrow().clone();
        for doc in docs {
            if doc.is_dirty()
                && doc.path().is_some()
                && !doc.changed_on_disk()
                && let Err(err) = doc.save(None)
            {
                log::warn!("autosave {}: {err}", doc.title());
            }
        }
    }

    fn autosave_soon(self: &Rc<Self>) {
        if let Some(id) = self.autosave_soon.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(1500), move || {
            if let Some(e) = weak.upgrade() {
                e.autosave_soon.borrow_mut().take();
                e.autosave();
            }
        });
        *self.autosave_soon.borrow_mut() = Some(id);
    }

    fn error(&self, message: &str) {
        gtk::AlertDialog::builder()
            .modal(true)
            .message(message)
            .build()
            .show(Some(&self.window));
    }

    fn find(&self, replace: bool) {
        if let Some(doc) = self.current() {
            self.findbar.show(&doc, replace);
        }
    }

    // ------------------------------------------------------- the views

    fn toggle_split(&self) {
        if let Some(doc) = self.current() {
            doc.toggle_split(&self.config);
            self.lsp.views_changed(&doc);
        }
    }

    fn toggle_terminal(&self) {
        let dir = self.folder().unwrap_or_else(glib::home_dir);
        self.terminal.toggle(&dir, &self.config.font, self.font_size.get());
        if !self.terminal.shown()
            && let Some(doc) = self.current()
        {
            doc.view.grab_focus();
        }
    }

    // ------------------------------------------------- language servers

    fn next_problem(&self) {
        if let Some(doc) = self.current() {
            self.lsp.next_diagnostic(&doc);
            doc.view.grab_focus();
        }
    }

    /// F2: a new name for what is under the cursor, everywhere.
    fn rename_symbol(self: &Rc<Self>) {
        let Some(doc) = self.current().filter(|d| self.lsp.can_rename(d)) else {
            return;
        };
        let at = doc.buffer.iter_at_mark(&doc.buffer.get_insert());
        let (mut start, mut end) = (at, at);
        if !start.starts_word() {
            start.backward_word_start();
        }
        if !end.ends_word() {
            end.forward_word_end();
        }
        let word = doc.buffer.text(&start, &end, false).to_string();
        let weak = Rc::downgrade(self);
        ask_text(
            self.window.upcast_ref(),
            &tr("edit-rename-symbol"),
            &word.clone(),
            move |name| {
                let Some(e) = weak.upgrade() else { return };
                if name.is_empty() || name == word {
                    return;
                }
                let weak = Rc::downgrade(&e);
                e.lsp.rename(&doc, &name, move |result| {
                    if let (Some(e), Err(reason)) = (weak.upgrade(), result) {
                        e.error(&tr_args("edit-rename-failed", &[("reason", text(reason))]));
                    }
                });
            },
        );
    }

    // ---------------------------------------------------------- palette

    /// Ctrl+Shift+P: every command.
    fn palette(self: &Rc<Self>) {
        type Act = fn(&Rc<Editor>);
        type DocAct = fn(&Rc<Editor>, &Rc<Doc>);
        let commands: Vec<(&str, &'static str, Act)> = vec![
            ("edit-new", "Ctrl+N", |e| {
                e.new_tab();
            }),
            ("edit-open", "Ctrl+O", |e| e.choose()),
            ("edit-open-folder", "Ctrl+Shift+O", |e| e.choose_folder()),
            ("edit-recent", "Ctrl+R", |e| e.open_recent()),
            ("edit-reopen-closed", "Ctrl+Shift+T", |e| e.reopen_closed()),
            ("edit-save", "Ctrl+S", |e| e.save_current(false)),
            ("edit-save-as", "Ctrl+Shift+S", |e| e.save_current(true)),
            ("edit-save-all", "", |e| e.save_all()),
            ("edit-close-tab", "Ctrl+W", |e| e.close_current()),
            ("edit-quick-open", "Ctrl+P", |e| e.quick_open()),
            ("edit-search-files", "Ctrl+Shift+F", |e| e.search_files()),
            ("edit-find", "Ctrl+F", |e| e.find(false)),
            ("edit-replace", "Ctrl+H", |e| e.find(true)),
            ("edit-goto", "Ctrl+G", |e| {
                e.position.popup();
                e.goto.grab_focus();
            }),
            ("edit-sidebar", "Ctrl+B", |e| {
                e.sidebar_toggle.set_active(!e.sidebar_toggle.is_active())
            }),
            ("edit-split", "Ctrl+\\", |e| e.toggle_split()),
            ("edit-terminal", "Ctrl+'", |e| e.toggle_terminal()),
            ("edit-format", "Ctrl+Shift+I", |e| e.format_current()),
            ("edit-definition", "F12", |e| {
                if let Some(d) = e.current() {
                    e.lsp.definition(&d);
                }
            }),
            ("edit-rename-symbol", "F2", |e| e.rename_symbol()),
            ("edit-next-problem", "F8", |e| e.next_problem()),
            ("edit-zoom-in", "Ctrl++", |e| e.zoom(1.0)),
            ("edit-zoom-out", "Ctrl+−", |e| e.zoom(-1.0)),
        ];
        let doc_commands: Vec<(&str, &'static str, DocAct)> = vec![
            ("edit-duplicate-lines", "Ctrl+Shift+D", |_, d| d.duplicate_lines()),
            ("edit-delete-lines", "Ctrl+Shift+K", |_, d| d.delete_lines()),
            ("edit-move-up", "Ctrl+Shift+↑", |_, d| d.move_lines(false)),
            ("edit-move-down", "Ctrl+Shift+↓", |_, d| d.move_lines(true)),
            ("edit-join-lines", "Ctrl+J", |_, d| d.join_lines()),
            ("edit-sort-lines", "", |_, d| d.sort_lines(false)),
            ("edit-sort-lines-reverse", "", |_, d| d.sort_lines(true)),
            ("edit-upper", "", |_, d| {
                d.change_case(sourceview5::ChangeCaseType::Upper)
            }),
            ("edit-lower", "", |_, d| {
                d.change_case(sourceview5::ChangeCaseType::Lower)
            }),
            ("edit-title-case", "", |_, d| {
                d.change_case(sourceview5::ChangeCaseType::Title)
            }),
            ("edit-toggle-comment", "Ctrl+/", |_, d| d.toggle_comment()),
            ("edit-preview", "Ctrl+Shift+M", |_, d| d.toggle_preview()),
            ("edit-toggle-wrap", "", |_, d| d.set_wrap(!d.wraps())),
            ("edit-toggle-minimap", "", |_, d| d.set_minimap(!d.minimap())),
            ("edit-toggle-whitespace", "", |_, d| d.set_whitespace(!d.whitespace())),
            ("edit-toggle-spell", "", |_, d| d.set_spell(!d.spells())),
            ("edit-trim-now", "", |_, d| d.trim_now()),
            ("edit-line-endings-lf", "", |e, d| {
                d.set_crlf(false);
                d.buffer.set_modified(true);
                e.status(d);
            }),
            ("edit-line-endings-crlf", "", |e, d| {
                d.set_crlf(true);
                d.buffer.set_modified(true);
                e.status(d);
            }),
            ("edit-reload", "", |_, d| d.reload()),
            ("edit-print", "", |e, d| d.print(e.window.upcast_ref())),
            ("edit-copy-path", "", |e, d| {
                if let Some(p) = d.path() {
                    e.window.clipboard().set_text(&p.to_string_lossy());
                }
            }),
        ];
        let mut list: Vec<Command> = commands
            .into_iter()
            .map(|(id, keys, f)| {
                let weak = Rc::downgrade(self);
                Command::new(tr(id), keys, move || {
                    if let Some(e) = weak.upgrade() {
                        f(&e);
                    }
                })
            })
            .collect();
        if let Some(doc) = self.current() {
            for (id, keys, f) in doc_commands {
                let weak = Rc::downgrade(self);
                let target = Rc::downgrade(&doc);
                list.push(Command::new(tr(id), keys, move || {
                    if let (Some(e), Some(d)) = (weak.upgrade(), target.upgrade()) {
                        f(&e, &d);
                    }
                }));
            }
        }
        palette::open(self.window.upcast_ref(), &tr("edit-commands"), list);
    }

    fn save_all(self: &Rc<Self>) {
        let docs: Vec<Rc<Doc>> = self.docs.borrow().clone();
        for doc in docs.into_iter().filter(|d| d.is_dirty() && d.path().is_some()) {
            self.save(&doc, false, || {});
        }
    }

    // ------------------------------------------------------- status line

    fn status(&self, doc: &Rc<Doc>) {
        self.updating.set(true);
        self.status_position(doc);
        let spaces = doc.view.is_insert_spaces_instead_of_tabs();
        let width = doc.view.tab_width();
        self.spaces.set_active(spaces);
        self.width.set_value(f64::from(width));
        self.indent.set_label(&if spaces {
            tr_args("edit-status-spaces", &[("n", number(f64::from(width)))])
        } else {
            tr_args("edit-status-tabs", &[("n", number(f64::from(width)))])
        });
        let id = doc.buffer.language().map(|l| l.id().to_string());
        let index = self.languages.iter().position(|l| *l == id).unwrap_or(0);
        self.language.set_selected(index as u32);
        self.encoding.set_label(&doc.encoding_label());
        self.status_diagnostics(doc);
        self.updating.set(false);
    }

    fn status_diagnostics(&self, doc: &Doc) {
        match self.lsp.counts(doc) {
            Some((errors, warnings)) => {
                self.diagnostics.set_label(&format!("✖ {errors}  ⚠ {warnings}"));
                self.diagnostics.set_visible(true);
                if let Some(name) = self.lsp.server_name(doc) {
                    self.diagnostics
                        .set_tooltip_text(Some(&format!("{name} · {}", shortcut("edit-next-problem", "F8"))));
                }
            }
            None => self.diagnostics.set_visible(false),
        }
    }

    fn status_position(&self, doc: &Doc) {
        let at = doc.buffer.iter_at_mark(&doc.buffer.get_insert());
        self.position.set_label(&tr_args(
            "edit-status-position",
            &[
                ("line", number(f64::from(at.line() + 1))),
                ("column", number(f64::from(at.line_offset() + 1))),
            ],
        ));
    }

    // -------------------------------------------------------------- font

    fn zoom(&self, by: f64) {
        self.font_size.set((self.font_size.get() + by).clamp(6.0, 40.0));
        self.apply_font();
    }

    fn apply_font(&self) {
        self.css.load_from_string(&format!(
            "window.orbit-edit textview.code {{ font-family: \"{}\", monospace; font-size: {}pt; }}",
            self.config.font.replace('"', ""),
            self.font_size.get()
        ));
    }

    // ----------------------------------------------------------- session

    fn restore(self: &Rc<Self>) {
        let saved = session::load();
        self.next_id.set(saved.next_id.max(1));
        *self.recent.borrow_mut() = saved.recent.clone();
        if let Some(root) = saved.root.as_ref().filter(|r| r.is_dir()) {
            self.set_root(Some(root));
        } else {
            self.sidebar_toggle.set_active(false);
            self.sidebar.widget.set_visible(false);
        }
        for tab in &saved.tabs {
            let unsaved = tab.unsaved.then(|| session::unsaved_text(tab.id)).flatten();
            // A file gone since, with nothing unsaved, is left behind.
            if unsaved.is_none() && !tab.path.as_ref().is_some_and(|p| p.is_file()) {
                continue;
            }
            let doc = self.new_tab_with_id(tab.id);
            if doc.load(tab.path.as_deref(), unsaved).is_ok() {
                doc.place_cursor(tab.cursor);
                self.opened(&doc);
            }
        }
        // Opening put them first; the saved order stands.
        *self.recent.borrow_mut() = saved.recent;
        if self.docs.borrow().is_empty() {
            self.new_tab();
        }
        let active = saved.active.min(self.notebook.n_pages().saturating_sub(1) as usize);
        self.notebook.set_current_page(Some(active as u32));
        if let Some(doc) = self.current() {
            self.status(&doc);
        }
    }

    fn new_tab_with_id(self: &Rc<Self>, id: u64) -> Rc<Doc> {
        let next = self.next_id.get();
        self.next_id.set(id);
        let doc = self.new_tab();
        self.next_id.set(next.max(id + 1));
        doc
    }

    fn save_session_soon(self: &Rc<Self>) {
        if let Some(id) = self.save_soon.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_secs(3), move || {
            if let Some(e) = weak.upgrade() {
                e.save_soon.borrow_mut().take();
                e.save_session();
            }
        });
        *self.save_soon.borrow_mut() = Some(id);
    }

    fn save_session(&self) {
        // The tabs in the notebook's order (tabs can be dragged).
        let docs = self.docs.borrow();
        let ordered: Vec<Rc<Doc>> = (0..self.notebook.n_pages())
            .filter_map(|n| self.notebook.nth_page(Some(n)))
            .filter_map(|page| {
                docs.iter()
                    .find(|d| d.widget.upcast_ref::<gtk::Widget>() == &page)
                    .cloned()
            })
            .collect();
        let mut texts = Vec::new();
        let tabs: Vec<Tab> = ordered
            .iter()
            // An empty untitled tab is not worth keeping.
            .filter(|d| d.path().is_some() || !d.text().is_empty())
            .map(|d| {
                if d.is_dirty() {
                    texts.push((d.id, d.text()));
                }
                Tab {
                    id: d.id,
                    path: d.path(),
                    unsaved: d.is_dirty(),
                    cursor: d.cursor_offset(),
                }
            })
            .collect();
        let state = Session {
            root: self.sidebar.root(),
            tabs,
            active: self.notebook.current_page().unwrap_or(0) as usize,
            next_id: self.next_id.get(),
            recent: self.recent.borrow().clone(),
        };
        if let Err(err) = session::save(&state, &texts) {
            log::warn!("saving the session: {err}");
        }
    }
}

fn shortcut(id: &str, key: &str) -> String {
    format!("{} ({key})", tr(id))
}

/// Runs a formatter: the text in, the formatted text out.
fn run_formatter(program: &[String], text: &str, cwd: Option<&Path>) -> Result<String, String> {
    use std::io::Write;
    let mut command = std::process::Command::new(&program[0]);
    command
        .args(&program[1..])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let mut stdin = child.stdin.take().ok_or("no input")?;
    let input = text.to_owned();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let _ = writer.join();
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

/// A small window that asks for a line of text.
pub fn ask_text(parent: &gtk::Window, title: &str, initial: &str, done: impl Fn(String) + 'static) {
    let window = gtk::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title(title)
        .default_width(380)
        .resizable(false)
        .css_classes(["orbit-edit", "ask-text"])
        .build();
    let entry = gtk::Entry::builder().text(initial).hexpand(true).build();
    let label = gtk::Label::builder().label(title).xalign(0.0).build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(["quick-open-box"])
        .build();
    content.append(&label);
    content.append(&entry);
    window.set_child(Some(&content));
    let done = Rc::new(done);
    {
        let window = window.clone();
        let done = done.clone();
        entry.connect_activate(move |e| {
            let text = e.text().to_string();
            window.close();
            done(text);
        });
    }
    let keys = gtk::EventControllerKey::new();
    {
        let window = window.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                window.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    window.add_controller(keys);
    window.present();
    entry.grab_focus();
    entry.select_region(0, -1);
}
