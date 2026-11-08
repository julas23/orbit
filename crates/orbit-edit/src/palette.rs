//! Ctrl+Shift+P: every command by a few letters of its name, with its
//! keys beside it — and the same list, of files, for the recent ones
//! (Ctrl+R).

use std::rc::Rc;

use gtk::{gdk, glib, prelude::*};

pub struct Command {
    pub label: String,
    pub keys: &'static str,
    pub run: Rc<dyn Fn()>,
}

impl Command {
    pub fn new(label: String, keys: &'static str, run: impl Fn() + 'static) -> Self {
        Self {
            label,
            keys,
            run: Rc::new(run),
        }
    }
}

pub fn open(parent: &gtk::Window, placeholder: &str, commands: Vec<Command>) {
    let window = gtk::Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .default_width(600)
        .default_height(420)
        .css_classes(["quick-open"])
        .build();
    let entry = gtk::SearchEntry::builder().placeholder_text(placeholder).build();
    let list = gtk::ListBox::builder().css_classes(["results"]).build();
    let scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(5)
        .css_classes(["quick-open-box"])
        .build();
    content.append(&entry);
    content.append(&scroll);
    window.set_child(Some(&content));

    let commands = Rc::new(commands);
    // The commands shown, by their index, in the list's order.
    let shown: Rc<std::cell::RefCell<Vec<usize>>> = Rc::default();
    let refresh = {
        let (commands, shown, list, entry) = (commands.clone(), shown.clone(), list.clone(), entry.clone());
        move || {
            while let Some(row) = list.first_child() {
                list.remove(&row);
            }
            let query = entry.text().to_lowercase();
            let mut scored: Vec<(i32, usize)> = commands
                .iter()
                .enumerate()
                .filter_map(|(n, c)| crate::sidebar::score(&query, &c.label).map(|s| (s, n)))
                .collect();
            // Best first; equal ones as listed.
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            let mut order = Vec::new();
            for (_, n) in scored.into_iter().take(80) {
                let c = &commands[n];
                let row = gtk::Box::builder().spacing(10).build();
                row.append(
                    &gtk::Label::builder()
                        .label(&c.label)
                        .xalign(0.0)
                        .hexpand(true)
                        .ellipsize(gtk::pango::EllipsizeMode::Start)
                        .build(),
                );
                if !c.keys.is_empty() {
                    row.append(&gtk::Label::builder().label(c.keys).css_classes(["keys"]).build());
                }
                list.append(&row);
                order.push(n);
            }
            *shown.borrow_mut() = order;
            if let Some(first) = list.row_at_index(0) {
                list.select_row(Some(&first));
            }
        }
    };
    let refresh = Rc::new(refresh);
    {
        let refresh = refresh.clone();
        entry.connect_search_changed(move |_| refresh());
    }
    let choose = {
        let (window, commands, shown) = (window.clone(), commands.clone(), shown.clone());
        move |index: i32| {
            let picked = shown.borrow().get(index as usize).copied();
            window.close();
            if let Some(n) = picked {
                let run = commands[n].run.clone();
                // After the window is gone, so focus is back where it was.
                glib::idle_add_local_once(move || run());
            }
        }
    };
    let choose = Rc::new(choose);
    {
        let choose = choose.clone();
        list.connect_row_activated(move |_, row| choose(row.index()));
    }
    {
        let (list, choose) = (list.clone(), choose.clone());
        entry.connect_activate(move |_| {
            if let Some(row) = list.selected_row() {
                choose(row.index());
            }
        });
    }
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (window, list) = (window.clone(), list.clone());
        keys.connect_key_pressed(move |_, key, _, _| {
            let step = match key {
                gdk::Key::Escape => {
                    window.close();
                    return glib::Propagation::Stop;
                }
                gdk::Key::Down => 1,
                gdk::Key::Up => -1,
                _ => return glib::Propagation::Proceed,
            };
            let at = list.selected_row().map_or(0, |r| r.index());
            if let Some(row) = list.row_at_index((at + step).max(0)) {
                list.select_row(Some(&row));
                row.grab_focus();
                GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
            }
            glib::Propagation::Stop
        });
    }
    window.add_controller(keys);
    refresh();
    window.present();
    entry.grab_focus();
}
