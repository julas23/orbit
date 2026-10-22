//! Locking when idle (PLANO-CORE §7.3 step 8): after `idle_lock` seconds
//! with no key, click or pointer motion, the core locks itself. On the
//! screen only: nested, the host session has its own.
//!
//! A program that must not be interrupted (a video playing in Firefox,
//! Chromium or mpv) says so with `idle-inhibit-v1`; while any of its
//! inhibitors lives, the idle time does not count, and it starts again
//! from zero when the last one goes. Most programs ask on the session bus
//! instead (`screensaver`), and count the same; so does any fullscreen
//! window shown, on any monitor (a video, a game). Super+L and `lock`
//! (IPC) lock always.

use std::time::{Duration, Instant};

use smithay::{
    reexports::{
        calloop::{
            RegistrationToken,
            timer::{TimeoutAction, Timer},
        },
        wayland_server::{Resource, protocol::wl_surface::WlSurface},
    },
    wayland::idle_inhibit::IdleInhibitHandler,
};

use crate::state::{Backend, Orbit};

#[derive(Debug)]
pub struct Idle {
    /// The last input from the user.
    pub last_input: Instant,
    /// The surfaces whose clients asked for no idle.
    pub inhibitors: Vec<WlSurface>,
    /// Some program holds an `org.freedesktop.ScreenSaver` inhibitor.
    pub bus_inhibited: bool,
    timer: Option<RegistrationToken>,
}

impl Default for Idle {
    fn default() -> Self {
        Self { last_input: Instant::now(), inhibitors: Vec::new(), bus_inhibited: false, timer: None }
    }
}

impl Orbit {
    /// Any input: the idle time starts again.
    pub fn touched(&mut self) {
        self.idle.last_input = Instant::now();
    }

    /// The timeout this machine has, if any.
    fn idle_timeout(&self) -> Option<Duration> {
        let seconds = self.config.idle_lock.get(&crate::config::hostname());
        (seconds > 0).then(|| Duration::from_secs(seconds))
    }

    /// (Re)arms the idle timer, from the config: at the start and at each
    /// `reload`.
    pub fn arm_idle(&mut self) {
        if let Some(token) = self.idle.timer.take() {
            self.loop_handle.remove(token);
        }
        if !matches!(self.backend, Backend::Drm(_)) {
            return;
        }
        let Some(timeout) = self.idle_timeout() else {
            tracing::info!("idle: never locks");
            return;
        };
        tracing::info!("idle: locks after {} s", timeout.as_secs());
        self.idle.last_input = Instant::now();
        let token = self.loop_handle.insert_source(Timer::from_duration(timeout), |_, _, state| state.idle_check());
        match token {
            Ok(token) => self.idle.timer = Some(token),
            Err(err) => tracing::error!("idle timer: {err}"),
        }
    }

    /// The timer went off: locks if the time is up, or waits for the rest.
    fn idle_check(&mut self) -> TimeoutAction {
        let Some(timeout) = self.idle_timeout() else {
            self.idle.timer = None;
            return TimeoutAction::Drop;
        };
        self.idle.inhibitors.retain(|s| s.is_alive());
        let fullscreen = self.space.outputs().any(|o| self.fullscreen_on(o).is_some());
        if self.locked() || self.idle.bus_inhibited || !self.idle.inhibitors.is_empty() || fullscreen {
            self.idle.last_input = Instant::now();
            return TimeoutAction::ToDuration(timeout);
        }
        let idle = self.idle.last_input.elapsed();
        if idle < timeout {
            return TimeoutAction::ToDuration(timeout - idle);
        }
        tracing::info!("idle for {} s: locking", idle.as_secs());
        self.lock();
        TimeoutAction::ToDuration(timeout)
    }
}

impl IdleInhibitHandler for Orbit {
    fn inhibit(&mut self, surface: WlSurface) {
        tracing::info!("idle inhibited by {:?}", surface.id());
        self.idle.inhibitors.push(surface);
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.idle.inhibitors.retain(|s| s != &surface && s.is_alive());
        if self.idle.inhibitors.is_empty() {
            tracing::info!("idle no longer inhibited");
            self.touched();
        }
    }
}
