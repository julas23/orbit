//! The card shown after a capture: a thumbnail, where it was saved, and
//! buttons to open it or save a copy elsewhere.
//!
//! A notification of our own rather than the desktop's: notification
//! daemons such as dunst show actions in a menu, not as buttons. It sits in
//! the top-right corner of the focused screen and closes by itself after a
//! few seconds — unless the pointer is on it.

use std::{cell::Cell, path::PathBuf, rc::Rc, time::Duration};

use gtk::{gdk, gdk_pixbuf, gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use orbit_app::i18n::{text, tr, tr_args};

use crate::image::Image;

const SHOWN_FOR: Duration = Duration::from_secs(8);

/// Shows the card for the capture saved at `path`.
pub fn saved(app: &gtk::Application, monitor: Option<&gdk::Monitor>, image: &Image, path: PathBuf) {
    let window = new_window(app, monitor);
    let title = gtk::Label::builder()
        .label(tr("blink-saved"))
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["title"])
        .build();
    let close = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .css_classes(["close"])
        .build();
    let header = gtk::Box::builder().spacing(10).build();
    header.append(&title);
    header.append(&close);

    let picture = gtk::Picture::for_paintable(&gdk::Texture::for_pixbuf(&thumbnail(image)));
    picture.set_can_shrink(false);
    let holder = gtk::Box::builder().halign(gtk::Align::Center).build();
    holder.append(&picture);

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let file = gtk::Label::builder()
        .label(name.as_str())
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .max_width_chars(1)
        .css_classes(["file"])
        .build();
    let place = gtk::Label::builder()
        .label(tr_args(
            "blink-saved-in",
            &[("size", text(format!("{} × {}", image.width, image.height)))],
        ))
        .xalign(0.0)
        .css_classes(["meta"])
        .build();

    let open = gtk::Button::with_label(&tr("blink-open"));
    let save_as = gtk::Button::with_label(&tr("blink-save-as"));
    let buttons = gtk::Box::builder().spacing(5).homogeneous(true).build();
    buttons.append(&open);
    buttons.append(&save_as);

    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(["card"])
        .build();
    card.append(&header);
    card.append(&holder);
    card.append(&file);
    card.append(&place);
    card.append(&buttons);
    window.set_child(Some(&card));

    let busy = Rc::new(Cell::new(false));
    close_later(&window, &busy);
    {
        let window = window.clone();
        close.connect_clicked(move |_| window.destroy());
    }
    {
        let window = window.clone();
        let path = path.clone();
        open.connect_clicked(move |_| {
            let uri = gio::File::for_path(&path).uri();
            if let Err(err) =
                gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
            {
                log::warn!("opening {uri}: {err}");
            }
            window.destroy();
        });
    }
    {
        let window = window.clone();
        save_as.connect_clicked(move |_| {
            busy.set(true);
            // Out of the way: it sits over the dialog's corner.
            window.set_visible(false);
            let dialog = gtk::FileDialog::builder()
                .title(tr("blink-save-as"))
                .initial_name(name.as_str())
                // Where the captures are, not wherever this was started.
                .initial_folder(&gio::File::for_path(glib::home_dir()))
                .modal(false)
                .build();
            let window = window.clone();
            let source = path.clone();
            dialog.save(
                None::<&gtk::Window>,
                None::<&gio::Cancellable>,
                move |result| {
                    // Onto itself, a copy would empty the file: nothing to do.
                    if let Ok(target) = result
                        && let Some(target) = target.path()
                        && !same_file(&source, &target)
                        && let Err(err) = std::fs::copy(&source, &target)
                    {
                        log::warn!("saving {}: {err}", target.display());
                    }
                    window.destroy();
                },
            );
        });
    }
    window.present();
}

/// Shows the card with an error.
pub fn failed(app: &gtk::Application, monitor: Option<&gdk::Monitor>, message: &str) {
    let window = new_window(app, monitor);
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(5)
        .css_classes(["card", "failed"])
        .build();
    card.append(
        &gtk::Label::builder()
            .label(tr("blink-failed"))
            .xalign(0.0)
            .css_classes(["title"])
            .build(),
    );
    card.append(
        &gtk::Label::builder()
            .label(message)
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(40)
            .css_classes(["meta"])
            .build(),
    );
    window.set_child(Some(&card));
    close_later(&window, &Rc::new(Cell::new(false)));
    window.present();
}

fn new_window(app: &gtk::Application, monitor: Option<&gdk::Monitor>) -> gtk::Window {
    let window = gtk::Window::builder()
        .application(app)
        .css_classes(["orbit-blink-card"])
        .decorated(false)
        .build();
    window.init_layer_shell();
    window.set_namespace(Some("orbit-blink"));
    window.set_layer(Layer::Top);
    window.set_monitor(monitor);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Right, true);
    window.set_margin(Edge::Top, 10);
    window.set_margin(Edge::Right, 10);
    window.set_keyboard_mode(KeyboardMode::None);
    window
}

/// Closes `window` after a while, waiting while the pointer is on it or
/// while `busy` (a dialog is open).
fn close_later(window: &gtk::Window, busy: &Rc<Cell<bool>>) {
    let inside = Rc::new(Cell::new(false));
    let motion = gtk::EventControllerMotion::new();
    {
        let inside = inside.clone();
        motion.connect_enter(move |_, _, _| inside.set(true));
    }
    {
        let inside = inside.clone();
        motion.connect_leave(move |_| inside.set(false));
    }
    window.add_controller(motion);
    let weak = window.downgrade();
    let busy = busy.clone();
    let mut left = SHOWN_FOR;
    let tick = Duration::from_millis(500);
    glib::timeout_add_local(tick, move || {
        let Some(window) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if inside.get() || busy.get() {
            left = SHOWN_FOR;
        } else if left <= tick {
            window.destroy();
            return glib::ControlFlow::Break;
        } else {
            left -= tick;
        }
        glib::ControlFlow::Continue
    });
}

fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// At most 320 × 180.
fn thumbnail(image: &Image) -> gdk_pixbuf::Pixbuf {
    let pixbuf = image.pixbuf();
    let scale = (320.0 / f64::from(image.width))
        .min(180.0 / f64::from(image.height))
        .min(1.0);
    let (w, h) = (
        ((f64::from(image.width) * scale) as i32).max(1),
        ((f64::from(image.height) * scale) as i32).max(1),
    );
    pixbuf
        .scale_simple(w, h, gdk_pixbuf::InterpType::Bilinear)
        .unwrap_or(pixbuf)
}
