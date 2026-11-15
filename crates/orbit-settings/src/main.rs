//! Orbit-Settings: every setting of the desktop, in one window
//! (docs/PLANO-SETTINGS.md): the pages in groups with a search over all of
//! them (`pages.rs`); and the desktop's menu.
//!
//!   orbit-settings [PAGE[/SECTION]]   the window, on that page (`services`,
//!                                     `monitors`, `wallpaper`…)
//!   orbit-settings menu X Y OUTPUT    the menu of a right click on the
//!                                     desktop (orbit-core's
//!                                     `desktop_menu`), at X Y of OUTPUT
//!
//! The window is single: started again, the running one comes forward on
//! the page asked for. Typing anywhere in it, or Ctrl+F, searches.

mod about;
mod core;
mod datetime;
mod dbus;
mod menu;
mod monitors;
mod network;
mod pages;
mod region;
mod services;
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
            let ids: Vec<&str> = pages::PAGES.iter().map(|p| p.id).collect();
            println!("Usage: orbit-settings [-c DIR] [{}] | menu X Y OUTPUT", ids.join("|"));
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
            .filter_map(|a| a.to_str().and_then(pages::find))
            .next()
            .map_or("wallpaper", |p| p.id);
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
        stack.set_visible_child_name(page);
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
    // Each page as wide as it needs, not as the widest one: the window fits
    // the tile the core gives it.
    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .hexpand(true)
        .hhomogeneous(false)
        .vhomogeneous(false)
        .build();
    for page in pages::PAGES {
        let widget = match page.id {
            "wallpaper" => wallpaper::page(dir),
            "monitors" => monitors::page(dir),
            "network" => network::page(),
            "datetime" => datetime::page(),
            "region" => region::page(),
            "about" => about::page(),
            "services" => services::page(),
            _ => continue,
        };
        stack.add_titled(&widget, Some(page.id), &tr(page.title));
    }
    let (sidebar, search) = pages::sidebar(&stack);
    search.set_key_capture_widget(Some(&window));
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.append(&sidebar);
    body.append(&stack);
    window.set_child(Some(&body));
    let close = gtk::EventControllerKey::new();
    close.connect_key_pressed({
        let window = window.clone();
        move |_, key, _, mods| {
            let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
            if ctrl && key == gdk::Key::f {
                search.grab_focus();
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::Escape && !search.text().is_empty() {
                search.set_text("");
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::Escape || (ctrl && key == gdk::Key::w) {
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
