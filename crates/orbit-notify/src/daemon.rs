//! The server: `org.freedesktop.Notifications` on the session bus, and
//! `io.github.julas23.Orbit.Notify` for the panel's bell.
//!
//! Every notification goes to a bubble (unless do-not-disturb is on and it
//! is not critical) and, unless it is transient or its app is in
//! `no_history`, to the centre. A bubble timing out does not close the
//! notification: it stays in the centre, its actions still live, until the
//! user dismisses it or its program closes it — what "persistence" in the
//! capabilities promises. Only bubble-only notifications close when their
//! bubble goes.
//!
//! If another server holds the name (dunst, while both are installed), this
//! one waits in the queue and takes over when it leaves; it never pushes
//! the other out.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    rc::{Rc, Weak},
    time::Duration,
};

use gtk::{gdk, gio, glib, prelude::*};

use crate::{
    bubbles::Bubbles,
    card::{self, Handlers, Place},
    centre::{Actions, Centre},
    config::Config,
    note::{self, Note, Saved},
};

pub const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
pub const OWN: &str = "io.github.julas23.Orbit.Notify";
pub const OWN_PATH: &str = "/io/github/julas23/Orbit/Notify";

const XML: &str = r#"
<node>
  <interface name="org.freedesktop.Notifications">
    <method name="Notify">
      <arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/>
      <arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/>
      <arg type="a{sv}" direction="in"/><arg type="i" direction="in"/>
      <arg type="u" direction="out"/>
    </method>
    <method name="CloseNotification"><arg type="u" direction="in"/></method>
    <method name="GetCapabilities"><arg type="as" direction="out"/></method>
    <method name="GetServerInformation">
      <arg type="s" direction="out"/><arg type="s" direction="out"/>
      <arg type="s" direction="out"/><arg type="s" direction="out"/>
    </method>
    <signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>
    <signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>
    <signal name="ActivationToken"><arg type="u"/><arg type="s"/></signal>
  </interface>
  <interface name="io.github.julas23.Orbit.Notify">
    <method name="Toggle"/>
    <method name="SetDnd"><arg type="b" direction="in"/></method>
    <method name="Clear"/>
    <property name="Unread" type="u" access="read"/>
    <property name="Dnd" type="b" access="read"/>
    <signal name="Changed"><arg type="u"/><arg type="b"/></signal>
  </interface>
</node>"#;

/// Why a notification closed, as the specification numbers them.
#[derive(Clone, Copy)]
enum Reason {
    Expired = 1,
    Dismissed = 2,
    Closed = 3,
}

pub struct Daemon {
    config: Config,
    conn: gio::DBusConnection,
    /// Kept in the centre, oldest first.
    notes: RefCell<Vec<Note>>,
    /// Bubble-only, alive while their bubble is.
    passing: RefCell<Vec<Note>>,
    /// Waiting for room among the bubbles.
    waiting: RefCell<VecDeque<u32>>,
    next_id: Cell<u32>,
    dnd: Cell<bool>,
    bubbles: Bubbles,
    centre: RefCell<Option<Rc<Centre>>>,
    /// Senders being watched, to forget their actions when they leave.
    senders: RefCell<HashMap<String, Box<dyn FnOnce()>>>,
    save_queued: Cell<bool>,
    keep: RefCell<Vec<Box<dyn std::any::Any>>>,
}

impl Daemon {
    pub fn start(app: &gtk::Application, config: Config, monitor: Option<gdk::Monitor>) -> anyhow::Result<Rc<Daemon>> {
        let conn = app.dbus_connection().ok_or_else(|| anyhow::anyhow!("no session bus"))?;
        let saved = note::load();
        let mut notes = saved.notes;
        notes.retain(|n| !n.summary.is_empty() || !n.body.is_empty());
        let next = notes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        let daemon = Rc::new(Daemon {
            bubbles: Bubbles::new(&config, monitor.as_ref()),
            config,
            conn,
            notes: RefCell::new(notes),
            passing: RefCell::default(),
            waiting: RefCell::default(),
            next_id: Cell::new(next),
            dnd: Cell::new(saved.dnd),
            centre: RefCell::new(None),
            senders: RefCell::default(),
            save_queued: Cell::new(false),
            keep: RefCell::default(),
        });

        let weak = Rc::downgrade(&daemon);
        let centre = Centre::new(app, &daemon.config, monitor.as_ref(), Actions {
            dnd: Box::new(glib::clone!(#[strong] weak, move |on| if let Some(d) = weak.upgrade() { d.set_dnd(on) })),
            clear: Box::new(glib::clone!(#[strong] weak, move || if let Some(d) = weak.upgrade() { d.clear(|_| true) })),
            clear_group: Rc::new(glib::clone!(#[strong] weak, move |group: String| if let Some(d) = weak.upgrade() { d.clear(|n| n.group() == group) })),
            card: Rc::new(glib::clone!(#[strong] weak, move |id| handlers(&weak, id))),
        });
        centre.set_dnd(daemon.dnd.get());
        // Opening the centre is reading what is in it.
        centre.window.connect_visible_notify(glib::clone!(
            #[strong]
            weak,
            move |w| {
                let Some(d) = weak.upgrade() else { return };
                d.bubbles.set_hidden(w.is_visible());
                if w.is_visible() {
                    d.redraw();
                    d.mark_seen();
                }
            }
        ));
        *daemon.centre.borrow_mut() = Some(centre);
        daemon.redraw();
        daemon.register()?;
        Ok(daemon)
    }

    pub fn centre(&self) -> Option<Rc<Centre>> {
        self.centre.borrow().clone()
    }

    fn register(self: &Rc<Self>) -> anyhow::Result<()> {
        let node = gio::DBusNodeInfo::for_xml(XML)?;
        let fdo = node.lookup_interface(NAME).ok_or_else(|| anyhow::anyhow!("no {NAME} in the description"))?;
        let own = node.lookup_interface(OWN).ok_or_else(|| anyhow::anyhow!("no {OWN} in the description"))?;

        let weak = Rc::downgrade(self);
        let id = self
            .conn
            .register_object(PATH, &fdo)
            .method_call(move |_, sender, _, _, method, params, invocation| {
                let Some(d) = weak.upgrade() else { return };
                match method {
                    "Notify" => {
                        let id = d.notify(&params, sender);
                        invocation.return_value(Some(&(id,).to_variant()));
                    }
                    "CloseNotification" => {
                        let id = params.child_value(0).get::<u32>().unwrap_or(0);
                        d.close(id, Reason::Closed);
                        invocation.return_value(None);
                    }
                    "GetCapabilities" => {
                        let caps = vec!["body", "body-markup", "body-hyperlinks", "actions", "icon-static", "persistence"];
                        invocation.return_value(Some(&(caps,).to_variant()));
                    }
                    "GetServerInformation" => {
                        invocation.return_value(Some(&("Orbit-Notify", "Orbit", env!("CARGO_PKG_VERSION"), "1.2").to_variant()));
                    }
                    _ => invocation.return_value(None),
                }
            })
            .build()?;
        self.keep.borrow_mut().push(Box::new(id));

        let (call, prop) = (Rc::downgrade(self), Rc::downgrade(self));
        let id = self
            .conn
            .register_object(OWN_PATH, &own)
            .method_call(move |_, _, _, _, method, params, invocation| {
                if let Some(d) = call.upgrade() {
                    match method {
                        "Toggle" => {
                            if let Some(c) = d.centre() {
                                c.toggle();
                            }
                        }
                        "SetDnd" => d.set_dnd(params.child_value(0).get::<bool>().unwrap_or(false)),
                        "Clear" => d.clear(|_| true),
                        _ => {}
                    }
                }
                invocation.return_value(None);
            })
            .property(move |_, _, _, _, name| {
                let d = prop.upgrade();
                match name {
                    "Unread" => d.map(|d| d.unread()).unwrap_or(0).to_variant(),
                    _ => d.is_some_and(|d| d.dnd.get()).to_variant(),
                }
            })
            .build()?;
        self.keep.borrow_mut().push(Box::new(id));

        let owner = gio::bus_own_name_on_connection(
            &self.conn,
            NAME,
            gio::BusNameOwnerFlags::NONE,
            |_, _| log::info!("serving {NAME}"),
            |_, _| log::warn!("{NAME} is held by another server (dunst?); waiting for it to leave"),
        );
        self.keep.borrow_mut().push(Box::new(owner));
        Ok(())
    }

    // ------------------------------------------------------------ events

    fn notify(self: &Rc<Self>, params: &glib::Variant, sender: Option<&str>) -> u32 {
        let replaces = params.child_value(1).get::<u32>().unwrap_or(0);
        let known = replaces != 0 && (self.notes.borrow().iter().any(|n| n.id == replaces) || self.passing.borrow().iter().any(|n| n.id == replaces));
        let id = if known {
            replaces
        } else {
            let id = self.next_id.get();
            self.next_id.set(id.wrapping_add(1).max(1));
            id
        };
        let mut note = Note::from_call(id, params, sender);
        // Named by its desktop file even when it gave only its name: one
        // group per app in the centre, and its icon.
        if note.desktop.is_empty()
            && let Some(id) = card::desktop_app(&note).and_then(|a| a.id())
        {
            note.desktop = id.trim_end_matches(".desktop").to_owned();
        }
        if let Some(sender) = sender {
            self.watch(sender);
        }
        let kept = !note.transient && !self.config.no_history.iter().any(|a| a.eq_ignore_ascii_case(&note.app));
        // In place of what it replaces, wherever that was.
        self.notes.borrow_mut().retain(|n| n.id != id);
        self.passing.borrow_mut().retain(|n| n.id != id);
        let bubble = !self.dnd.get() || note.urgency == 2;
        if kept {
            self.notes.borrow_mut().push(note.clone());
            self.trim();
            self.changed();
        } else {
            self.passing.borrow_mut().push(note.clone());
        }
        if bubble {
            self.bubble(&note);
        } else if !kept {
            // Nowhere to show it.
            self.close(id, Reason::Expired);
        }
        id
    }

    /// Shows `note` as a bubble, now or when there is room.
    fn bubble(self: &Rc<Self>, note: &Note) {
        if !self.bubbles.has(note.id) && self.bubbles.is_full() {
            if !self.waiting.borrow().contains(&note.id) {
                self.waiting.borrow_mut().push_back(note.id);
            }
            return;
        }
        let ms = match note.expire {
            ms if ms > 0 => ms as u32,
            0 => 0,
            _ => self.config.timeout_ms(note.urgency),
        };
        let card = card::build(note, Place::Bubble, &handlers(&Rc::downgrade(self), note.id));
        let weak = Rc::downgrade(self);
        let id = note.id;
        self.bubbles.show(id, &card, ms, move || {
            if let Some(d) = weak.upgrade() {
                d.bubble_gone(id, true);
            }
        });
    }

    /// The bubble of `id` went: bubble-only notifications close with it;
    /// the next waiting one comes up.
    fn bubble_gone(self: &Rc<Self>, id: u32, expired: bool) {
        self.bubbles.remove(id);
        if expired && self.passing.borrow().iter().any(|n| n.id == id) {
            self.close(id, Reason::Expired);
        }
        while !self.bubbles.is_full() {
            let Some(next) = self.waiting.borrow_mut().pop_front() else { break };
            let note = self.find(next);
            if let Some(note) = note {
                self.bubble(&note);
            }
        }
    }

    fn find(&self, id: u32) -> Option<Note> {
        self.notes.borrow().iter().chain(self.passing.borrow().iter()).find(|n| n.id == id).cloned()
    }

    /// Takes `id` away everywhere and tells its program.
    fn close(self: &Rc<Self>, id: u32, reason: Reason) {
        let note = self.find(id);
        let kept = self.notes.borrow().iter().any(|n| n.id == id);
        self.notes.borrow_mut().retain(|n| n.id != id);
        self.passing.borrow_mut().retain(|n| n.id != id);
        self.waiting.borrow_mut().retain(|w| *w != id);
        if self.bubbles.has(id) {
            self.bubble_gone(id, false);
        }
        if let Some(note) = note
            && note.sender.is_some()
        {
            self.emit(NAME, PATH, "NotificationClosed", &(id, reason as u32).to_variant());
        }
        if kept {
            self.changed();
        }
    }

    /// The click on a notification: its "default" action when its program
    /// listens, its app otherwise; then it is done.
    fn activate(self: &Rc<Self>, id: u32) {
        let Some(note) = self.find(id) else { return };
        if note.has_default() {
            self.invoke(id, "default");
            return;
        }
        if let Some(app) = card::desktop_app(&note) {
            let context = gdk::Display::default().map(|d| d.app_launch_context());
            if let Err(err) = app.launch(&[], context.as_ref()) {
                log::warn!("opening {}: {err}", note.app);
            }
        }
        self.dismissed(id);
    }

    fn invoke(self: &Rc<Self>, id: u32, key: &str) {
        let Some(note) = self.find(id) else { return };
        if note.sender.is_some() {
            self.emit(NAME, PATH, "ActionInvoked", &(id, key).to_variant());
        }
        if note.resident {
            return;
        }
        self.dismissed(id);
    }

    /// Done with it: the bubble goes and the centre closes over the click.
    fn dismissed(self: &Rc<Self>, id: u32) {
        self.close(id, Reason::Dismissed);
        if let Some(c) = self.centre() {
            c.window.set_visible(false);
        }
    }

    fn set_dnd(self: &Rc<Self>, on: bool) {
        if self.dnd.get() == on {
            return;
        }
        self.dnd.set(on);
        if let Some(c) = self.centre() {
            c.set_dnd(on);
        }
        if on {
            let notes = self.notes.borrow().clone();
            self.bubbles.clear(|id| notes.iter().any(|n| n.id == id && n.urgency == 2));
            self.waiting.borrow_mut().clear();
        }
        self.changed();
    }

    /// Dismisses every kept notification that `which` picks.
    fn clear(self: &Rc<Self>, which: impl Fn(&Note) -> bool) {
        let ids: Vec<u32> = self.notes.borrow().iter().filter(|n| which(n)).map(|n| n.id).collect();
        for id in ids {
            self.close(id, Reason::Dismissed);
        }
    }

    // ----------------------------------------------------------- helpers

    fn unread(&self) -> u32 {
        self.notes.borrow().iter().filter(|n| !n.seen).count() as u32
    }

    fn mark_seen(self: &Rc<Self>) {
        let mut any = false;
        for n in self.notes.borrow_mut().iter_mut().filter(|n| !n.seen) {
            n.seen = true;
            any = true;
        }
        if any {
            self.changed();
        }
    }

    /// Drops the oldest beyond the limit.
    fn trim(self: &Rc<Self>) {
        let extra = self.notes.borrow().len().saturating_sub(self.config.history_limit.max(1));
        let old: Vec<u32> = self.notes.borrow().iter().take(extra).map(|n| n.id).collect();
        for id in old {
            self.close(id, Reason::Expired);
        }
    }

    /// The kept list or do-not-disturb changed: the panel hears, the
    /// centre is redrawn, the history saved soon.
    fn changed(self: &Rc<Self>) {
        self.emit(OWN, OWN_PATH, "Changed", &(self.unread(), self.dnd.get()).to_variant());
        if self.centre().is_some_and(|c| c.window.is_visible()) {
            self.redraw();
            // Arriving while the centre is open is arriving seen.
            if self.unread() > 0 {
                for n in self.notes.borrow_mut().iter_mut() {
                    n.seen = true;
                }
                self.emit(OWN, OWN_PATH, "Changed", &(0u32, self.dnd.get()).to_variant());
            }
        }
        self.queue_save();
    }

    fn redraw(&self) {
        if let Some(c) = self.centre() {
            c.show(&self.notes.borrow());
        }
    }

    fn queue_save(self: &Rc<Self>) {
        if self.save_queued.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_secs(2), move || {
            if let Some(d) = weak.upgrade() {
                d.save_now();
            }
        });
    }

    pub fn save_now(&self) {
        self.save_queued.set(false);
        note::save(&Saved { dnd: self.dnd.get(), notes: self.notes.borrow().clone() });
    }

    fn emit(&self, iface: &str, path: &str, signal: &str, params: &glib::Variant) {
        if let Err(err) = self.conn.emit_signal(None, path, iface, signal, Some(params)) {
            log::warn!("emitting {signal}: {err}");
        }
    }

    /// Forgets the sender's actions when it leaves the bus: its
    /// notifications stay, clicking them opens the app instead.
    fn watch(self: &Rc<Self>, sender: &str) {
        if self.senders.borrow().contains_key(sender) {
            return;
        }
        let weak = Rc::downgrade(self);
        let name = sender.to_owned();
        let id = gio::bus_watch_name_on_connection(&self.conn, sender, gio::BusNameWatcherFlags::NONE, |_, _, _| {}, move |_, _| {
            let Some(d) = weak.upgrade() else { return };
            for n in d.notes.borrow_mut().iter_mut().filter(|n| n.sender.as_deref() == Some(name.as_str())) {
                n.sender = None;
            }
            if let Some(unwatch) = d.senders.borrow_mut().remove(&name) {
                // After this callback returns.
                glib::idle_add_local_once(unwatch);
            }
            if d.centre().is_some_and(|c| c.window.is_visible()) {
                d.redraw();
            }
        });
        self.senders.borrow_mut().insert(sender.to_owned(), Box::new(move || gio::bus_unwatch_name(id)));
    }
}

/// The clicks on the card of `id`.
fn handlers(weak: &Weak<Daemon>, id: u32) -> Handlers {
    let (a, b, c) = (weak.clone(), weak.clone(), weak.clone());
    Handlers {
        activate: Rc::new(move || {
            if let Some(d) = a.upgrade() {
                d.activate(id);
            }
        }),
        dismiss: Rc::new(move || {
            if let Some(d) = b.upgrade() {
                d.close(id, Reason::Dismissed);
            }
        }),
        action: Rc::new(move |key| {
            if let Some(d) = c.upgrade() {
                d.invoke(id, &key);
            }
        }),
    }
}
