//! One notification drawn as a card — the same in a bubble and in the
//! centre: icon, app and time, title, body, progress, action buttons.
//!
//! Left click is the notification's own click (its "default" action, or
//! opening its app); right click dismisses it; the × too.

use std::rc::Rc;

use gtk::{gio, glib, prelude::*};

use crate::note::Note;

/// What the card's clicks do.
#[derive(Clone)]
pub struct Handlers {
    pub activate: Rc<dyn Fn()>,
    pub dismiss: Rc<dyn Fn()>,
    pub action: Rc<dyn Fn(String)>,
}

/// Where the card goes; bubbles cut long bodies shorter.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Bubble,
    Centre,
}

pub fn build(note: &Note, place: Place, handlers: &Handlers) -> gtk::Box {
    let mut classes = vec!["note"];
    if note.urgency == 2 {
        classes.push("critical");
    }
    let card = gtk::Box::builder().spacing(10).css_classes(classes).build();
    card.append(&icon(note, 40));

    let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).build();
    let top = gtk::Box::builder().spacing(5).build();
    // In the centre the group's heading already names the app: the title
    // takes its place.
    let centre = place == Place::Centre;
    top.append(
        &gtk::Label::builder()
            .label(if centre { note.summary.clone() } else { app_name(note) })
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes([if centre { "summary" } else { "app" }])
            .build(),
    );
    top.append(&gtk::Label::builder().label(when(note.time)).css_classes(["time"]).build());
    let close = gtk::Button::builder().label("×").css_classes(["close"]).valign(gtk::Align::Start).build();
    {
        let dismiss = handlers.dismiss.clone();
        close.connect_clicked(move |_| dismiss());
    }
    top.append(&close);
    text.append(&top);

    if !centre && !note.summary.is_empty() {
        text.append(
            &gtk::Label::builder()
                .label(&note.summary)
                .xalign(0.0)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .max_width_chars(1)
                .lines(2)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["summary"])
                .build(),
        );
    }
    if !note.body.is_empty() {
        let body = gtk::Label::builder()
            .label(&note.body)
            .use_markup(true)
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            // A wrapping label asks for its whole text on one line: the
            // card's width is the config's, and the text wraps to it.
            .max_width_chars(1)
            .css_classes(["body"])
            .build();
        // Labels only cut by line count when they ellipsize.
        body.set_lines(if place == Place::Bubble { 4 } else { 8 });
        body.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&body);
    }
    if let Some(value) = note.value {
        let bar = gtk::ProgressBar::builder().fraction(value as f64 / 100.0).css_classes(["value"]).build();
        text.append(&bar);
    }
    // Buttons only while the program is there to hear them.
    if note.sender.is_some() {
        let buttons: Vec<_> = note.buttons().cloned().collect();
        if !buttons.is_empty() {
            let row = gtk::Box::builder().spacing(5).homogeneous(true).css_classes(["actions"]).build();
            for (key, label) in buttons {
                let button = gtk::Button::builder().label(label).build();
                let action = handlers.action.clone();
                button.connect_clicked(move |_| action(key.clone()));
                row.append(&button);
            }
            text.append(&row);
        }
    }
    card.append(&text);

    let click = gtk::GestureClick::builder().button(0).build();
    {
        let handlers = handlers.clone();
        click.connect_released(move |gesture, _, _, _| match gesture.current_button() {
            1 => (handlers.activate)(),
            2 | 3 => (handlers.dismiss)(),
            _ => {}
        });
    }
    card.add_controller(click);
    card
}

/// The notification's picture: the image it sent, its icon, its app's
/// icon, or a generic one.
pub fn icon(note: &Note, size: i32) -> gtk::Image {
    if let Some(texture) = &note.image {
        let image = gtk::Image::from_paintable(Some(texture));
        image.set_pixel_size(size);
        return image;
    }
    let spec = note.icon.strip_prefix("file://").map(|p| glib::Uri::unescape_string(p, None::<&str>).map(|s| s.to_string()).unwrap_or_else(|| p.to_owned()));
    let spec = spec.unwrap_or_else(|| note.icon.clone());
    if spec.starts_with('/') && std::path::Path::new(&spec).is_file() {
        return orbit_app::icon(&spec, size);
    }
    if !spec.is_empty() && !spec.contains('/') && has_icon(&spec) {
        return orbit_app::icon(&spec, size);
    }
    if let Some(info) = desktop_app(note)
        && let Some(gicon) = info.icon()
    {
        let image = gtk::Image::new();
        orbit_app::set_gicon(&image, &gicon, size);
        return image;
    }
    orbit_app::icon("dialog-information", size)
}

fn has_icon(name: &str) -> bool {
    gtk::gdk::Display::default().is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon(name))
}

/// The app that sent it, by its desktop file — or, without one, by its name.
pub fn desktop_app(note: &Note) -> Option<gio_unix::DesktopAppInfo> {
    if !note.desktop.is_empty()
        && let Some(info) = gio_unix::DesktopAppInfo::new(&format!("{}.desktop", note.desktop))
    {
        return Some(info);
    }
    // Many programs send only a name ("Firefox"): look it up among the apps.
    let name = note.app.to_lowercase();
    if name.is_empty() || name == "notify-send" {
        return None;
    }
    gio::AppInfo::all().into_iter().filter_map(|a| a.downcast::<gio_unix::DesktopAppInfo>().ok()).find(|a| {
        a.name().to_lowercase() == name || a.id().is_some_and(|id| id.to_lowercase().trim_end_matches(".desktop").ends_with(&name))
    })
}

/// The app's name as its menu entry has it ("Pathfinder", not the
/// program's "orbit-pathfinder"), or as it gave it.
pub fn app_name(note: &Note) -> String {
    if !note.desktop.is_empty()
        && let Some(info) = gio_unix::DesktopAppInfo::new(&format!("{}.desktop", note.desktop))
    {
        return info.name().to_string();
    }
    if !note.app.is_empty() {
        return note.app.clone();
    }
    desktop_app(note).map(|a| a.name().to_string()).unwrap_or_default()
}

/// "14:05" today, "20/10 14:05" before.
pub fn when(time: i64) -> String {
    let Ok(then) = glib::DateTime::from_unix_local(time) else { return String::new() };
    let today = glib::DateTime::now_local().ok().map(|n| n.ymd());
    let format = if today == Some(then.ymd()) { "%H:%M" } else { "%d/%m %H:%M" };
    then.format(format).map(|s| s.to_string()).unwrap_or_default()
}
