//! The Network drives page (docs/PLANO-SETTINGS.md §3.4): folders on other
//! machines (SMB, SFTP, NFS, WebDAV, FTP) that GVfs mounts in the session.
//! They are the GTK bookmarks with a remote address (the file Orbit-Files'
//! places and every GTK file dialog list), so a drive added here is there
//! too, and one added there is here. The password, asked by GVfs's own
//! dialog, can be kept in the keyring. Those mounted at login are listed in
//! `drives.yaml` (`login:`), and `orbit-settings drives --mount`, put in
//! core.yaml's `login:` while there is one, mounts them at the first
//! unlock, with the passwords the keyring holds.
//!
//! For every user, at boot: an SMB share or an NFS export as a systemd
//! automount under /mnt, written by orbit-services-helper
//! (`orbit_services::drives`); an SMB password goes to the helper on its
//! standard input, never on a command line, and is kept in a file only root
//! reads.

use std::{cell::Cell, path::PathBuf, rc::Rc, time::Duration};

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::tr;
use orbit_common::ConfigDir;
use serde_yaml_ng::Value;

use orbit_services::{Privileged, drives::{self as system, SystemDrive}};

use crate::apps_page;

/// What core.yaml's `login:` runs while a drive is mounted at login.
const AT_LOGIN: &str = "orbit-settings drives --mount";
/// The kinds offered: the address's scheme and its name.
const KINDS: [(&str, &str); 6] = [
    ("smb", "settings-drives-smb"),
    ("sftp", "settings-drives-sftp"),
    ("nfs", "settings-drives-nfs"),
    ("davs", "settings-drives-davs"),
    ("dav", "settings-drives-dav"),
    ("ftp", "settings-drives-ftp"),
];

fn bookmarks_file() -> PathBuf {
    glib::user_config_dir().join("gtk-3.0").join("bookmarks")
}

/// The bookmarks' lines: (address, name).
fn bookmarks() -> Vec<(String, String)> {
    std::fs::read_to_string(bookmarks_file())
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| match l.split_once(' ') {
            Some((uri, name)) => (uri.to_owned(), name.to_owned()),
            None => (l.to_owned(), String::new()),
        })
        .collect()
}

fn remote(uri: &str) -> bool {
    KINDS.iter().any(|(scheme, _)| uri.starts_with(&format!("{scheme}://")))
}

fn write_bookmarks(lines: &[(String, String)]) -> std::io::Result<()> {
    let path = bookmarks_file();
    std::fs::create_dir_all(path.parent().unwrap_or(&path))?;
    let text: String = lines
        .iter()
        .map(|(uri, name)| if name.is_empty() { format!("{uri}\n") } else { format!("{uri} {name}\n") })
        .collect();
    std::fs::write(path, text)
}

/// The address made from what was typed: `smb://user@server/share/dir`.
fn address(scheme: &str, server: &str, folder: &str, user: &str) -> Option<String> {
    let server = server.trim().trim_end_matches('/');
    if server.is_empty() || server.contains(char::is_whitespace) || server.contains('/') {
        return None;
    }
    let folder = glib::Uri::escape_string(folder.trim().trim_start_matches('/'), Some("/"), false);
    let user = user.trim();
    let who = if user.is_empty() { String::new() } else { format!("{}@", glib::Uri::escape_string(user, None, false)) };
    Some(format!("{scheme}://{who}{server}/{folder}"))
}

/// The drives mounted at login (`drives.yaml`).
fn at_login(dir: &ConfigDir) -> Vec<String> {
    std::fs::read_to_string(dir.path().join("drives.yaml"))
        .ok()
        .and_then(|t| serde_yaml_ng::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("login").and_then(Value::as_sequence).cloned())
        .map(|s| s.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

/// `drives.yaml`'s list, and core.yaml's line that mounts it.
fn save_at_login(dir: &ConfigDir, list: &[String]) -> anyhow::Result<()> {
    let mut block = serde_yaml_ng::Mapping::new();
    block.insert("login".into(), Value::Sequence(list.iter().map(|u| Value::from(u.as_str())).collect()));
    dir.save_app_keys("drives", &["login"], &serde_yaml_ng::to_string(&block)?)?;
    apps_page::ensure_login(dir, AT_LOGIN, !list.is_empty())
}

fn mounted(uri: &str) -> bool {
    gio::File::for_uri(uri).find_enclosing_mount(gio::Cancellable::NONE).is_ok()
}

/// `orbit-settings drives --mount`: mounts `drives.yaml`'s drives with no
/// window, the keyring's passwords only (one not kept there is skipped),
/// trying once more a little later when the network was not up yet.
pub fn mount_at_login(config: PathBuf) -> glib::ExitCode {
    let list = at_login(&ConfigDir::new(&config));
    if list.is_empty() {
        return glib::ExitCode::SUCCESS;
    }
    let main = glib::MainLoop::new(None, false);
    let left = Rc::new(Cell::new(list.len()));
    for uri in list {
        try_mount(uri, 3, main.clone(), left.clone());
    }
    glib::timeout_add_local_once(Duration::from_secs(180), {
        let main = main.clone();
        move || main.quit()
    });
    main.run();
    glib::ExitCode::SUCCESS
}

fn try_mount(uri: String, tries: u32, main: glib::MainLoop, left: Rc<Cell<usize>>) {
    let operation = gio::MountOperation::new();
    // Not kept in the keyring: nothing to answer with, no one to ask.
    operation.connect_ask_password(|op, _, _, _, _| op.reply(gio::MountOperationResult::Aborted));
    let file = gio::File::for_uri(&uri);
    file.mount_enclosing_volume(gio::MountMountFlags::NONE, Some(&operation), gio::Cancellable::NONE, move |result| {
        match result {
            Err(e) if !e.matches(gio::IOErrorEnum::AlreadyMounted) && tries > 1 => {
                log::info!("{uri}: {e}; trying again");
                glib::timeout_add_local_once(Duration::from_secs(20), move || try_mount(uri, tries - 1, main, left));
                return;
            }
            Err(e) if !e.matches(gio::IOErrorEnum::AlreadyMounted) => log::warn!("{uri}: {e}"),
            _ => log::info!("{uri}: mounted"),
        }
        left.set(left.get() - 1);
        if left.get() == 0 {
            main.quit();
        }
    });
}

/// The "add a drive" card's fields.
struct Form {
    kind: gtk::DropDown,
    server: gtk::Entry,
    folder: gtk::Entry,
    user: gtk::Entry,
    name: gtk::Entry,
    login: gtk::CheckButton,
}

struct Page {
    dir: ConfigDir,
    list: gtk::Box,
    message: gtk::Label,
}

impl Page {
    fn say(&self, text: &str) {
        crate::say(&self.message, text);
    }

    fn fill(self: &Rc<Self>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let login = at_login(&self.dir);
        let drives: Vec<(String, String)> = bookmarks().into_iter().filter(|(u, _)| remote(u)).collect();
        if drives.is_empty() {
            self.list.append(&gtk::Label::builder().label(tr("settings-drives-none")).xalign(0.0).css_classes(["dim"]).build());
        }
        for (uri, name) in drives {
            let line = gtk::Box::builder().spacing(10).css_classes(["row"]).build();
            let names = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
            let shown = if name.is_empty() { uri.clone() } else { name.clone() };
            let on = mounted(&uri);
            let state = if on { tr("settings-drives-mounted") } else { tr("settings-drives-not-mounted") };
            names.append(&gtk::Label::builder().label(shown).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build());
            names.append(
                &gtk::Label::builder()
                    .label(format!("{uri} · {state}"))
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::Middle)
                    .css_classes(["dim"])
                    .build(),
            );
            line.append(&names);

            let login_check = gtk::CheckButton::builder()
                .label(tr("settings-drives-at-login"))
                .active(login.contains(&uri))
                .valign(gtk::Align::Center)
                .build();
            let page = self.clone();
            let u = uri.clone();
            login_check.connect_toggled(move |c| {
                let mut list = at_login(&page.dir);
                list.retain(|x| *x != u);
                if c.is_active() {
                    list.push(u.clone());
                }
                page.say(&save_at_login(&page.dir, &list).err().map(|e| e.to_string()).unwrap_or_default());
            });
            line.append(&login_check);

            let (icon, tip) = if on {
                ("media-eject-symbolic", "settings-drives-disconnect")
            } else {
                ("network-server-symbolic", "settings-drives-connect")
            };
            let connect = gtk::Button::builder().icon_name(icon).tooltip_text(tr(tip)).css_classes(["flat"]).valign(gtk::Align::Center).build();
            let page = self.clone();
            let u = uri.clone();
            connect.connect_clicked(move |b| if on { page.unmount(&u) } else { page.mount(&u, b) });
            line.append(&connect);
            let open = gtk::Button::builder()
                .icon_name("folder-open-symbolic")
                .tooltip_text(tr("settings-drives-open"))
                .css_classes(["flat"])
                .valign(gtk::Align::Center)
                .sensitive(on)
                .build();
            let page = self.clone();
            let u = uri.clone();
            open.connect_clicked(move |_| {
                if let Err(e) = gio::AppInfo::launch_default_for_uri(&u, gio::AppLaunchContext::NONE) {
                    page.say(&e.to_string());
                }
            });
            line.append(&open);
            let remove = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text(tr("settings-kb-remove"))
                .css_classes(["flat"])
                .valign(gtk::Align::Center)
                .build();
            let page = self.clone();
            remove.connect_clicked(move |_| {
                let lines: Vec<(String, String)> = bookmarks().into_iter().filter(|(x, _)| *x != uri).collect();
                let mut list = at_login(&page.dir);
                list.retain(|x| *x != uri);
                let result = write_bookmarks(&lines).map_err(anyhow::Error::from).and_then(|()| save_at_login(&page.dir, &list));
                page.say(&result.err().map(|e| e.to_string()).unwrap_or_default());
                page.fill();
            });
            line.append(&remove);
            self.list.append(&line);
        }
    }

    /// Mounts `uri`, GVfs's dialog asking for a password when needed.
    fn mount(self: &Rc<Self>, uri: &str, over: &gtk::Button) {
        let window = over.root().and_downcast::<gtk::Window>();
        let operation = gtk::MountOperation::new(window.as_ref());
        self.say(&tr("settings-drives-connecting"));
        let page = self.clone();
        gio::File::for_uri(uri).mount_enclosing_volume(gio::MountMountFlags::NONE, Some(&operation), gio::Cancellable::NONE, move |r| {
            match r {
                Err(e) if !e.matches(gio::IOErrorEnum::AlreadyMounted) && !e.matches(gio::IOErrorEnum::FailedHandled) => {
                    page.say(&e.to_string())
                }
                _ => page.say(""),
            }
            page.fill();
        });
    }

    fn unmount(self: &Rc<Self>, uri: &str) {
        let Ok(mount) = gio::File::for_uri(uri).find_enclosing_mount(gio::Cancellable::NONE) else { return };
        let page = self.clone();
        mount.unmount_with_operation(gio::MountUnmountFlags::NONE, gio::MountOperation::NONE, gio::Cancellable::NONE, move |r| {
            page.say(&r.err().map(|e| e.to_string()).unwrap_or_default());
            page.fill();
        });
    }

    fn add(self: &Rc<Self>, form: &Form, over: &gtk::Button) {
        let scheme = KINDS.get(form.kind.selected() as usize).map_or("smb", |k| k.0);
        let Some(uri) = address(scheme, &form.server.text(), &form.folder.text(), &form.user.text()) else {
            self.say(&tr("settings-drives-bad-server"));
            return;
        };
        let mut lines = bookmarks();
        if !lines.iter().any(|(u, _)| *u == uri) {
            lines.push((uri.clone(), form.name.text().trim().to_owned()));
            if let Err(e) = write_bookmarks(&lines) {
                self.say(&e.to_string());
                return;
            }
        }
        if form.login.is_active() {
            let mut list = at_login(&self.dir);
            if !list.contains(&uri) {
                list.push(uri.clone());
            }
            if let Err(e) = save_at_login(&self.dir, &list) {
                self.say(&e.to_string());
            }
        }
        for e in [&form.server, &form.folder, &form.user, &form.name] {
            e.set_text("");
        }
        self.fill();
        self.mount(&uri, over);
    }
}

/// The card of the drives for every user: the list, and a form to add one.
fn system_card(message: &gtk::Label) -> gtk::Box {
    let card = crate::power_card(&tr("settings-drives-system"));
    card.append(&gtk::Label::builder().label(tr("settings-drives-system-tip")).xalign(0.0).wrap(true).css_classes(["dim"]).build());
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).build();
    card.append(&list);
    let fill: Rc<dyn Fn()> = Rc::new({
        let (list, message) = (list.clone(), message.clone());
        move || fill_system(&list, &message)
    });
    fill_system(&list, message);

    let kinds = [tr("settings-drives-smb"), tr("settings-drives-nfs")];
    let kind = gtk::DropDown::from_strings(&kinds.iter().map(String::as_str).collect::<Vec<_>>());
    card.append(&crate::row(&tr("settings-drives-kind"), &kind));
    let entry = |placeholder: &str| gtk::Entry::builder().placeholder_text(tr(placeholder)).hexpand(true).build();
    let server = entry("settings-drives-server-tip");
    let folder = entry("settings-drives-folder-tip");
    let user = entry("settings-drives-system-user-tip");
    let password = gtk::PasswordEntry::builder().show_peek_icon(true).hexpand(true).build();
    card.append(&crate::row(&tr("settings-drives-server"), &server));
    card.append(&crate::row(&tr("settings-drives-folder"), &folder));
    let user_row = crate::row(&tr("settings-drives-user"), &user);
    let password_row = crate::row(&tr("settings-drives-password"), &password);
    card.append(&user_row);
    card.append(&password_row);
    kind.connect_selected_notify({
        let (user_row, password_row) = (user_row.clone(), password_row.clone());
        move |d| {
            user_row.set_visible(d.selected() == 0);
            password_row.set_visible(d.selected() == 0);
        }
    });
    let go = gtk::Button::with_label(&tr("settings-drives-system-add"));
    go.set_halign(gtk::Align::End);
    let message = message.clone();
    go.connect_clicked(move |b| {
        let smb = kind.selected() == 0;
        let who = user.text().trim().to_owned();
        let at = if smb && !who.is_empty() { format!("{who}@") } else { String::new() };
        let scheme = if smb { "smb" } else { "nfs" };
        let typed = format!("{scheme}://{at}{}/{}", server.text().trim().trim_end_matches('/'), folder.text().trim().trim_start_matches('/'));
        let drive = match SystemDrive::parse(&typed) {
            Ok(d) => d,
            Err(_) => return crate::say(&message, &tr("settings-drives-system-bad")),
        };
        let input = drive.user.is_some().then(|| password.text().to_string());
        b.set_sensitive(false);
        crate::say(&message, &tr("settings-drives-connecting"));
        let (b, message, fill) = (b.clone(), message.clone(), fill.clone());
        let (server, folder, user, password) = (server.clone(), folder.clone(), user.clone(), password.clone());
        crate::helper::run_with(vec![Privileged::SystemDrive(drive.address())], input, |_| {}, move |error| {
            b.set_sensitive(true);
            if error.is_none() {
                for e in [server.upcast_ref::<gtk::Editable>(), folder.upcast_ref(), user.upcast_ref(), password.upcast_ref()] {
                    e.set_text("");
                }
            }
            crate::say(&message, &error.unwrap_or_default());
            fill();
        });
    });
    card.append(&go);
    card
}

fn fill_system(list: &gtk::Box, message: &gtk::Label) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let drives = system::list(std::path::Path::new(system::UNIT_DIR));
    if drives.is_empty() {
        list.append(&gtk::Label::builder().label(tr("settings-drives-none")).xalign(0.0).css_classes(["dim"]).build());
    }
    for drive in drives {
        let line = gtk::Box::builder().spacing(10).css_classes(["row"]).build();
        let names = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
        let point = drive.mount_point();
        names.append(&gtk::Label::builder().label(drive.address()).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).build());
        names.append(&gtk::Label::builder().label(point.display().to_string()).xalign(0.0).css_classes(["dim"]).build());
        line.append(&names);
        let open = gtk::Button::builder().icon_name("folder-open-symbolic").tooltip_text(tr("settings-drives-open")).css_classes(["flat"]).valign(gtk::Align::Center).build();
        let m = message.clone();
        open.connect_clicked(move |_| {
            if let Err(e) = gio::AppInfo::launch_default_for_uri(&gio::File::for_path(&point).uri(), gio::AppLaunchContext::NONE) {
                crate::say(&m, &e.to_string());
            }
        });
        line.append(&open);
        let remove = gtk::Button::builder().icon_name("user-trash-symbolic").tooltip_text(tr("settings-kb-remove")).css_classes(["flat"]).valign(gtk::Align::Center).build();
        let (list2, m) = (list.clone(), message.clone());
        remove.connect_clicked(move |b| {
            b.set_sensitive(false);
            let (list, m) = (list2.clone(), m.clone());
            crate::helper::run(vec![Privileged::SystemDriveRemove(drive.address())], |_| {}, move |error| {
                crate::say(&m, &error.unwrap_or_default());
                fill_system(&list, &m);
            });
        });
        line.append(&remove);
        list.append(&line);
    }
}

pub fn page(dir: &ConfigDir) -> gtk::Widget {
    let page = Rc::new(Page {
        dir: dir.clone(),
        list: gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).build(),
        message: crate::message(),
    });
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();

    let drives = crate::power_card(&tr("settings-drives-list"));
    drives.append(&gtk::Label::builder().label(tr("settings-drives-tip")).xalign(0.0).wrap(true).css_classes(["dim"]).build());
    drives.append(&page.list);
    body.append(&drives);

    let add = crate::power_card(&tr("settings-drives-add"));
    let labels: Vec<String> = KINDS.iter().map(|(_, l)| tr(l)).collect();
    let kind = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    add.append(&crate::row(&tr("settings-drives-kind"), &kind));
    let entry = |placeholder: &str| gtk::Entry::builder().placeholder_text(tr(placeholder)).hexpand(true).build();
    let server = entry("settings-drives-server-tip");
    let folder = entry("settings-drives-folder-tip");
    let user = entry("settings-drives-user-tip");
    let name = entry("settings-drives-name-tip");
    add.append(&crate::row(&tr("settings-drives-server"), &server));
    add.append(&crate::row(&tr("settings-drives-folder"), &folder));
    add.append(&crate::row(&tr("settings-drives-user"), &user));
    add.append(&crate::row(&tr("settings-drives-name"), &name));
    let login = gtk::CheckButton::with_label(&tr("settings-drives-at-login-long"));
    add.append(&login);
    let go = gtk::Button::with_label(&tr("settings-drives-add-button"));
    go.add_css_class("suggested-action");
    go.set_halign(gtk::Align::End);
    let form = Form { kind, server, folder, user, name, login };
    go.connect_clicked({
        let page = page.clone();
        move |b| page.add(&form, b)
    });
    add.append(&go);
    body.append(&add);
    body.append(&system_card(&page.message));

    body.append(&page.message);
    page.fill();
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .build()
        .upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_made() {
        assert_eq!(address("smb", "nas.home.lan", "media/Músicas", "").as_deref(), Some("smb://nas.home.lan/media/M%C3%BAsicas"));
        assert_eq!(address("sftp", "10.0.0.5", "/srv/data", "ana").as_deref(), Some("sftp://ana@10.0.0.5/srv/data"));
        assert_eq!(address("smb", "nas", "", "").as_deref(), Some("smb://nas/"));
        assert!(address("smb", "", "x", "").is_none());
        assert!(address("smb", "a b", "x", "").is_none());
        assert!(remote("smb://nas/x"));
        assert!(!remote("file:///home"));
    }
}
