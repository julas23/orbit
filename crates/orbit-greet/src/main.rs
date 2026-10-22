//! Orbit-Greet: the face of orbit-core's locked state — login and screen
//! lock are one thing (PLANO-CORE §7). The core launches it on a
//! connection of its own and shows its window, full screen on the monitor
//! with the pointer, only while locked; the password checked by PAM
//! (`pam`), it asks the core to unlock (`link`).
//!
//! Around the card (§7.2): keyboard layout, network and battery in a
//! corner (`status`), power off, reboot and suspend below (`power`, with
//! its commands in `greet.yaml`).
//!
//! It stays running between locks: at each `locked` the field is emptied
//! and ready. Wrong passwords wait longer each time. Run anywhere else it
//! is a preview: a right password just quits.
//!
//!   orbit-greet [-c|--config DIR]

mod link;
mod pam;
mod power;
mod status;

use std::{cell::Cell, path::PathBuf, rc::Rc, time::Duration};

use gtk::{gdk, gio, glib, prelude::*};
use orbit_app::i18n::{number, tr, tr_args};
use orbit_common::{ConfigDir, style::Style};

use crate::link::{Event, Link};

/// Wrong passwords that cost no wait; after them, 2, 4, 8… up to 30 s.
const FREE_TRIES: u32 = 2;
const MAX_WAIT: u64 = 30;

fn main() -> glib::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let config = match config_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("orbit-greet: {err}");
            return glib::ExitCode::FAILURE;
        }
    };
    // Never unique: a greet the core launches again must not hand over to
    // one still going away and quit.
    let app = gtk::Application::builder()
        .application_id("io.github.julas23.Orbit.Greet")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let keep = Rc::new(std::cell::RefCell::new(None));
    app.connect_activate(move |app| {
        if !app.windows().is_empty() {
            return;
        }
        let Some(display) = gdk::Display::default() else {
            log::error!("no display");
            app.quit();
            return;
        };
        let dir = ConfigDir::new(&config);
        let style = Style::install(&display, include_str!("../../../data/greet.css"), &dir.style_path());
        let theme = orbit_app::theme::install(&display);
        *keep.borrow_mut() = Some((style, theme));
        let power = dir.app::<power::Config>("greet").unwrap_or_else(|err| {
            log::error!("greet.yaml: {err:#}");
            power::Config::default()
        });
        build(app, &display, power);
    });
    app.run_with_args::<&str>(&[])
}

fn config_dir() -> Result<PathBuf, String> {
    let mut args = std::env::args().skip(1);
    let mut dir = ConfigDir::default_path();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-c" | "--config" => dir = args.next().ok_or("--config needs a directory")?.into(),
            "-h" | "--help" => {
                println!("Usage: orbit-greet [-c|--config DIR]  (launched by orbit-core while locked)");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(dir)
}

/// What the screen holds, for the handlers.
struct Greet {
    user: String,
    link: Option<Rc<Link>>,
    app: gtk::Application,
    password: gtk::PasswordEntry,
    message: gtk::Label,
    /// Wrong passwords since the last unlock, and whether a check or a
    /// wait is going on.
    failures: Cell<u32>,
    busy: Cell<bool>,
    /// The password or the question of a power button, and that question.
    pages: gtk::Stack,
    question: gtk::Label,
    yes: gtk::Button,
    no: gtk::Button,
    asked: Cell<Option<power::Action>>,
    power: power::Config,
}

fn build(app: &gtk::Application, display: &gdk::Display, power: power::Config) {
    let (user, real_name) = pam::user();
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Orbit")
        .css_classes(["orbit-greet"])
        .build();
    // Only the right password leaves.
    window.connect_close_request(|_| glib::Propagation::Stop);

    let time = gtk::Label::builder().css_classes(["time"]).build();
    let date = gtk::Label::builder().css_classes(["date"]).build();
    tick(&time, &date);
    {
        let (time, date) = (time.clone(), date.clone());
        glib::timeout_add_seconds_local(1, move || {
            tick(&time, &date);
            glib::ControlFlow::Continue
        });
    }

    let avatar = avatar(&user);
    let name = gtk::Label::builder().label(real_name.as_deref().unwrap_or(&user)).css_classes(["name"]).build();
    let password = gtk::PasswordEntry::builder()
        .placeholder_text(tr("greet-password"))
        .show_peek_icon(true)
        .css_classes(["password"])
        .build();
    let message = gtk::Label::builder().css_classes(["message"]).wrap(true).justify(gtk::Justification::Center).build();

    let login = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    for widget in [avatar.upcast_ref::<gtk::Widget>(), name.upcast_ref(), password.upcast_ref(), message.upcast_ref()] {
        login.append(widget);
    }
    let question = gtk::Label::builder().css_classes(["question"]).wrap(true).justify(gtk::Justification::Center).build();
    let no = gtk::Button::builder().label(tr("session-no")).hexpand(true).build();
    let yes = gtk::Button::builder().label(tr("session-yes")).hexpand(true).css_classes(["yes"]).build();
    let answers = gtk::Box::builder().spacing(10).homogeneous(true).build();
    answers.append(&no);
    answers.append(&yes);
    let confirm = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(20).valign(gtk::Align::Center).css_classes(["confirm"]).build();
    confirm.append(&question);
    confirm.append(&answers);
    let pages = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).vhomogeneous(true).build();
    pages.add_named(&login, Some("login"));
    pages.add_named(&confirm, Some("confirm"));

    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["card"])
        .build();
    for widget in [time.upcast_ref::<gtk::Widget>(), date.upcast_ref(), pages.upcast_ref()] {
        card.append(widget);
    }
    let screen = gtk::Overlay::builder().child(&card).build();
    screen.add_overlay(&status::new(display));

    let link = Link::connect(display);
    if link.is_none() {
        log::warn!("no orbit_greet_v1: not launched by orbit-core; a preview (the right password quits)");
    }
    let greet = Rc::new(Greet {
        user,
        link: link.clone(),
        app: app.clone(),
        password: password.clone(),
        message,
        failures: Cell::new(0),
        busy: Cell::new(false),
        pages,
        question,
        yes: yes.clone(),
        no: no.clone(),
        asked: Cell::new(None),
        power,
    });
    {
        let greet = greet.clone();
        screen.add_overlay(&power::new(move |action| greet.ask(action)));
    }
    window.set_child(Some(&screen));
    {
        let greet = greet.clone();
        no.connect_clicked(move |_| greet.back());
    }
    {
        let greet = greet.clone();
        yes.connect_clicked(move |_| greet.answered());
    }
    // Escape leaves a question; nothing else of the keys is the greet's.
    let keys = gtk::EventControllerKey::new();
    {
        let greet = greet.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape && greet.asked.get().is_some() {
                greet.back();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    window.add_controller(keys);
    {
        let greet = greet.clone();
        password.connect_activate(move |_| greet.submit());
    }
    if let Some(link) = &link {
        let greet = greet.clone();
        link.watch(move |event| greet.event(event));
    }
    window.present();
    password.grab_focus();
}

impl Greet {
    /// The core locked or unlocked.
    fn event(&self, event: Event) {
        log::info!("{event:?}");
        self.password.set_text("");
        self.back();
        if event == Event::Locked && !self.busy.get() {
            self.message.set_text("");
            self.password.set_sensitive(true);
            self.password.grab_focus();
        }
    }

    /// A power button: suspend at once, the others after a "yes".
    fn ask(&self, action: power::Action) {
        let Some(question) = action.question() else {
            action.run(&self.power);
            return;
        };
        self.question.set_text(&question);
        if action == power::Action::Poweroff {
            self.yes.add_css_class("poweroff");
        } else {
            self.yes.remove_css_class("poweroff");
        }
        self.asked.set(Some(action));
        self.pages.set_visible_child_name("confirm");
        // "No" first, so a stray Enter does not take the machine down.
        self.no.grab_focus();
    }

    fn answered(&self) {
        if let Some(action) = self.asked.get() {
            action.run(&self.power);
        }
        self.back();
    }

    /// Back to the password.
    fn back(&self) {
        if self.asked.take().is_none() {
            return;
        }
        self.pages.set_visible_child_name("login");
        if self.password.is_sensitive() {
            self.password.grab_focus();
        }
    }

    /// Enter in the field: the password goes to PAM, in a thread (a wrong
    /// one takes pam_unix a couple of seconds).
    fn submit(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        let password = self.password.text().to_string();
        self.password.set_text("");
        if password.is_empty() {
            return;
        }
        self.busy.set(true);
        self.password.set_sensitive(false);
        self.message.set_text(&tr("greet-checking"));
        let (tx, rx) = async_channel::bounded(1);
        let user = self.user.clone();
        std::thread::spawn(move || {
            let _ = tx.send_blocking(pam::check(&user, password));
        });
        let greet = self.clone();
        glib::spawn_future_local(async move {
            let result = rx.recv().await.unwrap_or_else(|_| Err(pam::Refused("PAM thread died".into())));
            greet.checked(result);
        });
    }

    fn checked(self: &Rc<Self>, result: Result<(), pam::Refused>) {
        match result {
            Ok(()) => {
                log::info!("password accepted");
                self.failures.set(0);
                self.busy.set(false);
                self.message.set_text("");
                self.password.set_sensitive(true);
                match &self.link {
                    Some(link) => link.unlock(),
                    None => self.app.quit(),
                }
            }
            Err(pam::Refused(why)) => {
                log::warn!("password refused: {why}");
                let failures = self.failures.get() + 1;
                self.failures.set(failures);
                let wait = failures.checked_sub(FREE_TRIES).map(|n| MAX_WAIT.min(1 << n.min(5))).unwrap_or(0);
                self.refused(&why, wait);
            }
        }
    }

    /// Says why, and keeps the field off for `wait` seconds, counting down.
    fn refused(self: &Rc<Self>, why: &str, wait: u64) {
        let text = if why.contains("Authentication failure") { tr("greet-wrong") } else { why.to_owned() };
        if wait == 0 {
            self.message.set_text(&text);
            self.busy.set(false);
            self.password.set_sensitive(true);
            self.password.grab_focus();
            return;
        }
        let left = Cell::new(wait);
        let greet = self.clone();
        let show = move || {
            greet.message.set_text(&format!("{text}\n{}", tr_args("greet-wait", &[("seconds", number(left.get() as f64))])));
            if left.get() == 0 {
                greet.message.set_text(&text);
                greet.busy.set(false);
                greet.password.set_sensitive(true);
                greet.password.grab_focus();
                return glib::ControlFlow::Break;
            }
            left.set(left.get() - 1);
            glib::ControlFlow::Continue
        };
        show();
        glib::timeout_add_local(Duration::from_secs(1), show);
    }
}

/// The clock, as the panel writes it ("ter 21 out").
fn tick(time: &gtk::Label, date: &gtk::Label) {
    let Ok(now) = glib::DateTime::now_local() else { return };
    let fmt = |f: &str| now.format(f).map(|s| s.to_string()).unwrap_or_default();
    time.set_text(&fmt("%H:%M"));
    date.set_text(&fmt("%a %-d %b"));
}

/// The user's picture: `~/.face`, or AccountsService's, or the theme's
/// generic one.
fn avatar(user: &str) -> gtk::Image {
    let image = [glib::home_dir().join(".face"), PathBuf::from("/var/lib/AccountsService/icons").join(user)]
        .into_iter()
        .find(|p| p.is_file())
        .map(gtk::Image::from_file)
        .unwrap_or_else(|| gtk::Image::from_icon_name("avatar-default-symbolic"));
    image.set_pixel_size(120);
    image.add_css_class("avatar");
    image
}
