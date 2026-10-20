//! The options window: what to capture, after how long, with or without
//! the pointer. Opened by `orbit-blink` without a mode (Super+Print).

use std::time::Duration;

use gtk::{glib, prelude::*};
use orbit_app::i18n::tr;

use crate::{Mode, Request};

/// Time for the window to leave the screen before the capture.
const VANISH: Duration = Duration::from_millis(300);

pub fn show(app: &gtk::Application) {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(tr("blink-title"))
        .css_classes(["orbit-blink"])
        .resizable(false)
        .build();

    let modes = gtk::Box::builder().css_classes(["linked", "modes"]).build();
    let mut group: Option<gtk::ToggleButton> = None;
    let mut buttons = Vec::new();
    for (mode, icon, label) in [
        (Mode::Screen, "video-display-symbolic", "blink-mode-screen"),
        (Mode::Window, "window-new-symbolic", "blink-mode-window"),
        (Mode::Area, "edit-select-all-symbolic", "blink-mode-area"),
    ] {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(5)
            .build();
        content.append(&gtk::Image::builder().icon_name(icon).pixel_size(30).build());
        content.append(&gtk::Label::new(Some(&tr(label))));
        let button = gtk::ToggleButton::builder()
            .child(&content)
            .active(mode == Mode::Area)
            .hexpand(true)
            .build();
        button.set_group(group.as_ref());
        group.get_or_insert_with(|| button.clone());
        modes.append(&button);
        buttons.push((mode, button));
    }

    let delay = gtk::SpinButton::with_range(0.0, 10.0, 1.0);
    delay.set_value(0.0);
    let delay_row = gtk::Box::builder().spacing(10).build();
    delay_row.append(
        &gtk::Label::builder()
            .label(tr("blink-delay"))
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    delay_row.append(&delay);

    let cursor = gtk::CheckButton::with_label(&tr("blink-cursor"));

    let take = gtk::Button::builder()
        .label(tr("blink-take"))
        .css_classes(["suggested-action", "take"])
        .build();

    let page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(15)
        .css_classes(["page"])
        .build();
    page.append(&modes);
    page.append(&delay_row);
    page.append(&cursor);
    page.append(&take);
    window.set_child(Some(&page));
    window.set_default_widget(Some(&take));

    {
        let window = window.clone();
        let app = app.clone();
        take.connect_clicked(move |_| {
            let mode = buttons
                .iter()
                .find(|(_, b)| b.is_active())
                .map_or(Mode::Area, |(m, _)| *m);
            let request = Request {
                mode,
                delay: Duration::from_secs_f64(delay.value()) + VANISH,
                cursor: cursor.is_active(),
            };
            // Keep running once the window is gone.
            let hold = app.hold();
            window.destroy();
            crate::start(&app, request);
            glib::timeout_add_local_once(request.delay, move || drop(hold));
        });
    }
    let keys = gtk::EventControllerKey::new();
    {
        let window = window.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                window.destroy();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    window.add_controller(keys);
    window.present();
}
