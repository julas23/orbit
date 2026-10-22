//! Orbit-Panel: a column (or row) of independent modules.
//!
//! `panel.yaml` lists the modules for each part of the panel, in order;
//! listing a module turns it on. Each module reads its own options from
//! `modules/<name>.yaml`.

mod bluetooth;
mod board;
mod brightness;
mod calendar;
mod clock;
mod confirm;
mod control;
mod dbusmenu;
mod disk;
mod edit;
mod forecast;
mod maintenance;
mod menu;
mod meters;
mod monitors;
mod network;
mod notifications;
mod pulse;
mod session;
mod sni;
mod spectrum;
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

use board::Form;

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
        edit::new(dir, config)
    } else {
        let part = |entries: &[ModuleEntry]| {
            // 5 px between modules: the panel grid step.
            let part = gtk::Box::new(gtk::Orientation::Horizontal, 5);
            for entry in entries {
                if let Some(module) = module(entry, dir, config.edge, Form::Line) {
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
        "network" | "keyboard" | "battery" | "notifications" | "bluetooth" | "monitors" => {
            &[(Normal, 4, 1), (Full, 4, 2), (Mini, 1, 1)]
        }
        "workspaces" => &[(Normal, 4, 2)],
        "spectrum" => &[(Normal, 4, 2), (Full, 4, 2), (Mini, 2, 1)],
        "cpu" | "ram" | "lan" | "gpu" | "disk" => &[(Normal, 4, 1), (Full, 4, 2), (Mini, 2, 1)],
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

/// Builds the module of `entry`. A module that is unknown or whose file does
/// not parse is left out with an error in the log, instead of taking the
/// whole panel down.
fn module(entry: &ModuleEntry, dir: &ConfigDir, edge: Edge, form: Form) -> Option<gtk::Widget> {
    let name = entry.name.as_str();
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
        "brightness" => dir.module("brightness").map(|c| brightness::new(&c, form)),
        "tray" => dir.module("tray").map(|c| tray::new(&c, vertical)),
        "network" => dir.module("network").map(|c| network::new(&c, form)),
        "keyboard" => dir.module("keyboard").map(|c| status::keyboard(&c, form)),
        "battery" => dir.module("battery").map(|c| status::battery(&c, form)),
        "bluetooth" => dir.module("bluetooth").map(|c| bluetooth::new(&c, form)),
        "monitors" => dir.module("monitors").map(|c| monitors::new(&c, form)),
        "spectrum" => dir.module("spectrum").map(|c| spectrum::new(&c, form)),
        "cpu" => dir.module("cpu").map(|c| meters::cpu(&c, form)),
        "ram" => dir.module("ram").map(|c| meters::ram(&c, form)),
        "lan" => dir.module("lan").map(|c| meters::lan(&c, form)),
        "gpu" => dir.module("gpu").map(|c| meters::gpu(&c, form)),
        // Each entry may show a filesystem of its own.
        "disk" => dir.module::<disk::DiskConfig>("disk").map(|mut c| {
            if let Some(path) = &entry.path {
                c.path = path.clone();
            }
            disk::new(&c, form)
        }),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The meters and disk in every size, drawn by GTK to
    /// `$ORBIT_PANEL_LOOK` (PNG), a few seconds after they start. Run it
    /// on a display of its own: `gtk4-broadwayd :5 &`, then
    /// `GDK_BACKEND=broadway BROADWAY_DISPLAY=:5 ORBIT_PANEL_LOOK=x.png
    /// cargo test -p orbit-panel look -- --ignored`.
    #[test]
    #[ignore]
    fn look() {
        let Ok(path) = std::env::var("ORBIT_PANEL_LOOK") else { return };
        gtk::init().expect("a display");
        let display = gtk::gdk::Display::default().unwrap();
        let _theme = orbit_app::theme::install(&display);
        let css = gtk::CssProvider::new();
        css.load_from_string(&format!("{}\nwindow {{ background: #1b1f27; }}", include_str!("../../../data/style.css")));
        gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        let dir = ConfigDir::new(std::path::Path::new("/nonexistent"));
        let grid = gtk::Grid::builder().row_spacing(10).column_spacing(10).margin_start(10).margin_end(10).margin_top(10).margin_bottom(10).build();
        for (col, form) in [Form::Mini, Form::Normal, Form::Full].into_iter().enumerate() {
            for (row, name) in ["cpu", "ram", "lan", "gpu", "disk"].into_iter().enumerate() {
                let &(_, cols, rows) = shapes(name).iter().find(|s| s.0 == form).unwrap();
                let Some(module) = module(&name.into(), &dir, Edge::Left, form) else { continue };
                let cell = gtk::Box::builder().css_classes(["panel", "grid"]).valign(gtk::Align::Start).halign(gtk::Align::Start).build();
                module.set_size_request(board::span(cols), board::span(rows));
                cell.append(&module);
                grid.attach(&cell, col as i32, row as i32, 1, 1);
            }
        }
        let window = gtk::Window::builder().css_classes(["orbit", "orbit-panel"]).child(&grid).build();
        window.present();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while std::time::Instant::now() < until {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let (w, h) = (window.width(), window.height());
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(w), f64::from(h));
        let node = snapshot.to_node().unwrap();
        let texture = window.renderer().unwrap().render_texture(&node, None);
        texture.save_to_png(&path).unwrap();
    }
}

