//! Switching windows from the keyboard, the last used first. The core
//! keeps every window in the order it last had the keyboard; a switch
//! walks that order while its modifier is held and settles when it is let
//! go — a single tap goes back to the window before.
//!
//! - `switch workspace` (Alt+Tab): the windows of the focused workspace,
//!   each one focused as it is reached.
//! - `switch all` (Alt+Shift+Tab): the windows of every workspace, which
//!   is shown as its window is reached.
//! - `switch cards` (Mod+Tab, `switch cards back` Mod+Shift+Tab): the
//!   windows of every workspace as a stack of cards over the blurred
//!   screen (`cards`); the one in front gets the keyboard at the end.
//!
//! Escape puts things back as they were; minimised windows are left out.

use std::time::{Duration, Instant};

use smithay::desktop::Window;

use crate::state::Orbit;

/// How long a card takes to go to the back of the stack, or come forward.
pub const SHIFT: Duration = Duration::from_millis(220);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Workspace,
    All,
    Cards,
}

impl Mode {
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "workspace" => Some(Self::Workspace),
            "all" => Some(Self::All),
            "cards" => Some(Self::Cards),
            _ => None,
        }
    }
}

pub struct Switcher {
    pub mode: Mode,
    /// The windows, the last used first; `list[0]` had the keyboard.
    pub list: Vec<Window>,
    /// The one reached.
    pub index: usize,
    /// The last step, for the cards' motion: when, and which way.
    pub stepped: Option<(Instant, bool)>,
    /// The cards' title bars and the blurred screen behind (`cards`).
    pub cache: crate::cards::Cache,
}

impl Switcher {
    /// How far the cards' last step has gone (0…1, eased).
    pub fn progress(&self) -> f32 {
        match self.stepped {
            Some((at, _)) => crate::eyes::ease(at.elapsed().as_secs_f32() / SHIFT.as_secs_f32()),
            None => 1.0,
        }
    }

    pub fn moving(&self) -> bool {
        self.mode == Mode::Cards && self.stepped.is_some_and(|(at, _)| at.elapsed() < SHIFT)
    }
}

impl Orbit {
    /// `window` had the keyboard: first in the order of use. Not while a
    /// switch walks the windows, which only settles at its end.
    pub fn used(&mut self, window: &Window) {
        if self.switcher.as_ref().is_some_and(|s| s.mode != Mode::Cards) {
            return;
        }
        self.mru.retain(|w| w != window);
        self.mru.insert(0, window.clone());
    }

    /// The windows a switch walks, the last used first.
    fn switch_list(&self, mode: Mode) -> Vec<Window> {
        let pool: Vec<Window> = match mode {
            Mode::Workspace => self.tiled(),
            Mode::All | Mode::Cards => self.workspaces.iter().flat_map(|w| w.windows.clone()).collect(),
        };
        let mut list: Vec<Window> = self.mru.iter().filter(|w| pool.contains(w)).cloned().collect();
        let rest: Vec<Window> = pool.into_iter().filter(|w| !list.contains(w)).collect();
        list.extend(rest);
        // The window with the keyboard comes first even if it was never
        // noted (focused before the core kept the order).
        if let Some(focused) = self.focused_window()
            && let Some(i) = list.iter().position(|w| *w == focused)
        {
            let w = list.remove(i);
            list.insert(0, w);
        }
        list
    }

    /// `switch MODE [back]`: starts a switch, or takes it one step further.
    pub fn switch(&mut self, mode: Mode, back: bool) {
        if let Some(switcher) = self.switcher.as_mut().filter(|s| s.mode == mode) {
            let n = switcher.list.len();
            switcher.index = if back { (switcher.index + n - 1) % n } else { (switcher.index + 1) % n };
            switcher.stepped = Some((Instant::now(), back));
            let window = switcher.list[switcher.index].clone();
            self.reach(mode, &window);
            return;
        }
        self.switch_cancel();
        let list = self.switch_list(mode);
        let needed = if mode == Mode::Cards { 1 } else { 2 };
        if list.len() < needed {
            return;
        }
        let index = match (list.len(), back) {
            (1, _) => 0,
            (n, true) => n - 1,
            _ => 1,
        };
        let window = list[index].clone();
        self.switcher = Some(Switcher { mode, list, index, stepped: None, cache: Default::default() });
        self.reach(mode, &window);
    }

    /// The window a switch reached: focused, or shown in front.
    fn reach(&mut self, mode: Mode, window: &Window) {
        match mode {
            Mode::Cards => self.request_render(),
            _ => self.focus_anywhere(window),
        }
    }

    /// The modifier was let go: the window reached keeps the keyboard.
    pub fn switch_finish(&mut self) {
        let Some(switcher) = self.switcher.take() else { return };
        let window = switcher.list[switcher.index].clone();
        if self.all_windows().any(|w| *w == window) {
            self.focus_anywhere(&window);
            self.used(&window);
        }
        self.request_render();
    }

    /// Escape (or the lock): back to the window the switch started from.
    pub fn switch_cancel(&mut self) {
        let Some(switcher) = self.switcher.take() else { return };
        if switcher.mode != Mode::Cards
            && let Some(first) = switcher.list.first().filter(|w| self.all_windows().any(|x| x == *w)).cloned()
        {
            self.focus_anywhere(&first);
        }
        self.request_render();
    }

    /// A window closed while a switch walks them: it leaves the walk.
    pub fn switch_forget(&mut self, window: &Window) {
        self.mru.retain(|w| w != window);
        let Some(switcher) = self.switcher.as_mut() else { return };
        let Some(i) = switcher.list.iter().position(|w| w == window) else { return };
        switcher.list.remove(i);
        if switcher.list.is_empty() {
            self.switcher = None;
            return;
        }
        if switcher.index > i || switcher.index == switcher.list.len() {
            switcher.index = switcher.index.saturating_sub(1);
        }
    }

    /// Whether frames must keep coming: the cards moving.
    pub fn switch_moving(&self) -> bool {
        self.switcher.as_ref().is_some_and(Switcher::moving)
    }
}
