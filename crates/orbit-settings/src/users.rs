//! The Users and groups page (docs/PLANO-SETTINGS.md §3.3).
//!
//! One's own account: name and picture through AccountsService (no
//! password asked for one's own; the greet shows the picture), the
//! password through `passwd` (crate::passwd). The machine's: people added
//! and removed, administrators, another's password, groups and who is in
//! them, the session opened at boot — orbit-accounts-helper, under pkexec
//! (orbit_services::accounts).

use std::{path::PathBuf, rc::Rc};

use gtk::{gdk_pixbuf::Pixbuf, gio, glib, prelude::*};
use orbit_app::i18n::tr;
use orbit_services::accounts::{self, ADMIN_GROUP, Person, Step};

use crate::{dbus, passwd};

const ACCOUNTS: &str = "org.freedesktop.Accounts";

/// A person's AccountsService object, when the service is there.
fn account(name: &str) -> Option<gio::DBusProxy> {
    let manager = dbus::system(ACCOUNTS)?;
    let path = manager
        .call_sync("FindUserByName", Some(&(name,).to_variant()), gio::DBusCallFlags::NONE, 2000, gio::Cancellable::NONE)
        .ok()?
        .child_value(0);
    let path = path.str()?.to_owned();
    dbus::system_at(ACCOUNTS, &path, "org.freedesktop.Accounts.User")
}

/// The picture of `person`: AccountsService's, then their `~/.face`.
fn picture(person: &Person, account: Option<&gio::DBusProxy>) -> Option<PathBuf> {
    let icon = account.map(|a| dbus::string(a, "IconFile")).filter(|p| !p.is_empty()).map(PathBuf::from);
    [icon, Some(PathBuf::from(&person.home).join(".face"))]
        .into_iter()
        .flatten()
        .find(|p| std::fs::File::open(p).is_ok())
}

fn avatar(person: &Person, account: Option<&gio::DBusProxy>, size: i32) -> gtk::Image {
    let image = gtk::Image::builder().pixel_size(size).css_classes(["avatar"]).build();
    match picture(person, account) {
        Some(path) => image.set_from_file(Some(&path)),
        None => image.set_icon_name(Some("avatar-default-symbolic")),
    }
    image
}

/// `file` cut square and made 256 px, saved where AccountsService can
/// take it from.
fn square(file: &gio::File) -> Result<PathBuf, String> {
    let path = file.path().ok_or("not a local file")?;
    let full = Pixbuf::from_file(&path).map_err(|e| e.to_string())?;
    let side = full.width().min(full.height());
    let cut = full.new_subpixbuf((full.width() - side) / 2, (full.height() - side) / 2, side, side);
    let small = cut.scale_simple(256, 256, gtk::gdk_pixbuf::InterpType::Bilinear).ok_or("could not scale it")?;
    let out = glib::user_runtime_dir().join("orbit-settings-face.png");
    small.savev(&out, "png", &[]).map_err(|e| e.to_string())?;
    Ok(out)
}

/// A small modal window over `over`'s, with its body and its buttons'
/// row (Cancel already in it).
fn dialog(over: &impl IsA<gtk::Widget>, title: &str) -> (gtk::Window, gtk::Box, gtk::Box) {
    let window = gtk::Window::builder()
        .title(title)
        .modal(true)
        .default_width(420)
        .css_classes(["orbit-settings"])
        .build();
    if let Some(parent) = over.root().and_downcast::<gtk::Window>() {
        window.set_transient_for(Some(&parent));
    }
    let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).css_classes(["page"]).build();
    let buttons = gtk::Box::builder().spacing(10).halign(gtk::Align::End).margin_top(10).build();
    let cancel = gtk::Button::with_label(&tr("settings-svc-cancel"));
    cancel.connect_clicked({
        let window = window.clone();
        move |_| window.close()
    });
    buttons.append(&cancel);
    let outer = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    outer.append(&body);
    let foot = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["page"]).build();
    foot.append(&buttons);
    outer.append(&foot);
    window.set_child(Some(&outer));
    (window, body, buttons)
}

fn password_entry(placeholder: &str) -> gtk::PasswordEntry {
    gtk::PasswordEntry::builder().placeholder_text(placeholder).show_peek_icon(true).hexpand(true).build()
}

struct Page {
    me: String,
    content: gtk::Box,
    message: gtk::Label,
}

impl Page {
    fn say(&self, text: &str) {
        crate::say(&self.message, text);
    }

    /// Runs `steps` with the helper (polkit asks for the password), then
    /// shows the page again.
    fn helper(self: &Rc<Self>, steps: Vec<Step>, secret: Option<String>) {
        let page = self.clone();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || accounts::run(&steps, secret.as_deref()))
                .await
                .unwrap_or_else(|_| Err("orbit-accounts-helper".into()));
            page.say(&result.err().unwrap_or_default());
            page.fill();
        });
    }

    fn fill(self: &Rc<Self>) {
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        let people = accounts::people();
        let me = people.iter().find(|p| p.name == self.me).cloned();
        if let Some(me) = me {
            self.content.append(&self.mine(&me));
        }
        self.content.append(&self.people(&people));
        self.content.append(&self.groups(&people));
        if let Some(boot) = self.boot(&people) {
            self.content.append(&boot);
        }
    }

    /// One's own account.
    fn mine(self: &Rc<Self>, me: &Person) -> gtk::Box {
        let card = crate::power_card(&tr("settings-users-you"));
        let account = account(&me.name);
        let top = gtk::Box::builder().spacing(15).build();
        top.append(&avatar(me, account.as_ref(), 72));
        let names = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).valign(gtk::Align::Center).build();
        names.append(&gtk::Label::builder().label(me.shown()).xalign(0.0).css_classes(["heading"]).build());
        let mut about = me.name.clone();
        if accounts::is_admin(&me.name) {
            about.push_str(&format!(" · {}", tr("settings-users-admin")));
        }
        names.append(&gtk::Label::builder().label(about).xalign(0.0).css_classes(["dim"]).build());
        top.append(&names);
        card.append(&top);

        match &account {
            Some(account) => {
                let name = gtk::Entry::builder().text(me.full_name.as_str()).hexpand(true).build();
                let page = self.clone();
                let proxy = account.clone();
                name.connect_activate(move |e| {
                    let text = e.text().trim().to_owned();
                    if !accounts::valid_full_name(&text) {
                        page.say(&tr("settings-users-bad-full-name"));
                        return;
                    }
                    let page = page.clone();
                    let proxy = proxy.clone();
                    glib::spawn_future_local(async move {
                        let result = dbus::call(&proxy, "SetRealName", (text,).to_variant()).await;
                        page.say(&result.err().unwrap_or_default());
                        page.fill();
                    });
                });
                let row = crate::row(&tr("settings-users-full-name"), &name);
                row.set_tooltip_text(Some(&tr("settings-users-enter")));
                card.append(&row);

                let choose = gtk::Button::with_label(&tr("settings-users-choose-picture"));
                let remove = gtk::Button::with_label(&tr("settings-users-no-picture"));
                let line = gtk::Box::builder().spacing(10).halign(gtk::Align::End).build();
                line.append(&remove);
                line.append(&choose);
                card.append(&crate::row(&tr("settings-users-picture"), &line));
                let page = self.clone();
                let proxy = account.clone();
                choose.connect_clicked(move |b| {
                    let filter = gtk::FileFilter::new();
                    filter.add_mime_type("image/*");
                    filter.set_name(Some(&tr("settings-apps-images")));
                    let filters = gio::ListStore::new::<gtk::FileFilter>();
                    filters.append(&filter);
                    let chooser = gtk::FileDialog::builder()
                        .title(tr("settings-users-choose-picture"))
                        .filters(&filters)
                        .modal(true)
                        .build();
                    let window = b.root().and_downcast::<gtk::Window>();
                    let page = page.clone();
                    let proxy = proxy.clone();
                    glib::spawn_future_local(async move {
                        let Ok(file) = chooser.open_future(window.as_ref()).await else { return };
                        let result = match square(&file) {
                            Ok(path) => dbus::call(&proxy, "SetIconFile", (path.to_string_lossy().as_ref(),).to_variant())
                                .await
                                .map(|_| ()),
                            Err(e) => Err(e),
                        };
                        page.say(&result.err().unwrap_or_default());
                        page.fill();
                    });
                });
                let page = self.clone();
                let proxy = account.clone();
                remove.connect_clicked(move |_| {
                    let page = page.clone();
                    let proxy = proxy.clone();
                    glib::spawn_future_local(async move {
                        let result = dbus::call(&proxy, "SetIconFile", ("",).to_variant()).await;
                        page.say(&result.err().unwrap_or_default());
                        page.fill();
                    });
                });
            }
            None => card.append(
                &gtk::Label::builder()
                    .label(tr("settings-users-no-accountsservice"))
                    .xalign(0.0)
                    .wrap(true)
                    .css_classes(["dim"])
                    .build(),
            ),
        }

        let change = gtk::Button::with_label(&tr("settings-users-change-password"));
        let page = self.clone();
        change.connect_clicked(move |b| page.own_password(b));
        card.append(&crate::row(&tr("settings-users-password"), &change));
        card
    }

    fn own_password(self: &Rc<Self>, over: &gtk::Button) {
        let (window, body, buttons) = dialog(over, &tr("settings-users-change-password"));
        let current = password_entry(&tr("settings-users-current"));
        let new = password_entry(&tr("settings-users-new"));
        let again = password_entry(&tr("settings-users-again"));
        let note = crate::message();
        for e in [&current, &new, &again] {
            body.append(e);
        }
        body.append(&note);
        let ok = gtk::Button::with_label(&tr("settings-users-change"));
        ok.add_css_class("suggested-action");
        buttons.append(&ok);
        let page = self.clone();
        let window2 = window.clone();
        ok.connect_clicked(move |b| {
            let (old, fresh) = (current.text().to_string(), new.text().to_string());
            if fresh.is_empty() || fresh != again.text().as_str() {
                crate::say(&note, &tr("settings-users-mismatch"));
                return;
            }
            b.set_sensitive(false);
            crate::say(&note, &tr("settings-users-changing"));
            let (b, note, window, page) = (b.clone(), note.clone(), window2.clone(), page.clone());
            glib::spawn_future_local(async move {
                let result = gio::spawn_blocking(move || passwd::change(&old, &fresh))
                    .await
                    .unwrap_or_else(|_| Err(passwd::Failure::Said("passwd".into())));
                b.set_sensitive(true);
                match result {
                    Ok(()) => {
                        page.say(&tr("settings-users-changed"));
                        window.close();
                    }
                    Err(passwd::Failure::Wrong) => crate::say(&note, &tr("settings-users-wrong")),
                    Err(passwd::Failure::Said(text)) => crate::say(&note, &text),
                }
            });
        });
        window.present();
    }

    /// The machine's people.
    fn people(self: &Rc<Self>, people: &[Person]) -> gtk::Box {
        let card = crate::power_card(&tr("settings-users-people"));
        let boot = accounts::boot_user();
        for person in people {
            let line = gtk::Box::builder().spacing(10).css_classes(["row"]).build();
            let account = account(&person.name);
            line.append(&avatar(person, account.as_ref(), 32));
            let names = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).valign(gtk::Align::Center).build();
            let mut shown = person.shown().to_owned();
            if person.name == self.me {
                shown.push_str(&format!(" ({})", tr("settings-users-is-you")));
            }
            names.append(&gtk::Label::builder().label(shown).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build());
            names.append(&gtk::Label::builder().label(person.name.as_str()).xalign(0.0).css_classes(["dim"]).build());
            line.append(&names);

            let admin = gtk::CheckButton::builder()
                .label(tr("settings-users-admin"))
                .active(accounts::is_admin(&person.name))
                .valign(gtk::Align::Center)
                .build();
            let page = self.clone();
            let name = person.name.clone();
            admin.connect_toggled(move |c| {
                let step = if c.is_active() {
                    Step::GroupJoin(name.clone(), ADMIN_GROUP.into())
                } else {
                    Step::GroupLeave(name.clone(), ADMIN_GROUP.into())
                };
                page.helper(vec![step], None);
            });
            line.append(&admin);

            if person.name != self.me {
                let key = gtk::Button::builder()
                    .icon_name("dialog-password-symbolic")
                    .tooltip_text(tr("settings-users-set-password"))
                    .css_classes(["flat"])
                    .valign(gtk::Align::Center)
                    .build();
                let page = self.clone();
                let who = person.clone();
                key.connect_clicked(move |b| page.their_password(b, &who));
                line.append(&key);
                let remove = gtk::Button::builder()
                    .icon_name("user-trash-symbolic")
                    .tooltip_text(tr("settings-kb-remove"))
                    .css_classes(["flat"])
                    .valign(gtk::Align::Center)
                    .sensitive(boot.as_deref() != Some(person.name.as_str()))
                    .build();
                let page = self.clone();
                let who = person.clone();
                remove.connect_clicked(move |b| page.remove(b, &who));
                line.append(&remove);
            }
            card.append(&line);
        }
        let add = gtk::Button::with_label(&tr("settings-users-add"));
        add.add_css_class("suggested-action");
        add.set_halign(gtk::Align::End);
        let page = self.clone();
        add.connect_clicked(move |b| page.add(b));
        card.append(&add);
        card
    }

    fn their_password(self: &Rc<Self>, over: &gtk::Button, who: &Person) {
        let title = tr("settings-users-password-of").replace("{name}", who.shown());
        let (window, body, buttons) = dialog(over, &title);
        let new = password_entry(&tr("settings-users-new"));
        let again = password_entry(&tr("settings-users-again"));
        let note = crate::message();
        body.append(&new);
        body.append(&again);
        body.append(&note);
        let ok = gtk::Button::with_label(&tr("settings-users-change"));
        ok.add_css_class("suggested-action");
        buttons.append(&ok);
        let page = self.clone();
        let name = who.name.clone();
        let window2 = window.clone();
        ok.connect_clicked(move |_| {
            let fresh = new.text().to_string();
            if fresh.is_empty() || fresh != again.text().as_str() {
                crate::say(&note, &tr("settings-users-mismatch"));
                return;
            }
            page.helper(vec![Step::Password(name.clone())], Some(fresh));
            window2.close();
        });
        window.present();
    }

    fn remove(self: &Rc<Self>, over: &gtk::Button, who: &Person) {
        let title = tr("settings-users-remove-of").replace("{name}", who.shown());
        let (window, body, buttons) = dialog(over, &title);
        body.append(&gtk::Label::builder().label(tr("settings-users-remove-tip")).xalign(0.0).wrap(true).build());
        let files = gtk::CheckButton::with_label(&tr("settings-users-remove-files").replace("{home}", &who.home));
        body.append(&files);
        let ok = gtk::Button::with_label(&tr("settings-kb-remove"));
        ok.add_css_class("destructive-action");
        buttons.append(&ok);
        let page = self.clone();
        let name = who.name.clone();
        let window2 = window.clone();
        ok.connect_clicked(move |_| {
            page.helper(vec![Step::UserDel(name.clone(), files.is_active())], None);
            window2.close();
        });
        window.present();
    }

    fn add(self: &Rc<Self>, over: &gtk::Button) {
        let (window, body, buttons) = dialog(over, &tr("settings-users-add"));
        let full = gtk::Entry::builder().placeholder_text(tr("settings-users-full-name")).hexpand(true).build();
        let login = gtk::Entry::builder().placeholder_text(tr("settings-users-login")).hexpand(true).build();
        let admin = gtk::CheckButton::with_label(&tr("settings-users-make-admin"));
        let new = password_entry(&tr("settings-users-new"));
        let again = password_entry(&tr("settings-users-again"));
        let note = crate::message();
        for w in [full.upcast_ref::<gtk::Widget>(), login.upcast_ref(), admin.upcast_ref(), new.upcast_ref(), again.upcast_ref(), note.upcast_ref()] {
            body.append(w);
        }
        // The login follows the full name until it is typed itself.
        full.connect_changed({
            let login = login.clone();
            move |e| {
                if login.widget_name() != "typed" {
                    login.set_text(&accounts::login_from(&e.text()));
                }
            }
        });
        login.connect_changed(|e| {
            if e.has_focus() {
                e.set_widget_name("typed");
            }
        });
        let ok = gtk::Button::with_label(&tr("settings-net-add"));
        ok.add_css_class("suggested-action");
        buttons.append(&ok);
        let page = self.clone();
        let window2 = window.clone();
        ok.connect_clicked(move |_| {
            let (name, full_name, fresh) = (login.text().to_string(), full.text().trim().to_owned(), new.text().to_string());
            let check = if !accounts::valid_name(&name) {
                Some(tr("settings-users-bad-login"))
            } else if !accounts::valid_full_name(&full_name) {
                Some(tr("settings-users-bad-full-name"))
            } else if accounts::people().iter().any(|p| p.name == name) || accounts::groups().iter().any(|g| g.name == name) {
                Some(tr("settings-users-taken"))
            } else if fresh.is_empty() || fresh != again.text().as_str() {
                Some(tr("settings-users-mismatch"))
            } else {
                None
            };
            if let Some(text) = check {
                crate::say(&note, &text);
                return;
            }
            page.helper(
                vec![Step::UserAdd(name.clone(), full_name, admin.is_active()), Step::Password(name)],
                Some(fresh),
            );
            window2.close();
        });
        window.present();
    }

    /// The groups and who is in them.
    fn groups(self: &Rc<Self>, people: &[Person]) -> gtk::Box {
        let card = crate::power_card(&tr("settings-users-groups"));
        card.append(&gtk::Label::builder().label(tr("settings-users-groups-tip")).xalign(0.0).wrap(true).css_classes(["dim"]).build());
        for group in accounts::shown_groups() {
            let line = gtk::Box::builder().spacing(10).css_classes(["row"]).build();
            let names = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
            names.append(&gtk::Label::builder().label(group.name.as_str()).xalign(0.0).build());
            let members: Vec<&str> = group.members.iter().map(String::as_str).collect();
            let shown = if members.is_empty() { tr("settings-panel-empty") } else { members.join(", ") };
            names.append(&gtk::Label::builder().label(shown).xalign(0.0).wrap(true).css_classes(["dim"]).build());
            line.append(&names);
            let edit = gtk::Button::builder()
                .icon_name("system-users-symbolic")
                .tooltip_text(tr("settings-users-members"))
                .css_classes(["flat"])
                .valign(gtk::Align::Center)
                .build();
            let page = self.clone();
            let (name, people2, members2) = (group.name.clone(), people.to_vec(), group.members.clone());
            edit.connect_clicked(move |b| page.members(b, &name, &people2, &members2));
            line.append(&edit);
            if accounts::removable_group(&group.name) {
                let remove = gtk::Button::builder()
                    .icon_name("user-trash-symbolic")
                    .tooltip_text(tr("settings-kb-remove"))
                    .css_classes(["flat"])
                    .valign(gtk::Align::Center)
                    .build();
                let page = self.clone();
                let name = group.name.clone();
                remove.connect_clicked(move |_| page.helper(vec![Step::GroupDel(name.clone())], None));
                line.append(&remove);
            }
            card.append(&line);
        }
        let entry = gtk::Entry::builder().placeholder_text(tr("settings-users-group-name")).hexpand(true).build();
        let add = gtk::Button::with_label(&tr("settings-net-add"));
        let line = gtk::Box::builder().spacing(10).build();
        line.append(&entry);
        line.append(&add);
        card.append(&line);
        let go = {
            let page = self.clone();
            let entry = entry.clone();
            let people = people.to_vec();
            move |over: &gtk::Widget| {
                let name = entry.text().trim().to_owned();
                if !accounts::valid_name(&name) {
                    page.say(&tr("settings-users-bad-login"));
                    return;
                }
                // A group that is there already (a system one, no one in
                // it yet): who is in it.
                match accounts::groups().into_iter().find(|g| g.name == name) {
                    Some(g) => page.members(over, &g.name, &people, &g.members),
                    None => page.helper(vec![Step::GroupAdd(name)], None),
                }
                entry.set_text("");
            }
        };
        add.connect_clicked({
            let go = go.clone();
            move |b| go(b.upcast_ref())
        });
        entry.connect_activate(move |e| go(e.upcast_ref()));
        card
    }

    /// Who is in `group`: a box each; the changes go in one go.
    fn members(self: &Rc<Self>, over: &impl IsA<gtk::Widget>, group: &str, people: &[Person], members: &[String]) {
        let title = tr("settings-users-members-of").replace("{name}", group);
        let (window, body, buttons) = dialog(over, &title);
        let checks: Vec<(String, gtk::CheckButton)> = people
            .iter()
            .map(|p| {
                let check = gtk::CheckButton::builder()
                    .label(format!("{} ({})", p.shown(), p.name))
                    .active(members.contains(&p.name))
                    .build();
                body.append(&check);
                (p.name.clone(), check)
            })
            .collect();
        let ok = gtk::Button::with_label(&tr("settings-users-save"));
        ok.add_css_class("suggested-action");
        buttons.append(&ok);
        let page = self.clone();
        let (group, members) = (group.to_owned(), members.to_vec());
        let window2 = window.clone();
        ok.connect_clicked(move |_| {
            let steps: Vec<Step> = checks
                .iter()
                .filter_map(|(name, check)| match (check.is_active(), members.contains(name)) {
                    (true, false) => Some(Step::GroupJoin(name.clone(), group.clone())),
                    (false, true) => Some(Step::GroupLeave(name.clone(), group.clone())),
                    _ => None,
                })
                .collect();
            if !steps.is_empty() {
                page.helper(steps, None);
            }
            window2.close();
        });
        window.present();
    }

    /// The session opened at boot (Orbit-Login), when it is there.
    fn boot(self: &Rc<Self>, people: &[Person]) -> Option<gtk::Box> {
        let now = accounts::boot_user()?;
        let card = crate::power_card(&tr("settings-users-boot"));
        card.append(&gtk::Label::builder().label(tr("settings-users-boot-tip")).xalign(0.0).wrap(true).css_classes(["dim"]).build());
        let labels: Vec<String> = people.iter().map(|p| format!("{} ({})", p.shown(), p.name)).collect();
        let pick = crate::short_dropdown(&labels);
        pick.set_selected(people.iter().position(|p| p.name == now).unwrap_or(0) as u32);
        let page = self.clone();
        let names: Vec<String> = people.iter().map(|p| p.name.clone()).collect();
        pick.connect_selected_notify(move |d| {
            if let Some(name) = names.get(d.selected() as usize)
                && *name != now
            {
                page.helper(vec![Step::BootUser(name.clone())], None);
            }
        });
        card.append(&crate::row(&tr("settings-users-boot-user"), &pick));
        Some(card)
    }
}

pub fn page() -> gtk::Widget {
    let page = Rc::new(Page {
        me: glib::user_name().to_string_lossy().into_owned(),
        content: gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(20).build(),
        message: crate::message(),
    });
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();
    body.append(&page.content);
    body.append(&page.message);
    page.fill();
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .build()
        .upcast()
}
