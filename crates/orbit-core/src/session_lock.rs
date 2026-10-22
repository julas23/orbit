//! `ext-session-lock-v1` (PLANO-CORE §7.3 step 4): lockers of others
//! (swaylock, hyprlock…) on the core's own lock.
//!
//! A locker that asks while the session is shown gets the same lock as
//! Super+L — the eyes close, the clients are cut off — with its surfaces
//! in the greet's place, one per monitor, and the keyboard on the one with
//! the pointer. It is told `locked` once the black is on every screen, and
//! its `unlock_and_destroy` opens the eyes. While it holds the lock the
//! greet cannot open it, and while the core is already locked (by Orbit or
//! another locker) a new one is refused (`finished`).
//!
//! A locker that dies leaves the session locked, as the protocol asks: the
//! greet takes over and the password opens it.
//!
//! What is shown while locked — the greet or the locker — is the lock's
//! face; the `face_*` functions pick it for drawing and input.

use std::time::Duration;

use smithay::{
    backend::renderer::{
        element::{Kind, surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree}},
        gles::GlesRenderer,
    },
    desktop::{WindowSurfaceType, utils::{send_frames_surface_tree, under_from_surface_tree}},
    output::Output,
    reexports::wayland_server::{
        Resource,
        protocol::{wl_output::WlOutput, wl_surface::WlSurface},
    },
    utils::{Logical, Point, SERIAL_COUNTER, Scale},
    wayland::session_lock::{LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker},
};
use smithay::reexports::wayland_protocols::ext::session_lock::v1::server::ext_session_lock_v1::ExtSessionLockV1;

use crate::{
    focus::KeyboardFocus,
    render::OutputElement,
    state::{Backend, Orbit},
};

/// The client holding the lock.
pub struct Locker {
    lock: ExtSessionLockV1,
    /// Its confirmation, until the black is on every screen.
    pending: Option<SessionLocker>,
    /// Its surfaces, and the monitors they cover.
    surfaces: Vec<(String, LockSurface)>,
}

pub fn init(dh: &smithay::reexports::wayland_server::DisplayHandle) -> SessionLockManagerState {
    SessionLockManagerState::new::<Orbit, _>(dh, |_| true)
}

impl SessionLockHandler for Orbit {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        // Dropping the confirmation tells the client `finished`.
        if self.greet_locked() || self.locker.is_some() {
            tracing::info!("session lock refused: already locked");
            return;
        }
        tracing::info!("session locked by a client");
        let lock = confirmation.ext_session_lock().clone();
        self.locker = Some(Locker { lock, pending: Some(confirmation), surfaces: Vec::new() });
        // The core's own lock (`lock`), not this trait's.
        Orbit::lock(self);
    }

    fn unlock(&mut self) {
        // Smithay let only the client holding the lock get here.
        if self.locker.take().is_none() {
            return;
        }
        tracing::info!("session unlocked by its locker");
        self.open();
    }

    fn new_surface(&mut self, surface: LockSurface, wl_output: WlOutput) {
        let Some(locker) = &mut self.locker else { return };
        if surface.ext_session_lock() != &locker.lock {
            return;
        }
        let Some(output) = Output::from_resource(&wl_output) else { return };
        let Some(geo) = self.space.output_geometry(&output) else { return };
        // Smithay sends the configure after this.
        surface.with_pending_state(|state| state.size = Some((geo.size.w as u32, geo.size.h as u32).into()));
        locker.surfaces.retain(|(_, s)| s.alive());
        locker.surfaces.push((output.name(), surface));
        if self.locked() {
            self.focus_face();
            self.refresh_pointer();
        }
        self.request_render();
    }
}

impl Orbit {
    /// The locker's surface on the monitor named `output`.
    fn lock_surface(&self, output: &str) -> Option<WlSurface> {
        let locker = self.locker.as_ref()?;
        locker.surfaces.iter().find(|(name, s)| name == output && s.alive()).map(|(_, s)| s.wl_surface().clone())
    }

    /// Whether `surface` is one of the locker's.
    fn is_lock_surface(&self, surface: &WlSurface) -> bool {
        self.locker.as_ref().is_some_and(|l| l.surfaces.iter().any(|(_, s)| s.wl_surface() == surface))
    }

    /// Whether `surface` is the lock's face: the greet's or the locker's.
    pub fn is_face(&self, surface: &WlSurface) -> bool {
        self.is_greet(surface) || self.is_lock_surface(surface)
    }

    /// The face's surface under `pos`, and where its origin is.
    pub fn face_surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        if self.locker.is_none() {
            return self.greet_surface_under(pos);
        }
        let output = self.output_at(pos)?;
        let origin = self.space.output_geometry(&output)?.loc;
        let surface = self.lock_surface(&output.name())?;
        under_from_surface_tree(&surface, pos, origin, WindowSurfaceType::ALL).map(|(s, p)| (s, p.to_f64()))
    }

    /// The pointer went to `output` while locked: the face follows it.
    pub fn face_follow(&mut self, output: &Output) {
        if self.locker.is_none() {
            self.greet_follow(output);
            return;
        }
        let Some(surface) = self.lock_surface(&output.name()) else { return };
        let has = matches!(self.seat.get_keyboard().and_then(|k| k.current_focus()), Some(KeyboardFocus::Surface(s)) if s == surface);
        if !has {
            self.focus_face();
        }
    }

    /// Gives the keyboard to the face: the greet, or the locker's surface
    /// on the pointer's monitor (any of them, if that one has none).
    pub fn focus_face(&mut self) {
        if self.locker.is_none() {
            self.focus_greet();
            return;
        }
        let pointer = self.pointer_output().map(|o| o.name());
        let surface = pointer
            .and_then(|name| self.lock_surface(&name))
            .or_else(|| self.locker.as_ref()?.surfaces.iter().find(|(_, s)| s.alive()).map(|(_, s)| s.wl_surface().clone()));
        let Some(surface) = surface else { return };
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, Some(KeyboardFocus::Surface(surface)), SERIAL_COUNTER.next_serial());
        }
    }

    /// Whether the keyboard is the face's.
    pub fn face_has_keyboard(&self) -> bool {
        matches!(self.seat.get_keyboard().and_then(|k| k.current_focus()), Some(KeyboardFocus::Surface(s)) if self.is_face(&s))
    }

    /// Lets the face draw its next frame on `output`.
    pub fn face_send_frame(&self, output: &Output, time: Duration) {
        if self.locker.is_none() {
            self.greet_send_frame(output, time);
            return;
        }
        if !self.locked() {
            return;
        }
        if let Some(surface) = self.lock_surface(&output.name()) {
            send_frames_surface_tree(&surface, output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
        }
    }

    /// Tells the locker `locked` once the black is on every screen.
    pub fn confirm_lock(&mut self) {
        if self.lock != crate::lock::Lock::Locked {
            return;
        }
        let Some(locker) = &mut self.locker else { return };
        if locker.pending.is_none() {
            return;
        }
        let shown = match &self.backend {
            Backend::Drm(drm) => drm.settled(),
            _ => true,
        };
        if shown && let Some(confirmation) = locker.pending.take() {
            tracing::info!("lock on screen: the locker is told");
            confirmation.lock();
        }
    }

    /// A client went away: if it held the lock, the greet takes over.
    pub fn client_left(&mut self) {
        let Some(locker) = &self.locker else { return };
        if locker.lock.is_alive() {
            return;
        }
        tracing::warn!("the locker went away while locked: the greet takes over");
        self.locker = None;
        if self.locked() {
            self.place_greet();
            self.launch_greet();
            self.focus_greet();
            self.refresh_pointer();
            self.request_render();
        }
    }
}

/// The face on `output`, at `alpha` (fading with the eyes).
pub fn face_elements(state: &Orbit, output: &Output, renderer: &mut GlesRenderer, scale: Scale<f64>, alpha: f32, elements: &mut Vec<OutputElement>) {
    if state.locker.is_none() {
        crate::greet::elements(state, output, renderer, scale, alpha, elements);
        return;
    }
    let Some(surface) = state.lock_surface(&output.name()) else { return };
    elements.extend(
        render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<GlesRenderer>>(renderer, &surface, (0, 0), scale, alpha, Kind::Unspecified)
            .into_iter()
            .map(OutputElement::Surface),
    );
}
