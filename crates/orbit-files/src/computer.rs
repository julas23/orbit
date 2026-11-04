//! "Computer": the disks and volumes, each a card with its icon, its
//! name and how full it is — the root, home when it is a disk of its own
//! (an NFS home is), the disks, partitions and pendrives, and the shares
//! GVfs mounted. Disks not mounted show too; a click mounts them. The
//! space is asked of each disk off the main thread, so a slow server
//! never holds the window.

use std::{cell::RefCell, path::Path, rc::Rc};

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::{text, tr, tr_args};

use crate::{entry::Location, places};

type Go = Rc<dyn Fn(Location)>;

pub struct Computer {
    pub widget: gtk::ScrolledWindow,
    flow: gtk::FlowBox,
    monitor: gio::VolumeMonitor,
    go: RefCell<Option<Go>>,
}

impl Computer {
    pub fn new() -> Rc<Self> {
        let flow = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .homogeneous(true)
            .min_children_per_line(1)
            .max_children_per_line(6)
            .column_spacing(10)
            .row_spacing(10)
            .valign(gtk::Align::Start)
            .css_classes(["computer"])
            .build();
        let widget = gtk::ScrolledWindow::builder()
            .child(&flow)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let computer = Rc::new(Self {
            widget,
            flow,
            monitor: gio::VolumeMonitor::get(),
            go: RefCell::default(),
        });
        for signal in [
            "mount-added",
            "mount-removed",
            "mount-changed",
            "volume-added",
            "volume-removed",
        ] {
            let weak = Rc::downgrade(&computer);
            computer.monitor.connect_local(signal, false, move |_| {
                if let Some(c) = weak.upgrade().filter(|c| c.widget.is_mapped()) {
                    c.fill();
                }
                None
            });
        }
        computer
    }

    /// Called with where a card leads.
    pub fn connect_go(&self, f: impl Fn(Location) + 'static) {
        *self.go.borrow_mut() = Some(Rc::new(f));
    }

    fn go(&self) -> Go {
        self.go.borrow().clone().unwrap_or_else(|| Rc::new(|_| {}))
    }

    pub fn fill(&self) {
        while let Some(child) = self.flow.first_child() {
            self.flow.remove(&child);
        }
        let root = Path::new("/");
        self.card(
            &gio::ThemedIcon::new("drive-harddisk").upcast(),
            &tr("files-filesystem"),
            Some(gio::File::for_path(root)),
        );
        let home = glib::home_dir();
        if !same_device(&home, root) {
            self.card(
                &gio::ThemedIcon::new("user-home").upcast(),
                &tr("files-home"),
                Some(gio::File::for_path(&home)),
            );
        }
        for mount in self
            .monitor
            .mounts()
            .into_iter()
            .filter(|m| !m.is_shadowed() && places::is_volume(m))
        {
            let root_file = mount.root();
            if root_file.path().is_some_and(|p| p == root || p == home) {
                continue;
            }
            self.card(&mount.icon(), &mount.name(), Some(root_file));
        }
        // Not mounted yet: a click mounts it, then goes there.
        for volume in self
            .monitor
            .volumes()
            .into_iter()
            .filter(|v| v.get_mount().is_none() && v.can_mount() && !places::is_network(v))
        {
            let button = self.card(&volume.icon(), &volume.name(), None);
            let go = self.go();
            button.connect_clicked(move |b| {
                let go = go.clone();
                let v = volume.clone();
                let op = gtk::MountOperation::new(b.root().and_downcast_ref::<gtk::Window>());
                volume.mount(
                    gio::MountMountFlags::NONE,
                    Some(&op),
                    gio::Cancellable::NONE,
                    move |r| match r {
                        Ok(()) => {
                            if let Some(m) = v.get_mount() {
                                go(Location::of(&m.root()));
                            }
                        }
                        Err(e) => log::warn!("mount: {e}"),
                    },
                );
            });
        }
    }

    /// A card; with `root`, a mounted one: its space, and a click goes
    /// there.
    fn card(&self, icon: &gio::Icon, name: &str, root: Option<gio::File>) -> gtk::Button {
        let texts = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(5)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        texts.append(
            &gtk::Label::builder()
                .label(name)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["computer-name"])
                .build(),
        );
        let bar = gtk::LevelBar::builder().min_value(0.0).max_value(1.0).build();
        // Only "full" past nine tenths; the theme's other marks say little.
        for offset in [
            gtk::LEVEL_BAR_OFFSET_LOW,
            gtk::LEVEL_BAR_OFFSET_HIGH,
            gtk::LEVEL_BAR_OFFSET_FULL,
        ] {
            bar.remove_offset_value(Some(offset));
        }
        bar.add_offset_value("ok", 0.9);
        bar.add_offset_value("almost-full", 1.0);
        let info = gtk::Label::builder().xalign(0.0).css_classes(["dim"]).build();
        texts.append(&bar);
        texts.append(&info);
        let content = gtk::Box::builder().spacing(10).build();
        content.append(&gtk::Image::builder().gicon(icon).pixel_size(48).build());
        content.append(&texts);
        let button = gtk::Button::builder()
            .child(&content)
            .css_classes(["computer-card"])
            .build();
        self.flow.append(&button);
        match root {
            Some(root) => {
                let go = self.go();
                let to = Location::of(&root);
                button.connect_clicked(move |_| go(to.clone()));
                root.query_filesystem_info_async(
                    "filesystem::size,filesystem::free",
                    glib::Priority::DEFAULT,
                    gio::Cancellable::NONE,
                    move |r| {
                        let Ok(fs) = r else {
                            bar.set_visible(false);
                            return;
                        };
                        let size = fs.attribute_uint64("filesystem::size");
                        let free = fs.attribute_uint64("filesystem::free");
                        if size == 0 {
                            bar.set_visible(false);
                            return;
                        }
                        bar.set_value(1.0 - free as f64 / size as f64);
                        info.set_label(&tr_args(
                            "files-space-free",
                            &[
                                ("free", text(crate::entry::size_text(free))),
                                ("size", text(crate::entry::size_text(size))),
                            ],
                        ));
                    },
                );
            }
            None => {
                bar.set_visible(false);
                info.set_label(&tr("files-not-mounted"));
            }
        }
        button
    }
}

fn same_device(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev(),
        _ => true,
    }
}
