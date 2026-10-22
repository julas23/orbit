//! The dialog: over everything (an overlay layer that takes the keyboard,
//! the screen dimmed around a card like the greet's), saying what a
//! program wants to do and asking for the password of who may allow it.
//! Each try runs polkit's helper (`helper`); a wrong password starts
//! another. It ends allowed, cancelled (Esc, Cancel, or polkitd), or
//! with the helper gone.

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use orbit_app::i18n::tr;

use crate::{
    agent::{CANCELLED, Request},
    helper::{self, Helper, Said},
    identity::User,
};

pub struct Dialog {
    request: RefCell<Option<Request>>,
    cookie: String,
    users: Vec<User>,
    window: gtk::Window,
    avatar: gtk::Image,
    password: gtk::Entry,
    message: gtk::Label,
    helper: RefCell<Option<Helper>>,
    /// The helper asked and waits for the answer.
    asking: Cell<bool>,
    /// Who the current try is for, in `users`.
    chosen: Cell<usize>,
    /// Which try is current: what an older helper says is ignored.
    tries: Cell<u32>,
    /// PAM said why the last try failed (shown instead of "wrong password").
    said_error: Cell<bool>,
    finished: RefCell<Option<Box<dyn FnOnce()>>>,
}

impl Dialog {
    /// Shows `request` and starts the first try; `finished` runs once,
    /// when polkitd has its answer.
    pub fn show(app: &gtk::Application, request: Request, finished: impl FnOnce() + 'static) -> Rc<Self> {
        let window = gtk::Window::builder()
            .application(app)
            .title(tr("polkit-title"))
            .css_classes(["orbit-polkit"])
            .build();
        if gtk4_layer_shell::is_supported() {
            window.init_layer_shell();
            window.set_namespace(Some("orbit-polkit"));
            window.set_layer(Layer::Overlay);
            for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                window.set_anchor(edge, true);
            }
            window.set_exclusive_zone(-1);
            window.set_keyboard_mode(KeyboardMode::Exclusive);
        } else {
            window.set_modal(true);
            window.set_resizable(false);
        }

        let icon = gtk::Image::builder()
            .icon_name(if request.icon.is_empty() { "dialog-password-symbolic" } else { &request.icon })
            .pixel_size(50)
            .css_classes(["icon"])
            .build();
        let title = gtk::Label::builder().label(tr("polkit-title")).css_classes(["title"]).build();
        let what = gtk::Label::builder()
            .label(&request.message)
            .wrap(true)
            .justify(gtk::Justification::Center)
            .max_width_chars(40)
            .css_classes(["what"])
            .build();

        let avatar = gtk::Image::builder().pixel_size(80).css_classes(["avatar"]).build();
        let who: gtk::Widget = if request.users.len() > 1 {
            let names: Vec<String> = request.users.iter().map(shown_name).collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            let list = gtk::DropDown::from_strings(&names);
            list.set_halign(gtk::Align::Center);
            list.add_css_class("users");
            list.upcast()
        } else {
            gtk::Label::builder().label(shown_name(&request.users[0])).css_classes(["name"]).build().upcast()
        };
        let password = gtk::Entry::builder()
            .visibility(false)
            .input_purpose(gtk::InputPurpose::Password)
            .placeholder_text(tr("greet-password"))
            .sensitive(false)
            .css_classes(["password"])
            .build();
        let message = gtk::Label::builder().wrap(true).justify(gtk::Justification::Center).css_classes(["message"]).build();

        let details = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["details"]).build();
        for (key, value) in std::iter::once(("action".to_owned(), request.action.clone())).chain(request.details.iter().cloned()) {
            let line = gtk::Label::builder()
                .label(format!("{key}: {value}"))
                .xalign(0.0)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .selectable(true)
                .build();
            details.append(&line);
        }
        let expander = gtk::Expander::builder().label(tr("polkit-details")).child(&details).css_classes(["more"]).build();

        let cancel = gtk::Button::builder().label(tr("polkit-cancel")).hexpand(true).build();
        let allow = gtk::Button::builder().label(tr("polkit-authenticate")).hexpand(true).css_classes(["suggested-action"]).build();
        let buttons = gtk::Box::builder().spacing(10).css_classes(["buttons"]).homogeneous(true).build();
        buttons.append(&cancel);
        buttons.append(&allow);

        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .css_classes(["card"])
            .build();
        for widget in [
            icon.upcast_ref::<gtk::Widget>(),
            title.upcast_ref(),
            what.upcast_ref(),
            avatar.upcast_ref(),
            &who,
            password.upcast_ref(),
            message.upcast_ref(),
            buttons.upcast_ref(),
            expander.upcast_ref(),
        ] {
            card.append(widget);
        }
        window.set_child(Some(&card));

        let dialog = Rc::new(Self {
            cookie: request.cookie.clone(),
            users: request.users.clone(),
            request: RefCell::new(Some(request)),
            window: window.clone(),
            avatar,
            password: password.clone(),
            message,
            helper: RefCell::new(None),
            asking: Cell::new(false),
            chosen: Cell::new(0),
            tries: Cell::new(0),
            said_error: Cell::new(false),
            finished: RefCell::new(Some(Box::new(finished))),
        });

        // The handlers hold it weakly: the agent keeps it while shown.
        let weak = || Rc::downgrade(&dialog);
        if let Ok(list) = who.clone().downcast::<gtk::DropDown>() {
            let d = weak();
            list.connect_selected_notify(move |list| {
                if let Some(d) = d.upgrade() {
                    d.start(list.selected() as usize);
                }
            });
        }
        {
            let d = weak();
            password.connect_activate(move |_| {
                if let Some(d) = d.upgrade() {
                    d.submit();
                }
            });
        }
        {
            let d = weak();
            allow.connect_clicked(move |_| {
                if let Some(d) = d.upgrade() {
                    d.submit();
                }
            });
        }
        {
            let d = weak();
            cancel.connect_clicked(move |_| {
                if let Some(d) = d.upgrade() {
                    d.cancel();
                }
            });
        }
        {
            let d = weak();
            let keys = gtk::EventControllerKey::new();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key != gdk::Key::Escape {
                    return glib::Propagation::Proceed;
                }
                if let Some(d) = d.upgrade() {
                    d.cancel();
                }
                glib::Propagation::Stop
            });
            window.add_controller(keys);
        }
        {
            let d = weak();
            window.connect_close_request(move |_| {
                if let Some(d) = d.upgrade() {
                    d.cancel();
                }
                glib::Propagation::Stop
            });
        }

        dialog.start(0);
        window.present();
        password.grab_focus();
        dialog
    }

    pub fn cookie(&self) -> &str {
        &self.cookie
    }

    /// A new try, as `self.users[user]`.
    fn start(self: &Rc<Self>, index: usize) {
        let Some(user) = self.users.get(index) else { return };
        self.chosen.set(index);
        match picture(&user.name) {
            Some(path) => self.avatar.set_from_file(Some(path)),
            None => self.avatar.set_icon_name(Some("avatar-default-symbolic")),
        }
        let tries = self.tries.get() + 1;
        self.tries.set(tries);
        self.asking.set(false);
        self.said_error.set(false);
        self.password.set_sensitive(false);
        self.helper.borrow_mut().take();
        let (tx, rx) = async_channel::unbounded();
        match Helper::start(&user.name, &self.cookie, tx) {
            Ok(h) => *self.helper.borrow_mut() = Some(h),
            Err(err) => {
                log::error!("polkit-agent-helper-1: {err}");
                self.message.set_text(&err.to_string());
                return;
            }
        }
        let d = self.clone();
        glib::spawn_future_local(async move {
            while let Ok(said) = rx.recv().await {
                if d.tries.get() != tries {
                    break;
                }
                d.heard(said);
            }
        });
    }

    fn heard(self: &Rc<Self>, said: Said) {
        match said {
            Said::Prompt { text, echo } => {
                let text = text.trim().trim_end_matches(':').trim();
                if !text.is_empty() {
                    self.password.set_placeholder_text(Some(text));
                }
                self.password.set_visibility(echo);
                self.asking.set(true);
                self.password.set_sensitive(true);
                self.password.grab_focus();
            }
            Said::Info(text) => self.message.set_text(&text),
            Said::Error(text) => {
                self.said_error.set(true);
                self.message.set_text(&text);
            }
            Said::Done(true) => {
                log::info!("authenticated");
                self.finish(None);
            }
            Said::Done(false) => {
                log::warn!("not authenticated");
                if !self.said_error.get() {
                    self.message.set_text(&tr("greet-wrong"));
                }
                self.password.set_text("");
                self.start(self.chosen.get());
            }
        }
    }

    /// Enter or Authenticate: the answer goes to the helper.
    fn submit(&self) {
        if !self.asking.get() {
            return;
        }
        let text = self.password.text().to_string();
        self.password.set_text("");
        if text.is_empty() {
            helper::wipe(text);
            return;
        }
        self.asking.set(false);
        self.password.set_sensitive(false);
        self.message.set_text(&tr("greet-checking"));
        match self.helper.borrow_mut().as_mut() {
            Some(h) => h.answer(text),
            None => helper::wipe(text),
        }
    }

    /// Esc, Cancel, closing the window, or polkitd taking it back.
    pub fn cancel(&self) {
        log::info!("cancelled");
        self.finish(Some(CANCELLED));
    }

    /// Answers polkitd (an error name, or allowed) and goes away; only the
    /// first call counts.
    fn finish(&self, error: Option<&str>) {
        let Some(request) = self.request.borrow_mut().take() else { return };
        self.tries.set(self.tries.get() + 1);
        self.helper.borrow_mut().take();
        match error {
            None => request.invocation.return_value(None),
            Some(name) => request.invocation.return_dbus_error(name, "the authentication dialog was dismissed"),
        }
        self.window.destroy();
        if let Some(finished) = self.finished.borrow_mut().take() {
            finished();
        }
    }
}

fn shown_name(user: &User) -> String {
    user.real_name.clone().unwrap_or_else(|| user.name.clone())
}

/// A user's picture: AccountsService's, or `~/.face` for the one running
/// the agent.
fn picture(user: &str) -> Option<PathBuf> {
    let mut places = vec![PathBuf::from("/var/lib/AccountsService/icons").join(user)];
    if glib::user_name().to_str() == Some(user) {
        places.insert(0, glib::home_dir().join(".face"));
    }
    places.into_iter().find(|p| p.is_file())
}
