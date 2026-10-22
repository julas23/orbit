//! The locked state (PLANO-CORE §7): login and screen lock are one layer of
//! the core. Locked, the screen is black and no client is drawn or gets
//! input — the session goes on behind, unseen. The core keeps the lock,
//! not a client: a greet that dies leaves the session locked.
//!
//! `orbit-core --locked` starts that way, before any client connects;
//! `lock` (IPC) locks a running session. Unlocking is the greet's alone
//! (`greet`), so nothing in the session can open it.

use smithay::utils::SERIAL_COUNTER;

use crate::state::Orbit;

/// Whether the session is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lock {
    #[default]
    Unlocked,
    Locked,
}

impl Orbit {
    pub fn locked(&self) -> bool {
        self.lock != Lock::Unlocked
    }

    /// Hides the session: the keyboard and the pointer leave every client
    /// for the greet, and the next frame is black.
    pub fn lock(&mut self) {
        if self.locked() {
            return;
        }
        tracing::info!("locked");
        self.lock = Lock::Locked;
        // A client's pointer image would be a client drawn.
        self.cursor_status = smithay::input::pointer::CursorImageStatus::default_named();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
        self.greet_state();
        self.place_greet();
        self.focus_greet();
        self.refresh_pointer();
        self.launch_greet();
        self.request_render();
    }

    /// Shows the session again, with the keyboard where it was. The
    /// greet's to call (`greet`).
    pub fn unlock(&mut self) {
        if !self.locked() {
            return;
        }
        tracing::info!("unlocked");
        self.lock = Lock::Unlocked;
        self.greet_state();
        match self.exclusive_layer() {
            Some(layer) => self.focus_layer(&layer),
            None => {
                let ws = self.current_ws();
                let focus = ws.last_focus.clone().filter(|w| ws.windows.contains(w)).or_else(|| ws.windows.last().cloned());
                self.focus_window(focus);
            }
        }
        // The greet's pointer image goes with it.
        self.cursor_status = smithay::input::pointer::CursorImageStatus::default_named();
        self.refresh_pointer();
        self.request_render();
    }

    /// Sends the pointer, where it is, to what is under it now: the greet
    /// while locked, the session otherwise.
    pub fn refresh_pointer(&mut self) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        let location = pointer.current_location();
        let under = if self.locked() { self.greet_surface_under(location) } else { self.surface_under(location) };
        let serial = SERIAL_COUNTER.next_serial();
        let time = smithay::backend::input::InputTime::now();
        pointer.motion(self, under, &smithay::input::pointer::MotionEvent { location, serial, time });
        pointer.frame(self);
    }
}
