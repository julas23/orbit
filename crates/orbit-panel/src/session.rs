//! Session module, after the session buttons of the eww panel: power off,
//! reboot and log out — always confirmed first (see `confirm`).
//!
//! An optional `before` command runs after "yes" and before acting, e.g. to
//! wait until a file sync has finished. If it fails or times out, a second
//! question asks whether to go on anyway.
//!
//! Block form: three buttons side by side, each its own card — one grid
//! column each (3 × 40 + 2 × 10 = 140 px). Line form: one power button that
//! opens the three in a popover.

use std::rc::Rc;

use gtk::{gio, glib, prelude::*};
use serde::Deserialize;

use crate::{
    confirm::{self, Question},
    sway,
};

/// `modules/session.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionConfig {
    pub poweroff: String,
    pub reboot: String,
    /// Empty: ask the compositor to exit (sway: the `exit` command).
    pub logout: String,
    pub before: Option<Before>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            poweroff: "systemctl poweroff".into(),
            reboot: "systemctl reboot".into(),
            logout: String::new(),
            before: None,
        }
    }
}

/// A command run before leaving the session.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Before {
    pub command: String,
    /// Shown as a notification while the command runs.
    pub message: Option<String>,
    /// The question asked when the command fails.
    pub failed: Option<String>,
}

#[derive(Clone, Copy)]
enum Action {
    Poweroff,
    Reboot,
    Logout,
}

impl Action {
    const ALL: [Action; 3] = [Action::Poweroff, Action::Reboot, Action::Logout];

    fn class(self) -> &'static str {
        match self {
            Action::Poweroff => "poweroff",
            Action::Reboot => "reboot",
            Action::Logout => "logout",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Action::Poweroff => "\u{f0425}", // 󰐥
            Action::Reboot => "\u{f0709}",   // 󰜉
            Action::Logout => "\u{f0343}",   // 󰍃
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Action::Poweroff => "desligar",
            Action::Reboot => "reiniciar",
            Action::Logout => "encerrar sessão (logoff)",
        }
    }

    fn question(self) -> &'static str {
        match self {
            Action::Poweroff => "Desligar o computador?",
            Action::Reboot => "Reiniciar o computador?",
            Action::Logout => "Encerrar a sessão gráfica?",
        }
    }
}

pub fn new(config: &SessionConfig, vertical: bool) -> gtk::Widget {
    let config = Rc::new(config.clone());
    let row = gtk::Box::builder()
        .css_classes(["session"])
        .spacing(10)
        .homogeneous(true)
        .build();
    for action in Action::ALL {
        let mut classes = vec!["action", action.class()];
        if vertical {
            classes.push("card");
        }
        let button = gtk::Button::builder()
            .label(action.glyph())
            .tooltip_text(action.tooltip())
            .css_classes(classes)
            .build();
        let config = config.clone();
        button.connect_clicked(move |_| confirm_then(action, config.clone()));
        row.append(&button);
    }
    if vertical {
        return row.upcast();
    }

    let popover = gtk::Popover::builder().child(&row).build();
    let button = gtk::Button::builder()
        .label(Action::Poweroff.glyph())
        .tooltip_text("sessão")
        .css_classes(["session-line"])
        .build();
    popover.set_parent(&button);
    button.connect_clicked(move |_| popover.popup());
    button.upcast()
}

fn confirm_then(action: Action, config: Rc<SessionConfig>) {
    confirm::ask(
        Question {
            text: action.question(),
            yes: "Sim",
            no: "Não",
        },
        move |yes| {
            if yes {
                prepare(action, config);
            }
        },
    );
}

/// Runs the `before` command, then acts — or asks again if it failed.
fn prepare(action: Action, config: Rc<SessionConfig>) {
    let Some(before) = config.before.clone() else {
        act(action, &config);
        return;
    };
    if let Some(message) = &before.message {
        notify(message);
    }
    let process = glib::shell_parse_argv(&before.command)
        .map_err(|err| err.to_string())
        .and_then(|argv| {
            let argv: Vec<&std::ffi::OsStr> = argv.iter().map(|a| a.as_ref()).collect();
            gio::Subprocess::newv(&argv, gio::SubprocessFlags::NONE).map_err(|e| e.to_string())
        });
    let process = match process {
        Ok(p) => p,
        Err(err) => {
            log::error!("session: running {:?}: {err}", before.command);
            ask_anyway(action, config, &before);
            return;
        }
    };
    process.wait_check_async(gio::Cancellable::NONE, move |result| match result {
        Ok(()) => act(action, &config),
        Err(err) => {
            log::warn!("session: {:?} failed: {err}", before.command);
            ask_anyway(action, config.clone(), &before);
        }
    });
}

fn ask_anyway(action: Action, config: Rc<SessionConfig>, before: &Before) {
    let text = before
        .failed
        .clone()
        .unwrap_or_else(|| "A preparação para sair não terminou.\nSair mesmo assim?".into());
    confirm::ask(
        Question {
            text: &text,
            yes: "Sair mesmo assim",
            no: "Cancelar",
        },
        move |yes| {
            if yes {
                act(action, &config);
            }
        },
    );
}

fn act(action: Action, config: &SessionConfig) {
    let command = match action {
        Action::Poweroff => &config.poweroff,
        Action::Reboot => &config.reboot,
        Action::Logout => &config.logout,
    };
    if command.is_empty() {
        // Logout without a command: ask the compositor to exit.
        let sent = sway::Connection::open().and_then(|mut c| c.request(sway::RUN_COMMAND, "exit"));
        if let Err(err) = sent {
            log::error!("session: could not ask sway to exit: {err}");
        }
        return;
    }
    if let Err(err) = glib::spawn_command_line_async(command) {
        log::error!("session: running {command:?}: {err}");
    }
}

fn notify(message: &str) {
    let Some(app) = gio::Application::default() else {
        return;
    };
    let notification = gio::Notification::new("Sessão");
    notification.set_body(Some(message));
    app.send_notification(Some("orbit-session"), &notification);
}
