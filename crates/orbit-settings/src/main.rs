//! Orbit-Settings: the wallpaper and the monitors, in one window, written
//! to `core.yaml` and told to the core at once; and the desktop's menu.
//!
//!   orbit-settings [wallpaper|monitors]   the window, on that page
//!   orbit-settings menu X Y OUTPUT        the menu of a right click on the
//!                                         desktop (orbit-core's
//!                                         `desktop_menu`), at X Y of OUTPUT
//!
//! The window is single: started again, the running one comes forward on
//! the page asked for.

mod core;
mod menu;
mod monitors;
mod wallpaper;

use std::{cell::RefCell, path::PathBuf, rc::Rc};

use gtk::{gdk, gio, glib, prelude::*};
use orbit_app::i18n::tr;
use orbit_common::{ConfigDir, style::Style};

const CSS: &str = include_str!("../../../data/settings.css");

fn main() -> glib::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut config = ConfigDir::default_path();
    if let Some(i) = args.iter().position(|a| a == "-c" || a == "--config") {
        args.remove(i);
        if i < args.len() {
            config = PathBuf::from(args.remove(i));
        }
    }
    match args.first().map(String::as_str) {
        Some("menu") => {
            let x = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
            let y = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
            let output = args.get(3).cloned().unwrap_or_default();
            menu::run(config, (x, y), output)
        }
        Some("-h" | "--help") => {
            println!("Usage: orbit-settings [-c DIR] [wallpaper|monitors] | menu X Y OUTPUT");
            glib::ExitCode::SUCCESS
        }
        _ => window(config),
    }
}

/// Installs the stylesheet and the system's colours; kept for the run.
fn style(config: &std::path::Path) -> Option<(Style, orbit_app::theme::Theme)> {
    let display = gdk::Display::default()?;
    let dir = ConfigDir::new(config);
    Some((
        Style::install(&display, CSS, &dir.style_path()),
        orbit_app::theme::install(&display),
    ))
}

fn window(config: PathBuf) -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("io.github.julas23.Orbit.Settings")
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    let keep = Rc::new(RefCell::new(None));
    let stack: Rc<RefCell<Option<gtk::Stack>>> = Rc::default();
    app.connect_command_line(move |app, line| {
        let page = line
            .arguments()
            .iter()
            .skip(1)
            .filter_map(|a| a.to_str().map(str::to_owned))
            .find(|a| a == "wallpaper" || a == "monitors")
            .unwrap_or_else(|| "wallpaper".into());
        if keep.borrow().is_none() {
            *keep.borrow_mut() = style(&config);
        }
        let built = stack.borrow().clone();
        let stack = match built {
            Some(stack) => stack,
            None => {
                let made = build(app, &ConfigDir::new(&config));
                *stack.borrow_mut() = Some(made.clone());
                made
            }
        };
        stack.set_visible_child_name(&page);
        if let Some(window) = app.windows().first() {
            window.present();
        }
        0.into()
    });
    app.run()
}

fn build(app: &gtk::Application, dir: &ConfigDir) -> gtk::Stack {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(tr("settings-title"))
        .default_width(900)
        .default_height(640)
        .css_classes(["orbit-settings"])
        .build();
    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .hexpand(true)
        .build();
    stack.add_titled(&wallpaper::page(dir), Some("wallpaper"), &tr("settings-wallpaper"));
    stack.add_titled(&monitors::page(dir), Some("monitors"), &tr("settings-monitors"));
    let sidebar = gtk::StackSidebar::builder()
        .stack(&stack)
        .css_classes(["sidebar"])
        .build();
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.append(&sidebar);
    body.append(&stack);
    window.set_child(Some(&body));
    let close = gtk::EventControllerKey::new();
    close.connect_key_pressed({
        let window = window.clone();
        move |_, key, _, mods| {
            let ctrl_w = mods.contains(gdk::ModifierType::CONTROL_MASK) && key == gdk::Key::w;
            if key == gdk::Key::Escape || ctrl_w {
                window.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    });
    window.add_controller(close);
    stack
}

/// A titled row of a page: the label on the left, `widget` on the right.
pub fn row(label: &str, widget: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::builder().spacing(10).css_classes(["row"]).build();
    row.append(&gtk::Label::builder().label(label).xalign(0.0).hexpand(true).build());
    row.append(widget);
    row
}

/// A line under a page's controls that says what went wrong, or empty.
pub fn message() -> gtk::Label {
    gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .css_classes(["message"])
        .build()
}

pub fn say(label: &gtk::Label, text: &str) {
    label.set_label(text);
    label.set_visible(!text.is_empty());
}
