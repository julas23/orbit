//! What the modules share to be set up from their own clicks: a left or
//! right click on a module, the menus those open, writing the module's
//! options to `modules/<name>.yaml` and building the module again with
//! them, and a time-zone picker.

use std::{cell::RefCell, rc::Rc};

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::tr;
use orbit_common::ConfigDir;
use serde_json::Value;

type Reloader = Box<dyn Fn(&[&str])>;

/// A form's list, filled again by itself after a row is removed.
pub type Refill = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

thread_local! {
    static DIR: RefCell<Option<ConfigDir>> = const { RefCell::new(None) };
    /// Builds the modules with these names again (the vertical panel's
    /// editor); without one, the panel starts over.
    static RELOAD: RefCell<Option<Reloader>> = const { RefCell::new(None) };
}

/// The panel's configuration directory, for the modules' own writes.
pub fn init(dir: &ConfigDir) {
    DIR.with(|d| *d.borrow_mut() = Some(dir.clone()));
}

pub fn dir() -> Option<ConfigDir> {
    DIR.with(|d| d.borrow().clone())
}

/// Reads `modules/<name>.yaml` again (the defaults when it cannot be).
pub fn load<T: serde::de::DeserializeOwned + Default>(name: &str) -> T {
    dir().and_then(|d| d.module(name).ok()).unwrap_or_default()
}

/// Writes `pairs` as top-level keys of `modules/<module>.yaml`, the rest of
/// the file kept, and builds the modules in `rebuild` again.
pub fn save(module: &str, pairs: &[(&str, Value)], rebuild: &[&str]) {
    let Some(dir) = dir() else { return };
    let mut map = serde_json::Map::new();
    for (key, value) in pairs {
        map.insert((*key).to_string(), value.clone());
    }
    let keys: Vec<&str> = pairs.iter().map(|(k, _)| *k).collect();
    let saved = serde_yaml_ng::to_string(&Value::Object(map))
        .map_err(anyhow::Error::from)
        .and_then(|blocks| dir.save_module_keys(module, &keys, &blocks));
    match saved {
        Ok(()) => {
            log::info!("panel: {module}: {} saved", keys.join(", "));
            reload(rebuild);
        }
        Err(err) => log::error!("panel: saving modules/{module}.yaml: {err:#}"),
    }
}

pub fn set_reloader(reload: impl Fn(&[&str]) + 'static) {
    RELOAD.with(|r| *r.borrow_mut() = Some(Box::new(reload)));
}

/// Builds the modules named `names` again, from their files; on the
/// horizontal panel, which has no editor, the whole panel starts over.
pub fn reload(names: &[&str]) {
    if names.is_empty() {
        return;
    }
    // After the click that asked for it: the module may be the one clicked.
    let names: Vec<String> = names.iter().map(|n| (*n).to_string()).collect();
    glib::idle_add_local_once(move || {
        if names.iter().any(|n| n == "weather") {
            crate::weather_data::reset();
        }
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let done = RELOAD.with(|r| match &*r.borrow() {
            Some(reload) => {
                reload(&names);
                true
            }
            None => false,
        });
        if !done {
            restart();
        }
    });
}

/// The panel replaced by a fresh copy of itself, same process (what a
/// change of the system's time zone needs: GLib keeps the zone it read).
pub fn restart() {
    use std::os::unix::process::CommandExt;
    let args: Vec<String> = std::env::args().skip(1).collect();
    log::info!("panel: starting over");
    let err = std::process::Command::new("/proc/self/exe").args(args).exec();
    log::error!("panel: starting over: {err}");
}

/// Calls `f` with the click's position on a click of `button` (1 left, 3
/// right) on `widget`. The click is claimed, so a module's own click does
/// not reach the panel's menu.
pub fn on_click(widget: &impl IsA<gtk::Widget>, button: u32, f: impl Fn(&gtk::Widget, f64, f64) + 'static) {
    let click = gtk::GestureClick::builder().button(button).build();
    click.connect_pressed(|gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    click.connect_released(move |gesture, _, x, y| {
        if let Some(widget) = gesture.widget() {
            f(&widget, x, y);
        }
    });
    widget.add_controller(click);
    if button == 1 {
        widget.set_cursor_from_name(Some("pointer"));
    }
}

/// A context menu being put together: sections of items, radio groups
/// and submenus, each action a closure.
pub struct Menu {
    pub model: gio::Menu,
    group: gio::SimpleActionGroup,
    next: std::cell::Cell<u32>,
}

impl Default for Menu {
    fn default() -> Self {
        Self::new()
    }
}

impl Menu {
    pub fn new() -> Self {
        Self {
            model: gio::Menu::new(),
            group: gio::SimpleActionGroup::new(),
            next: std::cell::Cell::new(0),
        }
    }

    fn name(&self) -> String {
        let n = self.next.get();
        self.next.set(n + 1);
        format!("a{n}")
    }

    /// A plain item in `section`.
    pub fn item(&self, section: &gio::Menu, label: &str, f: impl Fn() + 'static) {
        let name = self.name();
        let action = gio::SimpleAction::new(&name, None);
        action.connect_activate(move |_, _| f());
        self.group.add_action(&action);
        section.append(Some(label), Some(&format!("m.{name}")));
    }

    /// A radio group in `section`: `choices` as (value, label), `current`
    /// ticked; `f` gets the value picked.
    pub fn radio(&self, section: &gio::Menu, choices: &[(String, String)], current: &str, f: impl Fn(String) + 'static) {
        let name = self.name();
        let action = gio::SimpleAction::new_stateful(&name, Some(glib::VariantTy::STRING), &current.to_variant());
        action.connect_activate(move |action, value| {
            if let Some(value) = value.and_then(|v| v.str().map(str::to_owned)) {
                action.set_state(&value.to_variant());
                f(value);
            }
        });
        self.group.add_action(&action);
        for (value, label) in choices {
            let item = gio::MenuItem::new(Some(label), None);
            item.set_action_and_target_value(Some(&format!("m.{name}")), Some(&value.to_variant()));
            section.append_item(&item);
        }
    }

    /// Opens the menu at (`x`, `y`) of `anchor`.
    pub fn open(&self, anchor: &gtk::Widget, x: f64, y: f64) {
        let popover = gtk::PopoverMenu::from_model(Some(&self.model));
        popover.insert_action_group("m", Some(&self.group));
        orbit_app::popup::open(&popover, anchor, Some((x, y)));
    }
}

/// A section of `menu` with an optional title.
pub fn section(menu: &Menu, title: Option<&str>) -> gio::Menu {
    let section = gio::Menu::new();
    menu.model.append_section(title, &section);
    section
}

/// A submenu of `parent` called `label`.
pub fn submenu(parent: &gio::Menu, label: &str) -> gio::Menu {
    let sub = gio::Menu::new();
    parent.append_submenu(Some(label), &sub);
    sub
}

/// A popover with `child` at (`x`, `y`) of `anchor`, unparented once closed.
pub fn popover(anchor: &gtk::Widget, x: f64, y: f64, child: &impl IsA<gtk::Widget>) -> gtk::Popover {
    let popover = gtk::Popover::builder().child(child).build();
    popover.add_css_class("orbit-form");
    orbit_app::popup::open(&popover, anchor, Some((x, y)));
    popover
}

/// The IANA time zones, sorted: the system's list (zone1970.tab) and UTC.
pub fn zones() -> Vec<String> {
    let text = std::fs::read_to_string("/usr/share/zoneinfo/zone1970.tab")
        .or_else(|_| std::fs::read_to_string("/usr/share/zoneinfo/zone.tab"))
        .unwrap_or_default();
    let mut zones: Vec<String> = text
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split('\t').nth(2))
        .map(str::to_owned)
        .collect();
    zones.push("UTC".into());
    zones.sort();
    zones.dedup();
    zones
}

/// "São Paulo" → "sao paulo", for matching what is typed.
pub fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            '_' => ' ',
            c => c,
        })
        .collect()
}

/// A list to pick a time zone from, typed to narrow; `picked` gets the
/// zone (`America/Sao_Paulo`).
pub fn zone_picker(anchor: &gtk::Widget, x: f64, y: f64, title: &str, picked: impl Fn(String) + 'static) {
    let zones = zones();
    let search = gtk::SearchEntry::builder().placeholder_text(tr("panel-zone-search")).build();
    let list = gtk::ListBox::builder().css_classes(["zones"]).build();
    for zone in &zones {
        let label = gtk::Label::builder().label(zone.replace('_', " ")).xalign(0.0).build();
        let row = gtk::ListBoxRow::builder().child(&label).name(zone.as_str()).build();
        list.append(&row);
    }
    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .min_content_height(320)
        .min_content_width(280)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let content = form_box(title);
    content.append(&search);
    content.append(&scroll);

    let filter = Rc::new(RefCell::new(String::new()));
    {
        let filter = filter.clone();
        list.set_filter_func(move |row| {
            let wanted = filter.borrow();
            wanted.is_empty() || fold(row.widget_name().as_str()).contains(wanted.as_str())
        });
    }
    {
        let list = list.clone();
        search.connect_search_changed(move |search| {
            *filter.borrow_mut() = fold(search.text().trim());
            list.invalidate_filter();
        });
    }
    let popover = popover(anchor, x, y, &content);
    {
        let list = list.clone();
        search.connect_activate(move |_| {
            if let Some(row) = first_visible(&list) {
                row.activate();
            }
        });
    }
    let picked = Rc::new(picked);
    list.connect_row_activated(move |_, row| {
        picked(row.widget_name().to_string());
        popover.popdown();
    });
    search.grab_focus();
}

fn first_visible(list: &gtk::ListBox) -> Option<gtk::ListBoxRow> {
    let mut child = list.first_child();
    while let Some(widget) = child {
        if let Some(row) = widget.downcast_ref::<gtk::ListBoxRow>()
            && row.is_child_visible()
        {
            return Some(row.clone());
        }
        child = widget.next_sibling();
    }
    None
}

/// A vertical box with a bold title, the body of the modules' forms.
pub fn form_box(title: &str) -> gtk::Box {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .css_classes(["module-form"])
        .build();
    content.append(&gtk::Label::builder().label(title).css_classes(["form-title"]).xalign(0.0).build());
    content
}

/// The system's time zone (`timedatectl set-timezone`, which asks polkit),
/// then the panel starts over to show it.
pub fn set_system_zone(zone: &str) {
    let proc = gio::Subprocess::newv(
        &["timedatectl".as_ref(), "set-timezone".as_ref(), zone.as_ref()],
        gio::SubprocessFlags::STDERR_PIPE,
    );
    let zone = zone.to_string();
    match proc {
        Ok(proc) => {
            let done = proc.clone();
            proc.communicate_utf8_async(None, gio::Cancellable::NONE, move |res| match res {
                Ok(_) if done.is_successful() => {
                    log::info!("panel: the system's time zone is now {zone}");
                    restart();
                }
                Ok((_, err)) => log::error!("panel: timedatectl: {}", err.unwrap_or_default().trim()),
                Err(err) => log::error!("panel: timedatectl: {err}"),
            });
        }
        Err(err) => log::error!("panel: timedatectl: {err}"),
    }
}

/// The system's time zone now.
pub fn system_zone() -> String {
    glib::TimeZone::local().identifier().to_string()
}
