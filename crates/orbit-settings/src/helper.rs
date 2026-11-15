//! orbit-services-helper run from a page: its steps in a thread (pkexec
//! asks for the password), its events handed to the page as they come,
//! and the end said as an error or none.

use gtk::glib;
use orbit_app::i18n::tr;
use orbit_services::{Event, Privileged, Run};

/// Runs `steps`; `event` sees each event on the main loop, `end` the error
/// (`None`: all went through).
pub fn run(steps: Vec<Privileged>, event: impl Fn(&Event) + 'static, end: impl FnOnce(Option<String>) + 'static) {
    run_with(steps, None, event, end);
}

/// As [`run`], with `input` sent to the helper first, a line (a password a
/// step reads).
pub fn run_with(
    steps: Vec<Privileged>,
    input: Option<String>,
    event: impl Fn(&Event) + 'static,
    end: impl FnOnce(Option<String>) + 'static,
) {
    let (tx, rx) = async_channel::unbounded::<Result<Event, (bool, bool)>>();
    std::thread::spawn(move || {
        let mut run = match Run::spawn(&steps, false) {
            Ok(run) => run,
            Err(e) => {
                let _ = tx.send_blocking(Ok(Event::Error(format!("{}: {e}", orbit_services::HELPER))));
                let _ = tx.send_blocking(Err((true, false)));
                return;
            }
        };
        if let Some(line) = input {
            run.send(&line);
        }
        while let Some(event) = run.next_event() {
            let _ = tx.send_blocking(Ok(event));
        }
        let ended = run
            .wait()
            .map(|s| (!matches!(s.code(), Some(126 | 127)), s.success()))
            .unwrap_or((true, false));
        let _ = tx.send_blocking(Err(ended));
    });
    glib::spawn_future_local(async move {
        let mut error = None;
        while let Ok(msg) = rx.recv().await {
            match msg {
                Ok(Event::Error(e)) => {
                    event(&Event::Error(e.clone()));
                    error = Some(e);
                }
                Ok(e) => event(&e),
                Err((authorised, ok)) => {
                    end(if !authorised {
                        Some(tr("settings-svc-cancelled"))
                    } else if ok {
                        None
                    } else {
                        Some(error.unwrap_or_else(|| tr("settings-upd-failed")))
                    });
                    return;
                }
            }
        }
    });
}

/// A button installing `package` (one of orbit_services' EXTRA_PACKAGES)
/// through the helper; once `installed` says so, it hides and `done` runs.
pub fn install_button(
    package: &'static str,
    installed: fn() -> bool,
    message: &gtk::Label,
    done: impl Fn() + 'static,
) -> gtk::Button {
    use gtk::prelude::*;
    let button = gtk::Button::with_label(&tr("settings-a11y-install"));
    button.set_halign(gtk::Align::End);
    let message = message.clone();
    let done = std::rc::Rc::new(done);
    button.connect_clicked(move |b| {
        b.set_sensitive(false);
        crate::say(&message, &tr("settings-installing"));
        let (b, message, done) = (b.clone(), message.clone(), done.clone());
        run(vec![Privileged::Install(package.into())], |_| {}, move |error| {
            let ok = installed();
            b.set_visible(!ok);
            b.set_sensitive(true);
            crate::say(&message, &error.unwrap_or_default());
            if ok {
                done();
            }
        });
    });
    button
}
