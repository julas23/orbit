//! Confirmation dialog over a blurred, darkened screen — a yes-or-no
//! question ([`ask`]) or a choice among a few actions ([`choose`]).
//!
//! Every monitor is photographed with `grim` before anything is shown, and
//! each photo goes on a layer-shell OVERLAY surface covering that monitor —
//! panel and dock included — blurred and darkened by CSS (`.confirm-backdrop`),
//! so the GPU does the blur. The dialog sits on the focused monitor. Focus
//! starts on "no", so a stray Enter does not take the machine down; Escape
//! answers "no" too.
//!
//! Without layer-shell (X11) or without `grim`, the screen is only darkened.

use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::sway;

pub struct Question<'a> {
    pub text: &'a str,
    pub yes: &'a str,
    pub no: &'a str,
}

/// One of the actions offered by [`choose`].
pub struct Choice<'a> {
    /// A Nerd Font glyph, shown large.
    pub glyph: &'a str,
    pub label: &'a str,
    /// CSS class of its button (`poweroff`…).
    pub class: &'a str,
}

type Answer<T> = Rc<RefCell<Option<Box<dyn FnOnce(T)>>>>;
type Windows = Rc<RefCell<Vec<gtk::Window>>>;

/// Asks `question`; `answer` runs once, with `true` for "yes".
pub fn ask(question: Question, answer: impl FnOnce(bool) + 'static) {
    let answer: Answer<bool> = Rc::new(RefCell::new(Some(Box::new(answer))));
    let windows: Windows = Rc::default();
    let respond = {
        let windows = windows.clone();
        Rc::new(move |yes: bool| {
            close(&windows);
            if let Some(answer) = answer.borrow_mut().take() {
                answer(yes);
            }
        })
    };
    let (dialog, no) = dialog(&question, respond.clone());
    cover(&windows, dialog.upcast(), no.upcast(), Rc::new(move || respond(false)));
}

/// Offers `choices` under `title`, with a cancel button labelled `cancel`;
/// `answer` runs once, with the index chosen or `None`.
pub fn choose(
    title: &str,
    choices: &[Choice],
    cancel: &str,
    answer: impl FnOnce(Option<usize>) + 'static,
) {
    let answer: Answer<Option<usize>> = Rc::new(RefCell::new(Some(Box::new(answer))));
    let windows: Windows = Rc::default();
    let respond = {
        let windows = windows.clone();
        Rc::new(move |choice: Option<usize>| {
            close(&windows);
            if let Some(answer) = answer.borrow_mut().take() {
                answer(choice);
            }
        })
    };

    let row = gtk::Box::builder()
        .spacing(20)
        .homogeneous(true)
        .halign(gtk::Align::Center)
        .build();
    for (i, choice) in choices.iter().enumerate() {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .build();
        content.append(
            &gtk::Label::builder()
                .label(choice.glyph)
                .css_classes(["glyph"])
                .build(),
        );
        content.append(&gtk::Label::new(Some(choice.label)));
        let button = gtk::Button::builder()
            .child(&content)
            .css_classes(["choice", choice.class])
            .build();
        let respond = respond.clone();
        button.connect_clicked(move |_| respond(Some(i)));
        row.append(&button);
    }
    let cancel = gtk::Button::builder()
        .label(cancel)
        .halign(gtk::Align::Center)
        .css_classes(["cancel"])
        .build();
    {
        let respond = respond.clone();
        cancel.connect_clicked(move |_| respond(None));
    }
    let dialog = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["confirm-box", "choose"])
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    dialog.append(&gtk::Label::builder().label(title).css_classes(["question"]).build());
    dialog.append(&row);
    dialog.append(&cancel);
    cover(&windows, dialog.upcast(), cancel.upcast(), Rc::new(move || respond(None)));
}

fn close(windows: &Windows) {
    for window in windows.borrow_mut().drain(..) {
        window.destroy();
    }
}

/// Covers every monitor with its blurred photo and shows `dialog` on the
/// focused one, `focus` focused (the harmless answer); Escape runs `escape`.
fn cover(windows: &Windows, dialog: gtk::Widget, focus: gtk::Widget, escape: Rc<dyn Fn()>) {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let monitors: Vec<gdk::Monitor> = display
        .monitors()
        .iter::<gdk::Monitor>()
        .filter_map(Result::ok)
        .collect();
    let layered = gtk4_layer_shell::is_supported();
    let target = focused_monitor(&monitors);

    // Photograph every monitor before any overlay appears.
    let photos: Vec<Option<gdk::Texture>> = monitors
        .iter()
        .map(|m| if layered { photo(m) } else { None })
        .collect();

    let mut dialog = Some(dialog);
    for (i, monitor) in monitors.iter().enumerate() {
        let is_target = i == target;
        if !layered && !is_target {
            continue;
        }
        let window = gtk::Window::builder()
            .css_classes(["orbit", "confirm"])
            .decorated(false)
            .build();

        let overlay = gtk::Overlay::new();
        let backdrop: gtk::Widget = match &photos[i] {
            Some(texture) => gtk::Picture::builder()
                .paintable(texture)
                .content_fit(gtk::ContentFit::Fill)
                .css_classes(["confirm-backdrop"])
                .build()
                .upcast(),
            None => gtk::Box::builder()
                .css_classes(["confirm-dim"])
                .hexpand(true)
                .vexpand(true)
                .build()
                .upcast(),
        };
        overlay.set_child(Some(&backdrop));

        if is_target && let Some(dialog) = dialog.take() {
            overlay.add_overlay(&dialog);
            let keys = gtk::EventControllerKey::new();
            let escape = escape.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Escape {
                    escape();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            window.add_controller(keys);
            GtkWindowExt::set_focus(&window, Some(&focus));
        }
        window.set_child(Some(&overlay));

        if layered {
            window.init_layer_shell();
            window.set_layer(Layer::Overlay);
            window.set_namespace(Some("orbit-confirm"));
            window.set_monitor(Some(monitor));
            for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                window.set_anchor(edge, true);
            }
            window.set_exclusive_zone(-1);
            if is_target {
                window.set_keyboard_mode(KeyboardMode::Exclusive);
            }
        } else {
            window.set_modal(true);
        }
        window.present();
        windows.borrow_mut().push(window);
    }
}

/// The question and its two buttons; returns the "no" button for the focus.
fn dialog(question: &Question, respond: Rc<impl Fn(bool) + 'static>) -> (gtk::Box, gtk::Button) {
    let text = gtk::Label::builder()
        .label(question.text)
        .css_classes(["question"])
        .build();
    let no = gtk::Button::builder().label(question.no).build();
    let yes = gtk::Button::builder()
        .label(question.yes)
        .css_classes(["yes"])
        .build();
    {
        let respond = respond.clone();
        no.connect_clicked(move |_| respond(false));
    }
    yes.connect_clicked(move |_| respond(true));

    let buttons = gtk::Box::builder()
        .spacing(10)
        .halign(gtk::Align::End)
        .build();
    buttons.append(&no);
    buttons.append(&yes);

    let dialog = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["confirm-box"])
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    dialog.append(&text);
    dialog.append(&buttons);
    (dialog, no)
}

/// Index of the monitor sway has focused, or 0.
fn focused_monitor(monitors: &[gdk::Monitor]) -> usize {
    let focused = sway::Connection::open()
        .and_then(|mut c| c.query::<Vec<sway::Output>>(sway::GET_OUTPUTS))
        .ok()
        .and_then(|outputs| outputs.into_iter().find(|o| o.focused).map(|o| o.name));
    focused
        .and_then(|name| {
            monitors
                .iter()
                .position(|m| m.connector().as_deref() == Some(name.as_str()))
        })
        .unwrap_or(0)
}

/// A screenshot of `monitor`, or `None` without `grim`.
fn photo(monitor: &gdk::Monitor) -> Option<gdk::Texture> {
    let mut argv = vec!["grim", "-t", "png", "-l", "0"];
    let connector = monitor.connector();
    if let Some(name) = connector.as_deref() {
        argv.extend(["-o", name]);
    }
    argv.push("-");
    let process = gio::Subprocess::newv(
        &argv.iter().map(std::ffi::OsStr::new).collect::<Vec<_>>(),
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE,
    )
    .inspect_err(|err| log::warn!("confirm: no screenshot ({err}); dimming only"))
    .ok()?;
    let (png, _) = process.communicate(None, gio::Cancellable::NONE).ok()?;
    gdk::Texture::from_bytes(&png?).ok()
}
