//! Status modules, after the network/keyboard/battery card of the eww
//! panel — now independent modules sharing one shape: a larger icon with
//! two lines of text beside it (the network's own in `network`).
//!
//!   keyboard  language            /  the rest of the layout name
//!   battery   charge percentage   /  uptime
//!
//! Event-driven: UPower over D-Bus, the keyboard layout over sway IPC. Only
//! the uptime ticks, once a minute.
//!
//! Every status module takes the same clicks (`clicks`): left, the quick
//! part; right, the full part; the wheel steps through a value. Keyboard:
//! left the next layout, right the list to pick one, the wheel walks them.
//! Battery: left its details and the power profile, right `manager`.
//!
//! Normal (4 × 1 cells): the icon next to two lines of text. Full (4 × 2):
//! a larger icon next to up to four lines — the two, then what is otherwise
//! only in the tooltip (the network's kind, the battery's state, the other
//! keyboard layouts). Mini (1 × 1): the icon alone. Line form: icon and first
//! line. In every form the tooltip holds whatever the form leaves out.

use std::{cell::RefCell, rc::Rc};

use gtk::{gio, glib, prelude::*};
use serde::Deserialize;

use orbit_app::i18n::{number, text, tr, tr_args};

use crate::{
    board::{self, Form},
    sway,
};

/// None of the status modules has options yet; their files may be empty.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StatusConfig {}

/// The shared shape: an icon and lines of text beside it (also the bell's,
/// in `notifications`).
pub(crate) struct Row {
    pub(crate) root: gtk::Widget,
    icon: gtk::Label,
    /// First, second and the Full form's two more.
    lines: Vec<gtk::Label>,
    form: Form,
}

impl Row {
    pub(crate) fn new(class: &str, form: Form) -> Rc<Row> {
        let icon = gtk::Label::builder()
            .css_classes(["icon"])
            .valign(gtk::Align::Center)
            .build();
        let line = |class: &str| {
            gtk::Label::builder()
                .css_classes([class])
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build()
        };
        let lines = vec![
            line("first"),
            line("second"),
            line("second"),
            line("second"),
        ];
        let root = gtk::Box::builder()
            .spacing(5)
            .css_classes(if form.vertical() {
                vec!["card", "status", class, form.class()]
            } else {
                vec!["status-line", class]
            })
            .build();
        root.append(&icon);
        let shown = match form {
            Form::Full => 4,
            Form::Normal => 2,
            Form::Line => 1,
            Form::Mini => 0,
        };
        if form == Form::Mini {
            icon.set_hexpand(true);
            board::centre_glyph(&icon);
        }
        if form.vertical() && shown > 0 {
            let column = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .valign(gtk::Align::Center)
                .hexpand(true)
                .build();
            for line in &lines[..shown] {
                column.append(line);
            }
            root.append(&column);
        } else if shown > 0 {
            root.append(&lines[0]);
        }
        Rc::new(Row {
            root: root.upcast(),
            icon,
            lines,
            form,
        })
    }

    /// Shows `glyph` and `lines` — at most four; the first two are the main
    /// ones — with `tooltip` and whatever lines the form leaves out.
    pub(crate) fn show(&self, glyph: &str, lines: &[&str], tooltip: &str) {
        self.icon.set_label(glyph);
        for (i, label) in self.lines.iter().enumerate() {
            let text = lines.get(i).copied().unwrap_or_default();
            label.set_label(text);
            // A Full card shows only the lines it has.
            label.set_visible(i < 2 || !text.is_empty());
        }
        let shown = match self.form {
            Form::Full => 4,
            Form::Normal => 2,
            Form::Line => 1,
            Form::Mini => 0,
        };
        let tooltip = lines
            .iter()
            .skip(shown)
            .copied()
            .chain([tooltip])
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        self.root.set_tooltip_text(Some(&tooltip));
    }
}

/// Reads one D-Bus property.
pub(crate) fn property(
    conn: &gio::DBusConnection,
    dest: &str,
    path: &str,
    iface: &str,
    name: &str,
) -> Option<glib::Variant> {
    conn.call_sync(
        Some(dest),
        path,
        "org.freedesktop.DBus.Properties",
        "Get",
        Some(&(iface, name).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        1000,
        gio::Cancellable::NONE,
    )
    .ok()?
    .child_value(0)
    .as_variant()
}

/// Calls `refresh` once, on the next idle, after any PropertiesChanged from
/// `sender` — NetworkManager sends them in bursts.
pub(crate) fn on_changes(
    conn: &gio::DBusConnection,
    sender: &str,
    path: Option<&str>,
    refresh: impl Fn() + 'static,
) -> gio::SignalSubscription {
    let refresh = Rc::new(refresh);
    let pending = Rc::new(std::cell::Cell::new(false));
    conn.subscribe_to_signal(
        Some(sender),
        Some("org.freedesktop.DBus.Properties"),
        Some("PropertiesChanged"),
        path,
        None,
        gio::DBusSignalFlags::NONE,
        move |_| {
            if pending.replace(true) {
                return;
            }
            let (refresh, pending) = (refresh.clone(), pending.clone());
            glib::idle_add_local_once(move || {
                pending.set(false);
                refresh();
            });
        },
    )
}

pub(crate) fn system_bus() -> Option<gio::DBusConnection> {
    gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)
        .inspect_err(|err| log::error!("status: no system bus: {err}"))
        .ok()
}

/// Keeps `keep` alive as long as `widget` exists.
pub(crate) fn hold<T: 'static>(widget: &gtk::Widget, keep: T) {
    widget.connect_destroy(move |_| {
        let _ = &keep;
    });
}

/// Focuses the program's window if it is open (sway), or starts it.
pub(crate) fn open_or_focus(command: &str) {
    let program = command.split_whitespace().next().unwrap_or_default();
    let name = program.rsplit('/').next().unwrap_or(program);
    if sway::available() && !name.is_empty() {
        let focus = format!("[app_id=\"(?i){name}\"] focus");
        let focused = sway::Connection::open()
            .and_then(|mut c| c.request(sway::RUN_COMMAND, &focus))
            .ok()
            .and_then(|r| serde_json::from_slice::<Vec<sway::CommandResult>>(&r).ok())
            .is_some_and(|r| r.iter().all(|r| r.success));
        if focused {
            return;
        }
    }
    if let Err(err) = glib::spawn_command_line_async(command) {
        log::error!("status: running {command:?}: {err}");
    }
}

/// The clicks every status module answers the same way (PLANO-PAINEL §2):
/// left, the quick part; right, the full one; the wheel, a step up (+1) or
/// down (-1) where there is a value. The whole card takes them.
pub(crate) fn clicks(
    root: &gtk::Widget,
    left: impl Fn() + 'static,
    right: impl Fn() + 'static,
    wheel: Option<Box<dyn Fn(i32)>>,
) {
    let click = gtk::GestureClick::builder().button(0).build();
    click.connect_released(move |gesture, _, _, _| match gesture.current_button() {
        1 => left(),
        3 => right(),
        _ => {}
    });
    root.add_controller(click);
    if let Some(step) = wheel {
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.connect_scroll(move |_, _, dy| {
            step(if dy < 0.0 { 1 } else { -1 });
            glib::Propagation::Stop
        });
        root.add_controller(scroll);
    }
    root.add_css_class("clickable");
    root.set_cursor_from_name(Some("pointer"));
}

/// Opens `child` in a popover under `anchor`: a module's quick part.
pub(crate) fn popup(child: &impl IsA<gtk::Widget>, anchor: &impl IsA<gtk::Widget>) -> gtk::Popover {
    let popover = gtk::Popover::builder().child(child).build();
    orbit_app::popup::open(&popover, anchor, None);
    popover
}

/// A vertical list for a quick part, with `class` for its look.
pub(crate) fn quick_list(class: &str) -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(5)
        .css_classes(["quick", class])
        .build()
}

/// A line of `list`: `label` on the left, a switch on the right.
pub(crate) fn switch_row(label: &str, on: bool, list: &gtk::Box) -> gtk::Switch {
    let line = gtk::Box::builder().spacing(10).build();
    let name = gtk::Label::builder()
        .label(label)
        .xalign(0.0)
        .hexpand(true)
        .build();
    let switch = gtk::Switch::builder()
        .active(on)
        .valign(gtk::Align::Center)
        .build();
    line.append(&name);
    line.append(&switch);
    list.append(&line);
    switch
}

/// A small heading inside a quick part.
pub(crate) fn quick_title(label: &str, list: &gtk::Box) {
    list.append(
        &gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .css_classes(["title"])
            .build(),
    );
}

/// A flat button of a quick part; the popover closes when it is pressed.
pub(crate) fn quick_button(label: &str, list: &gtk::Box, pressed: impl Fn() + 'static) -> gtk::Button {
    let button = gtk::Button::builder().has_frame(false).build();
    button.set_child(Some(
        &gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    ));
    button.connect_clicked(move |button| {
        pressed();
        if let Some(popover) = button.ancestor(gtk::Popover::static_type()) {
            popover.downcast_ref::<gtk::Popover>().expect("a popover").popdown();
        }
    });
    list.append(&button);
    button
}

// ------------------------------------------------------------ keyboard

pub fn keyboard(_: &StatusConfig, form: Form) -> gtk::Widget {
    let row = Row::new("keyboard", form);
    if !sway::available() {
        log::warn!("keyboard: not running under sway");
        return row.root.clone();
    }
    let layouts: Rc<RefCell<Layouts>> = Rc::default();
    let refresh = {
        let (row, layouts) = (row.clone(), layouts.clone());
        move || show_keyboard(&row, &layouts)
    };
    refresh();
    sway::subscribe(&["input"], refresh);

    let (list, anchor) = (layouts.clone(), row.root.downgrade());
    let wheel = layouts.clone();
    clicks(
        &row.root,
        || switch_layout("next"),
        move || {
            if let Some(anchor) = anchor.upgrade() {
                pick_layout(&list.borrow(), &anchor);
            }
        },
        Some(Box::new(move |step| {
            if wheel.borrow().names.len() > 1 {
                switch_layout(if step > 0 { "prev" } else { "next" });
            }
        })),
    );
    row.root.clone()
}

/// The keyboard's layouts, and which one is on.
#[derive(Debug, Default)]
struct Layouts {
    names: Vec<String>,
    active: usize,
}

/// `next`, `prev` or a layout's index.
fn switch_layout(which: &str) {
    let command = format!("input type:keyboard xkb_switch_layout {which}");
    let _ = sway::Connection::open().and_then(|mut c| c.request(sway::RUN_COMMAND, &command));
}

/// The full part: every layout, the active one marked; a click picks one.
fn pick_layout(layouts: &Layouts, anchor: &gtk::Widget) {
    let list = quick_list("keyboard-quick");
    for (i, name) in layouts.names.iter().enumerate() {
        let button = quick_button(name, &list, move || switch_layout(&i.to_string()));
        if i == layouts.active {
            button.add_css_class("active");
        }
    }
    popup(&list, anchor);
}

fn show_keyboard(row: &Row, layouts: &RefCell<Layouts>) {
    let inputs: Vec<serde_json::Value> = sway::Connection::open()
        .and_then(|mut c| c.query(sway::GET_INPUTS))
        .unwrap_or_default();
    // The first real keyboard: the one with a list of layouts.
    let keyboard = inputs.iter().find(|i| {
        i["type"] == "keyboard"
            && i["xkb_layout_names"]
                .as_array()
                .is_some_and(|l| !l.is_empty())
    });
    let Some(keyboard) = keyboard else { return };
    let name = keyboard["xkb_active_layout_name"]
        .as_str()
        .unwrap_or_default();
    let all: Vec<&str> = keyboard["xkb_layout_names"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l.as_str())
        .collect();
    *layouts.borrow_mut() = Layouts {
        names: all.iter().map(|l| l.to_string()).collect(),
        active: all.iter().position(|l| *l == name).unwrap_or(0),
    };
    let (language, rest) = split_layout(name);
    let tooltip = if all.len() > 1 {
        format!(
            "{}\n{}\n{}",
            tr_args("keyboard-next", &[("name", text(name))]),
            all.join("\n"),
            tr("keyboard-tooltip")
        )
    } else {
        name.to_string()
    };
    // Full: the other layouts under the active one.
    let mut lines = vec![language, rest];
    lines.extend(all.iter().copied().filter(|l| *l != name).take(2));
    row.show("\u{f030c}", &lines, &tooltip); // 󰌌
}

/// "English (US, alt. intl.)" -> ("English", "US, alt. intl.").
fn split_layout(name: &str) -> (&str, &str) {
    match name.split_once(" (") {
        Some((language, rest)) => (language, rest.trim_end_matches(')')),
        None => (name, ""),
    }
}

// ------------------------------------------------------------- battery

const UPOWER: &str = "org.freedesktop.UPower";
const DISPLAY: &str = "/org/freedesktop/UPower/devices/DisplayDevice";

/// `modules/battery.yaml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BatteryConfig {
    /// Opened (or focused) by a right click; nothing when empty.
    pub manager: String,
}

/// power-profiles-daemon, by its new name and its old one.
const PROFILES: [(&str, &str); 2] = [
    ("org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles"),
    ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles"),
];

pub fn battery(config: &BatteryConfig, form: Form) -> gtk::Widget {
    let row = Row::new("battery", form);
    let conn = system_bus();
    // What the tooltip says, for the quick part.
    let details: Rc<RefCell<String>> = Rc::default();
    let refresh = {
        let (row, conn, details) = (row.clone(), conn.clone(), details.clone());
        move || *details.borrow_mut() = show_battery(&row, conn.as_ref())
    };
    refresh();
    if let Some(conn) = &conn {
        let sub = on_changes(conn, UPOWER, Some(DISPLAY), refresh.clone());
        hold(&row.root, sub);
    }
    // The uptime moves on its own: once a minute is enough for "5h 58min".
    glib::timeout_add_seconds_local(60, move || {
        refresh();
        glib::ControlFlow::Continue
    });

    let anchor = row.root.downgrade();
    let manager = config.manager.clone();
    clicks(
        &row.root,
        move || {
            if let Some(anchor) = anchor.upgrade() {
                battery_quick(&details.borrow(), conn.as_ref(), &anchor);
            }
        },
        move || {
            if !manager.is_empty() {
                open_or_focus(&manager);
            }
        },
        None,
    );
    row.root.clone()
}

/// The quick part: the details, and the power profile when
/// power-profiles-daemon runs.
fn battery_quick(details: &str, conn: Option<&gio::DBusConnection>, anchor: &gtk::Widget) {
    let list = quick_list("battery-quick");
    for line in details.lines() {
        list.append(&gtk::Label::builder().label(line).xalign(0.0).build());
    }
    let daemon = conn.and_then(|conn| {
        PROFILES.iter().find_map(|(name, path)| {
            let active = property(conn, name, path, name, "ActiveProfile")?.str()?.to_owned();
            let all = property(conn, name, path, name, "Profiles")?;
            let profiles: Vec<String> = (0..all.n_children())
                .filter_map(|i| glib::VariantDict::new(Some(&all.child_value(i))).lookup::<String>("Profile").ok().flatten())
                .collect();
            Some((conn.clone(), *name, *path, active, profiles))
        })
    });
    if let Some((conn, name, path, active, profiles)) = daemon {
        quick_title(&tr("battery-profile"), &list);
        for profile in profiles {
            let label = match profile.as_str() {
                "power-saver" => tr("battery-profile-saver"),
                "balanced" => tr("battery-profile-balanced"),
                "performance" => tr("battery-profile-performance"),
                other => other.to_string(),
            };
            let button = quick_button(&label, &list, {
                let (conn, profile) = (conn.clone(), profile.clone());
                move || {
                    let value = glib::Variant::from_variant(&profile.to_variant());
                    conn.call(
                        Some(name),
                        path,
                        "org.freedesktop.DBus.Properties",
                        "Set",
                        Some(&(name, "ActiveProfile", value).to_variant()),
                        None,
                        gio::DBusCallFlags::NONE,
                        5000,
                        gio::Cancellable::NONE,
                        |r| {
                            if let Err(e) = r {
                                log::warn!("battery: power profile: {e}");
                            }
                        },
                    );
                }
            });
            if profile == active {
                button.add_css_class("active");
            }
        }
    }
    popup(&list, anchor);
}

/// Shows the battery; returns what the tooltip says, without the hint.
fn show_battery(row: &Row, conn: Option<&gio::DBusConnection>) -> String {
    let get = |name: &str| {
        conn.and_then(|c| property(c, UPOWER, DISPLAY, "org.freedesktop.UPower.Device", name))
    };
    let present = get("IsPresent")
        .and_then(|v| v.get::<bool>())
        .unwrap_or(false)
        && get("Type").and_then(|v| v.get::<u32>()) == Some(2); // 2 = battery
    let uptime = uptime();
    let hint = tr("battery-hint");
    if !present {
        let details = tr_args("battery-uptime", &[("uptime", text(uptime.clone()))]);
        row.show(
            "\u{f06a5}",
            &[&tr("battery-none"), &uptime],
            &format!("{details}\n{hint}"),
        ); // 󰚥
        return details;
    }
    let percent = get("Percentage")
        .and_then(|v| v.get::<f64>())
        .unwrap_or_default();
    // UPower states: 1 charging, 2 discharging, 4 fully charged, 5 pending charge.
    let state = get("State")
        .and_then(|v| v.get::<u32>())
        .unwrap_or_default();
    let charging = matches!(state, 1 | 4);
    let glyph = match percent {
        _ if charging => "\u{f0084}",  // 󰂄
        p if p >= 90.0 => "\u{f0079}", // 󰁹
        p if p >= 70.0 => "\u{f0080}", // 󰂀
        p if p >= 45.0 => "\u{f007e}", // 󰁾
        p if p >= 20.0 => "\u{f007b}", // 󰁻
        _ => "\u{f007a}",              // 󰁺
    };
    let state_name = match state {
        1 => tr("battery-charging"),
        2 => tr("battery-discharging"),
        4 => tr("battery-full"),
        5 => tr("battery-pending"),
        _ => String::new(),
    };
    let remaining = match state {
        1 => get("TimeToFull"),
        2 => get("TimeToEmpty"),
        _ => None,
    }
    .and_then(|v| v.get::<i64>())
    .filter(|s| *s > 0)
    .map(|s| tr_args("battery-remaining", &[("time", text(duration(s as u64)))]))
    .unwrap_or_default();
    let detail = [state_name.as_str(), remaining.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let details = format!(
        "{}\n{}",
        tr_args(
            "battery-tooltip",
            &[
                ("percent", number(percent.round())),
                ("state", text(state_name)),
                ("remaining", text(remaining)),
            ],
        ),
        tr_args("battery-uptime", &[("uptime", text(uptime.clone()))])
    );
    row.show(
        glyph,
        &[&format!("{percent:.0}%"), &uptime, &detail],
        &format!("{details}\n{hint}"),
    );
    details
}

fn uptime() -> String {
    let seconds = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|t| t.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or_default();
    duration(seconds as u64)
}

/// 5h 58min, 2d 3h, 12min.
fn duration(seconds: u64) -> String {
    let (days, hours, minutes) = (seconds / 86_400, seconds / 3600 % 24, seconds / 60 % 60);
    match (days, hours) {
        (0, 0) => format!("{minutes}min"),
        (0, h) => format!("{h}h {minutes}min"),
        (d, h) => format!("{d}d {h}h"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_names_split_into_language_and_rest() {
        assert_eq!(
            split_layout("English (US, alt. intl.)"),
            ("English", "US, alt. intl.")
        );
        assert_eq!(
            split_layout("Portuguese (Brazil)"),
            ("Portuguese", "Brazil")
        );
        assert_eq!(split_layout("English"), ("English", ""));
    }

    #[test]
    fn durations() {
        assert_eq!(duration(12 * 60), "12min");
        assert_eq!(duration(5 * 3600 + 58 * 60), "5h 58min");
        assert_eq!(duration(2 * 86_400 + 3 * 3600 + 5), "2d 3h");
    }
}
