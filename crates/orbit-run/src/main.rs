//! Orbit-Run: a line over the screen where a command is typed and run
//! (Mod+R in orbit-core). Enter runs it through `sh -c`, detached, and
//! closes; a program that does not exist is said and the line stays. Tab
//! completes the program from $PATH, then files; Up and Down walk the
//! history (`line`). Esc, a click beside the card, or Mod+R again close it.
//!
//!   orbit-run [-c|--config DIR]

mod line;

use std::{
    cell::{Cell, RefCell},
    os::unix::process::CommandExt,
    path::PathBuf,
    rc::Rc,
};

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use orbit_app::i18n::{tr, tr_args};
use orbit_common::{ConfigDir, style::Style};

fn main() -> glib::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let config = match config_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("orbit-run: {err}");
            return glib::ExitCode::FAILURE;
        }
    };
    // Unique: started again while open (Mod+R twice), it closes.
    let app = gtk::Application::builder().application_id("io.github.julas23.Orbit.Run").build();
    let keep = Rc::new(RefCell::new(None));
    app.connect_activate(move |app| {
        if let Some(window) = app.active_window() {
            window.close();
            return;
        }
        let Some(display) = gdk::Display::default() else {
            log::error!("no display");
            app.quit();
            return;
        };
        let dir = ConfigDir::new(&config);
        let style = Style::install(&display, include_str!("../../../data/run.css"), &dir.style_path());
        let theme = orbit_app::theme::install(&display);
        *keep.borrow_mut() = Some((style, theme));
        show(app);
    });
    app.run_with_args::<&str>(&[])
}

fn show(app: &gtk::Application) {
    let window = gtk::Window::builder().application(app).title(tr("run-title")).css_classes(["orbit-run"]).build();
    if gtk4_layer_shell::is_supported() {
        window.init_layer_shell();
        window.set_namespace(Some("orbit-run"));
        window.set_layer(Layer::Overlay);
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(edge, true);
        }
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
    } else {
        window.set_resizable(false);
    }

    let icon = gtk::Image::builder().icon_name("system-run-symbolic").pixel_size(20).css_classes(["icon"]).build();
    let entry = gtk::Entry::builder().placeholder_text(tr("run-placeholder")).hexpand(true).css_classes(["command"]).build();
    let field = gtk::Box::builder().spacing(10).css_classes(["field"]).build();
    field.append(&icon);
    field.append(&entry);
    let message = gtk::Label::builder()
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .visible(false)
        .css_classes(["message"])
        .build();
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Start)
        .css_classes(["card"])
        .build();
    card.append(&field);
    card.append(&message);
    window.set_child(Some(&card));

    let programs = Rc::new(line::programs());
    let history = Rc::new(RefCell::new(line::History::load()));
    // Where Up and Down are in the history (its length: the line typed).
    let at = Rc::new(Cell::new(history.borrow().lines.len()));
    let typed = Rc::new(RefCell::new(String::new()));

    let say = {
        let message = message.clone();
        move |text: &str, error: bool| {
            message.set_label(text);
            message.set_visible(!text.is_empty());
            if error {
                message.add_css_class("error");
            } else {
                message.remove_css_class("error");
            }
        }
    };

    entry.connect_activate({
        let (window, programs, history, say) = (window.clone(), programs.clone(), history.clone(), say.clone());
        move |entry| {
            let text = entry.text().trim().to_owned();
            let Some(program) = line::program(&text) else { return };
            if !line::exists(program, &programs) {
                say(&tr_args("run-not-found", &[("program", program.to_owned().into())]), true);
                return;
            }
            match run(&text) {
                Ok(()) => {
                    history.borrow_mut().push(&text);
                    window.close();
                }
                Err(err) => say(&err.to_string(), true),
            }
        }
    });
    entry.connect_changed({
        let say = say.clone();
        move |_| say("", false)
    });

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let (window, entry, say) = (window.clone(), entry.clone(), say.clone());
        move |_, key, _, _| {
            let walk = |to: usize| {
                let lines = &history.borrow().lines;
                if at.get() == lines.len() {
                    *typed.borrow_mut() = entry.text().into();
                }
                at.set(to);
                let text = lines.get(to).cloned().unwrap_or_else(|| typed.borrow().clone());
                entry.set_text(&text);
                entry.set_position(-1);
            };
            match key {
                gdk::Key::Escape => window.close(),
                gdk::Key::Tab => {
                    let (done, shown) = line::complete(&entry.text(), &programs);
                    entry.set_text(&done);
                    entry.set_position(-1);
                    say(&shown.join("  "), false);
                }
                gdk::Key::Up if at.get() > 0 => walk(at.get() - 1),
                gdk::Key::Down if at.get() < history.borrow().lines.len() => walk(at.get() + 1),
                gdk::Key::Up | gdk::Key::Down => {}
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    });
    window.add_controller(keys);

    // A click beside the card closes.
    let click = gtk::GestureClick::new();
    click.connect_pressed({
        let (window, card) = (window.clone(), card.clone());
        move |_, _, x, y| {
            let inside = card.compute_bounds(&window).is_some_and(|b| b.contains_point(&gtk::graphene::Point::new(x as f32, y as f32)));
            if !inside {
                window.close();
            }
        }
    });
    window.add_controller(click);

    window.present();
    entry.grab_focus();
}

/// Runs `line` through `sh -c`, in a process group of its own so it
/// outlives this window.
fn run(line: &str) -> std::io::Result<()> {
    std::process::Command::new("sh")
        .args(["-c", line])
        .stdin(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .map(drop)
}

fn config_dir() -> Result<PathBuf, String> {
    let mut args = std::env::args().skip(1);
    let mut dir = ConfigDir::default_path();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-c" | "--config" => dir = args.next().ok_or("--config needs a directory")?.into(),
            "-h" | "--help" => {
                println!("Usage: orbit-run [-c|--config DIR]  (type a command and run it)");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(dir)
}
