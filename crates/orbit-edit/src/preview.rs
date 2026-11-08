//! The Markdown preview beside the text: laid out as Orbit-Read does it
//! (`orbit-markdown`), again a moment after each change, the scroll kept.

use std::{cell::RefCell, path::Path, rc::Rc, time::Duration};

use gtk::{glib, prelude::*};

pub struct Preview {
    pub widget: gtk::ScrolledWindow,
    view: gtk::TextView,
    soon: RefCell<Option<glib::SourceId>>,
    /// Whether the minimap was shown before (the preview takes its place).
    pub map_was: bool,
}

impl Preview {
    pub fn new(map_was: bool) -> Rc<Self> {
        let view = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .wrap_mode(gtk::WrapMode::WordChar)
            .left_margin(20)
            .right_margin(20)
            .top_margin(15)
            .bottom_margin(40)
            .css_classes(["md-preview"])
            .build();
        let widget = gtk::ScrolledWindow::builder()
            .child(&view)
            .hexpand(true)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(300)
            .build();
        Rc::new(Self {
            widget,
            view,
            soon: RefCell::default(),
            map_was,
        })
    }

    pub fn update_soon(self: &Rc<Self>, text: &str, path: Option<&Path>) {
        if let Some(id) = self.soon.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let text = text.to_owned();
        let base = path
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(glib::home_dir);
        let id = glib::timeout_add_local_once(Duration::from_millis(250), move || {
            let Some(p) = weak.upgrade() else { return };
            p.soon.borrow_mut().take();
            let scroll = p.widget.vadjustment().value();
            orbit_markdown::render(&p.view, &text, &base);
            let adjustment = p.widget.vadjustment();
            glib::idle_add_local_once(move || adjustment.set_value(scroll));
        });
        *self.soon.borrow_mut() = Some(id);
    }
}
