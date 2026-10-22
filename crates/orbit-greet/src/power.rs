//! Power off, reboot and suspend from the locked screen, as the panel's
//! session buttons (PLANO-CORE §7.2). Power off and reboot ask first, in
//! the card (`main`); suspend acts at once: the session is already locked.
//!
//! The commands come from `greet.yaml`:
//!
//!   poweroff: systemctl poweroff
//!   reboot: systemctl reboot
//!   suspend: systemctl suspend

use gtk::{glib, prelude::*};
use orbit_app::i18n::tr;
use serde::Deserialize;

/// `~/.config/orbit/greet.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub poweroff: String,
    pub reboot: String,
    pub suspend: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            poweroff: "systemctl poweroff".into(),
            reboot: "systemctl reboot".into(),
            suspend: "systemctl suspend".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Suspend,
    Reboot,
    Poweroff,
}

impl Action {
    const ALL: [Action; 3] = [Action::Suspend, Action::Reboot, Action::Poweroff];

    fn glyph(self) -> &'static str {
        match self {
            Action::Suspend => "\u{f04b2}",  // 󰒲
            Action::Reboot => "\u{f0709}",   // 󰜉
            Action::Poweroff => "\u{f0425}", // 󰐥
        }
    }

    fn class(self) -> &'static str {
        match self {
            Action::Suspend => "suspend",
            Action::Reboot => "reboot",
            Action::Poweroff => "poweroff",
        }
    }

    fn tooltip(self) -> String {
        tr(match self {
            Action::Suspend => "greet-suspend",
            Action::Reboot => "session-reboot",
            Action::Poweroff => "session-poweroff",
        })
    }

    /// The question asked before acting, if any.
    pub fn question(self) -> Option<String> {
        match self {
            Action::Suspend => None,
            Action::Reboot => Some(tr("session-reboot-question")),
            Action::Poweroff => Some(tr("session-poweroff-question")),
        }
    }

    pub fn run(self, config: &Config) {
        let command = match self {
            Action::Suspend => &config.suspend,
            Action::Reboot => &config.reboot,
            Action::Poweroff => &config.poweroff,
        };
        log::info!("{self:?}: {command}");
        if let Err(err) = glib::spawn_command_line_async(command) {
            log::error!("{command:?}: {err}");
        }
    }
}

/// The row of buttons; `pressed` gets the action of the one clicked.
pub fn new(pressed: impl Fn(Action) + Clone + 'static) -> gtk::Widget {
    let row = gtk::Box::builder().spacing(20).css_classes(["power"]).halign(gtk::Align::Center).valign(gtk::Align::End).build();
    for action in Action::ALL {
        let button = gtk::Button::builder()
            .label(action.glyph())
            .tooltip_text(action.tooltip().as_str())
            .css_classes(["action", action.class()])
            .can_focus(false)
            .build();
        let pressed = pressed.clone();
        button.connect_clicked(move |_| pressed(action));
        row.append(&button);
    }
    row.upcast()
}
