//! gcr's system prompter on the session bus (gcr `gcr-system-prompter.c`,
//! `gcr-dbus-constants.h`): `org.gnome.keyring.internal.Prompter` at
//! `/org/gnome/keyring/Prompter`, under the name
//! `org.gnome.keyring.SystemPrompter` that gnome-keyring calls.
//!
//! A caller (the keyring daemon) begins with `BeginPrompting(callback)`,
//! a path on its own connection; prompts wait their turn, one at a time.
//! Its turn comes as `PromptReady("", {}, exchange)` on the callback —
//! this side's public key (`exchange`). Each `PerformPrompt(callback,
//! type, properties, exchange)` shows the card (`dialog`) asking for a
//! password or a yes; the answer goes back as `PromptReady(reply,
//! changed properties, exchange)`, the password encrypted in the
//! exchange. A wrong password brings another `PerformPrompt` with a
//! warning. `StopPrompting` ends it and is answered with `PromptDone`; a
//! caller leaving the bus ends its prompts without one.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};

use crate::{
    dialog::{Answer, Dialog, Kind, Props},
    exchange::Exchange,
};

pub const NAME: &str = "org.gnome.keyring.SystemPrompter";
const PATH: &str = "/org/gnome/keyring/Prompter";
const CALLBACK: &str = "org.gnome.keyring.internal.Prompter.Callback";
const FAILED: &str = "org.freedesktop.DBus.Error.Failed";
const INVALID: &str = "org.freedesktop.DBus.Error.InvalidArgs";

const XML: &str = r#"<node>
  <interface name="org.gnome.keyring.internal.Prompter">
    <method name="BeginPrompting">
      <arg type="o" name="callback" direction="in"/>
    </method>
    <method name="PerformPrompt">
      <arg type="o" name="callback" direction="in"/>
      <arg type="s" name="type" direction="in"/>
      <arg type="a{sv}" name="properties" direction="in"/>
      <arg type="s" name="exchange" direction="in"/>
    </method>
    <method name="StopPrompting">
      <arg type="o" name="callback" direction="in"/>
    </method>
  </interface>
</node>"#;

/// A caller's callback object: its connection and path.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Callback {
    name: String,
    path: String,
}

/// The prompt whose turn it is.
struct Active {
    callback: Callback,
    exchange: Exchange,
    props: Props,
    /// The caller may ask (its PromptReady was delivered, and no prompt
    /// is waiting for the user).
    ready: bool,
    dialog: Option<Rc<Dialog>>,
}

pub struct Prompter {
    app: gtk::Application,
    conn: gio::DBusConnection,
    waiting: RefCell<VecDeque<Callback>>,
    active: RefCell<Option<Active>>,
    /// The callers watched, to end their prompts if they leave; each
    /// with what stops watching it.
    watches: RefCell<HashMap<String, Box<dyn FnOnce()>>>,
    /// The application stays while anything is begun.
    hold: RefCell<Option<gio::ApplicationHoldGuard>>,
    registration: RefCell<Option<gio::RegistrationId>>,
}

impl Prompter {
    pub fn start(app: &gtk::Application, conn: gio::DBusConnection) -> anyhow::Result<Rc<Self>> {
        let prompter = Rc::new(Self {
            app: app.clone(),
            conn,
            waiting: RefCell::default(),
            active: RefCell::default(),
            watches: RefCell::default(),
            hold: RefCell::default(),
            registration: RefCell::default(),
        });
        let node = gio::DBusNodeInfo::for_xml(XML)?;
        let iface = node.lookup_interface("org.gnome.keyring.internal.Prompter").ok_or_else(|| anyhow::anyhow!("no interface in the description"))?;
        let weak = Rc::downgrade(&prompter);
        let id = prompter
            .conn
            .register_object(PATH, &iface)
            .method_call(move |_, sender, _, _, method, params, invocation| {
                let Some(prompter) = weak.upgrade() else { return };
                let Some(sender) = sender else { return };
                let path = params.child_value(0).str().unwrap_or_default().to_owned();
                let callback = Callback { name: sender.to_owned(), path };
                match method {
                    "BeginPrompting" => prompter.begin(callback, invocation),
                    "PerformPrompt" => prompter.perform(callback, &params, invocation),
                    "StopPrompting" => {
                        prompter.stop(&callback, true);
                        invocation.return_value(None);
                        prompter.next();
                    }
                    _ => invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method),
                }
            })
            .build()?;
        *prompter.registration.borrow_mut() = Some(id);
        Ok(prompter)
    }

    fn known(&self, callback: &Callback) -> bool {
        self.waiting.borrow().contains(callback) || self.active.borrow().as_ref().is_some_and(|a| &a.callback == callback)
    }

    fn begin(self: &Rc<Self>, callback: Callback, invocation: gio::DBusMethodInvocation) {
        if self.known(&callback) {
            invocation.return_dbus_error(FAILED, "Already begun prompting for this prompt callback");
            return;
        }
        log::info!("prompting begun by {} {}", callback.name, callback.path);
        if !self.watches.borrow().contains_key(&callback.name) {
            let weak = Rc::downgrade(self);
            let watch = gio::bus_watch_name_on_connection(&self.conn, &callback.name, gio::BusNameWatcherFlags::NONE, |_, _, _| {}, move |_, name| {
                if let Some(prompter) = weak.upgrade() {
                    prompter.vanished(name);
                }
            });
            self.watches.borrow_mut().insert(callback.name.clone(), Box::new(move || gio::bus_unwatch_name(watch)));
        }
        self.waiting.borrow_mut().push_back(callback);
        self.hold.borrow_mut().get_or_insert_with(|| self.app.hold());
        invocation.return_value(None);
        self.next();
    }

    /// The next caller's turn, if none is prompting: it gets this side's
    /// public key.
    fn next(self: &Rc<Self>) {
        if self.active.borrow().is_some() {
            return;
        }
        let Some(callback) = self.waiting.borrow_mut().pop_front() else {
            self.idle();
            return;
        };
        let exchange = match Exchange::new() {
            Ok(e) => e,
            Err(err) => {
                log::error!("no key for the exchange: {err}");
                self.done(&callback);
                return self.next();
            }
        };
        let sent = exchange.begin();
        *self.active.borrow_mut() = Some(Active { callback, exchange, props: Props::default(), ready: false, dialog: None });
        self.ready("", HashMap::new(), sent);
    }

    /// `PromptReady` to the active caller; it may ask again once the call
    /// went through.
    fn ready(self: &Rc<Self>, reply: &str, changed: HashMap<String, glib::Variant>, exchange: String) {
        let Some(callback) = self.active.borrow().as_ref().map(|a| a.callback.clone()) else { return };
        let weak = Rc::downgrade(self);
        let delivered = callback.clone();
        self.conn.call(
            Some(&callback.name),
            &callback.path,
            CALLBACK,
            "PromptReady",
            Some(&(reply, changed, exchange).to_variant()),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            -1,
            gio::Cancellable::NONE,
            move |result| {
                let Some(prompter) = weak.upgrade() else { return };
                match result {
                    Ok(_) => {
                        if let Some(active) = prompter.active.borrow_mut().as_mut().filter(|a| a.callback == delivered) {
                            active.ready = true;
                        }
                    }
                    Err(err) => {
                        log::warn!("PromptReady to {}: {err}", delivered.name);
                        prompter.stop(&delivered, false);
                        prompter.next();
                    }
                }
            },
        );
    }

    fn perform(self: &Rc<Self>, callback: Callback, params: &glib::Variant, invocation: gio::DBusMethodInvocation) {
        type Args = (String, String, HashMap<String, glib::Variant>, String);
        let Some((_, kind, properties, received)) = params.get::<Args>() else {
            invocation.return_dbus_error(INVALID, "bad arguments");
            return;
        };
        let kind = match kind.as_str() {
            "password" => Kind::Password,
            "confirm" => Kind::Confirm,
            _ => {
                invocation.return_dbus_error(INVALID, "Invalid type argument");
                return;
            }
        };
        let mut active = self.active.borrow_mut();
        let Some(active) = active.as_mut().filter(|a| a.callback == callback) else {
            invocation.return_dbus_error(FAILED, "Not begun prompting for this prompt callback");
            return;
        };
        if !active.ready {
            invocation.return_dbus_error(FAILED, "Already performing a prompt for this prompt callback");
            return;
        }
        active.props.update(&properties);
        if let Err(err) = active.exchange.receive(&received) {
            log::warn!("secret exchange from {}: {err}", callback.name);
            invocation.return_dbus_error(INVALID, "Invalid secret exchange received");
            return;
        }
        active.ready = false;
        let dialog = active.dialog.get_or_insert_with(|| {
            let weak = Rc::downgrade(self);
            Dialog::show(&self.app, move |answer| {
                if let Some(prompter) = weak.upgrade() {
                    prompter.answered(answer);
                }
            })
        });
        dialog.ask(kind, &active.props);
        invocation.return_value(None);
    }

    /// The user answered the card: the reply, what changed on it, and
    /// the password sealed in the exchange.
    fn answered(self: &Rc<Self>, answer: Answer) {
        let sent = {
            let active = self.active.borrow();
            let Some(active) = active.as_ref() else { return };
            let mut changed = HashMap::new();
            if !active.props.choice_label.is_empty()
                && let Some(dialog) = &active.dialog
            {
                changed.insert("choice-chosen".to_owned(), dialog.choice_chosen().to_variant());
            }
            let (reply, secret) = match &answer {
                Answer::Password(password) => {
                    if active.props.password_new {
                        changed.insert("password-strength".to_owned(), i32::from(!password.is_empty()).to_variant());
                    }
                    ("yes", Some(password.as_bytes()))
                }
                Answer::Yes => ("yes", None),
                Answer::No => ("no", None),
            };
            match active.exchange.send(secret) {
                Ok(sent) => (reply, changed, sent),
                Err(err) => {
                    log::error!("secret exchange: {err}");
                    ("no", changed, active.exchange.begin())
                }
            }
        };
        let (reply, changed, exchange) = sent;
        log::info!("answered: {reply}");
        self.ready(reply, changed, exchange);
    }

    /// Ends `callback`'s prompt, telling it `PromptDone` if `done`.
    fn stop(&self, callback: &Callback, done: bool) {
        self.waiting.borrow_mut().retain(|c| c != callback);
        let active = self.active.borrow_mut().take_if(|a| &a.callback == callback);
        if let Some(active) = active {
            log::info!("prompting stopped for {} {}", callback.name, callback.path);
            if let Some(dialog) = active.dialog {
                dialog.close();
            }
        }
        if done {
            self.done(callback);
        }
    }

    fn done(&self, callback: &Callback) {
        self.conn.call(Some(&callback.name), &callback.path, CALLBACK, "PromptDone", None, None, gio::DBusCallFlags::NO_AUTO_START, -1, gio::Cancellable::NONE, |_| {});
    }

    /// A caller left the bus: its prompts end, unanswered.
    fn vanished(self: &Rc<Self>, name: &str) {
        log::info!("{name} left the bus");
        let unwatch = self.watches.borrow_mut().remove(name);
        if let Some(unwatch) = unwatch {
            unwatch();
        }
        self.waiting.borrow_mut().retain(|c| c.name != name);
        let gone = self.active.borrow().as_ref().map(|a| a.callback.clone()).filter(|c| c.name == name);
        if let Some(callback) = gone {
            self.stop(&callback, false);
        }
        self.next();
    }

    /// Nothing begun: the application may go after its inactivity time.
    fn idle(&self) {
        if self.waiting.borrow().is_empty() && self.active.borrow().is_none() {
            self.hold.borrow_mut().take();
        }
    }
}
