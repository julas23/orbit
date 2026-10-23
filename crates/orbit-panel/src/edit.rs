//! Editing the vertical panel in place.
//!
//! A right click on the panel where there is no module opens its menu:
//! edit the panel, add a module. While editing, the panel takes every click
//! from the modules: dragging one puts it before or after the module it is
//! dropped by, and a right click on one offers its sizes, moving it up,
//! down or to another part, and removing it. Every change is laid out at
//! once and written to `panel.yaml` (`ConfigDir::save_panel_modules`).

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::tr;
use orbit_common::{
    ConfigDir,
    config::{Edge, ModuleEntry, PanelConfig, Size},
};

use crate::board::{Board, Form, Part, Side};

/// Every module the panel knows, in the order the add menu lists them.
const MODULES: &[&str] = &[
    "menu",
    "windows",
    "clock",
    "calendar",
    "weather",
    "forecast",
    "tray",
    "volume",
    "brightness",
    "network",
    "keyboard",
    "battery",
    "notifications",
    "bluetooth",
    "monitors",
    "spectrum",
    "aigent",
    "cpu",
    "ram",
    "gpu",
    "lan",
    "disk",
    "workspaces",
    "session",
];

/// Modules that may be listed more than once, each showing something else.
const REPEATABLE: &[&str] = &["disk"];

const PARTS: [Part; 3] = [Part::Start, Part::Center, Part::End];

/// A drag shorter than this is a click.
const DRAG_MIN: f64 = 5.0;

/// One module in `panel.yaml`'s lists.
struct Slot {
    id: u64,
    part: Part,
    entry: ModuleEntry,
}

/// A module as built: `None` when it could not be or has nothing to show
/// (brightness without a backlight); it stays in the lists all the same.
struct Built {
    form: Form,
    widget: Option<gtk::Widget>,
}

struct Editor {
    board: Board,
    dir: ConfigDir,
    edge: Edge,
    /// The panel's thickness.
    size: i32,
    slots: RefCell<Vec<Slot>>,
    built: RefCell<HashMap<u64, Built>>,
    next: Cell<u64>,
    editing: Cell<bool>,
    /// Whether `panel.yaml` has been written in this run (the backup).
    saved: Cell<bool>,
    /// The module being dragged, and where the drag started.
    dragging: Cell<Option<(u64, f64, f64)>>,
    /// The module marked as the drop place, and the mark.
    mark: RefCell<Option<(gtk::Widget, &'static str)>>,
}

/// The vertical panel: `config`'s modules on a board that can be edited.
pub fn new(dir: &ConfigDir, config: &PanelConfig) -> gtk::Widget {
    let board = Board::default();
    board.set_css_classes(&["panel", "grid"]);
    let editor = Rc::new(Editor {
        board: board.clone(),
        dir: dir.clone(),
        edge: config.edge,
        size: config.size,
        slots: RefCell::default(),
        built: RefCell::default(),
        next: Cell::new(0),
        editing: Cell::new(false),
        saved: Cell::new(false),
        dragging: Cell::new(None),
        mark: RefCell::default(),
    });
    for (part, entries) in [
        (Part::Start, &config.start),
        (Part::Center, &config.center),
        (Part::End, &config.end),
    ] {
        for entry in entries {
            editor.push(part, entry.clone());
        }
    }
    editor.lay_out();
    gestures(&editor);
    // The gestures hold it weakly; the panel holds it for good.
    crate::status::hold(board.upcast_ref(), editor);
    board.upcast()
}

fn gestures(editor: &Rc<Editor>) {
    let board = &editor.board;
    // In the capture phase, so the panel sees a click before the module
    // under it and can keep it from the module while editing.
    let click = gtk::GestureClick::builder()
        .button(0)
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    let weak = Rc::downgrade(editor);
    click.connect_pressed(move |gesture, _, x, y| {
        let Some(editor) = weak.upgrade() else { return };
        let over = editor.board.at(x, y);
        let right = gesture.current_button() == gtk::gdk::BUTTON_SECONDARY;
        log::debug!("panel: press {} at {x:.0},{y:.0} over {over:?}", gesture.current_button());
        if editor.editing.get() {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if right {
                match over {
                    Some(id) => module_menu(&editor, id, x, y),
                    None => panel_menu(&editor, x, y),
                }
            }
        } else if right && over.is_none() {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            panel_menu(&editor, x, y);
        }
    });
    board.add_controller(click.clone());

    let drag = gtk::GestureDrag::builder()
        .button(gtk::gdk::BUTTON_PRIMARY)
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    board.add_controller(drag.clone());
    // One sequence state for both: what the click claims, the drag has.
    drag.group_with(&click);
    let weak = Rc::downgrade(editor);
    drag.connect_drag_begin(move |_, x, y| {
        let Some(editor) = weak.upgrade() else { return };
        if !editor.editing.get() {
            return;
        }
        if let Some(id) = editor.board.at(x, y) {
            editor.dragging.set(Some((id, x, y)));
            if let Some(widget) = editor.board.widget(id) {
                widget.add_css_class("dragged");
            }
        }
    });
    let weak = Rc::downgrade(editor);
    drag.connect_drag_update(move |_, dx, dy| {
        let Some(editor) = weak.upgrade() else { return };
        let Some((id, x, y)) = editor.dragging.get() else { return };
        let target = (dx.hypot(dy) >= DRAG_MIN)
            .then(|| editor.board.nearest(x + dx, y + dy, Some(id)))
            .flatten();
        editor.set_mark(target);
    });
    let weak = Rc::downgrade(editor);
    drag.connect_drag_end(move |_, dx, dy| {
        let Some(editor) = weak.upgrade() else { return };
        let Some((id, x, y)) = editor.dragging.take() else { return };
        editor.set_mark(None);
        if let Some(widget) = editor.board.widget(id) {
            widget.remove_css_class("dragged");
        }
        if dx.hypot(dy) < DRAG_MIN {
            return;
        }
        if let Some((target, side)) = editor.board.nearest(x + dx, y + dy, Some(id)) {
            editor.move_by(id, target, side.after());
        }
    });
}

impl Editor {
    fn push(&self, part: Part, entry: ModuleEntry) -> u64 {
        let id = self.next.get();
        self.next.set(id + 1);
        self.slots.borrow_mut().push(Slot { id, part, entry });
        id
    }

    /// Puts the modules on the board in the lists' order, building the ones
    /// that are new or changed size.
    fn lay_out(&self) {
        self.board.clear();
        let slots = self.slots.borrow();
        let mut built = self.built.borrow_mut();
        built.retain(|id, _| slots.iter().any(|slot| slot.id == *id));
        for part in PARTS {
            for slot in slots.iter().filter(|slot| slot.part == part) {
                let (form, cols, rows) = crate::shape(&slot.entry);
                let module = built.entry(slot.id).or_insert_with(|| Built { form, widget: None });
                if module.form != form || module.widget.is_none() {
                    module.form = form;
                    module.widget = crate::module(&slot.entry, &self.dir, self.edge, form)
                        // e.g. brightness on a machine without a backlight
                        .filter(|widget| widget.is_visible());
                }
                if let Some(widget) = &module.widget {
                    self.board.add(slot.id, widget, &slot.entry.name, part, cols, rows);
                }
            }
        }
    }

    /// Lays the panel out again and writes the lists to `panel.yaml`.
    fn changed(&self) {
        {
            // Each part together, in the parts' order, as the file has them.
            let mut slots = self.slots.borrow_mut();
            slots.sort_by_key(|slot| PARTS.iter().position(|p| *p == slot.part));
        }
        self.lay_out();
        let slots = self.slots.borrow();
        let list = |part: Part| -> Vec<ModuleEntry> {
            slots
                .iter()
                .filter(|slot| slot.part == part)
                .map(|slot| slot.entry.clone())
                .collect()
        };
        let first = !self.saved.replace(true);
        let saved = self.dir.save_panel_modules(
            &list(Part::Start),
            &list(Part::Center),
            &list(Part::End),
            first,
        );
        match saved {
            Ok(()) => log::info!("panel: layout saved to panel.yaml"),
            Err(err) => log::error!("panel: saving the layout: {err:#}"),
        }
    }

    fn index(&self, id: u64) -> Option<usize> {
        self.slots.borrow().iter().position(|slot| slot.id == id)
    }

    /// Puts module `id` just before or after module `target`, in its part.
    fn move_by(&self, id: u64, target: u64, after: bool) {
        {
            let mut slots = self.slots.borrow_mut();
            let Some(from) = slots.iter().position(|slot| slot.id == id) else { return };
            let mut slot = slots.remove(from);
            let Some(to) = slots.iter().position(|slot| slot.id == target) else {
                slots.insert(from, slot);
                return;
            };
            slot.part = slots[to].part;
            slots.insert(if after { to + 1 } else { to }, slot);
        }
        self.changed();
    }

    /// One place up (`-1`) or down (`1`) in its part; past the part's first
    /// or last place, into the part before or after it.
    fn step(&self, id: u64, by: isize) {
        {
            let mut slots = self.slots.borrow_mut();
            let Some(i) = slots.iter().position(|slot| slot.id == id) else { return };
            let j = i as isize + by;
            let same = usize::try_from(j)
                .ok()
                .filter(|&j| slots.get(j).is_some_and(|other| other.part == slots[i].part));
            match same {
                Some(j) => slots.swap(i, j),
                None => {
                    let k = PARTS.iter().position(|p| *p == slots[i].part).unwrap_or(0) as isize + by;
                    let Some(part) = usize::try_from(k).ok().and_then(|k| PARTS.get(k)) else { return };
                    slots[i].part = *part;
                    // Last of the part above, first of the part below.
                    let slot = slots.remove(i);
                    let at = if by < 0 {
                        slots.iter().rposition(|s| s.part == *part).map_or(0, |p| p + 1)
                    } else {
                        slots.iter().position(|s| s.part == *part).unwrap_or(slots.len())
                    };
                    slots.insert(at, slot);
                }
            }
        }
        self.changed();
    }

    /// Marks where a drag would drop (or no place).
    fn set_mark(&self, target: Option<(u64, Side)>) {
        let next = target.and_then(|(id, side)| Some((self.board.widget(id)?, side.class())));
        let mut mark = self.mark.borrow_mut();
        if *mark == next {
            return;
        }
        if let Some((widget, class)) = mark.take() {
            widget.remove_css_class(class);
        }
        if let Some((widget, class)) = &next {
            widget.add_css_class(class);
        }
        *mark = next;
    }

    fn set_editing(&self, editing: bool) {
        self.editing.set(editing);
        if editing {
            self.board.add_css_class("editing");
        } else {
            self.board.remove_css_class("editing");
        }
    }
}

/// The menu of the panel itself: edit it, move it, add a module where it
/// was clicked.
fn panel_menu(editor: &Rc<Editor>, x: f64, y: f64) {
    let model = gio::Menu::new();
    let group = gio::SimpleActionGroup::new();

    model.append(Some(&tr("panel-edit")), Some("edit.editing"));
    let editing = gio::SimpleAction::new_stateful("editing", None, &editor.editing.get().to_variant());
    let weak = Rc::downgrade(editor);
    editing.connect_activate(move |action, _| {
        let Some(editor) = weak.upgrade() else { return };
        let on = !editor.editing.get();
        action.set_state(&on.to_variant());
        editor.set_editing(on);
    });
    group.add_action(&editing);

    if orbit_app::mover::supported() {
        model.append(Some(&tr("panel-move")), Some("edit.move"));
        let action = gio::SimpleAction::new("move", None);
        let weak = Rc::downgrade(editor);
        action.connect_activate(move |_, _| {
            let Some(editor) = weak.upgrade() else { return };
            // Only the vertical panel is laid out on the grid, and edited.
            let surface = orbit_app::mover::Surface {
                file: "panel.yaml",
                edges: &[Edge::Left, Edge::Right],
                size: editor.size,
                edge: editor.edge,
                monitor: None,
            };
            orbit_app::mover::start(surface, &editor.board, &editor.dir);
        });
        group.add_action(&action);
    }

    let missing: Vec<&str> = {
        let slots = editor.slots.borrow();
        MODULES
            .iter()
            .copied()
            .filter(|name| REPEATABLE.contains(name) || !slots.iter().any(|slot| slot.entry.name == *name))
            .collect()
    };
    if !missing.is_empty() {
        let add = gio::Menu::new();
        for name in &missing {
            add.append(Some(&tr(&format!("panel-module-{name}"))), Some(&format!("edit.add::{name}")));
        }
        model.append_submenu(Some(&tr("panel-add")), &add);
        let action = gio::SimpleAction::new("add", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(editor);
        action.connect_activate(move |_, name| {
            let Some(editor) = weak.upgrade() else { return };
            let Some(name) = name.and_then(|n| n.str().map(str::to_owned)) else { return };
            // After the module nearest to the click, else at the top.
            let near = editor.board.nearest(x, y, None);
            let mut entry = ModuleEntry::from(name.as_str());
            if name == "disk" {
                // Another disk shows a filesystem none shows yet.
                let shown: Vec<String> = editor.slots.borrow().iter().filter(|s| s.entry.name == "disk").map(|s| s.entry.path.clone().unwrap_or_else(|| "/".into())).collect();
                entry.path = crate::disk::mounts().into_iter().find(|m| !shown.contains(m));
            }
            let id = editor.push(Part::Start, entry);
            match near {
                Some((target, side)) => editor.move_by(id, target, side.after()),
                None => {
                    let mut slots = editor.slots.borrow_mut();
                    let slot = slots.pop().expect("just pushed");
                    slots.insert(0, slot);
                    drop(slots);
                    editor.changed();
                }
            }
            editor.set_editing(true);
        });
        group.add_action(&action);
    }

    open(editor, &model, &group, x, y);
}

/// The menu of module `id` while editing.
fn module_menu(editor: &Rc<Editor>, id: u64, x: f64, y: f64) {
    let Some(i) = editor.index(id) else { return };
    let (name, part, form) = {
        let slots = editor.slots.borrow();
        let slot = &slots[i];
        (slot.entry.name.clone(), slot.part, crate::shape(&slot.entry).0)
    };
    let model = gio::Menu::new();
    let group = gio::SimpleActionGroup::new();

    let sizes: Vec<Form> = crate::shapes(&name).iter().map(|shape| shape.0).collect();
    let mut first = gio::Menu::new();
    if sizes.len() > 1 {
        for size in [Form::Mini, Form::Normal, Form::Full] {
            if sizes.contains(&size) {
                first.append(
                    Some(&tr(&format!("panel-size-{}", size.class()))),
                    Some(&format!("edit.size::{}", size.class())),
                );
            }
        }
        let action = gio::SimpleAction::new_stateful(
            "size",
            Some(glib::VariantTy::STRING),
            &form.class().to_variant(),
        );
        let weak = Rc::downgrade(editor);
        action.connect_activate(move |_, size| {
            let Some(editor) = weak.upgrade() else { return };
            let size = match size.and_then(|s| s.str().map(str::to_owned)).as_deref() {
                Some("mini") => Size::Mini,
                Some("full") => Size::Full,
                _ => Size::Normal,
            };
            if let Some(i) = editor.index(id) {
                editor.slots.borrow_mut()[i].entry.size = Some(size);
                editor.changed();
            }
        });
        group.add_action(&action);
        model.append_section(Some(&tr(&format!("panel-module-{name}"))), &first);
        first = gio::Menu::new();
    }

    if name == "disk" {
        filesystems(editor, id, &model, &group);
    }

    let simple = |menu: &gio::Menu, label: &str, name: &str, act: Box<dyn Fn(&Editor)>| {
        menu.append(Some(label), Some(&format!("edit.{name}")));
        let action = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(editor);
        action.connect_activate(move |_, _| {
            if let Some(editor) = weak.upgrade() {
                act(&editor);
            }
        });
        group.add_action(&action);
    };
    simple(&first, &tr("panel-move-up"), "up", Box::new(move |e| e.step(id, -1)));
    simple(&first, &tr("panel-move-down"), "down", Box::new(move |e| e.step(id, 1)));
    if sizes.len() > 1 {
        model.append_section(None, &first);
    } else {
        model.append_section(Some(&tr(&format!("panel-module-{name}"))), &first);
    }

    let parts = gio::Menu::new();
    for key in ["start", "center", "end"] {
        parts.append(Some(&tr(&format!("panel-part-{key}"))), Some(&format!("edit.part::{key}")));
    }
    let key = match part {
        Part::Start => "start",
        Part::Center => "center",
        Part::End => "end",
    };
    let action = gio::SimpleAction::new_stateful("part", Some(glib::VariantTy::STRING), &key.to_variant());
    let weak = Rc::downgrade(editor);
    action.connect_activate(move |_, key| {
        let Some(editor) = weak.upgrade() else { return };
        let part = match key.and_then(|k| k.str().map(str::to_owned)).as_deref() {
            Some("start") => Part::Start,
            Some("center") => Part::Center,
            _ => Part::End,
        };
        let Some(i) = editor.index(id) else { return };
        if editor.slots.borrow()[i].part == part {
            return;
        }
        {
            // The side of the part nearest to where the module was: the
            // start's last place, the centre's last, the end's first.
            let mut slots = editor.slots.borrow_mut();
            let mut slot = slots.remove(i);
            slot.part = part;
            let at = match part {
                Part::End => slots.iter().position(|s| s.part == Part::End).unwrap_or(slots.len()),
                _ => slots.iter().rposition(|s| s.part == part).map_or_else(
                    || slots.iter().position(|s| s.part == Part::End).unwrap_or(slots.len()),
                    |p| p + 1,
                ),
            };
            slots.insert(at, slot);
        }
        editor.changed();
    });
    group.add_action(&action);
    model.append_section(None, &parts);

    let last = gio::Menu::new();
    simple(
        &last,
        &tr("panel-remove"),
        "remove",
        Box::new(move |e| {
            if let Some(i) = e.index(id) {
                e.slots.borrow_mut().remove(i);
                e.changed();
            }
        }),
    );
    simple(&last, &tr("panel-edit-done"), "done", Box::new(|e| e.set_editing(false)));
    model.append_section(None, &last);

    open(editor, &model, &group, x, y);
}

fn open(editor: &Editor, model: &gio::Menu, group: &gio::SimpleActionGroup, x: f64, y: f64) {
    let popover = gtk::PopoverMenu::from_model(Some(model));
    popover.insert_action_group("edit", Some(group));
    orbit_app::popup::open(&popover, &editor.board, Some((x, y)));
}

/// The disk module's filesystems, the one it shows ticked; picking one
/// shows it there and keeps it in `panel.yaml`.
fn filesystems(editor: &Rc<Editor>, id: u64, model: &gio::Menu, group: &gio::SimpleActionGroup) {
    let Some(i) = editor.index(id) else { return };
    let current = editor.slots.borrow()[i].entry.path.clone().unwrap_or_else(|| "/".into());
    let mut mounts = crate::disk::mounts();
    if !mounts.contains(&current) {
        mounts.insert(0, current.clone());
    }
    let menu = gio::Menu::new();
    for mount in &mounts {
        let item = gio::MenuItem::new(Some(mount), None);
        item.set_action_and_target_value(Some("edit.path"), Some(&mount.to_variant()));
        menu.append_item(&item);
    }
    let action = gio::SimpleAction::new_stateful("path", Some(glib::VariantTy::STRING), &current.to_variant());
    let weak = Rc::downgrade(editor);
    action.connect_activate(move |_, path| {
        let Some(editor) = weak.upgrade() else { return };
        let Some(path) = path.and_then(|p| p.str().map(str::to_owned)) else { return };
        let Some(i) = editor.index(id) else { return };
        editor.slots.borrow_mut()[i].entry.path = Some(path);
        // Built again, showing the new one.
        editor.built.borrow_mut().remove(&id);
        editor.changed();
    });
    group.add_action(&action);
    model.append_section(Some(&tr("panel-filesystem")), &menu);
}
