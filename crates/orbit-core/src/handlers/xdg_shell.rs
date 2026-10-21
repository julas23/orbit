//! Windows (`xdg_toplevel`) and their menus (`xdg_popup`): a new window is
//! tiled and takes the keyboard; a closed one leaves its place to the
//! others; menus are kept inside the screen and hold the pointer and the
//! keyboard while open.
//!
//! Ported from Smithay's smallvil `src/handlers/xdg_shell.rs` @ 24647fc (MIT,
//! Copyright (c) 2017 Victor Berger and Victoria Brekenfeld); the popup grab
//! and keeping layer-surface menus on screen from anvil `src/shell/xdg.rs`
//! @ 24647fc (same licence). Moving by the title bar is `grab`'s; resizing
//! by the pointer comes with floating windows.

use smithay::{
    desktop::{
        PopupKeyboardGrab, PopupKind, PopupPointerGrab, PopupUngrabStrategy, Window, find_popup_root_surface,
        get_popup_toplevel_coords, layer_map_for_output,
    },
    input::{Seat, pointer::Focus},
    reexports::wayland_server::{Resource, protocol::wl_seat},
    utils::{Logical, Rectangle, Serial},
    wayland::shell::xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState},
};

use crate::{state::Orbit, workspace::info};

impl XdgShellHandler for Orbit {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let window = Window::new_wayland_window(surface);
        info(&window).borrow_mut().id = self.next_window_id;
        self.next_window_id += 1;
        // On the monitor under the pointer, in the workspace it shows.
        let num = self.pointer_output().and_then(|o| self.visible.get(&o.name()).copied()).unwrap_or(self.current);
        let i = self.ensure_ws(num);
        self.workspaces[i].windows.push(window.clone());
        self.space.map_element(window.clone(), (0, 0), false);
        self.arrange();
        self.window_event("new", &window);
        self.focus_window(Some(window));
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let Some(window) = self.window_for(surface.wl_surface()) else { return };
        let had_focus = self.focused_window().as_ref() == Some(&window) || self.focused_window().is_none();
        let index = self.workspace_of(&window).and_then(|n| self.windows_of(n).iter().position(|w| w == &window)).unwrap_or(0);
        if self.dragging.as_ref() == Some(&window) {
            self.dragging = None;
        }
        let Some(num) = self.forget(&window) else { return };
        self.arrange();
        if had_focus {
            // The neighbour on the left, as i3 does; or the first.
            let next = self.windows_of(num).get(index.saturating_sub(1)).cloned();
            self.focus_window(next);
        }
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for(surface.wl_surface()) {
            self.window_event("title", &window);
        }
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        self.title_changed(surface);
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        if let Err(err) = self.popups.track_popup(PopupKind::Xdg(surface)) {
            tracing::warn!("popup: {err}");
        }
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    /// A window's own title bar dragged: moved as with Mod+drag, while
    /// the button that began it is held.
    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let Some(pointer) = Seat::<Self>::from_resource(&seat).and_then(|s| s.get_pointer()) else { return };
        if self.locked() || !pointer.has_grab(serial) {
            return;
        }
        let Some(start) = pointer.grab_start_data() else { return };
        // The press began on this client's window.
        if !start.focus.as_ref().is_some_and(|(s, _)| s.id().same_client_as(&surface.wl_surface().id())) {
            return;
        }
        let Some(window) = self.window_for(surface.wl_surface()) else { return };
        self.start_move(window, serial, Some(start));
    }

    /// A menu holds the pointer and the keyboard until it closes.
    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        if self.locked() {
            return;
        }
        let Some(seat) = Seat::<Self>::from_resource(&seat) else { return };
        let kind = PopupKind::Xdg(surface);
        let Ok(root) = find_popup_root_surface(&kind) else { return };
        if self.window_for(&root).is_none() && self.layer_for(&root).is_none() {
            return;
        }
        let Ok(mut grab) = self.popups.grab_popup::<Self>(root, kind, &seat, serial) else { return };
        if let Some(keyboard) = seat.get_keyboard() {
            if keyboard.is_grabbed() && !(keyboard.has_grab(serial) || keyboard.has_grab(grab.previous_serial().unwrap_or(serial))) {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        }
        if let Some(pointer) = seat.get_pointer() {
            if pointer.is_grabbed() && !(pointer.has_grab(serial) || pointer.has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial()))) {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
        }
    }
}

impl Orbit {
    /// Keeps a menu inside the screen: the positioner moves or flips it
    /// within the output, as seen from its parent (a window or a layer
    /// surface such as the panel).
    pub fn unconstrain_popup(&self, popup: &PopupSurface) {
        let kind = PopupKind::Xdg(popup.clone());
        let Ok(root) = find_popup_root_surface(&kind) else { return };
        // The monitor the parent is on: a window's, or a layer surface's.
        let parent: Option<(Rectangle<i32, Logical>, smithay::output::Output)> = match self.window_for(&root) {
            Some(window) => self.space.element_geometry(&window).and_then(|geo| Some((geo, self.output_at(crate::output::center(geo).to_f64())?))),
            None => self.layer_for(&root).and_then(|layer| {
                let output = self.space.outputs().find(|o| layer_map_for_output(o).layers().any(|l| l == &layer))?.clone();
                let mut geo = layer_map_for_output(&output).layer_geometry(&layer)?;
                geo.loc += self.space.output_geometry(&output)?.loc;
                Some((geo, output))
            }),
        };
        let Some((parent, output)) = parent else { return };
        let Some(out) = self.space.output_geometry(&output) else { return };
        let mut target = out;
        target.loc -= get_popup_toplevel_coords(&kind);
        target.loc -= parent.loc;
        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}
