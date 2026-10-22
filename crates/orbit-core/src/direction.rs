//! Moving around by direction: `focus left|right|up|down` (Alt+arrows),
//! `move left|right|up|down` (Mod+Ctrl+arrows) and `workspace
//! left|right|up|down` (Ctrl+Alt+arrows, the area beside). The keyboard goes to
//! the nearest window that way, on any monitor; a tiled window swaps
//! places with the nearest tiled one. With none that way, a monitor there
//! is next; then the area beside, in the panel's grid of areas (`areas`,
//! `area_columns`: 1 2 / 3 4), where a moved window goes too. At the
//! grid's edge nothing happens.

use smithay::desktop::Window;

use crate::{floating::floats, state::Orbit, tile::Rect, workspace::info};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "up" => Some(Self::Up),
            "down" => Some(Self::Down),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

/// The candidate nearest to `from` going `dir`: its centre must lie that
/// way; straight ahead counts more than off to the side.
pub fn nearest(from: Rect, candidates: &[Rect], dir: Dir) -> Option<usize> {
    let centre = |r: &Rect| (r.0 as i64 * 2 + r.2 as i64, r.1 as i64 * 2 + r.3 as i64);
    let (fx, fy) = centre(&from);
    candidates
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            let (cx, cy) = centre(r);
            let (ahead, side) = match dir {
                Dir::Left => (fx - cx, cy - fy),
                Dir::Right => (cx - fx, cy - fy),
                Dir::Up => (fy - cy, cx - fx),
                Dir::Down => (cy - fy, cx - fx),
            };
            (ahead > 0).then_some((i, ahead + 2 * side.abs()))
        })
        .min_by_key(|(_, score)| *score)
        .map(|(i, _)| i)
}

/// The area beside `area` going `dir` in a grid of `areas` areas,
/// `columns` per row; `None` at the grid's edge.
pub fn area_toward(area: i32, dir: Dir, areas: i32, columns: i32) -> Option<i32> {
    let columns = columns.max(1);
    let (row, col) = ((area - 1) / columns, (area - 1) % columns);
    let (row, col) = match dir {
        Dir::Left => (row, col - 1),
        Dir::Right => (row, col + 1),
        Dir::Up => (row - 1, col),
        Dir::Down => (row + 1, col),
    };
    let next = row * columns + col + 1;
    (row >= 0 && col >= 0 && col < columns && next <= areas.max(1)).then_some(next)
}

impl Orbit {
    /// The windows of the workspaces on screen.
    fn shown_windows(&self) -> Vec<Window> {
        self.visible.values().flat_map(|n| self.windows_of(*n)).collect()
    }

    /// Where the keyboard is: the focused window's rectangle, or the
    /// focused monitor's area.
    fn focus_rect(&self) -> Option<Rect> {
        match self.focused_window() {
            Some(w) => Some(info(&w).borrow().rect),
            None => self.output().and_then(|o| self.usable_area(&o)),
        }
    }

    /// `workspace left|right|up|down`: every monitor goes to the area
    /// beside; nothing at the grid's edge.
    pub fn workspace_toward(&mut self, dir: Dir) {
        if let Some(num) = self.area_beside(dir, self.monitor_index(&self.focused_output_name())) {
            self.switch_workspace(num);
        }
    }

    /// The area beside the focused one going `dir`, on the `monitor`-th
    /// monitor.
    fn area_beside(&self, dir: Dir, monitor: usize) -> Option<i32> {
        let area = area_toward(self.area_of(self.current).0, dir, self.config.areas, self.config.area_columns)?;
        Some(self.ws_number(area, monitor))
    }

    /// The monitor beside the focused one going `dir`, if any.
    fn output_beside(&self, dir: Dir) -> Option<String> {
        self.output_toward(dir.name()).filter(|o| *o != self.focused_output_name())
    }

    fn monitor_index(&self, output: &str) -> usize {
        self.monitors().iter().position(|o| o == output).unwrap_or(0)
    }

    /// `focus left|right|up|down`.
    pub fn focus_toward(&mut self, dir: Dir) {
        let Some(from) = self.focus_rect() else { return };
        let focused = self.focused_window();
        let others: Vec<Window> = self.shown_windows().into_iter().filter(|w| Some(w) != focused.as_ref()).collect();
        let rects: Vec<Rect> = others.iter().map(|w| info(w).borrow().rect).collect();
        if let Some(i) = nearest(from, &rects, dir) {
            self.focus_window(Some(others[i].clone()));
        } else if let Some(output) = self.output_beside(dir) {
            self.focus_output(&output);
        } else if let Some(num) = self.area_beside(dir, self.monitor_index(&self.focused_output_name())) {
            self.switch_workspace(num);
        }
    }

    /// `move left|right|up|down`: the focused window swaps places with
    /// the tiled one that way, or goes to the monitor or area beside.
    pub fn move_toward(&mut self, dir: Dir) {
        let Some(window) = self.focused_window() else { return };
        let Some(from_ws) = self.workspace_of(&window) else { return };
        let from = info(&window).borrow().rect;
        if !floats(&window) {
            let tiled: Vec<Window> = self.shown_windows().into_iter().filter(|w| *w != window && !floats(w)).collect();
            let rects: Vec<Rect> = tiled.iter().map(|w| info(w).borrow().rect).collect();
            if let Some(i) = nearest(from, &rects, dir) {
                self.swap_windows(&window, &tiled[i]);
                self.focus_window(Some(window));
                return;
            }
        }
        let target = match self.output_beside(dir).and_then(|o| self.visible.get(&o).copied()) {
            Some(num) => num,
            None => {
                let monitor = self.ws_output(from_ws).map(|o| self.monitor_index(&o)).unwrap_or(0);
                let Some(num) = self.area_beside(dir, monitor) else { return };
                self.move_to_workspace(&window, num);
                self.switch_workspace(num);
                self.focus_window(Some(window));
                return;
            }
        };
        self.move_to_workspace(&window, target);
        self.focus_window(Some(window));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_column_that_way() {
        let columns = [(0, 0, 100, 500), (100, 0, 100, 500), (200, 0, 100, 500)];
        let from = columns[1];
        let others = [columns[0], columns[2]];
        assert_eq!(nearest(from, &others, Dir::Right), Some(1));
        assert_eq!(nearest(from, &others, Dir::Left), Some(0));
        // Columns side by side have nothing above or below.
        assert_eq!(nearest(from, &others, Dir::Up), None);
        assert_eq!(nearest(from, &others, Dir::Down), None);
    }

    #[test]
    fn straight_ahead_beats_off_to_the_side() {
        let from = (0, 0, 100, 100);
        let ahead = (300, 0, 100, 100);
        let aside = (150, 400, 100, 100);
        assert_eq!(nearest(from, &[aside, ahead], Dir::Right), Some(1));
    }

    #[test]
    fn the_grid_of_areas() {
        // 1 2
        // 3 4
        assert_eq!(area_toward(1, Dir::Right, 4, 2), Some(2));
        assert_eq!(area_toward(3, Dir::Right, 4, 2), Some(4));
        assert_eq!(area_toward(2, Dir::Right, 4, 2), None);
        assert_eq!(area_toward(1, Dir::Down, 4, 2), Some(3));
        assert_eq!(area_toward(4, Dir::Up, 4, 2), Some(2));
        assert_eq!(area_toward(3, Dir::Down, 4, 2), None);
        assert_eq!(area_toward(1, Dir::Left, 4, 2), None);
        assert_eq!(area_toward(1, Dir::Up, 4, 2), None);
        // A last row not full.
        assert_eq!(area_toward(2, Dir::Down, 3, 2), None);
    }
}
