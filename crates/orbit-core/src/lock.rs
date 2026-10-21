//! The locked state (PLANO-CORE §7): login and screen lock are one layer of
//! the core. Locked, the screen is black and no client is drawn or gets
//! input — the session goes on behind, unseen. The core keeps the lock,
//! not a client: a greet that dies leaves the session locked.
//!
//! `orbit-core --locked` starts that way, before any client connects;
//! `lock` (IPC) locks a running session. Unlocking is the greet's alone
//! (§7.3 step 3), so nothing in the session can open it.

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

    /// Hides the session: the keyboard and the pointer leave every client,
    /// and the next frame is black.
    pub fn lock(&mut self) {
        if self.locked() {
            return;
        }
        tracing::info!("locked");
        self.lock = Lock::Locked;
        // A client's pointer image would be a client drawn.
        self.cursor_status = smithay::input::pointer::CursorImageStatus::default_named();
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, None, serial);
        }
        if let Some(pointer) = self.seat.get_pointer() {
            let location = pointer.current_location();
            let time = smithay::backend::input::InputTime::now();
            pointer.motion(self, None, &smithay::input::pointer::MotionEvent { location, serial, time });
            pointer.frame(self);
        }
        self.request_render();
    }

    /// Shows the session again, with the keyboard where it was.
    #[allow(dead_code)] // The greet calls it (§7.3 step 3).
    pub fn unlock(&mut self) {
        if !self.locked() {
            return;
        }
        tracing::info!("unlocked");
        self.lock = Lock::Unlocked;
        match self.exclusive_layer() {
            Some(layer) => self.focus_layer(&layer),
            None => {
                let ws = self.current_ws();
                let focus = ws.last_focus.clone().filter(|w| ws.windows.contains(w)).or_else(|| ws.windows.last().cloned());
                self.focus_window(focus);
            }
        }
        self.request_render();
    }
}
