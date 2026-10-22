//! Bluetooth module: the adapter and the devices, from BlueZ over the
//! system bus. Same shape and sizes as the status modules.
//!
//!   first line   the connected device (or "Bluetooth")
//!   second line  its battery, or the adapter's state
//!   Full         the other connected devices
//!
//! Event-driven: BlueZ's PropertiesChanged, InterfacesAdded and
//! InterfacesRemoved, and its name coming and going.
//!
//! Left click, the quick part: a popover with the adapter's switch and one
//! switch per paired device (connect, disconnect). Right click, the full
//! part: the Bluetooth manager (`manager`, blueman's by default).

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};
use serde::Deserialize;

use orbit_app::i18n::{number, tr, tr_args};

use crate::{
    board::Form,
    status::{Row, hold, on_changes, open_or_focus, switch_row, system_bus},
};

const BLUEZ: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const BATTERY: &str = "org.bluez.Battery1";

/// `modules/bluetooth.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BluetoothConfig {
    /// Opened (or focused) by a right click.
    pub manager: String,
}

impl Default for BluetoothConfig {
    fn default() -> Self {
        Self {
            manager: "blueman-manager".into(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Adapter {
    path: String,
    powered: bool,
}

#[derive(Debug, Clone, Default)]
struct Device {
    path: String,
    alias: String,
    paired: bool,
    connected: bool,
    battery: Option<u8>,
}

/// What BlueZ has: the first adapter, and the devices it knows.
#[derive(Debug, Clone, Default)]
struct State {
    adapter: Option<Adapter>,
    devices: Vec<Device>,
}

struct Module {
    row: Rc<Row>,
    conn: gio::DBusConnection,
    state: RefCell<State>,
}

pub fn new(config: &BluetoothConfig, form: Form) -> gtk::Widget {
    let row = Row::new("bluetooth", form);
    let Some(conn) = system_bus() else {
        return row.root.clone();
    };
    let module = Rc::new(Module {
        row: row.clone(),
        conn: conn.clone(),
        state: RefCell::default(),
    });
    module.refresh();

    let refresh = {
        let weak = Rc::downgrade(&module);
        move || {
            if let Some(m) = weak.upgrade() {
                m.refresh();
            }
        }
    };
    let changes = on_changes(&conn, BLUEZ, None, refresh.clone());
    let objects = ["InterfacesAdded", "InterfacesRemoved"].map(|member| {
        let refresh = refresh.clone();
        conn.subscribe_to_signal(
            Some(BLUEZ),
            Some("org.freedesktop.DBus.ObjectManager"),
            Some(member),
            None,
            None,
            gio::DBusSignalFlags::NONE,
            move |_| refresh(),
        )
    });
    let (appeared, vanished) = (refresh.clone(), refresh);
    let watch = gio::bus_watch_name_on_connection(
        &conn,
        BLUEZ,
        gio::BusNameWatcherFlags::NONE,
        move |_, _, _| appeared(),
        move |_, _| vanished(),
    );
    hold(&row.root, (changes, objects, watch));

    let click = gtk::GestureClick::builder().button(0).build();
    {
        let (module, manager) = (module.clone(), config.manager.clone());
        click.connect_pressed(move |gesture, _, _, _| match gesture.current_button() {
            1 => module.quick(),
            3 => open_or_focus(&manager),
            _ => {}
        });
    }
    row.root.add_controller(click);
    row.root.set_cursor_from_name(Some("pointer"));
    row.root.clone()
}

impl Module {
    fn refresh(&self) {
        let state = read(&self.conn);
        self.show(&state);
        *self.state.borrow_mut() = state;
    }

    fn show(&self, state: &State) {
        let hint = tr("bluetooth-tooltip");
        let Some(adapter) = &state.adapter else {
            self.row.show(
                "\u{f00b2}", // 󰂲
                &[&tr("bluetooth-name"), &tr("bluetooth-none")],
                &hint,
            );
            return;
        };
        if !adapter.powered {
            self.row.show(
                "\u{f00b2}", // 󰂲
                &[&tr("bluetooth-name"), &tr("bluetooth-off")],
                &hint,
            );
            return;
        }
        let connected: Vec<&Device> = state.devices.iter().filter(|d| d.connected).collect();
        let describe = |d: &Device| match d.battery {
            Some(level) => format!("{} — {level}%", d.alias),
            None => d.alias.clone(),
        };
        let Some(first) = connected.first() else {
            self.row.show(
                "\u{f00af}", // 󰂯
                &[&tr("bluetooth-name"), &tr("bluetooth-on")],
                &hint,
            );
            return;
        };
        let second = match (first.battery, connected.len()) {
            (_, n) if n > 1 => tr_args("bluetooth-devices", &[("count", number(n as u32))]),
            (Some(level), _) => tr_args("bluetooth-battery", &[("percent", number(level))]),
            (None, _) => tr("bluetooth-connected"),
        };
        // Full: the others under the first.
        let others: Vec<String> = connected
            .iter()
            .skip(1)
            .take(2)
            .map(|d| describe(d))
            .collect();
        let mut lines = vec![first.alias.as_str(), second.as_str()];
        lines.extend(others.iter().map(String::as_str));
        let all: Vec<String> = connected.iter().map(|d| describe(d)).collect();
        self.row.show(
            "\u{f00b1}", // 󰂱
            &lines,
            &format!("{}\n{hint}", all.join("\n")),
        );
    }

    /// The quick part: the adapter's switch and the paired devices'.
    fn quick(self: &Rc<Self>) {
        let state = self.state.borrow().clone();
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(5)
            .css_classes(["bluetooth-quick"])
            .build();
        let Some(adapter) = state.adapter.clone() else {
            list.append(&gtk::Label::new(Some(&tr("bluetooth-none"))));
            self.popup(&list);
            return;
        };

        let power = switch_row(&tr("bluetooth-name"), adapter.powered, &list);
        power.add_css_class("power");
        {
            let conn = self.conn.clone();
            let path = adapter.path.clone();
            power.connect_state_set(move |_, on| {
                set_powered(&conn, &path, on);
                glib::Propagation::Proceed
            });
        }

        let paired: Vec<&Device> = state.devices.iter().filter(|d| d.paired).collect();
        if adapter.powered {
            if paired.is_empty() {
                list.append(&gtk::Label::new(Some(&tr("bluetooth-paired-none"))));
            }
            for device in paired {
                let label = match device.battery {
                    Some(level) => format!("{} — {level}%", device.alias),
                    None => device.alias.clone(),
                };
                let switch = switch_row(&label, device.connected, &list);
                let conn = self.conn.clone();
                let path = device.path.clone();
                // Set while a failed call turns the switch back, so that
                // does not call again.
                let reverting = Rc::new(Cell::new(false));
                switch.connect_state_set(move |switch, on| {
                    if !reverting.get() {
                        connect(&conn, &path, on, switch, reverting.clone());
                    }
                    glib::Propagation::Proceed
                });
            }
        }
        self.popup(&list);
    }

    fn popup(&self, child: &gtk::Box) {
        let popover = gtk::Popover::builder().child(child).build();
        orbit_app::popup::open(&popover, &self.row.root, None);
    }
}

/// Everything BlueZ knows, from its object manager.
fn read(conn: &gio::DBusConnection) -> State {
    let reply = conn.call_sync(
        Some(BLUEZ),
        "/",
        "org.freedesktop.DBus.ObjectManager",
        "GetManagedObjects",
        None,
        None,
        gio::DBusCallFlags::NO_AUTO_START,
        1000,
        gio::Cancellable::NONE,
    );
    let Ok(reply) = reply else {
        return State::default();
    };
    let objects = reply.child_value(0);
    let mut state = State::default();
    for i in 0..objects.n_children() {
        let entry = objects.child_value(i);
        let Some(path) = entry.child_value(0).str().map(str::to_owned) else {
            continue;
        };
        let interfaces = entry.child_value(1);
        // a{sa{sv}}: each interface and its properties.
        let props = |iface: &str| {
            (0..interfaces.n_children())
                .map(|k| interfaces.child_value(k))
                .find(|entry| entry.child_value(0).str() == Some(iface))
                .map(|entry| glib::VariantDict::new(Some(&entry.child_value(1))))
        };
        if let Some(adapter) = props(ADAPTER)
            && state.adapter.is_none()
        {
            state.adapter = Some(Adapter {
                path: path.clone(),
                powered: adapter
                    .lookup::<bool>("Powered")
                    .ok()
                    .flatten()
                    .unwrap_or(false),
            });
        }
        if let Some(device) = props(DEVICE) {
            let get = |name: &str| device.lookup::<bool>(name).ok().flatten().unwrap_or(false);
            state.devices.push(Device {
                path: path.clone(),
                alias: device
                    .lookup::<String>("Alias")
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
                paired: get("Paired"),
                connected: get("Connected"),
                battery: props(BATTERY).and_then(|b| b.lookup::<u8>("Percentage").ok().flatten()),
            });
        }
    }
    state.devices.sort_by_key(|d| d.alias.to_lowercase());
    state
}

fn set_powered(conn: &gio::DBusConnection, adapter: &str, on: bool) {
    let value = glib::Variant::from_variant(&on.to_variant());
    conn.call(
        Some(BLUEZ),
        adapter,
        "org.freedesktop.DBus.Properties",
        "Set",
        Some(&(ADAPTER, "Powered", value).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        5000,
        gio::Cancellable::NONE,
        |r| {
            if let Err(e) = r {
                // org.bluez.Error.Blocked: rfkill has the radio off.
                log::warn!("bluetooth: power: {e}");
            }
        },
    );
}

/// Connects or disconnects `device`. Connecting can take seconds; if it
/// fails, the switch goes back.
fn connect(
    conn: &gio::DBusConnection,
    device: &str,
    on: bool,
    switch: &gtk::Switch,
    reverting: Rc<Cell<bool>>,
) {
    let switch = switch.downgrade();
    conn.call(
        Some(BLUEZ),
        device,
        DEVICE,
        if on { "Connect" } else { "Disconnect" },
        None,
        None,
        gio::DBusCallFlags::NONE,
        30_000,
        gio::Cancellable::NONE,
        move |r| {
            if let Err(e) = r {
                log::warn!(
                    "bluetooth: {}: {e}",
                    if on { "connect" } else { "disconnect" }
                );
                if let Some(switch) = switch.upgrade() {
                    reverting.set(true);
                    switch.set_active(!on);
                    reverting.set(false);
                }
            }
        },
    );
}
