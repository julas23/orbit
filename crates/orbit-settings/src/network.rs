//! The Network page (docs/PLANO-SETTINGS.md §3.1). For now its VPN part:
//! this machine as a client of a WireGuard server that already exists (a
//! router's, a home server's), through NetworkManager, which speaks
//! WireGuard itself. Serving the tunnel is the server's job, not the
//! desktop's (the user, 2026-11-15).
//!
//! A tunnel comes from the `.conf` the server made for this machine: a file
//! imported, or its text pasted. NetworkManager keeps it as a system
//! connection (the private key root's alone), so adding and removing one
//! asks for an administrator's password (polkit); connecting and
//! disconnecting do not. Everything goes through `nmcli`, in a thread.

use std::{io::Write, os::unix::fs::OpenOptionsExt, path::Path, process::Command, rc::Rc};

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::{tr, tr_args};

/// A WireGuard connection of NetworkManager's.
#[derive(Debug, Clone, PartialEq)]
struct Tunnel {
    name: String,
    uuid: String,
    active: bool,
    autoconnect: bool,
    /// Its addresses inside the tunnel.
    address: String,
}

/// `nmcli -t` splits fields by `:`, escaping one inside a field as `\:`.
fn fields(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    out.last_mut().expect("one").push(next);
                }
            }
            ':' => out.push(String::new()),
            c => out.last_mut().expect("one").push(c),
        }
    }
    out
}

fn nmcli(args: &[&str]) -> Result<String, String> {
    let out = Command::new("nmcli")
        .args(args)
        .output()
        .map_err(|e| format!("nmcli: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("nmcli failed").trim().to_owned())
    }
}

/// The WireGuard connections NetworkManager knows.
fn tunnels() -> Result<Vec<Tunnel>, String> {
    let list = nmcli(&["-t", "-f", "NAME,UUID,TYPE,ACTIVE,AUTOCONNECT", "connection", "show"])?;
    let mut found = Vec::new();
    for line in list.lines() {
        let f = fields(line);
        if f.len() < 5 || f[2] != "wireguard" {
            continue;
        }
        let address = nmcli(&["-t", "-g", "ipv4.addresses", "connection", "show", "uuid", &f[1]])
            .unwrap_or_default()
            .trim()
            .replace("\\,", ",");
        found.push(Tunnel {
            name: f[0].clone(),
            uuid: f[1].clone(),
            active: f[3] == "yes",
            autoconnect: f[4] == "yes",
            address,
        });
    }
    Ok(found)
}

/// A name NetworkManager takes for a WireGuard interface: 1–15 letters,
/// digits, `-`, `_` or `.`.
fn valid_name(name: &str) -> bool {
    (1..=15).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// The interface name a file's name suggests (`wghome.conf` → `wghome`).
fn name_from(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("wg0");
    let clean: String = stem
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || "-_.".contains(*c))
        .take(15)
        .collect();
    if clean.is_empty() { "wg0".into() } else { clean }
}

/// Imports `text` (a WireGuard `.conf`) as the connection `name`, left
/// disconnected and not started on its own (nmcli connects it as it
/// imports): nmcli takes the interface's name from the file's.
fn import(name: &str, text: &str) -> Result<(), String> {
    if !valid_name(name) {
        return Err(tr("settings-net-bad-name"));
    }
    if !text.contains("[Interface]") || !text.contains("[Peer]") {
        return Err(tr("settings-net-not-wireguard"));
    }
    let dir = glib::user_runtime_dir().join(format!("orbit-settings-wg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{name}.conf"));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(text.as_bytes())
    })();
    let result = written.map_err(|e| e.to_string()).and_then(|()| {
        let path = path.to_string_lossy();
        nmcli(&["connection", "import", "type", "wireguard", "file", &path])?;
        nmcli(&["connection", "modify", "id", name, "connection.autoconnect", "no"])?;
        let _ = nmcli(&["connection", "down", "id", name]);
        Ok(())
    });
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);
    result
}

struct Page {
    list: gtk::Box,
    message: gtk::Label,
}

pub fn page() -> gtk::Widget {
    let page = Rc::new(Page {
        list: gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(15)
            .build(),
        message: crate::message(),
    });
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(15)
        .css_classes(["card"])
        .build();
    card.append(
        &gtk::Label::builder()
            .label(tr("settings-net-vpn"))
            .xalign(0.0)
            .css_classes(["heading"])
            .build(),
    );
    card.append(
        &gtk::Label::builder()
            .label(tr("settings-net-vpn-tip"))
            .xalign(0.0)
            .wrap(true)
            .css_classes(["dim"])
            .build(),
    );
    card.append(&page.list);
    let buttons = gtk::Box::builder().spacing(10).halign(gtk::Align::End).build();
    let paste = gtk::Button::with_label(&tr("settings-net-paste"));
    let open = gtk::Button::with_label(&tr("settings-net-import"));
    open.add_css_class("suggested-action");
    buttons.append(&paste);
    buttons.append(&open);
    card.append(&buttons);
    card.append(&page.message);
    open.connect_clicked({
        let page = page.clone();
        move |button| page.choose_file(button)
    });
    paste.connect_clicked({
        let page = page.clone();
        move |button| page.paste_dialog(button)
    });
    page.refresh();

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();
    body.append(&card);
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build()
        .upcast()
}

impl Page {
    /// Runs `work` in a thread; then shows its error, if any, and the list
    /// again.
    fn run(self: &Rc<Self>, work: impl FnOnce() -> Result<(), String> + Send + 'static) {
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(work());
        });
        let page = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(result) = rx.recv().await {
                crate::say(&page.message, &result.err().unwrap_or_default());
                page.refresh();
            }
        });
    }

    /// Reads the tunnels (in a thread) and shows them.
    fn refresh(self: &Rc<Self>) {
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(tunnels());
        });
        let page = self.clone();
        glib::spawn_future_local(async move {
            let Ok(found) = rx.recv().await else { return };
            while let Some(child) = page.list.first_child() {
                page.list.remove(&child);
            }
            match found {
                Ok(tunnels) if tunnels.is_empty() => page.list.append(
                    &gtk::Label::builder()
                        .label(tr("settings-net-none"))
                        .xalign(0.0)
                        .css_classes(["dim"])
                        .build(),
                ),
                Ok(tunnels) => {
                    for tunnel in tunnels {
                        page.list.append(&page.row(tunnel));
                    }
                }
                Err(e) => crate::say(&page.message, &e),
            }
        });
    }

    fn row(self: &Rc<Self>, tunnel: Tunnel) -> gtk::Box {
        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(3)
            .hexpand(true)
            .build();
        text.append(
            &gtk::Label::builder()
                .label(&tunnel.name)
                .xalign(0.0)
                .css_classes(["name"])
                .build(),
        );
        let state = tr(if tunnel.active { "settings-net-connected" } else { "settings-net-disconnected" });
        let detail = if tunnel.address.is_empty() { state } else { format!("{state} · {}", tunnel.address) };
        text.append(&gtk::Label::builder().label(detail).xalign(0.0).css_classes(["dim"]).build());
        let at_start = gtk::CheckButton::builder()
            .label(tr("settings-net-autoconnect"))
            .active(tunnel.autoconnect)
            .build();
        text.append(&at_start);

        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text(tr("settings-net-remove"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let switch = gtk::Switch::builder()
            .active(tunnel.active)
            .valign(gtk::Align::Center)
            .build();
        let line = gtk::Box::builder().spacing(10).css_classes(["service"]).build();
        line.append(&text);
        line.append(&remove);
        line.append(&switch);

        let uuid = tunnel.uuid.clone();
        switch.connect_active_notify({
            let page = self.clone();
            move |switch| {
                let verb = if switch.is_active() { "up" } else { "down" };
                let uuid = uuid.clone();
                switch.set_sensitive(false);
                page.run(move || nmcli(&["connection", verb, "uuid", &uuid]).map(|_| ()));
            }
        });
        let uuid = tunnel.uuid.clone();
        at_start.connect_toggled({
            let page = self.clone();
            move |check| {
                let value = if check.is_active() { "yes" } else { "no" };
                let uuid = uuid.clone();
                page.run(move || {
                    nmcli(&["connection", "modify", "uuid", &uuid, "connection.autoconnect", value]).map(|_| ())
                });
            }
        });
        remove.connect_clicked({
            let page = self.clone();
            move |button| {
                let dialog = gtk::AlertDialog::builder()
                    .message(tr_args("settings-net-remove-question", &[("name", tunnel.name.clone().into())]))
                    .detail(tr("settings-net-remove-detail"))
                    .buttons([tr("settings-svc-cancel"), tr("settings-net-remove")])
                    .cancel_button(0)
                    .default_button(0)
                    .modal(true)
                    .build();
                let window = button.root().and_downcast::<gtk::Window>();
                let page = page.clone();
                let uuid = tunnel.uuid.clone();
                dialog.choose(window.as_ref(), gio::Cancellable::NONE, move |answer| {
                    if answer == Ok(1) {
                        page.run(move || nmcli(&["connection", "delete", "uuid", &uuid]).map(|_| ()));
                    }
                });
            }
        });
        line
    }

    fn choose_file(self: &Rc<Self>, button: &gtk::Button) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(&tr("settings-net-conf-files")));
        filter.add_pattern("*.conf");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title(tr("settings-net-import"))
            .filters(&filters)
            .modal(true)
            .build();
        let window = button.root().and_downcast::<gtk::Window>();
        let page = self.clone();
        dialog.open(window.as_ref(), gio::Cancellable::NONE, move |file| {
            let Some(path) = file.ok().and_then(|f| f.path()) else { return };
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    let name = name_from(&path);
                    page.run(move || import(&name, &text));
                }
                Err(e) => crate::say(&page.message, &format!("{}: {e}", path.display())),
            }
        });
    }

    /// A dialog to paste a `.conf`'s text and name the tunnel.
    fn paste_dialog(self: &Rc<Self>, button: &gtk::Button) {
        let window = gtk::Window::builder()
            .title(tr("settings-net-paste"))
            .modal(true)
            .default_width(500)
            .default_height(420)
            .css_classes(["orbit-settings"])
            .build();
        if let Some(parent) = button.root().and_downcast::<gtk::Window>() {
            window.set_transient_for(Some(&parent));
        }
        let name = gtk::Entry::builder().text("wg0").max_length(15).build();
        let text = gtk::TextView::builder().monospace(true).wrap_mode(gtk::WrapMode::Char).build();
        let scroll = gtk::ScrolledWindow::builder().child(&text).vexpand(true).css_classes(["card"]).build();
        let message = crate::message();
        let cancel = gtk::Button::with_label(&tr("settings-svc-cancel"));
        let add = gtk::Button::with_label(&tr("settings-net-add"));
        add.add_css_class("suggested-action");
        let buttons = gtk::Box::builder().spacing(10).halign(gtk::Align::End).build();
        buttons.append(&cancel);
        buttons.append(&add);
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["page"])
            .build();
        body.append(&crate::row(&tr("settings-net-name"), &name));
        body.append(&gtk::Label::builder().label(tr("settings-net-paste-tip")).xalign(0.0).wrap(true).css_classes(["dim"]).build());
        body.append(&scroll);
        body.append(&message);
        body.append(&buttons);
        window.set_child(Some(&body));
        cancel.connect_clicked({
            let window = window.clone();
            move |_| window.close()
        });
        add.connect_clicked({
            let page = self.clone();
            let window = window.clone();
            move |_| {
                let buffer = text.buffer();
                let conf = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string();
                let tunnel = name.text().trim().to_owned();
                if !valid_name(&tunnel) {
                    crate::say(&message, &tr("settings-net-bad-name"));
                    return;
                }
                if !conf.contains("[Interface]") || !conf.contains("[Peer]") {
                    crate::say(&message, &tr("settings-net-not-wireguard"));
                    return;
                }
                window.close();
                page.run(move || import(&tunnel, &conf));
            }
        });
        window.present();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nmcli_fields() {
        assert_eq!(fields("wghome:abc-1:wireguard:yes:no"), ["wghome", "abc-1", "wireguard", "yes", "no"]);
        assert_eq!(fields(r"a\:b:c"), ["a:b", "c"]);
    }

    #[test]
    fn names() {
        assert!(valid_name("wghome"));
        assert!(!valid_name("a name"));
        assert!(!valid_name("sixteen-chars-xx"));
        assert_eq!(name_from(Path::new("/tmp/My Home.conf")), "MyHome");
        assert_eq!(name_from(Path::new("/tmp/!!.conf")), "wg0");
    }
}
