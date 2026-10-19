//! Orbit-Panel: app menu at the start, status widgets at the end.

mod clock;
mod maintenance;

use gtk::{glib, prelude::*};
use orbit_common::{
    Config, Placement,
    config::{MenuConfig, PanelConfig},
};

fn main() -> glib::ExitCode {
    orbit_app::run(orbit_app::Surface {
        name: "orbit-panel",
        app_id: "io.github.julas23.Orbit.Panel",
        build,
    })
}

fn build(app: &gtk::Application, config: &Config) -> Option<(gtk::Window, Placement)> {
    let panel = &config.panel;
    if !panel.enabled {
        return None;
    }
    let placement = Placement {
        edge: panel.edge,
        size: panel.size,
        monitor: panel.monitor.clone(),
        reserve: panel.reserve,
        namespace: "orbit-panel",
    };
    Some((window(app, panel), placement))
}

fn window(app: &gtk::Application, config: &PanelConfig) -> gtk::Window {
    let orientation = if config.edge.is_vertical() {
        gtk::Orientation::Vertical
    } else {
        gtk::Orientation::Horizontal
    };

    let start = gtk::Box::new(orientation, 8);
    if let Some(menu) = &config.menu {
        start.append(&menu_button(menu));
    }

    let end = gtk::Box::new(orientation, 4);
    end.append(&clock::new());

    let root = gtk::CenterBox::builder()
        .orientation(orientation)
        .css_classes(["panel"])
        .start_widget(&start)
        .end_widget(&end)
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

fn menu_button(menu: &MenuConfig) -> gtk::Button {
    let button = gtk::Button::builder().css_classes(["menu"]).build();
    match &menu.icon {
        Some(icon) => button.set_child(Some(&orbit_app::icon(icon, 26))),
        None => button.set_label("Apps"),
    }
    button.set_tooltip_text(menu.tooltip.as_deref());

    let command = menu.command.clone();
    button.connect_clicked(move |_| {
        if let Err(err) = glib::spawn_command_line_async(&command) {
            log::error!("running {command:?}: {err}");
        }
    });
    button
}
