//! F1: the keys, in a window of their own. Esc or F1 closes it.

use gtk::{gdk, glib, prelude::*};
use orbit_app::i18n::tr;

/// Each key and the message that says what it does.
const KEYS: &[(&str, &str)] = &[
    ("F1", "files-help-help"),
    ("F2", "files-help-rename"),
    ("F3, Ctrl+F, /", "files-help-filter"),
    ("F4", "files-help-edit"),
    ("F5", "files-help-copy"),
    ("F6", "files-help-move"),
    ("F7", "files-help-new-folder"),
    ("F8, Delete", "files-help-trash"),
    ("Shift+F8, Shift+Delete", "files-help-delete"),
    ("F9", "files-help-view"),
    ("F10", "files-help-split"),
    ("F11", "files-help-fullscreen"),
    ("F12", "files-help-terminal"),
    ("Ctrl+click, Ctrl+Space", "files-help-toggle"),
    ("Shift+click, Shift+↑ ↓", "files-help-range"),
    ("Space", "files-help-mark"),
    ("Ctrl+A", "files-help-all"),
    ("Ctrl+I", "files-help-invert"),
    ("Esc", "files-help-clear"),
    ("Ctrl+C X V", "files-help-clipboard"),
    ("Ctrl+R", "files-help-reload"),
    ("Ctrl+H", "files-help-hidden"),
    ("Ctrl+L", "files-help-path"),
    ("Ctrl+1 2 3", "files-help-views"),
    ("Ctrl+T, Ctrl+W", "files-help-tabs"),
    ("Tab", "files-help-other-pane"),
    ("Alt+← → ↑", "files-help-history"),
    ("j k h l, gg G", "files-help-vim"),
];

pub fn show(parent: &gtk::Window) {
    let grid = gtk::Grid::builder().column_spacing(20).row_spacing(5).css_classes(["help-keys"]).build();
    for (row, (key, message)) in KEYS.iter().enumerate() {
        let row = row as i32;
        grid.attach(&gtk::Label::builder().label(*key).xalign(0.0).css_classes(["help-key"]).build(), 0, row, 1, 1);
        grid.attach(&gtk::Label::builder().label(tr(message)).xalign(0.0).build(), 1, row, 1, 1);
    }
    let window = gtk::Window::builder()
        .transient_for(parent)
        .css_classes(["orbit-files-dialog"])
        .modal(true)
        .title(tr("files-help-title"))
        .resizable(false)
        .child(&crate::actions::padded("dialog-box", &grid))
        .build();
    let keys = gtk::EventControllerKey::new();
    let w = window.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        if matches!(key, gdk::Key::Escape | gdk::Key::F1)
            && let Some(w) = w.upgrade()
        {
            w.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    window.present();
}
