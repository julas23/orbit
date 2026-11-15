//! The Printers and scanners page (docs/PLANO-SETTINGS.md §3.4), through
//! CUPS's own tools (read in English, `LC_ALL=C`): the printers and their
//! state (`lpstat`); the default, the paper and two-sided printing, which
//! are each person's own (`lpoptions`, no password); the queue, cancelled
//! job by job; a test page; printers found on the network (`driverless`,
//! IPP Everywhere) or typed, added and removed (`lpadmin`, which the
//! lpadmin group may run, as Debian's first user is in it); "Advanced…",
//! system-config-printer. Scanners: what SANE finds, when sane-utils is
//! installed.

use std::{process::Command, rc::Rc};

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::tr;

/// Runs a CUPS tool; its output, or its last error line.
fn cups(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        Ok(text)
    } else {
        let errors = String::from_utf8_lossy(&out.stderr);
        Err(errors.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("failed").trim().to_owned())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Printer {
    name: String,
    /// `lpstat -p`'s state: idle, printing…, disabled.
    state: String,
    uri: String,
}

fn printers() -> Vec<Printer> {
    let names: Vec<String> = cups("lpstat", &["-e"]).unwrap_or_default().lines().map(|l| l.trim().to_owned()).filter(|l| !l.is_empty()).collect();
    let states = cups("lpstat", &["-p"]).unwrap_or_default();
    let uris = cups("lpstat", &["-v"]).unwrap_or_default();
    names
        .into_iter()
        .map(|name| Printer {
            state: state_of(&states, &name),
            uri: uris
                .lines()
                .find_map(|l| l.strip_prefix(&format!("device for {name}: ")))
                .unwrap_or_default()
                .to_owned(),
            name,
        })
        .collect()
}

/// "printer NAME is idle.  enabled since …" → "idle".
fn state_of(states: &str, name: &str) -> String {
    states
        .lines()
        .find_map(|l| l.strip_prefix(&format!("printer {name} ")))
        .map(|rest| {
            let rest = rest.trim_start_matches("is ").trim_start_matches("now ");
            rest.split(['.', ' ']).next().unwrap_or("").to_owned()
        })
        .unwrap_or_default()
}

fn default_printer() -> Option<String> {
    let out = cups("lpstat", &["-d"]).ok()?;
    out.lines().find_map(|l| l.split_once("destination: ")).map(|(_, n)| n.trim().to_owned())
}

/// An option of `lpoptions -l`: its name, its label, its choices and the
/// one in force (`*`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Choice {
    key: String,
    label: String,
    values: Vec<String>,
    current: usize,
}

fn choices(text: &str) -> Vec<Choice> {
    text.lines()
        .filter_map(|l| {
            let (head, values) = l.split_once(": ")?;
            let (key, label) = head.split_once('/').unwrap_or((head, head));
            let mut current = 0;
            let values: Vec<String> = values
                .split_whitespace()
                .enumerate()
                .map(|(i, v)| match v.strip_prefix('*') {
                    Some(v) => {
                        current = i;
                        v.to_owned()
                    }
                    None => v.to_owned(),
                })
                .collect();
            Some(Choice { key: key.to_owned(), label: label.to_owned(), values, current })
        })
        .collect()
}

/// Jobs waiting: (id, owner, size).
fn jobs(name: &str) -> Vec<(String, String, String)> {
    cups("lpstat", &["-o", name])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            Some((f.next()?.to_owned(), f.next()?.to_owned(), f.next()?.to_owned()))
        })
        .collect()
}

/// A queue's name from a printer's address or network name: letters,
/// digits, - and _.
fn queue_name(from: &str) -> String {
    let decoded = glib::Uri::unescape_string(from, None::<&str>).map(|s| s.to_string()).unwrap_or_else(|| from.to_owned());
    let host = decoded.split("://").nth(1).unwrap_or(&decoded);
    let host = host.split(['/', '?']).next().unwrap_or(host);
    // A network name ("HP LaserJet._ipp._tcp.local"): the part before
    // the service; an address: all of it.
    let base = host.split("._").next().unwrap_or(host);
    let mut out: String = base.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    out.truncate(40);
    let trimmed = out.trim_matches('_').to_owned();
    if trimmed.is_empty() { "printer".into() } else { trimmed }
}

struct Page {
    list: gtk::Box,
    found: gtk::Box,
    message: gtk::Label,
}

impl Page {
    fn say(&self, text: &str) {
        crate::say(&self.message, text);
    }

    /// Runs a CUPS tool in a thread; says its error, then shows the
    /// printers again.
    fn act(self: &Rc<Self>, program: &'static str, args: Vec<String>, done: Option<String>) {
        let page = self.clone();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                cups(program, &args)
            })
            .await
            .unwrap_or_else(|_| Err(program.into()));
            page.say(&match result {
                Ok(_) => done.unwrap_or_default(),
                Err(e) => e,
            });
            page.fill();
        });
    }

    fn fill(self: &Rc<Self>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let all = printers();
        if all.is_empty() {
            self.list.append(&dim(&tr("settings-print-none")));
        }
        let default = default_printer();
        for printer in all {
            let card = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).css_classes(["row"]).build();
            let top = gtk::Box::builder().spacing(10).build();
            let names = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
            let mut shown = printer.name.clone();
            if default.as_deref() == Some(printer.name.as_str()) {
                shown.push_str(&format!(" ({})", tr("settings-print-default")));
            }
            names.append(&gtk::Label::builder().label(shown).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build());
            names.append(
                &gtk::Label::builder()
                    .label(format!("{} · {}", printer.state, printer.uri))
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::Middle)
                    .css_classes(["dim"])
                    .build(),
            );
            top.append(&names);
            let name = printer.name.clone();
            let button = |icon: &str, tip: &str| {
                gtk::Button::builder().icon_name(icon).tooltip_text(tr(tip)).css_classes(["flat"]).valign(gtk::Align::Center).build()
            };
            let star = button("starred-symbolic", "settings-print-make-default");
            star.set_sensitive(default.as_deref() != Some(name.as_str()));
            let (page, n) = (self.clone(), name.clone());
            star.connect_clicked(move |_| page.act("lpoptions", vec!["-d".into(), n.clone()], None));
            top.append(&star);
            let test = button("document-print-symbolic", "settings-print-test");
            let (page, n) = (self.clone(), name.clone());
            test.connect_clicked(move |_| {
                page.act("lp", vec!["-d".into(), n.clone(), "/usr/share/cups/data/testprint".into()], Some(tr("settings-print-test-sent")))
            });
            top.append(&test);
            let remove = button("user-trash-symbolic", "settings-kb-remove");
            let (page, n) = (self.clone(), name.clone());
            remove.connect_clicked(move |_| page.act("lpadmin", vec!["-x".into(), n.clone()], None));
            top.append(&remove);
            card.append(&top);

            // Paper and two-sided printing: this person's own choices.
            let options = choices(&cups("lpoptions", &["-p", &name, "-l"]).unwrap_or_default());
            for key in ["PageSize", "media", "Duplex", "sides"] {
                let Some(choice) = options.iter().find(|c| c.key == key) else { continue };
                let drop = crate::short_dropdown(&choice.values);
                drop.set_selected(choice.current as u32);
                let (page, n, choice2) = (self.clone(), name.clone(), choice.clone());
                drop.connect_selected_notify(move |d| {
                    if let Some(v) = choice2.values.get(d.selected() as usize) {
                        page.act("lpoptions", vec!["-p".into(), n.clone(), "-o".into(), format!("{}={v}", choice2.key)], None);
                    }
                });
                card.append(&crate::row(&choice.label, &drop));
            }
            for (id, owner, size) in jobs(&name) {
                let line = gtk::Box::builder().spacing(10).build();
                line.append(&gtk::Label::builder().label(format!("{id} · {owner} · {size} B")).xalign(0.0).hexpand(true).css_classes(["dim"]).build());
                let cancel = button("process-stop-symbolic", "settings-print-cancel");
                let page = self.clone();
                cancel.connect_clicked(move |_| page.act("cancel", vec![id.clone()], None));
                line.append(&cancel);
                card.append(&line);
            }
            self.list.append(&card);
        }
    }

    /// Printers on the network (`driverless`, a few seconds), each with
    /// an Add.
    fn search(self: &Rc<Self>, button: &gtk::Button) {
        button.set_sensitive(false);
        while let Some(child) = self.found.first_child() {
            self.found.remove(&child);
        }
        self.found.append(&dim(&tr("settings-print-searching")));
        let (page, button) = (self.clone(), button.clone());
        glib::spawn_future_local(async move {
            let uris: Vec<String> = gio::spawn_blocking(|| cups("driverless", &[]))
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default()
                .lines()
                .map(|l| l.trim().to_owned())
                .filter(|l| !l.is_empty())
                .collect();
            button.set_sensitive(true);
            while let Some(child) = page.found.first_child() {
                page.found.remove(&child);
            }
            if uris.is_empty() {
                page.found.append(&dim(&tr("settings-print-found-none")));
            }
            let known: Vec<String> = printers().into_iter().map(|p| p.uri).collect();
            for uri in uris {
                let line = gtk::Box::builder().spacing(10).build();
                let shown = glib::Uri::unescape_string(&uri, None::<&str>).map(|s| s.to_string()).unwrap_or_else(|| uri.clone());
                line.append(&gtk::Label::builder().label(shown).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::Middle).build());
                let add = gtk::Button::with_label(&tr("settings-net-add"));
                add.set_sensitive(!known.contains(&uri));
                let p = page.clone();
                add.connect_clicked(move |_| p.add(&uri));
                line.append(&add);
                page.found.append(&line);
            }
        });
    }

    fn add(self: &Rc<Self>, uri: &str) {
        let name = queue_name(uri);
        self.act(
            "lpadmin",
            vec!["-p".into(), name, "-E".into(), "-v".into(), uri.to_owned(), "-m".into(), "everywhere".into()],
            Some(tr("settings-print-added")),
        );
    }
}

fn dim(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["dim"]).build()
}

pub fn page() -> gtk::Widget {
    let page = Rc::new(Page {
        list: gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build(),
        found: gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).build(),
        message: crate::message(),
    });
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();

    let printers = crate::power_card(&tr("settings-print-printers"));
    printers.append(&page.list);
    let advanced = gtk::Button::with_label(&tr("settings-print-advanced"));
    advanced.set_halign(gtk::Align::End);
    advanced.connect_clicked({
        let page = page.clone();
        move |_| {
            if let Err(e) = Command::new("system-config-printer").spawn() {
                page.say(&format!("system-config-printer: {e}"));
            }
        }
    });
    printers.append(&advanced);
    body.append(&printers);

    let add = crate::power_card(&tr("settings-print-add"));
    add.append(&dim(&tr("settings-print-add-tip")));
    let search = gtk::Button::with_label(&tr("settings-print-search"));
    search.set_halign(gtk::Align::End);
    search.connect_clicked({
        let page = page.clone();
        move |b| page.search(b)
    });
    add.append(&search);
    add.append(&page.found);
    let typed = gtk::Entry::builder().placeholder_text("ipp://10.0.0.20/ipp/print").hexpand(true).build();
    let go = gtk::Button::with_label(&tr("settings-net-add"));
    let line = gtk::Box::builder().spacing(10).build();
    line.append(&typed);
    line.append(&go);
    add.append(&crate::row(&tr("settings-print-address"), &line));
    go.connect_clicked({
        let page = page.clone();
        move |_| {
            let uri = typed.text().trim().to_owned();
            if uri.contains("://") {
                page.add(&uri);
                typed.set_text("");
            }
        }
    });
    body.append(&add);

    let scanners = crate::power_card(&tr("settings-print-scanners"));
    let found = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).build();
    scanners.append(&found);
    if glib::find_program_in_path("scanimage").is_some() {
        found.append(&dim(&tr("settings-print-searching")));
        glib::spawn_future_local(async move {
            let text = gio::spawn_blocking(|| cups("scanimage", &["-L"])).await.ok().and_then(Result::ok).unwrap_or_default();
            while let Some(child) = found.first_child() {
                found.remove(&child);
            }
            let lines: Vec<&str> = text.lines().filter(|l| l.starts_with("device")).collect();
            if lines.is_empty() {
                found.append(&dim(&tr("settings-print-no-scanner")));
            }
            for l in lines {
                found.append(&gtk::Label::builder().label(l.trim_start_matches("device ")).xalign(0.0).wrap(true).build());
            }
        });
    } else {
        found.append(&dim(&tr("settings-print-no-sane")));
    }
    body.append(&scanners);

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
    fn cups_output_is_read() {
        let states = "printer Office is idle.  enabled since Sat 15 Nov 2026\nprinter Lab is now printing Lab-3.  enabled since …\nprinter Old disabled since …\n";
        assert_eq!(state_of(states, "Office"), "idle");
        assert_eq!(state_of(states, "Lab"), "printing");
        assert_eq!(state_of(states, "Old"), "disabled");
        let c = choices("PageSize/Media Size: Letter *A4 Legal\nDuplex/2-Sided Printing: *None DuplexNoTumble DuplexTumble\n");
        assert_eq!(c[0].key, "PageSize");
        assert_eq!(c[0].label, "Media Size");
        assert_eq!(c[0].values[c[0].current], "A4");
        assert_eq!(c[1].values.len(), 3);
        assert_eq!(queue_name("ipp://HP%20LaserJet%20M15w._ipp._tcp.local/"), "HP_LaserJet_M15w");
        assert_eq!(queue_name("ipp://10.0.0.20/ipp/print"), "10_0_0_20");
        assert_eq!(queue_name("ipp://printer.home.lan:631/ipp/print"), "printer_home_lan_631");
    }
}
