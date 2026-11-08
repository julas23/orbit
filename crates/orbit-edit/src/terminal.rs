//! Ctrl+`: a terminal under the tabs, the user's shell in the folder of
//! the sidebar (or the file's). It stays running while hidden; when the
//! shell ends, the next Ctrl+` starts another.

use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};
use vte4::prelude::*;

pub struct Terminal {
    pub widget: gtk::Revealer,
    terminal: RefCell<Option<vte4::Terminal>>,
    holder: gtk::Box,
    running: Rc<Cell<bool>>,
}

impl Terminal {
    pub fn new() -> Rc<Self> {
        let holder = gtk::Box::builder()
            .height_request(220)
            .css_classes(["terminal-panel"])
            .build();
        let widget = gtk::Revealer::builder()
            .child(&holder)
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .build();
        Rc::new(Self {
            widget,
            terminal: RefCell::default(),
            holder,
            running: Rc::new(Cell::new(false)),
        })
    }

    pub fn shown(&self) -> bool {
        self.widget.reveals_child()
    }

    /// Shows it (starting a shell in `cwd` if none runs), or hides it.
    pub fn toggle(&self, cwd: &Path, font: &str, size: f64) {
        if self.shown() {
            self.widget.set_reveal_child(false);
            return;
        }
        if !self.running.get() {
            self.start(cwd, font, size);
        }
        self.widget.set_reveal_child(true);
        if let Some(t) = self.terminal.borrow().as_ref() {
            t.grab_focus();
        }
    }

    fn start(&self, cwd: &Path, font: &str, size: f64) {
        if let Some(old) = self.terminal.borrow_mut().take() {
            self.holder.remove(&old);
        }
        let terminal = vte4::Terminal::builder()
            .hexpand(true)
            .vexpand(true)
            .scrollback_lines(10_000)
            .build();
        let mut description = gtk::pango::FontDescription::from_string(font);
        description.set_size((size * f64::from(gtk::pango::SCALE)) as i32);
        terminal.set_font(Some(&description));
        self.holder.append(&terminal);
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
        self.running.set(true);
        let running = self.running.clone();
        terminal.connect_child_exited(move |_, _| running.set(false));
        let running = self.running.clone();
        terminal.spawn_async(
            vte4::PtyFlags::DEFAULT,
            Some(&cwd.to_string_lossy()),
            &[&shell],
            &[],
            glib::SpawnFlags::SEARCH_PATH,
            || {},
            -1,
            None::<&gio::Cancellable>,
            move |result| {
                if let Err(err) = result {
                    log::warn!("terminal: {err}");
                    running.set(false);
                }
            },
        );
        *self.terminal.borrow_mut() = Some(terminal);
    }
}
