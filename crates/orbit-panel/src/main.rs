//! Orbit-Panel: a column (or row) of independent modules.
//!
//! `panel.yaml` lists the modules for each part of the panel, in order;
//! listing a module turns it on. Each module reads its own options from
//! `modules/<name>.yaml`.

mod clock;
mod maintenance;
mod menu;

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
        for name in names {
            if let Some(module) = module(name, dir, vertical) {
                part.append(&module);
            }
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

/// Builds module `name`. A module that is unknown or whose file does not
/// parse is left out with an error in the log, instead of taking the whole
/// panel down.
fn module(name: &str, dir: &ConfigDir, vertical: bool) -> Option<gtk::Widget> {
    let built = match name {
        "menu" => dir.module("menu").map(|c| menu::new(&c)),
        "clock" => dir.module("clock").map(|c| clock::new(&c, vertical)),
        other => {
            log::error!("panel: unknown module {other:?}, leaving it out");
            return None;
        }
    };
    built
        .inspect_err(|err| log::error!("panel: module {name}: {err:#}, leaving it out"))
        .ok()
}
