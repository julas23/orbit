//! The desktop's menu: what a right click on the bare wallpaper opens
//! (orbit-core runs `orbit-settings menu X Y OUTPUT`).
//!
//! A transparent overlay over the whole monitor, with the menu's card at
//! the click: a click beside the card, Escape, or the pointer's other
//! button close it, as a click anywhere else would close a popup menu.

use std::{path::PathBuf, process::Command};

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use orbit_app::i18n::tr;

/// Room the card needs, so one opened near an edge stays on the monitor.
const CARD: (i32, i32) = (240, 110);

pub fn run(config: PathBuf, at: (i32, i32), output: String) -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("io.github.julas23.Orbit.Settings.Menu")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let keep = std::cell::RefCell::new(None);
    app.connect_activate(move |app| {
        *keep.borrow_mut() = crate::style(&config);
        show(app, at, &output);
    });
    app.run_with_args::<&str>(&[])
}

fn show(app: &gtk::Application, (x, y): (i32, i32), output: &str) {
    let window = gtk::Window::builder()
        .application(app)
        .css_classes(["orbit-desktop-menu"])
        .build();
    let monitor = gdk::Display::default().and_then(|display| {
        let monitors = display.monitors();
        (0..monitors.n_items())
            .filter_map(|i| monitors.item(i).and_downcast::<gdk::Monitor>())
            .find(|m| m.connector().is_some_and(|c| c == output))
    });
    if gtk4_layer_shell::is_supported() {
        window.init_layer_shell();
        window.set_namespace(Some("orbit-desktop-menu"));
        window.set_layer(Layer::Overlay);
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(edge, true);
        }
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        if let Some(monitor) = &monitor {
            window.set_monitor(Some(monitor));
        }
    }
    let (w, h) = monitor
        .as_ref()
        .map(|m| (m.geometry().width(), m.geometry().height()))
        .unwrap_or((i32::MAX, i32::MAX));
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .margin_start(x.min(w - CARD.0).max(0))
        .margin_top(y.min(h - CARD.1).max(0))
        .css_classes(["card"])
        .build();
    for (icon, label, page) in [
        (
            "preferences-desktop-wallpaper-symbolic",
            "settings-menu-wallpaper",
            "wallpaper",
        ),
        ("video-display-symbolic", "settings-menu-monitors", "monitors"),
    ] {
        let content = gtk::Box::builder().spacing(10).build();
        content.append(&gtk::Image::from_icon_name(icon));
        content.append(&gtk::Label::builder().label(tr(label)).xalign(0.0).build());
        let item = gtk::Button::builder()
            .child(&content)
            .css_classes(["flat", "item"])
            .build();
        item.connect_clicked({
            let window = window.clone();
            move |_| {
                open(page);
                window.close();
            }
        });
        card.append(&item);
    }
    window.set_child(Some(&card));

    // Beside the card, any button closes.
    let click = gtk::GestureClick::builder().button(0).build();
    click.connect_pressed({
        let (window, card) = (window.clone(), card.clone());
        move |_, _, x, y| {
            let inside = window
                .pick(x, y, gtk::PickFlags::DEFAULT)
                .is_some_and(|w| w == card || w.is_ancestor(&card));
            if !inside {
                window.close();
            }
        }
    });
    window.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let window = window.clone();
        move |_, key, _, _| {
            if key == gdk::Key::Escape {
                window.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    });
    window.add_controller(keys);
    window.present();
}

/// The settings window on `page`, as a program of its own: it outlives
/// the menu.
fn open(page: &str) {
    let program = std::env::current_exe().unwrap_or_else(|_| "orbit-settings".into());
    if let Err(err) = Command::new(program).arg(page).spawn() {
        log::error!("orbit-settings {page}: {err}");
    }
}
