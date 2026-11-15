//! The Lock screen page (docs/PLANO-SETTINGS.md §3.4): when the session
//! locks and the monitors go off (core.yaml, this machine's, as the Power
//! and Monitors pages write them), the lock's effects, whether the greet
//! shows the other people (greet.yaml `people`), and locking now. The core
//! always locks before the machine sleeps. Whose session opens at boot is
//! in Users and groups.

use gtk::prelude::*;
use orbit_app::i18n::tr;
use orbit_common::ConfigDir;
use serde_yaml_ng::Value;

use crate::{core, power};

fn greet_people(dir: &ConfigDir) -> bool {
    std::fs::read_to_string(dir.path().join("greet.yaml"))
        .ok()
        .and_then(|t| serde_yaml_ng::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("people").and_then(Value::as_bool))
        .unwrap_or(true)
}

pub fn page(dir: &ConfigDir) -> gtk::Widget {
    let message = crate::message();
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();

    let locking = crate::power_card(&tr("settings-lock-when"));
    locking.append(&power::idle_row(dir, "idle_lock", "settings-power-lock-after", core::IDLE_LOCK_DEFAULT, &message));
    locking.append(&power::idle_row(dir, "idle_off", "settings-lock-screen-off", core::IDLE_OFF_DEFAULT, &message));
    locking.append(&gtk::Label::builder().label(tr("settings-lock-sleep")).xalign(0.0).wrap(true).css_classes(["dim"]).build());
    let now = gtk::Button::with_label(&tr("settings-lock-now"));
    now.set_halign(gtk::Align::End);
    now.connect_clicked({
        let message = message.clone();
        move |_| crate::say(&message, &core::command("lock").err().map(|e| e.to_string()).unwrap_or_default())
    });
    locking.append(&now);
    body.append(&locking);

    let screen = crate::power_card(&tr("settings-lock-screen"));
    screen.append(&power::effects_row(dir, &message));
    let people = gtk::Switch::builder().active(greet_people(dir)).halign(gtk::Align::End).valign(gtk::Align::Center).build();
    people.connect_active_notify({
        let (dir, message) = (dir.clone(), message.clone());
        move |s| {
            let told = dir.save_app_keys("greet", &["people"], &format!("people: {}\n", s.is_active()));
            crate::say(&message, &told.err().map(|e| e.to_string()).unwrap_or_default());
        }
    });
    let row = crate::row(&tr("settings-lock-people"), &people);
    row.set_tooltip_text(Some(&tr("settings-lock-people-tip")));
    screen.append(&row);
    screen.append(&gtk::Label::builder().label(tr("settings-lock-boot")).xalign(0.0).wrap(true).css_classes(["dim"]).build());
    body.append(&screen);

    body.append(&message);
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .build()
        .upcast()
}
