//! Fullscreen windows: one per workspace covers its whole monitor, panel
//! and dock included, with no border and square corners. Programs ask for
//! it (a video, F11) through `xdg_toplevel` or `_NET_WM_STATE`; the user
//! with Mod+F or the `fullscreen` command. The keyboard going to another
//! window of that workspace (a new one, Mod+→) ends it, as the window
//! would be hidden behind it otherwise.
//!
//! Above a fullscreen window only the overlay layer (notifications,
//! dialogs), the top layer's surfaces that take the keyboard (the panel's
//! menus) and X menus are drawn.
//!
//! Maximized windows are the same, short of it: the title bar's button
//! lays one over what the panel and the dock leave free, border and
//! corners kept.

use smithay::{
    desktop::{LayerSurface, Window, layer_map_for_output},
    output::Output,
    wayland::shell::wlr_layer::{KeyboardInteractivity, Layer as WlrLayer},
};

use crate::{state::Orbit, workspace::info};

pub fn is_fullscreen(window: &Window) -> bool {
    info(window).borrow().fullscreen
}

pub fn is_maximized(window: &Window) -> bool {
    info(window).borrow().maximized
}

/// A top-layer surface that stays above a fullscreen window: one that
/// takes the keyboard (a menu, the launcher), never the bar itself.
pub fn above_fullscreen(layer: &LayerSurface) -> bool {
    layer.cached_state().keyboard_interactivity != KeyboardInteractivity::None
}

/// The top-layer surfaces of `output` that stay above a fullscreen
/// window, topmost first.
pub fn top_above(output: &Output) -> Vec<LayerSurface> {
    layer_map_for_output(output).layers_on(WlrLayer::Top).rev().filter(|l| above_fullscreen(l)).cloned().collect()
}

impl Orbit {
    /// The fullscreen window of the workspace `output` shows.
    pub fn fullscreen_on(&self, output: &Output) -> Option<Window> {
        let num = self.visible.get(&output.name())?;
        self.windows_of(*num).into_iter().find(is_fullscreen)
    }

    /// Puts `window` in fullscreen or out of it; any other fullscreen
    /// window of its workspace leaves it.
    pub fn set_fullscreen(&mut self, window: &Window, on: bool) {
        if on && let Some(num) = self.workspace_of(window) {
            for other in self.windows_of(num).iter().filter(|w| *w != window && is_fullscreen(w)) {
                self.mark_fullscreen(other, false);
            }
        }
        let changed = is_fullscreen(window) != on;
        self.mark_fullscreen(window, on);
        if on {
            self.space.raise_element(window, true);
        }
        self.arrange();
        if changed {
            self.window_event("fullscreen_mode", window);
        }
    }

    /// Notes it and tells the program; `arrange` gives it its size.
    fn mark_fullscreen(&mut self, window: &Window, on: bool) {
        info(window).borrow_mut().fullscreen = on;
        if let Some(x) = window.x11_surface() {
            if let Err(err) = x.set_fullscreen(on) {
                tracing::warn!("X window {}: {err}", x.window_id());
            }
        } else if let Some(toplevel) = window.toplevel() {
            use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State;
            toplevel.with_pending_state(|state| {
                if on {
                    state.states.set(State::Fullscreen);
                } else {
                    state.states.unset(State::Fullscreen);
                }
            });
        }
    }

    /// Maximizes `window` or gives it back its tile; any other maximized
    /// window of its workspace gives back its own.
    pub fn set_maximized(&mut self, window: &Window, on: bool) {
        if on && let Some(num) = self.workspace_of(window) {
            for other in self.windows_of(num).iter().filter(|w| *w != window && is_maximized(w)) {
                self.mark_maximized(other, false);
            }
        }
        self.mark_maximized(window, on);
        if on {
            self.space.raise_element(window, true);
        }
        self.arrange();
    }

    /// Notes it and tells the program; `arrange` gives it its size.
    fn mark_maximized(&mut self, window: &Window, on: bool) {
        info(window).borrow_mut().maximized = on;
        if let Some(x) = window.x11_surface() {
            if let Err(err) = x.set_maximized(on) {
                tracing::warn!("X window {}: {err}", x.window_id());
            }
        } else if let Some(toplevel) = window.toplevel() {
            use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State;
            toplevel.with_pending_state(|state| {
                if on {
                    state.states.set(State::Maximized);
                } else {
                    state.states.unset(State::Maximized);
                }
            });
        }
    }

    /// The keyboard goes to `window`: a fullscreen or maximized window of
    /// its workspace that is not it leaves that state.
    pub fn yield_fullscreen(&mut self, window: &Window) {
        let Some(num) = self.workspace_of(window) else { return };
        if let Some(full) = self.windows_of(num).into_iter().find(|w| w != window && is_fullscreen(w)) {
            self.set_fullscreen(&full, false);
        }
        if let Some(max) = self.windows_of(num).into_iter().find(|w| w != window && is_maximized(w)) {
            self.set_maximized(&max, false);
        }
    }
}
