//! A level control shared by the volume and brightness modules, after the
//! "controle" of the eww panel: a slider and an icon.
//!
//! Normal (2 × 1 cells): the clickable icon on the left and a vertical
//! slider (up = more) on the right. Full (4 × 1): the icon and a horizontal
//! slider. Mini (1 × 1) and the line form: only the icon; a left click opens
//! a horizontal slider in a popover, and a right click in Mini does what the
//! left click on the icon does in the larger forms (the mixer, for the
//! volume). In every form the mouse wheel over the module steps the level
//! and the tooltip has the level.

use std::{cell::Cell, rc::Rc};

use gtk::{glib, prelude::*};

use crate::board::Form;

pub struct Control {
    pub root: gtk::Widget,
    scale: gtk::Scale,
    icon: gtk::Label,
    /// Set while the panel itself moves the slider, so that the change is not
    /// sent back as if the user had made it.
    updating: Rc<Cell<bool>>,
}

type Callback = Box<dyn Fn()>;

/// What the control does when the user acts on it.
pub struct Actions {
    /// The slider moved to this percentage.
    pub set: Box<dyn Fn(u32)>,
    /// The wheel moved by this many steps (positive = up).
    pub step: Box<dyn Fn(i32)>,
    /// Left click on the icon (Normal and Full; in Mini and the line form
    /// the click opens the slider, and in Mini a right click does this).
    pub click: Option<Callback>,
    /// Right click on the icon (Normal, Full and the line form).
    pub right_click: Option<Callback>,
}

impl Control {
    pub fn new(class: &str, form: Form, actions: Actions) -> Self {
        let orientation = if form == Form::Normal {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        };
        let scale = gtk::Scale::with_range(orientation, 0.0, 100.0, 1.0);
        scale.set_draw_value(false);
        scale.set_hexpand(true);
        // A vertical scale grows downwards; turn it so that up is more.
        scale.set_inverted(form == Form::Normal);
        let icon = gtk::Label::builder()
            .css_classes(["icon"])
            .valign(gtk::Align::Center)
            .build();
        let updating = Rc::new(Cell::new(false));

        let actions = Rc::new(actions);
        {
            let actions = actions.clone();
            let updating = updating.clone();
            scale.connect_value_changed(move |scale| {
                if !updating.get() {
                    (actions.set)(scale.value().round() as u32);
                }
            });
        }

        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        {
            let actions = actions.clone();
            wheel.connect_scroll(move |_, _, dy| {
                (actions.step)(if dy < 0.0 { 1 } else { -1 });
                glib::Propagation::Stop
            });
        }

        let run = |action: fn(&Actions) -> &Option<Callback>| {
            let actions = actions.clone();
            move |_: &gtk::GestureClick, _: i32, _: f64, _: f64| {
                if let Some(f) = action(&actions) {
                    f();
                }
            }
        };
        let left = gtk::GestureClick::builder().button(1).build();
        let right = gtk::GestureClick::builder().button(3).build();
        icon.add_controller(left.clone());
        icon.add_controller(right.clone());

        let root: gtk::Widget = match form {
            Form::Normal | Form::Full => {
                let card = gtk::Box::builder()
                    .css_classes(["card", "control", class, form.class()])
                    .spacing(5)
                    .build();
                card.append(&icon);
                card.append(&scale);
                left.connect_released(run(|a| &a.click));
                right.connect_released(run(|a| &a.right_click));
                if actions.click.is_some() {
                    icon.set_cursor_from_name(Some("pointer"));
                }
                card.upcast()
            }
            Form::Mini | Form::Line => {
                let holder = gtk::Box::builder()
                    .css_classes(["control", class, "popup"])
                    .build();
                holder.append(&scale);
                let popover = gtk::Popover::builder().child(&holder).build();
                let classes = if form == Form::Mini {
                    vec!["card", "control", class, "mini"]
                } else {
                    vec!["control-line", class]
                };
                let root = gtk::Box::builder().css_classes(classes).build();
                icon.set_hexpand(true);
                root.append(&icon);
                popover.set_parent(&root);
                left.connect_released(move |_, _, _, _| popover.popup());
                if form == Form::Mini {
                    right.connect_released(run(|a| &a.click));
                } else {
                    right.connect_released(run(|a| &a.right_click));
                }
                icon.set_cursor_from_name(Some("pointer"));
                root.upcast()
            }
        };
        root.add_controller(wheel);

        Self {
            root,
            scale,
            icon,
            updating,
        }
    }

    /// Shows `percent` and `glyph` without echoing the change back.
    pub fn show(&self, percent: u32, glyph: &str, tooltip: &str) {
        self.updating.set(true);
        self.scale.set_value(f64::from(percent.min(100)));
        self.updating.set(false);
        self.icon.set_label(glyph);
        self.root.set_tooltip_text(Some(tooltip));
    }
}
