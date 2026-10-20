//! A pane: one folder, seen as a list (with columns), as icons, or as
//! Yazi's columns — the parent folder, the folder, and a preview of what
//! is selected. The three views share one model and one selection, so
//! switching keeps both.
//!
//! The folder fills in as its entries arrive (see `entry`); a monitor
//! keeps it current, entry by entry, without listing it again.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::{number, text, tr, tr_args};

use crate::{
    entry::{self, Entry, Listing, Location},
    preview::Preview,
    thumbs,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    List,
    Grid,
    Columns,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Modified,
    Kind,
}

type Callback<T> = RefCell<Option<Box<dyn Fn(T)>>>;

pub struct Pane {
    pub widget: gtk::Box,
    stack: gtk::Stack,
    store: gio::ListStore,
    filter: gtk::CustomFilter,
    sorter: gtk::CustomSorter,
    pub selection: gtk::MultiSelection,
    list: gtk::ColumnView,
    grid: gtk::GridView,
    current: gtk::ListView,
    parent_store: gio::ListStore,
    parent_list: gtk::ListView,
    pub preview: Rc<Preview>,
    status: gtk::Label,
    filter_entry: gtk::SearchEntry,
    filter_bar: gtk::Revealer,

    pub location: RefCell<Location>,
    back: RefCell<Vec<Location>>,
    forward: RefCell<Vec<Location>>,
    generation: Cell<u64>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    /// Selected again once the folder is listed (going up selects the
    /// folder left).
    reselect: RefCell<Option<String>>,
    pub show_hidden: Cell<bool>,
    sort: Cell<(SortKey, bool)>,
    view: Cell<View>,
    loading: Cell<bool>,
    /// Where the keyboard is, apart from the selection: with entries
    /// marked (Space), j and k move it and leave the marks alone.
    cursor: Cell<Option<u32>>,
    marking: Cell<bool>,
    on_open: Callback<Entry>,
    on_moved: Callback<()>,
    on_selected: Callback<()>,
    on_menu: Callback<(gtk::Widget, f64, f64)>,
}

impl Pane {
    pub fn new() -> Rc<Self> {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let filter = gtk::CustomFilter::new(|_| true);
        let sorter = gtk::CustomSorter::new(|_, _| gtk::Ordering::Equal);
        let filtered = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
        filtered.set_incremental(false);
        let sorted = gtk::SortListModel::new(Some(filtered), Some(sorter.clone()));
        let selection = gtk::MultiSelection::new(Some(sorted));

        let list = column_view(&selection);
        let grid = gtk::GridView::builder()
            .model(&selection)
            .factory(&grid_factory())
            .max_columns(30)
            .min_columns(1)
            .enable_rubberband(true)
            .css_classes(["grid"])
            .build();
        let current = gtk::ListView::builder()
            .model(&selection)
            .factory(&row_factory())
            .css_classes(["column-list"])
            .build();
        let parent_store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let parent_sorted = gtk::SortListModel::new(
            Some(parent_store.clone()),
            Some(gtk::CustomSorter::new(by_name)),
        );
        let parent_list = gtk::ListView::builder()
            .model(&gtk::SingleSelection::builder().model(&parent_sorted).autoselect(false).build())
            .factory(&row_factory())
            .css_classes(["column-list", "parent"])
            .build();
        let preview = Preview::new();

        let scroll = |child: &gtk::Widget| {
            gtk::ScrolledWindow::builder()
                .child(child)
                .hexpand(true)
                .vexpand(true)
                .build()
        };
        let columns = gtk::Paned::builder()
            .start_child(&scroll(parent_list.upcast_ref()))
            .end_child(
                &gtk::Paned::builder()
                    .start_child(&scroll(current.upcast_ref()))
                    .end_child(&preview.widget)
                    .position(350)
                    .shrink_end_child(false)
                    .build(),
            )
            .position(220)
            .build();
        let stack = gtk::Stack::builder().vexpand(true).build();
        stack.add_named(&scroll(list.upcast_ref()), Some("list"));
        stack.add_named(&scroll(grid.upcast_ref()), Some("grid"));
        stack.add_named(&columns, Some("columns"));

        let filter_entry = gtk::SearchEntry::builder()
            .placeholder_text(tr("files-filter"))
            .hexpand(true)
            .build();
        let filter_bar = gtk::Revealer::builder()
            .child(&crate::actions::padded("filter-bar", &filter_entry))
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .build();
        let status = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["pane-status"])
            .build();
        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["pane"])
            .build();
        widget.append(&filter_bar);
        widget.append(&stack);
        widget.append(&status);

        let pane = Rc::new(Self {
            widget,
            stack,
            store,
            filter,
            sorter,
            selection,
            list,
            grid,
            current,
            parent_store,
            parent_list,
            preview,
            status,
            filter_entry,
            filter_bar,
            location: RefCell::new(Location::Path(glib::home_dir())),
            back: RefCell::default(),
            forward: RefCell::default(),
            generation: Cell::new(0),
            monitor: RefCell::default(),
            reselect: RefCell::default(),
            show_hidden: Cell::new(false),
            sort: Cell::new((SortKey::Name, true)),
            view: Cell::new(View::List),
            loading: Cell::new(false),
            cursor: Cell::new(None),
            marking: Cell::new(false),
            on_open: RefCell::default(),
            on_moved: RefCell::default(),
            on_selected: RefCell::default(),
            on_menu: RefCell::default(),
        });
        pane.connect();
        pane.apply_filter();
        pane.apply_sort();
        pane
    }

    /// Called with a file (not a folder) to open.
    pub fn connect_open(&self, f: impl Fn(Entry) + 'static) {
        *self.on_open.borrow_mut() = Some(Box::new(f));
    }

    /// Called when the folder shown changes.
    pub fn connect_moved(&self, f: impl Fn(()) + 'static) {
        *self.on_moved.borrow_mut() = Some(Box::new(f));
    }

    /// Called when the entry under the cursor changes.
    pub fn connect_selected(&self, f: impl Fn(()) + 'static) {
        *self.on_selected.borrow_mut() = Some(Box::new(f));
    }

    fn selected_changed(&self) {
        if let Some(f) = self.on_selected.borrow().as_ref() {
            f(());
        }
    }

    /// Called for the context menu: the widget, and where.
    pub fn connect_menu(&self, f: impl Fn((gtk::Widget, f64, f64)) + 'static) {
        *self.on_menu.borrow_mut() = Some(Box::new(f));
    }

    fn connect(self: &Rc<Self>) {
        // Opening: a folder here, a file by whoever listens.
        for activate in [
            self.list.upcast_ref::<gtk::Widget>().clone(),
            self.grid.upcast_ref::<gtk::Widget>().clone(),
            self.current.upcast_ref::<gtk::Widget>().clone(),
        ] {
            let weak = Rc::downgrade(self);
            let handler = move |position: u32| {
                if let Some(p) = weak.upgrade()
                    && let Some(e) = p.entry_at(position)
                {
                    p.activate(&e);
                }
            };
            if let Some(v) = activate.downcast_ref::<gtk::ColumnView>() {
                let h = handler.clone();
                v.connect_activate(move |_, pos| h(pos));
            } else if let Some(v) = activate.downcast_ref::<gtk::GridView>() {
                let h = handler.clone();
                v.connect_activate(move |_, pos| h(pos));
            } else if let Some(v) = activate.downcast_ref::<gtk::ListView>() {
                v.connect_activate(move |_, pos| handler(pos));
            }
            // Right-click: the context menu.
            let click = gtk::GestureClick::builder().button(3).build();
            let weak = Rc::downgrade(self);
            let widget = activate.clone();
            click.connect_pressed(move |_, _, x, y| {
                if let Some(p) = weak.upgrade() {
                    p.select_under(&widget, x, y);
                    if let Some(f) = p.on_menu.borrow().as_ref() {
                        f((widget.clone(), x, y));
                    }
                }
            });
            activate.add_controller(click);
        }
        // The parent column: a click goes there.
        let weak = Rc::downgrade(self);
        self.parent_list.connect_activate(move |list, pos| {
            if let (Some(p), Some(e)) = (
                weak.upgrade(),
                list.model().and_then(|m| m.item(pos)).and_then(|o| entry_of(&o)),
            ) && e.dir
            {
                p.go(e.location.clone());
            }
        });

        // The preview follows the selection, a moment after it settles.
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        let weak = Rc::downgrade(self);
        self.selection.connect_selection_changed(move |_, _, _| {
            let Some(p) = weak.upgrade() else { return };
            // A click moves the cursor too.
            if !p.marking.get() {
                let set = p.selection.selection();
                p.cursor.set((!set.is_empty()).then(|| set.minimum()));
            }
            p.update_status();
            p.selected_changed();
            if p.view.get() != View::Columns {
                return;
            }
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let weak = Rc::downgrade(&p);
            let pending2 = pending.clone();
            let id = glib::timeout_add_local_once(Duration::from_millis(120), move || {
                pending2.borrow_mut().take();
                if let Some(p) = weak.upgrade() {
                    p.preview.show(p.focused().as_ref());
                }
            });
            *pending.borrow_mut() = Some(id);
        });

        let weak = Rc::downgrade(self);
        self.filter_entry.connect_search_changed(move |_| {
            if let Some(p) = weak.upgrade() {
                p.apply_filter();
            }
        });
        let weak = Rc::downgrade(self);
        self.filter_entry.connect_stop_search(move |_| {
            if let Some(p) = weak.upgrade() {
                p.close_filter();
            }
        });
        let weak = Rc::downgrade(self);
        self.filter_entry.connect_activate(move |_| {
            if let Some(p) = weak.upgrade() {
                p.focus_view();
            }
        });

        // Sorting from the list's headers.
        let weak = Rc::downgrade(self);
        if let Some(sorter) = self.list.sorter() {
            sorter.connect_changed(move |s, _| {
                let (Some(p), Some(s)) = (weak.upgrade(), s.downcast_ref::<gtk::ColumnViewSorter>()) else {
                    return;
                };
                let key = match s.primary_sort_column().and_then(|c| c.id()).as_deref() {
                    Some("size") => SortKey::Size,
                    Some("modified") => SortKey::Modified,
                    Some("kind") => SortKey::Kind,
                    _ => SortKey::Name,
                };
                p.set_sort(key, s.primary_sort_order() == gtk::SortType::Ascending);
            });
        }
    }

    // ------------------------------------------------------------ moving

    /// Shows `at` first, with no history before it.
    pub fn start(self: &Rc<Self>, at: Location) {
        *self.location.borrow_mut() = at;
        self.load(None);
    }

    pub fn go(self: &Rc<Self>, to: Location) {
        let from = self.location.replace(to.clone());
        if from != to {
            self.back.borrow_mut().push(from);
            self.forward.borrow_mut().clear();
        }
        self.load(None);
    }

    pub fn back(self: &Rc<Self>) {
        if let Some(to) = self.back.borrow_mut().pop() {
            let from = self.location.replace(to);
            self.forward.borrow_mut().push(from);
            self.load(None);
        }
    }

    pub fn forward(self: &Rc<Self>) {
        if let Some(to) = self.forward.borrow_mut().pop() {
            let from = self.location.replace(to);
            self.back.borrow_mut().push(from);
            self.load(None);
        }
    }

    /// Up a folder, the one left selected.
    pub fn up(self: &Rc<Self>) {
        let here = self.location.borrow().clone();
        if let Some(parent) = here.parent() {
            self.go(parent);
            *self.reselect.borrow_mut() = Some(here.name());
        }
    }

    pub fn reload(self: &Rc<Self>) {
        let keep = self.focused().map(|e| e.name);
        self.load(keep);
    }

    fn load(self: &Rc<Self>, keep: Option<String>) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        if keep.is_some() {
            *self.reselect.borrow_mut() = keep;
        }
        self.store.remove_all();
        self.marking.set(false);
        self.cursor.set(None);
        self.loading.set(true);
        self.update_status();
        let here = self.location.borrow().clone();
        self.watch(&here);
        self.load_parent(&here);
        if let Some(f) = self.on_moved.borrow().as_ref() {
            f(());
        }
        let rx = entry::list(here);
        let weak = Rc::downgrade(self);
        let started = std::time::Instant::now();
        glib::spawn_future_local(async move {
            while let Ok(message) = rx.recv().await {
                let Some(p) = weak.upgrade().filter(|p| p.generation.get() == generation) else {
                    return;
                };
                match message {
                    Listing::Batch(entries) => {
                        let objects: Vec<glib::BoxedAnyObject> =
                            entries.into_iter().map(glib::BoxedAnyObject::new).collect();
                        p.store.extend_from_slice(&objects);
                        p.update_status();
                    }
                    Listing::Done => {
                        log::debug!("listed {} entries in {:?}", p.store.n_items(), started.elapsed());
                        p.loading.set(false);
                        p.after_load();
                    }
                    Listing::Failed(err) => {
                        p.loading.set(false);
                        p.status.set_label(&tr_args("files-cannot-read", &[("reason", text(err))]));
                    }
                }
            }
        });
    }

    fn after_load(&self) {
        let wanted = self.reselect.borrow_mut().take();
        let position = wanted
            .and_then(|name| self.position_of(|e| e.name == name))
            .unwrap_or(0);
        if self.selection.n_items() > 0 {
            self.selection.select_item(position, true);
            self.scroll_to(position);
        }
        self.update_status();
        if self.view.get() == View::Columns {
            self.preview.show(self.focused().as_ref());
        }
    }

    fn load_parent(self: &Rc<Self>, here: &Location) {
        self.parent_store.remove_all();
        let Some(parent) = here.parent() else { return };
        let rx = entry::list(parent);
        let generation = self.generation.get();
        let weak = Rc::downgrade(self);
        let name = here.name();
        glib::spawn_future_local(async move {
            while let Ok(message) = rx.recv().await {
                let Some(p) = weak.upgrade().filter(|p| p.generation.get() == generation) else {
                    return;
                };
                match message {
                    Listing::Batch(entries) => {
                        let objects: Vec<glib::BoxedAnyObject> = entries
                            .into_iter()
                            .filter(|e| e.dir && (p.show_hidden.get() || !e.hidden))
                            .map(glib::BoxedAnyObject::new)
                            .collect();
                        p.parent_store.extend_from_slice(&objects);
                    }
                    _ => {
                        // Mark the folder we are in.
                        if let Some(model) = p.parent_list.model() {
                            for n in 0..model.n_items() {
                                if model.item(n).and_then(|o| entry_of(&o)).is_some_and(|e| e.name == name) {
                                    model.select_item(n, true);
                                    p.parent_list.scroll_to(n, gtk::ListScrollFlags::NONE, None);
                                    break;
                                }
                            }
                        }
                        return;
                    }
                }
            }
        });
    }

    /// Follows changes to the folder, entry by entry.
    fn watch(self: &Rc<Self>, here: &Location) {
        *self.monitor.borrow_mut() = None;
        let Ok(monitor) = here
            .file()
            .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        else {
            return;
        };
        let weak = Rc::downgrade(self);
        monitor.connect_changed(move |_, file, other, event| {
            let Some(p) = weak.upgrade() else { return };
            use gio::FileMonitorEvent as E;
            match event {
                E::Created | E::MovedIn => p.added(file),
                E::Deleted | E::MovedOut => p.removed(file),
                E::Renamed => {
                    p.removed(file);
                    if let Some(new) = other {
                        p.added(new);
                    }
                }
                E::ChangesDoneHint | E::AttributeChanged => {
                    p.removed(file);
                    p.added(file);
                }
                _ => {}
            }
        });
        *self.monitor.borrow_mut() = Some(monitor);
    }

    fn added(&self, file: &gio::File) {
        let name = file.basename().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if self.store_position(&name).is_some() {
            return;
        }
        let Ok(info) = file.query_info(
            "standard::type,standard::size,standard::is-symlink,time::modified",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        ) else {
            return;
        };
        let dir = info.file_type() == gio::FileType::Directory;
        self.store.append(&glib::BoxedAnyObject::new(Entry {
            hidden: name.starts_with('.'),
            mime: if dir { "inode/directory".into() } else { entry::mime_of(&name) },
            name,
            location: Location::of(file),
            dir,
            link: info.is_symlink(),
            size: info.size().max(0) as u64,
            modified: info.modification_date_time().map_or(0, |d| d.to_unix()),
        }));
        self.update_status();
    }

    fn removed(&self, file: &gio::File) {
        let name = file.basename().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(n) = self.store_position(&name) {
            self.store.remove(n);
            self.update_status();
        }
    }

    fn store_position(&self, name: &str) -> Option<u32> {
        (0..self.store.n_items()).find(|&n| {
            self.store.item(n).and_then(|o| entry_of(&o)).is_some_and(|e| e.name == name)
        })
    }

    fn activate(self: &Rc<Self>, e: &Entry) {
        if e.dir {
            self.go(e.location.clone());
        } else if let Some(f) = self.on_open.borrow().as_ref() {
            f(e.clone());
        }
    }

    pub fn open_focused(self: &Rc<Self>) {
        if let Some(e) = self.focused() {
            self.activate(&e);
        }
    }

    // ---------------------------------------------------------- selection

    pub fn entry_at(&self, position: u32) -> Option<Entry> {
        self.selection.item(position).and_then(|o| entry_of(&o))
    }

    /// The selected entries.
    pub fn selected(&self) -> Vec<Entry> {
        let set = self.selection.selection();
        let mut out = Vec::new();
        if let Some((iter, first)) = gtk::BitsetIter::init_first(&set) {
            out.extend(self.entry_at(first));
            for n in iter {
                out.extend(self.entry_at(n));
            }
        }
        out
    }

    /// The entry the cursor is on.
    pub fn focused(&self) -> Option<Entry> {
        self.focused_position().and_then(|p| self.entry_at(p))
    }

    fn focused_position(&self) -> Option<u32> {
        self.cursor.get().filter(|&p| p < self.selection.n_items()).or_else(|| {
            let set = self.selection.selection();
            (!set.is_empty()).then(|| set.minimum())
        })
    }

    fn position_of(&self, f: impl Fn(&Entry) -> bool) -> Option<u32> {
        (0..self.selection.n_items()).find(|&n| self.entry_at(n).is_some_and(|e| f(&e)))
    }

    /// Moves the cursor by `by` (Vim's j and k), or to an end.
    pub fn move_cursor(&self, by: i64) {
        let n = self.selection.n_items() as i64;
        if n == 0 {
            return;
        }
        let at = self.focused_position().map_or(0, |p| p as i64);
        let to = match by {
            i64::MIN => 0,
            i64::MAX => n - 1,
            _ => (at + by).clamp(0, n - 1),
        } as u32;
        self.cursor.set(Some(to));
        if !self.marking.get() {
            self.selection.select_item(to, true);
        }
        self.scroll_to(to);
        if self.view.get() == View::Columns {
            self.preview.show(self.focused().as_ref());
        }
        self.selected_changed();
    }

    /// Space: marks (or unmarks) the entry under the cursor and moves on.
    pub fn toggle_selected(&self) {
        let Some(at) = self.focused_position() else { return };
        if !self.marking.get() {
            // The first mark: what was merely selected is not marked.
            self.marking.set(true);
            self.selection.unselect_all();
        }
        if self.selection.is_selected(at) {
            self.selection.unselect_item(at);
        } else {
            self.selection.select_item(at, false);
        }
        let next = (at + 1).min(self.selection.n_items().saturating_sub(1));
        self.cursor.set(Some(next));
        self.scroll_to(next);
    }

    /// Escape: no marks, the cursor's entry selected alone.
    pub fn clear_marks(&self) {
        self.marking.set(false);
        if let Some(at) = self.focused_position() {
            self.selection.select_item(at, true);
        }
    }

    pub fn select_all(&self) {
        self.selection.select_all();
    }

    fn scroll_to(&self, position: u32) {
        match self.view.get() {
            View::List => self.list.scroll_to(position, None, gtk::ListScrollFlags::FOCUS, None),
            View::Grid => self.grid.scroll_to(position, gtk::ListScrollFlags::FOCUS, None),
            View::Columns => self.current.scroll_to(position, gtk::ListScrollFlags::FOCUS, None),
        }
    }

    /// Right-click on an unselected entry selects it alone.
    fn select_under(&self, widget: &gtk::Widget, x: f64, y: f64) {
        let Some(picked) = widget.pick(x, y, gtk::PickFlags::DEFAULT) else { return };
        let mut node = Some(picked);
        while let Some(w) = node {
            if let Some(position) = position_of_cell(&w)
                && !self.selection.is_selected(position)
            {
                self.selection.select_item(position, true);
                return;
            }
            if &w == widget {
                return;
            }
            node = w.parent();
        }
    }

    pub fn focus_view(&self) {
        match self.view.get() {
            View::List => self.list.grab_focus(),
            View::Grid => self.grid.grab_focus(),
            View::Columns => self.current.grab_focus(),
        };
    }

    // ------------------------------------------------- view, sort, filter

    pub fn view(&self) -> View {
        self.view.get()
    }

    pub fn set_view(&self, view: View) {
        self.view.set(view);
        self.stack.set_visible_child_name(match view {
            View::List => "list",
            View::Grid => "grid",
            View::Columns => "columns",
        });
        if view == View::Columns {
            self.preview.show(self.focused().as_ref());
        }
        if let Some(p) = self.focused_position() {
            self.scroll_to(p);
        }
        self.focus_view();
    }

    pub fn set_hidden(self: &Rc<Self>, shown: bool) {
        self.show_hidden.set(shown);
        self.apply_filter();
        let here = self.location.borrow().clone();
        self.load_parent(&here);
    }

    pub fn set_sort(&self, key: SortKey, ascending: bool) {
        self.sort.set((key, ascending));
        self.apply_sort();
    }

    fn apply_sort(&self) {
        let (key, ascending) = self.sort.get();
        self.sorter.set_sort_func(move |a, b| {
            let (Some(a), Some(b)) = (entry_of(a), entry_of(b)) else {
                return gtk::Ordering::Equal;
            };
            // Folders first, always.
            if a.dir != b.dir {
                return if a.dir { gtk::Ordering::Smaller } else { gtk::Ordering::Larger };
            }
            let order = match key {
                SortKey::Name => std::cmp::Ordering::Equal,
                SortKey::Size => a.size.cmp(&b.size),
                SortKey::Modified => a.modified.cmp(&b.modified),
                SortKey::Kind => a.mime.cmp(&b.mime),
            }
            .then_with(|| orbit_common::natural::natural(&a.name.to_lowercase(), &b.name.to_lowercase()));
            (if ascending { order } else { order.reverse() }).into()
        });
    }

    fn apply_filter(&self) {
        let hidden = self.show_hidden.get();
        let wanted = self.filter_entry.text().to_lowercase();
        self.filter.set_filter_func(move |o| {
            entry_of(o).is_some_and(|e| {
                (hidden || !e.hidden) && (wanted.is_empty() || e.name.to_lowercase().contains(&wanted))
            })
        });
        self.update_status();
    }

    pub fn open_filter(&self) {
        self.filter_bar.set_reveal_child(true);
        self.filter_entry.grab_focus();
    }

    pub fn close_filter(&self) {
        self.filter_entry.set_text("");
        self.filter_bar.set_reveal_child(false);
        self.focus_view();
    }

    pub fn filtering(&self) -> bool {
        self.filter_bar.reveals_child()
    }

    fn update_status(&self) {
        let shown = self.selection.n_items();
        let selected = self.selected();
        let text = if self.loading.get() {
            tr_args("files-status-loading", &[("n", number(shown as f64))])
        } else if selected.len() > 1 {
            let bytes: u64 = selected.iter().filter(|e| !e.dir).map(|e| e.size).sum();
            tr_args(
                "files-status-selected",
                &[
                    ("n", number(selected.len() as f64)),
                    ("total", number(shown as f64)),
                    ("size", text(entry::size_text(bytes))),
                ],
            )
        } else {
            tr_args("files-status-items", &[("n", number(shown as f64))])
        };
        self.status.set_label(&text);
    }
}

pub fn entry_of(object: &glib::Object) -> Option<Entry> {
    object
        .downcast_ref::<glib::BoxedAnyObject>()
        .map(|b| b.borrow::<Entry>().clone())
}

fn by_name(a: &glib::Object, b: &glib::Object) -> gtk::Ordering {
    match (entry_of(a), entry_of(b)) {
        (Some(a), Some(b)) => orbit_common::natural::natural(&a.name.to_lowercase(), &b.name.to_lowercase()).into(),
        _ => gtk::Ordering::Equal,
    }
}

/// The position a list cell stands for now: its list item, stored on it
/// when bound, knows — positions shift as the folder changes.
fn position_of_cell(widget: &gtk::Widget) -> Option<u32> {
    let item = unsafe { widget.data::<glib::WeakRef<gtk::ListItem>>("orbit-item") }
        .and_then(|w| unsafe { w.as_ref() }.upgrade())?;
    Some(item.position()).filter(|&p| p != gtk::INVALID_LIST_POSITION)
}

fn remember_position(widget: &impl IsA<gtk::Widget>, item: &gtk::ListItem) {
    unsafe {
        widget.set_data("orbit-item", item.downgrade());
    }
}

// ------------------------------------------------------------ the views

/// An icon and a name, a row of a list.
fn row_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        let row = gtk::Box::builder().spacing(10).css_classes(["entry-row"]).build();
        row.append(&gtk::Image::builder().pixel_size(16).build());
        row.append(&gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).build());
        item.set_child(Some(&row));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        let (Some(row), Some(e)) = (item.child(), item.item().and_then(|o| entry_of(&o))) else {
            return;
        };
        remember_position(&row, item);
        if let Some(image) = row.first_child().and_downcast::<gtk::Image>() {
            image.set_from_gicon(&entry::icon(&e));
        }
        if let Some(label) = row.last_child().and_downcast::<gtk::Label>() {
            label.set_label(&e.name);
            label.set_opacity(if e.hidden { 0.6 } else { 1.0 });
        }
    });
    factory
}

fn column_view(selection: &gtk::MultiSelection) -> gtk::ColumnView {
    let view = gtk::ColumnView::builder()
        .model(selection)
        .enable_rubberband(true)
        .reorderable(false)
        .css_classes(["files-list"])
        .build();
    let text_column = |id: &str, title: &str, f: fn(&Entry) -> String, expand: bool| {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
            item.set_child(Some(&gtk::Label::builder().xalign(0.0).css_classes(["dim"]).build()));
        });
        factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
            if let (Some(label), Some(e)) = (
                item.child().and_downcast::<gtk::Label>(),
                item.item().and_then(|o| entry_of(&o)),
            ) {
                remember_position(&label, item);
                label.set_label(&f(&e));
            }
        });
        gtk::ColumnViewColumn::builder()
            .id(id)
            .title(title)
            .factory(&factory)
            .resizable(true)
            .expand(expand)
            .sorter(&gtk::CustomSorter::new(|_, _| gtk::Ordering::Equal))
            .build()
    };
    let name = gtk::ColumnViewColumn::builder()
        .id("name")
        .title(tr("files-column-name"))
        .factory(&row_factory())
        .resizable(true)
        .expand(true)
        .sorter(&gtk::CustomSorter::new(|_, _| gtk::Ordering::Equal))
        .build();
    view.append_column(&name);
    view.append_column(&text_column(
        "size",
        &tr("files-column-size"),
        |e| if e.dir { String::new() } else { entry::size_text(e.size) },
        false,
    ));
    view.append_column(&text_column("modified", &tr("files-column-modified"), |e| entry::date_text(e.modified), false));
    view.append_column(&text_column(
        "kind",
        &tr("files-column-kind"),
        |e| gio::content_type_get_description(&e.mime).to_string(),
        false,
    ));
    view.sort_by_column(Some(&name), gtk::SortType::Ascending);
    view
}

/// A large icon (or a thumbnail) over the name.
fn grid_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        let cell = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(5)
            .width_request(110)
            .css_classes(["grid-cell"])
            .build();
        cell.append(&gtk::Image::builder().pixel_size(64).build());
        cell.append(
            &gtk::Label::builder()
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .lines(2)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .justify(gtk::Justification::Center)
                .max_width_chars(14)
                .build(),
        );
        item.set_child(Some(&cell));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        let (Some(cell), Some(e)) = (item.child(), item.item().and_then(|o| entry_of(&o))) else {
            return;
        };
        remember_position(&cell, item);
        if let Some(image) = cell.first_child().and_downcast::<gtk::Image>() {
            image.set_from_gicon(&entry::icon(&e));
            thumbs::request(&e, &image);
        }
        if let Some(label) = cell.last_child().and_downcast::<gtk::Label>() {
            label.set_label(&e.name);
            label.set_opacity(if e.hidden { 0.6 } else { 1.0 });
        }
    });
    factory
}
