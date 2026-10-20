//! Sway IPC for the panel: the client from orbit-common, plus event
//! subscriptions.
//!
//! Queries and commands run on the GTK thread — sway answers them in well
//! under a millisecond. Subscriptions block, so they run on their own thread
//! and only wake the GTK thread through a channel.

use std::{io, time::Duration};

pub use orbit_common::sway::*;
use serde::Deserialize;

/// Calls `notify` on the GTK thread after sway reports any of `events`
/// (e.g. `["workspace", "output"]`). A burst of events results in one call.
/// Reconnects if sway restarts.
pub fn subscribe(events: &'static [&'static str], notify: impl Fn() + 'static) {
    let (tx, rx) = async_channel::unbounded::<()>();
    let payload = serde_json::to_string(events).expect("a list of strings serializes");

    std::thread::spawn(move || {
        loop {
            let run = || -> io::Result<()> {
                let mut conn = Connection::open()?;
                conn.request(SUBSCRIBE, &payload)?;
                loop {
                    conn.receive()?;
                    if tx.send_blocking(()).is_err() {
                        return Ok(()); // the GTK side is gone
                    }
                }
            };
            match run() {
                Ok(()) => return,
                Err(err) => {
                    log::warn!("sway: subscription lost ({err}), retrying");
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        }
    });

    gtk::glib::spawn_future_local(async move {
        while rx.recv().await.is_ok() {
            while rx.try_recv().is_ok() {} // fold a burst into one update
            notify();
        }
    });
}

#[derive(Debug, Deserialize)]
pub struct Workspace {
    pub num: i32,
    pub focused: bool,
}

#[derive(Debug, Deserialize)]
pub struct Output {
    pub name: String,
    pub active: bool,
    pub focused: bool,
    pub rect: Rect,
}

/// One entry of a RUN_COMMAND reply.
#[derive(Debug, Deserialize)]
pub struct CommandResult {
    pub success: bool,
}

#[derive(Debug, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
}
