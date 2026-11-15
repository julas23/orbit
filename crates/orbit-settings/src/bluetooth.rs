//! The Bluetooth page (docs/PLANO-SETTINGS.md §3.1), from BlueZ over the
//! system bus:
//!
//! - the adapter: on or off, and seen by other devices or not;
//! - my devices (paired): connect, disconnect, remove, their battery;
//! - devices nearby: searched for while the page is shown, paired (then
//!   trusted and connected) with a click.
//!
//! Pairing asks this page's own agent (registered while the page lives,
//! not as the default: BlueZ asks the agent of whoever asked to pair), so
//! a passkey to confirm or a PIN to type shows here, whatever else runs in
//! the session. The lists follow BlueZ's signals.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::{tr, tr_args};

const BLUEZ: &str = "org.bluez";
const AGENT_PATH: &str = "/io/github/julas23/orbit/settings/agent";
const AGENT_XML: &str = r#"<node>
  <interface name="org.bluez.Agent1">
    <method name="Release"/>
    <method name="RequestPinCode"><arg type="o" direction="in"/><arg type="s" direction="out"/></method>
    <method name="DisplayPinCode"><arg type="o" direction="in"/><arg type="s" direction="in"/></method>
    <method name="RequestPasskey"><arg type="o" direction="in"/><arg type="u" direction="out"/></method>
    <method name="DisplayPasskey"><arg type="o" direction="in"/><arg type="u" direction="in"/><arg type="q" direction="in"/></method>
    <method name="RequestConfirmation"><arg type="o" direction="in"/><arg type="u" direction="in"/></method>
    <method name="RequestAuthorization"><arg type="o" direction="in"/></method>
    <method name="AuthorizeService"><arg type="o" direction="in"/><arg type="s" direction="in"/></method>
    <method name="Cancel"/>
  </interface>
</node>"#;

/// An adapter or a device, as GetManagedObjects gives it.
#[derive(Debug, Clone, Default)]
struct Device {
    path: String,
    name: String,
    icon: String,
    paired: bool,
    connected: bool,
    battery: Option<u8>,
    /// It says its name: devices that do not are shown by their address
    /// (BlueZ's alias), and are left out of "nearby".
    named: bool,
}

#[derive(Debug, Clone, Default)]
struct Adapter {
    path: String,
    alias: String,
    powered: bool,
    discoverable: bool,
}

fn bus() -> Option<gio::DBusConnection> {
    gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)
        .map_err(|e| log::warn!("system bus: {e}"))
        .ok()
}

fn lookup<T: glib::variant::FromVariant>(props: &glib::Variant, key: &str) -> Option<T> {
    glib::VariantDict::new(Some(props)).lookup::<T>(key).ok().flatten()
}

/// The adapter (the first) and the devices BlueZ knows.
fn objects(bus: &gio::DBusConnection) -> (Option<Adapter>, Vec<Device>) {
    let Ok(reply) = bus.call_sync(
        Some(BLUEZ),
        "/",
        "org.freedesktop.DBus.ObjectManager",
        "GetManagedObjects",
        None,
        None,
        gio::DBusCallFlags::NONE,
        -1,
        gio::Cancellable::NONE,
    ) else {
        return (None, Vec::new());
    };
    let mut adapter = None;
    let mut devices = Vec::new();
    for entry in reply.child_value(0).iter() {
        let path = entry.child_value(0).get::<glib::variant::ObjectPath>().map(|p| p.as_str().to_owned()).unwrap_or_default();
        let interfaces = entry.child_value(1);
        let interface = |name: &str| {
            interfaces
                .iter()
                .find(|i| i.child_value(0).get::<String>().as_deref() == Some(name))
                .map(|i| i.child_value(1))
        };
        if let Some(props) = interface("org.bluez.Adapter1") {
            adapter.get_or_insert(Adapter {
                path: path.clone(),
                alias: lookup(&props, "Alias").unwrap_or_default(),
                powered: lookup(&props, "Powered").unwrap_or(false),
                discoverable: lookup(&props, "Discoverable").unwrap_or(false),
            });
        }
        if let Some(props) = interface("org.bluez.Device1") {
            let named = lookup::<String>(&props, "Name").is_some();
            let name: Option<String> = lookup(&props, "Alias").or_else(|| lookup(&props, "Name"));
            devices.push(Device {
                path,
                named,
                name: name.unwrap_or_default(),
                icon: lookup(&props, "Icon").unwrap_or_default(),
                paired: lookup(&props, "Paired").unwrap_or(false),
                connected: lookup(&props, "Connected").unwrap_or(false),
                battery: interface("org.bluez.Battery1").and_then(|b| lookup::<u8>(&b, "Percentage")),
            });
        }
    }
    devices.sort_by(|a, b| b.connected.cmp(&a.connected).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    (adapter, devices)
}

struct Page {
    bus: gio::DBusConnection,
    adapter: RefCell<Option<Adapter>>,
    power: gtk::Switch,
    visible: gtk::Switch,
    name: gtk::Label,
    mine: gtk::Box,
    nearby: gtk::Box,
    nearby_card: gtk::Box,
    message: gtk::Label,
    updating: Cell<bool>,
    /// A refresh is due (signals come in bursts).
    pending: Cell<bool>,
    /// BlueZ's signals, heard while the page lives.
    subscription: RefCell<Option<gio::SignalSubscription>>,
}

pub fn page() -> gtk::Widget {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();
    let Some(bus) = bus() else {
        body.append(&dim(&tr("settings-bt-unavailable")));
        return body.upcast();
    };
    let page = Rc::new(Page {
        bus,
        adapter: RefCell::default(),
        power: gtk::Switch::builder().valign(gtk::Align::Center).build(),
        visible: gtk::Switch::builder().halign(gtk::Align::End).build(),
        name: dim(""),
        mine: vbox(),
        nearby: vbox(),
        nearby_card: card(&tr("settings-bt-nearby")),
        message: crate::message(),
        updating: Cell::new(false),
        pending: Cell::new(false),
        subscription: RefCell::default(),
    });

    let adapter_card = card(&tr("settings-bt"));
    adapter_card.first_child().and_downcast::<gtk::Box>().expect("head").append(&page.power);
    adapter_card.append(&page.name);
    adapter_card.append(&crate::row(&tr("settings-bt-visible"), &page.visible));
    let mine_card = card(&tr("settings-bt-mine"));
    mine_card.append(&page.mine);
    page.nearby_card.append(&page.nearby);
    body.append(&adapter_card);
    body.append(&mine_card);
    body.append(&page.nearby_card);
    body.append(&page.message);

    page.power.connect_active_notify({
        let page = page.clone();
        move |s| page.set_adapter("Powered", s.is_active())
    });
    page.visible.connect_active_notify({
        let page = page.clone();
        move |s| page.set_adapter("Discoverable", s.is_active())
    });
    page.register_agent();
    page.subscribe();
    page.refresh();

    // The search runs while the page is on screen.
    body.connect_map({
        let page = page.clone();
        move |_| page.discovery(true)
    });
    body.connect_unmap({
        let page = page.clone();
        move |_| page.discovery(false)
    });
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .build()
        .upcast()
}

fn vbox() -> gtk::Box {
    gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build()
}

fn card(title: &str) -> gtk::Box {
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(["card"])
        .build();
    let head = gtk::Box::builder().spacing(10).build();
    head.append(&gtk::Label::builder().label(title).xalign(0.0).hexpand(true).css_classes(["heading"]).build());
    card.append(&head);
    card
}

fn dim(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["dim"]).build()
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

impl Page {
    /// Calls `method` on `path`'s `interface`, then shows what failed.
    fn call(self: &Rc<Self>, path: &str, interface: &str, method: &str, args: Option<glib::Variant>) {
        let page = self.clone();
        let path = path.to_owned();
        let interface = interface.to_owned();
        let method = method.to_owned();
        glib::spawn_future_local(async move {
            let result = page
                .bus
                .call_future(Some(BLUEZ), &path, &interface, &method, args.as_ref(), None, gio::DBusCallFlags::NONE, 60_000)
                .await;
            let error = result.err().map(|e| e.to_string());
            crate::say(&page.message, error.as_deref().unwrap_or_default());
            page.refresh();
        });
    }

    fn set_adapter(self: &Rc<Self>, property: &str, value: bool) {
        if self.updating.get() {
            return;
        }
        let Some(adapter) = self.adapter.borrow().clone() else { return };
        self.call(
            &adapter.path,
            "org.freedesktop.DBus.Properties",
            "Set",
            Some(("org.bluez.Adapter1", property, value.to_variant()).to_variant()),
        );
    }

    fn discovery(self: &Rc<Self>, on: bool) {
        let Some(adapter) = self.adapter.borrow().clone() else { return };
        if !adapter.powered {
            return;
        }
        let method = if on { "StartDiscovery" } else { "StopDiscovery" };
        // Its errors (already searching, not searching) say nothing useful.
        let _ = self.bus.call_sync(
            Some(BLUEZ),
            &adapter.path,
            "org.bluez.Adapter1",
            method,
            None,
            None,
            gio::DBusCallFlags::NONE,
            -1,
            gio::Cancellable::NONE,
        );
    }

    /// A refresh a moment after BlueZ's signals (they come in bursts).
    fn subscribe(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let subscription = self.bus.subscribe_to_signal(
            Some(BLUEZ),
            None,
            None,
            None,
            None,
            gio::DBusSignalFlags::NONE,
            move |_| {
                let Some(page) = weak.upgrade() else { return };
                if page.pending.replace(true) {
                    return;
                }
                glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                    page.pending.set(false);
                    page.refresh();
                });
            },
        );
        *self.subscription.borrow_mut() = Some(subscription);
    }

    fn refresh(self: &Rc<Self>) {
        let (adapter, devices) = objects(&self.bus);
        self.updating.set(true);
        match &adapter {
            Some(a) => {
                self.power.set_sensitive(true);
                self.power.set_active(a.powered);
                self.visible.set_active(a.discoverable);
                self.visible.set_sensitive(a.powered);
                self.name.set_label(&tr_args("settings-bt-name", &[("name", a.alias.clone().into())]));
            }
            None => {
                self.power.set_sensitive(false);
                self.visible.set_sensitive(false);
                self.name.set_label(&tr("settings-bt-no-adapter"));
            }
        }
        self.updating.set(false);
        let powered = adapter.as_ref().is_some_and(|a| a.powered);
        *self.adapter.borrow_mut() = adapter;

        clear(&self.mine);
        clear(&self.nearby);
        let (mine, nearby): (Vec<Device>, Vec<Device>) = devices.into_iter().partition(|d| d.paired);
        if mine.is_empty() {
            self.mine.append(&dim(&tr("settings-bt-none")));
        }
        for device in mine {
            self.mine.append(&self.row(device));
        }
        self.nearby_card.set_visible(powered);
        let named: Vec<Device> = nearby.into_iter().filter(|d| d.named && !d.name.is_empty()).collect();
        if named.is_empty() {
            self.nearby.append(&dim(&tr("settings-bt-searching")));
        }
        for device in named {
            self.nearby.append(&self.row(device));
        }
    }

    fn row(self: &Rc<Self>, device: Device) -> gtk::Box {
        let line = gtk::Box::builder().spacing(10).build();
        let icon = if device.icon.is_empty() { "bluetooth".to_owned() } else { device.icon.clone() };
        line.append(&gtk::Image::from_icon_name(&format!("{icon}-symbolic")));
        let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
        text.append(&gtk::Label::builder().label(&device.name).xalign(0.0).css_classes(["name"]).build());
        let mut detail = tr(if device.connected { "settings-net-connected" } else { "settings-net-disconnected" });
        if let Some(b) = device.battery {
            detail.push_str(&format!(" · {}", tr_args("settings-bt-battery", &[("percent", i64::from(b).into())])));
        }
        if device.paired {
            text.append(&dim(&detail));
        }
        line.append(&text);
        let path = device.path.clone();
        if device.paired {
            let remove = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text(tr("settings-bt-remove"))
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            remove.connect_clicked({
                let page = self.clone();
                let path = path.clone();
                move |_| {
                    let Some(adapter) = page.adapter.borrow().clone() else { return };
                    let device = glib::variant::ObjectPath::try_from(path.clone()).map(|p| (p,).to_variant()).ok();
                    page.call(&adapter.path, "org.bluez.Adapter1", "RemoveDevice", device);
                }
            });
            line.append(&remove);
            let connect = gtk::Button::with_label(&tr(if device.connected {
                "settings-net-disconnect"
            } else {
                "settings-net-connect"
            }));
            connect.set_valign(gtk::Align::Center);
            let method = if device.connected { "Disconnect" } else { "Connect" };
            connect.connect_clicked({
                let page = self.clone();
                move |b| {
                    b.set_sensitive(false);
                    crate::say(&page.message, &tr("settings-svc-working"));
                    page.call(&path, "org.bluez.Device1", method, None);
                }
            });
            line.append(&connect);
        } else {
            let pair = gtk::Button::with_label(&tr("settings-bt-pair"));
            pair.set_valign(gtk::Align::Center);
            pair.connect_clicked({
                let page = self.clone();
                move |b| {
                    b.set_sensitive(false);
                    page.pair(path.clone());
                }
            });
            line.append(&pair);
        }
        line
    }

    /// Pairs (through this page's agent), then trusts and connects.
    fn pair(self: &Rc<Self>, path: String) {
        crate::say(&self.message, &tr("settings-bt-pairing"));
        let page = self.clone();
        glib::spawn_future_local(async move {
            let call = |interface: &'static str, method: &'static str, args: Option<glib::Variant>| {
                page.bus.call_future(Some(BLUEZ), &path, interface, method, args.as_ref(), None, gio::DBusCallFlags::NONE, 60_000)
            };
            let result = async {
                call("org.bluez.Device1", "Pair", None).await?;
                call(
                    "org.freedesktop.DBus.Properties",
                    "Set",
                    Some(("org.bluez.Device1", "Trusted", true.to_variant()).to_variant()),
                )
                .await?;
                call("org.bluez.Device1", "Connect", None).await?;
                Ok::<_, glib::Error>(())
            }
            .await;
            let error = result.err().map(|e| e.to_string());
            crate::say(&page.message, error.as_deref().unwrap_or_default());
            page.refresh();
        });
    }

    /// This page's agent: the passkeys and PINs of a pairing it started.
    fn register_agent(self: &Rc<Self>) {
        let Ok(info) = gio::DBusNodeInfo::for_xml(AGENT_XML) else { return };
        let Some(interface) = info.lookup_interface("org.bluez.Agent1") else { return };
        let page = Rc::downgrade(self);
        let registered = self
            .bus
            .register_object(AGENT_PATH, &interface)
            .method_call(move |_, _, _, _, method, params, invocation| {
                let Some(page) = page.upgrade() else {
                    invocation.return_dbus_error("org.bluez.Error.Rejected", "gone");
                    return;
                };
                page.agent(method, &params, invocation);
            })
            .build();
        if let Err(e) = registered {
            log::warn!("bluetooth agent: {e}");
            return;
        }
        let args = (glib::variant::ObjectPath::try_from(AGENT_PATH.to_owned()).expect("a path"), "KeyboardDisplay").to_variant();
        if let Err(e) = self.bus.call_sync(
            Some(BLUEZ),
            "/org/bluez",
            "org.bluez.AgentManager1",
            "RegisterAgent",
            Some(&args),
            None,
            gio::DBusCallFlags::NONE,
            -1,
            gio::Cancellable::NONE,
        ) {
            log::warn!("bluetooth agent: {e}");
        }
    }

    /// Answers BlueZ's questions about a pairing.
    fn agent(self: &Rc<Self>, method: &str, params: &glib::Variant, invocation: gio::DBusMethodInvocation) {
        let number = |i: usize| params.try_child_value(i).and_then(|v| v.get::<u32>()).unwrap_or(0);
        match method {
            "RequestConfirmation" => {
                let passkey = format!("{:06}", number(1));
                self.ask(&tr_args("settings-bt-confirm", &[("code", passkey.into())]), None, move |answer| {
                    match answer {
                        Some(_) => invocation.return_value(None),
                        None => invocation.return_dbus_error("org.bluez.Error.Rejected", "rejected"),
                    }
                });
            }
            "RequestAuthorization" => self.ask(&tr("settings-bt-authorize"), None, move |answer| match answer {
                Some(_) => invocation.return_value(None),
                None => invocation.return_dbus_error("org.bluez.Error.Rejected", "rejected"),
            }),
            "RequestPinCode" => self.ask(&tr("settings-bt-pin"), Some(false), move |answer| match answer {
                Some(pin) if !pin.is_empty() => invocation.return_value(Some(&(pin,).to_variant())),
                _ => invocation.return_dbus_error("org.bluez.Error.Canceled", "canceled"),
            }),
            "RequestPasskey" => self.ask(&tr("settings-bt-pin"), Some(true), move |answer| {
                match answer.and_then(|p| p.parse::<u32>().ok()) {
                    Some(key) => invocation.return_value(Some(&(key,).to_variant())),
                    None => invocation.return_dbus_error("org.bluez.Error.Canceled", "canceled"),
                }
            }),
            "DisplayPasskey" => {
                let code = format!("{:06}", number(1));
                crate::say(&self.message, &tr_args("settings-bt-type-code", &[("code", code.into())]));
                invocation.return_value(None);
            }
            "DisplayPinCode" => {
                let code = params.try_child_value(1).and_then(|v| v.get::<String>()).unwrap_or_default();
                crate::say(&self.message, &tr_args("settings-bt-type-code", &[("code", code.into())]));
                invocation.return_value(None);
            }
            // A paired device's own services: allowed.
            "AuthorizeService" | "Release" | "Cancel" => invocation.return_value(None),
            _ => invocation.return_dbus_error("org.bluez.Error.Rejected", "unknown"),
        }
    }

    /// A yes/no question, or with `entry` (`Some(digits_only)`) one to type
    /// an answer into; `done` gets the answer, or `None` for no.
    fn ask(self: &Rc<Self>, question: &str, entry: Option<bool>, done: impl FnOnce(Option<String>) + 'static) {
        let window = gtk::Window::builder()
            .title(tr("settings-bt"))
            .modal(true)
            .default_width(380)
            .css_classes(["orbit-settings"])
            .build();
        if let Some(parent) = self.mine.root().and_downcast::<gtk::Window>() {
            window.set_transient_for(Some(&parent));
        }
        let body = vbox();
        body.add_css_class("page");
        body.append(&gtk::Label::builder().label(question).xalign(0.0).wrap(true).build());
        let field = entry.map(|digits| {
            let e = gtk::Entry::builder().build();
            if digits {
                e.set_input_purpose(gtk::InputPurpose::Digits);
            }
            body.append(&e);
            e
        });
        let no = gtk::Button::with_label(&tr("settings-svc-cancel"));
        let yes = gtk::Button::with_label(&tr("settings-bt-pair"));
        yes.add_css_class("suggested-action");
        let buttons = gtk::Box::builder().spacing(10).halign(gtk::Align::End).build();
        buttons.append(&no);
        buttons.append(&yes);
        body.append(&buttons);
        window.set_child(Some(&body));
        let done = Rc::new(RefCell::new(Some(done)));
        let answer = {
            let done = done.clone();
            move |value: Option<String>| {
                if let Some(f) = done.borrow_mut().take() {
                    f(value);
                }
            }
        };
        yes.connect_clicked({
            let window = window.clone();
            let answer = answer.clone();
            move |_| {
                answer(Some(field.as_ref().map(|f| f.text().trim().to_owned()).unwrap_or_default()));
                window.close();
            }
        });
        no.connect_clicked({
            let window = window.clone();
            move |_| window.close()
        });
        // Closing it any way but "Pair" is a no.
        window.connect_close_request(move |_| {
            answer(None);
            glib::Propagation::Proceed
        });
        window.present();
    }
}
