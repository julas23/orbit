//! The Boot page (docs/PLANO-SETTINGS.md §3.4): systemd-boot's menu (how
//! long it shows, the entry started by default) and the system's
//! snapshots (snapper's, root's), read and changed by orbit-services-helper
//! (the ESP and snapper are root's: reading asks for the password too, so
//! the page reads when asked). Restarting into the boot menu needs no
//! password (logind). The boot theme is Plymouth's: the themes installed
//! are read here, choosing one goes through the helper (the initramfs is
//! made again). Going back to a snapshot is the helper's `rollback`
//! (snapper's, the UKIs made again from the snapshot's kernels, and an
//! entry in the boot menu for the system left behind; docs/PLANO-DISTRO.md
//! §6.4); started from that entry, the page offers keeping it.

use std::rc::Rc;

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::{tr, tr_args};
use orbit_services::{BOOT_TIMEOUTS, Event, Privileged};
use serde_json::Value;

struct Page {
    content: gtk::Box,
    read: gtk::Button,
    message: gtk::Label,
}

impl Page {
    /// Reads the boot settings and the snapshots (the helper), and shows
    /// them.
    fn read(self: &Rc<Self>, more: Vec<Privileged>) {
        self.read_then(more, &tr("settings-boot-reading"), || {});
    }

    /// `more` and then the reading, `after` told once it went well.
    fn read_then(self: &Rc<Self>, more: Vec<Privileged>, doing: &str, after: impl FnOnce() + 'static) {
        self.read.set_sensitive(false);
        crate::say(&self.message, doing);
        let info: Rc<std::cell::RefCell<Option<Value>>> = Rc::default();
        let mut steps = more;
        steps.push(Privileged::BootInfo);
        let (page, found) = (self.clone(), info.clone());
        crate::helper::run(
            steps,
            move |event| {
                if let Event::Info(text) = event {
                    *found.borrow_mut() = serde_json::from_str(text).ok();
                }
            },
            move |error| {
                page.read.set_sensitive(true);
                let ok = error.is_none();
                crate::say(&page.message, &error.unwrap_or_default());
                if let Some(info) = info.borrow().as_ref() {
                    page.show(info);
                }
                if ok {
                    after();
                }
            },
        );
    }

    fn show(self: &Rc<Self>, info: &Value) {
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        let menu = crate::power_card(&tr("settings-boot-menu"));
        let labels: Vec<String> = BOOT_TIMEOUTS
            .iter()
            .map(|(s, _)| if *s == 0 { tr("settings-boot-hidden") } else { tr_args("settings-boot-seconds", &[("seconds", i64::from(*s).into())]) })
            .collect();
        let timeout = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
        let now: u32 = info["timeout"].as_str().and_then(|t| t.parse().ok()).unwrap_or(0);
        timeout.set_selected(BOOT_TIMEOUTS.iter().position(|(s, _)| *s == now).unwrap_or(0) as u32);
        let page = self.clone();
        timeout.connect_selected_notify(move |d| {
            if let Some((s, _)) = BOOT_TIMEOUTS.get(d.selected() as usize) {
                page.read(vec![Privileged::BootTimeout(*s)]);
            }
        });
        let row = crate::row(&tr("settings-boot-timeout"), &timeout);
        row.set_tooltip_text(Some(&tr("settings-boot-timeout-tip")));
        menu.append(&row);

        let entries: Vec<&Value> = info["entries"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
        if entries.is_empty() {
            menu.append(&dim(&tr("settings-boot-no-entries")));
        } else {
            let ids: Vec<String> = entries.iter().map(|e| e["id"].as_str().unwrap_or("").to_owned()).collect();
            let labels: Vec<String> = entries
                .iter()
                .map(|e| {
                    let title = e["showTitle"].as_str().or(e["title"].as_str()).unwrap_or("?");
                    match e["version"].as_str() {
                        Some(v) if !title.contains(v) => format!("{title} ({v})"),
                        _ => title.to_owned(),
                    }
                })
                .collect();
            let default = crate::short_dropdown(&labels);
            default.set_selected(entries.iter().position(|e| e["isDefault"].as_bool() == Some(true)).unwrap_or(0) as u32);
            let page = self.clone();
            default.connect_selected_notify(move |d| {
                if let Some(id) = ids.get(d.selected() as usize).filter(|i| !i.is_empty()) {
                    page.read(vec![Privileged::BootDefault(id.clone())]);
                }
            });
            menu.append(&crate::row(&tr("settings-boot-default"), &default));
        }
        self.content.append(&menu);

        let snapshots = crate::power_card(&tr("settings-boot-snapshots"));
        snapshots.append(&dim(&tr("settings-boot-snapshots-tip")));
        let root = &info["root"];
        let can_go_back = root["rollback"].as_bool() == Some(true);
        let elsewhere = root["running"].is_string() && root["default"].is_string() && root["running"] != root["default"];
        if elsewhere {
            // Started from the boot menu's entry for the system from before
            // going back (or a subvolume by hand).
            let note = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).css_classes(["card"]).build();
            note.append(&gtk::Label::builder().label(tr("settings-boot-elsewhere")).xalign(0.0).wrap(true).build());
            if can_go_back {
                let keep = gtk::Button::builder().label(tr("settings-boot-keep-this")).halign(gtk::Align::End).build();
                let page = self.clone();
                keep.connect_clicked(move |b| page.go_back(b, "current", None));
                note.append(&keep);
            }
            snapshots.append(&note);
        } else if root["previous"].as_bool() == Some(true) {
            snapshots.append(&dim(&tr("settings-boot-previous")));
        }
        if !can_go_back && !info["snapshots"].is_null() {
            snapshots.append(&dim(&tr("settings-boot-no-rollback")));
        }
        let list: Vec<&Value> = info["snapshots"]["root"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
        let shown: Vec<&&Value> = list.iter().filter(|s| s["number"].as_u64().unwrap_or(0) > 0).collect();
        if shown.is_empty() {
            snapshots.append(&dim(&tr(if info["snapshots"].is_null() { "settings-boot-no-snapper" } else { "settings-panel-empty" })));
        }
        for s in shown.iter().rev().take(30) {
            let line = gtk::Box::builder().spacing(10).build();
            line.append(&gtk::Label::builder().label(format!("#{}", s["number"])).width_chars(5).xalign(0.0).css_classes(["dim"]).build());
            line.append(&gtk::Label::builder().label(s["date"].as_str().unwrap_or("")).xalign(0.0).build());
            let what = s["description"].as_str().unwrap_or("");
            let kind = s["type"].as_str().unwrap_or("");
            let mut text = format!("{what} · {kind}");
            if s["default"].as_bool() == Some(true) {
                text = format!("{text} · {}", tr("settings-boot-starts"));
            }
            line.append(
                &gtk::Label::builder()
                    .label(text)
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .css_classes(["dim"])
                    .build(),
            );
            if can_go_back && s["default"].as_bool() != Some(true) {
                let back = gtk::Button::builder()
                    .icon_name("edit-undo-symbolic")
                    .tooltip_text(tr("settings-boot-go-back"))
                    .css_classes(["flat"])
                    .build();
                let (page, number, date) = (self.clone(), s["number"].to_string(), s["date"].as_str().unwrap_or("").to_owned());
                back.connect_clicked(move |b| page.go_back(b, &number, Some(&date)));
                line.append(&back);
            }
            snapshots.append(&line);
        }
        let description = gtk::Entry::builder().placeholder_text(tr("settings-boot-snapshot-what")).hexpand(true).max_length(80).build();
        let take = gtk::Button::with_label(&tr("settings-boot-snapshot-take"));
        let line = gtk::Box::builder().spacing(10).build();
        line.append(&description);
        line.append(&take);
        snapshots.append(&line);
        let page = self.clone();
        take.connect_clicked(move |_| {
            let mut text = description.text().trim().to_owned();
            if text.is_empty() {
                text = tr("settings-boot-snapshot-default");
            }
            page.read(vec![Privileged::Snapshot(text)]);
        });
        self.content.append(&snapshots);
    }
}

impl Page {
    /// The system back to snapshot `number` (or `current`: this one kept),
    /// asked first; a restart offered after.
    fn go_back(self: &Rc<Self>, over: &gtk::Button, number: &str, date: Option<&str>) {
        let (message, detail) = match date {
            Some(date) => (
                tr_args("settings-boot-go-back-ask", &[("number", number.into())]),
                tr_args("settings-boot-go-back-detail", &[("date", date.into())]),
            ),
            None => (tr("settings-boot-keep-ask"), tr("settings-boot-keep-detail")),
        };
        let ask = gtk::AlertDialog::builder()
            .message(message)
            .detail(detail)
            .buttons([tr("settings-svc-cancel"), tr(if date.is_some() { "settings-boot-go-back" } else { "settings-boot-keep-this" })])
            .cancel_button(0)
            .default_button(0)
            .modal(true)
            .build();
        let window = over.root().and_downcast::<gtk::Window>();
        let (page, number) = (self.clone(), number.to_owned());
        glib::spawn_future_local(async move {
            if ask.choose_future(window.as_ref()).await != Ok(1) {
                return;
            }
            let done = page.clone();
            page.read_then(vec![Privileged::Rollback(number)], &tr("settings-boot-going-back"), move || {
                restart_asking(&done.read, &done.message, &tr("settings-boot-gone-back"), &["reboot"]);
            });
        });
    }
}

const THEMES: &str = "/usr/share/plymouth/themes";

/// Plymouth's themes installed, by name.
fn themes() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(THEMES)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            e.path().join(format!("{name}.plymouth")).is_file().then_some(name)
        })
        .collect();
    out.sort();
    out
}

/// The theme in use (`plymouth-set-default-theme` with no argument).
fn theme() -> Option<String> {
    let out = std::process::Command::new("plymouth-set-default-theme").output().ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (out.status.success() && !name.is_empty()).then_some(name)
}

/// The boot theme's card, or none without Plymouth.
fn theme_card(message: &gtk::Label) -> Option<gtk::Box> {
    let all = themes();
    if all.is_empty() {
        return None;
    }
    let card = crate::power_card(&tr("settings-boot-theme"));
    let now = theme();
    let pick = crate::short_dropdown(&all);
    pick.set_selected(now.as_ref().and_then(|n| all.iter().position(|t| t == n)).unwrap_or(0) as u32);
    let busy = gtk::Spinner::new();
    let line = gtk::Box::builder().spacing(10).build();
    line.append(&busy);
    line.append(&pick);
    let message = message.clone();
    let chosen = std::cell::RefCell::new(now);
    pick.connect_selected_notify(move |d| {
        let Some(name) = all.get(d.selected() as usize).cloned() else { return };
        if chosen.borrow().as_ref() == Some(&name) {
            return;
        }
        *chosen.borrow_mut() = Some(name.clone());
        d.set_sensitive(false);
        busy.start();
        crate::say(&message, &tr("settings-boot-theme-making"));
        let (d, busy, message) = (d.clone(), busy.clone(), message.clone());
        crate::helper::run(vec![Privileged::BootTheme(name)], |_| {}, move |error| {
            d.set_sensitive(true);
            busy.stop();
            crate::say(&message, &error.unwrap_or_default());
        });
    });
    card.append(&crate::row(&tr("settings-boot-theme-row"), &line));
    card.append(&dim(&tr("settings-boot-theme-tip")));
    Some(card)
}

fn dim(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["dim"]).build()
}

/// Restarts into the boot menu, asked first (logind: no password).
fn restart_to_menu(over: &gtk::Button, message: &gtk::Label) {
    restart_asking(over, message, &tr("settings-boot-restart-detail"), &["reboot", "--boot-loader-menu=30"]);
}

/// Restarts (`systemctl` with `how`), asked first; `detail` says why.
fn restart_asking(over: &gtk::Button, message: &gtk::Label, detail: &str, how: &'static [&'static str]) {
    let ask = gtk::AlertDialog::builder()
        .message(tr("settings-boot-restart-ask"))
        .detail(detail)
        .buttons([tr("settings-svc-cancel"), tr("settings-boot-restart")])
        .cancel_button(0)
        .default_button(0)
        .modal(true)
        .build();
    let window = over.root().and_downcast::<gtk::Window>();
    let message = message.clone();
    glib::spawn_future_local(async move {
        if ask.choose_future(window.as_ref()).await != Ok(1) {
            return;
        }
        let result = gio::spawn_blocking(move || std::process::Command::new("systemctl").args(how).output())
        .await;
        if let Ok(Ok(out)) = result
            && !out.status.success()
        {
            crate::say(&message, String::from_utf8_lossy(&out.stderr).trim());
        }
    });
}

pub fn page() -> gtk::Widget {
    let page = Rc::new(Page {
        content: gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(20).build(),
        read: gtk::Button::with_label(&tr("settings-boot-read")),
        message: crate::message(),
    });
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();
    let top = crate::power_card(&tr("settings-boot"));
    top.append(&dim(&tr("settings-boot-tip")));
    let restart = gtk::Button::with_label(&tr("settings-boot-restart-menu"));
    let buttons = gtk::Box::builder().spacing(10).halign(gtk::Align::End).build();
    buttons.append(&restart);
    buttons.append(&page.read);
    top.append(&buttons);
    body.append(&top);
    if let Some(card) = theme_card(&page.message) {
        body.append(&card);
    }
    body.append(&page.content);
    body.append(&page.message);
    page.read.connect_clicked({
        let page = page.clone();
        move |_| page.read(Vec::new())
    });
    restart.connect_clicked({
        let message = page.message.clone();
        move |b| restart_to_menu(b, &message)
    });
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .build()
        .upcast()
}
