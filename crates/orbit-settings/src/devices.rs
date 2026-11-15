//! The Devices page (docs/PLANO-SETTINGS.md §3.1): Thunderbolt devices
//! authorised (bolt), the graphics cards of a machine with two
//! (switcheroo-control), the screen turning with the machine (iio-sensor-
//! proxy's accelerometer, followed by `orbit-settings agent`, `devices.yaml`
//! `auto_rotate`), and the firmware's updates (fwupd, through `fwupdmgr`,
//! never restarting by itself). What a card needs and is not installed is
//! offered for installation. Colour profiles wait for the core to apply
//! one: none of them would show on a monitor yet.

use std::{path::Path, rc::Rc};

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::{tr, tr_args};
use orbit_common::ConfigDir;
use serde_json::Value as Json;
use serde_yaml_ng::Value;

use crate::{agent, dbus};

const BOLTD: &str = "/usr/libexec/boltd";
const SWITCHEROO: &str = "/usr/libexec/switcheroo-control";
const SENSORS: &str = "/usr/libexec/iio-sensor-proxy";
const FWUPDMGR: &str = "/usr/bin/fwupdmgr";

/// `devices.yaml`'s `auto_rotate`.
pub fn auto_rotate(dir: &ConfigDir) -> bool {
    std::fs::read_to_string(dir.path().join("devices.yaml"))
        .ok()
        .and_then(|t| serde_yaml_ng::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("auto_rotate").and_then(Value::as_bool))
        .unwrap_or(false)
}

/// The core's transform for iio-sensor-proxy's orientation (as Mutter
/// turns it).
pub fn transform_for(orientation: &str) -> Option<&'static str> {
    Some(match orientation {
        "normal" => "normal",
        "bottom-up" => "180",
        "left-up" => "90",
        "right-up" => "270",
        _ => return None,
    })
}

/// The machine's own screen: a laptop's or a tablet's panel.
pub fn internal(name: &str) -> bool {
    ["eDP", "LVDS", "DSI"].iter().any(|p| name.starts_with(p))
}

fn dim(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["dim"]).build()
}

fn clear(b: &gtk::Box) {
    while let Some(child) = b.first_child() {
        b.remove(&child);
    }
}

/// What makes a card's content again.
type Refill = Rc<dyn Fn()>;

/// A card whose content is made by `fill`, again after an installation.
fn card(title: &str, fill: impl Fn(&gtk::Box, &Rc<dyn Fn()>) + 'static) -> gtk::Box {
    let card = crate::power_card(&tr(title));
    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
    card.append(&content);
    // `again` refills the card: the card's own run, kept for itself (the
    // page lives as long as the window).
    let again: Rc<std::cell::RefCell<Option<Refill>>> = Rc::default();
    let run: Rc<dyn Fn()> = Rc::new({
        let (content, again) = (content.clone(), again.clone());
        move || {
            clear(&content);
            if let Some(a) = again.borrow().clone() {
                fill(&content, &a);
            }
        }
    });
    *again.borrow_mut() = Some(run.clone());
    run();
    card
}

/// Not installed: said, with the button that installs it.
fn missing(content: &gtk::Box, text: &str, package: &'static str, installed: fn() -> bool, message: &gtk::Label, again: &Rc<dyn Fn()>) {
    content.append(&dim(&tr(text)));
    let again = again.clone();
    content.append(&crate::helper::install_button(package, installed, message, move || again()));
}

fn thunderbolt(content: &gtk::Box, again: &Rc<dyn Fn()>, message: &gtk::Label) {
    if !Path::new(BOLTD).exists() {
        missing(content, "settings-dev-no-bolt", "bolt", || Path::new(BOLTD).exists(), message, again);
        return;
    }
    let Some(manager) = dbus::system_at("org.freedesktop.bolt", "/org/freedesktop/bolt", "org.freedesktop.bolt1.Manager") else {
        content.append(&dim(&tr("settings-dev-no-service")));
        return;
    };
    let level = dbus::string(&manager, "SecurityLevel");
    if !level.is_empty() {
        content.append(&dim(&tr_args("settings-dev-tb-level", &[("level", level.into())])));
    }
    let paths: Vec<String> = manager
        .call_sync("ListDevices", None, gio::DBusCallFlags::NONE, 5000, gio::Cancellable::NONE)
        .ok()
        .and_then(|v| v.child_value(0).get::<Vec<glib::variant::ObjectPath>>())
        .map(|v| v.into_iter().map(|p| p.as_str().to_owned()).collect())
        .unwrap_or_default();
    if paths.is_empty() {
        content.append(&dim(&tr("settings-dev-tb-none")));
    }
    for path in paths {
        let Some(device) = dbus::system_at("org.freedesktop.bolt", &path, "org.freedesktop.bolt1.Device") else { continue };
        let status = dbus::string(&device, "Status");
        let name = format!("{} {}", dbus::string(&device, "Vendor"), dbus::string(&device, "Name"));
        let line = gtk::Box::builder().spacing(10).css_classes(["row"]).build();
        line.append(&gtk::Label::builder().label(name.trim()).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build());
        line.append(&gtk::Label::builder().label(&status).css_classes(["dim"]).build());
        if status == "connected" || status == "auth-error" {
            let allow = gtk::Button::with_label(&tr("settings-dev-tb-allow"));
            let (manager, uid, message, again) = (manager.clone(), dbus::string(&device, "Uid"), message.clone(), again.clone());
            allow.connect_clicked(move |_| {
                let (manager, uid, message, again) = (manager.clone(), uid.clone(), message.clone(), again.clone());
                glib::spawn_future_local(async move {
                    let result = dbus::call(&manager, "EnrollDevice", (uid.as_str(), "default", "").to_variant()).await;
                    crate::say(&message, &result.err().unwrap_or_default());
                    again();
                });
            });
            line.append(&allow);
        }
        content.append(&line);
    }
}

fn graphics(content: &gtk::Box, again: &Rc<dyn Fn()>, message: &gtk::Label) {
    if !Path::new(SWITCHEROO).exists() {
        missing(content, "settings-dev-no-switcheroo", "switcheroo-control", || Path::new(SWITCHEROO).exists(), message, again);
        return;
    }
    let Some(proxy) = dbus::system_at("net.hadess.SwitcherooControl", "/net/hadess/SwitcherooControl", "net.hadess.SwitcherooControl") else {
        content.append(&dim(&tr("settings-dev-no-service")));
        return;
    };
    let gpus = proxy.cached_property("GPUs");
    let count = gpus.as_ref().map_or(0, |g| g.n_children());
    for i in 0..count {
        let Some(gpu) = gpus.as_ref().map(|g| glib::VariantDict::new(Some(&g.child_value(i)))) else { continue };
        let name = gpu.lookup::<String>("Name").ok().flatten().unwrap_or_default();
        let default = gpu.lookup::<bool>("Default").ok().flatten().unwrap_or(false);
        let line = gtk::Box::builder().spacing(10).css_classes(["row"]).build();
        line.append(&gtk::Label::builder().label(name).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build());
        if default {
            line.append(&gtk::Label::builder().label(tr("settings-dev-gpu-default")).css_classes(["dim"]).build());
        }
        content.append(&line);
    }
    content.append(&dim(&tr(if dbus::boolean(&proxy, "HasDualGpu") { "settings-dev-gpu-dual" } else { "settings-dev-gpu-one" })));
}

fn rotation(dir: &ConfigDir, content: &gtk::Box, again: &Rc<dyn Fn()>, message: &gtk::Label) {
    if !Path::new(SENSORS).exists() {
        missing(content, "settings-dev-no-sensors", "iio-sensor-proxy", || Path::new(SENSORS).exists(), message, again);
        return;
    }
    let has = dbus::system_at("net.hadess.SensorProxy", "/net/hadess/SensorProxy", "net.hadess.SensorProxy")
        .is_some_and(|p| dbus::boolean(&p, "HasAccelerometer"));
    if !has {
        content.append(&dim(&tr("settings-dev-rotate-none")));
        return;
    }
    let on = gtk::Switch::builder().active(auto_rotate(dir)).halign(gtk::Align::End).valign(gtk::Align::Center).build();
    let (dir, message) = (dir.clone(), message.clone());
    on.connect_active_notify(move |s| {
        let result = dir
            .save_app_keys("devices", &["auto_rotate"], &format!("auto_rotate: {}\n", s.is_active()))
            .and_then(|()| agent::ensure(&dir));
        crate::say(&message, &result.err().map(|e| format!("{e:#}")).unwrap_or_default());
    });
    content.append(&crate::row(&tr("settings-dev-rotate"), &on));
}

/// `fwupdmgr ARGS --json`, read; an empty object when it says nothing.
fn fwupd_json(args: &[&str]) -> Json {
    std::process::Command::new(FWUPDMGR)
        .args(args)
        .arg("--json")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
        .and_then(|o| serde_json::from_slice(&o.stdout).ok())
        .unwrap_or(Json::Null)
}

/// The devices fwupd can update: (name, version, the update's version).
fn firmware_list() -> Vec<(String, String, Option<String>)> {
    let devices = fwupd_json(&["get-devices"]);
    let updates = fwupd_json(&["get-updates"]);
    let newer = |id: &str| -> Option<String> {
        updates["Devices"].as_array()?.iter().find(|d| d["DeviceId"].as_str() == Some(id)).and_then(|d| {
            d["Releases"].as_array()?.first()?["Version"].as_str().map(str::to_owned)
        })
    };
    devices["Devices"]
        .as_array()
        .map(|all| {
            all.iter()
                .filter(|d| d["Flags"].as_array().is_some_and(|f| f.iter().any(|x| x == "updatable")))
                .map(|d| {
                    let id = d["DeviceId"].as_str().unwrap_or("");
                    (
                        d["Name"].as_str().unwrap_or("?").to_owned(),
                        d["Version"].as_str().unwrap_or("").to_owned(),
                        newer(id),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn firmware(content: &gtk::Box, again: &Rc<dyn Fn()>, message: &gtk::Label) {
    if !Path::new(FWUPDMGR).exists() {
        missing(content, "settings-dev-no-fwupd", "fwupd", || Path::new(FWUPDMGR).exists(), message, again);
        return;
    }
    content.append(&dim(&tr("settings-dev-fw-tip")));
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).build();
    content.append(&list);
    let look = gtk::Button::with_label(&tr("settings-dev-fw-look"));
    let update = gtk::Button::builder().label(tr("settings-dev-fw-update")).visible(false).css_classes(["suggested-action"]).build();
    let buttons = gtk::Box::builder().spacing(10).halign(gtk::Align::End).build();
    buttons.append(&look);
    buttons.append(&update);
    content.append(&buttons);
    let message = message.clone();
    look.connect_clicked({
        let (list, update, message) = (list.clone(), update.clone(), message.clone());
        move |b| {
            b.set_sensitive(false);
            crate::say(&message, &tr("settings-dev-fw-looking"));
            let (b, list, update, message) = (b.clone(), list.clone(), update.clone(), message.clone());
            glib::spawn_future_local(async move {
                let all = gio::spawn_blocking(|| {
                    // New metadata first (once a day is enough for fwupd).
                    let _ = std::process::Command::new(FWUPDMGR).arg("refresh").stdin(std::process::Stdio::null()).output();
                    firmware_list()
                })
                .await
                .unwrap_or_default();
                clear(&list);
                if all.is_empty() {
                    list.append(&dim(&tr("settings-dev-fw-none")));
                }
                let mut any = false;
                for (name, version, newer) in all {
                    let line = gtk::Box::builder().spacing(10).css_classes(["row"]).build();
                    line.append(&gtk::Label::builder().label(name).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build());
                    let shown = match &newer {
                        Some(n) => format!("{version} → {n}"),
                        None => version,
                    };
                    any |= newer.is_some();
                    line.append(&gtk::Label::builder().label(shown).css_classes(["dim"]).build());
                    list.append(&line);
                }
                update.set_visible(any);
                crate::say(&message, "");
                b.set_sensitive(true);
            });
        }
    });
    update.connect_clicked(move |b| {
        b.set_sensitive(false);
        crate::say(&message, &tr("settings-dev-fw-updating"));
        let (b, message) = (b.clone(), message.clone());
        glib::spawn_future_local(async move {
            let out = gio::spawn_blocking(|| {
                // -y answers fwupdmgr's questions; --no-reboot-check keeps it
                // from restarting the machine by itself.
                std::process::Command::new(FWUPDMGR)
                    .args(["update", "-y", "--no-reboot-check"])
                    .stdin(std::process::Stdio::null())
                    .output()
            })
            .await;
            let said = match out {
                Ok(Ok(o)) if o.status.success() => tr("settings-dev-fw-done"),
                Ok(Ok(o)) => String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("fwupdmgr").to_owned(),
                Ok(Err(e)) => e.to_string(),
                Err(_) => "fwupdmgr".into(),
            };
            crate::say(&message, &said);
            b.set_sensitive(true);
        });
    });
}

pub fn page(dir: &ConfigDir) -> gtk::Widget {
    let message = crate::message();
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();
    let m = message.clone();
    body.append(&card("settings-dev-tb", move |c, again| thunderbolt(c, again, &m)));
    let m = message.clone();
    body.append(&card("settings-dev-gpu", move |c, again| graphics(c, again, &m)));
    let (m, d) = (message.clone(), dir.clone());
    body.append(&card("settings-dev-rotate-title", move |c, again| rotation(&d, c, again, &m)));
    let m = message.clone();
    body.append(&card("settings-dev-fw", move |c, again| firmware(c, again, &m)));
    body.append(&message);
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .build()
        .upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_and_panels() {
        assert_eq!(transform_for("left-up"), Some("90"));
        assert_eq!(transform_for("undefined"), None);
        assert!(internal("eDP-1"));
        assert!(!internal("HDMI-A-1"));
    }
}
