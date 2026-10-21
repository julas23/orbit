//! Notifications module: the bell of Orbit-Notify, the notification
//! server — how many came since the centre was last opened, and whether
//! do-not-disturb is on. Same shape as the status modules.
//!
//! Event-driven: the server's `Changed` signal on the session bus, and its
//! name coming and going. Left click opens or closes the centre; right
//! click turns do-not-disturb on or off.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};
use serde::Deserialize;

use orbit_app::i18n::{number, tr, tr_args};

use crate::status::Row;

const NAME: &str = "io.github.julas23.Orbit.Notify";
const PATH: &str = "/io/github/julas23/Orbit/Notify";

/// No options yet; the file may be empty.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationsConfig {}

struct Module {
    row: Rc<Row>,
    conn: Option<gio::DBusConnection>,
    dnd: Cell<bool>,
    keep: RefCell<Vec<Box<dyn std::any::Any>>>,
}

pub fn new(_config: &NotificationsConfig, vertical: bool) -> gtk::Widget {
    let row = Row::new("notifications", vertical);
    let conn = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .inspect_err(|err| log::error!("notifications: no session bus: {err}"))
        .ok();
    let module = Rc::new(Module { row: row.clone(), conn, dnd: Cell::new(false), keep: RefCell::default() });
    module.show(None);

    if let Some(conn) = &module.conn {
        let weak = Rc::downgrade(&module);
        let sub = conn.subscribe_to_signal(Some(NAME), Some(NAME), Some("Changed"), Some(PATH), None, gio::DBusSignalFlags::NONE, move |s| {
            if let Some(m) = weak.upgrade() {
                let unread = s.parameters.child_value(0).get::<u32>().unwrap_or(0);
                let dnd = s.parameters.child_value(1).get::<bool>().unwrap_or(false);
                m.show(Some((unread, dnd)));
            }
        });
        module.keep.borrow_mut().push(Box::new(sub));
        let (appeared, vanished) = (Rc::downgrade(&module), Rc::downgrade(&module));
        let watch = gio::bus_watch_name_on_connection(
            conn,
            NAME,
            gio::BusNameWatcherFlags::NONE,
            move |_, _, _| {
                if let Some(m) = appeared.upgrade() {
                    m.read();
                }
            },
            move |_, _| {
                if let Some(m) = vanished.upgrade() {
                    m.show(None);
                }
            },
        );
        module.keep.borrow_mut().push(Box::new(watch));
    }

    let click = gtk::GestureClick::builder().button(0).build();
    {
        let module = module.clone();
        click.connect_released(move |gesture, _, _, _| match gesture.current_button() {
            1 => module.call("Toggle", None),
            3 => module.call("SetDnd", Some((!module.dnd.get(),).to_variant())),
            _ => {}
        });
    }
    row.root.add_controller(click);
    row.root.set_cursor_from_name(Some("pointer"));
    row.root.clone()
}

impl Module {
    /// Shows (unread, do-not-disturb), or no server.
    fn show(&self, state: Option<(u32, bool)>) {
        let tooltip = tr("notifications-tooltip");
        let Some((unread, dnd)) = state else {
            self.row.root.remove_css_class("unread");
            self.row.show("󰂜", &tr("notifications-none"), &tr("notifications-off"), &tooltip);
            return;
        };
        self.dnd.set(dnd);
        let first = if unread == 0 { tr("notifications-none") } else { tr_args("notifications-new", &[("count", number(unread))]) };
        let (glyph, second) = match (dnd, unread) {
            (true, _) => ("󰂛", tr("notifications-dnd")),
            (false, 0) => ("󰂜", tr("notifications-on")),
            (false, _) => ("󰂞", tr("notifications-on")),
        };
        if unread > 0 {
            self.row.root.add_css_class("unread");
        } else {
            self.row.root.remove_css_class("unread");
        }
        self.row.show(glyph, &first, &second, &tooltip);
    }

    /// Reads the server's state, when it shows up.
    fn read(self: &Rc<Self>) {
        let Some(conn) = &self.conn else { return };
        let weak = Rc::downgrade(self);
        let unread = Rc::new(Cell::new(None::<u32>));
        for prop in ["Unread", "Dnd"] {
            let (weak, unread) = (weak.clone(), unread.clone());
            conn.call(
                Some(NAME),
                PATH,
                "org.freedesktop.DBus.Properties",
                "Get",
                Some(&(NAME, prop).to_variant()),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                2000,
                gio::Cancellable::NONE,
                move |reply| {
                    let Some(m) = weak.upgrade() else { return };
                    let value = reply.ok().and_then(|r| r.child_value(0).as_variant());
                    match prop {
                        "Unread" => unread.set(value.and_then(|v| v.get::<u32>())),
                        _ => {
                            let dnd = value.and_then(|v| v.get::<bool>()).unwrap_or(false);
                            // Replies come in order: the count is in.
                            m.show(Some((unread.get().unwrap_or(0), dnd)));
                        }
                    }
                },
            );
        }
    }

    fn call(&self, method: &str, args: Option<glib::Variant>) {
        let Some(conn) = &self.conn else { return };
        let method = method.to_owned();
        conn.call(
            Some(NAME),
            PATH,
            NAME,
            &method.clone(),
            args.as_ref(),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            2000,
            gio::Cancellable::NONE,
            move |reply| {
                if let Err(err) = reply {
                    log::warn!("notifications: {method}: {err}");
                }
            },
        );
    }
}
