//! Locking before the machine sleeps (PLANO-CORE §7.3 step 8): the core
//! holds a logind `delay` inhibitor for sleep, so a suspend (the lid, the
//! power menu, `systemctl suspend`) waits for it. On `PrepareForSleep` the
//! core locks and lets go once the black frame is on every screen — the
//! session is never seen on waking. logind waits `InhibitDelayMaxSec` (5 s)
//! at most; the core never more than `MAX_WAIT`. A new inhibitor is taken
//! on waking. On the screen only: nested, the host session has its own.
//!
//! A thread of its own listens on the system bus, as `screensaver` does on
//! the session bus.

use std::{os::fd::OwnedFd, time::Duration};

use smithay::reexports::calloop::{
    channel::{self, Channel, Sender},
    timer::{TimeoutAction, Timer},
};
use zbus::blocking::{Connection, Proxy};

use crate::state::{Backend, Orbit};

/// The longest a suspend waits for the lock to be on screen.
const MAX_WAIT: Duration = Duration::from_secs(2);

/// The machine is about to sleep; the inhibitor, if one was held, to let
/// go once locked.
struct Sleeping(Option<OwnedFd>);

/// Takes a `delay` inhibitor for sleep.
fn inhibit(manager: &Proxy<'_>) -> Option<OwnedFd> {
    let fd: Result<zbus::zvariant::OwnedFd, _> =
        manager.call("Inhibit", &("sleep", "Orbit", "Lock the screen before sleeping", "delay"));
    fd.inspect_err(|err| tracing::warn!("no sleep inhibitor: {err}")).ok().map(OwnedFd::from)
}

/// Listens to logind until the core ends.
fn listen(events: Sender<Sleeping>) -> zbus::Result<()> {
    let connection = Connection::system()?;
    let manager = Proxy::new(&connection, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager")?;
    // Listening first: a suspend between the two is not missed.
    let signals = manager.receive_signal("PrepareForSleep")?;
    let mut held = inhibit(&manager);
    tracing::info!("locks before sleeping");
    for signal in signals {
        let Ok(going) = signal.body().deserialize::<bool>() else { continue };
        if going {
            if events.send(Sleeping(held.take())).is_err() {
                break;
            }
        } else {
            tracing::info!("woke up");
            held = inhibit(&manager);
        }
    }
    Ok(())
}

impl Orbit {
    /// Starts the bus thread and listens to it in the event loop.
    pub fn lock_before_sleep(&mut self) {
        let (events, channel): (Sender<Sleeping>, Channel<Sleeping>) = channel::channel();
        let inserted = self.loop_handle.insert_source(channel, |event, _, state| {
            if let channel::Event::Msg(Sleeping(fd)) = event {
                state.going_to_sleep(fd);
            }
        });
        if let Err(err) = inserted {
            tracing::error!("sleep channel: {err}");
            return;
        }
        std::thread::spawn(move || {
            if let Err(err) = listen(events) {
                tracing::error!("no lock before sleeping: {err}");
            }
        });
    }

    /// Locks, and holds the suspend until the lock is drawn.
    fn going_to_sleep(&mut self, fd: Option<OwnedFd>) {
        // Already locked: the inhibitor goes at once.
        if self.lock == crate::lock::Lock::Locked {
            return;
        }
        // No one watches the eyes close with the lid shut.
        tracing::info!("going to sleep: locking");
        self.lock_at_once();
        let Some(fd) = fd else { return };
        self.sleep_inhibitor = Some(fd);
        let timer = self.loop_handle.insert_source(Timer::from_duration(MAX_WAIT), |_, _, state| {
            if state.sleep_inhibitor.take().is_some() {
                tracing::warn!("the lock was not drawn in time: sleeping anyway");
            }
            TimeoutAction::Drop
        });
        if let Err(err) = timer {
            tracing::error!("sleep timer: {err}");
            self.sleep_inhibitor = None;
        }
    }

    /// A frame reached a screen: once every screen shows the lock, the
    /// machine may sleep.
    pub fn lock_shown(&mut self) {
        if self.sleep_inhibitor.is_none() || self.lock != crate::lock::Lock::Locked {
            return;
        }
        let Backend::Drm(drm) = &self.backend else { return };
        if drm.settled() {
            tracing::info!("lock on screen: sleeping");
            self.sleep_inhibitor = None;
        }
    }
}
