//! The Notifications page (docs/PLANO-SETTINGS.md §3.3): do not disturb
//! (orbit-notify's own D-Bus, as the panel's bell uses it), and notify.yaml:
//! how long a bubble stays by urgency, how many at once, how many the
//! centre keeps, which monitor, and the programs never kept. orbit-notify
//! reads its file at start: after a change it is started again (its
//! history is on disk and stays).

use std::{process::Command, rc::Rc, time::Duration};

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::tr;
use orbit_common::{ConfigDir, config::NOTIFY_NO_HISTORY};
use serde_yaml_ng::{Mapping, Value};

use crate::core;

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/io/github/julas23/Orbit/Notify";
const IFACE: &str = "io.github.julas23.Orbit.Notify";

fn proxy() -> Option<gio::DBusProxy> {
    gio::DBusProxy::for_bus_sync(
        gio::BusType::Session,
        gio::DBusProxyFlags::NONE,
        None,
        NAME,
        PATH,
        IFACE,
        gio::Cancellable::NONE,
    )
    .ok()
    .filter(|p| p.name_owner().is_some())
}

/// Starts orbit-notify again, to read its file, once the old one has gone
/// (a second one while it lives would only open its centre).
fn restart() {
    std::thread::spawn(|| {
        let _ = Command::new("pkill").args(["-x", "orbit-notify"]).status();
        for _ in 0..30 {
            if !Command::new("pgrep").args(["-x", "orbit-notify"]).status().is_ok_and(|s| s.success()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if let Err(e) = Command::new("setsid").args(["orbit-notify", "--background"]).spawn() {
            log::warn!("orbit-notify: {e}");
        }
    });
}

struct Page {
    dir: ConfigDir,
    message: gtk::Label,
    apps: gtk::Box,
}

impl Page {
    fn file(&self) -> Mapping {
        std::fs::read_to_string(self.dir.path().join("notify.yaml"))
            .ok()
            .and_then(|t| serde_yaml_ng::from_str::<Value>(&t).ok())
            .and_then(|v| v.as_mapping().cloned())
            .unwrap_or_default()
    }

    fn set(&self, key: &str, value: Value) {
        let mut block = Mapping::new();
        block.insert(key.into(), value);
        let text = serde_yaml_ng::to_string(&block).unwrap_or_default();
        let result = self.dir.save_app_keys("notify", &[key], &text).map(|()| restart());
        crate::say(&self.message, &result.err().map(|e| e.to_string()).unwrap_or_default());
    }

    fn number(&self, key: &str, default: i64) -> i64 {
        self.file().get(key).and_then(Value::as_i64).unwrap_or(default)
    }

    fn spin(self: &Rc<Self>, key: &'static str, label: &str, range: (f64, f64), default: i64) -> gtk::Box {
        let spin = gtk::SpinButton::with_range(range.0, range.1, 1.0);
        spin.set_value(self.number(key, default) as f64);
        let page = self.clone();
        spin.connect_value_changed(move |s| page.set(key, (s.value() as i64).into()));
        crate::row(&tr(label), &spin)
    }

    /// A bubble's seconds on screen for one urgency (`timeout.<level>`).
    fn timeout(self: &Rc<Self>, level: &'static str, label: &str, default: i64) -> gtk::Box {
        let now = self
            .file()
            .get("timeout")
            .and_then(Value::as_mapping)
            .and_then(|m| m.get(level))
            .and_then(Value::as_i64)
            .unwrap_or(default);
        let spin = gtk::SpinButton::with_range(0.0, 120.0, 1.0);
        spin.set_value(now as f64);
        spin.set_tooltip_text(Some(&tr("settings-notify-zero")));
        let page = self.clone();
        spin.connect_value_changed(move |s| {
            let mut timeout = page.file().get("timeout").and_then(Value::as_mapping).cloned().unwrap_or_default();
            timeout.insert(level.into(), (s.value() as i64).into());
            page.set("timeout", Value::Mapping(timeout));
        });
        crate::row(&tr(label), &spin)
    }

    fn no_history(&self) -> Vec<String> {
        match self.file().get("no_history").and_then(Value::as_sequence) {
            Some(s) => s.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
            None => NOTIFY_NO_HISTORY.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    fn fill_apps(self: &Rc<Self>) {
        while let Some(child) = self.apps.first_child() {
            self.apps.remove(&child);
        }
        let apps = self.no_history();
        if apps.is_empty() {
            self.apps.append(&gtk::Label::builder().label(tr("settings-panel-empty")).xalign(0.0).css_classes(["dim"]).build());
        }
        for (i, app) in apps.iter().enumerate() {
            let line = gtk::Box::builder().spacing(10).build();
            line.append(&gtk::Label::builder().label(app).xalign(0.0).hexpand(true).build());
            let remove = gtk::Button::builder().icon_name("user-trash-symbolic").css_classes(["flat"]).build();
            let page = self.clone();
            remove.connect_clicked(move |_| {
                let mut apps = page.no_history();
                if i < apps.len() {
                    apps.remove(i);
                }
                page.set("no_history", Value::Sequence(apps.into_iter().map(Value::from).collect()));
                page.fill_apps();
            });
            line.append(&remove);
            self.apps.append(&line);
        }
    }
}

pub fn page(dir: &ConfigDir) -> gtk::Widget {
    let page = Rc::new(Page {
        dir: dir.clone(),
        message: crate::message(),
        apps: gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).build(),
    });
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();

    let quiet = crate::power_card(&tr("settings-notify-quiet"));
    let dnd = gtk::Switch::builder().halign(gtk::Align::End).build();
    match proxy() {
        Some(p) => {
            dnd.set_active(p.cached_property("Dnd").and_then(|v| v.get::<bool>()).unwrap_or(false));
            let message = page.message.clone();
            dnd.connect_active_notify(move |s| {
                let on = s.is_active();
                let p = p.clone();
                let message = message.clone();
                glib::spawn_future_local(async move {
                    let result = p.call_future("SetDnd", Some(&(on,).to_variant()), gio::DBusCallFlags::NONE, -1).await;
                    crate::say(&message, &result.err().map(|e| e.to_string()).unwrap_or_default());
                });
            });
        }
        None => {
            dnd.set_sensitive(false);
            quiet.append(&gtk::Label::builder().label(tr("settings-notify-not-running")).xalign(0.0).css_classes(["dim"]).build());
        }
    }
    let row = crate::row(&tr("settings-notify-dnd"), &dnd);
    row.set_tooltip_text(Some(&tr("settings-notify-dnd-tip")));
    quiet.append(&row);
    body.append(&quiet);

    let bubbles = crate::power_card(&tr("settings-notify-bubbles"));
    bubbles.append(&page.timeout("low", "settings-notify-low", 5));
    bubbles.append(&page.timeout("normal", "settings-notify-normal", 8));
    bubbles.append(&page.timeout("critical", "settings-notify-critical", 0));
    bubbles.append(&page.spin("max_popups", "settings-notify-max", (1.0, 10.0), 5));
    bubbles.append(&page.spin("history_limit", "settings-notify-history", (0.0, 1000.0), 200));
    let names: Vec<String> = core::outputs().map(|o| o.into_iter().map(|o| o.name).collect()).unwrap_or_default();
    let mut labels = vec![tr("settings-notify-panel-monitor")];
    labels.extend(names.iter().cloned());
    let monitor = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let now = page.file().get("monitor").and_then(|v| v.as_str().map(str::to_owned));
    monitor.set_selected(now.and_then(|n| names.iter().position(|x| *x == n)).map_or(0, |i| i + 1) as u32);
    monitor.connect_selected_notify({
        let page = page.clone();
        move |d| {
            let value = match d.selected() as usize {
                0 => Value::Null,
                i => names.get(i - 1).cloned().map(Value::from).unwrap_or(Value::Null),
            };
            if value.is_null() {
                let result = page.dir.save_app_keys("notify", &["monitor"], "").map(|()| restart());
                crate::say(&page.message, &result.err().map(|e| e.to_string()).unwrap_or_default());
            } else {
                page.set("monitor", value);
            }
        }
    });
    bubbles.append(&crate::row(&tr("settings-notify-monitor"), &monitor));
    body.append(&bubbles);

    let never = crate::power_card(&tr("settings-notify-never"));
    never.append(&gtk::Label::builder().label(tr("settings-notify-never-tip")).xalign(0.0).wrap(true).css_classes(["dim"]).build());
    never.append(&page.apps);
    let entry = gtk::Entry::builder().placeholder_text(tr("settings-notify-app")).hexpand(true).build();
    let add = gtk::Button::with_label(&tr("settings-net-add"));
    let add_line = gtk::Box::builder().spacing(10).build();
    add_line.append(&entry);
    add_line.append(&add);
    never.append(&add_line);
    let go = {
        let page = page.clone();
        let entry = entry.clone();
        move || {
            let name = entry.text().trim().to_owned();
            if name.is_empty() {
                return;
            }
            let mut apps = page.no_history();
            if !apps.contains(&name) {
                apps.push(name);
                page.set("no_history", Value::Sequence(apps.into_iter().map(Value::from).collect()));
                page.fill_apps();
            }
            entry.set_text("");
        }
    };
    add.connect_clicked({
        let go = go.clone();
        move |_| go()
    });
    entry.connect_activate(move |_| go());
    page.fill_apps();
    body.append(&never);
    body.append(&page.message);
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .build()
        .upcast()
}
