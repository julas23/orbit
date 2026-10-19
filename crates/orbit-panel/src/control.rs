//! A level control shared by the volume and brightness modules, after the
//! "controle" of the eww panel: a slider and an icon.
//!
//! Block form: a half-width card — the two controls sit side by side — with
//! the clickable icon on the left and a vertical slider (up = more) on the
//! right. Line form:
//! only the icon; its click opens a horizontal slider in a popover. In both
//! forms the mouse wheel over the icon steps the level.

use std::{cell::Cell, rc::Rc};

use gtk::{glib, prelude::*};

pub struct Control {
    pub root: gtk::Widget,
    scale: gtk::Scale,
    icon: gtk::Label,
    /// Set while the panel itself moves the slider, so that the change is not
    /// sent back as if the user had made it.
    updating: Rc<Cell<bool>>,
}

/// What the control does when the user acts on it.
pub struct Actions {
    /// The slider moved to this percentage.
    pub set: Box<dyn Fn(u32)>,
    /// The wheel moved by this many steps (positive = up).
    pub step: Box<dyn Fn(i32)>,
    /// Left click on the icon (block form only; in the line form the click
    /// opens the slider).
    pub click: Option<Box<dyn Fn()>>,
    /// Right click on the icon.
    pub right_click: Option<Box<dyn Fn()>>,
}

impl Control {
    pub fn new(class: &str, vertical: bool, actions: Actions) -> Self {
        let orientation = if vertical {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        };
        let scale = gtk::Scale::with_range(orientation, 0.0, 100.0, 1.0);
        scale.set_draw_value(false);
        scale.set_hexpand(true);
        // A vertical scale grows downwards; turn it so that up is more.
        scale.set_inverted(vertical);
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
        icon.add_controller(wheel);

        let right = gtk::GestureClick::builder().button(3).build();
        {
            let actions = actions.clone();
            right.connect_released(move |_, _, _, _| {
                if let Some(f) = &actions.right_click {
                    f();
                }
            });
        }
        icon.add_controller(right);

        let left = gtk::GestureClick::builder().button(1).build();
        icon.add_controller(left.clone());

        let root: gtk::Widget = if vertical {
            let card = gtk::Box::builder()
                .css_classes(["card", "control", class])
                .spacing(5)
                .build();
            card.append(&icon);
            card.append(&scale);
            {
                let actions = actions.clone();
                left.connect_released(move |_, _, _, _| {
                    if let Some(f) = &actions.click {
                        f();
                    }
                });
            }
            if actions.click.is_some() {
                icon.set_cursor_from_name(Some("pointer"));
            }
            card.upcast()
        } else {
            let holder = gtk::Box::builder()
                .css_classes(["control", class, "popup"])
                .build();
            holder.append(&scale);
            let popover = gtk::Popover::builder().child(&holder).build();
            let line = gtk::Box::builder()
                .css_classes(["control-line", class])
                .build();
            line.append(&icon);
            popover.set_parent(&line);
            left.connect_released(move |_, _, _, _| popover.popup());
            icon.set_cursor_from_name(Some("pointer"));
            line.upcast()
        };

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
