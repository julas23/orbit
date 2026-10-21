//! Volume module, after the volume card of the eww panel: the default sink's
//! level on a slider, an icon that follows level and mute.
//!
//! Event-driven (see `pulse`): the card changes as soon as the volume changes
//! anywhere — keys, pavucontrol, another app. Clicking the icon brings the
//! mixer up, focusing the window if it is already open instead of opening a
//! second one; right click toggles mute; the wheel steps the level.

use std::{cell::RefCell, rc::Rc};

use gtk::{glib, prelude::*};
use orbit_app::i18n::{number, text, tr_args};
use serde::Deserialize;

use crate::{
    board::Form,
    control::{Actions, Control},
    pulse, sway,
};

/// `modules/volume.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VolumeConfig {
    /// Opened (or focused) by a click on the icon.
    pub mixer: String,
    /// Percentage per wheel step.
    pub step: u32,
    /// Highest level the slider and the wheel can set.
    pub max: u32,
}

impl Default for VolumeConfig {
    fn default() -> Self {
        Self {
            mixer: "pavucontrol".into(),
            step: 5,
            max: 100,
        }
    }
}

struct Module {
    config: VolumeConfig,
    control: RefCell<Option<Control>>,
    conn: RefCell<Option<pulse::Connection>>,
    sink: RefCell<Option<pulse::Sink>>,
}

pub fn new(config: &VolumeConfig, form: Form) -> gtk::Widget {
    let module = Rc::new(Module {
        config: config.clone(),
        control: RefCell::new(None),
        conn: RefCell::new(None),
        sink: RefCell::new(None),
    });

    let w = Rc::downgrade(&module);
    let set = {
        let w = w.clone();
        move |p: u32| {
            if let Some(m) = w.upgrade() {
                m.set(p);
            }
        }
    };
    let step = {
        let w = w.clone();
        move |s: i32| {
            if let Some(m) = w.upgrade() {
                m.step(s);
            }
        }
    };
    let click = {
        let w = w.clone();
        move || {
            if let Some(m) = w.upgrade() {
                m.open_mixer();
            }
        }
    };
    let mute = move || {
        if let Some(m) = w.upgrade() {
            m.toggle_mute();
        }
    };
    let control = Control::new(
        "volume",
        form,
        Actions {
            set: Box::new(set),
            step: Box::new(step),
            click: Some(Box::new(click)),
            right_click: Some(Box::new(mute)),
        },
    );
    let root = control.root.clone();
    *module.control.borrow_mut() = Some(control);

    module.refresh();
    let weak = Rc::downgrade(&module);
    pulse::subscribe(move || {
        if let Some(module) = weak.upgrade() {
            module.refresh();
        }
    });
    // The subscription holds a weak reference; the widget keeps the module.
    root.connect_destroy(move |_| {
        let _ = &module;
    });
    root
}

impl Module {
    fn with_conn<T>(&self, f: impl Fn(&mut pulse::Connection) -> std::io::Result<T>) -> Option<T> {
        let mut conn = self.conn.borrow_mut();
        for _ in 0..2 {
            if conn.is_none() {
                *conn = pulse::Connection::open()
                    .inspect_err(|err| log::warn!("volume: {err}"))
                    .ok();
            }
            match f(conn.as_mut()?) {
                Ok(value) => return Some(value),
                Err(_) => *conn = None, // stale connection: reconnect once
            }
        }
        None
    }

    fn refresh(&self) {
        let sink = self.with_conn(|c| c.default_sink());
        *self.sink.borrow_mut() = sink;
        let Some(sink) = sink else { return };
        let glyph = if sink.muted {
            "\u{f075f}" // 󰝟
        } else if sink.percent >= 60 {
            "\u{f057e}" // 󰕾
        } else if sink.percent >= 20 {
            "\u{f0580}" // 󰖀
        } else {
            "\u{f057f}" // 󰕿
        };
        let tooltip = tr_args(
            "volume-tooltip",
            &[
                ("percent", number(sink.percent)),
                ("muted", text(if sink.muted { "yes" } else { "no" })),
            ],
        );
        if let Some(control) = &*self.control.borrow() {
            control.show(sink.percent, glyph, &tooltip);
        }
    }

    fn set(&self, percent: u32) {
        let Some(sink) = *self.sink.borrow() else {
            return;
        };
        let percent = percent.min(self.config.max);
        self.with_conn(|c| c.set_volume(&sink, percent));
    }

    fn step(&self, steps: i32) {
        let Some(sink) = *self.sink.borrow() else {
            return;
        };
        let target = sink.percent as i32 + steps * self.config.step as i32;
        self.set(target.clamp(0, self.config.max as i32) as u32);
    }

    fn toggle_mute(&self) {
        let Some(sink) = *self.sink.borrow() else {
            return;
        };
        self.with_conn(|c| c.set_mute(&sink, !sink.muted));
    }

    /// Focuses the mixer if it is open (sway), or starts it.
    fn open_mixer(&self) {
        let program = self
            .config
            .mixer
            .split_whitespace()
            .next()
            .unwrap_or_default();
        let name = program.rsplit('/').next().unwrap_or(program);
        if sway::available() && !name.is_empty() {
            let command = format!("[app_id=\"(?i){}\"] focus", regex_escape(name));
            let focused = sway::Connection::open()
                .and_then(|mut c| c.request(sway::RUN_COMMAND, &command))
                .ok()
                .and_then(|reply| serde_json::from_slice::<Vec<sway::CommandResult>>(&reply).ok())
                .is_some_and(|results| results.iter().all(|r| r.success));
            if focused {
                return;
            }
        }
        if let Err(err) = glib::spawn_command_line_async(&self.config.mixer) {
            log::error!("volume: running {:?}: {err}", self.config.mixer);
        }
    }
}

fn regex_escape(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            let special = "\\.+*?()|[]{}^$".contains(c);
            special.then_some('\\').into_iter().chain([c])
        })
        .collect()
}
