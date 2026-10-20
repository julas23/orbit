//! The window: a bar along the top with what applies to the document open
//! — the sidebar, the page, the zoom, search, night mode and printing for a
//! PDF; the source for Markdown — the search bar under it, and the
//! document.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{gdk, gio, glib, prelude::*};
use orbit_app::i18n::{text, tr, tr_args};

use crate::{
    Kind,
    pdf::PdfView,
    textdoc::{self, TextDoc},
};

enum Doc {
    Pdf(Rc<PdfView>),
    Text(Rc<TextDoc>),
}

pub struct Reader {
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    holder: gtk::Box,
    message: gtk::Label,
    password: gtk::PasswordEntry,
    name: gtk::Label,
    bar: Bar,
    search_bar: gtk::SearchBar,
    search: gtk::SearchEntry,
    counter: gtk::Label,
    doc: RefCell<Option<Doc>>,
    path: RefCell<Option<PathBuf>>,
}

struct Bar {
    sidebar: gtk::ToggleButton,
    page: gtk::Entry,
    pages: gtk::Label,
    page_box: gtk::Box,
    zoom_box: gtk::Box,
    zoom: gtk::Label,
    find: gtk::ToggleButton,
    night: gtk::ToggleButton,
    print: gtk::Button,
    source: gtk::ToggleButton,
}

impl Reader {
    pub fn new(app: &gtk::Application) -> Rc<Self> {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title(tr("read-title"))
            .default_width(900)
            .default_height(800)
            .css_classes(["orbit-read"])
            .build();

        let button = |icon: &str, tip: &str, key: &str| {
            gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(shortcut(tip, key))
                .focus_on_click(false)
                .build()
        };
        let toggle = |icon: &str, tip: &str, key: &str| {
            gtk::ToggleButton::builder()
                .icon_name(icon)
                .tooltip_text(shortcut(tip, key))
                .focus_on_click(false)
                .build()
        };
        let sidebar = toggle("sidebar-show-symbolic", "read-sidebar", "F9");
        let open = button("document-open-symbolic", "read-open", "Ctrl+O");
        let name = gtk::Label::builder()
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .hexpand(true)
            .xalign(0.0)
            .css_classes(["name"])
            .build();
        let page = gtk::Entry::builder()
            .width_chars(4)
            .max_width_chars(5)
            .xalign(1.0)
            .tooltip_text(tr("read-page"))
            .build();
        let pages = gtk::Label::builder().css_classes(["pages-count"]).build();
        let page_box = gtk::Box::builder().spacing(5).build();
        page_box.append(&page);
        page_box.append(&pages);
        let zoom_out = button("zoom-out-symbolic", "read-zoom-out", "Ctrl+−");
        let zoom = gtk::Label::builder().width_chars(5).css_classes(["zoom"]).build();
        let zoom_in = button("zoom-in-symbolic", "read-zoom-in", "Ctrl++");
        let fit = button("zoom-fit-best-symbolic", "read-fit", "Ctrl+0");
        let zoom_box = gtk::Box::builder().css_classes(["group"]).build();
        for w in [zoom_out.upcast_ref::<gtk::Widget>(), zoom.upcast_ref(), zoom_in.upcast_ref(), fit.upcast_ref()] {
            zoom_box.append(w);
        }
        let find = toggle("system-search-symbolic", "read-search", "Ctrl+F");
        let night = toggle("weather-clear-night-symbolic", "read-night", "Ctrl+I");
        let print = button("printer-symbolic", "read-print", "Ctrl+P");
        let source = toggle("text-x-generic-symbolic", "read-source", "Ctrl+U");
        let controls = gtk::WindowControls::builder()
            .side(gtk::PackType::End)
            .decoration_layout(":minimize,maximize,close")
            .build();
        let row = gtk::Box::builder().spacing(10).css_classes(["bar"]).build();
        for w in [
            sidebar.upcast_ref::<gtk::Widget>(),
            open.upcast_ref(),
            name.upcast_ref(),
            page_box.upcast_ref(),
            zoom_box.upcast_ref(),
            find.upcast_ref(),
            source.upcast_ref(),
            night.upcast_ref(),
            print.upcast_ref(),
            controls.upcast_ref(),
        ] {
            row.append(w);
        }
        window.set_titlebar(Some(&gtk::WindowHandle::builder().child(&row).build()));

        let search = gtk::SearchEntry::builder()
            .placeholder_text(tr("read-search-placeholder"))
            .width_chars(30)
            .build();
        let previous = button("go-up-symbolic", "read-search-previous", "Shift+Enter");
        let next = button("go-down-symbolic", "read-search-next", "Enter");
        let counter = gtk::Label::builder().css_classes(["counter"]).build();
        let search_row = gtk::Box::builder().spacing(5).build();
        search_row.append(&search);
        search_row.append(&previous);
        search_row.append(&next);
        search_row.append(&counter);
        let search_bar = gtk::SearchBar::builder()
            .child(&search_row)
            .show_close_button(true)
            .build();
        search_bar.connect_entry(&search);

        let message = gtk::Label::builder()
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["message"])
            .build();
        let password = gtk::PasswordEntry::builder()
            .show_peek_icon(true)
            .placeholder_text(tr("read-password"))
            .width_chars(25)
            .visible(false)
            .build();
        let open_page = gtk::Button::builder()
            .label(tr("read-open-button"))
            .halign(gtk::Align::Center)
            .build();
        let page_message = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(15)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();
        page_message.append(
            &gtk::Image::builder()
                .icon_name("x-office-document-symbolic")
                .pixel_size(60)
                .css_classes(["message-icon"])
                .build(),
        );
        page_message.append(&message);
        page_message.append(&password);
        page_message.append(&open_page);
        let holder = gtk::Box::builder().hexpand(true).vexpand(true).build();
        let stack = gtk::Stack::builder().build();
        stack.add_named(&page_message, Some("message"));
        stack.add_named(&holder, Some("doc"));

        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        body.append(&search_bar);
        body.append(&stack);
        window.set_child(Some(&body));

        let reader = Rc::new(Self {
            window,
            stack,
            holder,
            message,
            password,
            name,
            bar: Bar {
                sidebar,
                page,
                pages,
                page_box,
                zoom_box,
                zoom,
                find,
                night,
                print,
                source,
            },
            search_bar,
            search,
            counter,
            doc: RefCell::default(),
            path: RefCell::default(),
        });

        let act = |button: &gtk::Button, f: fn(&Rc<Reader>)| {
            let weak = Rc::downgrade(&reader);
            button.connect_clicked(move |_| {
                if let Some(r) = weak.upgrade() {
                    f(&r);
                }
            });
        };
        act(&open, |r| r.choose());
        act(&open_page, |r| r.choose());
        act(&zoom_out, |r| r.zoom(Some(1.0 / 1.1)));
        act(&zoom_in, |r| r.zoom(Some(1.1)));
        act(&fit, |r| r.zoom(None));
        act(&reader.bar.print, |r| {
            if let Some(Doc::Pdf(pdf)) = r.doc.borrow().as_ref() {
                pdf.print(r.window.upcast_ref());
            }
        });
        act(&previous, |r| r.search_step(false));
        act(&next, |r| r.search_step(true));
        reader.connect();
        reader.set_message(&tr("read-empty"));
        reader
    }

    pub fn window(&self) -> &gtk::ApplicationWindow {
        &self.window
    }

    fn connect(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.bar.sidebar.connect_toggled(move |b| {
            if let Some(r) = weak.upgrade()
                && let Some(Doc::Pdf(pdf)) = r.doc.borrow().as_ref()
            {
                pdf.show_sidebar(b.is_active());
            }
        });
        let weak = Rc::downgrade(self);
        self.bar.source.connect_toggled(move |b| {
            if let Some(r) = weak.upgrade()
                && let Some(Doc::Text(doc)) = r.doc.borrow().as_ref()
            {
                doc.show_source(b.is_active());
                // The matches belong to the other view.
                r.run_search();
            }
        });
        let weak = Rc::downgrade(self);
        self.bar.night.connect_toggled(move |b| {
            if let Some(r) = weak.upgrade() {
                if b.is_active() {
                    r.holder.add_css_class("night");
                } else {
                    r.holder.remove_css_class("night");
                }
            }
        });
        self.bar
            .find
            .bind_property("active", &self.search_bar, "search-mode-enabled")
            .bidirectional()
            .build();
        let weak = Rc::downgrade(self);
        self.bar.page.connect_activate(move |entry| {
            let Some(r) = weak.upgrade() else { return };
            if let (Some(Doc::Pdf(pdf)), Ok(n)) = (r.doc.borrow().as_ref(), entry.text().trim().parse::<usize>()) {
                pdf.go_to(n.clamp(1, pdf.n_pages()) - 1, None);
            }
            r.update();
        });
        // Closing the search clears its marks.
        let weak = Rc::downgrade(self);
        self.search_bar.connect_search_mode_enabled_notify(move |_| {
            if let Some(r) = weak.upgrade() {
                r.run_search();
            }
        });
        let weak = Rc::downgrade(self);
        self.search.connect_search_changed(move |_| {
            if let Some(r) = weak.upgrade() {
                r.run_search();
            }
        });
        let weak = Rc::downgrade(self);
        self.search.connect_activate(move |_| {
            if let Some(r) = weak.upgrade() {
                r.search_step(true);
            }
        });
        let weak = Rc::downgrade(self);
        let shift_enter = gtk::EventControllerKey::new();
        shift_enter.connect_key_pressed(move |_, key, _, state| {
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
                && state.contains(gdk::ModifierType::SHIFT_MASK)
                && let Some(r) = weak.upgrade()
            {
                r.search_step(false);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.search.add_controller(shift_enter);
        let weak = Rc::downgrade(self);
        self.password.connect_activate(move |entry| {
            let Some(r) = weak.upgrade() else { return };
            let path = r.path.borrow().clone();
            if let Some(path) = path {
                r.open_pdf(&path, Some(entry.text().as_str()));
            }
        });

        // Keys.
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(r) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let is_pdf = matches!(r.doc.borrow().as_ref(), Some(Doc::Pdf(_)));
            use gdk::Key;
            match key {
                Key::o if ctrl => r.choose(),
                Key::f if ctrl => r.bar.find.set_active(true),
                Key::w | Key::q if ctrl => r.window.close(),
                Key::plus | Key::equal | Key::KP_Add if ctrl => r.zoom(Some(1.1)),
                Key::minus | Key::KP_Subtract if ctrl => r.zoom(Some(1.0 / 1.1)),
                Key::_0 | Key::KP_0 if ctrl => r.zoom(None),
                Key::u if ctrl => r.bar.source.set_active(!r.bar.source.is_active()),
                Key::F11 => {
                    if r.window.is_fullscreen() {
                        r.window.unfullscreen();
                    } else {
                        r.window.fullscreen();
                    }
                }
                Key::F9 if is_pdf => r.bar.sidebar.set_active(!r.bar.sidebar.is_active()),
                Key::i if ctrl && is_pdf => r.bar.night.set_active(!r.bar.night.is_active()),
                Key::p if ctrl && is_pdf => r.bar.print.emit_clicked(),
                Key::c if ctrl && is_pdf => r.copy_pdf_selection(),
                Key::Page_Down | Key::Page_Up | Key::Home | Key::End if is_pdf && !r.bar.page.has_focus() => {
                    r.pdf_key(key)
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
            let (Some(r), Ok(file)) = (weak.upgrade(), value.get::<gio::File>()) else {
                return false;
            };
            file.path().map(|p| r.open(&p)).is_some()
        });
        self.window.add_controller(target);
    }

    // ------------------------------------------------------------ opening

    pub fn open(self: &Rc<Self>, path: &Path) {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        *self.path.borrow_mut() = Some(path.clone());
        match crate::kind_of(&path) {
            Some(Kind::Pdf) => self.open_pdf(&path, None),
            Some(Kind::Markdown) => self.open_text(&path, textdoc::Kind::Markdown),
            Some(Kind::Text) | None => self.open_text(&path, textdoc::Kind::Text),
        }
    }

    fn open_pdf(self: &Rc<Self>, path: &Path, password: Option<&str>) {
        match PdfView::open(path, password) {
            Ok(pdf) => {
                let weak = Rc::downgrade(self);
                pdf.connect_change(move || {
                    if let Some(r) = weak.upgrade() {
                        r.update();
                    }
                });
                let title = pdf.title();
                pdf.show_sidebar(self.bar.sidebar.is_active());
                self.show(Doc::Pdf(pdf), path, title);
            }
            Err(err) if err.matches(poppler::Error::Encrypted) => {
                self.set_message(&tr_args(
                    if password.is_some() { "read-wrong-password" } else { "read-encrypted" },
                    &[("name", text(file_name(path)))],
                ));
                self.password.set_text("");
                self.password.set_visible(true);
                self.password.grab_focus();
            }
            Err(err) => self.failed(path, err.message()),
        }
    }

    fn open_text(self: &Rc<Self>, path: &Path, kind: textdoc::Kind) {
        match TextDoc::open(path, kind) {
            Ok(doc) => {
                let weak = Rc::downgrade(self);
                doc.connect_open(move |p| {
                    if let Some(r) = weak.upgrade() {
                        r.open(p);
                    }
                });
                self.bar.source.set_active(false);
                self.show(Doc::Text(doc), path, None);
            }
            Err(err) => self.failed(path, &err.to_string()),
        }
    }

    fn show(self: &Rc<Self>, doc: Doc, path: &Path, title: Option<String>) {
        while let Some(child) = self.holder.first_child() {
            self.holder.remove(&child);
        }
        let widget: gtk::Widget = match &doc {
            Doc::Pdf(pdf) => pdf.widget.clone().upcast(),
            Doc::Text(text) => text.widget.clone().upcast(),
        };
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        self.holder.append(&widget);
        *self.doc.borrow_mut() = Some(doc);
        self.password.set_visible(false);
        self.stack.set_visible_child_name("doc");
        let name = file_name(path);
        self.name.set_label(&name);
        self.window.set_title(Some(&title.unwrap_or(name)));
        self.update();
        self.run_search();
    }

    fn failed(&self, path: &Path, reason: &str) {
        self.set_message(&tr_args(
            "read-failed",
            &[("name", text(file_name(path))), ("reason", text(reason))],
        ));
    }

    fn set_message(&self, message: &str) {
        self.doc.borrow_mut().take();
        while let Some(child) = self.holder.first_child() {
            self.holder.remove(&child);
        }
        self.message.set_label(message);
        self.password.set_visible(false);
        self.stack.set_visible_child_name("message");
        self.name.set_label("");
        self.update();
    }

    fn choose(self: &Rc<Self>) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(&tr("read-filter")));
        for mime in crate::MIME_TYPES {
            filter.add_mime_type(mime);
        }
        for pattern in ["*.pdf", "*.md", "*.markdown", "*.txt"] {
            filter.add_pattern(pattern);
        }
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title(tr("read-open"))
            .filters(&filters)
            .default_filter(&filter)
            .modal(true)
            .build();
        if let Some(dir) = self.path.borrow().as_ref().and_then(|p| p.parent()) {
            dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
        }
        let weak = Rc::downgrade(self);
        dialog.open(Some(&self.window), gio::Cancellable::NONE, move |result| {
            if let (Some(r), Ok(file)) = (weak.upgrade(), result)
                && let Some(path) = file.path()
            {
                r.open(&path);
            }
        });
    }

    // ------------------------------------------------------------ the bar

    /// Shows what applies to the document, and its page and zoom.
    fn update(&self) {
        let doc = self.doc.borrow();
        let (pdf, text) = match doc.as_ref() {
            Some(Doc::Pdf(p)) => (Some(p), None),
            Some(Doc::Text(t)) => (None, Some(t)),
            None => (None, None),
        };
        let markdown = text.is_some_and(|t| t.kind == textdoc::Kind::Markdown);
        self.bar.sidebar.set_visible(pdf.is_some());
        self.bar.page_box.set_visible(pdf.is_some());
        self.bar.night.set_visible(pdf.is_some());
        self.bar.print.set_visible(pdf.is_some());
        self.bar.source.set_visible(markdown);
        self.bar.zoom_box.set_visible(doc.is_some());
        self.bar.find.set_visible(doc.is_some());
        if let Some(pdf) = pdf {
            if !self.bar.page.has_focus() {
                self.bar.page.set_text(&(pdf.current_page() + 1).to_string());
            }
            self.bar.pages.set_label(&tr_args(
                "read-of-pages",
                &[("total", orbit_app::i18n::number(pdf.n_pages() as f64))],
            ));
            self.bar.zoom.set_label(&format!("{:.0}%", pdf.zoom() * 100.0));
        }
        if let Some(text) = text {
            self.bar.zoom.set_label(&format!("{:.0}%", text.scale() * 100.0));
        }
    }

    /// `Some(factor)` zooms by it; `None` fits (PDF) or resets (text).
    fn zoom(&self, factor: Option<f64>) {
        match (self.doc.borrow().as_ref(), factor) {
            (Some(Doc::Pdf(pdf)), Some(f)) => pdf.zoom_by(f),
            (Some(Doc::Pdf(pdf)), None) => pdf.fit_width(),
            (Some(Doc::Text(text)), Some(f)) => text.zoom_by(f),
            (Some(Doc::Text(text)), None) => text.reset_zoom(),
            (None, _) => {}
        }
        self.update();
    }

    fn pdf_key(&self, key: gdk::Key) {
        if let Some(Doc::Pdf(pdf)) = self.doc.borrow().as_ref() {
            let current = pdf.current_page();
            let last = pdf.n_pages().saturating_sub(1);
            let target = match key {
                gdk::Key::Page_Down => (current + 1).min(last),
                gdk::Key::Page_Up => current.saturating_sub(1),
                gdk::Key::Home => 0,
                _ => last,
            };
            pdf.go_to(target, None);
        }
    }

    fn copy_pdf_selection(&self) {
        if let Some(Doc::Pdf(pdf)) = self.doc.borrow().as_ref()
            && let Some(text) = pdf.selected_text()
        {
            self.window.clipboard().set_text(&text);
        }
    }

    // ------------------------------------------------------------- search

    fn run_search(self: &Rc<Self>) {
        let query = if self.search_bar.is_search_mode() {
            self.search.text().to_string()
        } else {
            String::new()
        };
        match self.doc.borrow().as_ref() {
            Some(Doc::Pdf(pdf)) => {
                let weak = Rc::downgrade(self);
                pdf.search(&query, move |count| {
                    if let Some(r) = weak.upgrade() {
                        r.counter.set_label(&count);
                    }
                });
            }
            Some(Doc::Text(text)) => self.counter.set_label(&text.search(&query)),
            None => self.counter.set_label(""),
        }
    }

    fn search_step(&self, forward: bool) {
        let count = match self.doc.borrow().as_ref() {
            Some(Doc::Pdf(pdf)) => pdf.search_step(forward),
            Some(Doc::Text(text)) => text.search_step(forward),
            None => return,
        };
        self.counter.set_label(&count);
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn shortcut(id: &str, key: &str) -> String {
    format!("{} ({key})", tr(id))
}
