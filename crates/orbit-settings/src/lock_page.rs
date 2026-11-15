//! The Lock screen page (docs/PLANO-SETTINGS.md §3.4): when the session
//! locks and the monitors go off (core.yaml, this machine's, as the Power
//! and Monitors pages write them), the lock's effects, whether the greet
//! shows the other people (greet.yaml `people`), and locking now. The core
//! always locks before the machine sleeps. The picture behind the greet:
//! the wallpaper blurred, or one of its own (`wallpaper.lock`), sharp.
//! Whose session opens at boot is in Users and groups.

use std::rc::Rc;

use gtk::{gio, glib, prelude::*};
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

/// What the picture row shows: the file's name, or "the wallpaper".
fn picture_label(path: &str) -> String {
    if path.is_empty() {
        tr("settings-lock-picture-wallpaper")
    } else {
        std::path::Path::new(path).file_name().map_or_else(|| path.to_owned(), |n| n.to_string_lossy().into_owned())
    }
}

/// The lock screen's picture: chosen, or back to the wallpaper.
fn picture_row(dir: &ConfigDir, message: &gtk::Label) -> gtk::Box {
    let now = core::lock_picture(dir);
    let label = gtk::Label::builder().label(picture_label(&now)).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(28).build();
    let choose = gtk::Button::with_label(&tr("settings-lock-picture-choose"));
    let clear = gtk::Button::builder()
        .icon_name("edit-clear-symbolic")
        .tooltip_text(tr("settings-lock-picture-clear"))
        .css_classes(["flat"])
        .sensitive(!now.is_empty())
        .build();
    let controls = gtk::Box::builder().spacing(8).build();
    controls.append(&label);
    controls.append(&choose);
    controls.append(&clear);
    let set: Rc<dyn Fn(String)> = Rc::new({
        let (dir, message, label, clear) = (dir.clone(), message.clone(), label.clone(), clear.clone());
        move |path: String| {
            let told = core::save_lock_picture(&dir, &path).and_then(|()| core::command("reload"));
            crate::say(&message, &told.err().map(|e| format!("{e:#}")).unwrap_or_default());
            label.set_label(&picture_label(&core::lock_picture(&dir)));
            clear.set_sensitive(!core::lock_picture(&dir).is_empty());
        }
    });
    clear.connect_clicked({
        let set = set.clone();
        move |_| set(String::new())
    });
    choose.connect_clicked(move |b| {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(&tr("settings-lock-picture-images")));
        filter.add_pixbuf_formats();
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder().title(tr("settings-lock-picture-choose")).filters(&filters).modal(true).build();
        if let Some(pictures) = glib::user_special_dir(glib::UserDirectory::Pictures) {
            dialog.set_initial_folder(Some(&gio::File::for_path(pictures)));
        }
        let window = b.root().and_downcast::<gtk::Window>();
        let set = set.clone();
        dialog.open(window.as_ref(), None::<&gio::Cancellable>, move |file| {
            if let Some(path) = file.ok().and_then(|f| f.path()) {
                set(crate::wallpaper::short(&path));
            }
        });
    });
    crate::row(&tr("settings-lock-picture"), &controls)
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
    screen.append(&picture_row(dir, &message));
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
