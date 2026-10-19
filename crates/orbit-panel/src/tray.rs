//! System tray module, after the tray of the eww panel.
//!
//! Block form: a card whose icons wrap — `per_line` to a line, extra lines
//! grow the card downwards, never the panel sideways. Line form: one row.
//!
//! Left click activates the item (or opens its menu when the item is only a
//! menu); right click opens its menu (dbusmenu) or asks the item to show its
//! own; middle click is the secondary action; the wheel scrolls the item.
//! Passive items are hidden, as the protocol asks.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gtk::{gdk, gio, glib, prelude::*};
use serde::Deserialize;

use crate::{
    dbusmenu,
    sni::{self, Change, Host, Props},
};

/// `modules/tray.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TrayConfig {
    /// Icon size in px.
    pub icon_size: i32,
    /// Icons per line in a vertical panel.
    pub per_line: u32,
}

impl Default for TrayConfig {
    fn default() -> Self {
        // 5 × 20 + 4 × 5 = 120 px, inside the card's 130.
        Self {
            icon_size: 20,
            per_line: 5,
        }
    }
}

struct Entry {
    button: gtk::Button,
    image: gtk::Image,
    props: Rc<RefCell<Props>>,
    _signals: gio::SignalSubscription,
}

struct Module {
    config: TrayConfig,
    flow: gtk::FlowBox,
    conn: gio::DBusConnection,
    host: RefCell<Option<Rc<Host>>>,
    entries: RefCell<HashMap<String, Entry>>,
}

pub fn new(config: &TrayConfig, vertical: bool) -> gtk::Widget {
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .column_spacing(5)
        .row_spacing(5)
        .min_children_per_line(1)
        .max_children_per_line(if vertical { config.per_line.max(1) } else { 64 })
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    let card = gtk::Box::builder()
        .css_classes(if vertical {
            vec!["card", "tray"]
        } else {
            vec!["tray"]
        })
        .build();
    flow.set_hexpand(true);
    card.append(&flow);

    let conn = match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
        Ok(conn) => conn,
        Err(err) => {
            log::error!("tray: no session bus: {err}");
            return card.upcast();
        }
    };
    let module = Rc::new(Module {
        config: config.clone(),
        flow,
        conn: conn.clone(),
        host: RefCell::new(None),
        entries: RefCell::new(HashMap::new()),
    });
    let weak = Rc::downgrade(&module);
    let host = Host::start(conn, move |change| {
        let Some(module) = weak.upgrade() else { return };
        match change {
            Change::Added(item) => module.add(&item),
            Change::Removed(item) => module.remove(&item),
        }
    });
    *module.host.borrow_mut() = Some(host);
    card.connect_destroy(move |_| {
        let _ = &module;
    });
    card.upcast()
}

impl Module {
    fn add(self: &Rc<Self>, item: &str) {
        if self.entries.borrow().contains_key(item) {
            return;
        }
        let conn = self.conn.clone();
        let (bus, path) = sni::split(item);
        let image = gtk::Image::builder()
            .pixel_size(self.config.icon_size)
            .build();
        let button = gtk::Button::builder()
            .child(&image)
            .css_classes(["tray-item"])
            .visible(false) // until its properties arrive
            .build();
        let props = Rc::new(RefCell::new(Props::default()));
        self.wire(&button, &conn, bus, path, &props);
        self.flow.append(&button);

        // The item's own signals (NewIcon, NewStatus, NewToolTip…) say
        // "re-read me"; they do not carry the new values.
        let signals = {
            let weak = Rc::downgrade(self);
            let item = item.to_string();
            conn.subscribe_to_signal(
                Some(bus),
                Some(sni::ITEM),
                None,
                Some(path),
                None,
                gio::DBusSignalFlags::NONE,
                move |_| {
                    if let Some(module) = weak.upgrade() {
                        module.fetch(&item);
                    }
                },
            )
        };
        self.entries.borrow_mut().insert(
            item.to_string(),
            Entry {
                button,
                image,
                props,
                _signals: signals,
            },
        );
        self.fetch(item);
    }

    fn remove(&self, item: &str) {
        if let Some(entry) = self.entries.borrow_mut().remove(item) {
            // The FlowBox wraps each child in a FlowBoxChild.
            if let Some(child) = entry.button.parent() {
                self.flow.remove(&child);
            }
        }
    }

    /// Reads all properties of `item` and shows them.
    fn fetch(self: &Rc<Self>, item: &str) {
        let conn = &self.conn;
        let (bus, path) = sni::split(item);
        let weak = Rc::downgrade(self);
        let item = item.to_string();
        conn.call(
            Some(bus),
            path,
            "org.freedesktop.DBus.Properties",
            "GetAll",
            Some(&(sni::ITEM,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
            move |reply| {
                let (Some(module), Ok(reply)) = (weak.upgrade(), reply) else {
                    return;
                };
                module.show(&item, Props::parse(&reply));
            },
        );
    }

    fn show(&self, item: &str, props: Props) {
        let entries = self.entries.borrow();
        let Some(entry) = entries.get(item) else {
            return;
        };
        if !props.icon_theme_path.is_empty()
            && let Some(display) = gdk::Display::default()
        {
            let theme = gtk::IconTheme::for_display(&display);
            if !theme
                .search_path()
                .iter()
                .any(|p| p.as_os_str() == props.icon_theme_path.as_str())
            {
                theme.add_search_path(&props.icon_theme_path);
            }
        }
        let attention = props.status == "NeedsAttention" && !props.attention_icon_name.is_empty();
        let name = if attention {
            &props.attention_icon_name
        } else {
            &props.icon_name
        };
        set_icon(&entry.image, name, &props.pixmaps, self.config.icon_size);

        let tooltip = if props.tooltip.is_empty() {
            props.title.clone()
        } else {
            props.tooltip.clone()
        };
        // Tooltips may carry markup; show it only if it parses.
        if pango_ok(&tooltip) {
            entry.button.set_tooltip_markup(Some(&tooltip));
        } else {
            entry.button.set_tooltip_text(Some(&tooltip));
        }
        let visible = props.status != "Passive";
        entry.button.set_visible(visible);
        if let Some(child) = entry.button.parent() {
            child.set_visible(visible);
        }
        *entry.props.borrow_mut() = props;
    }

    fn wire(
        self: &Rc<Self>,
        button: &gtk::Button,
        conn: &gio::DBusConnection,
        bus: &str,
        path: &str,
        props: &Rc<RefCell<Props>>,
    ) {
        let call = {
            let conn = conn.clone();
            let (bus, path) = (bus.to_string(), path.to_string());
            move |method: &str, args: glib::Variant| {
                conn.call(
                    Some(&bus),
                    &path,
                    sni::ITEM,
                    method,
                    Some(&args),
                    None,
                    gio::DBusCallFlags::NONE,
                    2000,
                    gio::Cancellable::NONE,
                    |_| {},
                );
            }
        };
        let menu = {
            let conn = conn.clone();
            let bus = bus.to_string();
            let props = props.clone();
            let call = call.clone();
            let anchor = button.downgrade();
            move |at: orbit_app::popup::Point| {
                let Some(anchor) = anchor.upgrade() else {
                    return;
                };
                match props.borrow().menu.clone() {
                    Some(menu) => dbusmenu::show(&conn, &bus, &menu, &anchor, at),
                    None => call("ContextMenu", (0i32, 0i32).to_variant()),
                }
            }
        };

        let left = gtk::GestureClick::builder().button(1).build();
        {
            let props = props.clone();
            let call = call.clone();
            let menu = menu.clone();
            left.connect_released(move |_, _, x, y| {
                if props.borrow().item_is_menu {
                    menu(Some((x, y)));
                } else {
                    call("Activate", (0i32, 0i32).to_variant());
                }
            });
        }
        let right = gtk::GestureClick::builder().button(3).build();
        right.connect_pressed(move |_, _, x, y| menu(Some((x, y))));
        let middle = gtk::GestureClick::builder().button(2).build();
        {
            let call = call.clone();
            middle.connect_released(move |_, _, _, _| {
                call("SecondaryActivate", (0i32, 0i32).to_variant())
            });
        }
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        wheel.connect_scroll(move |_, _, dy| {
            call("Scroll", ((-dy).round() as i32, "vertical").to_variant());
            glib::Propagation::Stop
        });
        for controller in [left, right, middle] {
            button.add_controller(controller);
        }
        button.add_controller(wheel);
    }
}

/// The icon by name when the theme has it (or it is a file), else the
/// pixmap closest to `size`, else a generic one.
fn set_icon(image: &gtk::Image, name: &str, pixmaps: &[(i32, i32, Vec<u8>)], size: i32) {
    if name.starts_with('/') {
        image.set_from_file(Some(name));
        return;
    }
    let theme = gdk::Display::default().map(|d| gtk::IconTheme::for_display(&d));
    if !name.is_empty() && theme.as_ref().is_some_and(|t| t.has_icon(name)) {
        image.set_icon_name(Some(name));
        return;
    }
    let best = pixmaps
        .iter()
        .min_by_key(|(w, _, _)| ((w - size).abs(), -w));
    if let Some((w, h, bytes)) = best {
        let texture = gdk::MemoryTexture::new(
            *w,
            *h,
            gdk::MemoryFormat::A8r8g8b8, // SNI pixmaps: ARGB32, network order
            &glib::Bytes::from(&bytes[..(w * h * 4) as usize]),
            (*w * 4) as usize,
        );
        image.set_paintable(Some(&texture));
        return;
    }
    image.set_icon_name(Some(if name.is_empty() {
        "application-x-executable"
    } else {
        name
    }));
}

fn pango_ok(text: &str) -> bool {
    gtk::pango::parse_markup(text, '\0').is_ok()
}
