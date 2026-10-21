//! Counts on the launchers: unread messages, as applications announce
//! them with the Unity `LauncherEntry` signal (Orbit-Pathfinder,
//! Thunderbird, Telegram…) — `Update(app_uri, {count, count-visible})` on
//! the session bus. The count goes on the launcher of the same
//! application, found by the names the dock already matches desktop files
//! by; it goes away when the count does, or when the program leaves.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gtk::{gio, glib, prelude::*};

use crate::normalize;

const INTERFACE: &str = "com.canonical.Unity.LauncherEntry";

#[derive(Default)]
pub struct Badges {
    /// Each launcher's names and its badge.
    launchers: RefCell<Vec<(Vec<String>, gtk::Label)>>,
    /// The applications each program announced counts for, by its bus name.
    senders: RefCell<HashMap<String, Vec<String>>>,
}

impl Badges {
    /// Listens on the session bus; `keep` holds the subscriptions for as
    /// long as the dock lives.
    pub fn new(keep: &impl IsA<gtk::Widget>) -> Rc<Self> {
        let badges = Rc::new(Self::default());
        let conn = match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
            Ok(conn) => conn,
            Err(err) => {
                log::warn!("dock: no session bus, no counts: {err}");
                return badges;
            }
        };
        // The subscriptions hold the badges; they go when the dock does.
        let held = badges.clone();
        let update = conn.subscribe_to_signal(None, Some(INTERFACE), Some("Update"), None, None, gio::DBusSignalFlags::NONE, move |s| {
            let badges = &held;
            let uri = s.parameters.child_value(0).str().unwrap_or_default().to_owned();
            let props = glib::VariantDict::new(Some(&s.parameters.child_value(1)));
            let count = props.lookup_value("count", None).and_then(|v| v.get::<i64>()).unwrap_or(0);
            let visible = props.lookup_value("count-visible", None).and_then(|v| v.get::<bool>()).unwrap_or(false);
            {
                let mut senders = badges.senders.borrow_mut();
                let uris = senders.entry(s.sender_name.to_owned()).or_default();
                if !uris.contains(&uri) {
                    uris.push(uri.clone());
                }
            }
            badges.show(&uri, if visible { count.max(0) as u64 } else { 0 });
        });
        // A program that quits takes its counts along.
        let held = badges.clone();
        let gone = conn.subscribe_to_signal(
            Some("org.freedesktop.DBus"),
            Some("org.freedesktop.DBus"),
            Some("NameOwnerChanged"),
            Some("/org/freedesktop/DBus"),
            None,
            gio::DBusSignalFlags::NONE,
            move |s| {
                let badges = &held;
                let name = s.parameters.child_value(0).str().unwrap_or_default().to_owned();
                let new_owner = s.parameters.child_value(2).str().unwrap_or_default().to_owned();
                if !new_owner.is_empty() {
                    return;
                }
                let uris = badges.senders.borrow_mut().remove(&name).unwrap_or_default();
                for uri in uris {
                    badges.show(&uri, 0);
                }
            },
        );
        // Unsubscribed when the dock goes.
        let subscriptions = RefCell::new(Some((update, gone)));
        keep.as_ref().connect_destroy(move |_| {
            subscriptions.borrow_mut().take();
        });
        badges
    }

    /// Wraps `image` so it can carry a count, for the launcher known by
    /// `names`.
    pub fn wrap(&self, image: &gtk::Image, names: Vec<String>) -> gtk::Overlay {
        let badge = gtk::Label::builder().css_classes(["badge"]).halign(gtk::Align::End).valign(gtk::Align::Start).visible(false).build();
        badge.set_can_target(false);
        let overlay = gtk::Overlay::builder().child(image).build();
        overlay.add_overlay(&badge);
        self.launchers.borrow_mut().push((names, badge));
        overlay
    }

    fn show(&self, uri: &str, count: u64) {
        let names = names_of(uri);
        for (launcher, badge) in self.launchers.borrow().iter() {
            if launcher.iter().any(|n| names.contains(n)) {
                badge.set_label(&if count > 99 { "99+".to_owned() } else { count.to_string() });
                badge.set_visible(count > 0);
            }
        }
    }
}

/// "application://org.mozilla.Thunderbird.desktop" → the names of that
/// application: its desktop file's (name, executable, class) and its id.
fn names_of(uri: &str) -> Vec<String> {
    let file = uri.strip_prefix("application://").unwrap_or(uri);
    let id = file.trim_end_matches(".desktop");
    let mut names: Vec<String> = gio_unix::DesktopAppInfo::new(&format!("{id}.desktop")).map(|i| crate::keys(&i)).unwrap_or_default();
    names.push(normalize(id.rsplit('.').next().unwrap_or(id)));
    names.retain(|n| !n.is_empty());
    names
}
