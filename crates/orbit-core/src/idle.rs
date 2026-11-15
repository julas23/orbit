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
//!
//! After `idle_suspend` seconds (on mains) or `idle_suspend_battery` (on
//! battery), the machine suspends, through logind; the same programs and
//! fullscreen windows hold it, and `sleep.rs` locks before it sleeps.

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
    /// Looks every few seconds whether the monitors should go off.
    power_timer: Option<RegistrationToken>,
}

impl Default for Idle {
    fn default() -> Self {
        Self {
            last_input: Instant::now(),
            inhibitors: Vec::new(),
            bus_inhibited: false,
            timer: None,
            power_timer: None,
        }
    }
}

impl Orbit {
    /// Any input: the idle time starts again.
    pub fn touched(&mut self) {
        self.idle.last_input = Instant::now();
        // ext-idle-notify's clients (swayidle) hear the user is back.
        self.extra.idle_notifier.notify_activity(&self.seat);
        // Monitors switched off for idleness come back on.
        if crate::udev::any_off(self) {
            crate::udev::set_power(self, None, true);
        }
    }

    /// Whether something asks for the screen to stay on: a video playing
    /// (idle-inhibit, the session bus) or a fullscreen window.
    fn kept_awake(&mut self) -> bool {
        self.idle.inhibitors.retain(|s| s.is_alive());
        let fullscreen = self.space.outputs().any(|o| self.fullscreen_on(o).is_some());
        self.idle.bus_inhibited || !self.idle.inhibitors.is_empty() || fullscreen
    }

    /// Seconds with no input before the monitors switch off (`idle_off`);
    /// locked, a minute at most.
    fn power_timeout(&self) -> Option<Duration> {
        let seconds = self.config.idle_off.get(&crate::config::hostname());
        let seconds = if self.locked() {
            if seconds == 0 { 60 } else { seconds.min(60) }
        } else {
            seconds
        };
        (seconds > 0).then(|| Duration::from_secs(seconds))
    }

    /// Seconds with no input before the machine suspends, on mains or on
    /// battery as it is now.
    fn suspend_timeout(&self) -> Option<Duration> {
        let host = crate::config::hostname();
        let mains = crate::config::on_mains(std::path::Path::new("/sys/class/power_supply"));
        let seconds = if mains {
            self.config.idle_suspend.get(&host)
        } else {
            self.config.idle_suspend_battery.get(&host)
        };
        (seconds > 0).then(|| Duration::from_secs(seconds))
    }

    /// Suspends, through logind, when idle long enough; the idle time
    /// starts again, so waking up does not suspend once more.
    fn suspend_check(&mut self) {
        let Some(timeout) = self.suspend_timeout() else { return };
        if self.kept_awake() || self.idle.last_input.elapsed() < timeout {
            return;
        }
        tracing::info!("idle for {} s: suspending", self.idle.last_input.elapsed().as_secs());
        self.idle.last_input = Instant::now();
        // The call waits for logind; the loop must not.
        std::thread::spawn(|| {
            let result = zbus::blocking::Connection::system().and_then(|bus| {
                bus.call_method(
                    Some("org.freedesktop.login1"),
                    "/org/freedesktop/login1",
                    Some("org.freedesktop.login1.Manager"),
                    "Suspend",
                    &(false,),
                )
                .map(|_| ())
            });
            if let Err(err) = result {
                tracing::warn!("suspend: {err}");
            }
        });
    }

    fn power_check(&mut self) -> TimeoutAction {
        let every = Duration::from_secs(5);
        self.suspend_check();
        if crate::udev::any_off(self) {
            return TimeoutAction::ToDuration(every);
        }
        let Some(timeout) = self.power_timeout() else {
            return TimeoutAction::ToDuration(every);
        };
        // Locked, the screen goes dark however the programs ask.
        if !self.locked() && self.kept_awake() {
            return TimeoutAction::ToDuration(every);
        }
        if self.idle.last_input.elapsed() >= timeout {
            tracing::info!("idle for {} s: monitors off", self.idle.last_input.elapsed().as_secs());
            crate::udev::set_power(self, None, false);
        }
        TimeoutAction::ToDuration(every)
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
        if self.idle.power_timer.is_none() {
            match self
                .loop_handle
                .insert_source(Timer::from_duration(Duration::from_secs(5)), |_, _, state| {
                    state.power_check()
                }) {
                Ok(token) => self.idle.power_timer = Some(token),
                Err(err) => tracing::error!("power timer: {err}"),
            }
        }
        let Some(timeout) = self.idle_timeout() else {
            tracing::info!("idle: never locks");
            return;
        };
        tracing::info!("idle: locks after {} s", timeout.as_secs());
        self.idle.last_input = Instant::now();
        let token = self
            .loop_handle
            .insert_source(Timer::from_duration(timeout), |_, _, state| state.idle_check());
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
        self.extra.idle_notifier.set_is_inhibited(true);
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.idle.inhibitors.retain(|s| s != &surface && s.is_alive());
        if self.idle.inhibitors.is_empty() {
            tracing::info!("idle no longer inhibited");
            self.extra.idle_notifier.set_is_inhibited(false);
            self.touched();
        }
    }
}
