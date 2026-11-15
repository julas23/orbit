//! The Storage page (docs/PLANO-SETTINGS.md §3.4): each disk in use
//! (local and network file systems, once per device) with its space, and,
//! when asked, what the home folder's folders take (measured in a thread:
//! a home over NFS takes a while). What to do when a pen drive, a camera or
//! a phone is plugged in is not here yet: nothing of Orbit's mounts or
//! opens them by itself.

use std::{collections::HashSet, os::unix::fs::MetadataExt, path::Path, rc::Rc};

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::{tr, tr_args};

/// The file systems shown: on a disk or over the network.
const KINDS: [&str; 14] = [
    "ext4", "ext3", "ext2", "btrfs", "xfs", "f2fs", "vfat", "exfat", "ntfs3", "ntfs", "fuseblk", "nfs", "nfs4", "cifs",
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct Disk {
    device: String,
    path: String,
    kind: String,
}

/// `/proc/self/mounts`, each device once (its first mount: btrfs mounts
/// one device at several places).
fn disks(mounts: &str) -> Vec<Disk> {
    let mut seen = HashSet::new();
    mounts
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let (device, path, kind) = (f.next()?, f.next()?, f.next()?);
            // Octal escapes: a space is \040.
            let path = path.replace("\\040", " ");
            (KINDS.contains(&kind) && !path.starts_with("/snap") && seen.insert(device.to_owned())).then(|| Disk {
                device: device.to_owned(),
                path,
                kind: kind.to_owned(),
            })
        })
        .collect()
}

fn size(bytes: u64) -> String {
    glib::format_size(bytes).to_string()
}

/// What `path` holds, its subfolders on the same file system, links not
/// followed.
fn usage(path: &Path, device: u64) -> u64 {
    let Ok(meta) = path.symlink_metadata() else { return 0 };
    if meta.dev() != device {
        return 0;
    }
    if !meta.is_dir() {
        return meta.blocks() * 512;
    }
    let inside: u64 = std::fs::read_dir(path)
        .map(|d| d.flatten().map(|e| usage(&e.path(), device)).sum())
        .unwrap_or(0);
    meta.blocks() * 512 + inside
}

/// The home folder's entries and their sizes, the largest first; the
/// hidden ones together.
fn home_usage() -> Vec<(String, u64)> {
    let home = glib::home_dir();
    let Ok(device) = home.metadata().map(|m| m.dev()) else { return Vec::new() };
    let mut hidden = 0;
    let mut out: Vec<(String, u64)> = Vec::new();
    for entry in std::fs::read_dir(&home).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let bytes = usage(&entry.path(), device);
        if name.starts_with('.') {
            hidden += bytes;
        } else {
            out.push((name, bytes));
        }
    }
    out.push((tr("settings-storage-hidden"), hidden));
    out.sort_by_key(|a| std::cmp::Reverse(a.1));
    out
}

fn bar(used: f64) -> gtk::LevelBar {
    let bar = gtk::LevelBar::builder().min_value(0.0).max_value(1.0).value(used.clamp(0.0, 1.0)).hexpand(true).build();
    bar.add_offset_value("low", 0.75);
    bar.add_offset_value("high", 0.9);
    bar.add_offset_value("full", 1.0);
    bar
}

pub fn page() -> gtk::Widget {
    let message = crate::message();
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();

    let card = crate::power_card(&tr("settings-storage-disks"));
    let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    for disk in disks(&mounts) {
        let info = gio::File::for_path(&disk.path)
            .query_filesystem_info("filesystem::size,filesystem::free", gio::Cancellable::NONE)
            .ok();
        let (total, free) = info.map_or((0, 0), |i| {
            (i.attribute_uint64("filesystem::size"), i.attribute_uint64("filesystem::free"))
        });
        let used = total.saturating_sub(free);
        let line = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).css_classes(["row"]).build();
        let top = gtk::Box::builder().spacing(10).build();
        top.append(&gtk::Label::builder().label(disk.path.as_str()).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build());
        top.append(
            &gtk::Label::builder()
                .label(tr_args(
                    "settings-storage-used",
                    &[("used", size(used).into()), ("total", size(total).into())],
                ))
                .xalign(1.0)
                .build(),
        );
        let open = gtk::Button::builder().icon_name("folder-open-symbolic").tooltip_text(tr("settings-drives-open")).css_classes(["flat"]).build();
        let (path, message2) = (disk.path.clone(), message.clone());
        open.connect_clicked(move |_| {
            let uri = gio::File::for_path(&path).uri();
            if let Err(e) = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE) {
                crate::say(&message2, &e.to_string());
            }
        });
        top.append(&open);
        line.append(&top);
        line.append(&bar(if total == 0 { 0.0 } else { used as f64 / total as f64 }));
        line.append(
            &gtk::Label::builder()
                .label(format!("{} · {}", disk.device, disk.kind))
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .css_classes(["dim"])
                .build(),
        );
        card.append(&line);
    }
    body.append(&card);

    let home = crate::power_card(&tr("settings-storage-home"));
    home.append(&gtk::Label::builder().label(tr("settings-storage-home-tip")).xalign(0.0).wrap(true).css_classes(["dim"]).build());
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
    home.append(&list);
    let measure = gtk::Button::with_label(&tr("settings-storage-measure"));
    measure.set_halign(gtk::Align::End);
    let list = Rc::new(list);
    measure.connect_clicked(move |b| {
        b.set_sensitive(false);
        b.set_label(&tr("settings-storage-measuring"));
        let (b, list) = (b.clone(), list.clone());
        glib::spawn_future_local(async move {
            let all = gio::spawn_blocking(home_usage).await.unwrap_or_default();
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            let largest = all.first().map_or(1, |a| a.1.max(1));
            for (name, bytes) in all.into_iter().filter(|a| a.1 > 0).take(16) {
                let line = gtk::Box::builder().spacing(10).build();
                line.append(&gtk::Label::builder().label(name).xalign(0.0).width_chars(16).max_width_chars(16).ellipsize(gtk::pango::EllipsizeMode::End).build());
                line.append(&bar(bytes as f64 / largest as f64));
                line.append(&gtk::Label::builder().label(size(bytes)).xalign(1.0).width_chars(9).build());
                list.append(&line);
            }
            b.set_label(&tr("settings-storage-measure"));
            b.set_sensitive(true);
        });
    });
    home.append(&measure);
    body.append(&home);

    body.append(&message);
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
    fn disks_once_per_device() {
        let mounts = "/dev/nvme0n1p2 / btrfs rw,subvol=/@ 0 0\nproc /proc proc rw 0 0\n/dev/nvme0n1p2 /.snapshots btrfs rw 0 0\n10.0.0.250:/datastore/jas /home/juliano nfs4 rw 0 0\n/dev/sda1 /media/My\\040Disk vfat rw 0 0\ntmpfs /tmp tmpfs rw 0 0\n";
        let d = disks(mounts);
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].path, "/");
        assert_eq!(d[1].kind, "nfs4");
        assert_eq!(d[2].path, "/media/My Disk");
    }
}
