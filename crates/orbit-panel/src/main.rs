//! Orbit-Panel: a column (or row) of independent modules.
//!
//! `panel.yaml` lists the modules for each part of the panel, in order;
//! listing a module turns it on. Each module reads its own options from
//! `modules/<name>.yaml`.

mod brightness;
mod calendar;
mod clock;
mod confirm;
mod control;
mod dbusmenu;
mod maintenance;
mod menu;
mod pulse;
mod session;
mod sni;
mod sway;
mod tray;
mod volume;
mod windows;
mod workspaces;

use gtk::{glib, prelude::*};
use orbit_common::{ConfigDir, Placement, config::PanelConfig};

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
    let vertical = config.edge.is_vertical();
    let orientation = if vertical {
        gtk::Orientation::Vertical
    } else {
        gtk::Orientation::Horizontal
    };
    let part = |names: &[String]| {
        // 5 px between modules: the panel grid step.
        let part = gtk::Box::new(orientation, 5);
        // In a vertical panel, two half-width modules in a row share a line.
        let mut waiting: Option<gtk::Widget> = None;
        for name in names {
            let Some(module) = module(name, dir, vertical) else {
                continue;
            };
            if !module.is_visible() {
                continue; // e.g. brightness on a machine without a backlight
            }
            if vertical && span(name) == Span::Half {
                match waiting.take() {
                    Some(first) => part.append(&pair(&first, Some(&module))),
                    None => waiting = Some(module),
                }
                continue;
            }
            if let Some(first) = waiting.take() {
                part.append(&pair(&first, None));
            }
            part.append(&module);
        }
        if let Some(first) = waiting.take() {
            part.append(&pair(&first, None));
        }
        part
    };

    let root = gtk::CenterBox::builder()
        .orientation(orientation)
        .css_classes(["panel"])
        .start_widget(&part(&config.start))
        .center_widget(&part(&config.center))
        .end_widget(&part(&config.end))
        .build();

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Orbit-Panel")
        .css_classes(["orbit", "orbit-panel"])
        .build();
    if config.maintenance {
        window.set_child(Some(&maintenance::wrap(&root)));
    } else {
        window.set_child(Some(&root));
    }
    window.upcast()
}

/// How much of a vertical panel's width a module takes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Span {
    Full,
    /// Half: 65 px of the 140, so two fit side by side with the 10 px gap
    /// between columns.
    Half,
}

fn span(name: &str) -> Span {
    match name {
        "volume" | "brightness" => Span::Half,
        _ => Span::Full,
    }
}

/// A line of two half-width modules; a lone one keeps its half.
fn pair(first: &gtk::Widget, second: Option<&gtk::Widget>) -> gtk::Box {
    let line = gtk::Box::builder().spacing(10).homogeneous(true).build();
    line.append(first);
    match second {
        Some(second) => line.append(second),
        None => line.append(&gtk::Box::new(gtk::Orientation::Horizontal, 0)),
    }
    line
}

/// Builds module `name`. A module that is unknown or whose file does not
/// parse is left out with an error in the log, instead of taking the whole
/// panel down.
fn module(name: &str, dir: &ConfigDir, vertical: bool) -> Option<gtk::Widget> {
    let built = match name {
        "menu" => dir.module("menu").map(|c| menu::new(&c)),
        "clock" => dir.module("clock").map(|c| clock::new(&c, vertical)),
        "calendar" => dir.module("calendar").map(|c| calendar::new(&c, vertical)),
        "session" => dir.module("session").map(|c| session::new(&c, vertical)),
        "volume" => dir.module("volume").map(|c| volume::new(&c, vertical)),
        "brightness" => dir
            .module("brightness")
            .map(|c| brightness::new(&c, vertical)),
        "tray" => dir.module("tray").map(|c| tray::new(&c, vertical)),
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
