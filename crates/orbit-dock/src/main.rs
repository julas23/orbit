//! Orbit-Dock: a row of application launchers, split into groups.
//!
//! Left click launches. Right click opens a menu at the click: "Open", the
//! application's own actions (New Window, New Private Window…) — taken from
//! the system's desktop file of the same application when the dock's own
//! file has none — and its recent files, which open in it.

use std::{collections::HashMap, rc::Rc};

use gio_unix::prelude::*;
use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::tr;
use orbit_common::{
    ConfigDir, Placement,
    config::{Align, DockApp, DockConfig, expand_home},
};

fn main() -> glib::ExitCode {
    orbit_app::run(orbit_app::Surface {
        name: "orbit-dock",
        app_id: "io.github.julas23.Orbit.Dock",
        build,
    })
}

/// Reads `dock.yaml` and builds the dock.
fn build(app: &gtk::Application, dir: &ConfigDir) -> orbit_app::Built {
    let dock = dir.dock()?;
    if !dock.enabled {
        return Ok(None);
    }
    let placement = Placement {
        edge: dock.edge,
        size: dock.size,
        monitor: dock.monitor.clone(),
        reserve: dock.reserve,
        namespace: "orbit-dock",
    };
    Ok(Some((window(app, &dock), placement)))
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

    let system = SystemApps::load();
    let mut last_group: Option<&str> = None;
    for entry in config.apps.iter().filter(|entry| entry.show) {
        let Some(launcher) = Launcher::resolve(entry, &system) else {
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
    /// Where the menu's application actions come from: the entry's own
    /// desktop file, or the system's one for the same application.
    actions: Option<gio_unix::DesktopAppInfo>,
    /// Names the application may be recorded under in the recent files.
    recent_names: Vec<String>,
}

/// How many recent files the menu lists.
const RECENT: usize = 8;

/// The system's applications, by normalized name, executable and window
/// class — to find the desktop file that matches a dock entry.
struct SystemApps(HashMap<String, gio_unix::DesktopAppInfo>);

impl SystemApps {
    fn load() -> Self {
        let mut map = HashMap::new();
        for info in gio::AppInfo::all() {
            let Ok(info) = info.downcast::<gio_unix::DesktopAppInfo>() else {
                continue;
            };
            for key in keys(&info) {
                map.entry(key).or_insert_with(|| info.clone());
            }
        }
        Self(map)
    }

    /// The system entry for the same application as `info`, other than itself.
    fn matching(&self, info: &gio_unix::DesktopAppInfo) -> Option<gio_unix::DesktopAppInfo> {
        keys(info).into_iter().find_map(|key| {
            self.0
                .get(&key)
                .filter(|found| found.filename() != info.filename())
                .cloned()
        })
    }
}

/// "Google Chrome" -> "googlechrome": names compared without case or
/// punctuation ("bambu-studio" matches "Bambu Studio").
fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The names an application goes by: name, executable, window class, id.
fn keys(info: &gio_unix::DesktopAppInfo) -> Vec<String> {
    let executable = info.executable();
    let executable = executable
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let id = info
        .id()
        .map(|id| {
            id.trim_end_matches(".desktop")
                .rsplit('.')
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .unwrap_or_default();
    [
        info.name().to_string(),
        executable,
        info.startup_wm_class()
            .map(|c| c.to_string())
            .unwrap_or_default(),
        id,
    ]
    .iter()
    .map(|k| normalize(k))
    .filter(|k| !k.is_empty())
    .collect()
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
    fn resolve(entry: &DockApp, system: &SystemApps) -> Option<Self> {
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
        // The dock's desktop files are often minimal; the system's one for the
        // same application usually lists its actions.
        let system_info = info.as_ref().and_then(|i| system.matching(i));
        let actions = [info.clone(), system_info.clone()]
            .into_iter()
            .flatten()
            .find(|i| !i.list_actions().is_empty());
        let mut recent_names: Vec<String> = [&info, &system_info]
            .into_iter()
            .flatten()
            .flat_map(keys)
            .collect();
        recent_names.push(normalize(&name));
        recent_names.sort();
        recent_names.dedup();
        Some(Self {
            name,
            icon,
            action,
            actions,
            recent_names,
        })
    }

    fn button(self, icon_size: i32) -> gtk::Button {
        let this = Rc::new(self);
        let image = match &this.icon {
            Some(IconSource::Spec(spec)) => orbit_app::icon(spec, icon_size),
            Some(IconSource::GIcon(gicon)) => {
                let image = gtk::Image::new();
                orbit_app::set_gicon(&image, gicon, icon_size);
                image
            }
            None => orbit_app::icon("application-x-executable", icon_size),
        };
        let button = gtk::Button::builder()
            .css_classes(["app"])
            .tooltip_text(this.name.as_str())
            .child(&image)
            .build();

        {
            let this = this.clone();
            button.connect_clicked(move |button| this.launch(button, &[]));
        }
        let right = gtk::GestureClick::builder().button(3).build();
        let anchor = button.downgrade();
        right.connect_pressed(move |_, _, x, y| {
            if let Some(anchor) = anchor.upgrade() {
                menu(&this, &anchor, Some((x, y)));
            }
        });
        button.add_controller(right);
        button
    }

    /// Starts the application, with `files` to open.
    fn launch(&self, widget: &impl IsA<gtk::Widget>, files: &[gio::File]) {
        let result = match &self.action {
            Action::App(info) => {
                let context = widget.display().app_launch_context();
                info.launch(files, Some(&context))
                    .map_err(|err| err.to_string())
            }
            Action::Command(command) => {
                let paths: Vec<String> = files
                    .iter()
                    .filter_map(|f| f.path())
                    .map(|p| glib::shell_quote(p).to_string_lossy().into_owned())
                    .collect();
                let command = [command.clone()]
                    .into_iter()
                    .chain(paths)
                    .collect::<Vec<_>>()
                    .join(" ");
                glib::spawn_command_line_async(command).map_err(|err| err.to_string())
            }
        };
        if let Err(err) = result {
            log::error!("launching {}: {err}", self.name);
        }
    }

    /// The application's recent files, newest first.
    fn recent(&self) -> Vec<gtk::RecentInfo> {
        let mut items: Vec<gtk::RecentInfo> = gtk::RecentManager::default()
            .items()
            .into_iter()
            .filter(|item| {
                item.applications()
                    .iter()
                    .any(|app| self.recent_names.contains(&normalize(app)))
            })
            .filter(|item| !item.is_local() || item.exists())
            .collect();
        items.sort_by_key(|item| std::cmp::Reverse(item.modified().to_unix()));
        items.truncate(RECENT);
        items
    }
}

/// The right-click menu of a launcher.
fn menu(launcher: &Rc<Launcher>, anchor: &gtk::Button, at: orbit_app::popup::Point) {
    let model = gio::Menu::new();
    let group = gio::SimpleActionGroup::new();
    let add = |section: &gio::Menu, label: &str, name: &str, act: Box<dyn Fn(&gtk::Button)>| {
        section.append(Some(label), Some(&format!("d.{name}")));
        let action = gio::SimpleAction::new(name, None);
        let anchor = anchor.downgrade();
        action.connect_activate(move |_, _| {
            if let Some(anchor) = anchor.upgrade() {
                act(&anchor);
            }
        });
        group.add_action(&action);
    };

    let main = gio::Menu::new();
    {
        let l = launcher.clone();
        add(
            &main,
            &tr("dock-open"),
            "open",
            Box::new(move |w| l.launch(w, &[])),
        );
    }
    if let Some(info) = &launcher.actions {
        for (i, action) in info.list_actions().iter().enumerate() {
            let label = info.action_name(action);
            let (info, action) = (info.clone(), action.to_string());
            add(
                &main,
                &label,
                &format!("a{i}"),
                Box::new(move |w| {
                    let context = w.display().app_launch_context();
                    info.launch_action(&action, Some(&context));
                }),
            );
        }
    }
    model.append_section(None, &main);

    let recent = launcher.recent();
    if !recent.is_empty() {
        let section = gio::Menu::new();
        for (i, item) in recent.iter().enumerate() {
            let file = gio::File::for_uri(&item.uri());
            let l = launcher.clone();
            add(
                &section,
                &item.display_name(),
                &format!("r{i}"),
                Box::new(move |w| l.launch(w, std::slice::from_ref(&file))),
            );
        }
        model.append_section(Some(&tr("dock-recent")), &section);
    }

    let popover = gtk::PopoverMenu::from_model(Some(&model));
    popover.insert_action_group("d", Some(&group));
    orbit_app::popup::open(&popover, anchor, at);
}

/// Loads a desktop entry from a path, or by id from the XDG application dirs.
fn desktop_app_info(spec: &str) -> Option<gio_unix::DesktopAppInfo> {
    if spec.contains('/') {
        gio_unix::DesktopAppInfo::from_filename(expand_home(spec.as_ref()))
    } else {
        gio_unix::DesktopAppInfo::new(spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_without_case_or_punctuation() {
        assert_eq!(normalize("Bambu Studio"), normalize("bambu-studio"));
        assert_eq!(normalize("Google Chrome"), "googlechrome");
        assert_ne!(normalize("Firefox"), normalize("Thunderbird"));
    }

    /// What the menus of this machine's dock would show:
    /// `cargo test -p orbit-dock -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn menus_of_this_dock() {
        gtk::init().expect("a display");
        let dock = orbit_common::ConfigDir::new(&orbit_common::ConfigDir::default_path())
            .dock()
            .expect("dock.yaml");
        let system = SystemApps::load();
        let mut with_menu = 0;
        for entry in dock.apps.iter().filter(|e| e.show) {
            let Some(l) = Launcher::resolve(entry, &system) else {
                continue;
            };
            let actions: Vec<String> = l
                .actions
                .iter()
                .flat_map(|i| {
                    i.list_actions()
                        .iter()
                        .map(|a| i.action_name(a).to_string())
                        .collect::<Vec<_>>()
                })
                .collect();
            let recent = l.recent().len();
            if !actions.is_empty() || recent > 0 {
                with_menu += 1;
                eprintln!("{:<20} actions {:?}  recent {recent}", l.name, actions);
            }
        }
        eprintln!("{with_menu} launchers with more than \"Open\"");
    }
}
