//! The Power page (docs/PLANO-SETTINGS.md §3.1):
//!
//! - the power profile (power-saver, balanced, performance), through
//!   power-profiles-daemon;
//! - idleness: locking, and suspending on mains and on battery, written to
//!   core.yaml for this machine (`idle_lock`, `idle_suspend`,
//!   `idle_suspend_battery`), the core reloaded at once; and the effects
//!   (`effects`: on, off on battery, off);
//! - the battery, when there is one (UPower): its charge, state, time
//!   left, health, and the charge limit that spares it, when the hardware
//!   has one;
//! - what the power key and, with a lid, closing it do (logind, through
//!   orbit-services-helper: an administrator's password).

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::{tr, tr_args};
use orbit_common::ConfigDir;
use orbit_services::{Event, Privileged, Run};

use crate::{core, dbus};

/// The choices of an idle time, in minutes (0: never).
const MINUTES: [u64; 9] = [0, 1, 2, 5, 10, 15, 30, 60, 120];

/// A dropdown of `MINUTES`, on the value nearest `seconds`.
fn minutes_dropdown(seconds: u64) -> gtk::DropDown {
    let labels: Vec<String> = MINUTES
        .iter()
        .map(|m| match m {
            0 => tr("settings-saver-never"),
            m => tr_args("settings-saver-minutes", &[("minutes", (*m as i64).into())]),
        })
        .collect();
    let dropdown = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let minutes = seconds / 60;
    let index = MINUTES
        .iter()
        .enumerate()
        .min_by_key(|(_, m)| m.abs_diff(minutes))
        .map_or(0, |(i, _)| i);
    dropdown.set_selected(index as u32);
    dropdown
}

fn card(title: &str) -> gtk::Box {
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(["card"])
        .build();
    card.append(&gtk::Label::builder().label(title).xalign(0.0).css_classes(["heading"]).build());
    card
}

fn dim(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["dim"]).build()
}

/// Whether this machine has a lid (a laptop's).
fn has_lid() -> bool {
    std::fs::read_dir("/proc/acpi/button/lid").is_ok_and(|mut d| d.next().is_some())
}

pub fn page(dir: &ConfigDir) -> gtk::Widget {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();
    let message = crate::message();
    let battery = battery();

    body.append(&profile_card(&message));
    if let Some(battery) = &battery {
        body.append(&battery_card(battery, &message));
    }
    body.append(&idle_card(dir, battery.is_some(), &message));
    body.append(&buttons_card(&message));
    body.append(&message);
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .build()
        .upcast()
}

// --- The power profile ------------------------------------------------------

fn profile_card(message: &gtk::Label) -> gtk::Box {
    let card = card(&tr("settings-power-profile"));
    let proxy = dbus::system_at(
        "org.freedesktop.UPower.PowerProfiles",
        "/org/freedesktop/UPower/PowerProfiles",
        "org.freedesktop.UPower.PowerProfiles",
    )
    .filter(|p| p.name_owner().is_some());
    let Some(proxy) = proxy else {
        card.append(&dim(&tr("settings-power-no-profiles")));
        return card;
    };
    let available: Vec<String> = proxy
        .cached_property("Profiles")
        .and_then(|v| v.get::<Vec<glib::VariantDict>>())
        .unwrap_or_default()
        .iter()
        .filter_map(|d| d.lookup::<String>("Profile").ok().flatten())
        .collect();
    let active = dbus::string(&proxy, "ActiveProfile");
    let line = gtk::Box::builder().spacing(0).css_classes(["linked"]).homogeneous(true).build();
    let mut first: Option<gtk::ToggleButton> = None;
    for (id, label) in [
        ("power-saver", "settings-power-saver"),
        ("balanced", "settings-power-balanced"),
        ("performance", "settings-power-performance"),
    ] {
        if !available.iter().any(|p| p == id) {
            continue;
        }
        let button = gtk::ToggleButton::with_label(&tr(label));
        button.set_active(active == id);
        if let Some(group) = &first {
            button.set_group(Some(group));
        } else {
            first = Some(button.clone());
        }
        let proxy = proxy.clone();
        let message = message.clone();
        button.connect_toggled(move |button| {
            if !button.is_active() {
                return;
            }
            let proxy = proxy.clone();
            let message = message.clone();
            glib::spawn_future_local(async move {
                let result = dbus::set_property(&proxy, "ActiveProfile", id.to_variant()).await;
                crate::say(&message, &result.err().unwrap_or_default());
            });
        });
        line.append(&button);
    }
    card.append(&line);
    card
}

// --- Idleness and effects ---------------------------------------------------

fn idle_card(dir: &ConfigDir, battery: bool, message: &gtk::Label) -> gtk::Box {
    let card = card(&tr("settings-power-idle"));
    let rows: Vec<(&str, &str, u64)> = {
        let mut rows = vec![
            ("idle_lock", "settings-power-lock-after", core::IDLE_LOCK_DEFAULT),
            ("idle_suspend", "settings-power-suspend-mains", 0),
        ];
        if battery {
            rows.push(("idle_suspend_battery", "settings-power-suspend-battery", 0));
        }
        rows
    };
    for (key, label, default) in rows {
        let dropdown = minutes_dropdown(core::per_host(dir, key, default));
        let dir = dir.clone();
        let message = message.clone();
        dropdown.connect_selected_notify(move |d| {
            let minutes = MINUTES.get(d.selected() as usize).copied().unwrap_or(0);
            let told = core::save_per_host(&dir, key, minutes * 60, default).and_then(|()| core::command("reload"));
            crate::say(&message, &told.err().map(|e| e.to_string()).unwrap_or_default());
        });
        card.append(&crate::row(&tr(label), &dropdown));
    }
    card.append(&dim(&tr("settings-power-idle-tip")));

    let effects = ["on", "auto", "off"];
    let labels = [tr("settings-power-effects-on"), tr("settings-power-effects-auto"), tr("settings-power-effects-off")];
    let dropdown = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let now = core::effects(dir);
    dropdown.set_selected(effects.iter().position(|e| *e == now).unwrap_or(1) as u32);
    let dir = dir.clone();
    let message = message.clone();
    dropdown.connect_selected_notify(move |d| {
        let value = effects.get(d.selected() as usize).copied().unwrap_or("auto");
        let told = core::save_effects(&dir, value).and_then(|()| core::command("reload"));
        crate::say(&message, &told.err().map(|e| e.to_string()).unwrap_or_default());
    });
    let row = crate::row(&tr("settings-power-effects"), &dropdown);
    row.set_tooltip_text(Some(&tr("settings-power-effects-tip")));
    card.append(&row);
    card
}

// --- The battery -------------------------------------------------------------

/// The battery UPower knows, if any: its proxy.
fn battery() -> Option<gio::DBusProxy> {
    let upower = dbus::system_at("org.freedesktop.UPower", "/org/freedesktop/UPower", "org.freedesktop.UPower")?;
    let paths = upower
        .call_sync("EnumerateDevices", None, gio::DBusCallFlags::NONE, -1, gio::Cancellable::NONE)
        .ok()?
        .child_value(0)
        .get::<Vec<glib::variant::ObjectPath>>()?;
    paths.iter().find_map(|path| {
        let device = dbus::system_at("org.freedesktop.UPower", path.as_str(), "org.freedesktop.UPower.Device")?;
        let kind = device.cached_property("Type").and_then(|v| v.get::<u32>()).unwrap_or(0);
        // Type 2 is a battery; PowerSupply: the machine's, not a mouse's.
        (kind == 2 && dbus::boolean(&device, "PowerSupply")).then_some(device)
    })
}

/// "1 h 25 min" from seconds.
fn duration(seconds: i64) -> String {
    let minutes = seconds / 60;
    if minutes >= 60 {
        format!("{} h {} min", minutes / 60, minutes % 60)
    } else {
        format!("{minutes} min")
    }
}

fn battery_card(device: &gio::DBusProxy, message: &gtk::Label) -> gtk::Box {
    let card = card(&tr("settings-power-battery"));
    let percent = device.cached_property("Percentage").and_then(|v| v.get::<f64>()).unwrap_or(0.0);
    let state = device.cached_property("State").and_then(|v| v.get::<u32>()).unwrap_or(0);
    let (state_text, seconds) = match state {
        1 | 5 => (
            tr("settings-power-charging"),
            device.cached_property("TimeToFull").and_then(|v| v.get::<i64>()).unwrap_or(0),
        ),
        2 | 6 => (
            tr("settings-power-discharging"),
            device.cached_property("TimeToEmpty").and_then(|v| v.get::<i64>()).unwrap_or(0),
        ),
        4 => (tr("settings-power-full"), 0),
        _ => (tr("settings-power-unknown"), 0),
    };
    let mut line = format!("{percent:.0}% · {state_text}");
    if seconds > 0 {
        line.push_str(&format!(" · {}", tr_args("settings-power-left", &[("time", duration(seconds).into())])));
    }
    let level = gtk::LevelBar::builder().min_value(0.0).max_value(100.0).value(percent).build();
    card.append(&level);
    card.append(&dim(&line));
    let health = device.cached_property("Capacity").and_then(|v| v.get::<f64>()).unwrap_or(0.0);
    if health > 0.0 {
        card.append(&crate::row(&tr("settings-power-health"), &dim(&format!("{health:.0}%"))));
    }
    if dbus::boolean(device, "ChargeThresholdSupported") {
        let limit = gtk::Switch::builder()
            .active(dbus::boolean(device, "ChargeThresholdEnabled"))
            .halign(gtk::Align::End)
            .build();
        let end = device.cached_property("ChargeEndThreshold").and_then(|v| v.get::<u32>()).unwrap_or(80);
        let row = crate::row(&tr_args("settings-power-limit", &[("percent", i64::from(end).into())]), &limit);
        row.set_tooltip_text(Some(&tr("settings-power-limit-tip")));
        card.append(&row);
        let device = device.clone();
        let message = message.clone();
        limit.connect_state_set(move |switch, on| {
            let device = device.clone();
            let message = message.clone();
            let switch = switch.clone();
            glib::spawn_future_local(async move {
                match dbus::call(&device, "EnableChargeThreshold", (on,).to_variant()).await {
                    Ok(_) => {
                        switch.set_state(on);
                        crate::say(&message, "");
                    }
                    Err(e) => {
                        switch.set_state(!on);
                        crate::say(&message, &e);
                    }
                }
            });
            glib::Propagation::Stop
        });
    }
    card
}

// --- The power key and the lid -----------------------------------------------

/// What the power key and the lid may do, in their dropdowns' order.
const ACTIONS: [(&str, &str); 5] = [
    ("suspend", "settings-power-do-suspend"),
    ("hibernate", "settings-power-do-hibernate"),
    ("poweroff", "settings-power-do-poweroff"),
    ("lock", "settings-power-do-lock"),
    ("ignore", "settings-power-do-nothing"),
];

fn buttons_card(message: &gtk::Label) -> gtk::Box {
    let card = card(&tr("settings-power-buttons"));
    let Some(logind) = dbus::system_at(
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    ) else {
        return card;
    };
    // Hibernating needs a swap as large as the memory; offered when logind
    // says it can.
    let can_hibernate = logind
        .call_sync("CanHibernate", None, gio::DBusCallFlags::NONE, -1, gio::Cancellable::NONE)
        .ok()
        .and_then(|v| v.child_value(0).get::<String>())
        .is_some_and(|v| v == "yes");
    let actions: Vec<(&str, &str)> = ACTIONS
        .iter()
        .copied()
        .filter(|(id, _)| *id != "hibernate" || can_hibernate)
        .collect();
    let mut keys = vec![("HandlePowerKey", "settings-power-key")];
    if has_lid() {
        keys.push(("HandleLidSwitch", "settings-power-lid-battery"));
        keys.push(("HandleLidSwitchExternalPower", "settings-power-lid-mains"));
    }
    for (key, label) in keys {
        let labels: Vec<String> = actions.iter().map(|(_, l)| tr(l)).collect();
        let dropdown = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
        let now = dbus::string(&logind, key);
        let index = actions.iter().position(|(id, _)| *id == now);
        dropdown.set_selected(index.map_or(gtk::INVALID_LIST_POSITION, |i| i as u32));
        let actions = actions.clone();
        let message = message.clone();
        dropdown.connect_selected_notify(move |d| {
            let Some((value, _)) = actions.get(d.selected() as usize).copied() else { return };
            let Ok(step) = Privileged::logind(key, value) else { return };
            set_logind(step, &message);
        });
        card.append(&crate::row(&tr(label), &dropdown));
    }
    card
}

/// Runs the helper's logind step in a thread, saying what went wrong.
fn set_logind(step: Privileged, message: &gtk::Label) {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = (|| {
            let mut run = Run::spawn(&[step], false).map_err(|e| e.to_string())?;
            let mut error = None;
            while let Some(event) = run.next_event() {
                if let Event::Error(e) = event {
                    error = Some(e);
                }
            }
            let status = run.wait().map_err(|e| e.to_string())?;
            if matches!(status.code(), Some(126 | 127)) {
                return Err(tr("settings-svc-cancelled"));
            }
            error.map_or(Ok(()), Err)
        })();
        let _ = tx.send_blocking(result);
    });
    let message = message.clone();
    glib::spawn_future_local(async move {
        if let Ok(result) = rx.recv().await {
            crate::say(&message, &result.err().unwrap_or_default());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(duration(25 * 60), "25 min");
        assert_eq!(duration(85 * 60), "1 h 25 min");
    }
}
