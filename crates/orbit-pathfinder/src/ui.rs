//! Small things the window and its dialogs share.

use gtk::{glib, prelude::*};

/// Runs `work` on a thread and hands its result to `done` on the main loop.
pub fn background<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(work());
    });
    glib::spawn_future_local(async move {
        if let Ok(value) = rx.recv().await {
            done(value);
        }
    });
}

/// Esc closes `window` — and so cancels it: nothing is saved but by its
/// own button.
pub fn close_on_escape(window: &gtk::Window) {
    // Seen before the focused field, which would otherwise take the key.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = window.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key != gtk::gdk::Key::Escape {
            return glib::Propagation::Proceed;
        }
        if let Some(window) = weak.upgrade() {
            window.close();
        }
        glib::Propagation::Stop
    });
    window.add_controller(keys);
}
