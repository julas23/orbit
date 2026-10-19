//! Menu module: the application-menu button.

use gtk::{glib, prelude::*};
use serde::Deserialize;

/// `modules/menu.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MenuConfig {
    /// Run on click, e.g. `rofi -show drun`.
    pub command: Option<String>,
    /// Icon name from the theme, or a path to an image file.
    pub icon: Option<String>,
    pub tooltip: Option<String>,
}

impl Default for MenuConfig {
    fn default() -> Self {
        Self {
            command: None,
            icon: Some("start-here".into()),
            tooltip: None,
        }
    }
}

pub fn new(config: &MenuConfig) -> gtk::Widget {
    let button = gtk::Button::builder().css_classes(["menu"]).build();
    match &config.icon {
        // 30 px + 5 px padding each side = 40 px, one column of the grid.
        Some(icon) => button.set_child(Some(&orbit_app::icon(icon, 30))),
        None => button.set_label("Apps"),
    }
    button.set_tooltip_text(config.tooltip.as_deref());

    let command = config.command.clone();
    button.connect_clicked(move |_| {
        let Some(command) = &command else {
            log::warn!("menu: no `command` in modules/menu.yaml");
            return;
        };
        if let Err(err) = glib::spawn_command_line_async(command) {
            log::error!("running {command:?}: {err}");
        }
    });
    button.upcast()
}
