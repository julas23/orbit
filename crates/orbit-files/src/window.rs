//! The window: the bar along the top (history, the path, the views, the
//! split), the places on the left, tabs — each one folder, or two side by
//! side (F3) — and the jobs bar along the bottom.
//!
//! The keyboard works both ways: the usual keys (arrows, Enter, Backspace,
//! Ctrl+C/X/V, Delete…), the function keys as in Total Commander and
//! Midnight Commander (F1 help … F12, see `help`), and Yazi's and Vim's
//! (h j k l, gg, G, Space to mark, yy, dd, pp, /).

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::Instant,
};

use gtk::{gdk, gio, glib, prelude::*};
use orbit_app::i18n::{number, text, tr, tr_args};

use crate::{
    actions::{self, Action},
    clipboard,
    entry::{Entry, Location},
    ops::{Jobs, Kind},
    pane::{Pane, View},
    places::Places,
    rename,
};

/// A tab: one pane, or two side by side.
struct Tab {
    root: gtk::Paned,
    panes: RefCell<Vec<Rc<Pane>>>,
    active: Cell<usize>,
    label: gtk::Label,
}

pub struct Window {
    window: gtk::ApplicationWindow,
    notebook: gtk::Notebook,
    tabs: RefCell<Vec<Rc<Tab>>>,
    path_bar: gtk::Box,
    path_entry: gtk::Entry,
    path_stack: gtk::Stack,
    views: [gtk::ToggleButton; 3],
    split: gtk::ToggleButton,
    hidden: gtk::ToggleButton,
    places: RefCell<Option<Rc<Places>>>,
    places_widget: gtk::Box,
    /// Under the places: the selection's thumbnail, name and size.
    side_image: gtk::Image,
    side_name: gtk::Label,
    side_info: gtk::Label,
    jobs: Rc<Jobs>,
    actions: Vec<Action>,
    /// The first key of a two-key Vim command (gg, yy, dd, pp), and when.
    pending: Cell<Option<(char, Instant)>>,
    /// Set while the toggles are set from the pane, not by a click.
    syncing: Cell<bool>,
}

impl Window {
    pub fn new(app: &gtk::Application, config: crate::Config, actions: Vec<Action>) -> Rc<Self> {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title(tr("files-title"))
            .default_width(1200)
            .default_height(750)
            .css_classes(["orbit-files"])
            .build();

        let button = |icon: &str, tip: &str, key: &str| {
            gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(format!("{} ({key})", tr(tip)))
                .focus_on_click(false)
                .build()
        };
        let toggle = |icon: &str, tip: &str, key: &str| {
            gtk::ToggleButton::builder()
                .icon_name(icon)
                .tooltip_text(format!("{} ({key})", tr(tip)))
                .focus_on_click(false)
                .build()
        };
        let back = button("go-previous-symbolic", "files-back", "Alt+←");
        let forward = button("go-next-symbolic", "files-forward", "Alt+→");
        let up = button("go-up-symbolic", "files-up", "Alt+↑");
        let path_bar = gtk::Box::builder().css_classes(["path-bar", "linked"]).build();
        let path_entry = gtk::Entry::builder().hexpand(true).build();
        let path_stack = gtk::Stack::builder().hexpand(true).build();
        path_stack.add_named(
            &gtk::ScrolledWindow::builder()
                .child(&path_bar)
                .vscrollbar_policy(gtk::PolicyType::Never)
                .hscrollbar_policy(gtk::PolicyType::External)
                .build(),
            Some("buttons"),
        );
        path_stack.add_named(&path_entry, Some("entry"));
        let list = toggle("view-list-symbolic", "files-view-list", "Ctrl+1");
        let grid = toggle("view-grid-symbolic", "files-view-grid", "Ctrl+2");
        let columns = toggle("view-dual-symbolic", "files-view-columns", "Ctrl+3");
        grid.set_group(Some(&list));
        columns.set_group(Some(&list));
        let views_box = gtk::Box::builder().css_classes(["linked"]).build();
        for b in [&list, &grid, &columns] {
            views_box.append(b);
        }
        let split = toggle("view-paged-symbolic", "files-split", "F10");
        let hidden = toggle("view-reveal-symbolic", "files-hidden", "Ctrl+H");
        let search = button("system-search-symbolic", "files-filter", "F3");
        let row = gtk::Box::builder().spacing(5).css_classes(["bar"]).build();
        let nav = gtk::Box::builder().css_classes(["linked"]).build();
        for b in [&back, &forward, &up] {
            nav.append(b);
        }
        row.append(&nav);
        row.append(&path_stack);
        row.append(&search);
        row.append(&hidden);
        row.append(&views_box);
        row.append(&split);
        row.append(
            &gtk::WindowControls::builder()
                .side(gtk::PackType::End)
                .decoration_layout(":minimize,maximize,close")
                .build(),
        );
        window.set_titlebar(Some(&gtk::WindowHandle::builder().child(&row).build()));

        let notebook = gtk::Notebook::builder()
            .scrollable(true)
            .show_border(false)
            .show_tabs(false)
            .hexpand(true)
            .vexpand(true)
            .build();
        let jobs = Jobs::new(window.upcast_ref());
        let places_widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["places-box"])
            .build();
        let side_image = gtk::Image::builder().pixel_size(128).css_classes(["side-thumb"]).build();
        let side_name = gtk::Label::builder()
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .justify(gtk::Justification::Center)
            .max_width_chars(20)
            .css_classes(["side-name"])
            .build();
        let side_info = gtk::Label::builder()
            .wrap(true)
            .justify(gtk::Justification::Center)
            .max_width_chars(22)
            .css_classes(["side-info"])
            .build();
        let side = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(5)
            .css_classes(["side-preview"])
            .build();
        side.append(&side_image);
        side.append(&side_name);
        side.append(&side_info);
        let right = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        right.append(&notebook);
        right.append(&jobs.widget);
        let body = gtk::Paned::builder()
            .start_child(&places_widget)
            .end_child(&right)
            .shrink_start_child(false)
            .resize_start_child(false)
            .position(200)
            .build();
        window.set_child(Some(&body));

        let w = Rc::new(Self {
            window,
            notebook,
            tabs: RefCell::default(),
            path_bar,
            path_entry,
            path_stack,
            views: [list.clone(), grid.clone(), columns.clone()],
            split: split.clone(),
            hidden: hidden.clone(),
            places: RefCell::default(),
            places_widget,
            side_image,
            side_name,
            side_info,
            jobs,
            actions,
            pending: Cell::new(None),
            syncing: Cell::new(false),
        });
        let weak = Rc::downgrade(&w);
        let places = Places::new(config.devices == "all", move |to| {
            if let Some(w) = weak.upgrade() {
                w.pane().go(to);
            }
        });
        places.widget.set_vexpand(true);
        w.places_widget.append(&places.widget);
        w.places_widget.append(&side);
        *w.places.borrow_mut() = Some(places);

        let act = |b: &gtk::Button, f: fn(&Rc<Window>)| {
            let weak = Rc::downgrade(&w);
            b.connect_clicked(move |_| {
                if let Some(w) = weak.upgrade() {
                    f(&w);
                }
            });
        };
        act(&back, |w| w.pane().back());
        act(&forward, |w| w.pane().forward());
        act(&up, |w| w.pane().up());
        act(&search, |w| w.pane().open_filter());
        for (n, b) in [&list, &grid, &columns].into_iter().enumerate() {
            let weak = Rc::downgrade(&w);
            b.connect_toggled(move |b| {
                if let Some(w) = weak.upgrade()
                    && b.is_active()
                    && !w.syncing.get()
                {
                    w.pane().set_view([View::List, View::Grid, View::Columns][n]);
                }
            });
        }
        let weak = Rc::downgrade(&w);
        split.connect_toggled(move |b| {
            if let Some(w) = weak.upgrade()
                && !w.syncing.get()
            {
                w.set_split(b.is_active());
            }
        });
        let weak = Rc::downgrade(&w);
        hidden.connect_toggled(move |b| {
            if let Some(w) = weak.upgrade()
                && !w.syncing.get()
            {
                for tab in w.tabs.borrow().iter() {
                    for pane in tab.panes.borrow().iter() {
                        pane.set_hidden(b.is_active());
                    }
                }
            }
        });
        let weak = Rc::downgrade(&w);
        w.path_entry.connect_activate(move |e| {
            if let Some(w) = weak.upgrade() {
                w.go_typed(&e.text());
            }
        });
        let weak = Rc::downgrade(&w);
        w.notebook.connect_switch_page(move |_, _, _| {
            if let Some(w) = weak.upgrade() {
                glib::idle_add_local_once(move || w.sync());
            }
        });
        w.connect_keys();
        w.connect_mouse_buttons();
        w.actions_group();
        w
    }

    /// The mouse's back and forward buttons (8 and 9), read from the raw
    /// events: a click gesture follows sequences of clicks and loses the
    /// second of two different buttons.
    fn connect_mouse_buttons(self: &Rc<Self>) {
        let events = gtk::EventControllerLegacy::new();
        events.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        events.connect_event(move |_, event| {
            let (Some(w), Some(button)) = (weak.upgrade(), event.downcast_ref::<gdk::ButtonEvent>()) else {
                return glib::Propagation::Proceed;
            };
            if event.event_type() != gdk::EventType::ButtonPress {
                return glib::Propagation::Proceed;
            }
            match button.button() {
                8 => w.pane().back(),
                9 => w.pane().forward(),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.window.add_controller(events);
    }

    /// The selection's thumbnail under the places.
    fn show_side(&self) {
        let pane = self.pane();
        let entry = pane.focused();
        match entry {
            Some(e) => {
                self.side_image.set_from_gicon(&crate::entry::icon(&e));
                crate::thumbs::request(&e, &self.side_image);
                self.side_name.set_label(&e.name);
                let kind = gio::content_type_get_description(&e.mime).to_string();
                self.side_info.set_label(&if e.dir {
                    kind
                } else {
                    format!("{}\n{kind}", crate::entry::size_text(e.size))
                });
            }
            None => {
                self.side_image.set_icon_name(None);
                self.side_name.set_label("");
                self.side_info.set_label("");
            }
        }
    }

    pub fn window(&self) -> &gtk::ApplicationWindow {
        &self.window
    }

    pub fn has_tabs(&self) -> bool {
        !self.tabs.borrow().is_empty()
    }

    /// Shows `location` — in the tab shown, or the first tab.
    pub fn show(self: &Rc<Self>, location: Location) {
        if self.has_tabs() {
            self.pane().go(location);
        } else {
            self.new_tab(location);
        }
    }

    // -------------------------------------------------------------- tabs

    pub fn new_tab(self: &Rc<Self>, at: Location) {
        let pane = self.make_pane();
        let root = gtk::Paned::builder().start_child(&pane.widget).wide_handle(true).build();
        let label = gtk::Label::builder().ellipsize(gtk::pango::EllipsizeMode::Middle).max_width_chars(20).build();
        let close = gtk::Button::builder().icon_name("window-close-symbolic").css_classes(["flat", "tab-close"]).build();
        let tab_label = gtk::Box::builder().spacing(5).build();
        tab_label.append(&label);
        tab_label.append(&close);
        let tab = Rc::new(Tab {
            root: root.clone(),
            panes: RefCell::new(vec![pane.clone()]),
            active: Cell::new(0),
            label,
        });
        let page = self.notebook.append_page(&root, Some(&tab_label));
        self.notebook.set_tab_reorderable(&root, true);
        self.tabs.borrow_mut().push(tab.clone());
        self.notebook.set_show_tabs(self.tabs.borrow().len() > 1);
        let weak = Rc::downgrade(self);
        let t = Rc::downgrade(&tab);
        close.connect_clicked(move |_| {
            if let (Some(w), Some(t)) = (weak.upgrade(), t.upgrade()) {
                w.close_tab(&t);
            }
        });
        pane.set_hidden(self.hidden.is_active());
        pane.start(at);
        self.notebook.set_current_page(Some(page));
    }

    fn close_tab(self: &Rc<Self>, tab: &Rc<Tab>) {
        if self.tabs.borrow().len() <= 1 {
            self.window.close();
            return;
        }
        if let Some(n) = self.notebook.page_num(&tab.root) {
            self.notebook.remove_page(Some(n));
        }
        self.tabs.borrow_mut().retain(|t| !Rc::ptr_eq(t, tab));
        self.notebook.set_show_tabs(self.tabs.borrow().len() > 1);
    }

    fn tab(&self) -> Rc<Tab> {
        let page = self.notebook.current_page().and_then(|n| self.notebook.nth_page(Some(n)));
        let tabs = self.tabs.borrow();
        page.and_then(|p| tabs.iter().find(|t| t.root.upcast_ref::<gtk::Widget>() == &p).cloned())
            .unwrap_or_else(|| tabs[0].clone())
    }

    /// The pane the keyboard works in.
    fn pane(&self) -> Rc<Pane> {
        let tab = self.tab();
        let panes = tab.panes.borrow();
        panes[tab.active.get().min(panes.len() - 1)].clone()
    }

    /// The other pane of a split tab, if any.
    fn other_pane(&self) -> Option<Rc<Pane>> {
        let tab = self.tab();
        let panes = tab.panes.borrow();
        (panes.len() == 2).then(|| panes[1 - tab.active.get()].clone())
    }

    fn make_pane(self: &Rc<Self>) -> Rc<Pane> {
        let pane = Pane::new();
        let weak = Rc::downgrade(self);
        pane.connect_open(move |e| {
            if let Some(w) = weak.upgrade() {
                w.open(&e);
            }
        });
        let weak = Rc::downgrade(self);
        let p = Rc::downgrade(&pane);
        pane.connect_moved(move |_| {
            if let (Some(w), Some(p)) = (weak.upgrade(), p.upgrade()) {
                w.activate_pane(&p);
                w.sync();
            }
        });
        let weak = Rc::downgrade(self);
        let p = Rc::downgrade(&pane);
        pane.connect_selected(move |_| {
            if let (Some(w), Some(p)) = (weak.upgrade(), p.upgrade())
                && Rc::ptr_eq(&w.pane(), &p)
            {
                w.show_side();
            }
        });
        let weak = Rc::downgrade(self);
        let p = Rc::downgrade(&pane);
        pane.connect_menu(move |(widget, x, y)| {
            if let (Some(w), Some(p)) = (weak.upgrade(), p.upgrade()) {
                w.activate_pane(&p);
                w.context_menu(&widget, x, y);
            }
        });
        // A click in a pane makes it the active one.
        let click = gtk::GestureClick::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        let p = Rc::downgrade(&pane);
        click.connect_pressed(move |_, _, _, _| {
            if let (Some(w), Some(p)) = (weak.upgrade(), p.upgrade()) {
                w.activate_pane(&p);
                w.sync();
            }
        });
        pane.widget.add_controller(click);
        self.connect_drop(&pane);
        self.connect_drag(&pane);
        pane
    }

    fn activate_pane(&self, pane: &Rc<Pane>) {
        let tab = self.tab();
        let panes = tab.panes.borrow();
        if let Some(n) = panes.iter().position(|p| Rc::ptr_eq(p, pane)) {
            tab.active.set(n);
            for (i, p) in panes.iter().enumerate() {
                if i == n {
                    p.widget.add_css_class("active");
                } else {
                    p.widget.remove_css_class("active");
                }
            }
        }
    }

    fn set_split(self: &Rc<Self>, on: bool) {
        let tab = self.tab();
        let count = tab.panes.borrow().len();
        if on && count == 1 {
            let pane = self.make_pane();
            pane.set_hidden(self.hidden.is_active());
            tab.root.set_end_child(Some(&pane.widget));
            let width = tab.root.width();
            if width > 0 {
                tab.root.set_position(width / 2);
            }
            let here = self.pane().location.borrow().clone();
            tab.panes.borrow_mut().push(pane.clone());
            pane.start(here);
            self.activate_pane(&pane);
        } else if !on && count == 2 {
            let keep = self.pane();
            let gone = tab.panes.borrow().iter().find(|p| !Rc::ptr_eq(p, &keep)).cloned();
            tab.root.set_start_child(Some(&keep.widget));
            tab.root.set_end_child(None::<&gtk::Widget>);
            if let Some(g) = gone {
                tab.panes.borrow_mut().retain(|p| !Rc::ptr_eq(p, &g));
            }
            tab.active.set(0);
            keep.widget.remove_css_class("active");
        }
        self.sync();
    }

    /// The bar, the toggles and the title follow the active pane.
    fn sync(&self) {
        let pane = self.pane();
        let here = pane.location.borrow().clone();
        self.syncing.set(true);
        let n = match pane.view() {
            View::List => 0,
            View::Grid => 1,
            View::Columns => 2,
        };
        self.views[n].set_active(true);
        self.split.set_active(self.tab().panes.borrow().len() == 2);
        self.hidden.set_active(pane.show_hidden.get());
        self.syncing.set(false);
        self.window.set_title(Some(&format!("{} — Orbit-Files", here.name())));
        self.tab().label.set_label(&here.name());
        self.fill_path(&here);
        self.show_side();
    }

    // ---------------------------------------------------------- the path

    fn fill_path(&self, here: &Location) {
        self.path_stack.set_visible_child_name("buttons");
        while let Some(child) = self.path_bar.first_child() {
            self.path_bar.remove(&child);
        }
        let mut chain = vec![here.clone()];
        while let Some(parent) = chain.last().and_then(Location::parent) {
            chain.push(parent);
        }
        chain.reverse();
        // From home, rather than from the root.
        let home = Location::Path(glib::home_dir());
        if let Some(i) = chain.iter().position(|l| *l == home) {
            chain.drain(..i);
        }
        let last = chain.len().saturating_sub(1);
        for (n, place) in chain.into_iter().enumerate() {
            let label = if place == home {
                tr("files-home")
            } else if place.parent().is_none() {
                "/".into()
            } else {
                place.name()
            };
            let b = gtk::Button::builder().label(label).focus_on_click(false).build();
            if n == last {
                b.add_css_class("current");
            }
            let pane = Rc::downgrade(&self.pane());
            b.connect_clicked(move |_| {
                if let Some(p) = pane.upgrade() {
                    p.go(place.clone());
                }
            });
            self.path_bar.append(&b);
        }
        self.path_entry.set_text(&here.display());
    }

    fn edit_path(&self) {
        self.path_stack.set_visible_child_name("entry");
        self.path_entry.grab_focus();
    }

    /// A typed path: `~` for home, a folder or a file's folder, or a URI
    /// (`smb://…`) that GVfs mounts first.
    fn go_typed(self: &Rc<Self>, typed: &str) {
        let typed = typed.trim();
        let pane = self.pane();
        if typed.contains("://") {
            let file = gio::File::for_uri(typed);
            let op = gtk::MountOperation::new(Some(&self.window));
            let p = Rc::downgrade(&pane);
            let f = file.clone();
            file.mount_enclosing_volume(gio::MountMountFlags::NONE, Some(&op), gio::Cancellable::NONE, move |_| {
                if let Some(p) = p.upgrade() {
                    p.go(Location::of(&f));
                }
            });
        } else {
            let path = match typed.strip_prefix('~') {
                Some(rest) => glib::home_dir().join(rest.trim_start_matches('/')),
                None => PathBuf::from(typed),
            };
            let path = if path.is_file() { path.parent().map(PathBuf::from).unwrap_or(path) } else { path };
            pane.go(Location::Path(path));
        }
        pane.focus_view();
    }

    // ----------------------------------------------------- doing things

    fn open(&self, e: &Entry) {
        actions::open(e, None, &WidgetExt::display(&self.window));
    }

    fn here(&self) -> Option<PathBuf> {
        self.pane().location.borrow().path().map(PathBuf::from)
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        self.pane().selected().iter().filter_map(|e| e.location.path().map(PathBuf::from)).collect()
    }

    fn copy(&self, cut: bool) {
        let paths = self.selected_paths();
        if !paths.is_empty() {
            clipboard::put(&self.window, &paths, cut);
        }
    }

    fn paste(self: &Rc<Self>) {
        let Some(target) = self.here() else { return };
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Some(w) = weak.upgrade() else { return };
            if let Some((paths, cut)) = clipboard::take(&w.window).await {
                w.jobs.start(if cut { Kind::Move } else { Kind::Copy }, paths, Some(target));
                if cut {
                    w.window.clipboard().set_text("");
                }
            }
        });
    }

    fn trash(&self) {
        let paths = self.selected_paths();
        self.jobs.start(Kind::Trash, paths, None);
    }

    /// Deleting for good asks first.
    fn delete(self: &Rc<Self>) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let dialog = gtk::AlertDialog::builder()
            .modal(true)
            .message(tr_args("files-delete-question", &[("n", number(paths.len() as f64))]))
            .detail(tr("files-delete-detail"))
            .buttons([tr("files-cancel"), tr("files-delete")])
            .cancel_button(0)
            .default_button(0)
            .build();
        let jobs = self.jobs.clone();
        dialog.choose(Some(&self.window), gio::Cancellable::NONE, move |r| {
            if r == Ok(1) {
                jobs.start(Kind::Delete, paths, None);
            }
        });
    }

    fn new_folder(self: &Rc<Self>) {
        let Some(dir) = self.here() else { return };
        let entry = gtk::Entry::builder().text(tr("files-new-folder-name")).width_chars(30).build();
        let window = gtk::Window::builder()
            .transient_for(&self.window)
            .css_classes(["orbit-files-dialog"])
            .modal(true)
            .title(tr("files-new-folder"))
            .resizable(false)
            .child(&crate::actions::padded("dialog-box", &entry))
            .build();
        let w = window.clone();
        entry.connect_activate(move |e| {
            let name = e.text().trim().to_owned();
            if name.is_empty() || name.contains('/') {
                return;
            }
            let path = dir.join(&name);
            if path.symlink_metadata().is_ok() {
                e.add_css_class("error");
                return;
            }
            if let Err(err) = std::fs::create_dir(&path) {
                log::warn!("new folder: {err}");
            }
            w.close();
        });
        window.present();
        entry.grab_focus();
        entry.select_region(0, -1);
    }

    fn rename(&self) {
        let paths = self.selected_paths();
        match paths.len() {
            0 => {}
            1 => rename::single(self.window.upcast_ref(), &paths[0]),
            _ => rename::bulk(self.window.upcast_ref(), paths),
        }
    }

    fn compress(&self) {
        let (Some(dir), selected) = (self.here(), self.pane().selected()) else { return };
        if selected.is_empty() {
            return;
        }
        let names = selected.iter().map(|e| e.name.clone()).collect();
        actions::compress_dialog(self.window.upcast_ref(), dir, names, self.jobs.clone());
    }

    fn extract(&self) {
        for path in self.selected_paths() {
            let into = actions::extract_folder(&path);
            if std::fs::create_dir_all(&into).is_err() {
                continue;
            }
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let dir = path.parent().map(PathBuf::from).unwrap_or_default();
            self.jobs.command(
                tr_args("files-job-extract", &[("name", text(name))]),
                actions::extract_command(&path, &into),
                dir,
            );
        }
    }

    fn copy_paths(&self) {
        let paths: Vec<String> = self.selected_paths().iter().map(|p| p.display().to_string()).collect();
        if !paths.is_empty() {
            self.window.clipboard().set_text(&paths.join("\n"));
        }
    }

    /// F5 and F6, as in Total Commander: copy or move the selection to
    /// the other pane's folder or, with one pane, to a folder chosen in a
    /// dialog that browses the disk.
    fn transfer(self: &Rc<Self>, kind: Kind) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        if let Some(other) = self.other_pane() {
            if let Some(target) = other.location.borrow().path().map(PathBuf::from) {
                self.jobs.start(kind, paths, Some(target));
            }
            return;
        }
        let title = match kind {
            Kind::Move => tr("files-move-to"),
            _ => tr("files-copy-to"),
        };
        let dialog = gtk::FileDialog::builder().title(title).accept_label(tr("files-choose")).modal(true).build();
        if let Some(here) = self.here() {
            dialog.set_initial_folder(Some(&gio::File::for_path(here)));
        }
        let jobs = self.jobs.clone();
        dialog.select_folder(Some(&self.window), gio::Cancellable::NONE, move |r| {
            if let Some(target) = r.ok().and_then(|f| f.path()) {
                jobs.start(kind, paths, Some(target));
            }
        });
    }

    /// F4: text in Orbit-Edit; anything else in its own application.
    fn edit(&self) {
        let (text, other): (Vec<Entry>, Vec<Entry>) =
            self.pane().selected().into_iter().filter(|e| !e.dir).partition(|e| actions::is_text(&e.mime, &e.name));
        actions::launch_orbit("Edit", &text, &WidgetExt::display(&self.window));
        for e in other {
            self.open(&e);
        }
    }

    /// F9: a look at the file — images in Orbit-View, documents and text in
    /// Orbit-Read, video and sound in Orbit-Play, the rest in its own
    /// application.
    fn view_files(&self) {
        let display = WidgetExt::display(&self.window);
        let mut by_app: Vec<(&str, Vec<Entry>)> = vec![("View", vec![]), ("Read", vec![]), ("Play", vec![])];
        for e in self.pane().selected().into_iter().filter(|e| !e.dir) {
            match actions::viewer(&e.mime, &e.name) {
                Some(app) => by_app.iter_mut().find(|(a, _)| *a == app).expect("a viewer").1.push(e),
                None => self.open(&e),
            }
        }
        for (app, entries) in by_app {
            actions::launch_orbit(app, &entries, &display);
        }
    }

    /// F12: a terminal in this folder.
    fn terminal(&self) {
        let Some(dir) = self.here() else { return };
        let terminal = ["orbit-term", "ghostty", "alacritty", "kitty", "foot", "x-terminal-emulator"]
            .into_iter()
            .find(|t| glib::find_program_in_path(t).is_some());
        if let Some(t) = terminal
            && let Err(err) = std::process::Command::new(t).current_dir(&dir).spawn()
        {
            log::warn!("terminal: {err}");
        }
    }

    /// F11: full screen, or back.
    fn toggle_fullscreen(&self) {
        if self.window.is_fullscreen() {
            self.window.unfullscreen();
        } else {
            self.window.fullscreen();
        }
    }

    /// F1: the keys.
    fn help(&self) {
        crate::help::show(self.window.upcast_ref());
    }

    // ------------------------------------------------- the context menu

    fn actions_group(self: &Rc<Self>) {
        let group = gio::SimpleActionGroup::new();
        let add = |name: &str, f: fn(&Rc<Window>)| {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(w) = weak.upgrade() {
                    f(&w);
                }
            });
            group.add_action(&action);
        };
        add("open", |w| w.pane().open_focused());
        add("cut", |w| w.copy(true));
        add("copy", |w| w.copy(false));
        add("paste", |w| w.paste());
        add("rename", |w| w.rename());
        add("trash", |w| w.trash());
        add("delete", |w| w.delete());
        add("new-folder", |w| w.new_folder());
        add("compress", |w| w.compress());
        add("extract", |w| w.extract());
        add("copy-path", |w| w.copy_paths());
        add("new-tab", |w| {
            let at = w.pane().focused().filter(|e| e.dir).map(|e| e.location).unwrap_or_else(|| w.pane().location.borrow().clone());
            w.new_tab(at);
        });
        let open_with = gio::SimpleAction::new("open-with", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(self);
        open_with.connect_activate(move |_, id| {
            let (Some(w), Some(id)) = (weak.upgrade(), id.and_then(|v| v.get::<String>())) else { return };
            let app = gio::AppInfo::all().into_iter().find(|a| a.id().as_deref() == Some(id.as_str()));
            for e in w.pane().selected() {
                actions::open(&e, app.as_ref(), &WidgetExt::display(&w.window));
            }
        });
        group.add_action(&open_with);
        let custom = gio::SimpleAction::new("custom", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(self);
        custom.connect_activate(move |_, n| {
            let (Some(w), Some(n)) = (weak.upgrade(), n.and_then(|v| v.get::<String>()).and_then(|s| s.parse::<usize>().ok())) else {
                return;
            };
            let (Some(dir), Some(action)) = (w.here(), w.actions.get(n)) else { return };
            action.run(&dir, &w.pane().selected());
        });
        group.add_action(&custom);
        self.window.insert_action_group("files", Some(&group));
    }

    /// The context menu: a popover of plain buttons bound to the window's
    /// actions. (GTK's model-driven menu measures itself short with this
    /// theme and hides its last entries behind a scroll.)
    fn context_menu(self: &Rc<Self>, widget: &gtk::Widget, x: f64, y: f64) {
        let selected = self.pane().selected();
        let popover = gtk::Popover::builder().css_classes(["context-menu"]).build();
        let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        popover.set_child(Some(&column));
        let item_icon = |label: &str, action: &str, target: Option<glib::Variant>, icon: Option<&str>| {
            let content = gtk::Box::builder().spacing(10).build();
            if let Some(icon) = icon {
                content.append(&gtk::Image::from_icon_name(icon));
            }
            content.append(&gtk::Label::builder().label(label).xalign(0.0).build());
            let button = gtk::Button::builder()
                .child(&content)
                .action_name(action)
                .css_classes(["flat", "menu-item"])
                .build();
            if let Some(t) = target {
                button.set_action_target_value(Some(&t));
            }
            let p = popover.downgrade();
            button.connect_clicked(move |_| {
                if let Some(p) = p.upgrade() {
                    p.popdown();
                }
            });
            button.upcast::<gtk::Widget>()
        };
        let item = |label: &str, action: &str, target: Option<glib::Variant>| item_icon(label, action, target, None);
        let mut sections: Vec<Vec<gtk::Widget>> = Vec::new();
        if selected.is_empty() {
            sections.push(vec![
                item(&tr("files-new-folder"), "files.new-folder", None),
                item(&tr("files-paste"), "files.paste", None),
            ]);
        } else {
            let mut open = vec![item(&tr("files-open"), "files.open", None)];
            if let Some(first) = selected.first().filter(|e| !e.dir) {
                let apps: Vec<gtk::Widget> = actions::apps_for(&first.mime)
                    .into_iter()
                    .take(12)
                    .filter_map(|app| {
                        let id = app.id()?;
                        Some(item(&app.display_name(), "files.open-with", Some(id.to_variant())))
                    })
                    .collect();
                if !apps.is_empty() {
                    open.push(submenu(&tr("files-open-with"), apps));
                }
            }
            if selected.iter().all(|e| e.dir) {
                open.push(item(&tr("files-open-tab"), "files.new-tab", None));
            }
            sections.push(open);
            sections.push(vec![
                item(&tr("files-cut"), "files.cut", None),
                item(&tr("files-copy"), "files.copy", None),
                item(&tr("files-paste"), "files.paste", None),
                item(&tr("files-copy-path"), "files.copy-path", None),
            ]);
            // Also here: a full folder may leave no empty space to click.
            sections.push(vec![item(&tr("files-new-folder"), "files.new-folder", None)]);
            let mut manage = vec![item(&tr("files-rename-menu"), "files.rename", None)];
            if selected.iter().all(|e| actions::is_archive(&e.name)) {
                manage.push(item(&tr("files-extract"), "files.extract", None));
            }
            manage.push(item(&tr("files-compress-menu"), "files.compress", None));
            sections.push(manage);
            sections.push(vec![
                item(&tr("files-trash-menu"), "files.trash", None),
                item(&tr("files-delete-menu"), "files.delete", None),
            ]);
        }
        let custom: Vec<gtk::Widget> = self
            .actions
            .iter()
            .enumerate()
            .filter(|(_, a)| a.applies(&selected))
            .map(|(n, a)| item_icon(&a.name, "files.custom", Some(n.to_string().to_variant()), a.icon.as_deref()))
            .collect();
        if !custom.is_empty() {
            sections.push(custom);
        }
        for (n, section) in sections.into_iter().enumerate() {
            if n > 0 {
                column.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            }
            for w in section {
                column.append(&w);
            }
        }
        orbit_app::popup::open(&popover, widget, Some((x, y)));
    }

    // ------------------------------------------------- drag and drop

    fn connect_drag(self: &Rc<Self>, pane: &Rc<Pane>) {
        let source = gtk::DragSource::builder().actions(gdk::DragAction::COPY | gdk::DragAction::MOVE).build();
        let p = Rc::downgrade(pane);
        source.connect_prepare(move |_, _, _| {
            let p = p.upgrade()?;
            let files: Vec<gio::File> = p.selected().iter().map(|e| e.location.file()).collect();
            (!files.is_empty()).then(|| gdk::ContentProvider::for_value(&gdk::FileList::from_array(&files).to_value()))
        });
        pane.widget.add_controller(source);
    }

    /// Dropped files: moved within a disk, copied across; Ctrl copies,
    /// Shift moves.
    fn connect_drop(self: &Rc<Self>, pane: &Rc<Pane>) {
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY | gdk::DragAction::MOVE);
        let weak = Rc::downgrade(self);
        let p = Rc::downgrade(pane);
        target.connect_drop(move |t, value, _, _| {
            let (Some(w), Some(p), Ok(list)) = (weak.upgrade(), p.upgrade(), value.get::<gdk::FileList>()) else {
                return false;
            };
            let Some(dir) = p.location.borrow().path().map(PathBuf::from) else { return false };
            let paths: Vec<PathBuf> = list.files().iter().filter_map(|f| f.path()).collect();
            let state = t.current_event_state();
            let same_disk = paths.first().is_some_and(|s| same_device(s, &dir));
            let kind = if state.contains(gdk::ModifierType::CONTROL_MASK) {
                Kind::Copy
            } else if state.contains(gdk::ModifierType::SHIFT_MASK) || same_disk {
                Kind::Move
            } else {
                Kind::Copy
            };
            w.jobs.start(kind, paths, Some(dir));
            true
        });
        pane.widget.add_controller(target);
    }

    // ------------------------------------------------------------ keys

    fn connect_keys(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(w) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if w.key(key, state) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.window.add_controller(keys);
    }

    /// Handles a key; whether it was one of ours.
    fn key(self: &Rc<Self>, key: gdk::Key, state: gdk::ModifierType) -> bool {
        use gdk::Key;
        let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = state.contains(gdk::ModifierType::ALT_MASK);
        let pane = self.pane();
        // Typing into a field: its keys, but Escape.
        let typing = gtk::prelude::GtkWindowExt::focus(&self.window).is_some_and(|f| f.is::<gtk::Text>());
        if typing {
            if key == Key::Escape {
                if pane.filtering() {
                    pane.close_filter();
                } else {
                    self.sync();
                    pane.focus_view();
                }
                return true;
            }
            return false;
        }
        if alt {
            match key {
                Key::Left => pane.back(),
                Key::Right => pane.forward(),
                Key::Up => pane.up(),
                _ => return false,
            }
            return true;
        }
        if ctrl {
            match key {
                Key::space => pane.toggle_cursor(),
                Key::i | Key::I => pane.invert_selection(),
                Key::r | Key::R => pane.reload(),
                Key::c | Key::C => self.copy(false),
                Key::x | Key::X => self.copy(true),
                Key::v | Key::V => self.paste(),
                Key::a | Key::A => pane.select_all(),
                Key::h | Key::H => self.hidden.set_active(!self.hidden.is_active()),
                Key::l | Key::L => self.edit_path(),
                Key::f | Key::F => pane.open_filter(),
                Key::t | Key::T => {
                    let here = pane.location.borrow().clone();
                    self.new_tab(here);
                }
                Key::w | Key::W => self.close_tab(&self.tab()),
                Key::q | Key::Q => self.window.close(),
                Key::N if shift => self.new_folder(),
                Key::n if shift => self.new_folder(),
                Key::_1 => self.views[0].set_active(true),
                Key::_2 => self.views[1].set_active(true),
                Key::_3 => self.views[2].set_active(true),
                Key::Tab | Key::Page_Down => self.cycle(1),
                Key::ISO_Left_Tab | Key::Page_Up => self.cycle(-1),
                _ => return false,
            }
            return true;
        }
        // Shift with the arrows: the selection runs from where it started.
        if shift && pane.view() != View::Grid {
            let by = match key {
                Key::Down => Some(1),
                Key::Up => Some(-1),
                Key::Page_Down => Some(20),
                Key::Page_Up => Some(-20),
                Key::Home => Some(i64::MIN),
                Key::End => Some(i64::MAX),
                _ => None,
            };
            if let Some(by) = by {
                pane.extend_cursor(by);
                return true;
            }
        }
        // Two-key Vim commands: gg, yy, dd, pp.
        let typed = key.to_unicode();
        if let Some(c) = typed.filter(|c| "gydp".contains(*c))
            && let Some((first, when)) = self.pending.take()
            && first == c
            && when.elapsed().as_millis() < 600
        {
            match c {
                'g' => pane.move_cursor(i64::MIN),
                'y' => self.copy(false),
                'd' => self.copy(true),
                _ => self.paste(),
            }
            return true;
        }
        if let Some(c) = typed.filter(|c| "gydp".contains(*c)) {
            self.pending.set(Some((c, Instant::now())));
            return true;
        }
        match key {
            // In the icon view the arrows are the grid's own.
            Key::j => pane.move_cursor(1),
            Key::k => pane.move_cursor(-1),
            Key::Down if pane.view() != View::Grid => pane.move_cursor(1),
            Key::Up if pane.view() != View::Grid => pane.move_cursor(-1),
            Key::l | Key::Return | Key::KP_Enter => pane.open_focused(),
            Key::Right if pane.view() == View::Columns => pane.open_focused(),
            Key::h | Key::BackSpace => pane.up(),
            Key::Left if pane.view() == View::Columns => pane.up(),
            Key::G => pane.move_cursor(i64::MAX),
            Key::Home => pane.move_cursor(i64::MIN),
            Key::End => pane.move_cursor(i64::MAX),
            Key::Page_Down => pane.move_cursor(20),
            Key::Page_Up => pane.move_cursor(-20),
            Key::space => pane.toggle_selected(),
            Key::slash => pane.open_filter(),
            Key::Escape => {
                if pane.filtering() {
                    pane.close_filter();
                } else {
                    pane.clear_marks();
                }
            }
            Key::Delete | Key::KP_Delete if shift => self.delete(),
            Key::Delete | Key::KP_Delete => self.trash(),
            Key::F1 => self.help(),
            Key::F2 => self.rename(),
            Key::F3 => pane.open_filter(),
            Key::F4 => self.edit(),
            Key::F5 => self.transfer(Kind::Copy),
            Key::F6 => self.transfer(Kind::Move),
            Key::F7 => self.new_folder(),
            Key::F8 if shift => self.delete(),
            Key::F8 => self.trash(),
            Key::F9 => self.view_files(),
            Key::F10 => self.split.set_active(!self.split.is_active()),
            Key::F11 => self.toggle_fullscreen(),
            Key::F12 => self.terminal(),
            Key::Tab => {
                if let Some(other) = self.other_pane() {
                    self.activate_pane(&other);
                    other.focus_view();
                    self.sync();
                }
            }
            _ => return false,
        }
        true
    }

    fn cycle(&self, by: i32) {
        let n = self.notebook.n_pages() as i32;
        if n > 1 {
            let next = (self.notebook.current_page().map_or(0, |p| p as i32) + by).rem_euclid(n);
            self.notebook.set_current_page(Some(next as u32));
        }
    }
}

fn same_device(a: &std::path::Path, b: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev(),
        _ => false,
    }
}

/// A menu entry that opens `items` in a popover beside it.
fn submenu(label: &str, items: Vec<gtk::Widget>) -> gtk::Widget {
    let row = gtk::Box::builder().spacing(10).build();
    row.append(&gtk::Label::builder().label(label).xalign(0.0).hexpand(true).build());
    row.append(&gtk::Image::from_icon_name("go-next-symbolic"));
    let button = gtk::Button::builder().child(&row).css_classes(["flat", "menu-item"]).build();
    let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    let popover = gtk::Popover::builder()
        .child(&column)
        .position(gtk::PositionType::Right)
        .has_arrow(false)
        .css_classes(["context-menu"])
        .build();
    for w in &items {
        column.append(w);
        // Choosing one closes this too (the menu under it closes itself).
        if let Some(item) = w.downcast_ref::<gtk::Button>() {
            let p = popover.downgrade();
            item.connect_clicked(move |_| {
                if let Some(p) = p.upgrade() {
                    p.popdown();
                }
            });
        }
    }
    button.connect_clicked(move |b| orbit_app::popup::open(&popover, b, None));
    button.upcast()
}
