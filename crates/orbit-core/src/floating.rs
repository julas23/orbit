//! Floating windows: above the tiled ones of their workspace, where they
//! were put, at the size they ask for. Dialogs float by themselves — an X
//! window transient for another or typed dialog, utility, splash or
//! toolbar, a Wayland window with a parent, any window of a fixed size —
//! and open centred over their parent, on its workspace. Any window floats
//! or tiles again with `floating enable|disable|toggle` (Mod+Shift+Space or
//! Mod+Esc); Mod+drag moves a floating one where it is let go (`grab`),
//! Mod+right-drag resizes it (`resize`).
//!
//! Snapped windows (`snap left|right`, Mod+←/→) are laid over the left or
//! right half of what the panel and the dock leave free, above the tiled
//! ones too, until `restore` (Mod+↓) gives them back their tile or their
//! floating place.

use smithay::{
    desktop::Window,
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::Rectangle,
    wayland::{compositor::with_states, shell::xdg::SurfaceCachedState},
    xwayland::xwm::WmWindowType,
};

use crate::{state::Orbit, tile::Rect, workspace::info};

pub fn is_floating(window: &Window) -> bool {
    info(window).borrow().floating
}

/// A half of the screen a window is snapped to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Half {
    Left,
    Right,
}

pub fn snapped(window: &Window) -> Option<Half> {
    info(window).borrow().snap
}

/// Whether a window is laid out above the tiled ones: floating or
/// snapped.
pub fn floats(window: &Window) -> bool {
    let info = info(window).borrow();
    info.floating || info.snap.is_some()
}

/// Notes that `window` floats, not yet placed, at `size` (`None`: its
/// own); `arrange` places it.
pub fn mark_floating(window: &Window, size: Option<(i32, i32)>) {
    let mut info = info(window).borrow_mut();
    info.floating = true;
    info.float_at = None;
    info.float_size = size;
}

/// Whether a window is a dialog, which floats from the start: for a
/// Wayland one, known at its first commit.
pub fn is_dialog(window: &Window) -> bool {
    let fixed = |min: (i32, i32), max: (i32, i32)| min.0 > 0 && min.1 > 0 && min == max;
    if let Some(x) = window.x11_surface() {
        let typed = matches!(
            x.window_type(),
            Some(WmWindowType::Dialog | WmWindowType::Utility | WmWindowType::Splash | WmWindowType::Toolbar)
        );
        let size = |s: Option<smithay::utils::Size<i32, smithay::utils::Logical>>| s.map(|s| (s.w, s.h)).unwrap_or_default();
        return typed || x.is_transient_for().is_some() || x.is_modal() || fixed(size(x.min_size()), size(x.max_size()));
    }
    let Some(toplevel) = window.toplevel() else { return false };
    let (min, max) = with_states(toplevel.wl_surface(), |states| {
        let mut cached = states.cached_state.get::<SurfaceCachedState>();
        let current = cached.current();
        ((current.min_size.w, current.min_size.h), (current.max_size.w, current.max_size.h))
    });
    toplevel.parent().is_some() || fixed(min, max)
}

impl Orbit {
    /// The window `window` belongs to (its parent, or the X window it is
    /// transient for), if the core manages it.
    pub fn parent_of(&self, window: &Window) -> Option<Window> {
        if let Some(x) = window.x11_surface() {
            let id = x.is_transient_for()?;
            return self.all_windows().find(|w| w.x11_surface().is_some_and(|p| p.window_id() == id)).cloned();
        }
        let parent = window.toplevel()?.parent()?;
        self.window_for(&parent)
    }

    /// A Wayland window that turned out a dialog (at its first commit, or
    /// when it was given a parent) floats, on its parent's workspace.
    pub fn float_if_dialog(&mut self, window: &Window) {
        if is_floating(window) || !is_dialog(window) {
            return;
        }
        if let Some(num) = self.parent_of(window).and_then(|p| self.workspace_of(&p))
            && Some(num) != self.workspace_of(window)
        {
            self.move_to_workspace(window, num);
        }
        mark_floating(window, None);
        self.arrange();
        self.window_event("floating", window);
    }

    /// Floats `window` (two thirds of its monitor's area, centred) or
    /// tiles it again.
    pub fn set_floating(&mut self, window: &Window, on: bool) {
        if is_floating(window) == on {
            return;
        }
        if on {
            let output = self.workspace_of(window).and_then(|n| self.ws_output(n)).and_then(|o| self.output_named(&o));
            let area = output.and_then(|o| self.usable_area(&o));
            mark_floating(window, area.map(|a| (a.2 * 2 / 3, a.3 * 2 / 3)));
        } else {
            info(window).borrow_mut().floating = false;
        }
        self.arrange();
        self.window_event("floating", window);
    }

    /// Snaps `window` to a half of its monitor (`None`: back to its tile
    /// or its floating place); a maximized one leaves that first.
    pub fn snap(&mut self, window: &Window, half: Option<Half>) {
        if crate::fullscreen::is_maximized(window) {
            self.set_maximized(window, false);
        }
        info(window).borrow_mut().snap = half;
        if half.is_some() {
            self.space.raise_element(window, false);
        }
        self.arrange();
        self.window_event("move", window);
    }

    /// `restore` (Mod+↓): a maximized window leaves that, a snapped one
    /// its half.
    pub fn restore(&mut self, window: &Window) {
        if crate::fullscreen::is_maximized(window) {
            self.set_maximized(window, false);
        } else if snapped(window).is_some() {
            self.snap(window, None);
        }
    }

    /// Lays out a floating or snapped window on a monitor whose usable
    /// area is `area` and whole rectangle `full`. Snapped: over its half.
    /// Floating: where it was put, or centred over its parent (over `area`
    /// without one) until it has a size; back inside `full` if it was left
    /// on another monitor.
    pub fn place_floating(&mut self, window: &Window, area: Rect, full: Rect) {
        if let Some(half) = snapped(window) {
            let halves = crate::tile::columns(2, area, self.config.gaps, self.config.border);
            let rect = halves[usize::from(half == Half::Right)];
            info(window).borrow_mut().rect = rect;
            self.configure_floating(window, rect, Some((rect.2, rect.3)));
            return;
        }
        let (at, size) = {
            let info = info(window).borrow();
            (info.float_at, info.float_size)
        };
        // A Wayland window without a size of its own is the one it drew;
        // an X window, two thirds of the area (it always has one).
        let (w, h) = size.unwrap_or_else(|| match window.x11_surface() {
            Some(_) => (area.2 * 2 / 3, area.3 * 2 / 3),
            None => {
                let geo = window.geometry();
                (geo.size.w, geo.size.h)
            }
        });
        let inside = |(x, y): (i32, i32)| {
            let (cx, cy) = (x + w / 2, y + h / 2);
            cx >= full.0 && cx < full.0 + full.2 && cy >= full.1 && cy < full.1 + full.3
        };
        let (x, y) = match at.filter(|at| inside(*at)) {
            Some(at) => at,
            None => {
                let over = self
                    .parent_of(window)
                    .map(|p| info(&p).borrow().rect)
                    .filter(|r| inside((r.0 + r.2 / 2 - w / 2, r.1 + r.3 / 2 - h / 2)))
                    .unwrap_or(area);
                let x = (over.0 + (over.2 - w) / 2).clamp(area.0, (area.0 + area.2 - w).max(area.0));
                let y = (over.1 + (over.3 - h) / 2).clamp(area.1, (area.1 + area.3 - h).max(area.1));
                (x, y)
            }
        };
        {
            let mut info = info(window).borrow_mut();
            info.rect = (x, y, w, h);
            // Placed once it has a size: from then on it stays.
            if w > 0 && h > 0 {
                info.float_at = Some((x, y));
            }
        }
        self.configure_floating(window, (x, y, w, h), size);
    }

    /// Tells the program its place and size (`size`; `None`: a Wayland
    /// window keeps its own) and puts it there, above the tiled ones.
    fn configure_floating(&mut self, window: &Window, (x, y, w, h): Rect, size: Option<(i32, i32)>) {
        if let Some(surface) = window.x11_surface() {
            if let Err(err) = surface.configure(Rectangle::new((x, y).into(), (w.max(1), h.max(1)).into())) {
                tracing::warn!("X window {}: {err}", surface.window_id());
            }
        } else if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|state| {
                state.size = size.map(Into::into);
                for edge in [
                    xdg_toplevel::State::TiledLeft,
                    xdg_toplevel::State::TiledRight,
                    xdg_toplevel::State::TiledTop,
                    xdg_toplevel::State::TiledBottom,
                ] {
                    state.states.unset(edge);
                }
            });
            toplevel.send_pending_configure();
        }
        if self.dragging.as_ref() != Some(window) {
            self.space.map_element(window.clone(), (x, y), false);
        }
    }

    /// A floating window's program asked for another size (X).
    pub fn resize_floating(&mut self, window: &Window, w: Option<u32>, h: Option<u32>) {
        {
            let mut info = info(window).borrow_mut();
            let (old_w, old_h) = info.float_size.unwrap_or((info.rect.2, info.rect.3));
            info.float_size = Some((w.map_or(old_w, |w| w as i32), h.map_or(old_h, |h| h as i32)));
        }
        self.arrange();
    }

    /// A floating window still without its place drew a new size: it is
    /// centred again.
    pub fn floating_committed(&mut self, window: &Window) {
        let unplaced = {
            let info = info(window).borrow();
            info.floating && info.float_at.is_none()
        };
        if unplaced {
            self.arrange();
        }
    }
}

