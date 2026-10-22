//! Window list module, after the window list of the eww panel.
//!
//! One entry per open window: icon and APPLICATION name (the title goes to
//! the tooltip), with a tab on the left in the light colour of the window's
//! area. Focused: bold on a light background. Minimized (sway scratchpad):
//! faded and italic, still listed so a click brings it back; under
//! orbit-core its tab keeps the colour of the area it left.
//!
//! Order is geometric: by area, then by monitor in physical position (top
//! to bottom, left to right), then by window position on that monitor in the
//! same reading order. Minimized windows have no place on screen and go last.
//!
//! Left click focuses (a minimized window goes back to the area it left
//! under orbit-core, comes to the current one under sway).
//! Right click opens a menu: show/minimize, maximize, restore, send to area
//! 1–4, close.
//!
//! Names and icons come from desktop files: the `desktop_dirs` first (the
//! dock's, with their own names), then the system's.

use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
};

use gio_unix::prelude::*;
use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::{number, tr, tr_args};
use serde::Deserialize;
use serde_json::Value;

use crate::sway;

/// `modules/windows.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WindowsConfig {
    /// Searched for desktop files before the system's, in order.
    pub desktop_dirs: Vec<String>,
    /// Workspace-number distance between the same area on two monitors.
    pub stride: i32,
    /// How many areas the menu offers.
    pub areas: i32,
    /// Run with `{id}` replaced by the window's sway con_id. Empty: the menu
    /// entry is left out.
    pub maximize: String,
    pub restore: String,
}

impl Default for WindowsConfig {
    fn default() -> Self {
        Self {
            desktop_dirs: Vec::new(),
            stride: 10,
            areas: 4,
            maximize: String::new(),
            restore: String::new(),
        }
    }
}

/// A window as the list shows it.
#[derive(Debug, Clone, PartialEq)]
struct Window {
    id: i64,
    class: String,
    title: String,
    /// 1-based area; `None` when minimized.
    area: Option<i32>,
    /// Minimized: the area it left, when the compositor says (orbit-core).
    left: Option<i32>,
    output: String,
    x: i32,
    y: i32,
    focused: bool,
    /// Position in sway's tree, the last tie-breaker (tabbed windows share a
    /// rectangle).
    order: usize,
}

impl Window {
    fn minimized(&self) -> bool {
        self.area.is_none()
    }
}

pub fn new(config: &WindowsConfig, vertical: bool) -> gtk::Widget {
    let list = gtk::Box::builder()
        .orientation(if vertical {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        })
        .spacing(5)
        .css_classes(["windows"])
        .build();

    if !sway::available() {
        log::warn!("windows: not running under sway; the list stays empty");
        return list.upcast();
    }

    let module = Rc::new(Module {
        config: config.clone(),
        vertical,
        list: list.clone(),
        apps: Apps::load(&config.desktop_dirs),
        items: RefCell::new(HashMap::new()),
        conn: RefCell::new(None),
    });
    module.refresh();
    let weak = Rc::downgrade(&module);
    sway::subscribe(&["window", "workspace", "output"], move || {
        if let Some(module) = weak.upgrade() {
            module.refresh();
        }
    });
    list.connect_destroy(move |_| {
        let _ = &module;
    });
    list.upcast()
}

struct Item {
    button: gtk::Button,
    icon: gtk::Image,
    name: gtk::Label,
    /// The window as last shown, for the click handlers.
    window: Rc<RefCell<Window>>,
}

struct Module {
    config: WindowsConfig,
    vertical: bool,
    list: gtk::Box,
    apps: Apps,
    items: RefCell<HashMap<i64, Item>>,
    conn: RefCell<Option<sway::Connection>>,
}

impl Module {
    fn with_conn<T>(&self, f: impl Fn(&mut sway::Connection) -> std::io::Result<T>) -> Option<T> {
        let mut conn = self.conn.borrow_mut();
        for _ in 0..2 {
            if conn.is_none() {
                *conn = sway::Connection::open().ok();
            }
            match f(conn.as_mut()?) {
                Ok(value) => return Some(value),
                Err(_) => *conn = None,
            }
        }
        None
    }

    fn command(&self, command: &str) {
        if self
            .with_conn(|c| c.request(sway::RUN_COMMAND, command))
            .is_none()
        {
            log::error!("windows: could not reach sway");
        }
    }

    fn refresh(self: &Rc<Self>) {
        let Some(tree) = self.with_conn(|c| c.query::<Value>(sway::GET_TREE)) else {
            return;
        };
        let outputs: Vec<sway::Output> = self
            .with_conn(|c| c.query(sway::GET_OUTPUTS))
            .unwrap_or_default();
        let mut windows = collect(&tree, self.config.stride.max(1));
        sort(&mut windows, &outputs);

        let mut items = self.items.borrow_mut();
        items.retain(|id, item| {
            let open = windows.iter().any(|w| w.id == *id);
            if !open {
                self.list.remove(&item.button);
            }
            open
        });
        let mut previous: Option<gtk::Widget> = None;
        for window in &windows {
            let item = items.entry(window.id).or_insert_with(|| self.item(window));
            self.show(item, window);
            // Put the entry in its place, after the previous one.
            self.list
                .reorder_child_after(&item.button, previous.as_ref());
            previous = Some(item.button.clone().upcast());
        }
    }

    fn item(self: &Rc<Self>, window: &Window) -> Item {
        let icon = gtk::Image::builder().pixel_size(16).build();
        let name = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .hexpand(true)
            .build();
        let inner = gtk::Box::builder().spacing(5).build();
        inner.append(&icon);
        if self.vertical {
            inner.append(&name);
        }
        let button = gtk::Button::builder().child(&inner).build();
        self.list.append(&button);

        let shared = Rc::new(RefCell::new(window.clone()));
        {
            let weak = Rc::downgrade(self);
            let shared = shared.clone();
            button.connect_clicked(move |_| {
                if let Some(module) = weak.upgrade() {
                    module.focus(&shared.borrow());
                }
            });
        }
        let right = gtk::GestureClick::builder().button(3).build();
        {
            let weak = Rc::downgrade(self);
            let shared = shared.clone();
            let anchor = button.downgrade();
            right.connect_pressed(move |_, _, x, y| {
                if let (Some(module), Some(anchor)) = (weak.upgrade(), anchor.upgrade()) {
                    menu(&module, &anchor, &shared.borrow(), Some((x, y)));
                }
            });
        }
        button.add_controller(right);
        Item {
            button,
            icon,
            name,
            window: shared,
        }
    }

    fn show(&self, item: &Item, window: &Window) {
        let (app, gicon) = self.apps.lookup(&window.class);
        match gicon {
            Some(gicon) => orbit_app::set_gicon(&item.icon, &gicon, 16),
            None => item.icon.set_icon_name(Some(&fallback_icon(&window.class))),
        }
        item.name.set_label(&app);
        let mut classes = vec!["window".to_string()];
        if let Some(area) = window.area.or(window.left) {
            classes.push(format!("area-{}", area.clamp(1, 4)));
        }
        if window.focused {
            classes.push("focused".into());
        }
        if window.minimized() {
            classes.push("minimized".into());
        }
        let classes: Vec<&str> = classes.iter().map(String::as_str).collect();
        item.button.set_css_classes(&classes);
        let place = match window.area {
            Some(area) => tr_args("windows-place-area", &[("area", number(area))]),
            None => tr("windows-place-minimized"),
        };
        let tooltip = if self.vertical {
            format!("{}  [{place}]", window.title)
        } else {
            format!("{app} — {}  [{place}]", window.title)
        };
        item.button.set_tooltip_text(Some(&tooltip));
        *item.window.borrow_mut() = window.clone();
    }

    /// Focuses `window`; a minimized one comes back to the current area,
    /// tiled — `scratchpad show` alone would leave it floating, and tiling
    /// it in place could drop it into another area. orbit-core's own
    /// `focus` puts it back where it was.
    fn focus(&self, window: &Window) {
        if window.minimized() && window.left.is_none() {
            let current = self
                .with_conn(|c| c.query::<Vec<sway::Workspace>>(sway::GET_WORKSPACES))
                .and_then(|ws| ws.into_iter().find(|w| w.focused).map(|w| w.num));
            if let Some(num) = current {
                self.command(&format!(
                    "[con_id={}] move container to workspace number {num}, floating disable, focus",
                    window.id
                ));
                return;
            }
        }
        self.command(&format!("[con_id={}] focus", window.id));
    }

    /// Sends `window` to `area` on its own monitor, without moving the view.
    fn send(&self, window: &Window, area: i32) {
        let outputs: Vec<sway::Output> = self
            .with_conn(|c| c.query(sway::GET_OUTPUTS))
            .unwrap_or_default();
        let mut active: Vec<&sway::Output> = outputs.iter().filter(|o| o.active).collect();
        // The order of ~/.config/sway/areas.sh: laptop panel first.
        active.sort_by_key(|o| (!o.name.starts_with("eDP"), o.rect.y, o.rect.x));
        let output = if window.minimized() {
            active.iter().position(|o| o.focused)
        } else {
            active.iter().position(|o| o.name == window.output)
        }
        .unwrap_or(0) as i32;
        let number = output * self.config.stride.max(1) + area;
        self.command(&format!(
            "[con_id={}] move container to workspace number {number}",
            window.id
        ));
    }

    fn run(&self, template: &str, window: &Window) {
        let command = template.replace("{id}", &window.id.to_string());
        if let Err(err) = glib::spawn_command_line_async(&command) {
            log::error!("windows: running {command:?}: {err}");
        }
    }
}

/// The right-click menu of one entry.
fn menu(module: &Rc<Module>, anchor: &gtk::Button, window: &Window, at: orbit_app::popup::Point) {
    let model = gio::Menu::new();
    let group = gio::SimpleActionGroup::new();
    let add = |label: &str, name: &str, act: Box<dyn Fn(&Module)>| {
        model.append(Some(label), Some(&format!("win.{name}")));
        let action = gio::SimpleAction::new(name, None);
        let weak: Weak<Module> = Rc::downgrade(module);
        action.connect_activate(move |_, _| {
            if let Some(module) = weak.upgrade() {
                act(&module);
            }
        });
        group.add_action(&action);
    };

    let w = window.clone();
    if window.minimized() {
        add(&tr("windows-show"), "show", Box::new(move |m| m.focus(&w)));
    } else {
        add(
            &tr("windows-minimize"),
            "minimize",
            Box::new(move |m| m.command(&format!("[con_id={}] move scratchpad", w.id))),
        );
    }
    if !module.config.maximize.is_empty() {
        let w = window.clone();
        add(
            &tr("windows-maximize"),
            "maximize",
            Box::new(move |m| m.run(&m.config.maximize, &w)),
        );
    }
    if !module.config.restore.is_empty() {
        let w = window.clone();
        add(
            &tr("windows-restore"),
            "restore",
            Box::new(move |m| m.run(&m.config.restore, &w)),
        );
    }
    for area in 1..=module.config.areas.max(1) {
        let w = window.clone();
        add(
            &tr_args("windows-area", &[("area", number(area))]),
            &format!("area{area}"),
            Box::new(move |m| m.send(&w, area)),
        );
    }
    let w = window.clone();
    add(
        &tr("windows-close"),
        "close",
        Box::new(move |m| m.command(&format!("[con_id={}] kill", w.id))),
    );

    let popover = gtk::PopoverMenu::from_model(Some(&model));
    popover.insert_action_group("win", Some(&group));
    orbit_app::popup::open(&popover, anchor, at);
}

/// Every application window in the tree, with its area and output.
fn collect(tree: &Value, stride: i32) -> Vec<Window> {
    fn walk(node: &Value, output: &str, area: Option<i32>, stride: i32, out: &mut Vec<Window>) {
        let kind = node["type"].as_str().unwrap_or_default();
        let mut output = output.to_string();
        let mut area = area;
        match kind {
            "output" => output = node["name"].as_str().unwrap_or_default().to_string(),
            "workspace" => {
                area = match node["name"].as_str() {
                    Some("__i3_scratch") => None,
                    _ => node["num"]
                        .as_i64()
                        .filter(|n| *n > 0)
                        .map(|n| n as i32 % stride),
                }
            }
            "con" | "floating_con" if node["pid"].is_i64() => {
                let class = node["app_id"]
                    .as_str()
                    .or_else(|| node["window_properties"]["class"].as_str())
                    .or_else(|| node["window_properties"]["instance"].as_str())
                    .unwrap_or("?")
                    .to_string();
                let title = node["name"].as_str().unwrap_or_default().trim().to_string();
                out.push(Window {
                    id: node["id"].as_i64().unwrap_or_default(),
                    title: if title.is_empty() {
                        class.clone()
                    } else {
                        title
                    },
                    class,
                    area,
                    left: node["orbit_workspace"]
                        .as_i64()
                        .filter(|n| *n > 0)
                        .map(|n| n as i32 % stride),
                    output: output.clone(),
                    x: node["rect"]["x"].as_i64().unwrap_or_default() as i32,
                    y: node["rect"]["y"].as_i64().unwrap_or_default() as i32,
                    focused: node["focused"].as_bool().unwrap_or(false),
                    order: out.len(),
                });
            }
            _ => {}
        }
        for key in ["nodes", "floating_nodes"] {
            for child in node[key].as_array().into_iter().flatten() {
                walk(child, &output, area, stride, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(tree, "", Some(0), stride, &mut out);
    out
}

/// Area, then monitor in physical position, then window position — both in
/// reading order (top to bottom, left to right). Minimized windows last.
fn sort(windows: &mut [Window], outputs: &[sway::Output]) {
    let mut physical: Vec<&sway::Output> = outputs.iter().collect();
    physical.sort_by_key(|o| (o.rect.y, o.rect.x));
    let monitor = |name: &str| {
        physical
            .iter()
            .position(|o| o.name == name)
            .unwrap_or(usize::MAX)
    };
    windows.sort_by_key(|w| {
        (
            w.area.unwrap_or(i32::MAX),
            monitor(&w.output),
            w.y,
            w.x,
            w.order,
        )
    });
}

/// Names and icons of applications, by window class.
struct Apps {
    by_key: HashMap<String, (String, Option<gio::Icon>)>,
}

impl Apps {
    fn load(dirs: &[String]) -> Self {
        let mut by_key = HashMap::new();
        let mut add = |info: &gio_unix::DesktopAppInfo| {
            let name = info.name().to_string();
            let icon = info.icon();
            let id = info
                .id()
                .map(|id| id.trim_end_matches(".desktop").to_string());
            let keys = [
                info.startup_wm_class().map(|c| c.to_string()),
                id.clone(),
                id.as_deref()
                    .and_then(|id| id.rsplit('.').next())
                    .map(str::to_string),
                Some(name.clone()),
                Some(name.chars().filter(|c| c.is_alphanumeric()).collect()),
            ];
            for key in keys.into_iter().flatten() {
                by_key
                    .entry(key.to_lowercase())
                    .or_insert_with(|| (name.clone(), icon.clone()));
            }
        };
        for dir in dirs {
            let dir = orbit_common::config::expand_home(dir.as_ref());
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut paths: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
            paths.sort();
            for path in paths
                .iter()
                .filter(|p| p.extension().is_some_and(|e| e == "desktop"))
            {
                if let Some(info) = gio_unix::DesktopAppInfo::from_filename(path) {
                    add(&info);
                }
            }
        }
        for info in gio::AppInfo::all() {
            if let Ok(info) = info.downcast::<gio_unix::DesktopAppInfo>() {
                add(&info);
            }
        }
        Self { by_key }
    }

    /// The application name and icon for a window class.
    fn lookup(&self, class: &str) -> (String, Option<gio::Icon>) {
        let c = class.to_lowercase();
        let candidates = [
            c.clone(),
            c.split('-').next().unwrap_or_default().to_string(),
            c.replace(' ', ""),
            c.rsplit('.').next().unwrap_or_default().to_string(),
        ];
        for key in candidates {
            if let Some(found) = self.by_key.get(&key) {
                return found.clone();
            }
        }
        let mut name: Vec<char> = class.chars().collect();
        if let Some(first) = name.first_mut() {
            *first = first.to_ascii_uppercase();
        }
        (name.into_iter().collect(), None)
    }
}

/// The class as an icon name if the theme has it, else a generic one.
fn fallback_icon(class: &str) -> String {
    let theme = gtk::gdk::Display::default().map(|d| gtk::IconTheme::for_display(&d));
    let name = class.to_lowercase();
    if theme.is_some_and(|t| t.has_icon(&name)) {
        name
    } else {
        "application-x-executable".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: i64, area: Option<i32>, output: &str, x: i32, y: i32) -> Window {
        Window {
            id,
            class: String::new(),
            title: String::new(),
            area,
            left: None,
            output: output.into(),
            x,
            y,
            focused: false,
            order: id as usize,
        }
    }

    fn output(name: &str, x: i32, y: i32) -> sway::Output {
        sway::Output {
            name: name.into(),
            active: true,
            focused: false,
            rect: sway::Rect { x, y },
        }
    }

    fn ids(windows: &[Window]) -> Vec<i64> {
        windows.iter().map(|w| w.id).collect()
    }

    #[test]
    fn stacked_monitors_top_one_first_then_reading_order() {
        // HDMI below the laptop panel, as on the hp; sway lists it first.
        let outputs = [output("HDMI-A-1", 0, 1080), output("eDP-1", 0, 0)];
        let mut w = vec![
            window(1, Some(1), "HDMI-A-1", 156, 1086),
            window(2, Some(1), "eDP-1", 1000, 48), // right half
            window(3, Some(1), "eDP-1", 156, 48),  // left half
            window(4, Some(1), "eDP-1", 156, 600), // below the left one
        ];
        sort(&mut w, &outputs);
        assert_eq!(ids(&w), [3, 2, 4, 1]);
    }

    #[test]
    fn side_by_side_monitors_left_one_first() {
        let outputs = [output("DP-1", 1920, 0), output("eDP-1", 0, 0)];
        let mut w = vec![
            window(1, Some(1), "DP-1", 1940, 10),
            window(2, Some(1), "eDP-1", 20, 500),
        ];
        sort(&mut w, &outputs);
        assert_eq!(ids(&w), [2, 1]);
    }

    #[test]
    fn grouped_by_area_minimized_last() {
        let outputs = [output("eDP-1", 0, 0)];
        let mut w = vec![
            window(1, None, "eDP-1", 0, 0),
            window(2, Some(2), "eDP-1", 0, 0),
            window(3, Some(1), "eDP-1", 500, 500),
        ];
        sort(&mut w, &outputs);
        assert_eq!(ids(&w), [3, 2, 1]);
    }
}
