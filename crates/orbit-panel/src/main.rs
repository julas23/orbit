//! Orbit-Panel: a column (or row) of independent modules.
//!
//! `panel.yaml` lists the modules for each part of the panel, in order;
//! listing a module turns it on. Each module reads its own options from
//! `modules/<name>.yaml`.

mod board;
mod brightness;
mod calendar;
mod clock;
mod confirm;
mod control;
mod dbusmenu;
mod forecast;
mod maintenance;
mod menu;
mod notifications;
mod pulse;
mod session;
mod sni;
mod status;
mod sway;
mod tray;
mod volume;
mod weather;
mod weather_data;
mod windows;
mod workspaces;

use gtk::{glib, prelude::*};
use orbit_common::{
    ConfigDir, Placement,
    config::{Edge, ModuleEntry, PanelConfig},
};

use board::{Form, Part};

fn main() -> glib::ExitCode {
    orbit_app::run(orbit_app::Surface {
        name: "orbit-panel",
        app_id: "io.github.julas23.Orbit.Panel",
        build,
    })
}

/// Reads `panel.yaml` and builds the panel.
fn build(app: &gtk::Application, dir: &ConfigDir) -> orbit_app::Built {
    let panel = dir.panel()?;
    if !panel.enabled {
        return Ok(None);
    }
    let placement = Placement {
        edge: panel.edge,
        size: panel.size,
        monitor: panel.monitor.clone(),
        reserve: panel.reserve,
        namespace: "orbit-panel",
    };
    Ok(Some((window(app, dir, &panel), placement)))
}

fn window(app: &gtk::Application, dir: &ConfigDir, config: &PanelConfig) -> gtk::Window {
    let root: gtk::Widget = if config.edge.is_vertical() {
        let board = board::Board::default();
        board.set_css_classes(&["panel", "grid"]);
        for (part, entries) in [
            (Part::Start, &config.start),
            (Part::Center, &config.center),
            (Part::End, &config.end),
        ] {
            for entry in entries {
                let (size, cols, rows) = shape(entry);
                let Some(module) = module(&entry.name, dir, config.edge, size) else {
                    continue;
                };
                if !module.is_visible() {
                    continue; // e.g. brightness on a machine without a backlight
                }
                board.add(&module, &entry.name, part, cols, rows);
            }
        }
        board.upcast()
    } else {
        let part = |entries: &[ModuleEntry]| {
            // 5 px between modules: the panel grid step.
            let part = gtk::Box::new(gtk::Orientation::Horizontal, 5);
            for entry in entries {
                if let Some(module) = module(&entry.name, dir, config.edge, Form::Line) {
                    part.append(&module);
                }
            }
            part
        };
        gtk::CenterBox::builder()
            .css_classes(["panel"])
            .start_widget(&part(&config.start))
            .center_widget(&part(&config.center))
            .end_widget(&part(&config.end))
            .build()
            .upcast()
    };

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Orbit-Panel")
        .css_classes(["orbit", "orbit-panel"])
        .build();
    if config.maintenance {
        window.set_child(Some(&maintenance::wrap(&root, config.edge.is_vertical())));
    } else {
        window.set_child(Some(&root));
    }
    window.upcast()
}

/// The sizes module `name` comes in on the vertical panel's grid, as
/// (size, columns, rows); the first is its default. A module not listed
/// has one shape: the whole width, as many rows as its content needs.
fn shapes(name: &str) -> &'static [(Form, i32, i32)] {
    use Form::{Full, Mini, Normal};
    match name {
        "clock" => &[(Full, 4, 2), (Normal, 4, 1), (Mini, 3, 1)],
        "calendar" | "weather" => &[(Full, 4, 4), (Normal, 4, 2), (Mini, 3, 1)],
        "volume" | "brightness" => &[(Normal, 2, 1), (Full, 4, 1), (Mini, 1, 1)],
        "network" | "keyboard" | "battery" | "notifications" => {
            &[(Normal, 4, 1), (Full, 4, 2), (Mini, 1, 1)]
        }
        "workspaces" => &[(Normal, 4, 2)],
        _ => &[(Normal, 4, 1)],
    }
}

/// The shape `entry` asks for, or the module's default when it has no such
/// size.
fn shape(entry: &ModuleEntry) -> (Form, i32, i32) {
    let shapes = shapes(&entry.name);
    let wanted = entry.size.map(Form::from);
    match wanted.and_then(|form| shapes.iter().find(|s| s.0 == form)) {
        Some(&shape) => shape,
        None => {
            if let Some(form) = wanted {
                log::warn!(
                    "panel: {} has no {} size, using its default",
                    entry.name,
                    form.class()
                );
            }
            shapes[0]
        }
    }
}

/// Builds module `name`. A module that is unknown or whose file does not
/// parse is left out with an error in the log, instead of taking the whole
/// panel down.
fn module(name: &str, dir: &ConfigDir, edge: Edge, form: Form) -> Option<gtk::Widget> {
    let vertical = edge.is_vertical();
    let built = match name {
        // The menu's power button offers the session module's actions.
        "menu" => dir.module("menu").and_then(|c| {
            dir.module("session")
                .map(|session| menu::new(&c, edge, session))
        }),
        "clock" => dir.module("clock").map(|c| clock::new(&c, form)),
        // The season in its title needs the hemisphere: the weather's place.
        "calendar" => dir.module("calendar").map(|c| {
            let weather = dir.module("weather").ok();
            calendar::new(&c, weather.as_ref(), form)
        }),
        "session" => dir.module("session").map(|c| session::new(&c, vertical)),
        "volume" => dir.module("volume").map(|c| volume::new(&c, form)),
        "brightness" => dir
            .module("brightness")
            .map(|c| brightness::new(&c, form)),
        "tray" => dir.module("tray").map(|c| tray::new(&c, vertical)),
        "network" => dir.module("network").map(|c| status::network(&c, form)),
        "keyboard" => dir
            .module("keyboard")
            .map(|c| status::keyboard(&c, form)),
        "battery" => dir.module("battery").map(|c| status::battery(&c, form)),
        "notifications" => dir
            .module("notifications")
            .map(|c| notifications::new(&c, form)),
        "weather" => dir.module("weather").map(|c| weather::new(&c, form)),
        // The forecast shares the weather module's source and icons.
        "forecast" => dir.module("forecast").and_then(|f| {
            dir.module("weather")
                .map(|w| forecast::new(&f, &w, vertical))
        }),
        "windows" => dir.module("windows").map(|c| windows::new(&c, vertical)),
        "workspaces" => dir
            .module("workspaces")
            .map(|c| workspaces::new(&c, vertical)),
        other => {
            log::error!("panel: unknown module {other:?}, leaving it out");
            return None;
        }
    };
    built
        .inspect_err(|err| log::error!("panel: module {name}: {err:#}, leaving it out"))
        .ok()
}
