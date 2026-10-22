//! Network module: the connection NetworkManager calls primary, over the
//! system bus. Same shape and sizes as the status modules.
//!
//!   first line   the connection's name
//!   second line  its IP address and prefix
//!   Full         its kind (Wi-Fi, wired, VPN)
//!
//! Event-driven: NetworkManager's PropertiesChanged, in bursts, taken once.
//!
//! Left click, the quick part: switches for networking and Wi-Fi, the Wi-Fi
//! networks in reach (the strongest first; a new protected one asks for its
//! password through the session's secret agent, nm-applet's), and the VPNs.
//! Right click, the full part: the connection editor (`editor`).

use std::collections::HashMap;

use gtk::{gio, glib, prelude::*};
use serde::Deserialize;

use orbit_app::i18n::tr;

use crate::{
    board::Form,
    status::{Row, clicks, hold, on_changes, open_or_focus, popup, property, quick_button, quick_list, quick_title, switch_row, system_bus},
};

const NM: &str = "org.freedesktop.NetworkManager";
const ROOT: &str = "/org/freedesktop/NetworkManager";
const SETTINGS: &str = "/org/freedesktop/NetworkManager/Settings";
const DEVICE: &str = "org.freedesktop.NetworkManager.Device";
const WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const AP: &str = "org.freedesktop.NetworkManager.AccessPoint";
const ACTIVE: &str = "org.freedesktop.NetworkManager.Connection.Active";

/// How many Wi-Fi networks the quick part lists.
const NETWORKS: usize = 8;

/// `modules/network.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkConfig {
    /// Opened (or focused) by a right click.
    pub editor: String,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            editor: "nm-connection-editor".into(),
        }
    }
}

pub fn new(config: &NetworkConfig, form: Form) -> gtk::Widget {
    let row = Row::new("network", form);
    let Some(conn) = system_bus() else {
        return row.root.clone();
    };
    let refresh = {
        let (row, conn) = (row.clone(), conn.clone());
        move || show(&row, &conn)
    };
    refresh();
    let sub = on_changes(&conn, NM, None, refresh);
    hold(&row.root, sub);

    let (anchor, editor) = (row.root.downgrade(), config.editor.clone());
    let right = config.editor.clone();
    clicks(
        &row.root,
        move || {
            if let Some(anchor) = anchor.upgrade() {
                quick(&conn, &anchor, &editor);
            }
        },
        move || open_or_focus(&right),
        None,
    );
    row.root.clone()
}

fn show(row: &Row, conn: &gio::DBusConnection) {
    let get = |path: &str, iface: &str, name: &str| property(conn, NM, path, iface, name);
    let primary = get(ROOT, NM, "PrimaryConnection")
        .and_then(|v| v.str().map(str::to_owned))
        .filter(|p| p != "/");
    let hint = tr("network-tooltip");
    let Some(active) = primary else {
        row.show(
            "\u{f05aa}",
            &[&tr("network-none")],
            &format!("{}\n{hint}", tr("network-none-tooltip")),
        ); // 󰖪
        return;
    };
    let id = get(&active, ACTIVE, "Id")
        .and_then(|v| v.str().map(str::to_owned))
        .unwrap_or_default();
    let kind = get(&active, ACTIVE, "Type")
        .and_then(|v| v.str().map(str::to_owned))
        .unwrap_or_default();
    let address = get(&active, ACTIVE, "Ip4Config")
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
        &[&id, &address, &kind_name],
        &format!("{id} ({kind_name}) — {address}\n{hint}"),
    );
}

/// A saved connection.
#[derive(Debug, Clone)]
struct Saved {
    path: String,
    id: String,
    kind: String,
    ssid: Option<Vec<u8>>,
}

/// A Wi-Fi network in reach: its strongest access point.
#[derive(Debug, Clone)]
struct Network {
    ap: String,
    ssid: Vec<u8>,
    strength: u8,
    secure: bool,
}

/// The quick part, read afresh each time it opens.
fn quick(conn: &gio::DBusConnection, anchor: &gtk::Widget, editor: &str) {
    let flag = |name: &str| {
        property(conn, NM, ROOT, NM, name)
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false)
    };
    let list = quick_list("network-quick");

    let networking = switch_row(&tr("network-menu-networking"), flag("NetworkingEnabled"), &list);
    {
        let conn = conn.clone();
        networking.connect_state_set(move |_, on| {
            call(&conn, ROOT, NM, "Enable", Some((on,).to_variant()));
            glib::Propagation::Proceed
        });
    }
    let wifi_on = flag("WirelessEnabled");
    let wifi = switch_row(&tr("network-menu-wifi"), wifi_on, &list);
    {
        let conn = conn.clone();
        wifi.connect_state_set(move |_, on| {
            let value = glib::Variant::from_variant(&on.to_variant());
            call(
                &conn,
                ROOT,
                "org.freedesktop.DBus.Properties",
                "Set",
                Some((NM, "WirelessEnabled", value).to_variant()),
            );
            glib::Propagation::Proceed
        });
    }

    let saved = saved(conn);
    // Settings path -> its active connection.
    let active: HashMap<String, String> = paths(property(conn, NM, ROOT, NM, "ActiveConnections"))
        .into_iter()
        .filter_map(|a| {
            let settings = property(conn, NM, &a, ACTIVE, "Connection")?.str()?.to_owned();
            Some((settings, a))
        })
        .collect();

    if let Some(device) = wifi_on.then(|| wifi_device(conn)).flatten() {
        // Fresh results for the next time; this one shows what NM has.
        call(conn, &device, WIRELESS, "RequestScan", Some((glib::VariantDict::new(None).end(),).to_variant()));
        let current = property(conn, NM, &device, WIRELESS, "ActiveAccessPoint")
            .and_then(|v| v.str().map(str::to_owned))
            .and_then(|ap| property(conn, NM, &ap, AP, "Ssid"))
            .and_then(|v| v.get::<Vec<u8>>());
        let networks = networks(conn, &device);
        if !networks.is_empty() {
            quick_title(&tr("network-quick-wifi"), &list);
        }
        for network in networks.into_iter().take(NETWORKS) {
            let name = String::from_utf8_lossy(&network.ssid).into_owned();
            let label = format!(
                "{} {name}{}",
                signal(network.strength),
                if network.secure { " \u{f033e}" } else { "" }, // 󰌾
            );
            let button = quick_button(&label, &list, {
                let (conn, device) = (conn.clone(), device.clone());
                let known = saved.iter().find(|s| s.ssid.as_deref() == Some(&network.ssid)).map(|s| s.path.clone());
                move || join(&conn, &device, &network, known.as_deref())
            });
            if current.as_deref() == Some(name.as_bytes()) {
                button.add_css_class("active");
            }
        }
    }

    let vpns: Vec<&Saved> = saved.iter().filter(|s| matches!(s.kind.as_str(), "vpn" | "wireguard")).collect();
    if !vpns.is_empty() {
        quick_title(&tr("network-kind-vpn"), &list);
    }
    for vpn in vpns {
        let on = active.get(&vpn.path).cloned();
        let switch = switch_row(&vpn.id, on.is_some(), &list);
        let (conn, path) = (conn.clone(), vpn.path.clone());
        switch.connect_state_set(move |_, want| {
            match (&on, want) {
                (None, true) => call(&conn, ROOT, NM, "ActivateConnection", Some((op(&path), op("/"), op("/")).to_variant())),
                (Some(active), false) => call(&conn, ROOT, NM, "DeactivateConnection", Some((op(active),).to_variant())),
                _ => {}
            }
            glib::Propagation::Proceed
        });
    }

    let editor = editor.to_string();
    quick_button(&tr("network-menu-edit"), &list, move || open_or_focus(&editor));
    popup(&list, anchor);
}

/// Connects to `network`: its saved connection, or a new one — NM then
/// asks the secret agent for the password.
fn join(conn: &gio::DBusConnection, device: &str, network: &Network, saved: Option<&str>) {
    match saved {
        Some(path) => call(conn, ROOT, NM, "ActivateConnection", Some((op(path), op(device), op(&network.ap)).to_variant())),
        None => {
            let settings: HashMap<String, HashMap<String, glib::Variant>> = HashMap::new();
            call(conn, ROOT, NM, "AddAndActivateConnection", Some((settings, op(device), op(&network.ap)).to_variant()));
        }
    }
}

/// The first Wi-Fi device.
fn wifi_device(conn: &gio::DBusConnection) -> Option<String> {
    let devices = conn
        .call_sync(Some(NM), ROOT, NM, "GetDevices", None, None, gio::DBusCallFlags::NONE, 1000, gio::Cancellable::NONE)
        .ok()?;
    paths(Some(devices.child_value(0)))
        .into_iter()
        .find(|d| property(conn, NM, d, DEVICE, "DeviceType").and_then(|v| v.get::<u32>()) == Some(2))
}

/// The networks `device` sees, one per name, the strongest first.
fn networks(conn: &gio::DBusConnection, device: &str) -> Vec<Network> {
    let reply = conn.call_sync(Some(NM), device, WIRELESS, "GetAllAccessPoints", None, None, gio::DBusCallFlags::NONE, 1000, gio::Cancellable::NONE);
    let mut best: HashMap<Vec<u8>, Network> = HashMap::new();
    for ap in paths(reply.ok().map(|r| r.child_value(0))) {
        let get = |name: &str| property(conn, NM, &ap, AP, name);
        let Some(ssid) = get("Ssid").and_then(|v| v.get::<Vec<u8>>()).filter(|s| !s.is_empty()) else {
            continue;
        };
        let strength = get("Strength").and_then(|v| v.get::<u8>()).unwrap_or(0);
        let secure = ["Flags", "WpaFlags", "RsnFlags"]
            .iter()
            .any(|f| get(f).and_then(|v| v.get::<u32>()).unwrap_or(0) != 0);
        if best.get(&ssid).is_none_or(|n| n.strength < strength) {
            best.insert(ssid.clone(), Network { ap, ssid, strength, secure });
        }
    }
    let mut networks: Vec<Network> = best.into_values().collect();
    networks.sort_by(|a, b| b.strength.cmp(&a.strength).then_with(|| a.ssid.cmp(&b.ssid)));
    networks
}

/// Every saved connection, with its kind and, for Wi-Fi, its network name.
fn saved(conn: &gio::DBusConnection) -> Vec<Saved> {
    const CONNECTION: &str = "org.freedesktop.NetworkManager.Settings.Connection";
    let list = conn.call_sync(
        Some(NM),
        SETTINGS,
        "org.freedesktop.NetworkManager.Settings",
        "ListConnections",
        None,
        None,
        gio::DBusCallFlags::NONE,
        1000,
        gio::Cancellable::NONE,
    );
    paths(list.ok().map(|r| r.child_value(0)))
        .into_iter()
        .filter_map(|path| {
            let reply = conn
                .call_sync(Some(NM), &path, CONNECTION, "GetSettings", None, None, gio::DBusCallFlags::NONE, 1000, gio::Cancellable::NONE)
                .ok()?;
            // a{sa{sv}}: each setting and its values.
            let groups = reply.child_value(0);
            let group = |name: &str| {
                (0..groups.n_children())
                    .map(|k| groups.child_value(k))
                    .find(|entry| entry.child_value(0).str() == Some(name))
                    .map(|entry| glib::VariantDict::new(Some(&entry.child_value(1))))
            };
            let connection = group("connection")?;
            Some(Saved {
                id: connection.lookup::<String>("id").ok().flatten().unwrap_or_default(),
                kind: connection.lookup::<String>("type").ok().flatten().unwrap_or_default(),
                ssid: group("802-11-wireless").and_then(|w| w.lookup::<Vec<u8>>("ssid").ok().flatten()),
                path,
            })
        })
        .collect()
}

/// The object paths in an `ao` value.
fn paths(value: Option<glib::Variant>) -> Vec<String> {
    let Some(value) = value else { return Vec::new() };
    (0..value.n_children())
        .filter_map(|i| value.child_value(i).str().map(str::to_owned))
        .collect()
}

fn op(path: &str) -> glib::variant::ObjectPath {
    glib::variant::ObjectPath::try_from(path.to_owned()).unwrap_or_else(|_| glib::variant::ObjectPath::try_from("/".to_owned()).expect("root path"))
}

/// Calls NetworkManager without waiting; failures go to the log.
fn call(conn: &gio::DBusConnection, path: &str, iface: &str, method: &str, args: Option<glib::Variant>) {
    let method_name = method.to_owned();
    conn.call(
        Some(NM),
        path,
        iface,
        method,
        args.as_ref(),
        None,
        gio::DBusCallFlags::NONE,
        30_000,
        gio::Cancellable::NONE,
        move |r| {
            if let Err(e) = r {
                log::warn!("network: {method_name}: {e}");
            }
        },
    );
}

/// The signal's glyph, in five steps.
fn signal(strength: u8) -> &'static str {
    match strength {
        80.. => "\u{f0928}", // 󰤨
        60.. => "\u{f0925}", // 󰤥
        40.. => "\u{f0922}", // 󰤢
        20.. => "\u{f091f}", // 󰤟
        _ => "\u{f092f}",    // 󰤯
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs NetworkManager: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn reads_this_machine() {
        let conn = system_bus().unwrap();
        let saved = saved(&conn);
        eprintln!("saved: {:?}", saved.iter().map(|s| (&s.id, &s.kind)).collect::<Vec<_>>());
        if let Some(device) = wifi_device(&conn) {
            for n in networks(&conn, &device) {
                eprintln!("{} {} {}", String::from_utf8_lossy(&n.ssid), n.strength, n.secure);
            }
        }
        assert!(!saved.is_empty());
    }

    #[test]
    fn signal_steps() {
        assert_eq!(signal(100), "\u{f0928}");
        assert_eq!(signal(59), "\u{f0922}");
        assert_eq!(signal(0), "\u{f092f}");
    }
}
