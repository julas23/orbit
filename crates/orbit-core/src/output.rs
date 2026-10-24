//! The monitors: where each sits in the layout, which one is under the
//! pointer, and what happens when one comes or goes.
//!
//! A monitor goes where `output … pos` last put it (remembered between
//! sessions, see `config::placed`) or where `outputs:` in the config says;
//! the others stack downwards, the laptop's own panel first, each centred
//! on the widest (the hp's desk: the laptop on top, the external screen
//! below it). Every change is told to the IPC's `output` subscribers.

use std::collections::HashMap;

use smithay::{
    output::Output,
    utils::{Logical, Point, Rectangle},
};

use crate::{config::OutputConfig, state::Orbit};

/// Where each monitor goes: `outputs` are names and logical sizes, in the
/// order they came; the answer is in the same order.
pub fn place(outputs: &[(String, (i32, i32))], config: &HashMap<String, OutputConfig>) -> Vec<(i32, i32)> {
    let fixed = |name: &str| config.get(name).and_then(|c| c.position);
    // The free ones start below whatever has a fixed place.
    let mut y = outputs.iter().filter_map(|(name, (_, h))| fixed(name).map(|(_, fy)| fy + h)).max().unwrap_or(0);
    let widest = outputs.iter().map(|(_, (w, _))| *w).max().unwrap_or(0);
    let mut free: Vec<usize> = (0..outputs.len()).filter(|i| fixed(&outputs[*i].0).is_none()).collect();
    free.sort_by_key(|i| (!internal(&outputs[*i].0), outputs[*i].0.clone()));
    let mut places = vec![(0, 0); outputs.len()];
    for (i, (name, _)) in outputs.iter().enumerate() {
        if let Some(at) = fixed(name) {
            places[i] = at;
        }
    }
    for i in free {
        let (w, h) = outputs[i].1;
        places[i] = ((widest - w) / 2, y);
        y += h;
    }
    places
}

/// The middle of `r`.
pub fn center(r: Rectangle<i32, Logical>) -> Point<i32, Logical> {
    r.loc + Point::from((r.size.w / 2, r.size.h / 2))
}

/// A laptop's own panel.
fn internal(name: &str) -> bool {
    ["eDP", "LVDS", "DSI"].iter().any(|p| name.starts_with(p))
}

impl Orbit {
    /// The monitor called `name`.
    pub fn output_named(&self, name: &str) -> Option<Output> {
        self.space.outputs().find(|o| o.name() == name).cloned()
    }

    /// The monitor at `pos`, or the first when `pos` is on none.
    pub fn output_at(&self, pos: Point<f64, Logical>) -> Option<Output> {
        self.space
            .outputs()
            .find(|o| self.space.output_geometry(o).is_some_and(|g| g.to_f64().contains(pos)))
            .or_else(|| self.space.outputs().next())
            .cloned()
    }

    /// The monitor under the pointer: new windows open there.
    pub fn pointer_output(&self) -> Option<Output> {
        let pos = self.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default();
        self.output_at(pos)
    }

    /// The monitor called `which`, or the nearest one that way from the
    /// focused one (`up`, `down`, `left`, `right`).
    pub fn output_toward(&self, which: &str) -> Option<String> {
        if self.output_named(which).is_some() {
            return Some(which.to_owned());
        }
        let here = center(self.space.output_geometry(&self.output()?)?);
        let (dx, dy) = match which {
            "left" => (-1, 0),
            "right" => (1, 0),
            "up" => (-1, -1),
            "down" => (1, 1),
            _ => return None,
        };
        let vertical = dx == dy;
        self.space
            .outputs()
            .filter_map(|o| Some((o.name(), center(self.space.output_geometry(o)?) - here)))
            .filter(|(_, d)| if vertical { d.y * dy > 0 } else { d.x * dx > 0 })
            .min_by_key(|(_, d)| d.x.abs() + d.y.abs())
            .map(|(name, _)| name)
    }

    /// Lays every monitor out again (one came or went, or the config
    /// changed).
    pub fn place_outputs(&mut self) {
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        // Turned first: a turned monitor's size is the other way round.
        for output in &outputs {
            let turned = self.config.outputs.get(&output.name()).and_then(|o| o.transform.as_deref());
            if let Some(transform) = turned.and_then(crate::config::parse_transform)
                && output.current_transform() != transform
            {
                output.change_current_state(None, Some(transform), None, None);
            }
        }
        let sizes: Vec<(String, (i32, i32))> = outputs
            .iter()
            .map(|o| {
                let size = self.space.output_geometry(o).map(|g| (g.size.w, g.size.h)).unwrap_or_default();
                (o.name(), size)
            })
            .collect();
        for (output, (x, y)) in outputs.iter().zip(place(&sizes, &self.config.outputs)) {
            output.change_current_state(None, None, None, Some((x, y).into()));
            self.space.map_output(output, (x, y));
        }
    }

    /// A monitor came: it takes its place and shows the focused area.
    pub fn add_output(&mut self, output: &Output) {
        self.space.map_output(output, (0, 0));
        self.place_outputs();
        self.renumber();
        self.arrange();
        self.output_event();
    }

    /// Puts monitor `name` at `at` (`output … pos`), now and in the next
    /// sessions.
    pub fn move_output(&mut self, name: &str, at: (i32, i32)) {
        self.config.outputs.entry(name.to_owned()).or_default().position = Some(at);
        crate::config::place(name, at);
        self.place_outputs();
        self.renumber();
        self.arrange();
        self.output_event();
    }

    /// A monitor went: its windows go to the first one left, in the same
    /// areas.
    pub fn remove_output(&mut self, output: &Output) {
        self.space.unmap_output(output);
        self.place_outputs();
        self.renumber();
        self.arrange();
        let focus = self.current_ws().last_focus.clone();
        self.focus_window(focus);
        self.output_event();
    }

    /// The monitors in area order: the laptop's panel first, then top to
    /// bottom, left to right (the panel's order too).
    pub fn monitors(&self) -> Vec<String> {
        let mut list: Vec<(String, (bool, i32, i32))> = self
            .space
            .outputs()
            .map(|o| {
                let loc = self.space.output_geometry(o).map(|g| g.loc).unwrap_or_default();
                (o.name(), (!internal(&o.name()), loc.y, loc.x))
            })
            .collect();
        list.sort_by_key(|(_, key)| *key);
        list.into_iter().map(|(name, _)| name).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outputs(list: &[(&str, i32, i32)]) -> Vec<(String, (i32, i32))> {
        list.iter().map(|(n, w, h)| (n.to_string(), (*w, *h))).collect()
    }

    #[test]
    fn the_laptop_on_top_whatever_came_first() {
        let places = place(&outputs(&[("HDMI-A-1", 1920, 1080), ("eDP-1", 1920, 1080)]), &HashMap::new());
        assert_eq!(places, vec![(0, 1080), (0, 0)]);
    }

    #[test]
    fn a_wider_screen_below_is_centred() {
        let places = place(&outputs(&[("eDP-1", 1920, 1080), ("HDMI-A-4", 3840, 2160)]), &HashMap::new());
        assert_eq!(places, vec![(960, 0), (0, 1080)]);
    }

    #[test]
    fn the_config_wins_and_the_rest_go_below() {
        let mut config = HashMap::new();
        config.insert("HDMI-A-1".to_string(), OutputConfig { position: Some((0, 0)), ..Default::default() });
        let places = place(&outputs(&[("eDP-1", 1920, 1080), ("HDMI-A-1", 2560, 1440)]), &config);
        assert_eq!(places, vec![(320, 1440), (0, 0)]);
    }
}
