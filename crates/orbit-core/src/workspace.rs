//! Numbered workspaces, as sway's: each holds its windows in their order on
//! screen; one is shown at a time (multi-monitor is Phase 2). A workspace
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
    pub windows: Vec<Window>,
    /// The window to give the keyboard back to when the workspace is shown.
    pub last_focus: Option<Window>,
}

impl Workspace {
    fn new(num: i32) -> Self {
        Self { num, windows: Vec::new(), last_focus: None }
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
    /// The workspace shown; made when missing.
    pub fn current_ws(&mut self) -> &mut Workspace {
        let num = self.current;
        let i = self.ensure_ws(num);
        &mut self.workspaces[i]
    }

    /// The windows of the workspace shown, in order.
    pub fn tiled(&self) -> Vec<Window> {
        self.workspaces.iter().find(|w| w.num == self.current).map(|w| w.windows.clone()).unwrap_or_default()
    }

    /// Every window, on any workspace or in the scratchpad.
    pub fn all_windows(&self) -> impl Iterator<Item = &Window> {
        self.workspaces.iter().flat_map(|w| &w.windows).chain(&self.scratchpad)
    }

    /// The index of workspace `num`, made (and announced) when missing;
    /// the list stays in number order.
    pub fn ensure_ws(&mut self, num: i32) -> usize {
        if let Some(i) = self.workspaces.iter().position(|w| w.num == num) {
            return i;
        }
        let i = self.workspaces.iter().position(|w| w.num > num).unwrap_or(self.workspaces.len());
        self.workspaces.insert(i, Workspace::new(num));
        self.workspace_event("init", num, None);
        i
    }

    /// The workspace a window is on; `None` in the scratchpad.
    pub fn workspace_of(&self, window: &Window) -> Option<i32> {
        self.workspaces.iter().find(|w| w.windows.contains(window)).map(|w| w.num)
    }

    /// Shows workspace `num`: the windows of the one shown leave the screen,
    /// its own come back, and the keyboard goes to the one that had it.
    pub fn switch_workspace(&mut self, num: i32) {
        if num == self.current {
            return;
        }
        let old = self.current;
        self.previous_ws = Some(old);
        for window in self.tiled() {
            self.space.unmap_elem(&window);
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

    /// Moves `window` to workspace `num` (from another, or from the
    /// scratchpad), without showing it.
    pub fn move_to_workspace(&mut self, window: &Window, num: i32) {
        let was_shown = self.workspace_of(window) == Some(self.current);
        self.take(window);
        let i = self.ensure_ws(num);
        self.workspaces[i].windows.push(window.clone());
        if num == self.current {
            self.space.map_element(window.clone(), (0, 0), false);
        } else if was_shown {
            self.space.unmap_elem(window);
            if self.focused_window().as_ref() == Some(window) {
                let next = self.tiled().last().cloned();
                self.focus_window(next);
            }
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

    /// Forgets a window that closed; whether it was on the workspace shown.
    pub fn forget(&mut self, window: &Window) -> bool {
        let shown = self.workspace_of(window) == Some(self.current);
        self.take(window);
        self.space.unmap_elem(window);
        self.window_event("close", window);
        self.drop_empty();
        shown
    }

    /// Workspaces neither shown nor holding windows go, as in sway.
    fn drop_empty(&mut self) {
        let current = self.current;
        let gone: Vec<i32> = self.workspaces.iter().filter(|w| w.num != current && w.windows.is_empty()).map(|w| w.num).collect();
        for num in gone {
            self.workspace_event("empty", num, None);
            self.workspaces.retain(|w| w.num != num);
        }
    }
}
