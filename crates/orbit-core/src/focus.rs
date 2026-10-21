//! What has the keyboard: a Wayland surface (a window, the panel, a menu)
//! or an X window. An X window is told it has the keyboard through the X
//! server (input focus, `WM_TAKE_FOCUS`) besides its surface, which it
//! may not have yet when it is focused — smithay's `X11Surface` sends the
//! surface's part once it comes.
//!
//! The shape follows anvil `src/focus.rs` @ 24647fc (Smithay, MIT, Copyright
//! (c) 2017 Victor Berger and Victoria Brekenfeld).

use std::borrow::Cow;

use smithay::{
    backend::input::KeyState,
    desktop::PopupKind,
    input::{
        Seat,
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
    },
    reexports::wayland_server::{backend::ObjectId, protocol::wl_surface::WlSurface},
    utils::{IsAlive, Serial},
    wayland::seat::WaylandFocus,
    xwayland::X11Surface,
};

use crate::state::Orbit;

// Made when the keyboard moves, not per event: the size does not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum KeyboardFocus {
    Surface(WlSurface),
    X11(X11Surface),
}

impl KeyboardFocus {
    /// Whether this is `surface`.
    pub fn is(&self, surface: &WlSurface) -> bool {
        self.wl_surface().as_deref() == Some(surface)
    }
}

impl IsAlive for KeyboardFocus {
    fn alive(&self) -> bool {
        match self {
            Self::Surface(s) => s.alive(),
            Self::X11(x) => x.alive(),
        }
    }
}

impl WaylandFocus for KeyboardFocus {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            Self::Surface(s) => Some(Cow::Borrowed(s)),
            Self::X11(x) => x.wl_surface().map(Cow::Owned),
        }
    }

    fn same_client_as(&self, object_id: &ObjectId) -> bool {
        match self {
            Self::Surface(s) => s.same_client_as(object_id),
            Self::X11(x) => x.same_client_as(object_id),
        }
    }
}

impl From<WlSurface> for KeyboardFocus {
    fn from(surface: WlSurface) -> Self {
        Self::Surface(surface)
    }
}

impl From<PopupKind> for KeyboardFocus {
    fn from(popup: PopupKind) -> Self {
        Self::Surface(popup.wl_surface().clone())
    }
}

/// For menus: they and their parents are always Wayland surfaces, so an X
/// window never comes here (if one did, it is its surface, or none).
impl From<KeyboardFocus> for WlSurface {
    fn from(focus: KeyboardFocus) -> Self {
        match focus {
            KeyboardFocus::Surface(s) => s,
            KeyboardFocus::X11(x) => x.wl_surface().unwrap_or_else(|| unreachable!("menus have Wayland parents")),
        }
    }
}

impl KeyboardTarget<Orbit> for KeyboardFocus {
    fn enter(&self, seat: &Seat<Orbit>, data: &mut Orbit, keys: Vec<KeysymHandle<'_>>, serial: Serial) {
        match self {
            Self::Surface(s) => KeyboardTarget::enter(s, seat, data, keys, serial),
            Self::X11(x) => KeyboardTarget::enter(x, seat, data, keys, serial),
        }
    }

    fn leave(&self, seat: &Seat<Orbit>, data: &mut Orbit, serial: Serial) {
        match self {
            Self::Surface(s) => KeyboardTarget::leave(s, seat, data, serial),
            Self::X11(x) => KeyboardTarget::leave(x, seat, data, serial),
        }
    }

    fn key(&self, seat: &Seat<Orbit>, data: &mut Orbit, key: KeysymHandle<'_>, state: KeyState, serial: Serial, time: smithay::backend::input::InputTime) {
        match self {
            Self::Surface(s) => KeyboardTarget::key(s, seat, data, key, state, serial, time),
            Self::X11(x) => KeyboardTarget::key(x, seat, data, key, state, serial, time),
        }
    }

    fn modifiers(&self, seat: &Seat<Orbit>, data: &mut Orbit, modifiers: ModifiersState, serial: Serial) {
        match self {
            Self::Surface(s) => KeyboardTarget::modifiers(s, seat, data, modifiers, serial),
            Self::X11(x) => KeyboardTarget::modifiers(x, seat, data, modifiers, serial),
        }
    }
}
