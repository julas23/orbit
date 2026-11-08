//! A pane: one folder, seen as a list (with columns), as icons, or as
//! Yazi's columns — the parent folder, the folder, and a preview of what
//! is selected. The three views share one model and one selection, so
//! switching keeps both.
//!
//! The folder fills in as its entries arrive (see `entry`); a monitor
//! keeps it current, entry by entry, without listing it again.
//!
//! The filter bar (F3) narrows the folder by name; with "Subfolders" it
//! searches everything under it instead, and with "Contents" the text
//! inside the files too — the results fill the same views, with a column
//! that says where each one is. "Computer" (see `computer`) is a page of
//! its own. Ctrl with the wheel, or with + and −, makes the icons larger
//! or smaller; the list's columns are chosen with a right-click on their
//! headers.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use gtk::{gdk, gio, glib, prelude::*};
use orbit_app::i18n::{number, text, tr, tr_args};

use crate::{
    computer::Computer,
    entry::{self, Entry, Listing, Location},
    preview::Preview,
    thumbs,
};

/// Icon sizes, by zoom: in the lists, and in the icon view.
pub const LIST_ZOOM: [i32; 5] = [16, 24, 32, 48, 64];
pub const GRID_ZOOM: [i32; 7] = [32, 48, 64, 96, 128, 192, 256];
/// Where each starts.
pub const ZOOM_DEFAULT: (usize, usize) = (0, 2);

/// The list's columns besides the name, in order; those shown unless
/// chosen otherwise.
pub const COLUMNS: [&str; 7] = ["size", "modified", "kind", "permissions", "owner", "group", "location"];
pub const COLUMNS_DEFAULT: [&str; 3] = ["size", "modified", "kind"];

/// What the filter bar does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Search {
    /// Narrows the folder.
    Filter,
    /// Searches the folders under it by name…
    Names,
    /// …or by what the files hold.
    Contents,
}

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
    Location,
    Permissions,
    Owner,
    Group,
}

impl SortKey {
    pub fn id(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Size => "size",
            SortKey::Modified => "modified",
            SortKey::Kind => "kind",
            SortKey::Location => "location",
            SortKey::Permissions => "permissions",
            SortKey::Owner => "owner",
            SortKey::Group => "group",
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "size" => SortKey::Size,
            "modified" => SortKey::Modified,
            "kind" => SortKey::Kind,
            "location" => SortKey::Location,
            "permissions" => SortKey::Permissions,
            "owner" => SortKey::Owner,
            "group" => SortKey::Group,
            _ => SortKey::Name,
        }
    }
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
    search_names: gtk::ToggleButton,
    search_contents: gtk::ToggleButton,
    computer: Rc<Computer>,

    pub location: RefCell<Location>,
    back: RefCell<Vec<Location>>,
    forward: RefCell<Vec<Location>>,
    generation: Cell<u64>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    /// Names the monitor saw change, looked at together a moment later.
    changed: RefCell<std::collections::HashSet<String>>,
    flush: RefCell<Option<glib::SourceId>>,
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
    /// Where a Shift range starts: the last entry clicked or moved to.
    anchor: Cell<Option<u32>>,
    /// Set while the selection is changed here, with the cursor already
    /// where it belongs.
    quiet: Cell<bool>,
    /// Showing what a search found, not the folder.
    searching: Cell<bool>,
    /// Stops the search running.
    search_stop: RefCell<Option<Arc<AtomicBool>>>,
    /// A search starts a moment after the typing stops.
    search_wait: RefCell<Option<glib::SourceId>>,
    /// Set while the bar is emptied here, not by typing.
    resetting: Cell<bool>,
    /// The zoom of the lists and of the icon view (indexes).
    zoom: Cell<(usize, usize)>,
    /// The columns chosen, besides the name.
    columns: RefCell<Vec<String>>,
    on_open: Callback<Entry>,
    on_moved: Callback<()>,
    on_selected: Callback<()>,
    on_menu: Callback<(gtk::Widget, f64, f64)>,
    on_columns_menu: Callback<(gtk::Widget, f64, f64)>,
    on_zoom: Callback<i32>,
    on_dropped: Callback<(Vec<PathBuf>, PathBuf, bool)>,
    /// The folder a drag hovers, lit.
    drop_row: RefCell<Option<gtk::Widget>>,
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
        let computer = Computer::new();
        let current = gtk::ListView::builder()
            .model(&selection)
            .factory(&row_factory())
            .css_classes(["column-list"])
            .build();
        let parent_store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let parent_sorted = gtk::SortListModel::new(Some(parent_store.clone()), Some(gtk::CustomSorter::new(by_name)));
        let parent_list = gtk::ListView::builder()
            .model(
                &gtk::SingleSelection::builder()
                    .model(&parent_sorted)
                    .autoselect(false)
                    .build(),
            )
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
        stack.add_named(&computer.widget, Some("computer"));

        let filter_entry = gtk::SearchEntry::builder()
            .placeholder_text(tr("files-filter"))
            .hexpand(true)
            .build();
        let search_names = gtk::ToggleButton::builder()
            .label(tr("files-search-subfolders"))
            .tooltip_text(tr("files-search-subfolders-tip"))
            .focus_on_click(false)
            .build();
        let search_contents = gtk::ToggleButton::builder()
            .label(tr("files-search-contents"))
            .tooltip_text(tr("files-search-contents-tip"))
            .focus_on_click(false)
            .build();
        let bar_row = gtk::Box::builder().spacing(5).build();
        bar_row.append(&filter_entry);
        bar_row.append(&search_names);
        bar_row.append(&search_contents);
        let filter_bar = gtk::Revealer::builder()
            .child(&crate::actions::padded("filter-bar", &bar_row))
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
            search_names,
            search_contents,
            computer,
            location: RefCell::new(Location::Path(glib::home_dir())),
            back: RefCell::default(),
            forward: RefCell::default(),
            generation: Cell::new(0),
            monitor: RefCell::default(),
            changed: RefCell::default(),
            flush: RefCell::default(),
            reselect: RefCell::default(),
            show_hidden: Cell::new(false),
            sort: Cell::new((SortKey::Name, true)),
            view: Cell::new(View::List),
            loading: Cell::new(false),
            cursor: Cell::new(None),
            marking: Cell::new(false),
            anchor: Cell::new(None),
            quiet: Cell::new(false),
            searching: Cell::new(false),
            search_stop: RefCell::default(),
            search_wait: RefCell::default(),
            resetting: Cell::new(false),
            zoom: Cell::new(ZOOM_DEFAULT),
            columns: RefCell::new(COLUMNS_DEFAULT.iter().map(|c| c.to_string()).collect()),
            on_open: RefCell::default(),
            on_moved: RefCell::default(),
            on_selected: RefCell::default(),
            on_menu: RefCell::default(),
            on_columns_menu: RefCell::default(),
            on_zoom: RefCell::default(),
            on_dropped: RefCell::default(),
            drop_row: RefCell::default(),
        });
        pane.connect();
        pane.apply_filter();
        pane.apply_sort();
        pane.apply_zoom();
        pane.apply_columns();
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

    /// Called for the columns' menu (a right-click on the list's headers).
    pub fn connect_columns_menu(&self, f: impl Fn((gtk::Widget, f64, f64)) + 'static) {
        *self.on_columns_menu.borrow_mut() = Some(Box::new(f));
    }

    /// Called with +1 or −1: Ctrl with the wheel.
    pub fn connect_zoom(&self, f: impl Fn(i32) + 'static) {
        *self.on_zoom.borrow_mut() = Some(Box::new(f));
    }

    /// Called with files dropped here: what, the folder they go to, and
    /// whether they are copied (or else moved).
    pub fn connect_dropped(&self, f: impl Fn((Vec<PathBuf>, PathBuf, bool)) + 'static) {
        *self.on_dropped.borrow_mut() = Some(Box::new(f));
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
            // Taken before the view's own handling, which would make the
            // entry the only one selected.
            let click = gtk::GestureClick::builder().button(3).build();
            click.set_propagation_phase(gtk::PropagationPhase::Capture);
            let weak = Rc::downgrade(self);
            let widget = activate.clone();
            click.connect_pressed(move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                if let Some(p) = weak.upgrade() {
                    if on_header(&widget, x, y) {
                        if let Some(f) = p.on_columns_menu.borrow().as_ref() {
                            f((widget.clone(), x, y));
                        }
                        return;
                    }
                    p.select_under(&widget, x, y);
                    if let Some(f) = p.on_menu.borrow().as_ref() {
                        f((widget.clone(), x, y));
                    }
                }
            });
            activate.add_controller(click);
            // Ctrl+click and Shift+click, taken before the view's own
            // handling, so they mean the same in every view.
            let click = gtk::GestureClick::builder().button(1).build();
            click.set_propagation_phase(gtk::PropagationPhase::Capture);
            let weak = Rc::downgrade(self);
            let widget = activate.clone();
            click.connect_pressed(move |gesture, _, x, y| {
                let state = gesture.current_event_state();
                let ctrl = state.contains(gtk::gdk::ModifierType::CONTROL_MASK);
                let shift = state.contains(gtk::gdk::ModifierType::SHIFT_MASK);
                let Some(p) = weak.upgrade() else { return };
                if !(ctrl || shift) {
                    // A plain click on no entry: nothing selected.
                    if on_empty(&widget, x, y) {
                        p.marking.set(false);
                        p.selection.unselect_all();
                        p.focus_view();
                    }
                    return;
                }
                let Some(position) = position_under(&widget, x, y) else {
                    return;
                };
                gesture.set_state(gtk::EventSequenceState::Claimed);
                if shift {
                    p.select_range_to(position, ctrl);
                } else {
                    p.toggle_at(position);
                }
                p.focus_view();
            });
            activate.add_controller(click);
            // Ctrl with the wheel: larger or smaller.
            let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
            scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
            let weak = Rc::downgrade(self);
            scroll.connect_scroll(move |c, _, dy| {
                if !c.current_event_state().contains(gtk::gdk::ModifierType::CONTROL_MASK) || dy == 0.0 {
                    return glib::Propagation::Proceed;
                }
                if let Some(p) = weak.upgrade()
                    && let Some(f) = p.on_zoom.borrow().as_ref()
                {
                    f(if dy < 0.0 { 1 } else { -1 });
                }
                glib::Propagation::Stop
            });
            activate.add_controller(scroll);
            self.connect_dnd(&activate);
        }
        let weak = Rc::downgrade(self);
        self.computer.connect_go(move |to| {
            if let Some(p) = weak.upgrade() {
                p.go(to);
            }
        });
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
            if !p.marking.get() && !p.quiet.get() {
                let set = p.selection.selection();
                p.cursor.set((!set.is_empty()).then(|| set.minimum()));
                p.anchor.set(p.cursor.get());
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
            if let Some(p) = weak.upgrade()
                && !p.resetting.get()
            {
                p.text_changed();
            }
        });
        // "Contents" searches the subfolders too; without "Subfolders",
        // not the contents either.
        let weak = Rc::downgrade(self);
        self.search_names.connect_toggled(move |b| {
            let Some(p) = weak.upgrade().filter(|p| !p.resetting.get()) else {
                return;
            };
            if !b.is_active() && p.search_contents.is_active() {
                p.search_contents.set_active(false);
                return;
            }
            p.mode_changed();
        });
        let weak = Rc::downgrade(self);
        self.search_contents.connect_toggled(move |b| {
            let Some(p) = weak.upgrade().filter(|p| !p.resetting.get()) else {
                return;
            };
            if b.is_active() && !p.search_names.is_active() {
                p.search_names.set_active(true);
                return;
            }
            p.mode_changed();
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
                let key = SortKey::from_id(
                    s.primary_sort_column()
                        .and_then(|c| c.id())
                        .as_deref()
                        .unwrap_or("name"),
                );
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
        // Somewhere else: the search (or the filter) is over.
        self.end_search();
        if self.filtering() {
            self.resetting.set(true);
            self.filter_entry.set_text("");
            self.resetting.set(false);
            self.filter_bar.set_reveal_child(false);
            self.apply_filter();
        }
        self.store.remove_all();
        self.marking.set(false);
        self.cursor.set(None);
        self.anchor.set(None);
        let here = self.location.borrow().clone();
        self.show_page();
        if here == Location::Computer {
            *self.monitor.borrow_mut() = None;
            self.parent_store.remove_all();
            self.computer.fill();
            self.loading.set(false);
            self.update_status();
            if let Some(f) = self.on_moved.borrow().as_ref() {
                f(());
            }
            return;
        }
        self.loading.set(true);
        self.update_status();
        self.watch(&here);
        self.load_parent(&here);
        if let Some(f) = self.on_moved.borrow().as_ref() {
            f(());
        }
        let rx = entry::list(here);
        self.fill_from(rx, generation, false);
    }

    /// Fills the store from `rx` — a folder's listing, or what a search
    /// finds.
    fn fill_from(self: &Rc<Self>, rx: async_channel::Receiver<Listing>, generation: u64, search: bool) {
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
                        if search {
                            p.update_status();
                        } else {
                            p.after_load();
                        }
                        return;
                    }
                    Listing::Failed(err) => {
                        p.loading.set(false);
                        p.status
                            .set_label(&tr_args("files-cannot-read", &[("reason", text(err))]));
                    }
                }
            }
        });
    }

    fn after_load(&self) {
        // Nothing selected in a folder just opened; going up, the folder
        // left; reloading, what was selected.
        let wanted = self.reselect.borrow_mut().take();
        if let Some(position) = wanted.and_then(|name| self.position_of(|e| e.name == name)) {
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
        if *here == Location::Computer {
            return;
        }
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
            if !matches!(
                event,
                E::Created
                    | E::MovedIn
                    | E::Deleted
                    | E::MovedOut
                    | E::Renamed
                    | E::ChangesDoneHint
                    | E::AttributeChanged
            ) {
                return;
            }
            p.changed_name(file);
            if let Some(o) = other.filter(|_| event == E::Renamed) {
                p.changed_name(o);
            }
        });
        *self.monitor.borrow_mut() = Some(monitor);
    }

    /// A name to look at again. A copy into the folder changes its files
    /// hundreds of times a second: they are gathered for a moment and
    /// looked at on a thread — over NFS a look at a file being written
    /// waits for it, and the window must not.
    fn changed_name(self: &Rc<Self>, file: &gio::File) {
        let Some(name) = file.basename().map(|n| n.to_string_lossy().into_owned()) else {
            return;
        };
        self.changed.borrow_mut().insert(name);
        if self.flush.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(250), move || {
            if let Some(p) = weak.upgrade() {
                p.flush.borrow_mut().take();
                p.apply_changes();
            }
        });
        *self.flush.borrow_mut() = Some(id);
    }

    fn apply_changes(self: &Rc<Self>) {
        let names: Vec<String> = self.changed.borrow_mut().drain().collect();
        // Search results are not the folder's entries.
        if self.searching.get() {
            return;
        }
        let here = self.location.borrow().clone();
        let generation = self.generation.get();
        let (tx, rx) = async_channel::bounded(1);
        match &here {
            Location::Path(dir) => {
                let dir = dir.clone();
                std::thread::Builder::new()
                    .name("orbit-files-changes".into())
                    .spawn(move || {
                        let found: Vec<(String, Option<Entry>)> = names
                            .into_iter()
                            .map(|n| {
                                let e = entry::entry_for(&dir.join(&n));
                                (n, e)
                            })
                            .collect();
                        let _ = tx.send_blocking(found);
                    })
                    .ok();
            }
            // The trash, a share GIO alone reaches: just listed again.
            Location::Uri(_) | Location::Computer => {
                self.reload();
                return;
            }
        }
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok(found) = rx.recv().await else { return };
            let Some(p) = weak.upgrade().filter(|p| p.generation.get() == generation) else {
                return;
            };
            let mut positions: std::collections::HashMap<String, u32> = (0..p.store.n_items())
                .filter_map(|n| p.store.item(n).and_then(|o| entry_of(&o)).map(|e| (e.name, n)))
                .collect();
            let mut gone: Vec<u32> = Vec::new();
            for (name, entry) in found {
                match (positions.remove(&name), entry) {
                    // Changed: in place, so the selection stays.
                    (Some(n), Some(e)) => {
                        if let Some(o) = p.store.item(n).and_downcast::<glib::BoxedAnyObject>() {
                            *o.borrow_mut::<Entry>() = e;
                            p.store.items_changed(n, 1, 1);
                        }
                    }
                    (Some(n), None) => gone.push(n),
                    (None, Some(e)) => p.store.append(&glib::BoxedAnyObject::new(e)),
                    (None, None) => {}
                }
            }
            gone.sort_unstable();
            for n in gone.into_iter().rev() {
                p.store.remove(n);
            }
            p.update_status();
        });
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

    // ------------------------------------------------- drag and drop

    /// Dragging and dropping in a view. A drag starts on an entry — the
    /// selection with it when it is selected, else it alone — and leaves
    /// the empty space to the rubberband; taken before the view's own
    /// handling, which would rubberband from an entry too. A drop goes
    /// into the folder under the pointer, or else into the one shown:
    /// moved, or copied with Ctrl.
    fn connect_dnd(self: &Rc<Self>, view: &gtk::Widget) {
        let source = gtk::DragSource::builder()
            .actions(gdk::DragAction::COPY | gdk::DragAction::MOVE)
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        let weak = Rc::downgrade(self);
        let widget = view.clone();
        source.connect_prepare(move |source, x, y| {
            let p = weak.upgrade()?;
            let position = position_under(&widget, x, y)?;
            if !p.selection.is_selected(position) {
                p.selection.select_item(position, true);
            }
            let files: Vec<gio::File> = p.selected().iter().map(|e| e.location.file()).collect();
            if files.is_empty() {
                return None;
            }
            if let Some(row) = row_under(&widget, x, y) {
                let at = widget
                    .compute_point(&row, &gtk::graphene::Point::new(x as f32, y as f32))
                    .unwrap_or_else(|| gtk::graphene::Point::new(0.0, 0.0));
                source.set_icon(
                    Some(&gtk::WidgetPaintable::new(Some(&row))),
                    at.x() as i32,
                    at.y() as i32,
                );
            }
            Some(gdk::ContentProvider::for_value(
                &gdk::FileList::from_array(&files).to_value(),
            ))
        });
        view.add_controller(source);

        let target = gtk::DropTarget::new(
            gdk::FileList::static_type(),
            gdk::DragAction::COPY | gdk::DragAction::MOVE,
        );
        // The files are known while hovering, to say copy or move.
        target.set_preload(true);
        let weak = Rc::downgrade(self);
        let widget = view.clone();
        target.connect_motion(move |t, x, y| {
            let Some(p) = weak.upgrade() else {
                return gdk::DragAction::empty();
            };
            let (to, row) = p.drop_place(&widget, x, y);
            let Some(to) = to else {
                p.light_drop(None);
                return gdk::DragAction::empty();
            };
            let Some(paths) = t
                .value()
                .and_then(|v| v.get::<gdk::FileList>().ok())
                .map(|l| paths_of(&l))
            else {
                // Not read yet.
                p.light_drop(row);
                return gdk::DragAction::COPY;
            };
            match drop_kind(&paths, &to, t.current_event_state()) {
                Some(copy) => {
                    p.light_drop(row);
                    if copy {
                        gdk::DragAction::COPY
                    } else {
                        gdk::DragAction::MOVE
                    }
                }
                None => {
                    p.light_drop(None);
                    gdk::DragAction::empty()
                }
            }
        });
        let weak = Rc::downgrade(self);
        target.connect_leave(move |_| {
            if let Some(p) = weak.upgrade() {
                p.light_drop(None);
            }
        });
        let weak = Rc::downgrade(self);
        let widget = view.clone();
        target.connect_drop(move |t, value, x, y| {
            let Some(p) = weak.upgrade() else { return false };
            p.light_drop(None);
            let (Ok(list), (Some(to), _)) = (value.get::<gdk::FileList>(), p.drop_place(&widget, x, y)) else {
                return false;
            };
            let paths = paths_of(&list);
            let Some(copy) = drop_kind(&paths, &to, t.current_event_state()) else {
                return false;
            };
            let paths: Vec<PathBuf> = paths.into_iter().filter(|s| s.parent() != Some(to.as_path())).collect();
            if let Some(f) = p.on_dropped.borrow().as_ref() {
                f((paths, to, copy));
            }
            true
        });
        view.add_controller(target);
    }

    /// Where a drop at (x, y) goes: the folder under it (with its row),
    /// or the one shown.
    fn drop_place(&self, view: &gtk::Widget, x: f64, y: f64) -> (Option<PathBuf>, Option<gtk::Widget>) {
        if let Some(e) = position_under(view, x, y).and_then(|n| self.entry_at(n))
            && e.dir
            && let Some(path) = e.location.path()
        {
            return (Some(path.to_path_buf()), row_under(view, x, y));
        }
        (self.location.borrow().path().map(Path::to_path_buf), None)
    }

    fn light_drop(&self, row: Option<gtk::Widget>) {
        let mut lit = self.drop_row.borrow_mut();
        if *lit == row {
            return;
        }
        if let Some(old) = lit.take() {
            old.remove_css_class("drop-into");
        }
        if let Some(new) = &row {
            new.add_css_class("drop-into");
        }
        *lit = row;
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
        // Nothing selected yet: the first key lands on the first entry.
        let to = match (by, self.focused_position()) {
            (i64::MIN, _) => 0,
            (i64::MAX, _) => n - 1,
            (_, None) => 0,
            (_, Some(at)) => (at as i64 + by).clamp(0, n - 1),
        } as u32;
        self.cursor.set(Some(to));
        self.anchor.set(Some(to));
        if !self.marking.get() {
            self.selection.select_item(to, true);
        }
        self.moved_to(to);
    }

    /// Shift with the arrows: the cursor moves by `by` (or to an end) and
    /// the selection runs from the anchor to it.
    pub fn extend_cursor(&self, by: i64) {
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
        self.select_range_to(to, false);
        self.moved_to(to);
    }

    /// Selects from the anchor to `to` — instead of the selection, or, with
    /// `add`, besides it — and puts the cursor on `to`.
    pub fn select_range_to(&self, to: u32, add: bool) {
        let from = self.anchor.get().or(self.focused_position()).unwrap_or(to);
        let (first, last) = (from.min(to), from.max(to));
        self.marking.set(false);
        self.quiet.set(true);
        self.selection.select_range(first, last - first + 1, !add);
        self.quiet.set(false);
        self.anchor.set(Some(from));
        self.cursor.set(Some(to));
        self.update_status();
        self.selected_changed();
    }

    /// Ctrl+click and Ctrl+Space: `position` in or out of the selection,
    /// the rest left as it is; the cursor goes there.
    pub fn toggle_at(&self, position: u32) {
        self.marking.set(false);
        self.quiet.set(true);
        if self.selection.is_selected(position) {
            self.selection.unselect_item(position);
        } else {
            self.selection.select_item(position, false);
        }
        self.quiet.set(false);
        self.cursor.set(Some(position));
        self.anchor.set(Some(position));
        self.update_status();
        self.selected_changed();
    }

    /// Ctrl+Space: the entry under the cursor in or out of the selection.
    pub fn toggle_cursor(&self) {
        if let Some(at) = self.focused_position() {
            self.toggle_at(at);
        }
    }

    /// Ctrl+I: what was selected is not, and the rest is.
    pub fn invert_selection(&self) {
        let n = self.selection.n_items();
        if n == 0 {
            return;
        }
        let all = gtk::Bitset::new_range(0, n);
        let inverted = all.copy();
        inverted.subtract(&self.selection.selection());
        self.marking.set(false);
        self.selection.set_selection(&inverted, &all);
    }

    /// After the cursor moved: in view, and the preview follows.
    fn moved_to(&self, to: u32) {
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

    /// Right-click: on an unselected entry, it alone; on a selected one,
    /// the selection as it is; on no entry, none — the folder's own menu.
    fn select_under(&self, widget: &gtk::Widget, x: f64, y: f64) {
        match position_under(widget, x, y) {
            Some(position) if self.selection.is_selected(position) => {}
            Some(position) => {
                self.selection.select_item(position, true);
            }
            None => {
                self.selection.unselect_all();
            }
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
        self.show_page();
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

    /// The view's page — or Computer's, there, whatever the view.
    fn show_page(&self) {
        self.stack
            .set_visible_child_name(if *self.location.borrow() == Location::Computer {
                "computer"
            } else {
                match self.view.get() {
                    View::List => "list",
                    View::Grid => "grid",
                    View::Columns => "columns",
                }
            });
    }

    pub fn set_sort(&self, key: SortKey, ascending: bool) {
        self.sort.set((key, ascending));
        self.apply_sort();
    }

    pub fn sort(&self) -> (SortKey, bool) {
        self.sort.get()
    }

    /// Sorts by `key` through the list's headers, so they show it.
    pub fn sort_by(&self, key: SortKey, ascending: bool) {
        let columns = self.list.columns();
        let column = (0..columns.n_items())
            .filter_map(|n| columns.item(n).and_downcast::<gtk::ColumnViewColumn>())
            .find(|c| c.id().as_deref() == Some(key.id()));
        match column {
            Some(c) => {
                if !c.is_visible() && key != SortKey::Name {
                    c.set_visible(true);
                }
                self.list.sort_by_column(
                    Some(&c),
                    if ascending {
                        gtk::SortType::Ascending
                    } else {
                        gtk::SortType::Descending
                    },
                );
            }
            None => self.set_sort(key, ascending),
        }
    }

    // --------------------------------------------------------- zoom

    pub fn set_zoom(&self, zoom: (usize, usize)) {
        self.zoom
            .set((zoom.0.min(LIST_ZOOM.len() - 1), zoom.1.min(GRID_ZOOM.len() - 1)));
        self.apply_zoom();
    }

    /// Zoom `by` steps in the view shown (the lists share one).
    pub fn zoomed(&self, by: i32) -> (usize, usize) {
        let (list, grid) = self.zoom.get();
        let step = |at: usize, len: usize| (at as i32 + by).clamp(0, len as i32 - 1) as usize;
        if self.view.get() == View::Grid {
            (list, step(grid, GRID_ZOOM.len()))
        } else {
            (step(list, LIST_ZOOM.len()), grid)
        }
    }

    /// The sizes, as CSS classes on the views (the icons take their size
    /// from them, see files.css).
    fn apply_zoom(&self) {
        let (list, grid) = self.zoom.get();
        let set = |w: &gtk::Widget, prefix: &str, at: usize, len: usize| {
            for n in 0..len {
                w.remove_css_class(&format!("{prefix}{n}"));
            }
            w.add_css_class(&format!("{prefix}{at}"));
        };
        for w in [
            self.list.upcast_ref::<gtk::Widget>(),
            self.current.upcast_ref(),
            self.parent_list.upcast_ref(),
        ] {
            set(w, "zoom-list-", list, LIST_ZOOM.len());
        }
        set(self.grid.upcast_ref(), "zoom-grid-", grid, GRID_ZOOM.len());
    }

    // ------------------------------------------------------- columns

    pub fn set_columns(&self, columns: &[String]) {
        *self.columns.borrow_mut() = columns.to_vec();
        self.apply_columns();
    }

    /// The chosen columns shown, the rest not; "location" while
    /// searching too.
    fn apply_columns(&self) {
        let chosen = self.columns.borrow();
        let columns = self.list.columns();
        for n in 0..columns.n_items() {
            let Some(c) = columns.item(n).and_downcast::<gtk::ColumnViewColumn>() else {
                continue;
            };
            let id = c.id().map(|i| i.to_string()).unwrap_or_default();
            c.set_visible(id == "name" || chosen.contains(&id) || (id == "location" && self.searching.get()));
        }
    }

    // ------------------------------------------------------- search

    fn mode(&self) -> Search {
        if self.search_contents.is_active() {
            Search::Contents
        } else if self.search_names.is_active() {
            Search::Names
        } else {
            Search::Filter
        }
    }

    /// Typed into the bar: the filter now, or a search in a moment.
    fn text_changed(self: &Rc<Self>) {
        if self.mode() == Search::Filter {
            self.apply_filter();
            return;
        }
        self.schedule_search();
    }

    fn mode_changed(self: &Rc<Self>) {
        if self.mode() == Search::Filter {
            if self.searching.get() {
                self.reload_folder();
            }
            self.apply_filter();
        } else {
            self.apply_filter();
            self.schedule_search();
        }
        self.filter_entry.set_placeholder_text(Some(&tr(match self.mode() {
            Search::Filter => "files-filter",
            Search::Names => "files-search-names-placeholder",
            Search::Contents => "files-search-contents-placeholder",
        })));
    }

    fn schedule_search(self: &Rc<Self>) {
        if let Some(id) = self.search_wait.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(350), move || {
            if let Some(p) = weak.upgrade() {
                p.search_wait.borrow_mut().take();
                p.run_search();
            }
        });
        *self.search_wait.borrow_mut() = Some(id);
    }

    /// Searches under the folder for what the bar holds; with less than
    /// two letters, the folder again.
    fn run_search(self: &Rc<Self>) {
        let wanted = self.filter_entry.text().trim().to_owned();
        let Some(dir) = self.location.borrow().path().map(std::path::PathBuf::from) else {
            return;
        };
        if wanted.chars().count() < 2 {
            if self.searching.get() {
                self.reload_folder();
            }
            return;
        }
        if let Some(stop) = self.search_stop.borrow_mut().take() {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.searching.set(true);
        self.apply_columns();
        self.apply_filter();
        self.store.remove_all();
        self.marking.set(false);
        self.cursor.set(None);
        self.anchor.set(None);
        self.loading.set(true);
        self.update_status();
        let stop = Arc::new(AtomicBool::new(false));
        *self.search_stop.borrow_mut() = Some(stop.clone());
        let rx = entry::search(
            dir,
            wanted,
            self.mode() == Search::Contents,
            self.show_hidden.get(),
            stop,
        );
        self.fill_from(rx, generation, true);
    }

    /// Stops a search and forgets it was one (the folder is listed by
    /// whoever calls).
    fn end_search(&self) {
        if let Some(id) = self.search_wait.borrow_mut().take() {
            id.remove();
        }
        if let Some(stop) = self.search_stop.borrow_mut().take() {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if self.searching.replace(false) {
            self.apply_columns();
        }
        self.resetting.set(true);
        self.search_contents.set_active(false);
        self.search_names.set_active(false);
        self.resetting.set(false);
        self.filter_entry.set_placeholder_text(Some(&tr("files-filter")));
    }

    /// Back from search results to the folder's entries, the bar kept.
    fn reload_folder(self: &Rc<Self>) {
        if let Some(stop) = self.search_stop.borrow_mut().take() {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.searching.set(false);
        self.apply_columns();
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.store.remove_all();
        self.loading.set(true);
        self.apply_filter();
        let here = self.location.borrow().clone();
        self.fill_from(entry::list(here), generation, false);
    }

    /// Ctrl+Shift+F: the bar, searching the subfolders.
    pub fn open_search(&self) {
        self.search_names.set_active(true);
        self.open_filter();
    }
    fn apply_sort(&self) {
        let (key, ascending) = self.sort.get();
        self.sorter.set_sort_func(move |a, b| {
            let (Some(a), Some(b)) = (entry_of(a), entry_of(b)) else {
                return gtk::Ordering::Equal;
            };
            // Folders first, always.
            if a.dir != b.dir {
                return if a.dir {
                    gtk::Ordering::Smaller
                } else {
                    gtk::Ordering::Larger
                };
            }
            let order = match key {
                SortKey::Name => std::cmp::Ordering::Equal,
                SortKey::Location => entry::folder_text(&a).cmp(&entry::folder_text(&b)),
                SortKey::Permissions => (a.mode & 0o7777).cmp(&(b.mode & 0o7777)),
                SortKey::Owner => entry::user_name(a.uid).cmp(&entry::user_name(b.uid)),
                SortKey::Group => entry::group_name(a.gid).cmp(&entry::group_name(b.gid)),
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
        // Searching, the bar holds what was searched for, not a filter.
        let wanted = if self.mode() == Search::Filter {
            self.filter_entry.text().to_lowercase()
        } else {
            String::new()
        };
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

    pub fn close_filter(self: &Rc<Self>) {
        let searched = self.searching.get();
        self.end_search();
        self.resetting.set(true);
        self.filter_entry.set_text("");
        self.resetting.set(false);
        self.filter_bar.set_reveal_child(false);
        if searched {
            self.reload_folder();
        }
        self.apply_filter();
        self.focus_view();
    }

    pub fn filtering(&self) -> bool {
        self.filter_bar.reveals_child()
    }

    fn update_status(&self) {
        let shown = self.selection.n_items();
        let selected = self.selected();
        let text = if *self.location.borrow() == Location::Computer {
            String::new()
        } else if self.searching.get() && self.loading.get() {
            tr_args("files-status-searching", &[("n", number(shown as f64))])
        } else if self.searching.get() && selected.len() <= 1 {
            tr_args("files-status-found", &[("n", number(shown as f64))])
        } else if self.loading.get() {
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
        // Through admin://, everything here is done as root: said.
        let text = if self.as_admin() {
            format!("{} · {text}", tr("files-admin-status"))
        } else {
            text
        };
        self.status.set_label(&text);
        if self.as_admin() {
            self.widget.add_css_class("admin");
        } else {
            self.widget.remove_css_class("admin");
        }
    }

    /// Whether this folder is reached through admin:// (as root).
    pub fn as_admin(&self) -> bool {
        is_admin(&self.location.borrow())
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

/// The position of the entry at (`x`, `y`) in a view, if there is one.
fn position_under(view: &gtk::Widget, x: f64, y: f64) -> Option<u32> {
    let mut node = view.pick(x, y, gtk::PickFlags::DEFAULT);
    while let Some(w) = node {
        if let Some(position) = position_of_cell(&w) {
            return Some(position);
        }
        if &w == view {
            return None;
        }
        node = w.parent();
    }
    None
}

/// The row (or icon) of the entry at (`x`, `y`): what holds all of it,
/// all the columns of a list's row.
fn row_under(view: &gtk::Widget, x: f64, y: f64) -> Option<gtk::Widget> {
    let mut node = view.pick(x, y, gtk::PickFlags::DEFAULT);
    let mut cell = None;
    while let Some(w) = node {
        if &w == view {
            break;
        }
        if cell.is_none() {
            if position_of_cell(&w).is_some() {
                cell = Some(w.clone());
            }
        } else if w.css_name() == "row" || w.css_name() == "child" {
            return Some(w);
        }
        node = w.parent();
    }
    cell
}

pub fn paths_of(list: &gdk::FileList) -> Vec<PathBuf> {
    list.files().iter().filter_map(|f| f.path()).collect()
}

/// Whether files dropped in `to` are copied (true) or moved, or None:
/// into themselves, or where they are already. Moved, as a rule — also
/// across disks (here the home's folders are mounts of their own); Ctrl
/// copies.
pub fn drop_kind(paths: &[PathBuf], to: &Path, state: gdk::ModifierType) -> Option<bool> {
    if paths.is_empty() || paths.iter().any(|s| to.starts_with(s)) || paths.iter().all(|s| s.parent() == Some(to)) {
        return None;
    }
    Some(state.contains(gdk::ModifierType::CONTROL_MASK))
}

/// Whether `location` is reached through GVfs's admin:// — as root.
pub fn is_admin(location: &Location) -> bool {
    match location {
        Location::Uri(u) => u.starts_with("admin://"),
        Location::Path(p) => {
            let gvfs = glib::user_runtime_dir().join("gvfs");
            p.strip_prefix(&gvfs)
                .ok()
                .and_then(|rest| rest.components().next())
                .is_some_and(|mount| mount.as_os_str().to_string_lossy().starts_with("admin"))
        }
        Location::Computer => false,
    }
}

/// Whether (x, y) falls on the list's column headers.
fn on_header(view: &gtk::Widget, x: f64, y: f64) -> bool {
    let mut node = view.pick(x, y, gtk::PickFlags::DEFAULT);
    while let Some(w) = node {
        if w.css_name() == "header" {
            return true;
        }
        if &w == view {
            return false;
        }
        node = w.parent();
    }
    false
}

/// Whether (x, y) falls on the view's empty space: no entry, and not
/// its scrollbars or column headers.
fn on_empty(view: &gtk::Widget, x: f64, y: f64) -> bool {
    let mut node = view.pick(x, y, gtk::PickFlags::DEFAULT);
    while let Some(w) = node {
        if position_of_cell(&w).is_some() || w.is::<gtk::Scrollbar>() || w.css_name() == "header" {
            return false;
        }
        if &w == view {
            return true;
        }
        node = w.parent();
    }
    false
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
    factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        let row = gtk::Box::builder().spacing(10).css_classes(["entry-row"]).build();
        row.append(&gtk::Image::builder().css_classes(["entry-icon"]).build());
        row.append(
            &gtk::Label::builder()
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .build(),
        );
        item.set_child(Some(&row));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        let (Some(row), Some(e)) = (item.child(), item.item().and_then(|o| entry_of(&o))) else {
            return;
        };
        remember_position(&row, item);
        let mut child = row.first_child();
        while let Some(w) = child {
            if let Some(image) = w.downcast_ref::<gtk::Image>() {
                image.set_from_gicon(&entry::icon(&e));
                break;
            }
            child = w.next_sibling();
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
            item.set_child(Some(
                &gtk::Label::builder()
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::Start)
                    .css_classes(["dim"])
                    .build(),
            ));
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
    view.append_column(&text_column(
        "modified",
        &tr("files-column-modified"),
        |e| entry::date_text(e.modified),
        false,
    ));
    view.append_column(&text_column(
        "kind",
        &tr("files-column-kind"),
        |e| gio::content_type_get_description(&e.mime).to_string(),
        false,
    ));
    view.append_column(&text_column(
        "permissions",
        &tr("files-column-permissions"),
        entry::mode_text,
        false,
    ));
    view.append_column(&text_column(
        "owner",
        &tr("files-column-owner"),
        |e| entry::user_name(e.uid),
        false,
    ));
    view.append_column(&text_column(
        "group",
        &tr("files-column-group"),
        |e| entry::group_name(e.gid),
        false,
    ));
    // A long path would take the name's room: the end of it, in a column
    // of its own width (which can be dragged wider).
    let location = text_column("location", &tr("files-column-location"), entry::folder_text, false);
    location.set_fixed_width(260);
    view.append_column(&location);
    view.sort_by_column(Some(&name), gtk::SortType::Ascending);
    view
}

/// A large icon (or a thumbnail) over the name.
fn grid_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        let cell = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(5)
            .css_classes(["grid-cell"])
            .build();
        cell.append(&gtk::Image::builder().css_classes(["entry-icon"]).build());
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
