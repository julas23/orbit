//! The watchdog: a core whose loop stops turning (a lock taken twice, a
//! call that never returns) holds the screen, the keyboard and the
//! console switch, and only a reboot gets them back. A timer in the loop
//! beats every second; a thread of its own ends the core when no beat has
//! come for `STALL`, so the console is given back and, on the hp, the
//! getty starts Orbit again, locked. On the screen only: nested, the host
//! session is still there to close the window.
//!
//! The clock does not run while the machine sleeps, so waking up is not
//! a stall.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use smithay::reexports::calloop::{
    EventLoop,
    timer::{TimeoutAction, Timer},
};

use crate::state::Orbit;

const BEAT: Duration = Duration::from_secs(1);
const STALL: Duration = Duration::from_secs(10);

/// Starts the beat in the loop and the thread that listens for it.
pub fn start(event_loop: &mut EventLoop<'static, Orbit>) {
    let start = Instant::now();
    let last = Arc::new(AtomicU64::new(0));
    let beat = last.clone();
    let timer = event_loop.handle().insert_source(Timer::from_duration(BEAT), move |_, _, _| {
        beat.store(start.elapsed().as_millis() as u64, Ordering::Relaxed);
        TimeoutAction::ToDuration(BEAT)
    });
    if let Err(err) = timer {
        tracing::error!("no watchdog: {err}");
        return;
    }
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(BEAT);
            let silent = start.elapsed().saturating_sub(Duration::from_millis(last.load(Ordering::Relaxed)));
            if silent >= STALL {
                tracing::error!("the loop has not turned for {} s: ending the core", silent.as_secs());
                std::process::abort();
            }
        }
    });
}
