//! The centre: a column on the screen's right edge, the whole height, over
//! the windows. The notifications kept, grouped by app — the group heard
//! from last on top — with do-not-disturb and clear-all in the header.
//!
//! It is the application's window: launching `orbit-notify` again (or the
//! panel's bell) shows or hides it; Esc hides it.

use std::rc::Rc;

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use orbit_app::i18n::tr;

use crate::{
    card::{self, Handlers, Place},
    config::Config,
    note::Note,
};

/// The glyphs of the do-not-disturb switch (Nerd Font bell, bell-off).
pub const BELL: &str = "󰂚";
pub const BELL_OFF: &str = "󰂛";

pub struct Centre {
    pub window: gtk::ApplicationWindow,
    list: gtk::Box,
    dnd: gtk::ToggleButton,
    clear: gtk::Button,
    card: Rc<dyn Fn(u32) -> Handlers>,
    clear_group: Rc<dyn Fn(String)>,
}

/// What the centre asks of the daemon.
pub struct Actions {
    pub dnd: Box<dyn Fn(bool)>,
    pub clear: Box<dyn Fn()>,
    pub clear_group: Rc<dyn Fn(String)>,
    /// Handlers for the card of notification `id`.
    pub card: Rc<dyn Fn(u32) -> Handlers>,
}

impl Centre {
    pub fn new(app: &gtk::Application, config: &Config, monitor: Option<&gdk::Monitor>, actions: Actions) -> Rc<Centre> {
        let title = gtk::Label::builder().label(tr("notify-title")).xalign(0.0).hexpand(true).css_classes(["title"]).build();
        let dnd = gtk::ToggleButton::builder().label(BELL).tooltip_text(tr("notify-dnd")).css_classes(["dnd"]).build();
        let clear = gtk::Button::builder().label(tr("notify-clear")).css_classes(["clear"]).build();
        let header = gtk::Box::builder().spacing(5).css_classes(["header"]).build();
        header.append(&title);
        header.append(&dnd);
        header.append(&clear);

        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).css_classes(["groups"]).build();
        let scrolled = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&list).build();
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).css_classes(["centre"]).build();
        body.append(&header);
        body.append(&scrolled);

        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title(tr("notify-title"))
            .css_classes(["orbit", "orbit-notify-centre"])
            .decorated(false)
            .default_width(config.center_width)
            .child(&body)
            .build();
        window.init_layer_shell();
        window.set_namespace(Some("orbit-notify"));
        window.set_layer(Layer::Overlay);
        window.set_monitor(monitor);
        for edge in [Edge::Top, Edge::Right, Edge::Bottom] {
            window.set_anchor(edge, true);
        }
        window.set_margin(Edge::Top, config.margin);
        window.set_margin(Edge::Right, config.margin);
        window.set_margin(Edge::Bottom, config.margin);
        // Keys only when clicked into: Esc closes, and nothing is taken
        // from the window being typed in when it opens.
        window.set_keyboard_mode(KeyboardMode::OnDemand);
        let escape = gtk::EventControllerKey::new();
        escape.set_propagation_phase(gtk::PropagationPhase::Capture);
        escape.connect_key_pressed(glib::clone!(
            #[weak]
            window,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| {
                if key == gdk::Key::Escape {
                    window.set_visible(false);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        ));
        window.add_controller(escape);

        let Actions { dnd: on_dnd, clear: on_clear, clear_group, card } = actions;
        dnd.connect_toggled(move |b| on_dnd(b.is_active()));
        clear.connect_clicked(move |_| on_clear());
        Rc::new(Centre { window, list, dnd, clear, card, clear_group })
    }

    pub fn toggle(&self) {
        if self.window.is_visible() {
            self.window.set_visible(false);
        } else {
            self.window.present();
        }
    }

    pub fn set_dnd(&self, on: bool) {
        if self.dnd.is_active() != on {
            self.dnd.set_active(on);
        }
        self.dnd.set_label(if on { BELL_OFF } else { BELL });
    }

    /// Redraws the list from `notes` (oldest first).
    pub fn show(&self, notes: &[Note]) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        self.clear.set_sensitive(!notes.is_empty());
        if notes.is_empty() {
            self.list.append(&gtk::Label::builder().label(tr("notify-empty")).css_classes(["placeholder"]).vexpand(true).build());
            return;
        }
        // Groups in the order last heard from, newest first.
        let mut groups: Vec<(&str, Vec<&Note>)> = Vec::new();
        for note in notes.iter().rev() {
            match groups.iter_mut().find(|(g, _)| *g == note.group()) {
                Some((_, list)) => list.push(note),
                None => groups.push((note.group(), vec![note])),
            }
        }
        for (group, list) in groups {
            let first = list[0];
            let head = gtk::Box::builder().spacing(5).css_classes(["group"]).build();
            head.append(&card::icon(first, 16));
            head.append(&gtk::Label::builder().label(card::app_name(first)).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build());
            let close = gtk::Button::builder().label("×").css_classes(["close"]).tooltip_text(tr("notify-clear-group")).build();
            {
                let clear_group = self.clear_group.clone();
                let group = group.to_owned();
                close.connect_clicked(move |_| clear_group(group.clone()));
            }
            head.append(&close);
            self.list.append(&head);
            for note in list {
                self.list.append(&card::build(note, Place::Centre, &(self.card)(note.id)));
            }
        }
    }
}
