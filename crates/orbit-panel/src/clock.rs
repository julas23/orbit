//! Clock card: large `HH:MM`, small seconds beside it, date underneath.

use gtk::{glib, prelude::*};

pub fn new() -> gtk::Box {
    let time = gtk::Label::builder().css_classes(["time"]).build();
    let seconds = gtk::Label::builder()
        .css_classes(["seconds"])
        .valign(gtk::Align::End)
        .build();
    let date = gtk::Label::builder().css_classes(["date"]).build();

    let row = gtk::Box::builder()
        .spacing(2)
        .halign(gtk::Align::Center)
        .build();
    row.append(&time);
    row.append(&seconds);

    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["card", "clock"])
        .build();
    card.append(&row);
    card.append(&date);

    let update = move || {
        let Ok(now) = glib::DateTime::now_local() else {
            return;
        };
        let format = |f: &str| now.format(f).map(|s| s.to_string()).unwrap_or_default();
        time.set_label(&format("%H:%M"));
        seconds.set_label(&format("%S"));
        date.set_label(&format("%a %b %d"));
    };
    update();
    glib::timeout_add_seconds_local(1, move || {
        update();
        glib::ControlFlow::Continue
    });

    card
}
