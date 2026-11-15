//! The Updates page (docs/PLANO-SETTINGS.md §3.4): what an upgrade would
//! bring (apt's simulation, no root), fetching the lists again and
//! installing everything (orbit-services-helper's `refresh` and
//! `upgrade`, apt's progress shown), what unattended-upgrades installs
//! alone, the repositories, and what apt did lately
//! (orbit_services::updates).

use std::rc::Rc;

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::{tr, tr_args};
use orbit_services::{
    Event, Privileged,
    updates::{self, AUTO_MODES},
};

const AUTO_LABELS: [&str; 3] = ["settings-upd-auto-off", "settings-upd-auto-security", "settings-upd-auto-all"];

struct Page {
    status: gtk::Label,
    progress: gtk::ProgressBar,
    list: gtk::Box,
    check: gtk::Button,
    install: gtk::Button,
    history: gtk::Box,
    message: gtk::Label,
}

impl Page {
    /// Reads what an upgrade would bring, in a thread.
    fn read(self: &Rc<Self>) {
        self.status.set_label(&tr("settings-upd-reading"));
        let page = self.clone();
        glib::spawn_future_local(async move {
            let pending = gio::spawn_blocking(updates::pending).await.unwrap_or_else(|_| Err("apt-get".into()));
            page.show(pending);
        });
    }

    fn show(&self, pending: Result<Vec<updates::Upgrade>, String>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let all = match pending {
            Ok(all) => all,
            Err(e) => {
                self.status.set_label(&e);
                self.install.set_sensitive(false);
                return;
            }
        };
        self.status.set_label(&if all.is_empty() {
            tr("settings-upd-none")
        } else {
            tr_args("settings-upd-count", &[("count", (all.len() as i64).into())])
        });
        self.install.set_sensitive(!all.is_empty());
        for u in all {
            let line = gtk::Box::builder().spacing(10).build();
            line.append(&gtk::Label::builder().label(u.package.as_str()).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build());
            let versions = if u.from.is_empty() { u.to.clone() } else { format!("{} → {}", u.from, u.to) };
            line.append(&gtk::Label::builder().label(versions).xalign(1.0).ellipsize(gtk::pango::EllipsizeMode::Middle).max_width_chars(36).css_classes(["dim"]).build());
            self.list.append(&line);
        }
    }

    fn fill_history(&self) {
        while let Some(child) = self.history.first_child() {
            self.history.remove(&child);
        }
        let all = updates::history(12);
        if all.is_empty() {
            self.history.append(&dim(&tr("settings-panel-empty")));
        }
        for t in all {
            let line = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["row"]).build();
            let changes: Vec<String> = t
                .changes
                .iter()
                .map(|(kind, n)| tr_args(&format!("settings-upd-did-{}", kind.to_lowercase()), &[("count", (*n as i64).into())]))
                .collect();
            line.append(&gtk::Label::builder().label(format!("{} · {}", t.date, changes.join(", "))).xalign(0.0).build());
            if !t.command.is_empty() {
                line.append(
                    &gtk::Label::builder()
                        .label(t.command.as_str())
                        .xalign(0.0)
                        .ellipsize(gtk::pango::EllipsizeMode::End)
                        .css_classes(["dim", "monospace"])
                        .build(),
                );
            }
            self.history.append(&line);
        }
    }

    /// Runs `steps` with the helper, apt's progress on the bar; then
    /// reads everything again.
    fn run(self: &Rc<Self>, steps: Vec<Privileged>) {
        self.check.set_sensitive(false);
        self.install.set_sensitive(false);
        self.progress.set_visible(true);
        self.progress.set_fraction(0.0);
        crate::say(&self.message, "");
        let (page, bar) = (self.clone(), self.progress.clone());
        crate::helper::run(
            steps,
            move |event| match event {
                Event::Step(text) => bar.set_text(Some(text)),
                Event::Progress(p, text) => {
                    bar.set_fraction(f64::from(*p) / 100.0);
                    bar.set_text(Some(text));
                }
                _ => {}
            },
            move |error| {
                crate::say(&page.message, &error.unwrap_or_default());
                page.progress.set_visible(false);
                page.check.set_sensitive(true);
                page.read();
                page.fill_history();
            },
        );
    }
}

fn dim(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["dim"]).build()
}

pub fn page() -> gtk::Widget {
    let page = Rc::new(Page {
        status: gtk::Label::builder().xalign(0.0).wrap(true).hexpand(true).build(),
        progress: gtk::ProgressBar::builder().show_text(true).visible(false).build(),
        list: gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(3).build(),
        check: gtk::Button::with_label(&tr("settings-upd-check")),
        install: gtk::Button::builder().label(tr("settings-upd-install")).sensitive(false).css_classes(["suggested-action"]).build(),
        history: gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).build(),
        message: crate::message(),
    });
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();

    let now = crate::power_card(&tr("settings-upd-now"));
    now.append(&page.status);
    let buttons = gtk::Box::builder().spacing(10).halign(gtk::Align::End).build();
    buttons.append(&page.check);
    buttons.append(&page.install);
    now.append(&buttons);
    now.append(&page.progress);
    let expander = gtk::Expander::builder().label(tr("settings-upd-which")).child(&page.list).build();
    now.append(&expander);
    page.check.connect_clicked({
        let page = page.clone();
        move |_| page.run(vec![Privileged::Refresh])
    });
    page.install.connect_clicked({
        let page = page.clone();
        move |_| page.run(vec![Privileged::Refresh, Privileged::Upgrade])
    });
    body.append(&now);

    let auto = crate::power_card(&tr("settings-upd-auto"));
    let labels: Vec<String> = AUTO_LABELS.iter().map(|l| tr(l)).collect();
    let mode = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let current = updates::auto_mode();
    mode.set_selected(AUTO_MODES.iter().position(|m| *m == current).unwrap_or(0) as u32);
    mode.connect_selected_notify({
        let page = page.clone();
        move |d| {
            if let Some(m) = AUTO_MODES.get(d.selected() as usize) {
                page.run(vec![Privileged::AutoUpdates((*m).to_owned())]);
            }
        }
    });
    auto.append(&crate::row(&tr("settings-upd-auto-what"), &mode));
    auto.append(&dim(&tr("settings-upd-auto-tip")));
    body.append(&auto);

    let repos = crate::power_card(&tr("settings-upd-repos"));
    for source in updates::sources() {
        let line = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["row"]).build();
        let mut uri = source.uri.clone();
        if !source.enabled {
            uri.push_str(&format!(" ({})", tr("settings-upd-off")));
        }
        line.append(&gtk::Label::builder().label(uri).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build());
        line.append(&dim(&format!("{} · {}", source.what, source.file)));
        if !source.enabled {
            line.add_css_class("dim");
        }
        repos.append(&line);
    }
    body.append(&repos);

    let history = crate::power_card(&tr("settings-upd-history"));
    history.append(&page.history);
    body.append(&history);
    body.append(&page.message);

    page.read();
    page.fill_history();
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .build()
        .upcast()
}
