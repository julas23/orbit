//! The Network page's wired and Wi-Fi parts (docs/PLANO-SETTINGS.md §3.1),
//! through NetworkManager's `nmcli`, in a thread.
//!
//! Wi-Fi: the radio on or off, the networks in reach (strongest first,
//! hidden ones left to "Hidden network…"), connecting (a known network
//! again, or a new one with its password), disconnecting, forgetting. A new
//! network is made through NetworkManager's D-Bus API, as this user's
//! connection: no administrator's password, and the key travels inside the
//! call, never on a command line where `ps` would show it.
//! Wired: each cable's state and address, connected or disconnected.
//! Everything else (fixed addresses, DNS, 802.1X, proxies) is in
//! "Advanced…", NetworkManager's own editor.

use std::{collections::HashMap, process::Command, rc::Rc};

use gtk::{glib, prelude::*};
use orbit_app::i18n::{tr, tr_args};

use crate::network::{fields, nmcli};

/// A network in reach.
#[derive(Debug, Clone, PartialEq)]
struct Network {
    ssid: String,
    signal: u8,
    secured: bool,
    in_use: bool,
}

/// A wired or Wi-Fi device.
#[derive(Debug, Clone, PartialEq)]
struct Device {
    name: String,
    kind: String,
    state: String,
    connection: String,
    address: String,
}

#[derive(Debug, Clone, Default)]
struct Snapshot {
    radio: bool,
    devices: Vec<Device>,
    networks: Vec<Network>,
    /// The Wi-Fi connections NetworkManager keeps (known networks).
    known: Vec<String>,
}

/// The networks of `nmcli -t -f IN-USE,SSID,SIGNAL,SECURITY device wifi
/// list`: one per name, the strongest; hidden ones (no name) left out.
fn parse_networks(text: &str) -> Vec<Network> {
    let mut found: Vec<Network> = Vec::new();
    for line in text.lines() {
        let f = fields(line);
        if f.len() < 4 || f[1].is_empty() {
            continue;
        }
        let network = Network {
            ssid: f[1].clone(),
            signal: f[2].parse().unwrap_or(0),
            secured: !f[3].is_empty() && f[3] != "--",
            in_use: f[0] == "*",
        };
        match found.iter_mut().find(|n| n.ssid == network.ssid) {
            Some(n) => {
                n.in_use |= network.in_use;
                n.signal = n.signal.max(network.signal);
            }
            None => found.push(network),
        }
    }
    found.sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)));
    found
}

fn snapshot(rescan: bool) -> Result<Snapshot, String> {
    let radio = nmcli(&["radio", "wifi"])?.trim() == "enabled";
    let mut devices = Vec::new();
    for line in nmcli(&["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device", "status"])?.lines() {
        let f = fields(line);
        if f.len() < 4 || (f[1] != "wifi" && f[1] != "ethernet") {
            continue;
        }
        let address = nmcli(&["-t", "-g", "IP4.ADDRESS", "device", "show", &f[0]])
            .unwrap_or_default()
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned();
        devices.push(Device {
            name: f[0].clone(),
            kind: f[1].clone(),
            state: f[2].clone(),
            connection: f[3].clone(),
            address,
        });
    }
    // A scan takes seconds: the list NetworkManager has is shown at once,
    // and a new scan is asked for after (`rescan`).
    let networks = if radio {
        let rescan = if rescan { "yes" } else { "no" };
        parse_networks(&nmcli(&["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list", "--rescan", rescan])?)
    } else {
        Vec::new()
    };
    let known = nmcli(&["-t", "-f", "NAME,TYPE", "connection", "show"])?
        .lines()
        .map(fields)
        .filter(|f| f.len() >= 2 && f[1] == "802-11-wireless")
        .map(|f| f[0].clone())
        .collect();
    Ok(Snapshot {
        radio,
        devices,
        networks,
        known,
    })
}

/// The icon of a signal's strength.
fn signal_icon(signal: u8) -> &'static str {
    match signal {
        75.. => "network-wireless-signal-excellent-symbolic",
        50..=74 => "network-wireless-signal-good-symbolic",
        25..=49 => "network-wireless-signal-ok-symbolic",
        _ => "network-wireless-signal-weak-symbolic",
    }
}

pub struct Wifi {
    radio: gtk::Switch,
    /// The Wi-Fi device new networks are joined on.
    device: std::cell::RefCell<Option<String>>,
    wired: gtk::Box,
    networks: gtk::Box,
    message: gtk::Label,
    /// The radio's switch is being set to what was read, not by the user.
    updating: std::cell::Cell<bool>,
}

/// The wired and Wi-Fi cards, for the Network page.
pub fn cards() -> (gtk::Box, gtk::Box) {
    let wifi = Rc::new(Wifi {
        radio: gtk::Switch::builder().valign(gtk::Align::Center).build(),
        device: std::cell::RefCell::default(),
        wired: gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build(),
        networks: gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).build(),
        message: crate::message(),
        updating: std::cell::Cell::new(false),
    });

    let wired_card = card(&tr("settings-net-wired"));
    wired_card.append(&wifi.wired);

    let wifi_card = card(&tr("settings-net-wifi"));
    let head = wifi_card.first_child().and_downcast::<gtk::Box>().expect("the card's head");
    head.append(&wifi.radio);
    wifi_card.append(&wifi.networks);
    let buttons = gtk::Box::builder().spacing(10).halign(gtk::Align::End).build();
    let advanced = gtk::Button::with_label(&tr("settings-net-advanced"));
    let hidden = gtk::Button::with_label(&tr("settings-net-hidden"));
    let rescan = gtk::Button::with_label(&tr("settings-net-rescan"));
    buttons.append(&advanced);
    buttons.append(&hidden);
    buttons.append(&rescan);
    wifi_card.append(&buttons);
    wifi_card.append(&wifi.message);

    wifi.radio.connect_active_notify({
        let wifi = wifi.clone();
        move |switch| {
            if wifi.updating.get() {
                return;
            }
            let on = if switch.is_active() { "on" } else { "off" };
            wifi.run(move || nmcli(&["radio", "wifi", on]).map(|_| ()));
        }
    });
    rescan.connect_clicked({
        let wifi = wifi.clone();
        move |_| wifi.refresh(true)
    });
    hidden.connect_clicked({
        let wifi = wifi.clone();
        move |button| wifi.password_dialog(button, None)
    });
    advanced.connect_clicked({
        let wifi = wifi.clone();
        move |_| {
            if let Err(e) = Command::new("nm-connection-editor").spawn() {
                crate::say(&wifi.message, &format!("nm-connection-editor: {e}"));
            }
        }
    });
    wifi.networks.append(&dim(&tr("settings-net-searching")));
    wifi.refresh(false);
    wifi.refresh(true);
    (wired_card, wifi_card)
}

/// A card with its title on a line of its own (where a switch may go).
fn card(title: &str) -> gtk::Box {
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(15)
        .css_classes(["card"])
        .build();
    let head = gtk::Box::builder().spacing(10).build();
    head.append(
        &gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["heading"])
            .build(),
    );
    card.append(&head);
    card
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

impl Wifi {
    fn run(self: &Rc<Self>, work: impl FnOnce() -> Result<(), String> + Send + 'static) {
        crate::say(&self.message, &tr("settings-svc-working"));
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(work());
        });
        let wifi = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(result) = rx.recv().await {
                crate::say(&wifi.message, &result.err().unwrap_or_default());
                wifi.refresh(false);
            }
        });
    }

    fn refresh(self: &Rc<Self>, rescan: bool) {
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(snapshot(rescan));
        });
        let wifi = self.clone();
        glib::spawn_future_local(async move {
            let Ok(result) = rx.recv().await else { return };
            match result {
                Ok(snap) => wifi.show(snap),
                Err(e) => crate::say(&wifi.message, &e),
            }
        });
    }

    fn show(self: &Rc<Self>, snap: Snapshot) {
        self.updating.set(true);
        self.radio.set_active(snap.radio);
        self.updating.set(false);

        *self.device.borrow_mut() = snap.devices.iter().find(|d| d.kind == "wifi").map(|d| d.name.clone());
        clear(&self.wired);
        let cables: Vec<&Device> = snap.devices.iter().filter(|d| d.kind == "ethernet").collect();
        if cables.is_empty() {
            self.wired.append(&dim(&tr("settings-net-no-cable")));
        }
        for device in cables {
            self.wired.append(&self.wired_row(device));
        }

        clear(&self.networks);
        if !snap.radio {
            self.networks.append(&dim(&tr("settings-net-wifi-off")));
            return;
        }
        if snap.networks.is_empty() {
            self.networks.append(&dim(&tr("settings-net-no-networks")));
        }
        for network in snap.networks {
            let known = snap.known.contains(&network.ssid);
            self.networks.append(&self.network_row(network, known));
        }
    }

    fn wired_row(self: &Rc<Self>, device: &Device) -> gtk::Box {
        let connected = device.state == "connected";
        let state = match device.state.as_str() {
            "connected" => tr("settings-net-connected"),
            "unavailable" => tr("settings-net-unplugged"),
            _ => tr("settings-net-disconnected"),
        };
        let detail = if device.address.is_empty() { state } else { format!("{state} · {}", device.address) };
        let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
        text.append(&gtk::Label::builder().label(&device.name).xalign(0.0).css_classes(["name"]).build());
        text.append(&dim(&detail));
        let switch = gtk::Switch::builder()
            .active(connected)
            .sensitive(device.state != "unavailable")
            .valign(gtk::Align::Center)
            .build();
        let line = gtk::Box::builder().spacing(10).build();
        line.append(&text);
        line.append(&switch);
        let name = device.name.clone();
        switch.connect_active_notify({
            let wifi = self.clone();
            move |switch| {
                let verb = if switch.is_active() { "connect" } else { "disconnect" };
                let name = name.clone();
                switch.set_sensitive(false);
                wifi.run(move || nmcli(&["device", verb, &name]).map(|_| ()));
            }
        });
        line
    }

    fn network_row(self: &Rc<Self>, network: Network, known: bool) -> gtk::Box {
        let line = gtk::Box::builder().spacing(10).css_classes(["network"]).build();
        line.append(&gtk::Image::from_icon_name(signal_icon(network.signal)));
        let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
        text.append(&gtk::Label::builder().label(&network.ssid).xalign(0.0).css_classes(["name"]).build());
        let mut detail = format!("{}%", network.signal);
        if network.secured {
            detail.push_str(&format!(" · {}", tr("settings-net-secured")));
        }
        if network.in_use {
            detail = format!("{} · {detail}", tr("settings-net-connected"));
        }
        text.append(&dim(&detail));
        line.append(&text);
        if known {
            let forget = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text(tr("settings-net-forget"))
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            let ssid = network.ssid.clone();
            forget.connect_clicked({
                let wifi = self.clone();
                move |_| {
                    let ssid = ssid.clone();
                    wifi.run(move || nmcli(&["connection", "delete", "id", &ssid]).map(|_| ()));
                }
            });
            line.append(&forget);
        }
        let action = gtk::Button::with_label(&tr(if network.in_use {
            "settings-net-disconnect"
        } else {
            "settings-net-connect"
        }));
        action.set_valign(gtk::Align::Center);
        let ssid = network.ssid.clone();
        action.connect_clicked({
            let wifi = self.clone();
            move |button| {
                let ssid = ssid.clone();
                if network.in_use {
                    wifi.run(move || nmcli(&["connection", "down", "id", &ssid]).map(|_| ()));
                } else if known {
                    wifi.run(move || nmcli(&["connection", "up", "id", &ssid]).map(|_| ()));
                } else if network.secured {
                    wifi.password_dialog(button, Some(ssid));
                } else {
                    wifi.join(ssid, None, false);
                }
            }
        });
        line.append(&action);
        line
    }

    /// Asks for a network's password (and, for a hidden one, its name).
    fn password_dialog(self: &Rc<Self>, button: &gtk::Button, ssid: Option<String>) {
        let window = gtk::Window::builder()
            .title(ssid.clone().unwrap_or_else(|| tr("settings-net-hidden")))
            .modal(true)
            .default_width(420)
            .css_classes(["orbit-settings"])
            .build();
        if let Some(parent) = button.root().and_downcast::<gtk::Window>() {
            window.set_transient_for(Some(&parent));
        }
        let name = gtk::Entry::builder().text(ssid.clone().unwrap_or_default()).build();
        let password = gtk::PasswordEntry::builder().show_peek_icon(true).hexpand(true).build();
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["page"])
            .build();
        if ssid.is_none() {
            body.append(&crate::row(&tr("settings-net-network-name"), &name));
        } else {
            body.append(&gtk::Label::builder().label(tr_args("settings-net-password-for", &[("name", ssid.clone().unwrap_or_default().into())])).xalign(0.0).wrap(true).build());
        }
        body.append(&crate::row(&tr("settings-net-password"), &password));
        let cancel = gtk::Button::with_label(&tr("settings-svc-cancel"));
        let join = gtk::Button::with_label(&tr("settings-net-connect"));
        join.add_css_class("suggested-action");
        let buttons = gtk::Box::builder().spacing(10).halign(gtk::Align::End).build();
        buttons.append(&cancel);
        buttons.append(&join);
        body.append(&buttons);
        window.set_child(Some(&body));
        cancel.connect_clicked({
            let window = window.clone();
            move |_| window.close()
        });
        let go = {
            let wifi = self.clone();
            let window = window.clone();
            let password = password.clone();
            let hidden = ssid.is_none();
            move || {
                let ssid = name.text().trim().to_owned();
                if ssid.is_empty() {
                    return;
                }
                let key = password.text().to_string();
                window.close();
                wifi.join(ssid, (!key.is_empty()).then_some(key), hidden);
            }
        };
        join.connect_clicked({
            let go = go.clone();
            move |_| go()
        });
        password.connect_activate(move |_| go());
        window.present();
    }
}

/// The settings of a new Wi-Fi connection, this user's alone, as
/// AddAndActivateConnection takes them (`a{sa{sv}}`).
fn wifi_settings(ssid: &str, password: Option<&str>, hidden: bool) -> glib::Variant {
    let mut connection: HashMap<String, glib::Variant> = HashMap::new();
    connection.insert("id".into(), ssid.to_variant());
    connection.insert("type".into(), "802-11-wireless".to_variant());
    connection.insert("permissions".into(), vec![format!("user:{}", glib::user_name().to_string_lossy())].to_variant());
    let mut wireless: HashMap<String, glib::Variant> = HashMap::new();
    wireless.insert("ssid".into(), ssid.as_bytes().to_vec().to_variant());
    if hidden {
        wireless.insert("hidden".into(), true.to_variant());
    }
    let mut all: HashMap<String, HashMap<String, glib::Variant>> = HashMap::new();
    all.insert("connection".into(), connection);
    all.insert("802-11-wireless".into(), wireless);
    if let Some(key) = password {
        let mut security: HashMap<String, glib::Variant> = HashMap::new();
        security.insert("key-mgmt".into(), "wpa-psk".to_variant());
        security.insert("psk".into(), key.to_variant());
        all.insert("802-11-wireless-security".into(), security);
    }
    all.to_variant()
}

impl Wifi {
    /// Joins `ssid` on the Wi-Fi device, through NetworkManager's D-Bus.
    fn join(self: &Rc<Self>, ssid: String, password: Option<String>, hidden: bool) {
        let Some(device) = self.device.borrow().clone() else {
            crate::say(&self.message, &tr("settings-net-no-wifi-device"));
            return;
        };
        crate::say(&self.message, &tr("settings-svc-working"));
        let wifi = self.clone();
        glib::spawn_future_local(async move {
            let result = async {
                let nm = crate::dbus::system("org.freedesktop.NetworkManager").ok_or("NetworkManager")?;
                let path = crate::dbus::call(&nm, "GetDeviceByIpIface", (device.as_str(),).to_variant()).await?;
                let path = path.child_value(0);
                let any = glib::variant::ObjectPath::try_from("/".to_owned()).map_err(|e| e.to_string())?;
                let args = glib::Variant::tuple_from_iter([
                    wifi_settings(&ssid, password.as_deref(), hidden),
                    path,
                    any.to_variant(),
                ]);
                crate::dbus::call(&nm, "AddAndActivateConnection", args).await?;
                Ok::<_, String>(())
            }
            .await;
            crate::say(&wifi.message, &result.err().unwrap_or_default());
            // The connection comes up a moment after the call returns.
            glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || wifi.refresh(false));
        });
    }
}

fn dim(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["dim"]).build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_for_a_new_network() {
        let v = wifi_settings("Casa", Some("segredo"), true);
        assert_eq!(v.type_().as_str(), "a{sa{sv}}");
        let text = v.print(false).to_string();
        assert!(text.contains("802-11-wireless-security"));
        assert!(text.contains("'hidden': <true>"));
        let open = wifi_settings("Cafe", None, false).print(false).to_string();
        assert!(!open.contains("security"));
    }

    #[test]
    fn networks_merged_and_sorted() {
        let text = " :Casa:60:WPA2\n*:Julas:100:WPA2\n ::92:WPA2\n :Julas:90:WPA2\n :Cafe:40:--\n";
        let n = parse_networks(text);
        assert_eq!(n.len(), 3);
        assert_eq!(n[0].ssid, "Julas");
        assert!(n[0].in_use);
        assert_eq!(n[0].signal, 100);
        assert!(!n[2].secured);
        assert_eq!(n[2].ssid, "Cafe");
    }
}
