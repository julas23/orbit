//! Editing the dock in place.
//!
//! "Edit dock" — in a launcher's menu, or in the dock's own on a right click
//! where there is no launcher — turns editing on. While editing, the dock
//! takes every click from the launchers: a launcher or a separator dragged
//! along the dock goes where it is dropped, and dropped off the dock it is
//! removed; a right click adds a launcher (from the installed applications
//! or an AppImage file) or a separator after the launcher under the pointer
//! (in a gap, where the pointer is). An AppImage dropped on the dock is
//! added where it is dropped (see `appimage`). Every change is shown at
//! once and written to `dock.yaml` (`ConfigDir::save_dock_apps`).
//!
//! Separators are entries of their own in `dock.yaml` (`- separator`), as
//! many as wanted and anywhere: side by side, at the ends. An older file's
//! groups (a separator between neighbouring launchers of different groups)
//! are read as separators, and written as them the first time the dock
//! changes.

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

use gtk::{gdk, gio, glib, prelude::*};
use gtk4_layer_shell::{KeyboardMode, LayerShell};
use orbit_app::i18n::tr;
use orbit_common::{
    ConfigDir,
    config::{DockApp, DockEntry, Edge},
};

use crate::{Launcher, SystemApps, appimage, badges::Badges};

/// A drag shorter than this is a click.
const DRAG_MIN: f64 = 5.0;

enum Kind {
    App(DockApp),
    Separator,
}

struct Item {
    id: u64,
    kind: Kind,
    /// `None` for a launcher that is hidden (`show: false`) or whose
    /// program is missing: it stays in `dock.yaml` all the same.
    widget: Option<gtk::Widget>,
}

pub struct Editor {
    dir: ConfigDir,
    icon_size: i32,
    vertical: bool,
    root: gtk::Box,
    launchers: gtk::Box,
    badges: Rc<Badges>,
    system: SystemApps,
    items: RefCell<Vec<Item>>,
    next: Cell<u64>,
    editing: Cell<bool>,
    /// Whether `dock.yaml` has been written in this run (the backup).
    saved: Cell<bool>,
    /// The item being dragged, and where the drag started.
    dragging: Cell<Option<(u64, f64, f64)>>,
    /// The widget marked as the drop place, and the mark.
    mark: RefCell<Option<(gtk::Widget, &'static str)>>,
    /// Where the menu was opened, for what it adds.
    menu_at: Cell<(f64, f64)>,
    weak: RefCell<std::rc::Weak<Editor>>,
    /// The dock's edge and thickness, for moving it.
    edge: Edge,
    size: i32,
}

/// Where a drag would end.
#[derive(Clone, Copy, PartialEq)]
enum Drop {
    /// Before item `index` of the list (or at the end, past the last).
    At(usize),
    Off,
}

impl Editor {
    /// The dock's launchers and separators, from `apps`, in `launchers`;
    /// `root` is the whole dock, which takes the clicks while editing.
    pub fn new(
        dir: &ConfigDir,
        apps: &[DockEntry],
        icon_size: i32,
        (edge, size): (Edge, i32),
        root: &gtk::Box,
        launchers: &gtk::Box,
    ) -> Rc<Self> {
        let editor = Rc::new(Self {
            dir: dir.clone(),
            icon_size,
            vertical: edge.is_vertical(),
            root: root.clone(),
            launchers: launchers.clone(),
            badges: Badges::new(launchers),
            system: SystemApps::load(),
            items: RefCell::default(),
            next: Cell::new(0),
            editing: Cell::new(false),
            saved: Cell::new(false),
            dragging: Cell::new(None),
            mark: RefCell::default(),
            menu_at: Cell::new((0.0, 0.0)),
            weak: RefCell::default(),
            edge,
            size,
        });
        editor.weak.replace(Rc::downgrade(&editor));
        for entry in with_group_separators(apps) {
            editor.push(match entry {
                Some(app) => Kind::App(app.clone()),
                None => Kind::Separator,
            });
        }
        editor.lay_out();
        gestures(&editor);
        editor
    }

    fn push(&self, kind: Kind) -> u64 {
        let id = self.next.get();
        self.next.set(id + 1);
        let widget = self.widget_for(&kind);
        self.items.borrow_mut().push(Item { id, kind, widget });
        id
    }

    fn widget_for(&self, kind: &Kind) -> Option<gtk::Widget> {
        match kind {
            Kind::Separator => {
                // The line, in a box as thick as a launcher's gap: the box is
                // what a click or a drag finds, not the 2 px line alone.
                let line = gtk::Box::builder()
                    .css_classes(["line"])
                    .hexpand(self.vertical)
                    .vexpand(!self.vertical)
                    .build();
                let separator = gtk::Box::builder().css_classes(["separator"]).build();
                separator.append(&line);
                Some(separator.upcast())
            }
            Kind::App(app) if app.show => {
                let launcher = Launcher::resolve(app, &self.system)?;
                let weak = self.weak.borrow().clone();
                let edit: Rc<dyn Fn()> = Rc::new(move || {
                    if let Some(editor) = weak.upgrade() {
                        editor.set_editing(true);
                    }
                });
                Some(launcher.button(self.icon_size, &self.badges, edit).upcast())
            }
            Kind::App(_) => None,
        }
    }

    /// Puts the launchers and separators in the dock in the list's order:
    /// every separator, wherever it is.
    fn lay_out(&self) {
        while let Some(child) = self.launchers.first_child() {
            self.launchers.remove(&child);
        }
        for widget in self.items.borrow().iter().filter_map(|item| item.widget.as_ref()) {
            self.launchers.append(widget);
        }
    }

    /// Shows the change and writes the launchers to `dock.yaml`.
    fn changed(&self) {
        self.lay_out();
        // The separators as entries; the groups, which made them before,
        // would add more on the next start.
        let list: Vec<DockEntry> = self
            .items
            .borrow()
            .iter()
            .map(|item| match &item.kind {
                Kind::App(app) => DockEntry::App(DockApp {
                    group: None,
                    ..app.clone()
                }),
                Kind::Separator => DockEntry::SEPARATOR,
            })
            .collect();
        let first = !self.saved.replace(true);
        match self.dir.save_dock_apps(&list, first) {
            Ok(()) => log::info!("dock: launchers saved to dock.yaml"),
            Err(err) => log::error!("dock: saving the launchers: {err:#}"),
        }
    }

    fn set_editing(&self, editing: bool) {
        self.editing.set(editing);
        if editing {
            self.root.add_css_class("editing");
        } else {
            self.root.remove_css_class("editing");
        }
    }

    /// The items laid out, with where each is drawn in the dock.
    fn shown(&self) -> Vec<(u64, gtk::graphene::Rect)> {
        self.items
            .borrow()
            .iter()
            .filter_map(|item| {
                let widget = item.widget.as_ref()?;
                (widget.parent().as_ref() == Some(self.launchers.upcast_ref()))
                    .then(|| widget.compute_bounds(&self.root))
                    .flatten()
                    .map(|rect| (item.id, rect))
            })
            .collect()
    }

    /// The item at (`x`, `y`), in the dock's coordinates.
    fn at(&self, x: f64, y: f64) -> Option<u64> {
        let point = gtk::graphene::Point::new(x as f32, y as f32);
        self.shown()
            .into_iter()
            .find(|(_, rect)| rect.contains_point(&point))
            .map(|(id, _)| id)
    }

    fn index(&self, id: u64) -> Option<usize> {
        self.items.borrow().iter().position(|item| item.id == id)
    }

    /// Where item `id` would go if dropped at (`x`, `y`), and the item
    /// that shows it (with the side, as a CSS class).
    fn drop_at(&self, id: u64, x: f64, y: f64) -> (Drop, Option<(u64, &'static str)>) {
        let (w, h) = (f64::from(self.root.width()), f64::from(self.root.height()));
        if x < 0.0 || y < 0.0 || x > w || y > h {
            return (Drop::Off, Some((id, "drop-off")));
        }
        let along = |r: &gtk::graphene::Rect| self.middle(r);
        let pointer = if self.vertical { y } else { x };
        let shown: Vec<_> = self.shown().into_iter().filter(|(other, _)| *other != id).collect();
        let (before, after) = if self.vertical {
            ("drop-above", "drop-below")
        } else {
            ("drop-left", "drop-right")
        };
        match shown.iter().find(|(_, r)| pointer < along(r)) {
            Some((next, _)) => (Drop::At(self.index(*next).unwrap_or(0)), Some((*next, before))),
            None => match shown.last() {
                Some((last, _)) => (Drop::At(self.index(*last).map_or(0, |i| i + 1)), Some((*last, after))),
                None => (Drop::At(self.items.borrow().len()), None),
            },
        }
    }

    fn set_mark(&self, mark: Option<(u64, &'static str)>) {
        let next = mark.and_then(|(id, class)| {
            let items = self.items.borrow();
            let widget = items.iter().find(|item| item.id == id)?.widget.clone()?;
            Some((widget, class))
        });
        let mut current = self.mark.borrow_mut();
        if *current == next {
            return;
        }
        if let Some((widget, class)) = current.take() {
            widget.remove_css_class(class);
        }
        if let Some((widget, class)) = &next {
            widget.add_css_class(class);
        }
        *current = next;
    }

    /// Moves item `id` before list place `to`, or off the dock.
    fn drop_item(&self, id: u64, to: Drop) {
        {
            let mut items = self.items.borrow_mut();
            let Some(from) = items.iter().position(|item| item.id == id) else {
                return;
            };
            match to {
                Drop::Off => {
                    items.remove(from);
                }
                Drop::At(to) => {
                    let item = items.remove(from);
                    let to = if to > from { to - 1 } else { to }.min(items.len());
                    items.insert(to, item);
                }
            }
        }
        self.changed();
    }

    /// The middle of `rect` along the dock.
    fn middle(&self, rect: &gtk::graphene::Rect) -> f64 {
        if self.vertical {
            f64::from(rect.y() + rect.height() / 2.0)
        } else {
            f64::from(rect.x() + rect.width() / 2.0)
        }
    }

    /// The list place after the item under (`x`, `y`); elsewhere (a gap)
    /// the place between the items the pointer is between, or the end.
    fn place_after(&self, x: f64, y: f64) -> usize {
        if let Some(i) = self.at(x, y).and_then(|id| self.index(id)) {
            return i + 1;
        }
        let pointer = if self.vertical { y } else { x };
        self.shown()
            .into_iter()
            .find(|(_, rect)| pointer < self.middle(rect))
            .and_then(|(id, _)| self.index(id))
            .unwrap_or_else(|| self.items.borrow().len())
    }

    fn insert(&self, at: usize, kind: Kind) {
        let id = self.push(kind);
        {
            let mut items = self.items.borrow_mut();
            let item = items.pop().expect("just pushed");
            debug_assert_eq!(item.id, id);
            let at = at.min(items.len());
            items.insert(at, item);
        }
        self.changed();
    }

    /// Adds AppImage `path` at list place `at`, once its icon and desktop
    /// file are out of it (in a thread: it runs the AppImage's runtime).
    fn add_appimage(&self, path: PathBuf, at: usize) {
        let weak = self.weak.borrow().clone();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || appimage::install(&path)).await;
            let Some(editor) = weak.upgrade() else { return };
            match result {
                Ok(Ok(desktop)) => editor.insert(at, Kind::App(launcher(desktop))),
                Ok(Err(err)) => log::error!("dock: adding the AppImage: {err:#}"),
                Err(_) => log::error!("dock: adding the AppImage failed"),
            }
        });
    }
}

/// A new launcher for desktop file `desktop` (an id or a path).
fn launcher(desktop: String) -> DockApp {
    DockApp {
        desktop: Some(desktop),
        name: None,
        exec: None,
        icon: None,
        group: None,
        show: true,
    }
}

/// `apps:` as the dock lays it out (`None` a separator): its separators,
/// and one more between neighbouring launchers of different groups where
/// there is none already.
fn with_group_separators(apps: &[DockEntry]) -> Vec<Option<&DockApp>> {
    let mut out = Vec::new();
    // The group of the launcher before, when nothing separates it.
    let mut last: Option<Option<&str>> = None;
    for entry in apps {
        match entry {
            DockEntry::Separator(_) => {
                out.push(None);
                last = None;
            }
            DockEntry::App(app) => {
                let group = app.group.as_deref();
                if last.is_some_and(|last| last != group) {
                    out.push(None);
                }
                last = Some(group);
                out.push(Some(app));
            }
        }
    }
    out
}

fn gestures(editor: &Rc<Editor>) {
    let root = &editor.root;
    // In the capture phase, so the dock sees a click before the launcher
    // under it and can keep it from the launcher while editing.
    let click = gtk::GestureClick::builder()
        .button(0)
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    let weak = Rc::downgrade(editor);
    click.connect_pressed(move |gesture, _, x, y| {
        let Some(editor) = weak.upgrade() else { return };
        let right = gesture.current_button() == gtk::gdk::BUTTON_SECONDARY;
        if editor.editing.get() {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if right {
                dock_menu(&editor, x, y);
            }
        } else if right && editor.at(x, y).is_none() {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            dock_menu(&editor, x, y);
        }
    });
    root.add_controller(click.clone());

    // AppImages dropped on the dock, from a file manager.
    let files = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    let weak = Rc::downgrade(editor);
    files.connect_drop(move |_, value, x, y| {
        let Some(editor) = weak.upgrade() else { return false };
        let Ok(list) = value.get::<gdk::FileList>() else {
            return false;
        };
        let paths: Vec<PathBuf> = list
            .files()
            .iter()
            .filter_map(|file| file.path())
            .filter(|path| appimage::is_appimage(path))
            .collect();
        let at = editor.place_after(x, y);
        for path in paths.iter().rev() {
            editor.add_appimage(path.clone(), at);
        }
        !paths.is_empty()
    });
    root.add_controller(files);

    let drag = gtk::GestureDrag::builder()
        .button(gtk::gdk::BUTTON_PRIMARY)
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    root.add_controller(drag.clone());
    // One sequence state for both: what the click claims, the drag has.
    drag.group_with(&click);
    let weak = Rc::downgrade(editor);
    drag.connect_drag_begin(move |_, x, y| {
        let Some(editor) = weak.upgrade() else { return };
        if !editor.editing.get() {
            return;
        }
        if let Some(id) = editor.at(x, y) {
            editor.dragging.set(Some((id, x, y)));
            editor.set_dragged(id, true);
        }
    });
    let weak = Rc::downgrade(editor);
    drag.connect_drag_update(move |_, dx, dy| {
        let Some(editor) = weak.upgrade() else { return };
        let Some((id, x, y)) = editor.dragging.get() else {
            return;
        };
        let mark = if dx.hypot(dy) < DRAG_MIN {
            None
        } else {
            editor.drop_at(id, x + dx, y + dy).1
        };
        editor.set_mark(mark);
    });
    let weak = Rc::downgrade(editor);
    drag.connect_drag_end(move |_, dx, dy| {
        let Some(editor) = weak.upgrade() else { return };
        let Some((id, x, y)) = editor.dragging.take() else {
            return;
        };
        editor.set_mark(None);
        editor.set_dragged(id, false);
        if dx.hypot(dy) >= DRAG_MIN {
            let (to, _) = editor.drop_at(id, x + dx, y + dy);
            editor.drop_item(id, to);
        }
    });
}

impl Editor {
    fn set_dragged(&self, id: u64, on: bool) {
        let items = self.items.borrow();
        if let Some(widget) = items
            .iter()
            .find(|item| item.id == id)
            .and_then(|item| item.widget.as_ref())
        {
            if on {
                widget.add_css_class("dragged");
            } else {
                widget.remove_css_class("dragged");
            }
        }
    }
}

/// The dock's own menu: edit it; while editing, add a launcher or a
/// separator after the item under the pointer.
fn dock_menu(editor: &Rc<Editor>, x: f64, y: f64) {
    editor.menu_at.set((x, y));
    let model = gio::Menu::new();
    let group = gio::SimpleActionGroup::new();
    let add = |section: &gio::Menu, label: &str, name: &str, act: fn(&Rc<Editor>)| {
        section.append(Some(label), Some(&format!("edit.{name}")));
        let action = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(editor);
        action.connect_activate(move |_, _| {
            if let Some(editor) = weak.upgrade() {
                act(&editor);
            }
        });
        group.add_action(&action);
    };
    let section = gio::Menu::new();
    if editor.editing.get() {
        add(&section, &tr("dock-add-launcher"), "launcher", |e| {
            let (x, y) = e.menu_at.get();
            chooser(e, x, y);
        });
        add(&section, &tr("dock-add-appimage"), "appimage", |e| {
            let (x, y) = e.menu_at.get();
            pick_appimage(e, x, y);
        });
        add(&section, &tr("dock-add-separator"), "separator", |e| {
            let (x, y) = e.menu_at.get();
            e.insert(e.place_after(x, y), Kind::Separator);
        });
        model.append_section(None, &section);
        let last = gio::Menu::new();
        add(&last, &tr("dock-edit-done"), "done", |e| e.set_editing(false));
        model.append_section(None, &last);
    } else {
        add(&section, &tr("dock-edit"), "edit", |e| e.set_editing(true));
        model.append_section(None, &section);
    }
    if orbit_app::mover::supported() {
        let section = gio::Menu::new();
        add(&section, &tr("dock-move"), "move", |e| {
            let surface = orbit_app::mover::Surface {
                file: "dock.yaml",
                edges: &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right],
                size: e.size,
                edge: e.edge,
                monitor: None,
            };
            orbit_app::mover::start(surface, &e.root, &e.dir);
        });
        model.append_section(None, &section);
    }
    let popover = gtk::PopoverMenu::from_model(Some(&model));
    popover.insert_action_group("edit", Some(&group));
    orbit_app::popup::open(&popover, &editor.root, Some((x, y)));
}

/// Asks for an AppImage file; it goes after the item under (`x`, `y`).
fn pick_appimage(editor: &Rc<Editor>, x: f64, y: f64) {
    let at = editor.place_after(x, y);
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("AppImage"));
    filter.add_suffix("AppImage");
    filter.add_suffix("appimage");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title(tr("dock-add-appimage"))
        .modal(true)
        .filters(&filters)
        .default_filter(&filter)
        .build();
    let usual = glib::home_dir().join(".appimage");
    if usual.is_dir() {
        dialog.set_initial_folder(Some(&gio::File::for_path(usual)));
    }
    let weak = Rc::downgrade(editor);
    // No parent: the dock is a layer surface, not a window to stack on.
    dialog.open(None::<&gtk::Window>, gio::Cancellable::NONE, move |result| {
        if let (Some(editor), Ok(file)) = (weak.upgrade(), result)
            && let Some(path) = file.path()
        {
            editor.add_appimage(path, at);
        }
    });
}

/// The installed applications to pick a new launcher from, with a search
/// field; the one picked goes after the item under (`x`, `y`).
fn chooser(editor: &Rc<Editor>, x: f64, y: f64) {
    let at = editor.place_after(x, y);
    let search = gtk::SearchEntry::builder().placeholder_text(tr("dock-search")).build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Browse)
        .activate_on_single_click(true)
        .build();
    let mut apps: Vec<gio::AppInfo> = gio::AppInfo::all()
        .into_iter()
        .filter(|app| app.should_show())
        .collect();
    apps.sort_by_key(|app| app.display_name().to_lowercase());
    for app in &apps {
        let row = gtk::Box::builder().spacing(8).css_classes(["choice"]).build();
        let image = gtk::Image::builder().pixel_size(20).build();
        if let Some(icon) = app.icon() {
            image.set_from_gicon(&icon);
        }
        row.append(&image);
        row.append(
            &gtk::Label::builder()
                .label(app.display_name().as_str())
                .xalign(0.0)
                .build(),
        );
        let row = gtk::ListBoxRow::builder().child(&row).build();
        row.set_widget_name(app.id().as_deref().unwrap_or_default());
        row.set_tooltip_text(Some(app.display_name().as_str()));
        list.append(&row);
    }
    {
        let search = search.clone();
        list.set_filter_func(move |row| {
            let text = search.text().to_lowercase();
            text.is_empty()
                || row
                    .tooltip_text()
                    .is_some_and(|name| name.to_lowercase().contains(&text))
        });
    }
    {
        let list = list.clone();
        search.connect_search_changed(move |_| list.invalidate_filter());
    }
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(320)
        .min_content_width(260)
        .child(&list)
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(5)
        .css_classes(["dock-chooser"])
        .build();
    content.append(&search);
    content.append(&scroller);
    let popover = gtk::Popover::builder().child(&content).build();

    let weak = Rc::downgrade(editor);
    let shut = popover.downgrade();
    list.connect_row_activated(move |_, row| {
        let Some(editor) = weak.upgrade() else { return };
        let id = row.widget_name();
        if !id.is_empty() {
            editor.insert(at, Kind::App(launcher(id.to_string())));
        }
        if let Some(popover) = shut.upgrade() {
            popover.popdown();
        }
    });
    {
        // Enter picks the first application the search leaves.
        let list = list.clone();
        search.connect_activate(move |_| {
            let mut i = 0;
            while let Some(row) = list.row_at_index(i) {
                if row.is_child_visible() {
                    row.activate();
                    return;
                }
                i += 1;
            }
        });
    }

    // The dock takes no keyboard; the search needs it while it is open.
    let window = editor.root.root().and_downcast::<gtk::Window>();
    let layered = window.as_ref().is_some_and(|w| w.is_layer_window());
    if let (Some(window), true) = (&window, layered) {
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        let window = window.clone();
        popover.connect_closed(move |_| window.set_keyboard_mode(KeyboardMode::None));
    }
    orbit_app::popup::open(&popover, &editor.root, Some((x, y)));
    search.grab_focus();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, group: Option<&str>) -> DockEntry {
        DockEntry::App(DockApp {
            desktop: Some(name.into()),
            name: None,
            exec: None,
            icon: None,
            group: group.map(Into::into),
            show: true,
        })
    }

    fn laid_out(apps: &[DockEntry]) -> String {
        with_group_separators(apps)
            .iter()
            .map(|e| e.and_then(|a| a.desktop.as_deref()).unwrap_or("|"))
            .collect()
    }

    #[test]
    fn separators_from_groups_and_of_their_own() {
        let sep = DockEntry::SEPARATOR;
        // Groups: one between each two that differ.
        let list = [app("a", Some("web")), app("b", Some("dev")), app("c", Some("dev"))];
        assert_eq!(laid_out(&list), "a|bc");
        // A separator of its own where the groups change: just the one.
        let list = [app("a", Some("web")), sep.clone(), app("b", Some("dev"))];
        assert_eq!(laid_out(&list), "a|b");
        // Anywhere, side by side, at the ends.
        let list = [
            sep.clone(),
            app("a", None),
            sep.clone(),
            sep.clone(),
            app("b", None),
            sep,
        ];
        assert_eq!(laid_out(&list), "|a||b|");
    }
}
