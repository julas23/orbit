//! The handlers of the Wayland protocols the core speaks. `delegate_dispatch2`
//! routes every protocol to the handler traits implemented here and in the
//! submodules.
//!
//! Ported from Smithay's smallvil `src/handlers/mod.rs` @ 24647fc (MIT,
//! Copyright (c) 2017 Victor Berger and Victoria Brekenfeld); the selections
//! and decorations follow anvil @ 24647fc (same licence).

mod compositor;
mod layer_shell;
mod xdg_shell;

use smithay::{
    input::{
        Seat, SeatHandler, SeatState,
        dnd::{DnDGrab, DndGrabHandler, GrabType, Source},
        pointer::{CursorImageStatus, Focus},
    },
    reexports::{
        wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode,
        wayland_server::{Resource, protocol::wl_surface::WlSurface},
    },
    wayland::seat::WaylandFocus,
    utils::Serial,
    wayland::{
        output::OutputHandler,
        selection::{
            SelectionHandler, SelectionSource, SelectionTarget,
            data_device::{DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler, set_data_device_focus},
            primary_selection::{PrimarySelectionHandler, PrimarySelectionState, set_primary_focus},
            wlr_data_control::{DataControlHandler, DataControlState},
        },
        pointer_constraints::PointerConstraintsHandler,
        shell::xdg::{ToplevelSurface, decoration::XdgDecorationHandler},
    },
};

use smithay::{
    backend::{allocator::dmabuf::Dmabuf, renderer::ImportDma},
    wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
};

use crate::{
    focus::KeyboardFocus,
    state::{Backend, Orbit},
};

impl SeatHandler for Orbit {
    type KeyboardFocus = KeyboardFocus;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Orbit> {
        &mut self.seat_state
    }

    /// The image a client gives the pointer (drawn by the core on the
    /// screen; nested, the host draws its own).
    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_status = image;
        if matches!(self.backend, crate::state::Backend::Drm(_)) {
            self.request_render();
        }
    }

    /// The clipboard follows the keyboard: the focused client may read it.
    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&KeyboardFocus>) {
        let dh = &self.display_handle;
        let client = focused.and_then(|f| f.wl_surface()).and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client.clone());
        set_primary_focus(dh, seat, client);
    }
}

/// A Wayland client's selection goes to the X programs too; the X
/// programs' own reach Wayland through `xwayland`.
impl SelectionHandler for Orbit {
    type SelectionUserData = ();

    fn new_selection(&mut self, ty: SelectionTarget, source: Option<SelectionSource>, _seat: Seat<Self>) {
        if let Some(xwm) = self.xwm.as_mut()
            && let Err(err) = xwm.new_selection(ty, source.map(|s| s.mime_types()))
        {
            tracing::warn!("the {ty:?} for X: {err}");
        }
    }

    fn send_selection(&mut self, ty: SelectionTarget, mime_type: String, fd: std::os::unix::io::OwnedFd, _seat: Seat<Self>, _user_data: &()) {
        if let Some(xwm) = self.xwm.as_mut()
            && let Err(err) = xwm.send_selection(ty, mime_type, fd)
        {
            tracing::warn!("the X {ty:?} for Wayland: {err}");
        }
    }
}

impl DataDeviceHandler for Orbit {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl PrimarySelectionHandler for Orbit {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.primary_selection_state
    }
}

impl DataControlHandler for Orbit {
    fn data_control_state(&mut self) -> &mut DataControlState {
        &mut self.data_control_state
    }
}

impl DndGrabHandler for Orbit {}

impl WaylandDndGrabHandler for Orbit {
    fn dnd_requested<S: Source>(&mut self, source: S, _icon: Option<WlSurface>, seat: Seat<Self>, serial: Serial, type_: GrabType) {
        match type_ {
            GrabType::Pointer => {
                let pointer = seat.get_pointer().unwrap();
                let Some(start_data) = pointer.grab_start_data() else {
                    source.cancel();
                    return;
                };
                let grab = DnDGrab::new_pointer(&self.display_handle, start_data, source, seat);
                pointer.set_grab(self, grab, serial, Focus::Keep);
            }
            // No touch yet.
            GrabType::Touch => source.cancel(),
        }
    }
}

impl OutputHandler for Orbit {}

/// Client buffers in GPU memory, imported by the backend's renderer.
impl DmabufHandler for Orbit {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        match &mut self.backend {
            Backend::Winit(w) => &mut w.dmabuf_state,
            Backend::Drm(d) => &mut d.dmabuf_state,
            Backend::None => unreachable!("dmabuf requests before a backend"),
        }
    }

    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
        let imported = match &mut self.backend {
            Backend::Winit(w) => w.backend.renderer().import_dmabuf(&dmabuf, None).is_ok(),
            Backend::Drm(d) => d.renderer.import_dmabuf(&dmabuf, None).is_ok(),
            Backend::None => false,
        };
        if imported {
            let _ = notifier.successful::<Orbit>();
        } else {
            notifier.failed();
        }
    }
}

// Pointer locks (games) come later; the seat needs the handler.
impl PointerConstraintsHandler for Orbit {}

/// The core draws the borders: windows are asked not to draw their own
/// title bars (GTK and Qt then go without; others ignore it).
impl XdgDecorationHandler for Orbit {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| state.decoration_mode = Some(DecorationMode::ServerSide));
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: DecorationMode) {
        toplevel.with_pending_state(|state| state.decoration_mode = Some(DecorationMode::ServerSide));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.request_mode(toplevel, DecorationMode::ServerSide);
    }
}

smithay::delegate_dispatch2!(Orbit);
