//! The card: over everything (an overlay layer that takes the keyboard,
//! the screen dimmed around it), in the look of the greet and of
//! orbit-polkit. It says what the keyring asks — the prompt's `message`,
//! `description` and `warning` — and asks for a password (twice, for a
//! new one) or for a yes, with the prompt's own choice ("unlock it at
//! login") when it has one. One card per prompt, asked again in place
//! when the keyring asks again (a wrong password).

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use orbit_app::i18n::tr;
use zeroize::Zeroizing;

/// What the keyring asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Password,
    Confirm,
}

pub enum Answer {
    Password(Zeroizing<String>),
    Yes,
    No,
}

/// The prompt's properties (gcr `GcrPrompt`), as the keyring last set
/// them; each `PerformPrompt` sends the ones that changed.
#[derive(Debug, Default)]
pub struct Props {
    pub title: String,
    pub message: String,
    pub description: String,
    pub warning: String,
    pub password_new: bool,
    pub choice_label: String,
    pub choice_chosen: bool,
    pub continue_label: String,
    pub cancel_label: String,
}

impl Props {
    pub fn update(&mut self, changed: &HashMap<String, glib::Variant>) {
        for (name, value) in changed {
            // Read from `a{sv}` the value is the inner one; a variant
            // left inside is opened.
            let value = if value.is_type(glib::VariantTy::VARIANT) { value.as_variant().unwrap_or_else(|| value.clone()) } else { value.clone() };
            let text = || value.get::<String>().unwrap_or_default();
            let flag = || value.get::<bool>().unwrap_or_default();
            match name.as_str() {
                "title" => self.title = text(),
                "message" => self.message = text(),
                "description" => self.description = text(),
                "warning" => self.warning = text(),
                "password-new" => self.password_new = flag(),
                "choice-label" => self.choice_label = text(),
                "choice-chosen" => self.choice_chosen = flag(),
                "continue-label" => self.continue_label = text(),
                "cancel-label" => self.cancel_label = text(),
                _ => {}
            }
        }
    }
}

pub struct Dialog {
    window: gtk::Window,
    title: gtk::Label,
    what: gtk::Label,
    password: gtk::PasswordEntry,
    again: gtk::PasswordEntry,
    choice: gtk::CheckButton,
    message: gtk::Label,
    cancel: gtk::Button,
    ok: gtk::Button,
    kind: Cell<Kind>,
    /// A question is shown and not yet answered.
    asking: Cell<bool>,
    answer: RefCell<Box<dyn Fn(Answer)>>,
}

impl Dialog {
    /// The card, empty until `ask`; `answer` gets each answer.
    pub fn show(app: &gtk::Application, answer: impl Fn(Answer) + 'static) -> Rc<Self> {
        let window = gtk::Window::builder().application(app).css_classes(["orbit-prompter"]).build();
        if gtk4_layer_shell::is_supported() {
            window.init_layer_shell();
            window.set_namespace(Some("orbit-prompter"));
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

        let icon = gtk::Image::builder().icon_name("dialog-password-symbolic").pixel_size(50).css_classes(["icon"]).build();
        let title = gtk::Label::builder().wrap(true).justify(gtk::Justification::Center).max_width_chars(40).css_classes(["title"]).build();
        let what = gtk::Label::builder().wrap(true).justify(gtk::Justification::Center).max_width_chars(40).css_classes(["what"]).build();
        let password = gtk::PasswordEntry::builder().placeholder_text(tr("greet-password")).show_peek_icon(true).css_classes(["password"]).build();
        let again = gtk::PasswordEntry::builder().placeholder_text(tr("prompter-again")).show_peek_icon(true).css_classes(["password", "again"]).build();
        let choice = gtk::CheckButton::builder().css_classes(["choice"]).build();
        let message = gtk::Label::builder().wrap(true).justify(gtk::Justification::Center).max_width_chars(40).css_classes(["message"]).build();
        let cancel = gtk::Button::builder().hexpand(true).build();
        let ok = gtk::Button::builder().hexpand(true).css_classes(["suggested-action"]).build();
        let buttons = gtk::Box::builder().spacing(10).css_classes(["buttons"]).homogeneous(true).build();
        buttons.append(&cancel);
        buttons.append(&ok);

        let card = gtk::Box::builder().orientation(gtk::Orientation::Vertical).halign(gtk::Align::Center).valign(gtk::Align::Center).css_classes(["card"]).build();
        for widget in [
            icon.upcast_ref::<gtk::Widget>(),
            title.upcast_ref(),
            what.upcast_ref(),
            password.upcast_ref(),
            again.upcast_ref(),
            choice.upcast_ref(),
            message.upcast_ref(),
            buttons.upcast_ref(),
        ] {
            card.append(widget);
        }
        window.set_child(Some(&card));

        let dialog = Rc::new(Self {
            window: window.clone(),
            title,
            what,
            password: password.clone(),
            again: again.clone(),
            choice,
            message,
            cancel: cancel.clone(),
            ok: ok.clone(),
            kind: Cell::new(Kind::Password),
            asking: Cell::new(false),
            answer: RefCell::new(Box::new(answer)),
        });

        let weak = || Rc::downgrade(&dialog);
        for entry in [&password, &again] {
            let d = weak();
            entry.connect_activate(move |_| {
                if let Some(d) = d.upgrade() {
                    d.submit();
                }
            });
        }
        {
            let d = weak();
            ok.connect_clicked(move |_| {
                if let Some(d) = d.upgrade() {
                    d.submit();
                }
            });
        }
        {
            let d = weak();
            cancel.connect_clicked(move |_| {
                if let Some(d) = d.upgrade() {
                    d.reply(Answer::No);
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
                    d.reply(Answer::No);
                }
                glib::Propagation::Stop
            });
            window.add_controller(keys);
        }
        {
            let d = weak();
            window.connect_close_request(move |_| {
                if let Some(d) = d.upgrade() {
                    d.reply(Answer::No);
                }
                glib::Propagation::Stop
            });
        }
        window.present();
        dialog
    }

    /// Shows what `props` say and asks for `kind`.
    pub fn ask(&self, kind: Kind, props: &Props) {
        self.kind.set(kind);
        let or = |text: &str, default: &str| if text.is_empty() { tr(default) } else { text.to_owned() };
        self.window.set_title(Some(&or(&props.title, "prompter-title")));
        self.title.set_text(&or(if props.message.is_empty() { &props.title } else { &props.message }, "prompter-title"));
        self.what.set_text(&props.description);
        self.what.set_visible(!props.description.is_empty());
        let password = kind == Kind::Password;
        self.password.set_visible(password);
        self.again.set_visible(password && props.password_new);
        self.choice.set_label(Some(&props.choice_label));
        self.choice.set_active(props.choice_chosen);
        self.choice.set_visible(!props.choice_label.is_empty());
        self.message.set_text(&props.warning);
        if props.warning.is_empty() {
            self.message.remove_css_class("warning");
        } else {
            self.message.add_css_class("warning");
        }
        self.ok.set_label(&or(&props.continue_label, "prompter-continue"));
        self.cancel.set_label(&or(&props.cancel_label, "polkit-cancel"));
        self.password.set_text("");
        self.again.set_text("");
        self.set_sensitive(true);
        self.asking.set(true);
        if password {
            self.password.grab_focus();
        } else {
            self.ok.grab_focus();
        }
    }

    pub fn choice_chosen(&self) -> bool {
        self.choice.is_active()
    }

    fn set_sensitive(&self, on: bool) {
        for widget in [self.password.upcast_ref::<gtk::Widget>(), self.again.upcast_ref(), self.choice.upcast_ref(), self.ok.upcast_ref()] {
            widget.set_sensitive(on);
        }
    }

    /// Enter or the continue button.
    fn submit(&self) {
        if !self.asking.get() {
            return;
        }
        if self.kind.get() == Kind::Confirm {
            self.reply(Answer::Yes);
            return;
        }
        let password = Zeroizing::new(self.password.text().to_string());
        if self.again.is_visible() {
            let again = Zeroizing::new(self.again.text().to_string());
            if *password != *again {
                self.message.set_text(&tr("prompter-mismatch"));
                self.message.add_css_class("warning");
                self.again.set_text("");
                self.again.grab_focus();
                return;
            }
        } else if password.is_empty() {
            return;
        }
        self.reply(Answer::Password(password));
    }

    /// Hands `answer` over; the card waits, insensitive, for the keyring's
    /// next word.
    fn reply(&self, answer: Answer) {
        if !self.asking.replace(false) {
            return;
        }
        self.password.set_text("");
        self.again.set_text("");
        self.set_sensitive(false);
        if matches!(answer, Answer::Password(_)) {
            self.message.remove_css_class("warning");
            self.message.set_text(&tr("greet-checking"));
        }
        (self.answer.borrow())(answer);
    }

    pub fn close(&self) {
        self.asking.set(false);
        self.window.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn properties_come_from_a_dbus_dictionary() {
        let sent = glib::Variant::parse(None, "{'message': <'Autenticação necessária'>, 'password-new': <true>, 'choice-chosen': <true>, 'other': <1>}").unwrap();
        let changed: HashMap<String, glib::Variant> = sent.get().unwrap();
        let mut props = Props::default();
        props.update(&changed);
        assert_eq!(props.message, "Autenticação necessária");
        assert!(props.password_new && props.choice_chosen);
        assert!(props.title.is_empty());
    }
}
