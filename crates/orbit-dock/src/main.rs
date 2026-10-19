//! Orbit-Dock: a row of application launchers, split into groups.

use gio_unix::prelude::*;
use gtk::{gio, glib, prelude::*};
use orbit_common::{
    Config, Placement,
    config::{Align, DockApp, DockConfig, expand_home},
};

fn main() -> glib::ExitCode {
    orbit_app::run(orbit_app::Surface {
        name: "orbit-dock",
        app_id: "io.github.julas23.Orbit.Dock",
        build,
    })
}

fn build(app: &gtk::Application, config: &Config) -> Option<(gtk::Window, Placement)> {
    let dock = &config.dock;
    if !dock.enabled {
        return None;
    }
    let placement = Placement {
        edge: dock.edge,
        size: dock.size,
        monitor: dock.monitor.clone(),
        reserve: dock.reserve,
        namespace: "orbit-dock",
    };
    Some((window(app, dock), placement))
}

fn window(app: &gtk::Application, config: &DockConfig) -> gtk::Window {
    let orientation = if config.edge.is_vertical() {
        gtk::Orientation::Vertical
    } else {
        gtk::Orientation::Horizontal
    };
    let align = match config.align {
        Align::Start => gtk::Align::Start,
        Align::Center => gtk::Align::Center,
        Align::End => gtk::Align::End,
    };

    let launchers = gtk::Box::builder()
        .orientation(orientation)
        .spacing(1)
        .halign(if config.edge.is_vertical() {
            gtk::Align::Fill
        } else {
            align
        })
        .valign(if config.edge.is_vertical() {
            align
        } else {
            gtk::Align::Fill
        })
        .build();

    let mut last_group: Option<&str> = None;
    for entry in config.apps.iter().filter(|entry| entry.show) {
        let Some(launcher) = Launcher::resolve(entry) else {
            continue;
        };
        let group = entry.group.as_deref();
        if last_group.is_some() && group != last_group {
            launchers.append(&gtk::Box::builder().css_classes(["separator"]).build());
        }
        last_group = group;
        launchers.append(&launcher.button(config.icon_size));
    }

    let root = gtk::Box::builder().css_classes(["dock"]).build();
    root.append(&launchers);
    launchers.set_hexpand(true);
    launchers.set_vexpand(true);

    gtk::ApplicationWindow::builder()
        .application(app)
        .title("Orbit-Dock")
        .css_classes(["orbit", "orbit-dock"])
        .child(&root)
        .build()
        .upcast()
}

/// A dock entry with its `.desktop` file (if any) merged with explicit fields.
struct Launcher {
    name: String,
    icon: Option<IconSource>,
    action: Action,
}

enum IconSource {
    Spec(String),
    GIcon(gio::Icon),
}

enum Action {
    App(gio_unix::DesktopAppInfo),
    Command(String),
}

impl Launcher {
    fn resolve(entry: &DockApp) -> Option<Self> {
        let info = entry.desktop.as_deref().and_then(desktop_app_info);

        let action = match (&entry.exec, &info) {
            (Some(exec), _) => Action::Command(exec.clone()),
            (None, Some(info)) => Action::App(info.clone()),
            (None, None) => {
                match &entry.desktop {
                    // GIO also rejects entries whose program is not installed.
                    Some(desktop) => log::warn!(
                        "dock: skipping {desktop:?}: file not found, invalid, or its program is not installed"
                    ),
                    None => log::warn!("dock: skipping entry without `desktop` or `exec`"),
                }
                return None;
            }
        };
        let name = entry
            .name
            .clone()
            .or_else(|| info.as_ref().map(|info| info.name().to_string()))
            .unwrap_or_default();
        let icon = match (&entry.icon, &info) {
            (Some(icon), _) => Some(IconSource::Spec(icon.clone())),
            (None, Some(info)) => info.icon().map(IconSource::GIcon),
            (None, None) => None,
        };
        Some(Self { name, icon, action })
    }

    fn button(self, icon_size: i32) -> gtk::Button {
        let image = match &self.icon {
            Some(IconSource::Spec(spec)) => orbit_app::icon(spec, icon_size),
            Some(IconSource::GIcon(gicon)) => {
                let image = gtk::Image::from_gicon(gicon);
                image.set_pixel_size(icon_size);
                image
            }
            None => orbit_app::icon("application-x-executable", icon_size),
        };
        let button = gtk::Button::builder()
            .css_classes(["app"])
            .tooltip_text(self.name.as_str())
            .child(&image)
            .build();

        button.connect_clicked(move |button| self.launch(button));
        button
    }

    fn launch(&self, button: &gtk::Button) {
        let result = match &self.action {
            Action::App(info) => {
                let context = button.display().app_launch_context();
                info.launch(&[], Some(&context))
                    .map_err(|err| err.to_string())
            }
            Action::Command(command) => {
                glib::spawn_command_line_async(command).map_err(|err| err.to_string())
            }
        };
        if let Err(err) = result {
            log::error!("launching {}: {err}", self.name);
        }
    }
}

/// Loads a desktop entry from a path, or by id from the XDG application dirs.
fn desktop_app_info(spec: &str) -> Option<gio_unix::DesktopAppInfo> {
    if spec.contains('/') {
        gio_unix::DesktopAppInfo::from_filename(expand_home(spec.as_ref()))
    } else {
        gio_unix::DesktopAppInfo::new(spec)
    }
}
