//! The locked state (PLANO-CORE §7): login and screen lock are one layer of
//! the core. Locked, the screen is black and no client is drawn or gets
//! input — the session goes on behind, unseen. The core keeps the lock,
//! not a client: a greet that dies leaves the session locked.
//!
//! Locking closes the eyes and unlocking opens them (`eyes`): for
//! `eyes::DURATION` the session blurs and fades to black, or the other way.
//! Input leaves the clients on the first frame of closing and comes back
//! after the last of opening. Before the first frame (`--locked`), or when
//! the shaders failed, and before sleeping, it is at once.
//!
//! `orbit-core --locked` starts that way, before any client connects;
//! `lock` (IPC) locks a running session. Unlocking is the greet's alone
//! (`greet`), so nothing in the session can open it — or, when a locker
//! of others holds the lock, that locker's (`session_lock`).

use std::time::Instant;

use smithay::utils::SERIAL_COUNTER;

use crate::{eyes::DURATION, state::Orbit};

/// Whether the session is shown, and the eyes moving between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lock {
    #[default]
    Unlocked,
    /// Locking since then.
    Closing(Instant),
    Locked,
    /// Unlocking since then.
    Opening(Instant),
}

impl Orbit {
    /// Whether the clients are cut off: locked, or the eyes moving.
    pub fn locked(&self) -> bool {
        self.lock != Lock::Unlocked
    }

    /// Whether the greet is told the session is locked: locked or closing.
    pub fn greet_locked(&self) -> bool {
        matches!(self.lock, Lock::Locked | Lock::Closing(_))
    }

    /// Whether the session is on screen, sharp or not: everything but locked.
    pub fn session_shown(&self) -> bool {
        self.lock != Lock::Locked
    }

    /// How closed the eyes are (0 open, 1 closed) while they move.
    pub fn eyes(&self) -> Option<f32> {
        let progress = |start: Instant| (start.elapsed().as_secs_f32() / DURATION.as_secs_f32()).min(1.0);
        match self.lock {
            Lock::Closing(start) => Some(crate::eyes::ease(progress(start))),
            Lock::Opening(start) => Some(1.0 - crate::eyes::ease(progress(start))),
            _ => None,
        }
    }

    /// Whether frames must keep coming (the eyes moving).
    pub fn animating(&self) -> bool {
        matches!(self.lock, Lock::Closing(_) | Lock::Opening(_))
    }

    /// The eyes finish moving once their time is up; called at each frame.
    pub fn step_eyes(&mut self) {
        match self.lock {
            Lock::Closing(start) if start.elapsed() >= DURATION => {
                self.lock = Lock::Locked;
                self.eyes_cache.clear();
            }
            Lock::Opening(start) if start.elapsed() >= DURATION => self.opened(),
            _ => {}
        }
    }

    /// The moment a turn the other way starts from, so the eyes go back
    /// from where they are (the curve is symmetric).
    fn turn(start: Instant) -> Instant {
        let left = DURATION.saturating_sub(start.elapsed());
        Instant::now().checked_sub(left).unwrap_or_else(Instant::now)
    }

    /// Whether locking and unlocking are drawn: the shaders are there.
    fn can_animate(&self) -> bool {
        matches!(self.shaders, Some(Some(_)))
    }

    /// Hides the session, closing the eyes: the keyboard and the pointer
    /// leave every client for the greet at once.
    pub fn lock(&mut self) {
        self.lock = match self.lock {
            Lock::Locked | Lock::Closing(_) => return,
            Lock::Opening(start) => Lock::Closing(Self::turn(start)),
            Lock::Unlocked if self.can_animate() => Lock::Closing(Instant::now()),
            Lock::Unlocked => Lock::Locked,
        };
        tracing::info!("locked");
        self.cut_off();
    }

    /// Hides the session with no transition: the next frame is black.
    pub fn lock_at_once(&mut self) {
        let was = std::mem::replace(&mut self.lock, Lock::Locked);
        self.eyes_cache.clear();
        if !matches!(was, Lock::Locked | Lock::Closing(_)) {
            tracing::info!("locked");
            self.cut_off();
        }
    }

    /// The clients lose the keyboard and the pointer to the greet.
    fn cut_off(&mut self) {
        // A client's pointer image would be a client drawn.
        self.cursor_status = smithay::input::pointer::CursorImageStatus::default_named();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
        self.greet_state();
        // A locker of others is the face itself (`session_lock`).
        if self.locker.is_none() {
            self.place_greet();
        }
        self.focus_face();
        self.refresh_pointer();
        if self.locker.is_none() {
            self.launch_greet();
        }
        self.request_render();
        self.lock_event("locked");
    }

    /// Shows the session again, opening the eyes; the keyboard goes back
    /// where it was once they are open. The greet's to call (`greet`),
    /// unless a locker of others holds the lock.
    pub fn unlock(&mut self) {
        if self.locker.is_some() {
            tracing::warn!("unlock refused: a locker of others holds the lock");
            return;
        }
        self.open();
    }

    /// Opens the eyes, for the greet or for the locker holding the lock.
    pub fn open(&mut self) {
        match self.lock {
            Lock::Unlocked | Lock::Opening(_) => return,
            Lock::Closing(start) => self.lock = Lock::Opening(Self::turn(start)),
            Lock::Locked if self.can_animate() => self.lock = Lock::Opening(Instant::now()),
            Lock::Locked => {
                self.lock = Lock::Opening(Instant::now());
                self.opened();
            }
        }
        tracing::info!("unlocked");
        self.greet_state();
        self.request_render();
        self.log_in();
        self.lock_event("unlocked");
    }

    /// The eyes are open: the clients get the keyboard and the pointer.
    fn opened(&mut self) {
        self.lock = Lock::Unlocked;
        self.eyes_cache.clear();
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

    /// Runs the config's `login:` commands, the first time only.
    pub fn log_in(&mut self) {
        if std::mem::replace(&mut self.logged_in, true) {
            return;
        }
        for line in self.config.login.clone() {
            self.spawn(&line);
        }
    }

    /// Sends the pointer, where it is, to what is under it now: the greet
    /// while locked, the session otherwise.
    pub fn refresh_pointer(&mut self) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        let location = pointer.current_location();
        let under = if self.locked() { self.face_surface_under(location) } else { self.surface_under(location) };
        let serial = SERIAL_COUNTER.next_serial();
        let time = smithay::backend::input::InputTime::now();
        pointer.motion(self, under, &smithay::input::pointer::MotionEvent { location, serial, time });
        pointer.frame(self);
    }
}
