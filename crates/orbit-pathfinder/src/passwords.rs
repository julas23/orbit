//! Passwords, by way of Bitwarden: filling a page's login form from the
//! vault (the key button, Ctrl+Shift+L) and importing what other browsers
//! saved. The vault is unlocked once per run, with the master password; the
//! session key is kept here, in memory (see `bitwarden`).
//!
//! `bw` is a program that takes a second or so per call, so every call runs
//! on a thread and the window carries on.

use std::{cell::RefCell, rc::Rc};

use gtk::{glib, prelude::*};
use orbit_app::i18n::{text, tr, tr_args};
use webkit6::prelude::*;

use crate::{bitwarden, import, ui::{background, close_on_escape}};

/// Fills the login form in the page: the password field, and the user
/// field before it (or the one that looks like it). Runs in a world of its
/// own, apart from the page's scripts; the values come as arguments, never
/// pasted into the code.
const FILL: &str = r#"
const visible = e => e && e.offsetParent !== null && !e.disabled && !e.readOnly;
const inputs = [...document.querySelectorAll('input')];
const pass = inputs.find(e => e.type === 'password' && visible(e));
const users = inputs.filter(e => ['text', 'email', 'tel', ''].includes(e.type) && visible(e));
let user = null;
if (pass) {
  for (let i = inputs.indexOf(pass) - 1; i >= 0; i--) {
    if (users.includes(inputs[i])) { user = inputs[i]; break; }
  }
}
if (!user) user = users.find(e => /user|email|login|account|ident/i.test(e.name + e.id + e.autocomplete)) || (pass ? null : users[0]);
const set = (e, v) => {
  if (!e || !v) return;
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(e, v);
  e.dispatchEvent(new Event('input', { bubbles: true }));
  e.dispatchEvent(new Event('change', { bubbles: true }));
};
set(user, username);
set(pass, password);
(pass || user)?.focus();
return (user ? 1 : 0) + (pass ? 2 : 0);
"#;

fn message(parent: &gtk::Window, text: &str) {
    gtk::AlertDialog::builder().message(text).modal(true).build().show(Some(parent));
}

#[derive(Default)]
pub struct Vault {
    session: RefCell<Option<String>>,
}

impl Vault {
    /// Calls `f` with the session key, unlocking the vault first (asking
    /// for the master password) when it is not yet — or with `None` when
    /// that does not happen (no `bw`, not logged in, the dialog closed).
    pub fn with_session(self: &Rc<Self>, parent: &gtk::Window, f: impl FnOnce(Option<String>) + 'static) {
        if let Some(key) = self.session.borrow().clone() {
            f(Some(key));
            return;
        }
        let (this, parent) = (self.clone(), parent.clone());
        background(bitwarden::status, move |status| match status {
            bitwarden::Status::Missing => {
                message(&parent, &tr("pathfinder-bw-missing"));
                f(None);
            }
            bitwarden::Status::LoggedOut => {
                message(&parent, &tr("pathfinder-bw-login"));
                f(None);
            }
            bitwarden::Status::Locked => this.ask_master(&parent, f),
        });
    }

    fn ask_master(self: &Rc<Self>, parent: &gtk::Window, f: impl FnOnce(Option<String>) + 'static) {
        let entry = gtk::PasswordEntry::builder().show_peek_icon(true).hexpand(true).activates_default(true).build();
        let status = gtk::Label::builder().xalign(0.0).css_classes(["error-text"]).visible(false).build();
        let go = gtk::Button::builder().label(tr("pathfinder-bw-unlock")).css_classes(["suggested-action"]).halign(gtk::Align::End).build();
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).css_classes(["dialog-box"]).build();
        body.append(&gtk::Label::builder().label(tr("pathfinder-bw-master")).xalign(0.0).build());
        body.append(&entry);
        body.append(&status);
        body.append(&go);
        let dialog = gtk::Window::builder()
            .transient_for(parent)
            .modal(true)
            .title("Bitwarden")
            .resizable(false)
            .default_width(380)
            .css_classes(["orbit-pathfinder-dialog"])
            .child(&body)
            .default_widget(&go)
            .build();
        close_on_escape(&dialog);
        let f = Rc::new(RefCell::new(Some(Box::new(f) as Box<dyn FnOnce(Option<String>)>)));
        // Closed without unlocking: the caller hears of it.
        dialog.connect_close_request(glib::clone!(
            #[strong]
            f,
            move |_| {
                if let Some(f) = f.borrow_mut().take() {
                    f(None);
                }
                glib::Propagation::Proceed
            }
        ));
        let this = self.clone();
        let run = Rc::new(glib::clone!(
            #[weak]
            dialog,
            #[weak]
            entry,
            #[weak]
            go,
            #[weak]
            status,
            move || {
                let master = entry.text().to_string();
                if master.is_empty() {
                    return;
                }
                go.set_sensitive(false);
                status.set_visible(false);
                let (this, f) = (this.clone(), f.clone());
                background(
                    move || bitwarden::unlock(&master),
                    move |result| match result {
                        Ok(key) => {
                            *this.session.borrow_mut() = Some(key.clone());
                            let f = f.borrow_mut().take();
                            dialog.close();
                            if let Some(f) = f {
                                f(Some(key));
                            }
                        }
                        Err(err) => {
                            log::warn!("bw unlock: {err}");
                            status.set_label(&tr("pathfinder-bw-wrong"));
                            status.set_visible(true);
                            go.set_sensitive(true);
                            entry.set_text("");
                            entry.grab_focus();
                        }
                    },
                );
            }
        ));
        {
            let run = run.clone();
            go.connect_clicked(move |_| run());
        }
        entry.connect_activate(move |_| run());
        dialog.present();
        entry.grab_focus();
    }

    /// Fills the page's login form from the vault; with several logins for
    /// the site, offers them in a menu under `anchor`.
    pub fn fill(self: &Rc<Self>, parent: &gtk::Window, view: &webkit6::WebView, anchor: &gtk::Widget) {
        let Some(url) = view.uri().map(|u| u.to_string()) else { return };
        let (parent2, view, anchor) = (parent.clone(), view.clone(), anchor.clone());
        self.with_session(parent, move |key| {
            let Some(key) = key else { return };
            background(
                move || bitwarden::find(&key, &url),
                move |found| match found {
                    Ok(list) if list.is_empty() => message(&parent2, &tr("pathfinder-bw-none")),
                    Ok(list) if list.len() == 1 => inject(&view, &list[0]),
                    Ok(list) => choose(&anchor, &view, list),
                    Err(err) => message(&parent2, &err),
                },
            )
        });
    }

    /// The import dialog: the browsers found, with how many passwords each,
    /// and the Firefox primary password if there is one.
    pub fn import_dialog(self: &Rc<Self>, parent: &gtk::Window) {
        let found = import::discover();
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).css_classes(["dialog-box"]).build();
        body.append(&gtk::Label::builder().label(tr("pathfinder-import-intro")).wrap(true).max_width_chars(50).xalign(0.0).build());
        let mut checks = Vec::new();
        for f in &found {
            let check = gtk::CheckButton::builder()
                .label(tr_args("pathfinder-import-source", &[("name", text(f.label())), ("count", text(f.count.to_string()))]))
                .active(true)
                .build();
            body.append(&check);
            checks.push((check, f.clone()));
        }
        if found.is_empty() {
            body.append(&gtk::Label::builder().label(tr("pathfinder-import-nothing")).xalign(0.0).build());
        }
        let firefox = gtk::PasswordEntry::builder().show_peek_icon(true).placeholder_text(tr("pathfinder-import-firefox-pw")).build();
        if found.iter().any(|f| f.source == import::Source::Firefox) {
            body.append(&firefox);
        }
        let status = gtk::Label::builder().xalign(0.0).wrap(true).max_width_chars(50).visible(false).build();
        let go = gtk::Button::builder().label(tr("pathfinder-import-go")).css_classes(["suggested-action"]).halign(gtk::Align::End).sensitive(!found.is_empty()).build();
        body.append(&status);
        body.append(&go);
        let dialog = gtk::Window::builder()
            .transient_for(parent)
            .modal(true)
            .title(tr("pathfinder-import-title"))
            .resizable(false)
            .default_width(440)
            .css_classes(["orbit-pathfinder-dialog"])
            .child(&body)
            .build();
        close_on_escape(&dialog);
        let this = self.clone();
        go.connect_clicked(glib::clone!(
            #[weak]
            dialog,
            #[weak]
            status,
            #[weak]
            firefox,
            move |go| {
                let chosen: Vec<import::Found> = checks.iter().filter(|(c, _)| c.is_active()).map(|(_, f)| f.clone()).collect();
                if chosen.is_empty() {
                    return;
                }
                let firefox_pw = Some(firefox.text().to_string()).filter(|p| !p.is_empty());
                go.set_sensitive(false);
                status.set_label(&tr("pathfinder-import-working"));
                status.set_visible(true);
                let go = go.clone();
                this.with_session(&dialog.clone().upcast(), move |key| {
                    let Some(key) = key else {
                        status.set_visible(false);
                        go.set_sensitive(true);
                        return;
                    };
                    background(
                        move || {
                            let mut logins = Vec::new();
                            let mut problems = Vec::new();
                            for f in &chosen {
                                match import::read(f, firefox_pw.as_deref()) {
                                    Ok(mut l) => logins.append(&mut l),
                                    Err(import::Error::FirefoxPassword) => problems.push(tr("pathfinder-import-firefox-needed")),
                                    Err(import::Error::Other(e)) => problems.push(format!("{}: {e}", f.label())),
                                }
                            }
                            let result = if logins.is_empty() { Ok(bitwarden::Imported::default()) } else { bitwarden::import(&key, &logins) };
                            (result, problems)
                        },
                        move |(result, problems)| {
                            let mut lines = Vec::new();
                            match result {
                                Ok(r) => lines.push(tr_args(
                                    "pathfinder-import-done",
                                    &[("added", text(r.added.to_string())), ("already", text(r.already.to_string()))],
                                )),
                                Err(e) => lines.push(tr_args("pathfinder-import-failed", &[("error", text(e))])),
                            }
                            lines.extend(problems);
                            status.set_label(&lines.join("\n"));
                            go.set_sensitive(true);
                        },
                    )
                });
            }
        ));
        dialog.present();
    }
}

fn inject(view: &webkit6::WebView, credential: &bitwarden::Credential) {
    let args = glib::VariantDict::new(None);
    args.insert("username", credential.username.as_str());
    args.insert("password", credential.password.as_str());
    view.call_async_javascript_function(FILL, Some(&args.end()), Some("orbit-pathfinder"), None, gtk::gio::Cancellable::NONE, |result| {
        if let Err(err) = result {
            log::warn!("filling the form: {err}");
        }
    });
}

/// Several logins for the site: a menu of them.
fn choose(anchor: &gtk::Widget, view: &webkit6::WebView, list: Vec<bitwarden::Credential>) {
    let popover = gtk::Popover::builder().has_arrow(false).build();
    popover.set_parent(anchor);
    let box_ = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["menu-list"]).build();
    for credential in list {
        let label = if credential.username.is_empty() { credential.name.clone() } else { format!("{}  ·  {}", credential.username, credential.name) };
        let button = gtk::Button::builder().label(label).css_classes(["flat", "menu-item"]).build();
        let (view, p) = (view.clone(), popover.clone());
        button.connect_clicked(move |_| {
            p.popdown();
            inject(&view, &credential);
        });
        box_.append(&button);
    }
    popover.set_child(Some(&box_));
    popover.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    popover.popup();
}
