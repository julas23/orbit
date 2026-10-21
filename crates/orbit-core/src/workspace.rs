//! Numbered workspaces, as sway's: each belongs to a monitor and holds its
//! windows in their order on screen; each monitor shows one, and the one
//! with the keyboard is the focused workspace (`current`). A workspace
//! exists while it is shown or holds windows. The scratchpad holds windows
//! taken off every workspace — the panel's "minimise".
//!
//! Every change is told to the IPC's subscribers (`workspace` and `window`
//! events), as sway does.

use std::cell::RefCell;

use smithay::desktop::Window;

use crate::state::Orbit;

pub struct Workspace {
    pub num: i32,
    /// The monitor it belongs to (its connector name).
    pub output: String,
    pub windows: Vec<Window>,
    /// The window to give the keyboard back to when the workspace is shown.
    pub last_focus: Option<Window>,
}

impl Workspace {
    pub fn new(num: i32, output: String) -> Self {
        Self { num, output, windows: Vec::new(), last_focus: None }
    }
}

/// What the core keeps with each window: its id (sway's `con_id`) and the
/// rectangle it was last laid out in (kept while its workspace is hidden).
pub struct Info {
    pub id: u64,
    pub rect: (i32, i32, i32, i32),
}

/// The window's info; given on the first call.
pub fn info(window: &Window) -> &RefCell<Info> {
    window.user_data().insert_if_missing(|| RefCell::new(Info { id: 0, rect: (0, 0, 0, 0) }));
    window.user_data().get::<RefCell<Info>>().expect("just inserted")
}

pub fn id(window: &Window) -> u64 {
    info(window).borrow().id
}

impl Orbit {
    /// The focused workspace; made when missing.
    pub fn current_ws(&mut self) -> &mut Workspace {
        let num = self.current;
        let i = self.ensure_ws(num);
        &mut self.workspaces[i]
    }

    /// The windows of the focused workspace, in order.
    pub fn tiled(&self) -> Vec<Window> {
        self.windows_of(self.current)
    }

    /// The windows of workspace `num`, in order.
    pub fn windows_of(&self, num: i32) -> Vec<Window> {
        self.workspaces.iter().find(|w| w.num == num).map(|w| w.windows.clone()).unwrap_or_default()
    }

    /// Every window, on any workspace or in the scratchpad.
    pub fn all_windows(&self) -> impl Iterator<Item = &Window> {
        self.workspaces.iter().flat_map(|w| &w.windows).chain(&self.scratchpad)
    }

    /// Whether a monitor shows workspace `num`.
    pub fn is_visible(&self, num: i32) -> bool {
        self.visible.values().any(|n| *n == num)
    }

    /// The monitor workspace `num` belongs to.
    pub fn ws_output(&self, num: i32) -> Option<String> {
        self.workspaces.iter().find(|w| w.num == num).map(|w| w.output.clone())
    }

    /// The monitor of the focused workspace.
    pub fn focused_output_name(&self) -> String {
        self.ws_output(self.current)
            .or_else(|| self.visible.iter().find(|(_, n)| **n == self.current).map(|(o, _)| o.clone()))
            .or_else(|| self.space.outputs().next().map(|o| o.name()))
            .unwrap_or_default()
    }

    /// The index of workspace `num`, made (and announced) when missing, on
    /// the focused monitor; the list stays in number order.
    pub fn ensure_ws(&mut self, num: i32) -> usize {
        if let Some(i) = self.workspaces.iter().position(|w| w.num == num) {
            return i;
        }
        let output = self.focused_output_name();
        let i = self.workspaces.iter().position(|w| w.num > num).unwrap_or(self.workspaces.len());
        self.workspaces.insert(i, Workspace::new(num, output));
        self.workspace_event("init", num, None);
        i
    }

    /// The workspace a window is on; `None` in the scratchpad.
    pub fn workspace_of(&self, window: &Window) -> Option<i32> {
        self.workspaces.iter().find(|w| w.windows.contains(window)).map(|w| w.num)
    }

    /// Puts workspace `num` on monitor `output` in place of the one it
    /// showed, whose windows leave the screen. Nothing is laid out yet.
    pub fn show_on(&mut self, output: &str, num: i32) {
        if let Some(old) = self.visible.get(output).copied().filter(|old| *old != num) {
            for window in self.windows_of(old) {
                self.space.unmap_elem(&window);
            }
        }
        // Shown elsewhere before: that monitor is left without it.
        self.visible.retain(|_, n| *n != num);
        let i = self.ensure_ws(num);
        self.workspaces[i].output = output.to_owned();
        self.visible.insert(output.to_owned(), num);
        for window in self.windows_of(num) {
            self.space.map_element(window, (0, 0), false);
        }
    }

    /// Focuses workspace `num`: shown on another monitor, the keyboard goes
    /// there; hidden, it comes to its own monitor (a new one, to the
    /// focused monitor) in place of what that showed.
    pub fn switch_workspace(&mut self, num: i32) {
        if num == self.current {
            return;
        }
        let old = self.current;
        self.previous_ws = Some(old);
        if !self.is_visible(num) {
            let output = self.ws_output(num).filter(|o| self.visible.contains_key(o)).unwrap_or_else(|| self.focused_output_name());
            self.show_on(&output, num);
        }
        self.current = num;
        let i = self.ensure_ws(num);
        let ws = &self.workspaces[i];
        let focus = ws.last_focus.clone().filter(|w| ws.windows.contains(w)).or_else(|| ws.windows.last().cloned());
        self.arrange();
        self.focus_window(focus);
        self.workspace_event("focus", num, Some(old));
        self.drop_empty();
    }

    /// The keyboard went to a window on another monitor: its workspace is
    /// the focused one now.
    pub fn follow_focus(&mut self, num: i32) {
        if num == self.current || !self.is_visible(num) {
            return;
        }
        let old = self.current;
        self.previous_ws = Some(old);
        self.current = num;
        self.workspace_event("focus", num, Some(old));
        self.drop_empty();
    }

    /// Moves `window` to workspace `num` (from another, or from the
    /// scratchpad), without focusing that workspace.
    pub fn move_to_workspace(&mut self, window: &Window, num: i32) {
        let was_focused = self.focused_window().as_ref() == Some(window);
        let from = self.workspace_of(window);
        self.take(window);
        let i = self.ensure_ws(num);
        self.workspaces[i].windows.push(window.clone());
        if self.is_visible(num) {
            self.space.map_element(window.clone(), (0, 0), false);
        } else {
            self.space.unmap_elem(window);
        }
        if was_focused && !self.is_visible(num) {
            let next = from.and_then(|n| self.windows_of(n).last().cloned());
            self.focus_window(next);
        }
        self.arrange();
        self.window_event("move", window);
        self.drop_empty();
    }

    /// Takes `window` off its workspace into the scratchpad.
    pub fn move_to_scratchpad(&mut self, window: &Window) {
        let was_focused = self.focused_window().as_ref() == Some(window);
        self.take(window);
        self.space.unmap_elem(window);
        self.scratchpad.push(window.clone());
        self.arrange();
        if was_focused {
            let next = self.tiled().last().cloned();
            self.focus_window(next);
        }
        self.window_event("move", window);
        self.drop_empty();
    }

    /// Swaps two windows' places, on one workspace or across two.
    pub fn swap_windows(&mut self, a: &Window, b: &Window) {
        let place = |state: &Self, w: &Window| {
            state.workspaces.iter().enumerate().find_map(|(i, ws)| ws.windows.iter().position(|x| x == w).map(|j| (i, j)))
        };
        let (Some((ia, ja)), Some((ib, jb))) = (place(self, a), place(self, b)) else { return };
        self.workspaces[ia].windows[ja] = b.clone();
        self.workspaces[ib].windows[jb] = a.clone();
        self.arrange();
        self.window_event("move", a);
    }

    /// Takes a window out of wherever it is (workspace or scratchpad).
    fn take(&mut self, window: &Window) {
        for ws in &mut self.workspaces {
            ws.windows.retain(|w| w != window);
            if ws.last_focus.as_ref() == Some(window) {
                ws.last_focus = None;
            }
        }
        self.scratchpad.retain(|w| w != window);
    }

    /// Forgets a window that closed; the workspace it was on, if shown.
    pub fn forget(&mut self, window: &Window) -> Option<i32> {
        let on = self.workspace_of(window).filter(|n| self.is_visible(*n));
        self.take(window);
        self.space.unmap_elem(window);
        self.window_event("close", window);
        self.drop_empty();
        on
    }

    /// Workspaces neither shown nor holding windows go, as in sway.
    fn drop_empty(&mut self) {
        let gone: Vec<i32> = self
            .workspaces
            .iter()
            .filter(|w| w.num != self.current && !self.is_visible(w.num) && w.windows.is_empty())
            .map(|w| w.num)
            .collect();
        for num in gone {
            self.workspace_event("empty", num, None);
            self.workspaces.retain(|w| w.num != num);
        }
    }
}
