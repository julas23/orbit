//! `org.freedesktop.ScreenSaver` on the session bus: how most programs ask
//! not to be locked while they play. Chromium, VLC and Qt programs call it
//! themselves; Firefox asks the portal, whose GTK backend, without a GNOME
//! session, passes the request on here. A thread of its own serves the bus
//! and tells the core, through a channel, when the first inhibitor comes
//! and the last goes (`idle`). On the screen only, as the idle lock.
//!
//! A program that leaves the bus without `UnInhibit` (a crash) has its
//! cookies dropped with it.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use smithay::reexports::calloop::channel::{self, Channel, Sender};
use zbus::{
    blocking::{Connection, connection, fdo::DBusProxy},
    interface,
    message::Header,
    names::OwnedUniqueName,
};

use crate::state::Orbit;

/// What the bus thread tells the core.
#[derive(Debug)]
pub enum Event {
    /// Some program holds an inhibitor (true) or none does (false).
    Inhibited(bool),
    /// `SimulateUserActivity`: as if the user had touched something.
    Activity,
}

#[derive(Default)]
struct Inhibitors {
    /// Cookie → who holds it, and why (for the log).
    held: HashMap<u32, (OwnedUniqueName, String)>,
    next: u32,
}

#[derive(Clone)]
struct ScreenSaver {
    inhibitors: Arc<Mutex<Inhibitors>>,
    events: Sender<Event>,
}

impl ScreenSaver {
    /// Drops the cookies `keep` refuses, and says so if none is left.
    fn release(&self, keep: impl Fn(u32, &OwnedUniqueName) -> bool) {
        let mut inhibitors = self.inhibitors.lock().unwrap();
        let before = inhibitors.held.len();
        inhibitors.held.retain(|cookie, (owner, _)| keep(*cookie, owner));
        if before > 0 && inhibitors.held.is_empty() {
            let _ = self.events.send(Event::Inhibited(false));
        }
    }
}

#[interface(name = "org.freedesktop.ScreenSaver")]
impl ScreenSaver {
    fn inhibit(&self, #[zbus(header)] header: Header<'_>, application_name: &str, reason_for_inhibit: &str) -> u32 {
        let Some(sender) = header.sender() else { return 0 };
        let mut inhibitors = self.inhibitors.lock().unwrap();
        inhibitors.next = inhibitors.next.wrapping_add(1).max(1);
        let cookie = inhibitors.next;
        tracing::info!("idle inhibited by {application_name} ({sender}): {reason_for_inhibit}");
        inhibitors.held.insert(cookie, (sender.to_owned().into(), application_name.to_owned()));
        if inhibitors.held.len() == 1 {
            let _ = self.events.send(Event::Inhibited(true));
        }
        cookie
    }

    fn un_inhibit(&self, cookie: u32) {
        self.release(|held, _| held != cookie);
    }

    fn simulate_user_activity(&self) {
        let _ = self.events.send(Event::Activity);
    }
}

/// Serves the bus until the core ends; the cookies of programs that leave
/// it go with them.
fn serve(screensaver: ScreenSaver) -> zbus::Result<()> {
    let connection: Connection = connection::Builder::session()?
        .name("org.freedesktop.ScreenSaver")?
        .serve_at("/org/freedesktop/ScreenSaver", screensaver.clone())?
        .serve_at("/ScreenSaver", screensaver.clone())?
        .build()?;
    tracing::info!("serving org.freedesktop.ScreenSaver");
    for signal in DBusProxy::new(&connection)?.receive_name_owner_changed()? {
        let Ok(args) = signal.args() else { continue };
        if args.new_owner().is_none() {
            let gone = args.name().to_string();
            screensaver.release(|_, owner| owner.as_str() != gone);
        }
    }
    Ok(())
}

impl Orbit {
    /// Starts the bus thread and listens to it in the event loop.
    pub fn serve_screensaver(&mut self) {
        let (events, channel): (Sender<Event>, Channel<Event>) = channel::channel();
        let screensaver = ScreenSaver { inhibitors: Default::default(), events };
        let inserted = self.loop_handle.insert_source(channel, |event, _, state| {
            if let channel::Event::Msg(event) = event {
                state.screensaver_event(event);
            }
        });
        if let Err(err) = inserted {
            tracing::error!("screensaver channel: {err}");
            return;
        }
        std::thread::spawn(move || {
            if let Err(err) = serve(screensaver) {
                tracing::error!("no org.freedesktop.ScreenSaver: {err}");
            }
        });
    }

    fn screensaver_event(&mut self, event: Event) {
        match event {
            Event::Inhibited(true) => self.idle.bus_inhibited = true,
            Event::Inhibited(false) => {
                tracing::info!("idle no longer inhibited on the bus");
                self.idle.bus_inhibited = false;
                self.touched();
            }
            Event::Activity => self.touched(),
        }
    }
}
