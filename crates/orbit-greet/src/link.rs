//! The greet's side of `orbit_greet_v1` (`orbit-protocols`): on GDK's own
//! Wayland connection, with a queue of its own, read when the connection
//! has something and dispatched in GTK's main loop.
//!
//! The core shows the global only to the greet it launched; anywhere else
//! (another compositor, a terminal of the session) there is no link, and
//! the greet runs as a preview.

use std::{cell::RefCell, rc::Rc, time::Duration};

use gtk::{gdk, glib, prelude::*};
use orbit_protocols::client::orbit_greet_v1::{self, OrbitGreetV1};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry,
};

/// What the core tells the greet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Locked,
    Unlocked,
}

#[derive(Default)]
struct Inbox(Vec<Event>);

pub struct Link {
    conn: Connection,
    queue: RefCell<EventQueue<Inbox>>,
    greet: OrbitGreetV1,
}

impl Link {
    /// The link, when GTK runs on Wayland and the compositor offers
    /// `orbit_greet_v1` to this client.
    pub fn connect(display: &gdk::Display) -> Option<Rc<Self>> {
        let display = display.downcast_ref::<gdk4_wayland::WaylandDisplay>()?;
        let raw = display.wl_display_raw()?;
        // Safety: GDK's wl_display, alive for as long as the display, which
        // is as long as the application.
        let backend = unsafe { wayland_backend::client::Backend::from_foreign_display(raw.as_ptr() as *mut _) };
        let conn = Connection::from_backend(backend);
        let (globals, queue) = registry_queue_init::<Inbox>(&conn)
            .inspect_err(|err| log::error!("registry: {err}"))
            .ok()?;
        let greet = globals.bind::<OrbitGreetV1, _, _>(&queue.handle(), 1..=1, ()).ok()?;
        Some(Rc::new(Self { conn, queue: RefCell::new(queue), greet }))
    }

    /// Calls `on_event` with each event, in GTK's main loop, starting
    /// with the state the core sent on bind.
    pub fn watch(self: &Rc<Self>, on_event: impl Fn(Event) + 'static) {
        let on_event: Rc<dyn Fn(Event)> = Rc::new(on_event);
        self.dispatch(&*on_event);
        let fd = {
            use std::os::fd::AsRawFd;
            self.conn.backend().poll_fd().as_raw_fd()
        };
        let (link, handler) = (self.clone(), on_event.clone());
        glib_unix::unix_fd_add_local(fd, glib::IOCondition::IN, move |_, _| {
            link.read();
            link.dispatch(&*handler);
            glib::ControlFlow::Continue
        });
        // GDK reads the same connection: events it read for this queue
        // wait there until the next look.
        let link = self.clone();
        glib::timeout_add_local(Duration::from_millis(200), move || {
            link.dispatch(&*on_event);
            glib::ControlFlow::Continue
        });
    }

    fn read(&self) {
        if let Some(guard) = self.conn.prepare_read() {
            // Nothing to read is not an error worth a word.
            let _ = guard.read();
        }
    }

    fn dispatch(&self, on_event: &dyn Fn(Event)) {
        let mut inbox = Inbox::default();
        if let Err(err) = self.queue.borrow_mut().dispatch_pending(&mut inbox) {
            log::error!("wayland: {err}");
        }
        for event in inbox.0 {
            on_event(event);
        }
    }

    /// The user proved who they are: the core shows the session.
    pub fn unlock(&self) {
        self.greet.unlock();
        if let Err(err) = self.conn.flush() {
            log::error!("unlock: {err}");
        }
    }
}

impl Dispatch<OrbitGreetV1, ()> for Inbox {
    fn event(inbox: &mut Self, _: &OrbitGreetV1, event: orbit_greet_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            orbit_greet_v1::Event::Locked => inbox.0.push(Event::Locked),
            orbit_greet_v1::Event::Unlocked => inbox.0.push(Event::Unlocked),
            _ => {}
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Inbox {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}
