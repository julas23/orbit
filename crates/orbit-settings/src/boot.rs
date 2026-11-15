//! The Boot page (docs/PLANO-SETTINGS.md §3.4): systemd-boot's menu (how
//! long it shows, the entry started by default) and the system's
//! snapshots (snapper's, root's), read and changed by orbit-services-helper
//! (the ESP and snapper are root's: reading asks for the password too, so
//! the page reads when asked). Restarting into the boot menu needs no
//! password (logind). Going back to a snapshot comes with Orbit Linux's
//! boot hook (docs/PLANO-DISTRO.md §6.4), not yet.

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
        self.read.set_sensitive(false);
        crate::say(&self.message, &tr("settings-boot-reading"));
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
                crate::say(&page.message, &error.unwrap_or_default());
                if let Some(info) = info.borrow().as_ref() {
                    page.show(info);
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
            line.append(
                &gtk::Label::builder()
                    .label(format!("{what} · {kind}"))
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .css_classes(["dim"])
                    .build(),
            );
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

fn dim(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["dim"]).build()
}

/// Restarts into the boot menu, asked first (logind: no password).
fn restart_to_menu(over: &gtk::Button, message: &gtk::Label) {
    let ask = gtk::AlertDialog::builder()
        .message(tr("settings-boot-restart-ask"))
        .detail(tr("settings-boot-restart-detail"))
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
        let result = gio::spawn_blocking(|| {
            std::process::Command::new("systemctl").args(["reboot", "--boot-loader-menu=30"]).output()
        })
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
