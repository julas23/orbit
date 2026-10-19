//! Workspaces module, after the areas of the eww panel.
//!
//! Areas are GLOBAL: every monitor has the same areas and they switch
//! together. In sway, workspace names are unique, so area N is one workspace
//! per monitor — N on the first, stride+N on the second, 2·stride+N on the
//! third — and a workspace's area is its number modulo the stride. Monitor
//! order: the laptop panel (eDP*) first, then top to bottom, left to right.
//! This is the convention of `~/.config/sway/areas.sh`.
//!
//! Each area has a fixed colour. The focused one only turns its label white
//! and bold; an occupied one is semi-bold. Clicking an area takes every
//! monitor there and gives the focus back to the monitor it came from.
//!
//! Block form: a grid, `columns` per row. Line form: one row.

use std::{cell::RefCell, rc::Rc};

use gtk::prelude::*;
use serde::Deserialize;

use crate::sway;

/// `modules/workspaces.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspacesConfig {
    /// How many areas to show, at least.
    pub count: i32,
    /// Areas per row in the block form.
    pub columns: i32,
    /// Workspace-number distance between the same area on two monitors.
    pub stride: i32,
}

impl Default for WorkspacesConfig {
    fn default() -> Self {
        Self {
            count: 4,
            columns: 2,
            stride: 10,
        }
    }
}

pub fn new(config: &WorkspacesConfig, vertical: bool) -> gtk::Widget {
    let grid = gtk::Grid::builder()
        .css_classes(["areas"])
        .column_homogeneous(true)
        .row_homogeneous(true)
        .column_spacing(10)
        .row_spacing(5)
        .build();
    let card = gtk::Box::builder()
        .css_classes(if vertical {
            vec!["card", "workspaces"]
        } else {
            vec!["workspaces"]
        })
        .build();
    card.append(&grid);

    if !sway::available() {
        log::warn!("workspaces: not running under sway; areas will not update");
    }

    let module = Rc::new(Module {
        config: config.clone(),
        vertical,
        grid,
        buttons: RefCell::new(Vec::new()),
        conn: RefCell::new(None),
    });
    module.refresh();
    let weak = Rc::downgrade(&module);
    sway::subscribe(&["workspace", "output"], move || {
        if let Some(module) = weak.upgrade() {
            module.refresh();
        }
    });
    // The subscription holds only a weak reference; the card keeps the
    // module alive for as long as the card exists.
    card.connect_destroy(move |_| {
        let _ = &module;
    });
    card.upcast()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Free,
    Occupied,
    Focused,
}

struct Module {
    config: WorkspacesConfig,
    vertical: bool,
    grid: gtk::Grid,
    buttons: RefCell<Vec<gtk::Button>>,
    /// Kept open between queries; reopened if sway restarts.
    conn: RefCell<Option<sway::Connection>>,
}

impl Module {
    fn with_conn<T>(&self, f: impl Fn(&mut sway::Connection) -> std::io::Result<T>) -> Option<T> {
        let mut conn = self.conn.borrow_mut();
        for _ in 0..2 {
            if conn.is_none() {
                *conn = sway::Connection::open().ok();
            }
            let c = conn.as_mut()?;
            match f(c) {
                Ok(value) => return Some(value),
                Err(_) => *conn = None, // stale socket: reconnect once
            }
        }
        None
    }

    /// Re-reads the workspaces and updates the buttons.
    fn refresh(self: &Rc<Self>) {
        let stride = self.config.stride.max(1);
        let workspaces: Vec<sway::Workspace> = self
            .with_conn(|c| c.query(sway::GET_WORKSPACES))
            .unwrap_or_default();
        let areas: Vec<(i32, bool)> = workspaces
            .iter()
            .filter(|w| w.num > 0)
            .map(|w| (w.num % stride, w.focused))
            .collect();
        let shown = areas
            .iter()
            .map(|&(area, _)| area)
            .max()
            .unwrap_or(0)
            .max(self.config.count);

        self.ensure_buttons(shown);
        for (i, button) in self.buttons.borrow().iter().enumerate() {
            let area = i as i32 + 1;
            let state = if areas.iter().any(|&(a, f)| a == area && f) {
                State::Focused
            } else if areas.iter().any(|&(a, _)| a == area) {
                State::Occupied
            } else {
                State::Free
            };
            let index = format!("area-{}", area.min(4));
            let state = match state {
                State::Free => "free",
                State::Occupied => "occupied",
                State::Focused => "focused",
            };
            button.set_css_classes(&["area", &index, state]);
        }
    }

    /// Makes sure there are `count` buttons, laid out in the grid.
    fn ensure_buttons(self: &Rc<Self>, count: i32) {
        let mut buttons = self.buttons.borrow_mut();
        let columns = if self.vertical {
            self.config.columns.max(1)
        } else {
            i32::MAX
        };
        while (buttons.len() as i32) < count {
            let area = buttons.len() as i32 + 1;
            let button = gtk::Button::builder()
                .label(area.to_string())
                .tooltip_text(format!("área {area} — em todas as telas (Super+{area})"))
                .hexpand(true)
                .build();
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(module) = weak.upgrade() {
                    module.go(area);
                }
            });
            let i = area - 1;
            self.grid.attach(&button, i % columns, i / columns, 1, 1);
            buttons.push(button);
        }
    }

    /// Takes every monitor to `area`, keeping the focus on the current one.
    fn go(&self, area: i32) {
        let outputs: Vec<sway::Output> = self
            .with_conn(|c| c.query(sway::GET_OUTPUTS))
            .unwrap_or_default();
        let command = switch_command(&outputs, area, self.config.stride.max(1));
        if self
            .with_conn(|c| c.request(sway::RUN_COMMAND, &command))
            .is_none()
        {
            log::error!("workspaces: could not reach sway");
        }
    }
}

/// The sway command that takes every active monitor to `area`: the other
/// monitors first and the focused one last, so it keeps the focus. A single
/// message, because sway applies the whole list before sending any event.
fn switch_command(outputs: &[sway::Output], area: i32, stride: i32) -> String {
    let mut active: Vec<&sway::Output> = outputs.iter().filter(|o| o.active).collect();
    active.sort_by_key(|o| (!o.name.starts_with("eDP"), o.rect.y, o.rect.x));

    let mut command = String::new();
    let mut last = String::new();
    for (k, output) in active.iter().enumerate() {
        let step = format!(
            "focus output {}; workspace number {}; ",
            output.name,
            k as i32 * stride + area
        );
        if output.focused {
            last = step;
        } else {
            command.push_str(&step);
        }
    }
    command.push_str(&last);
    if command.is_empty() {
        return format!("workspace number {area}");
    }
    command.trim_end_matches("; ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(name: &str, focused: bool, y: i32) -> sway::Output {
        sway::Output {
            name: name.into(),
            active: true,
            focused,
            rect: sway::Rect { x: 0, y },
        }
    }

    #[test]
    fn laptop_first_and_focus_returns_to_the_current_monitor() {
        // The laptop panel comes first whatever the order sway reports.
        let outputs = [output("HDMI-A-1", false, 1080), output("eDP-1", true, 0)];
        assert_eq!(
            switch_command(&outputs, 2, 10),
            "focus output HDMI-A-1; workspace number 12; focus output eDP-1; workspace number 2"
        );
        let outputs = [output("HDMI-A-1", true, 1080), output("eDP-1", false, 0)];
        assert_eq!(
            switch_command(&outputs, 3, 10),
            "focus output eDP-1; workspace number 3; focus output HDMI-A-1; workspace number 13"
        );
    }

    #[test]
    fn inactive_outputs_are_skipped_and_no_output_falls_back() {
        let mut off = output("DP-1", false, 0);
        off.active = false;
        assert_eq!(
            switch_command(&[output("eDP-1", true, 0), off], 4, 10),
            "focus output eDP-1; workspace number 4"
        );
        assert_eq!(switch_command(&[], 1, 10), "workspace number 1");
    }
}
