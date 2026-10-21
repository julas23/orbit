//! X programs, through XWayland run by the core (PLANO-CORE: integrated,
//! not `xwayland-satellite`). The core starts `Xwayland` with itself and
//! is its window manager (`X11Wm`): an X window is tiled and focused like
//! any other; its menus and tooltips (override-redirect windows) are drawn
//! where they ask, above the rest, and never take the keyboard. The X
//! clipboard and primary selection are bridged both ways.
//!
//! `DISPLAY` is known as soon as XWayland is spawned, before it is ready
//! (the core holds the socket), so every program the core starts gets it.
//!
//! Ported from anvil `src/shell/x11.rs` and `src/state.rs`
//! (`start_xwayland`) @ 24647fc (Smithay, MIT, Copyright (c) 2017 Victor
//! Berger and Victoria Brekenfeld); resizing and maximizing wait for
//! floating windows.

use std::{os::unix::io::OwnedFd, process::Stdio};

use smithay::{
    desktop::Window,
    utils::{Logical, Rectangle},
    wayland::{
        selection::{
            SelectionTarget,
            data_device::{
                clear_data_device_selection, current_data_device_selection_userdata, request_data_device_client_selection,
                set_data_device_selection,
            },
            primary_selection::{
                clear_primary_selection, current_primary_selection_userdata, request_primary_client_selection, set_primary_selection,
            },
        },
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler,
        xwm::{Reorder, ResizeEdge, WmWindowProperty, XwmId},
    },
};

use crate::{focus::KeyboardFocus, state::Orbit};

impl Orbit {
    /// Spawns XWayland; its window manager starts once it is ready.
    pub fn start_xwayland(&mut self) {
        let spawned = XWayland::spawn(
            &self.display_handle,
            None,
            std::iter::empty::<(String, String)>(),
            std::iter::empty::<String>(),
            true,
            Stdio::null(),
            Stdio::null(),
            |_| (),
        );
        let (xwayland, client) = match spawned {
            Ok(spawned) => spawned,
            Err(err) => {
                tracing::error!("XWayland: {err}; X programs will not run");
                return;
            }
        };
        self.xdisplay = Some(xwayland.display_number());
        tracing::info!("XWayland on DISPLAY=:{}", xwayland.display_number());
        let dh = self.display_handle.clone();
        let inserted = self.loop_handle.insert_source(xwayland, move |event, _, state| match event {
            XWaylandEvent::Ready { x11_socket, display_number } => {
                match X11Wm::start_wm(state.loop_handle.clone(), &dh, x11_socket, client.clone()) {
                    Ok(mut wm) => {
                        // The root window's pointer: the theme's arrow.
                        let image = state.cursor.image();
                        if let Err(err) = wm.set_cursor(
                            &image.pixels_rgba,
                            (image.width as u16, image.height as u16).into(),
                            (image.xhot as u16, image.yhot as u16).into(),
                        ) {
                            tracing::warn!("XWayland cursor: {err}");
                        }
                        state.xwm = Some(wm);
                        tracing::info!("XWayland ready on :{display_number}");
                    }
                    Err(err) => tracing::error!("X window manager: {err}"),
                }
            }
            XWaylandEvent::Error => {
                tracing::error!("XWayland exited while starting");
                state.xdisplay = None;
            }
        });
        if let Err(err) = inserted {
            tracing::error!("XWayland in the event loop: {err}");
            self.xdisplay = None;
        }
    }

    /// The managed window (on a workspace or in the scratchpad) of an X
    /// window.
    pub fn x11_window(&self, surface: &X11Surface) -> Option<Window> {
        self.all_windows().find(|w| w.x11_surface() == Some(surface)).cloned()
    }

    /// The menu or tooltip (override-redirect) of an X window, on screen.
    fn x11_overlay(&self, surface: &X11Surface) -> Option<Window> {
        self.space.elements().find(|w| w.x11_surface() == Some(surface)).cloned()
    }

    /// The window was closed or hidden by its program.
    fn x11_gone(&mut self, surface: &X11Surface) {
        if let Some(window) = self.x11_window(surface) {
            self.remove_window(&window);
        } else if let Some(overlay) = self.x11_overlay(surface) {
            self.space.unmap_elem(&overlay);
            self.request_render();
        }
    }
}

impl XWaylandShellHandler for Orbit {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

impl XwmHandler for Orbit {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().expect("an X window manager")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    /// A window shown: tiled, as a Wayland one.
    fn map_window_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Err(err) = surface.set_mapped(true) {
            tracing::warn!("X window {}: {err}", surface.window_id());
            return;
        }
        if self.x11_window(&surface).is_some() {
            return;
        }
        self.add_window(Window::new_x11_window(surface));
    }

    /// A menu or tooltip: where it asks, above everything.
    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        let location = surface.geometry().loc;
        self.space.map_element(Window::new_x11_window(surface), location, true);
        self.request_render();
    }

    fn unmapped_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        self.x11_gone(&surface);
        if !surface.is_override_redirect() {
            let _ = surface.set_mapped(false);
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        self.x11_gone(&surface);
    }

    /// A tiled window keeps its tile, whatever it asks; one not shown yet
    /// may take the size it wants.
    fn configure_request(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        if let Some(window) = self.x11_window(&surface) {
            let (x, y, w, h) = crate::workspace::info(&window).borrow().rect;
            let _ = surface.configure(Rectangle::new((x, y).into(), (w, h).into()));
            return;
        }
        let mut geo = surface.last_configure();
        if let Some(x) = x {
            geo.loc.x = x;
        }
        if let Some(y) = y {
            geo.loc.y = y;
        }
        if let Some(w) = w {
            geo.size.w = w as i32;
        }
        if let Some(h) = h {
            geo.size.h = h as i32;
        }
        let _ = surface.configure(geo);
    }

    /// A menu or tooltip moved.
    fn configure_notify(&mut self, _xwm: XwmId, surface: X11Surface, geometry: Rectangle<i32, Logical>, _above: Option<u32>) {
        if !surface.is_override_redirect() {
            return;
        }
        if let Some(overlay) = self.x11_overlay(&surface) {
            self.space.map_element(overlay, geometry.loc, true);
            self.request_render();
        }
    }

    fn property_notify(&mut self, _xwm: XwmId, surface: X11Surface, property: WmWindowProperty) {
        if matches!(property, WmWindowProperty::Title | WmWindowProperty::Class)
            && let Some(window) = self.x11_window(&surface)
        {
            self.window_event("title", &window);
        }
    }

    // Resizing waits for floating windows: a tiled window has its size.
    fn resize_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32, _edges: ResizeEdge) {}

    /// Its own title bar dragged: moved as with Mod+drag.
    fn move_request(&mut self, _xwm: XwmId, surface: X11Surface, _button: u32) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        if self.locked() {
            return;
        }
        let Some(window) = self.x11_window(&surface) else { return };
        let Some(start) = pointer.grab_start_data() else { return };
        self.start_move(window, smithay::utils::SERIAL_COUNTER.next_serial(), Some(start));
    }

    /// X programs read the clipboard only while one of them has the
    /// keyboard.
    fn allow_selection_access(&mut self, xwm: XwmId, _selection: SelectionTarget) -> bool {
        matches!(
            self.seat.get_keyboard().and_then(|k| k.current_focus()),
            Some(KeyboardFocus::X11(x)) if x.xwm_id() == Some(xwm)
        )
    }

    fn send_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_type: String, fd: OwnedFd) {
        let sent = match selection {
            SelectionTarget::Clipboard => request_data_device_client_selection(&self.seat, mime_type, fd).map_err(|e| e.to_string()),
            SelectionTarget::Primary => request_primary_client_selection(&self.seat, mime_type, fd).map_err(|e| e.to_string()),
        };
        if let Err(err) = sent {
            tracing::warn!("the Wayland {selection:?} for X: {err}");
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        match selection {
            SelectionTarget::Clipboard => set_data_device_selection(&self.display_handle, &self.seat, mime_types, ()),
            SelectionTarget::Primary => set_primary_selection(&self.display_handle, &self.seat, mime_types, ()),
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(&self.seat).is_some() {
                    clear_data_device_selection(&self.display_handle, &self.seat);
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(&self.seat).is_some() {
                    clear_primary_selection(&self.display_handle, &self.seat);
                }
            }
        }
    }

    fn disconnected(&mut self, _xwm: XwmId) {
        tracing::warn!("the X window manager lost XWayland");
        self.xwm = None;
    }
}
