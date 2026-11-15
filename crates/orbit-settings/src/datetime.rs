//! The Date and time page (docs/PLANO-SETTINGS.md §3.2), through timedated:
//! the time now, the time zone (searched by typing in its list) and the
//! time kept by the network (NTP). Changing either asks for an
//! administrator's password once (polkit).

use std::{cell::Cell, process::Command, rc::Rc};

use gtk::{glib, prelude::*};
use orbit_app::i18n::tr;

use crate::dbus;

/// The time zones systemd knows (`timedatectl list-timezones`).
fn zones() -> Vec<String> {
    Command::new("timedatectl")
        .arg("list-timezones")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// "America/Sao_Paulo" as people read it: "America / Sao Paulo".
fn readable(zone: &str) -> String {
    zone.replace('_', " ").replace('/', " / ")
}

pub fn page() -> gtk::Widget {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(["card"])
        .build();
    body.append(&card);
    let message = crate::message();

    let now = gtk::Label::builder().xalign(0.0).css_classes(["heading"]).build();
    card.append(&now);
    let show_now = {
        let now = now.clone();
        move || {
            if let Ok(t) = glib::DateTime::now_local() {
                now.set_label(&t.format("%A, %x  %X  %Z").map(|s| s.to_string()).unwrap_or_default());
            }
        }
    };
    show_now();
    glib::timeout_add_seconds_local(1, move || {
        show_now();
        glib::ControlFlow::Continue
    });

    let Some(proxy) = dbus::system("org.freedesktop.timedate1") else {
        crate::say(&message, &tr("settings-time-unavailable"));
        card.append(&message);
        return body.upcast();
    };

    let list = zones();
    let labels: Vec<String> = list.iter().map(|z| readable(z)).collect();
    let model = gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let zone = gtk::DropDown::builder()
        .model(&model)
        .enable_search(true)
        .expression(gtk::PropertyExpression::new(
            gtk::StringObject::static_type(),
            None::<gtk::Expression>,
            "string",
        ))
        .build();
    // The search matches anywhere in a name, not only at its start.
    zone.set_search_match_mode(gtk::StringFilterMatchMode::Substring);
    let current = dbus::string(&proxy, "Timezone");
    let setting = Rc::new(Cell::new(true));
    if let Some(i) = list.iter().position(|z| *z == current) {
        zone.set_selected(i as u32);
    }
    setting.set(false);
    card.append(&crate::row(&tr("settings-time-zone"), &zone));

    let ntp = gtk::Switch::builder()
        .active(dbus::boolean(&proxy, "NTP"))
        .sensitive(dbus::boolean(&proxy, "CanNTP"))
        .halign(gtk::Align::End)
        .build();
    let ntp_row = crate::row(&tr("settings-time-ntp"), &ntp);
    ntp_row.set_tooltip_text(Some(&tr("settings-time-ntp-tip")));
    card.append(&ntp_row);
    card.append(&message);

    zone.connect_selected_notify({
        let proxy = proxy.clone();
        let message = message.clone();
        let setting = setting.clone();
        move |zone| {
            if setting.get() {
                return;
            }
            let Some(name) = list.get(zone.selected() as usize).cloned() else { return };
            let proxy = proxy.clone();
            let message = message.clone();
            let zone = zone.clone();
            let setting = setting.clone();
            let before = dbus::string(&proxy, "Timezone");
            let back = list.iter().position(|z| *z == before);
            glib::spawn_future_local(async move {
                match dbus::call(&proxy, "SetTimezone", (name.as_str(), true).to_variant()).await {
                    Ok(_) => crate::say(&message, ""),
                    Err(e) => {
                        crate::say(&message, &e);
                        if let Some(i) = back {
                            setting.set(true);
                            zone.set_selected(i as u32);
                            setting.set(false);
                        }
                    }
                }
            });
        }
    });
    ntp.connect_state_set({
        let message = message.clone();
        move |switch, on| {
            let proxy = proxy.clone();
            let message = message.clone();
            let switch = switch.clone();
            glib::spawn_future_local(async move {
                match dbus::call(&proxy, "SetNTP", (on, true).to_variant()).await {
                    Ok(_) => {
                        switch.set_state(on);
                        crate::say(&message, "");
                    }
                    Err(e) => {
                        // set_state, not set_active: no new state-set.
                        switch.set_state(!on);
                        crate::say(&message, &e);
                    }
                }
            });
            glib::Propagation::Stop
        }
    });

    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build()
        .upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones_read() {
        assert_eq!(readable("America/Sao_Paulo"), "America / Sao Paulo");
        assert_eq!(readable("UTC"), "UTC");
    }
}
