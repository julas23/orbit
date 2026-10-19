//! StatusNotifierItem: the D-Bus side of the system tray, with no widgets.
//!
//! Applications (nm-applet, blueman, udiskie…) register their tray item with
//! the `org.kde.StatusNotifierWatcher` on the session bus; a host (the panel)
//! reads the list from it and talks to each item. Only one process can own
//! the watcher name, so:
//!
//! - if nobody owns it, the panel becomes the watcher itself, and is also its
//!   own host;
//! - if someone does (the eww panel, while both run), the panel only
//!   registers as a host and follows that watcher's list and signals — and
//!   takes the name over if that watcher goes away.
//!
//! Items are named `bus-name/object/path`, the form watchers use.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
pub const ITEM: &str = "org.kde.StatusNotifierItem";

const WATCHER_XML: &str = r#"
<node>
  <interface name="org.kde.StatusNotifierWatcher">
    <method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
    <method name="RegisterStatusNotifierHost"><arg type="s" direction="in"/></method>
    <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
    <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
    <property name="ProtocolVersion" type="i" access="read"/>
    <signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
    <signal name="StatusNotifierItemUnregistered"><arg type="s"/></signal>
    <signal name="StatusNotifierHostRegistered"/>
  </interface>
</node>"#;

/// Splits an item name into bus name and object path.
pub fn split(item: &str) -> (&str, &str) {
    match item.find('/') {
        Some(i) => (&item[..i], &item[i..]),
        None => (item, "/StatusNotifierItem"),
    }
}

/// What the host tells the tray widget.
pub enum Change {
    Added(String),
    Removed(String),
}

pub struct Host {
    pub conn: gio::DBusConnection,
    items: RefCell<Vec<String>>,
    notify: Box<dyn Fn(Change)>,
    /// Keeps signal subscriptions and name watches alive.
    keep: RefCell<Vec<Box<dyn std::any::Any>>>,
    watcher: Cell<bool>,
}

impl Host {
    /// Starts the tray on `conn` (the session bus); `notify` hears of items
    /// coming and going — including, before this returns, the ones already
    /// there.
    pub fn start(conn: gio::DBusConnection, notify: impl Fn(Change) + 'static) -> Rc<Host> {
        let host = Rc::new(Host {
            conn,
            items: RefCell::default(),
            notify: Box::new(notify),
            keep: RefCell::default(),
            watcher: Cell::new(false),
        });
        if host.has_owner(WATCHER) {
            host.follow();
        } else {
            host.become_watcher();
        }
        host
    }

    fn has_owner(&self, name: &str) -> bool {
        self.conn
            .call_sync(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "NameHasOwner",
                Some(&(name,).to_variant()),
                Some(glib::VariantTy::new("(b)").expect("valid type")),
                gio::DBusCallFlags::NONE,
                1000,
                gio::Cancellable::NONE,
            )
            .ok()
            .and_then(|reply| reply.child_value(0).get::<bool>())
            .unwrap_or(false)
    }

    fn add(&self, item: &str) {
        if self.items.borrow().iter().any(|i| i == item) {
            return;
        }
        self.items.borrow_mut().push(item.to_string());
        (self.notify)(Change::Added(item.to_string()));
    }

    fn remove(&self, item: &str) {
        let had = {
            let mut items = self.items.borrow_mut();
            let before = items.len();
            items.retain(|i| i != item);
            before != items.len()
        };
        if had {
            (self.notify)(Change::Removed(item.to_string()));
        }
    }

    // ---------------------------------------------------------- host mode

    /// Follows the watcher someone else runs.
    fn follow(self: &Rc<Self>) {
        log::info!("tray: following the existing {WATCHER}");
        let host_name = format!("org.kde.StatusNotifierHost-{}", std::process::id());
        let _ = self.conn.call_sync(
            Some(WATCHER),
            WATCHER_PATH,
            WATCHER,
            "RegisterStatusNotifierHost",
            Some(&(host_name.as_str(),).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            1000,
            gio::Cancellable::NONE,
        );

        for (signal, added) in [
            ("StatusNotifierItemRegistered", true),
            ("StatusNotifierItemUnregistered", false),
        ] {
            let weak = Rc::downgrade(self);
            let sub = self.conn.subscribe_to_signal(
                Some(WATCHER),
                Some(WATCHER),
                Some(signal),
                Some(WATCHER_PATH),
                None,
                gio::DBusSignalFlags::NONE,
                move |s| {
                    let (Some(host), Some(item)) = (
                        weak.upgrade(),
                        s.parameters.child_value(0).str().map(str::to_owned),
                    ) else {
                        return;
                    };
                    if added {
                        host.add(&item)
                    } else {
                        host.remove(&item)
                    }
                },
            );
            self.keep.borrow_mut().push(Box::new(sub));
        }

        let reply = self.conn.call_sync(
            Some(WATCHER),
            WATCHER_PATH,
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&(WATCHER, "RegisteredStatusNotifierItems").to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            1000,
            gio::Cancellable::NONE,
        );
        if let Ok(reply) = reply
            && let Some(items) = reply.child_value(0).as_variant()
            && let Some(items) = items.get::<Vec<String>>()
        {
            for item in items {
                self.add(&item);
            }
        }

        // If that watcher goes away, take its place.
        let weak = Rc::downgrade(self);
        let id = gio::bus_watch_name_on_connection(
            &self.conn,
            WATCHER,
            gio::BusNameWatcherFlags::NONE,
            |_, _, _| {},
            move |_, _| {
                if let Some(host) = weak.upgrade()
                    && !host.watcher.get()
                {
                    log::info!("tray: {WATCHER} went away, taking it over");
                    for item in host.items.borrow().clone() {
                        host.remove(&item);
                    }
                    host.keep.borrow_mut().clear();
                    host.become_watcher();
                }
            },
        );
        self.keep.borrow_mut().push(Box::new(id));
    }

    // ------------------------------------------------------- watcher mode

    /// Runs the watcher, and is its own host.
    fn become_watcher(self: &Rc<Self>) {
        let info = gio::DBusNodeInfo::for_xml(WATCHER_XML)
            .ok()
            .and_then(|node| node.lookup_interface(WATCHER));
        let Some(info) = info else {
            log::error!("tray: bad watcher interface description");
            return;
        };
        let call = Rc::downgrade(self);
        let prop = Rc::downgrade(self);
        let registered = self
            .conn
            .register_object(WATCHER_PATH, &info)
            .method_call(move |conn, sender, _, _, method, params, invocation| {
                let Some(host) = call.upgrade() else { return };
                let arg = params.child_value(0).str().unwrap_or_default().to_string();
                match method {
                    "RegisterStatusNotifierItem" => {
                        let sender = sender.unwrap_or_default();
                        // A path means "this object, on the caller's bus name".
                        let item = if arg.starts_with('/') {
                            format!("{sender}{arg}")
                        } else if arg.is_empty() {
                            format!("{sender}/StatusNotifierItem")
                        } else if arg.contains('/') {
                            arg.clone()
                        } else {
                            format!("{arg}/StatusNotifierItem")
                        };
                        invocation.return_value(None);
                        host.watch_owner(&item);
                        host.add(&item);
                        emit(&conn, "StatusNotifierItemRegistered", Some(&item));
                    }
                    "RegisterStatusNotifierHost" => {
                        invocation.return_value(None);
                        emit(&conn, "StatusNotifierHostRegistered", None);
                    }
                    _ => invocation.return_value(None),
                }
            })
            .property(move |_, _, _, _, name| match name {
                "RegisteredStatusNotifierItems" => prop
                    .upgrade()
                    .map(|h| h.items.borrow().clone())
                    .unwrap_or_default()
                    .to_variant(),
                "IsStatusNotifierHostRegistered" => true.to_variant(),
                _ => 0i32.to_variant(),
            })
            .build();
        if let Err(err) = registered {
            log::error!("tray: registering the watcher object: {err}");
            return;
        }
        let weak = Rc::downgrade(self);
        let owner = gio::bus_own_name_on_connection(
            &self.conn,
            WATCHER,
            gio::BusNameOwnerFlags::DO_NOT_QUEUE,
            |_, _| log::info!("tray: running the {WATCHER}"),
            move |_, _| {
                if let Some(host) = weak.upgrade() {
                    host.watcher.set(false);
                }
                log::warn!("tray: could not own {WATCHER}");
            },
        );
        self.watcher.set(true);
        self.keep.borrow_mut().push(Box::new(owner));
    }

    /// Drops `item` when its bus name leaves the bus.
    fn watch_owner(self: &Rc<Self>, item: &str) {
        let (bus, _) = split(item);
        let weak = Rc::downgrade(self);
        let item = item.to_string();
        let id = gio::bus_watch_name_on_connection(
            &self.conn,
            bus,
            gio::BusNameWatcherFlags::NONE,
            |_, _, _| {},
            move |conn, _| {
                if let Some(host) = weak.upgrade() {
                    host.remove(&item);
                    emit(&conn, "StatusNotifierItemUnregistered", Some(&item));
                }
            },
        );
        self.keep.borrow_mut().push(Box::new(id));
    }
}

fn emit(conn: &gio::DBusConnection, signal: &str, item: Option<&str>) {
    let params = item.map(|i| (i,).to_variant());
    if let Err(err) = conn.emit_signal(None, WATCHER_PATH, WATCHER, signal, params.as_ref()) {
        log::warn!("tray: emitting {signal}: {err}");
    }
}

/// The parts of an item the panel shows.
#[derive(Debug, Default, Clone)]
pub struct Props {
    pub icon_name: String,
    pub icon_theme_path: String,
    pub attention_icon_name: String,
    /// (width, height, ARGB32 big-endian bytes)
    pub pixmaps: Vec<(i32, i32, Vec<u8>)>,
    pub status: String,
    pub title: String,
    pub tooltip: String,
    pub menu: Option<String>,
    pub item_is_menu: bool,
}

impl Props {
    /// Parses the reply to `GetAll(org.kde.StatusNotifierItem)`.
    pub fn parse(reply: &glib::Variant) -> Props {
        let dict = glib::VariantDict::new(Some(&reply.child_value(0)));
        let string = |key: &str| {
            dict.lookup::<String>(key)
                .ok()
                .flatten()
                .unwrap_or_default()
        };
        let pixmaps = dict
            .lookup_value("IconPixmap", None)
            .map(|v| pixmaps(&v))
            .unwrap_or_default();
        let tooltip = dict
            .lookup_value("ToolTip", None)
            .map(|t| {
                // (s icon, a(iiay) pixmaps, s title, s text)
                let title = t.child_value(2).str().unwrap_or_default().to_string();
                let text = t.child_value(3).str().unwrap_or_default().to_string();
                [title, text]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let menu = dict
            .lookup_value("Menu", None)
            .and_then(|v| v.str().map(str::to_owned))
            .filter(|p| p != "/" && !p.is_empty());
        Props {
            icon_name: string("IconName"),
            icon_theme_path: string("IconThemePath"),
            attention_icon_name: string("AttentionIconName"),
            pixmaps,
            status: string("Status"),
            title: string("Title"),
            tooltip,
            menu,
            item_is_menu: dict
                .lookup::<bool>("ItemIsMenu")
                .ok()
                .flatten()
                .unwrap_or(false),
        }
    }
}

fn pixmaps(value: &glib::Variant) -> Vec<(i32, i32, Vec<u8>)> {
    (0..value.n_children())
        .filter_map(|i| {
            let p = value.child_value(i);
            let w = p.child_value(0).get::<i32>()?;
            let h = p.child_value(1).get::<i32>()?;
            let bytes = p.child_value(2).data().to_vec();
            (w > 0 && h > 0 && bytes.len() >= (w * h * 4) as usize).then_some((w, h, bytes))
        })
        .collect()
}
