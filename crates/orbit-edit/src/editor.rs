//! The window: the bar along the top, the folder on the left, the tabs,
//! the find bar over them, and the status line under them — where the
//! cursor is, the indentation, the language, the encoding and the line
//! endings.

use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
    time::Duration,
};

use gtk::{gdk, gio, glib, prelude::*};
use orbit_app::i18n::{number, text, tr, tr_args};
use sourceview5::prelude::*;

use crate::{
    Config,
    doc::Doc,
    findbar::FindBar,
    session::{self, Session, Tab},
    sidebar::{self, Sidebar},
};

pub struct Editor {
    window: gtk::ApplicationWindow,
    config: Config,
    notebook: gtk::Notebook,
    sidebar: Rc<Sidebar>,
    findbar: Rc<FindBar>,
    docs: RefCell<Vec<Rc<Doc>>>,
    next_id: Cell<u64>,
    position: gtk::MenuButton,
    goto: gtk::Entry,
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
        let quick = button("system-search-symbolic", "edit-quick-open", "Ctrl+P");
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
        row.append(&find);
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

        let editor = Rc::new(Self {
            window,
            font_size: Cell::new(config.font_size),
            config,
            notebook,
            sidebar,
            findbar,
            docs: RefCell::default(),
            next_id: Cell::new(1),
            position,
            goto,
            indent,
            spaces,
            width,
            language,
            languages,
            encoding,
            sidebar_toggle,
            css,
            save_soon: RefCell::default(),
            updating: Cell::new(false),
        });
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
        act(&quick, |e| e.quick_open());
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
        self.notebook.connect_switch_page(move |_, page, _| {
            let Some(e) = weak.upgrade() else { return };
            let doc = e.docs.borrow().iter().find(|d| d.widget.upcast_ref::<gtk::Widget>() == page).cloned();
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
                doc.view.set_insert_spaces_instead_of_tabs(b.is_active());
                e.status(&doc);
            }
        });
        let weak = Rc::downgrade(self);
        self.width.connect_value_changed(move |s| {
            if let Some(e) = weak.upgrade()
                && !e.updating.get()
                && let Some(doc) = e.current()
            {
                doc.view.set_tab_width(s.value() as u32);
                doc.view.set_indent_width(s.value() as i32);
                e.status(&doc);
            }
        });
        let weak = Rc::downgrade(self);
        self.goto.connect_activate(move |entry| {
            let Some(e) = weak.upgrade() else { return };
            if let (Some(doc), Ok(line)) = (e.current(), entry.text().trim().parse::<i32>()) {
                let iter = doc
                    .buffer
                    .iter_at_line((line - 1).max(0))
                    .unwrap_or_else(|| doc.buffer.end_iter());
                doc.place_cursor(iter.offset());
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
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
            if !ctrl {
                return glib::Propagation::Proceed;
            }
            use gdk::Key;
            match key {
                Key::n | Key::N if !shift => {
                    e.new_tab();
                }
                Key::o | Key::O if shift => e.choose_folder(),
                Key::o | Key::O => e.choose(),
                Key::s | Key::S => e.save_current(shift),
                Key::w | Key::W => e.close_current(),
                Key::q | Key::Q => e.window.close(),
                Key::f | Key::F => e.find(false),
                Key::h | Key::H => e.find(true),
                Key::g | Key::G => {
                    e.position.popup();
                    e.goto.grab_focus();
                }
                Key::p | Key::P => e.quick_open(),
                Key::b | Key::B => e.sidebar_toggle.set_active(!e.sidebar_toggle.is_active()),
                Key::slash | Key::KP_Divide => {
                    if let Some(doc) = e.current() {
                        doc.toggle_comment();
                    }
                }
                Key::bracketleft | Key::braceleft if shift => {
                    if let Some(doc) = e.current() {
                        doc.folds.fold_here();
                    }
                }
                Key::bracketright | Key::braceright if shift => {
                    if let Some(doc) = e.current() {
                        doc.folds.unfold_here();
                    }
                }
                Key::Tab | Key::Page_Down => e.cycle(1),
                Key::ISO_Left_Tab | Key::Page_Up => e.cycle(-1),
                Key::plus | Key::equal | Key::KP_Add => e.zoom(1.0),
                Key::minus | Key::KP_Subtract => e.zoom(-1.0),
                Key::_0 | Key::KP_0 => {
                    e.font_size.set(e.config.font_size);
                    e.apply_font();
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
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

        // Closing keeps everything, unsaved text included, for next time.
        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(e) = weak.upgrade() {
                e.save_session();
            }
            glib::Propagation::Proceed
        });
    }

    // -------------------------------------------------------------- tabs

    fn current(&self) -> Option<Rc<Doc>> {
        let page = self.notebook.nth_page(self.notebook.current_page())?;
        self.docs.borrow().iter().find(|d| d.widget.upcast_ref::<gtk::Widget>() == &page).cloned()
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
            }
        });
        let weak = Rc::downgrade(self);
        let target = Rc::downgrade(&doc);
        doc.buffer.connect_language_notify(move |_| {
            if let (Some(e), Some(d)) = (weak.upgrade(), target.upgrade())
                && e.current().is_some_and(|c| Rc::ptr_eq(&c, &d))
            {
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
        let existing = self.docs.borrow().iter().find(|d| d.path().as_deref() == Some(path.as_path())).cloned();
        if let Some(doc) = existing {
            self.show(&doc);
            return;
        }
        let blank = self
            .current()
            .filter(|d| d.path().is_none() && !d.is_dirty() && d.text().is_empty());
        let doc = blank.unwrap_or_else(|| self.new_tab());
        if let Err(err) = doc.load(Some(&path), None) {
            self.error(&tr_args("edit-open-failed", &[("name", text(path.display().to_string())), ("reason", text(err.to_string()))]));
            return;
        }
        if self.sidebar.root().is_none()
            && let Some(dir) = path.parent()
        {
            self.set_root(Some(dir));
        }
        self.show(&doc);
        self.save_session_soon();
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
        if let Some(n) = self.notebook.page_num(&doc.widget) {
            self.notebook.remove_page(Some(n));
        }
        self.docs.borrow_mut().retain(|d| !Rc::ptr_eq(d, doc));
        if self.docs.borrow().is_empty() {
            self.new_tab();
        }
        self.save_session_soon();
    }

    // ---------------------------------------------------- files, folders

    fn choose(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder().title(tr("edit-open")).modal(true).build();
        if let Some(dir) = self.current().and_then(|d| d.path()).and_then(|p| p.parent().map(Path::to_path_buf)).or_else(|| self.sidebar.root()) {
            dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
        }
        let weak = Rc::downgrade(self);
        dialog.open_multiple(Some(&self.window), gio::Cancellable::NONE, move |result| {
            let (Some(e), Ok(files)) = (weak.upgrade(), result) else { return };
            for file in files.iter::<gio::File>().filter_map(Result::ok) {
                if let Some(path) = file.path() {
                    e.open(&path);
                }
            }
        });
    }

    fn choose_folder(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder().title(tr("edit-open-folder")).modal(true).build();
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

    fn quick_open(self: &Rc<Self>) {
        let root = self
            .sidebar
            .root()
            .or_else(|| self.current().and_then(|d| d.path()).and_then(|p| p.parent().map(Path::to_path_buf)));
        let Some(root) = root else {
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

    fn save_current(self: &Rc<Self>, choose: bool) {
        if let Some(doc) = self.current() {
            self.save(&doc, choose, || {});
        }
    }

    /// Saves `doc` — asking where if it has no file yet, or if `choose` —
    /// then runs `then`.
    fn save(self: &Rc<Self>, doc: &Rc<Doc>, choose: bool, then: impl FnOnce() + 'static) {
        if !choose && doc.path().is_some() {
            match doc.save(None) {
                Ok(()) => then(),
                Err(err) => self.error(&tr_args("edit-save-failed", &[("reason", text(err.to_string()))])),
            }
            self.save_session_soon();
            return;
        }
        let dialog = gtk::FileDialog::builder()
            .title(tr("edit-save-as"))
            .modal(true)
            .initial_name(doc.path().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| tr("edit-untitled-file")))
            .build();
        if let Some(dir) = doc.path().and_then(|p| p.parent().map(Path::to_path_buf)).or_else(|| self.sidebar.root()) {
            dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
        }
        let weak = Rc::downgrade(self);
        let doc = doc.clone();
        dialog.save(Some(&self.window), gio::Cancellable::NONE, move |result| {
            let (Some(e), Ok(file)) = (weak.upgrade(), result) else { return };
            let Some(path) = file.path() else { return };
            match doc.save(Some(&path)) {
                Ok(()) => {
                    e.show(&doc);
                    then();
                }
                Err(err) => e.error(&tr_args("edit-save-failed", &[("reason", text(err.to_string()))])),
            }
            e.save_session_soon();
        });
    }

    fn error(&self, message: &str) {
        gtk::AlertDialog::builder().modal(true).message(message).build().show(Some(&self.window));
    }

    fn find(&self, replace: bool) {
        if let Some(doc) = self.current() {
            self.findbar.show(&doc, replace);
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
        self.updating.set(false);
    }

    fn status_position(&self, doc: &Doc) {
        let at = doc.buffer.iter_at_mark(&doc.buffer.get_insert());
        self.position.set_label(&tr_args(
            "edit-status-position",
            &[("line", number(f64::from(at.line() + 1))), ("column", number(f64::from(at.line_offset() + 1)))],
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
            }
        }
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
            .filter_map(|page| docs.iter().find(|d| d.widget.upcast_ref::<gtk::Widget>() == &page).cloned())
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
        };
        if let Err(err) = session::save(&state, &texts) {
            log::warn!("saving the session: {err}");
        }
    }
}

fn shortcut(id: &str, key: &str) -> String {
    format!("{} ({key})", tr(id))
}
