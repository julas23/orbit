//! The Privacy page (docs/PLANO-SETTINGS.md §3.4): recent files (kept or
//! not, for how long, cleared), the trash and temporary files emptied of
//! old ones (GNOME's privacy settings, which GTK and its portal read, and
//! `orbit-housekeeping.timer`, which acts on them), and what sandboxed
//! applications were allowed through the portals (the permission store);
//! and the location service (geoclue, its agent `orbit-settings agent`,
//! the switch and the precision in GNOME's org.gnome.system.location).

use std::process::Command;

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::{tr, tr_args};
use orbit_common::ConfigDir;
use orbit_services::Privileged;

use crate::{agent, housekeeping::TIMER};

/// The precision apps may get, in GSettings' names.
const ACCURACY: [&str; 5] = ["country", "city", "neighborhood", "street", "exact"];
/// What D-Bus starts geoclue from: there when it is installed.
const GEOCLUE_SERVICE: &str = "/usr/share/dbus-1/system-services/org.freedesktop.GeoClue2.service";

const SCHEMA: &str = "org.gnome.desktop.privacy";
/// Days kept, for recent files (-1: forever) and old trash.
const RECENT_DAYS: [i32; 5] = [-1, 1, 7, 30, 90];
const OLD_DAYS: [u32; 5] = [1, 7, 14, 30, 60];
/// The permission store's tables Orbit shows, and what each one is.
const TABLES: [(&str, &str); 5] = [
    ("devices", "settings-privacy-devices"),
    ("location", "settings-privacy-location"),
    ("screenshot", "settings-privacy-screenshot"),
    ("notifications", "settings-privacy-notifications"),
    ("background", "settings-privacy-background"),
];

fn days_label(days: i64) -> String {
    if days < 0 { tr("settings-privacy-forever") } else { tr_args("settings-privacy-days", &[("days", days.into())]) }
}

/// The housekeeping timer on while either kind of cleaning is.
fn timer(settings: &gio::Settings) -> Result<(), String> {
    let on = settings.boolean("remove-old-trash-files") || settings.boolean("remove-old-temp-files");
    let verb = if on { "enable" } else { "disable" };
    let out = Command::new("systemctl")
        .args(["--user", verb, "--now", TIMER])
        .output()
        .map_err(|e| format!("systemctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

fn store() -> Option<gio::DBusProxy> {
    gio::DBusProxy::for_bus_sync(
        gio::BusType::Session,
        gio::DBusProxyFlags::DO_NOT_LOAD_PROPERTIES,
        None,
        "org.freedesktop.impl.portal.PermissionStore",
        "/org/freedesktop/impl/portal/PermissionStore",
        "org.freedesktop.impl.portal.PermissionStore",
        gio::Cancellable::NONE,
    )
    .ok()
}

/// Every (table, id, app, permissions) the store holds, of `TABLES`.
fn grants(store: &gio::DBusProxy) -> Vec<(&'static str, String, String, Vec<String>)> {
    let call = |method: &str, args: glib::Variant| {
        store.call_sync(method, Some(&args), gio::DBusCallFlags::NONE, 2000, gio::Cancellable::NONE).ok()
    };
    let mut out = Vec::new();
    for (table, _) in TABLES {
        let ids = call("List", (table,).to_variant())
            .and_then(|v| v.child_value(0).get::<Vec<String>>())
            .unwrap_or_default();
        for id in ids {
            let Some(found) = call("Lookup", (table, id.as_str()).to_variant()) else { continue };
            let apps = found.child_value(0).get::<std::collections::HashMap<String, Vec<String>>>().unwrap_or_default();
            let mut apps: Vec<_> = apps.into_iter().collect();
            apps.sort();
            for (app, perms) in apps {
                out.push((table, id.clone(), app, perms));
            }
        }
    }
    out
}

fn fill_grants(list: &gtk::Box, message: &gtk::Label) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let Some(store) = store() else {
        list.append(&dim(&tr("settings-privacy-no-store")));
        return;
    };
    let all = grants(&store);
    if all.is_empty() {
        list.append(&dim(&tr("settings-privacy-no-grants")));
    }
    for (table, id, app, perms) in all {
        let line = gtk::Box::builder().spacing(10).css_classes(["row"]).build();
        let names = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
        let shown = if app.is_empty() { tr("settings-privacy-unsandboxed") } else { app.clone() };
        names.append(&gtk::Label::builder().label(shown).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build());
        let what = TABLES.iter().find(|(t, _)| *t == table).map(|(_, l)| tr(l)).unwrap_or_else(|| table.to_owned());
        names.append(&dim(&format!("{what} · {id}: {}", perms.join(", "))));
        line.append(&names);
        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text(tr("settings-privacy-revoke"))
            .css_classes(["flat"])
            .valign(gtk::Align::Center)
            .build();
        let (store, list2, message2) = (store.clone(), list.clone(), message.clone());
        remove.connect_clicked(move |_| {
            let result = store.call_sync(
                "DeletePermission",
                Some(&(table, id.as_str(), app.as_str()).to_variant()),
                gio::DBusCallFlags::NONE,
                2000,
                gio::Cancellable::NONE,
            );
            crate::say(&message2, &result.err().map(|e| e.to_string()).unwrap_or_default());
            fill_grants(&list2, &message2);
        });
        line.append(&remove);
        list.append(&line);
    }
}

fn dim(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["dim"]).build()
}

/// The location service's card, or none without its schema.
fn location_card(dir: &ConfigDir, message: &gtk::Label) -> Option<gtk::Box> {
    gio::SettingsSchemaSource::default()?.lookup("org.gnome.system.location", true)?;
    let settings = gio::Settings::new("org.gnome.system.location");
    let card = crate::power_card(&tr("settings-privacy-location-service"));
    card.append(&dim(&tr("settings-privacy-location-tip")));
    let installed = std::path::Path::new(GEOCLUE_SERVICE).exists();
    let on = gtk::Switch::builder().active(settings.boolean("enabled")).halign(gtk::Align::End).valign(gtk::Align::Center).sensitive(installed).build();
    on.connect_active_notify({
        let (settings, dir, message) = (settings.clone(), dir.clone(), message.clone());
        move |s| {
            let mut result = settings.set_boolean("enabled", s.is_active()).map_err(anyhow::Error::from);
            if s.is_active() && result.is_ok() {
                result = agent::ensure(&dir);
            }
            crate::say(&message, &result.err().map(|e| format!("{e:#}")).unwrap_or_default());
        }
    });
    card.append(&crate::row(&tr("settings-privacy-location-on"), &on));
    let labels: Vec<String> = ACCURACY.iter().map(|a| tr(&format!("settings-privacy-accuracy-{a}"))).collect();
    let level = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let now = settings.string("max-accuracy-level");
    level.set_selected(ACCURACY.iter().position(|a| *a == now.as_str()).unwrap_or(4) as u32);
    level.connect_selected_notify({
        let (settings, message) = (settings.clone(), message.clone());
        move |d| {
            let name = ACCURACY.get(d.selected() as usize).copied().unwrap_or("exact");
            crate::say(&message, &settings.set_string("max-accuracy-level", name).err().map(|e| e.to_string()).unwrap_or_default());
        }
    });
    on.bind_property("active", &level, "sensitive").sync_create().build();
    card.append(&crate::row(&tr("settings-privacy-accuracy"), &level));
    if !installed {
        card.append(&dim(&tr("settings-privacy-no-geoclue")));
        let install = gtk::Button::with_label(&tr("settings-privacy-install-geoclue"));
        install.set_halign(gtk::Align::End);
        let (message, on) = (message.clone(), on.clone());
        install.connect_clicked(move |b| {
            b.set_sensitive(false);
            crate::say(&message, &tr("settings-installing"));
            let (b, message, on) = (b.clone(), message.clone(), on.clone());
            crate::helper::run(vec![Privileged::Install("geoclue-2.0".into())], |_| {}, move |error| {
                let ok = std::path::Path::new(GEOCLUE_SERVICE).exists();
                on.set_sensitive(ok);
                b.set_visible(!ok);
                b.set_sensitive(true);
                crate::say(&message, &error.unwrap_or_default());
            });
        });
        card.append(&install);
    }
    Some(card)
}

pub fn page(dir: &ConfigDir) -> gtk::Widget {
    let message = crate::message();
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();
    let settings = gio::Settings::new(SCHEMA);

    let recent = crate::power_card(&tr("settings-privacy-recent"));
    let remember = gtk::Switch::builder().halign(gtk::Align::End).valign(gtk::Align::Center).build();
    settings.bind("remember-recent-files", &remember, "active").build();
    recent.append(&crate::row(&tr("settings-privacy-remember"), &remember));
    let labels: Vec<String> = RECENT_DAYS.iter().map(|d| days_label(i64::from(*d))).collect();
    let keep = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let now = settings.int("recent-files-max-age");
    keep.set_selected(RECENT_DAYS.iter().position(|d| *d == now).unwrap_or(0) as u32);
    keep.connect_selected_notify({
        let (settings, message) = (settings.clone(), message.clone());
        move |d| {
            let days = RECENT_DAYS.get(d.selected() as usize).copied().unwrap_or(-1);
            crate::say(&message, &settings.set_int("recent-files-max-age", days).err().map(|e| e.to_string()).unwrap_or_default());
        }
    });
    remember.bind_property("active", &keep, "sensitive").sync_create().build();
    recent.append(&crate::row(&tr("settings-privacy-keep-recent"), &keep));
    let clear = gtk::Button::with_label(&tr("settings-privacy-clear-recent"));
    clear.set_halign(gtk::Align::End);
    clear.connect_clicked({
        let message = message.clone();
        move |_| {
            let result = gtk::RecentManager::default().purge_items();
            crate::say(
                &message,
                &match result {
                    Ok(n) => tr_args("settings-privacy-cleared", &[("count", i64::from(n).into())]),
                    Err(e) => e.to_string(),
                },
            );
        }
    });
    recent.append(&clear);
    body.append(&recent);

    let old = crate::power_card(&tr("settings-privacy-old"));
    for (key, label) in [
        ("remove-old-trash-files", "settings-privacy-old-trash"),
        ("remove-old-temp-files", "settings-privacy-old-temp"),
    ] {
        let switch = gtk::Switch::builder().active(settings.boolean(key)).halign(gtk::Align::End).valign(gtk::Align::Center).build();
        let (settings2, message) = (settings.clone(), message.clone());
        switch.connect_active_notify(move |s| {
            let result = settings2
                .set_boolean(key, s.is_active())
                .map_err(|e| e.to_string())
                .and_then(|()| {
                    gio::Settings::sync();
                    timer(&settings2)
                });
            crate::say(&message, &result.err().unwrap_or_default());
        });
        old.append(&crate::row(&tr(label), &switch));
    }
    let labels: Vec<String> = OLD_DAYS.iter().map(|d| days_label(i64::from(*d))).collect();
    let age = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let now = settings.uint("old-files-age");
    age.set_selected(OLD_DAYS.iter().position(|d| *d == now).unwrap_or(3) as u32);
    age.connect_selected_notify({
        let (settings, message) = (settings.clone(), message.clone());
        move |d| {
            let days = OLD_DAYS.get(d.selected() as usize).copied().unwrap_or(30);
            crate::say(&message, &settings.set_uint("old-files-age", days).err().map(|e| e.to_string()).unwrap_or_default());
        }
    });
    old.append(&crate::row(&tr("settings-privacy-older-than"), &age));
    let empty = gtk::Button::with_label(&tr("settings-privacy-empty-trash"));
    empty.set_halign(gtk::Align::End);
    empty.connect_clicked({
        let message = message.clone();
        move |_| {
            let result = Command::new("gio").args(["trash", "--empty"]).status();
            crate::say(
                &message,
                &match result {
                    Ok(s) if s.success() => tr("settings-privacy-emptied"),
                    Ok(_) => "gio trash --empty failed".into(),
                    Err(e) => format!("gio: {e}"),
                },
            );
        }
    });
    old.append(&empty);
    body.append(&old);

    if let Some(card) = location_card(dir, &message) {
        body.append(&card);
    }

    let apps = crate::power_card(&tr("settings-privacy-apps"));
    apps.append(&dim(&tr("settings-privacy-apps-tip")));
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).build();
    fill_grants(&list, &message);
    apps.append(&list);
    body.append(&apps);

    body.append(&message);
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .build()
        .upcast()
}
