//! The places along the left: home and the user's folders, the GTK
//! bookmarks (shared with every GTK file dialog), the trash, the disks —
//! mounted or not, with eject for those that can be — and "Connect to
//! server" for SMB, SFTP, FTP or WebDAV shares, which GVfs mounts (asking
//! for a password when one is needed).

use std::{path::PathBuf, rc::Rc};

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::tr;

use crate::entry::{self, Location};

type Go = Rc<dyn Fn(Location)>;

pub struct Places {
    pub widget: gtk::ScrolledWindow,
    list: gtk::ListBox,
    monitor: gio::VolumeMonitor,
    go: Go,
    /// Every mount under "Devices", not just volumes.
    all_mounts: bool,
}

impl Places {
    pub fn new(all_mounts: bool, go: impl Fn(Location) + 'static) -> Rc<Self> {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["places"])
            .build();
        let widget = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(190)
            .build();
        let places = Rc::new(Self {
            widget,
            list,
            monitor: gio::VolumeMonitor::get(),
            go: Rc::new(go),
            all_mounts,
        });
        places.fill();
        // Disks come and go.
        for signal in ["mount-added", "mount-removed", "mount-changed", "volume-added", "volume-removed"] {
            let weak = Rc::downgrade(&places);
            places.monitor.connect_local(signal, false, move |_| {
                if let Some(p) = weak.upgrade() {
                    p.fill();
                }
                None
            });
        }
        places
    }

    fn fill(self: &Rc<Self>) {
        while let Some(row) = self.list.first_child() {
            self.list.remove(&row);
        }
        self.header(&tr("files-places"));
        let home = glib::home_dir();
        self.place("user-home", &tr("files-home"), Location::Path(home.clone()));
        use glib::UserDirectory as U;
        for dir in [U::Desktop, U::Documents, U::Downloads, U::Music, U::Pictures, U::Videos] {
            if let Some(path) = glib::user_special_dir(dir).filter(|p| p.is_dir() && *p != home) {
                let icon = entry::special_folder_icon(&path).unwrap_or("folder");
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.place(icon, &name, Location::Path(path));
            }
        }
        self.place("user-trash", &tr("files-trash"), Location::Uri("trash:///".into()));
        self.place("drive-harddisk", &tr("files-computer"), Location::Path(PathBuf::from("/")));

        let bookmarks = bookmarks();
        if !bookmarks.is_empty() {
            self.header(&tr("files-bookmarks"));
            for (file, name) in bookmarks {
                let label = name.unwrap_or_else(|| file.basename().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                self.place("folder", &label, Location::of(&file));
            }
        }

        let mounts: Vec<gio::Mount> = self
            .monitor
            .mounts()
            .into_iter()
            .filter(|m| !m.is_shadowed() && (self.all_mounts || is_volume(m)))
            .collect();
        let unmounted: Vec<gio::Volume> = self
            .monitor
            .volumes()
            .into_iter()
            .filter(|v| v.get_mount().is_none() && v.can_mount())
            .collect();
        // No heading over nothing.
        if !mounts.is_empty() || !unmounted.is_empty() {
            self.header(&tr("files-devices"));
        }
        for mount in mounts {
            let root = mount.root();
            let icon = mount.symbolic_icon();
            let row = self.row_with_icon(&icon, &mount.name());
            if mount.can_eject() || mount.can_unmount() {
                let eject = gtk::Button::builder()
                    .icon_name("media-eject-symbolic")
                    .css_classes(["flat", "eject"])
                    .tooltip_text(tr("files-eject"))
                    .build();
                let m = mount.clone();
                eject.connect_clicked(move |_| {
                    let op = gtk::MountOperation::new(None::<&gtk::Window>);
                    if m.can_eject() {
                        m.eject_with_operation(gio::MountUnmountFlags::NONE, Some(&op), gio::Cancellable::NONE, |r| {
                            if let Err(e) = r {
                                log::warn!("eject: {e}");
                            }
                        });
                    } else {
                        m.unmount_with_operation(gio::MountUnmountFlags::NONE, Some(&op), gio::Cancellable::NONE, |r| {
                            if let Err(e) = r {
                                log::warn!("unmount: {e}");
                            }
                        });
                    }
                });
                if let Some(content) = row.child().and_downcast::<gtk::Box>() {
                    content.append(&eject);
                }
            }
            let go = self.go.clone();
            self.on_click(&row, move || go(Location::of(&root)));
        }
        // Disks not mounted yet: a click mounts them.
        for volume in unmounted {
            let row = self.row_with_icon(&volume.symbolic_icon(), &volume.name());
            let go = self.go.clone();
            self.on_click(&row, move || {
                let go = go.clone();
                let v = volume.clone();
                let op = gtk::MountOperation::new(None::<&gtk::Window>);
                volume.mount(gio::MountMountFlags::NONE, Some(&op), gio::Cancellable::NONE, move |r| match r {
                    Ok(()) => {
                        if let Some(m) = v.get_mount() {
                            go(Location::of(&m.root()));
                        }
                    }
                    Err(e) => log::warn!("mount: {e}"),
                });
            });
        }

        self.header(&tr("files-network"));
        let row = self.row_with_icon(&gio::ThemedIcon::new("network-server-symbolic").upcast(), &tr("files-connect"));
        let weak = Rc::downgrade(self);
        self.on_click(&row, move || {
            if let Some(p) = weak.upgrade() {
                p.connect_dialog();
            }
        });
    }

    fn header(&self, title: &str) {
        let label = gtk::Label::builder().label(title).xalign(0.0).css_classes(["places-header"]).build();
        let row = gtk::ListBoxRow::builder().child(&label).activatable(false).selectable(false).build();
        self.list.append(&row);
    }

    fn place(&self, icon: &str, name: &str, to: Location) {
        let row = self.row_with_icon(&gio::ThemedIcon::new(&format!("{icon}-symbolic")).upcast(), name);
        let go = self.go.clone();
        self.on_click(&row, move || go(to.clone()));
    }

    fn row_with_icon(&self, icon: &gio::Icon, name: &str) -> gtk::ListBoxRow {
        let content = gtk::Box::builder().spacing(10).build();
        content.append(&gtk::Image::builder().gicon(icon).pixel_size(16).build());
        content.append(&gtk::Label::builder().label(name).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build());
        let row = gtk::ListBoxRow::builder().child(&content).css_classes(["place"]).build();
        self.list.append(&row);
        row
    }

    fn on_click(&self, row: &gtk::ListBoxRow, f: impl Fn() + 'static) {
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| f());
        row.add_controller(click);
    }

    /// "smb://server/share", "sftp://user@host/", …: mounted by GVfs,
    /// then shown.
    fn connect_dialog(&self) {
        let parent = self.widget.root().and_downcast::<gtk::Window>();
        let entry = gtk::Entry::builder()
            .placeholder_text("smb://servidor/pasta  ·  sftp://usuario@host/")
            .width_chars(40)
            .build();
        let window = gtk::Window::builder()
            .modal(true)
            .css_classes(["orbit-files-dialog"])
            .title(tr("files-connect"))
            .resizable(false)
            .child(&crate::actions::padded("dialog-box", &entry))
            .build();
        window.set_transient_for(parent.as_ref());
        let go = self.go.clone();
        let w = window.clone();
        entry.connect_activate(move |e| {
            let uri = e.text().trim().to_owned();
            if uri.is_empty() {
                return;
            }
            let file = gio::File::for_uri(&uri);
            let op = gtk::MountOperation::new(w.transient_for().as_ref());
            let go = go.clone();
            let f = file.clone();
            file.mount_enclosing_volume(gio::MountMountFlags::NONE, Some(&op), gio::Cancellable::NONE, move |r| match r {
                Ok(()) => go(Location::of(&f)),
                // Mounted already: just go.
                Err(e) if e.matches(gio::IOErrorEnum::AlreadyMounted) => go(Location::of(&f)),
                Err(e) => log::warn!("connect {uri}: {e}"),
            });
            w.close();
        });
        window.present();
    }
}

/// A disk, partition, pendrive or network share — not a folder mounted
/// somewhere in the tree (NFS into home, bind mounts), which has neither a
/// volume nor a drive behind it and a plain local path.
fn is_volume(mount: &gio::Mount) -> bool {
    mount.volume().is_some() || mount.drive().is_some() || !mount.root().has_uri_scheme("file")
}

/// The GTK bookmarks: (location, name).
fn bookmarks() -> Vec<(gio::File, Option<String>)> {
    let file = glib::user_config_dir().join("gtk-3.0").join("bookmarks");
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut parts = l.splitn(2, ' ');
            let uri = parts.next().unwrap_or_default();
            (gio::File::for_uri(uri), parts.next().map(str::to_owned))
        })
        .collect()
}
