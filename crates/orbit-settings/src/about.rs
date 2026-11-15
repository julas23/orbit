//! The About page (docs/PLANO-SETTINGS.md §3.3): this machine's name (set
//! through hostnamed), its hardware and the versions of what it runs, and a
//! report of all of it to paste into a bug.

use std::{fs, process::Command, rc::Rc};

use gtk::{gdk, glib, prelude::*};
use orbit_app::i18n::tr;

use crate::dbus;

/// What the page shows, read once (in a thread: lsblk and dpkg-query).
#[derive(Debug, Clone, Default)]
struct Facts {
    hostname: String,
    pretty: String,
    os: String,
    orbit: String,
    debian: String,
    kernel: String,
    vendor: String,
    model: String,
    cpu: String,
    memory: String,
    gpus: Vec<String>,
    disks: Vec<String>,
}

impl Facts {
    /// The lines of the report, `label: value`.
    fn report(&self) -> String {
        let mut lines = vec![
            format!("{}: {}", tr("settings-about-name"), self.shown_name()),
            format!("{}: {}", tr("settings-about-system"), self.os),
            format!("Orbit: {}", self.orbit),
            format!("Debian: {}", self.debian),
            format!("{}: {}", tr("settings-about-kernel"), self.kernel),
            format!("{}: {} {}", tr("settings-about-model"), self.vendor, self.model),
            format!("{}: {}", tr("settings-about-cpu"), self.cpu),
            format!("{}: {}", tr("settings-about-memory"), self.memory),
        ];
        for gpu in &self.gpus {
            lines.push(format!("{}: {gpu}", tr("settings-about-gpu")));
        }
        for disk in &self.disks {
            lines.push(format!("{}: {disk}", tr("settings-about-disk")));
        }
        lines.join("\n")
    }

    fn shown_name(&self) -> &str {
        if self.pretty.is_empty() { &self.hostname } else { &self.pretty }
    }
}

fn read(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_default().trim().to_owned()
}

/// The CPU's model, and how many threads.
fn cpu() -> String {
    let info = read("/proc/cpuinfo");
    let model = info
        .lines()
        .find_map(|l| l.strip_prefix("model name").and_then(|r| r.split_once(':')).map(|(_, v)| v.trim()))
        .unwrap_or("?");
    let threads = info.lines().filter(|l| l.starts_with("processor")).count();
    format!("{model} × {threads}")
}

fn memory() -> String {
    let kb: u64 = read("/proc/meminfo")
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))
        .and_then(|v| v.split_whitespace().next()?.parse().ok())
        .unwrap_or(0);
    format!("{:.1} GiB", kb as f64 / 1024.0 / 1024.0)
}

/// The name pci.ids gives `vendor:device`, from the lines under the vendor.
fn pci_name(ids: &str, vendor: &str, device: &str) -> Option<String> {
    let mut vendor_name = None;
    for line in ids.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if !line.starts_with('\t') {
            if vendor_name.is_some() {
                break;
            }
            if let Some(name) = line.strip_prefix(vendor).and_then(|r| r.strip_prefix("  ")) {
                vendor_name = Some(name.trim().to_owned());
            }
            continue;
        }
        if let Some(v) = &vendor_name
            && let Some(name) = line.strip_prefix('\t').and_then(|r| r.strip_prefix(device)).and_then(|r| r.strip_prefix("  "))
        {
            return Some(format!("{v} {}", name.trim()));
        }
    }
    vendor_name
}

/// The GPUs, from /sys/class/drm's cards and pci.ids.
fn gpus() -> Vec<String> {
    let ids = ["/usr/share/misc/pci.ids", "/usr/share/hwdata/pci.ids"]
        .iter()
        .find_map(|p| fs::read_to_string(p).ok())
        .unwrap_or_default();
    let mut found = Vec::new();
    let Ok(cards) = fs::read_dir("/sys/class/drm") else { return found };
    let mut cards: Vec<_> = cards
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("card") && n[4..].chars().all(|c| c.is_ascii_digit()))
        })
        .collect();
    cards.sort();
    for card in cards {
        let uevent = fs::read_to_string(card.join("device/uevent")).unwrap_or_default();
        let field = |key: &str| uevent.lines().find_map(|l| l.strip_prefix(key)).unwrap_or("").to_owned();
        let (driver, pci) = (field("DRIVER="), field("PCI_ID="));
        let Some((vendor, device)) = pci.split_once(':') else { continue };
        let name = pci_name(&ids, &vendor.to_lowercase(), &device.to_lowercase()).unwrap_or(pci.clone());
        found.push(if driver.is_empty() { name } else { format!("{name} ({driver})") });
    }
    found
}

/// The disks (not partitions, loops, or zram: swap in memory): size and
/// model.
fn disks() -> Vec<String> {
    let out = Command::new("lsblk")
        .args(["-d", "-n", "-o", "NAME,SIZE,TYPE,MODEL"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    out.lines()
        .filter_map(|l| {
            let mut words = l.split_whitespace();
            let (name, size, kind) = (words.next()?, words.next()?, words.next()?);
            let model: Vec<&str> = words.collect();
            (kind == "disk" && !name.starts_with("zram"))
                .then(|| format!("{name}: {size} {}", model.join(" ")).trim().to_owned())
        })
        .collect()
}

fn os_release() -> String {
    read("/etc/os-release")
        .lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim_matches('"').to_owned())
        .unwrap_or_default()
}

fn orbit_version() -> String {
    Command::new("dpkg-query")
        .args(["-W", "-f", "${Version}", "orbit-desktop"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_else(|| "?".into())
}

/// Everything but the hostnames (the proxy's, on the main thread).
fn read_facts() -> Facts {
    Facts {
        os: os_release(),
        orbit: orbit_version(),
        debian: read("/etc/debian_version"),
        kernel: read("/proc/sys/kernel/osrelease"),
        vendor: read("/sys/class/dmi/id/sys_vendor"),
        model: read("/sys/class/dmi/id/product_name"),
        cpu: cpu(),
        memory: memory(),
        gpus: gpus(),
        disks: disks(),
        ..Facts::default()
    }
}

/// A hostname from what the user typed: lowercase letters, digits and
/// hyphens, as hostnamed wants a static name to be.
fn static_name(typed: &str) -> String {
    let name: String = crate::pages::fold(typed.trim())
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    name.trim_matches('-').chars().take(63).collect()
}

pub fn page() -> gtk::Widget {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();
    let proxy = dbus::system("org.freedesktop.hostname1");
    let name = gtk::Entry::builder().width_chars(24).build();
    let save = gtk::Button::with_label(&tr("settings-about-rename"));
    let message = crate::message();
    let name_line = gtk::Box::builder().spacing(10).build();
    name_line.append(&name);
    name_line.append(&save);

    let head = gtk::Box::builder().spacing(15).css_classes(["card"]).build();
    let mark = gtk::Image::from_icon_name("orbit-linux");
    mark.set_pixel_size(64);
    head.append(&mark);
    let head_text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).hexpand(true).build();
    let os = gtk::Label::builder().xalign(0.0).css_classes(["heading"]).build();
    head_text.append(&os);
    head_text.append(&crate::row(&tr("settings-about-name"), &name_line));
    head_text.append(&message);
    head.append(&head_text);
    body.append(&head);

    let details = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(5)
        .css_classes(["card"])
        .build();
    body.append(&details);
    let copy = gtk::Button::with_label(&tr("settings-about-copy"));
    copy.set_halign(gtk::Align::End);
    body.append(&copy);

    let facts: Rc<std::cell::RefCell<Facts>> = Rc::default();
    if let Some(proxy) = &proxy {
        let mut f = facts.borrow_mut();
        f.hostname = dbus::string(proxy, "Hostname");
        f.pretty = dbus::string(proxy, "PrettyHostname");
        name.set_text(f.shown_name());
    }
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(read_facts());
    });
    glib::spawn_future_local({
        let facts = facts.clone();
        let details = details.clone();
        let os = os.clone();
        async move {
            let Ok(read) = rx.recv().await else { return };
            {
                let mut f = facts.borrow_mut();
                let (hostname, pretty) = (std::mem::take(&mut f.hostname), std::mem::take(&mut f.pretty));
                *f = Facts { hostname, pretty, ..read };
            }
            let f = facts.borrow();
            os.set_label(&f.os);
            let mut rows = vec![
                (tr("settings-about-model"), format!("{} {}", f.vendor, f.model).trim().to_owned()),
                (tr("settings-about-cpu"), f.cpu.clone()),
                (tr("settings-about-memory"), f.memory.clone()),
            ];
            rows.extend(f.gpus.iter().map(|g| (tr("settings-about-gpu"), g.clone())));
            rows.extend(f.disks.iter().map(|d| (tr("settings-about-disk"), d.clone())));
            rows.push(("Orbit".into(), f.orbit.clone()));
            rows.push(("Debian".into(), f.debian.clone()));
            rows.push((tr("settings-about-kernel"), f.kernel.clone()));
            for (label, value) in rows {
                let value = gtk::Label::builder()
                    .label(value)
                    .xalign(1.0)
                    .wrap(true)
                    .selectable(true)
                    .css_classes(["dim"])
                    .build();
                details.append(&crate::row(&label, &value));
            }
        }
    });

    copy.connect_clicked({
        let facts = facts.clone();
        move |button| {
            if let Some(display) = gdk::Display::default() {
                display.clipboard().set_text(&facts.borrow().report());
                button.set_label(&tr("settings-about-copied"));
            }
        }
    });
    save.connect_clicked({
        let facts = facts.clone();
        let name = name.clone();
        let message = message.clone();
        move |_| {
            let Some(proxy) = proxy.clone() else { return };
            let typed = name.text().trim().to_owned();
            let host = static_name(&typed);
            if host.is_empty() {
                crate::say(&message, &tr("settings-about-bad-name"));
                return;
            }
            let facts = facts.clone();
            let message = message.clone();
            glib::spawn_future_local(async move {
                let result = async {
                    dbus::call(&proxy, "SetStaticHostname", (host.as_str(), true).to_variant()).await?;
                    dbus::call(&proxy, "SetPrettyHostname", (typed.as_str(), true).to_variant()).await?;
                    Ok::<_, String>(())
                }
                .await;
                match result {
                    Ok(()) => {
                        let mut f = facts.borrow_mut();
                        f.hostname = host;
                        f.pretty = typed;
                        crate::say(&message, "");
                    }
                    Err(e) => crate::say(&message, &e),
                }
            });
        }
    });
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
    fn names_from_pci_ids() {
        let ids = "# comment\n8086  Intel Corporation\n\t3e92  CoffeeLake-S GT2 [UHD Graphics 630]\n1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\t743f  Navi 24 [Radeon RX 6400/6500 XT/6500M]\n";
        assert_eq!(
            pci_name(ids, "8086", "3e92").as_deref(),
            Some("Intel Corporation CoffeeLake-S GT2 [UHD Graphics 630]")
        );
        assert_eq!(pci_name(ids, "1002", "ffff").as_deref(), Some("Advanced Micro Devices, Inc. [AMD/ATI]"));
        assert_eq!(pci_name(ids, "10de", "0001"), None);
    }

    #[test]
    fn static_names() {
        assert_eq!(static_name("Think Desk"), "think-desk");
        assert_eq!(static_name("  Máquina 2 "), "maquina-2");
        assert_eq!(static_name("!!!"), "");
    }
}
