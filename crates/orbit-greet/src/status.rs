//! The corner of the locked screen: keyboard layout, network and battery,
//! after the panel's status modules (PLANO-CORE §7.2), one glyph and one
//! line each.
//!
//! The layouts are the keymap's own (GDK reads them from the core's
//! keyboard); a click moves to the next one through the core's IPC, when
//! there is more than one. NetworkManager and UPower over D-Bus, read again
//! when they say something changed. No battery, no battery item.

use std::{cell::Cell, rc::Rc};

use gtk::{gdk, gio, glib, prelude::*};
use orbit_app::i18n::tr;
use orbit_common::sway;

const NM: &str = "org.freedesktop.NetworkManager";
const UPOWER: &str = "org.freedesktop.UPower";
const DISPLAY: &str = "/org/freedesktop/UPower/devices/DisplayDevice";

/// One item: a glyph and a line, in a button (only the keyboard's does
/// anything).
struct Item {
    button: gtk::Button,
    glyph: gtk::Label,
    line: gtk::Label,
}

impl Item {
    fn new(class: &str) -> Self {
        let glyph = gtk::Label::builder().css_classes(["glyph"]).build();
        let line = gtk::Label::builder().css_classes(["line"]).build();
        let content = gtk::Box::builder().spacing(5).build();
        content.append(&glyph);
        content.append(&line);
        let button = gtk::Button::builder().child(&content).css_classes(["item", class]).can_focus(false).build();
        Self { button, glyph, line }
    }

    fn show(&self, glyph: &str, line: &str) {
        self.glyph.set_label(glyph);
        self.line.set_label(line);
        self.button.set_visible(true);
    }
}

pub fn new(display: &gdk::Display) -> gtk::Widget {
    let row = gtk::Box::builder().spacing(10).css_classes(["status"]).halign(gtk::Align::End).valign(gtk::Align::Start).build();
    for item in [keyboard(display), network(), battery()].into_iter().flatten() {
        row.append(&item);
    }
    row.upcast()
}

fn keyboard(display: &gdk::Display) -> Option<gtk::Widget> {
    let device = display.default_seat()?.keyboard()?;
    let item = Rc::new(Item::new("keyboard"));
    let count = Rc::new(Cell::new(0));
    let show = {
        let (item, count) = (item.clone(), count.clone());
        move |device: &gdk::Device| {
            let names = device.layout_names();
            count.set(names.len());
            let active = usize::try_from(device.active_layout_index()).ok().and_then(|i| names.get(i));
            match active {
                Some(name) => {
                    item.show("\u{f030c}", short_layout(name)); // 󰌌
                    item.button.set_tooltip_text(Some(name));
                }
                None => item.button.set_visible(false),
            }
        }
    };
    show(&device);
    device.connect_active_layout_index_notify(show.clone());
    device.connect_changed(show);
    item.button.connect_clicked(move |_| {
        if count.get() > 1 {
            let sent = sway::Connection::open().and_then(|mut c| c.request(sway::RUN_COMMAND, "input type:keyboard xkb_switch_layout next"));
            if let Err(err) = sent {
                log::warn!("keyboard: no layout switch through the core's IPC: {err}");
            }
        }
    });
    Some(item.button.clone().upcast())
}

/// "English (US, alt. intl.)" → "US, alt. intl."; a name without the
/// parentheses stays whole.
fn short_layout(name: &str) -> &str {
    name.split_once(" (").map_or(name, |(_, rest)| rest.trim_end_matches(')'))
}

fn network() -> Option<gtk::Widget> {
    let conn = system_bus()?;
    let item = Rc::new(Item::new("network"));
    let refresh = {
        let (item, conn) = (item.clone(), conn.clone());
        move || show_network(&item, &conn)
    };
    refresh();
    let sub = on_changes(&conn, NM, None, refresh);
    hold(&item.button, sub);
    Some(item.button.clone().upcast())
}

fn show_network(item: &Item, conn: &gio::DBusConnection) {
    let get = |path: &str, iface: &str, name: &str| property(conn, NM, path, iface, name).and_then(|v| v.str().map(str::to_owned));
    let Some(active) = get("/org/freedesktop/NetworkManager", NM, "PrimaryConnection").filter(|p| p != "/") else {
        item.show("\u{f05aa}", &tr("network-none")); // 󰖪
        return;
    };
    let iface = "org.freedesktop.NetworkManager.Connection.Active";
    let id = get(&active, iface, "Id").unwrap_or_default();
    let glyph = match get(&active, iface, "Type").as_deref() {
        Some("802-11-wireless") => "\u{f05a9}", // 󰖩
        Some("vpn" | "wireguard") => "\u{f0582}", // 󰖂
        _ => "\u{f0200}", // 󰈀
    };
    item.show(glyph, &id);
}

fn battery() -> Option<gtk::Widget> {
    let conn = system_bus()?;
    let item = Rc::new(Item::new("battery"));
    let refresh = {
        let (item, conn) = (item.clone(), conn.clone());
        move || show_battery(&item, &conn)
    };
    refresh();
    let sub = on_changes(&conn, UPOWER, Some(DISPLAY), refresh);
    hold(&item.button, sub);
    Some(item.button.clone().upcast())
}

fn show_battery(item: &Item, conn: &gio::DBusConnection) {
    let get = |name: &str| property(conn, UPOWER, DISPLAY, "org.freedesktop.UPower.Device", name);
    let present = get("IsPresent").and_then(|v| v.get::<bool>()).unwrap_or(false) && get("Type").and_then(|v| v.get::<u32>()) == Some(2); // 2 = battery
    if !present {
        item.button.set_visible(false);
        return;
    }
    let percent = get("Percentage").and_then(|v| v.get::<f64>()).unwrap_or_default();
    // UPower states: 1 charging, 4 fully charged.
    let charging = matches!(get("State").and_then(|v| v.get::<u32>()), Some(1 | 4));
    let glyph = match percent {
        _ if charging => "\u{f0084}",  // 󰂄
        p if p >= 90.0 => "\u{f0079}", // 󰁹
        p if p >= 70.0 => "\u{f0080}", // 󰂀
        p if p >= 45.0 => "\u{f007e}", // 󰁾
        p if p >= 20.0 => "\u{f007b}", // 󰁻
        _ => "\u{f007a}",              // 󰁺
    };
    item.show(glyph, &format!("{percent:.0}%"));
}

fn system_bus() -> Option<gio::DBusConnection> {
    gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE).inspect_err(|err| log::warn!("status: no system bus: {err}")).ok()
}

/// Reads one D-Bus property.
fn property(conn: &gio::DBusConnection, dest: &str, path: &str, iface: &str, name: &str) -> Option<glib::Variant> {
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

/// Calls `refresh` once, on the next idle, after any PropertiesChanged
/// from `sender` (NetworkManager sends them in bursts).
fn on_changes(conn: &gio::DBusConnection, sender: &str, path: Option<&str>, refresh: impl Fn() + 'static) -> gio::SignalSubscription {
    let refresh = Rc::new(refresh);
    let pending = Rc::new(Cell::new(false));
    conn.subscribe_to_signal(Some(sender), Some("org.freedesktop.DBus.Properties"), Some("PropertiesChanged"), path, None, gio::DBusSignalFlags::NONE, move |_| {
        if pending.replace(true) {
            return;
        }
        let (refresh, pending) = (refresh.clone(), pending.clone());
        glib::idle_add_local_once(move || {
            pending.set(false);
            refresh();
        });
    })
}

/// Keeps `keep` alive as long as `widget` exists.
fn hold<T: 'static>(widget: &impl IsA<gtk::Widget>, keep: T) {
    widget.connect_destroy(move |_| {
        let _ = &keep;
    });
}

#[cfg(test)]
mod tests {
    use super::short_layout;

    #[test]
    fn layout_names_lose_the_language() {
        assert_eq!(short_layout("English (US, alt. intl.)"), "US, alt. intl.");
        assert_eq!(short_layout("Portuguese (Brazil)"), "Brazil");
        assert_eq!(short_layout("us"), "us");
    }
}
