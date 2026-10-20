//! Status modules, after the network/keyboard/battery card of the eww
//! panel — now three independent modules sharing one shape: a larger icon
//! with two lines of text beside it.
//!
//!   network   connection name     /  IP address and prefix
//!   keyboard  language            /  the rest of the layout name
//!   battery   charge percentage   /  uptime
//!
//! Event-driven: NetworkManager and UPower over D-Bus, the keyboard layout
//! over sway IPC. Only the uptime ticks, once a minute.
//!
//! The network icon is a button, as in XFCE: left click opens the connection
//! editor, right click the network menu (nm-applet's own, when it runs).
//!
//! Block form: icon 30 px (glyph 24 px) next to two 15 px lines, card
//! 5 + 30 + 5 = 40 px. Line form: icon and first line; second in the tooltip.

use std::{cell::RefCell, rc::Rc};

use gtk::{gio, glib, prelude::*};
use serde::Deserialize;

use orbit_app::i18n::{number, text, tr, tr_args};

use crate::{dbusmenu, sway};

/// None of the status modules has options yet; their files may be empty.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StatusConfig {}

/// The shared shape: icon, first line, second line.
struct Row {
    root: gtk::Widget,
    icon: gtk::Label,
    first: gtk::Label,
    second: gtk::Label,
    vertical: bool,
}

impl Row {
    fn new(class: &str, vertical: bool) -> Rc<Row> {
        let icon = gtk::Label::builder()
            .css_classes(["icon"])
            .valign(gtk::Align::Center)
            .build();
        let line = |class: &str| {
            gtk::Label::builder()
                .css_classes([class])
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build()
        };
        let (first, second) = (line("first"), line("second"));
        let root = gtk::Box::builder()
            .spacing(5)
            .css_classes(if vertical {
                vec!["card", "status", class]
            } else {
                vec!["status-line", class]
            })
            .build();
        root.append(&icon);
        if vertical {
            let lines = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .valign(gtk::Align::Center)
                .hexpand(true)
                .build();
            lines.append(&first);
            lines.append(&second);
            root.append(&lines);
        } else {
            root.append(&first);
        }
        Rc::new(Row {
            root: root.upcast(),
            icon,
            first,
            second,
            vertical,
        })
    }

    fn show(&self, glyph: &str, first: &str, second: &str, tooltip: &str) {
        self.icon.set_label(glyph);
        self.first.set_label(first);
        self.second.set_label(second);
        let tooltip = if self.vertical {
            tooltip.to_string()
        } else {
            [first, second, tooltip]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        };
        self.root.set_tooltip_text(Some(&tooltip));
    }
}

/// Reads one D-Bus property.
fn property(
    conn: &gio::DBusConnection,
    dest: &str,
    path: &str,
    iface: &str,
    name: &str,
) -> Option<glib::Variant> {
    conn.call_sync(
        Some(dest),
        path,
        "org.freedesktop.DBus.Properties",
        "Get",
        Some(&(iface, name).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        1000,
        gio::Cancellable::NONE,
    )
    .ok()?
    .child_value(0)
    .as_variant()
}

/// Calls `refresh` once, on the next idle, after any PropertiesChanged from
/// `sender` — NetworkManager sends them in bursts.
fn on_changes(
    conn: &gio::DBusConnection,
    sender: &str,
    path: Option<&str>,
    refresh: impl Fn() + 'static,
) -> gio::SignalSubscription {
    let refresh = Rc::new(refresh);
    let pending = Rc::new(std::cell::Cell::new(false));
    conn.subscribe_to_signal(
        Some(sender),
        Some("org.freedesktop.DBus.Properties"),
        Some("PropertiesChanged"),
        path,
        None,
        gio::DBusSignalFlags::NONE,
        move |_| {
            if pending.replace(true) {
                return;
            }
            let (refresh, pending) = (refresh.clone(), pending.clone());
            glib::idle_add_local_once(move || {
                pending.set(false);
                refresh();
            });
        },
    )
}

fn system_bus() -> Option<gio::DBusConnection> {
    gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)
        .inspect_err(|err| log::error!("status: no system bus: {err}"))
        .ok()
}

/// Keeps `keep` alive as long as `widget` exists.
fn hold<T: 'static>(widget: &gtk::Widget, keep: T) {
    widget.connect_destroy(move |_| {
        let _ = &keep;
    });
}

// ------------------------------------------------------------- network

const NM: &str = "org.freedesktop.NetworkManager";

/// `modules/network.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// Opened (or focused) by a left click on the icon.
    pub editor: String,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            editor: "nm-connection-editor".into(),
        }
    }
}

pub fn network(config: &NetworkConfig, vertical: bool) -> gtk::Widget {
    let row = Row::new("network", vertical);
    let Some(conn) = system_bus() else {
        return row.root.clone();
    };
    network_clicks(&row.icon, config, &conn);
    let refresh = {
        let (row, conn) = (row.clone(), conn.clone());
        move || show_network(&row, &conn)
    };
    refresh();
    let sub = on_changes(&conn, NM, None, refresh);
    hold(&row.root, sub);
    row.root.clone()
}

/// Left click: the connection editor. Right click: nm-applet's menu — the
/// one XFCE shows, with the networks to join, VPNs and so on — or, without
/// nm-applet, a small menu of our own.
fn network_clicks(icon: &gtk::Label, config: &NetworkConfig, system: &gio::DBusConnection) {
    icon.add_css_class("button");
    icon.set_cursor_from_name(Some("pointer"));

    let left = gtk::GestureClick::builder().button(1).build();
    let editor = config.editor.clone();
    left.connect_released(move |_, _, _, _| open_or_focus(&editor));
    icon.add_controller(left);

    let right = gtk::GestureClick::builder().button(3).build();
    let (anchor, system, editor) = (icon.downgrade(), system.clone(), config.editor.clone());
    right.connect_pressed(move |_, _, x, y| {
        let Some(anchor) = anchor.upgrade() else {
            return;
        };
        let at = Some((x, y));
        let session = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok();
        match session.as_ref().and_then(nm_applet_menu) {
            Some((bus, path)) => {
                dbusmenu::show(session.as_ref().expect("checked"), &bus, &path, &anchor, at)
            }
            None => fallback_menu(&anchor, &system, &editor, at),
        }
    });
    icon.add_controller(right);
}

/// Focuses the program's window if it is open (sway), or starts it.
fn open_or_focus(command: &str) {
    let program = command.split_whitespace().next().unwrap_or_default();
    let name = program.rsplit('/').next().unwrap_or(program);
    if sway::available() && !name.is_empty() {
        let focus = format!("[app_id=\"(?i){name}\"] focus");
        let focused = sway::Connection::open()
            .and_then(|mut c| c.request(sway::RUN_COMMAND, &focus))
            .ok()
            .and_then(|r| serde_json::from_slice::<Vec<sway::CommandResult>>(&r).ok())
            .is_some_and(|r| r.iter().all(|r| r.success));
        if focused {
            return;
        }
    }
    if let Err(err) = glib::spawn_command_line_async(command) {
        log::error!("network: running {command:?}: {err}");
    }
}

/// (bus name, menu path) of nm-applet's tray item, if it runs.
fn nm_applet_menu(session: &gio::DBusConnection) -> Option<(String, String)> {
    const WATCHER: &str = "org.kde.StatusNotifierWatcher";
    let items = property(
        session,
        WATCHER,
        "/StatusNotifierWatcher",
        WATCHER,
        "RegisteredStatusNotifierItems",
    )?
    .get::<Vec<String>>()?;
    items.iter().find_map(|item| {
        let (bus, path) = crate::sni::split(item);
        let id = property(session, bus, path, crate::sni::ITEM, "Id")?;
        if id.str()? != "nm-applet" {
            return None;
        }
        let menu = property(session, bus, path, crate::sni::ITEM, "Menu")?;
        Some((bus.to_string(), menu.str()?.to_string()))
    })
}

/// Without nm-applet: networking and Wi-Fi on/off, and the editor.
fn fallback_menu(
    anchor: &gtk::Label,
    system: &gio::DBusConnection,
    editor: &str,
    at: orbit_app::popup::Point,
) {
    let flag = |name: &str| {
        property(system, NM, "/org/freedesktop/NetworkManager", NM, name)
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false)
    };
    let group = gio::SimpleActionGroup::new();
    let networking = gio::SimpleAction::new_stateful(
        "networking",
        None,
        &flag("NetworkingEnabled").to_variant(),
    );
    {
        let system = system.clone();
        networking.connect_activate(move |action, _| {
            let on = !action
                .state()
                .and_then(|s| s.get::<bool>())
                .unwrap_or(false);
            system.call(
                Some(NM),
                "/org/freedesktop/NetworkManager",
                NM,
                "Enable",
                Some(&(on,).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                5000,
                gio::Cancellable::NONE,
                |r| {
                    if let Err(e) = r {
                        log::warn!("network: {e}")
                    }
                },
            );
        });
    }
    let wifi = gio::SimpleAction::new_stateful("wifi", None, &flag("WirelessEnabled").to_variant());
    {
        let system = system.clone();
        wifi.connect_activate(move |action, _| {
            let on = !action
                .state()
                .and_then(|s| s.get::<bool>())
                .unwrap_or(false);
            let value = glib::Variant::from_variant(&on.to_variant());
            system.call(
                Some(NM),
                "/org/freedesktop/NetworkManager",
                "org.freedesktop.DBus.Properties",
                "Set",
                Some(&(NM, "WirelessEnabled", value).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                5000,
                gio::Cancellable::NONE,
                |r| {
                    if let Err(e) = r {
                        log::warn!("network: {e}")
                    }
                },
            );
        });
    }
    let edit = gio::SimpleAction::new("edit", None);
    {
        let editor = editor.to_string();
        edit.connect_activate(move |_, _| open_or_focus(&editor));
    }
    for action in [&networking, &wifi, &edit] {
        group.add_action(action);
    }
    let model = gio::Menu::new();
    model.append(Some(&tr("network-menu-networking")), Some("n.networking"));
    model.append(Some(&tr("network-menu-wifi")), Some("n.wifi"));
    model.append(Some(&tr("network-menu-edit")), Some("n.edit"));

    let popover = gtk::PopoverMenu::from_model(Some(&model));
    popover.insert_action_group("n", Some(&group));
    orbit_app::popup::open(&popover, anchor, at);
}

fn show_network(row: &Row, conn: &gio::DBusConnection) {
    let get = |path: &str, iface: &str, name: &str| property(conn, NM, path, iface, name);
    let primary = get("/org/freedesktop/NetworkManager", NM, "PrimaryConnection")
        .and_then(|v| v.str().map(str::to_owned))
        .filter(|p| p != "/");
    let Some(active) = primary else {
        row.show(
            "\u{f05aa}",
            &tr("network-none"),
            "",
            &tr("network-none-tooltip"),
        ); // 󰖪
        return;
    };
    let iface = "org.freedesktop.NetworkManager.Connection.Active";
    let id = get(&active, iface, "Id")
        .and_then(|v| v.str().map(str::to_owned))
        .unwrap_or_default();
    let kind = get(&active, iface, "Type")
        .and_then(|v| v.str().map(str::to_owned))
        .unwrap_or_default();
    let address = get(&active, iface, "Ip4Config")
        .and_then(|v| v.str().map(str::to_owned))
        .filter(|p| p != "/")
        .and_then(|config| {
            get(
                &config,
                "org.freedesktop.NetworkManager.IP4Config",
                "AddressData",
            )
        })
        .and_then(|data| {
            let first = glib::VariantDict::new(Some(&data.try_child_value(0)?));
            let address = first.lookup::<String>("address").ok()??;
            let prefix = first.lookup::<u32>("prefix").ok()??;
            Some(format!("{address}/{prefix}"))
        })
        .unwrap_or_else(|| tr("network-no-address"));
    let (glyph, kind_name) = match kind.as_str() {
        "802-11-wireless" => ("\u{f05a9}", tr("network-kind-wifi")), // 󰖩
        "802-3-ethernet" => ("\u{f0200}", tr("network-kind-wired")), // 󰈀
        "vpn" | "wireguard" => ("\u{f0582}", tr("network-kind-vpn")), // 󰖂
        _ => ("\u{f0200}", kind.clone()),
    };
    row.show(
        glyph,
        &id,
        &address,
        &format!("{id} ({kind_name}) — {address}"),
    );
}

// ------------------------------------------------------------ keyboard

pub fn keyboard(_: &StatusConfig, vertical: bool) -> gtk::Widget {
    let row = Row::new("keyboard", vertical);
    if !sway::available() {
        log::warn!("keyboard: not running under sway");
        return row.root.clone();
    }
    let layouts: Rc<RefCell<usize>> = Rc::default();
    let refresh = {
        let (row, layouts) = (row.clone(), layouts.clone());
        move || show_keyboard(&row, &layouts)
    };
    refresh();
    sway::subscribe(&["input"], refresh);

    // A click moves to the next layout, when there is more than one.
    let click = gtk::GestureClick::new();
    click.connect_released(move |_, _, _, _| {
        if *layouts.borrow() > 1 {
            let _ = sway::Connection::open().and_then(|mut c| {
                c.request(
                    sway::RUN_COMMAND,
                    "input type:keyboard xkb_switch_layout next",
                )
            });
        }
    });
    row.root.add_controller(click);
    row.root.clone()
}

fn show_keyboard(row: &Row, layouts: &RefCell<usize>) {
    let inputs: Vec<serde_json::Value> = sway::Connection::open()
        .and_then(|mut c| c.query(sway::GET_INPUTS))
        .unwrap_or_default();
    // The first real keyboard: the one with a list of layouts.
    let keyboard = inputs.iter().find(|i| {
        i["type"] == "keyboard"
            && i["xkb_layout_names"]
                .as_array()
                .is_some_and(|l| !l.is_empty())
    });
    let Some(keyboard) = keyboard else { return };
    let name = keyboard["xkb_active_layout_name"]
        .as_str()
        .unwrap_or_default();
    *layouts.borrow_mut() = keyboard["xkb_layout_names"].as_array().map_or(0, Vec::len);
    let (language, rest) = split_layout(name);
    let all: Vec<&str> = keyboard["xkb_layout_names"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l.as_str())
        .collect();
    let tooltip = if all.len() > 1 {
        format!(
            "{}\n{}",
            tr_args("keyboard-next", &[("name", text(name))]),
            all.join("\n")
        )
    } else {
        name.to_string()
    };
    row.show("\u{f030c}", language, rest, &tooltip); // 󰌌
}

/// "English (US, alt. intl.)" -> ("English", "US, alt. intl.").
fn split_layout(name: &str) -> (&str, &str) {
    match name.split_once(" (") {
        Some((language, rest)) => (language, rest.trim_end_matches(')')),
        None => (name, ""),
    }
}

// ------------------------------------------------------------- battery

const UPOWER: &str = "org.freedesktop.UPower";
const DISPLAY: &str = "/org/freedesktop/UPower/devices/DisplayDevice";

pub fn battery(_: &StatusConfig, vertical: bool) -> gtk::Widget {
    let row = Row::new("battery", vertical);
    let conn = system_bus();
    let refresh = {
        let (row, conn) = (row.clone(), conn.clone());
        move || show_battery(&row, conn.as_ref())
    };
    refresh();
    if let Some(conn) = &conn {
        let sub = on_changes(conn, UPOWER, Some(DISPLAY), refresh.clone());
        hold(&row.root, sub);
    }
    // The uptime moves on its own: once a minute is enough for "5h 58min".
    glib::timeout_add_seconds_local(60, move || {
        refresh();
        glib::ControlFlow::Continue
    });
    row.root.clone()
}

fn show_battery(row: &Row, conn: Option<&gio::DBusConnection>) {
    let get = |name: &str| {
        conn.and_then(|c| property(c, UPOWER, DISPLAY, "org.freedesktop.UPower.Device", name))
    };
    let present = get("IsPresent")
        .and_then(|v| v.get::<bool>())
        .unwrap_or(false)
        && get("Type").and_then(|v| v.get::<u32>()) == Some(2); // 2 = battery
    let uptime = uptime();
    if !present {
        row.show(
            "\u{f06a5}",
            &tr("battery-none"),
            &uptime,
            &tr_args("battery-uptime", &[("uptime", text(uptime.clone()))]),
        ); // 󰚥
        return;
    }
    let percent = get("Percentage")
        .and_then(|v| v.get::<f64>())
        .unwrap_or_default();
    // UPower states: 1 charging, 2 discharging, 4 fully charged, 5 pending charge.
    let state = get("State")
        .and_then(|v| v.get::<u32>())
        .unwrap_or_default();
    let charging = matches!(state, 1 | 4);
    let glyph = match percent {
        _ if charging => "\u{f0084}",  // 󰂄
        p if p >= 90.0 => "\u{f0079}", // 󰁹
        p if p >= 70.0 => "\u{f0080}", // 󰂀
        p if p >= 45.0 => "\u{f007e}", // 󰁾
        p if p >= 20.0 => "\u{f007b}", // 󰁻
        _ => "\u{f007a}",              // 󰁺
    };
    let state_name = match state {
        1 => tr("battery-charging"),
        2 => tr("battery-discharging"),
        4 => tr("battery-full"),
        5 => tr("battery-pending"),
        _ => String::new(),
    };
    let remaining = match state {
        1 => get("TimeToFull"),
        2 => get("TimeToEmpty"),
        _ => None,
    }
    .and_then(|v| v.get::<i64>())
    .filter(|s| *s > 0)
    .map(|s| tr_args("battery-remaining", &[("time", text(duration(s as u64)))]))
    .unwrap_or_default();
    row.show(
        glyph,
        &format!("{percent:.0}%"),
        &uptime,
        &format!(
            "{}\n{}",
            tr_args(
                "battery-tooltip",
                &[
                    ("percent", number(percent.round())),
                    ("state", text(state_name)),
                    ("remaining", text(remaining)),
                ],
            ),
            tr_args("battery-uptime", &[("uptime", text(uptime.clone()))])
        ),
    );
}

fn uptime() -> String {
    let seconds = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|t| t.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or_default();
    duration(seconds as u64)
}

/// 5h 58min, 2d 3h, 12min.
fn duration(seconds: u64) -> String {
    let (days, hours, minutes) = (seconds / 86_400, seconds / 3600 % 24, seconds / 60 % 60);
    match (days, hours) {
        (0, 0) => format!("{minutes}min"),
        (0, h) => format!("{h}h {minutes}min"),
        (d, h) => format!("{d}d {h}h"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_names_split_into_language_and_rest() {
        assert_eq!(
            split_layout("English (US, alt. intl.)"),
            ("English", "US, alt. intl.")
        );
        assert_eq!(
            split_layout("Portuguese (Brazil)"),
            ("Portuguese", "Brazil")
        );
        assert_eq!(split_layout("English"), ("English", ""));
    }

    /// Needs a session with nm-applet running: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn finds_nm_applet_menu_on_this_session() {
        let session = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).unwrap();
        let (bus, path) = nm_applet_menu(&session).expect("nm-applet item with a menu");
        eprintln!("nm-applet menu: {bus} {path}");
        assert!(path.starts_with('/'));
    }

    #[test]
    fn durations() {
        assert_eq!(duration(12 * 60), "12min");
        assert_eq!(duration(5 * 3600 + 58 * 60), "5h 58min");
        assert_eq!(duration(2 * 86_400 + 3 * 3600 + 5), "2d 3h");
    }
}
