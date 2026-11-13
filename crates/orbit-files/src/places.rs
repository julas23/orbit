//! The places along the left: the computer, home and the user's folders,
//! the trash, the GTK bookmarks (shared with every GTK file dialog: added
//! with the + on their heading, Ctrl+D or a folder dropped on it,
//! reordered by dragging, and moved or removed from their right-click
//! menu), the disks —
//! mounted or not, with eject for those that can be — and "Connect to
//! server" for SMB, SFTP, FTP or WebDAV shares, which GVfs mounts (asking
//! for a password when one is needed).

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

use gtk::{gdk, gio, glib, prelude::*};
use orbit_app::i18n::tr;

use crate::entry::{self, Location};

type Go = Rc<dyn Fn(Location)>;
/// Files dropped on a place: what, where (None: the trash), and whether
/// copied (or else moved).
type Dropped = Rc<RefCell<Option<Box<dyn Fn((Vec<PathBuf>, Option<PathBuf>, bool))>>>>;

pub struct Places {
    pub widget: gtk::ScrolledWindow,
    list: gtk::ListBox,
    monitor: gio::VolumeMonitor,
    go: Go,
    /// Every mount under "Devices", not just volumes; the eye on the
    /// heading turns it over.
    all_mounts: Cell<bool>,
    on_dropped: Dropped,
    /// Where the window is: what the + on "Bookmarks" adds.
    here: RefCell<Option<Box<dyn Fn() -> Location>>>,
    /// The bookmarks file, changed by another window or a GTK dialog.
    bookmarks_monitor: RefCell<Option<gio::FileMonitor>>,
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
            all_mounts: Cell::new(all_mounts),
            on_dropped: Rc::default(),
            here: RefCell::default(),
            bookmarks_monitor: RefCell::default(),
        });
        places.fill();
        if let Ok(monitor) =
            gio::File::for_path(bookmarks_file()).monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        {
            let weak = Rc::downgrade(&places);
            monitor.connect_changed(move |_, _, _, event| {
                if matches!(
                    event,
                    gio::FileMonitorEvent::ChangesDoneHint
                        | gio::FileMonitorEvent::Created
                        | gio::FileMonitorEvent::Deleted
                        | gio::FileMonitorEvent::MovedIn
                        | gio::FileMonitorEvent::Renamed
                ) && let Some(p) = weak.upgrade()
                {
                    p.refill();
                }
            });
            *places.bookmarks_monitor.borrow_mut() = Some(monitor);
        }
        // Disks come and go.
        for signal in [
            "mount-added",
            "mount-removed",
            "mount-changed",
            "volume-added",
            "volume-removed",
        ] {
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

    /// Where the window is, for the + on "Bookmarks".
    pub fn connect_here(&self, f: impl Fn() -> Location + 'static) {
        *self.here.borrow_mut() = Some(Box::new(f));
    }

    /// Adds folders to the bookmarks, after the ones there (those already
    /// there stay where they are).
    pub fn add_bookmarks(self: &Rc<Self>, files: &[gio::File]) {
        let mut lines = read_bookmarks();
        let mut changed = false;
        for file in files {
            if !bookmarkable(file) {
                continue;
            }
            let uri = file.uri().to_string();
            if !lines.iter().any(|l| split(l).0 == uri) {
                lines.push(uri);
                changed = true;
            }
        }
        if changed {
            self.save_bookmarks(&lines);
        }
    }

    fn save_bookmarks(self: &Rc<Self>, lines: &[String]) {
        if let Err(e) = write_bookmarks(lines) {
            log::warn!("bookmarks: {e}");
        }
        self.refill();
    }

    /// Fills the list again — later, not from inside a row being removed.
    fn refill(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(p) = weak.upgrade() {
                p.fill();
            }
        });
    }

    /// Called with files dropped on a place.
    pub fn connect_dropped(&self, f: impl Fn((Vec<PathBuf>, Option<PathBuf>, bool)) + 'static) {
        *self.on_dropped.borrow_mut() = Some(Box::new(f));
    }

    /// Files dropped on `row` go into `to` (a local folder) — or, for the
    /// trash, to the trash.
    fn take_drops(&self, row: &gtk::ListBoxRow, to: &Location) {
        let trash = matches!(to, Location::Uri(u) if u == "trash:///");
        let Some(dir) = to.path().map(PathBuf::from).or(trash.then(PathBuf::new)) else {
            return;
        };
        let target = gtk::DropTarget::new(
            gdk::FileList::static_type(),
            gdk::DragAction::COPY | gdk::DragAction::MOVE,
        );
        target.set_preload(true);
        let d = dir.clone();
        target.connect_motion(move |t, _, _| {
            let paths = t
                .value()
                .and_then(|v| v.get::<gdk::FileList>().ok())
                .map(|l| crate::pane::paths_of(&l));
            match paths {
                _ if trash => gdk::DragAction::MOVE,
                None => gdk::DragAction::COPY,
                Some(p) => match crate::pane::drop_kind(&p, &d, t.current_event_state()) {
                    Some(true) => gdk::DragAction::COPY,
                    Some(false) => gdk::DragAction::MOVE,
                    None => gdk::DragAction::empty(),
                },
            }
        });
        let dropped = self.on_dropped.clone();
        target.connect_drop(move |t, value, _, _| {
            let Ok(list) = value.get::<gdk::FileList>() else {
                return false;
            };
            let paths = crate::pane::paths_of(&list);
            let (to, copy) = if trash {
                if paths.is_empty() {
                    return false;
                }
                (None, false)
            } else {
                let Some(copy) = crate::pane::drop_kind(&paths, &dir, t.current_event_state()) else {
                    return false;
                };
                (Some(dir.clone()), copy)
            };
            let paths: Vec<PathBuf> = paths
                .into_iter()
                .filter(|s| to.as_deref().is_none_or(|to| s.parent() != Some(to)))
                .collect();
            if let Some(f) = dropped.borrow().as_ref() {
                f((paths, to, copy));
            }
            true
        });
        row.add_controller(target);
    }

    fn fill(self: &Rc<Self>) {
        while let Some(row) = self.list.first_child() {
            self.list.remove(&row);
        }
        self.header(&tr("files-places"));
        let home = glib::home_dir();
        self.place("computer", &tr("files-computer"), Location::Computer);
        self.place("user-home", &tr("files-home"), Location::Path(home.clone()));
        self.separator();
        use glib::UserDirectory as U;
        for dir in [U::Desktop, U::Documents, U::Downloads, U::Music, U::Pictures, U::Videos] {
            if let Some(path) = glib::user_special_dir(dir).filter(|p| p.is_dir() && *p != home) {
                let icon = entry::special_folder_icon(&path).unwrap_or("folder");
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.place(icon, &name, Location::Path(path));
            }
        }
        self.separator();
        self.place("user-trash", &tr("files-trash"), Location::Uri("trash:///".into()));

        // Always there, for its +.
        self.bookmarks_header();
        let lines = read_bookmarks();
        for (n, line) in lines.iter().enumerate() {
            let (uri, name) = split(line);
            let file = gio::File::for_uri(uri);
            let label = name.map(str::to_owned).unwrap_or_else(|| {
                file.basename()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
            let icon = file
                .path()
                .and_then(|p| entry::special_folder_icon(&p))
                .unwrap_or("folder");
            let row = self.place(icon, &label, Location::of(&file));
            self.manage_bookmark(&row, n, lines.len());
        }

        let all = self.all_mounts.get();
        let mut hidden = 0;
        let mounts: Vec<gio::Mount> = self
            .monitor
            .mounts()
            .into_iter()
            .filter(|m| !m.is_shadowed())
            .filter(|m| {
                all || is_volume(m) || {
                    hidden += 1;
                    false
                }
            })
            .collect();
        let unmounted: Vec<gio::Volume> = self
            .monitor
            .volumes()
            .into_iter()
            .filter(|v| v.get_mount().is_none() && v.can_mount())
            .filter(|v| {
                all || !is_network(v) || {
                    hidden += 1;
                    false
                }
            })
            .collect();
        // No heading over nothing — but over what is hidden, its eye.
        if !mounts.is_empty() || !unmounted.is_empty() || hidden > 0 {
            self.devices_header(hidden);
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
                        m.unmount_with_operation(
                            gio::MountUnmountFlags::NONE,
                            Some(&op),
                            gio::Cancellable::NONE,
                            |r| {
                                if let Err(e) = r {
                                    log::warn!("unmount: {e}");
                                }
                            },
                        );
                    }
                });
                if let Some(content) = row.child().and_downcast::<gtk::Box>() {
                    content.append(&eject);
                }
            }
            self.take_drops(&row, &Location::of(&root));
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

        self.header(&tr("files-network"));
        let row = self.row_with_icon(
            &gio::ThemedIcon::new("network-server-symbolic").upcast(),
            &tr("files-connect"),
        );
        let weak = Rc::downgrade(self);
        self.on_click(&row, move || {
            if let Some(p) = weak.upgrade() {
                p.connect_dialog();
            }
        });
    }

    /// "Devices", and the eye that shows or hides the network folders
    /// mounted into the tree (NFS into home, and the like).
    fn devices_header(self: &Rc<Self>, hidden: usize) {
        let content = gtk::Box::builder().spacing(5).build();
        content.append(
            &gtk::Label::builder()
                .label(tr("files-devices"))
                .xalign(0.0)
                .hexpand(true)
                .css_classes(["places-header"])
                .build(),
        );
        let all = self.all_mounts.get();
        if all || hidden > 0 {
            let eye = gtk::Button::builder()
                .icon_name(if all {
                    "view-conceal-symbolic"
                } else {
                    "view-reveal-symbolic"
                })
                .tooltip_text(tr(if all { "files-mounts-hide" } else { "files-mounts-show" }))
                .css_classes(["flat", "eject"])
                .build();
            let weak = Rc::downgrade(self);
            eye.connect_clicked(move |_| {
                if let Some(p) = weak.upgrade() {
                    p.all_mounts.set(!p.all_mounts.get());
                    // Not from inside the button being removed.
                    glib::idle_add_local_once(move || p.fill());
                }
            });
            content.append(&eye);
        }
        let row = gtk::ListBoxRow::builder()
            .child(&content)
            .activatable(false)
            .selectable(false)
            .build();
        self.list.append(&row);
    }

    /// "Bookmarks", with a + that adds the folder the window is in; a
    /// folder dropped on it is added too.
    fn bookmarks_header(self: &Rc<Self>) {
        let content = gtk::Box::builder().spacing(5).build();
        content.append(
            &gtk::Label::builder()
                .label(tr("files-bookmarks"))
                .xalign(0.0)
                .hexpand(true)
                .css_classes(["places-header"])
                .build(),
        );
        let add = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text(tr("files-bookmark-add-here"))
            .css_classes(["flat", "eject"])
            .build();
        let weak = Rc::downgrade(self);
        add.connect_clicked(move |_| {
            let Some(p) = weak.upgrade() else { return };
            let here = p.here.borrow().as_ref().map(|f| f());
            if let Some(here) = here {
                p.add_bookmarks(&[here.file()]);
            }
        });
        content.append(&add);
        let row = gtk::ListBoxRow::builder()
            .child(&content)
            .activatable(false)
            .selectable(false)
            .css_classes(["bookmarks-header"])
            .build();
        let target = gtk::DropTarget::new(
            gdk::FileList::static_type(),
            gdk::DragAction::LINK | gdk::DragAction::COPY,
        );
        let weak = Rc::downgrade(self);
        target.connect_drop(move |_, value, _, _| {
            let (Some(p), Ok(list)) = (weak.upgrade(), value.get::<gdk::FileList>()) else {
                return false;
            };
            let dirs: Vec<gio::File> = list
                .files()
                .into_iter()
                .filter(|f| {
                    f.query_file_type(gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE) == gio::FileType::Directory
                })
                .collect();
            p.add_bookmarks(&dirs);
            !dirs.is_empty()
        });
        row.add_controller(target);
        self.list.append(&row);
    }

    /// Bookmark `n` of `count`: dragged onto another to take its place,
    /// and a right-click menu to move it up or down or remove it.
    fn manage_bookmark(self: &Rc<Self>, row: &gtk::ListBoxRow, n: usize, count: usize) {
        row.add_css_class("bookmark");
        // Dragging it: its number, which only this program takes.
        let source = gtk::DragSource::builder().actions(gdk::DragAction::MOVE).build();
        source.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&(n as u32).to_value())));
        let r = row.downgrade();
        source.connect_drag_begin(move |s, _| {
            if let Some(r) = r.upgrade() {
                s.set_icon(Some(&gtk::WidgetPaintable::new(Some(&r))), 0, 0);
            }
        });
        row.add_controller(source);
        let target = gtk::DropTarget::new(u32::static_type(), gdk::DragAction::MOVE);
        let weak = Rc::downgrade(self);
        target.connect_drop(move |_, value, _, _| {
            let (Some(p), Ok(from)) = (weak.upgrade(), value.get::<u32>()) else {
                return false;
            };
            p.save_bookmarks(&moved(read_bookmarks(), from as usize, n));
            true
        });
        row.add_controller(target);

        let menu = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let weak = Rc::downgrade(self);
        let r = row.downgrade();
        menu.connect_pressed(move |_, _, x, y| {
            let (Some(p), Some(row)) = (weak.upgrade(), r.upgrade()) else {
                return;
            };
            let popover = gtk::Popover::builder().css_classes(["context-menu"]).build();
            let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
            let mut items: Vec<(&str, Option<usize>)> = Vec::new();
            if n > 0 {
                items.push(("files-bookmark-up", Some(n - 1)));
            }
            if n + 1 < count {
                items.push(("files-bookmark-down", Some(n + 1)));
            }
            items.push(("files-bookmark-remove", None));
            for (label, to) in items {
                let button = gtk::Button::builder()
                    .child(&gtk::Label::builder().label(tr(label)).xalign(0.0).build())
                    .css_classes(["flat", "menu-item"])
                    .build();
                let weak = Rc::downgrade(&p);
                let pop = popover.downgrade();
                button.connect_clicked(move |_| {
                    if let Some(pop) = pop.upgrade() {
                        pop.popdown();
                    }
                    let Some(p) = weak.upgrade() else { return };
                    let mut lines = read_bookmarks();
                    match to {
                        Some(to) => lines = moved(lines, n, to),
                        None if n < lines.len() => {
                            lines.remove(n);
                        }
                        None => return,
                    }
                    p.save_bookmarks(&lines);
                });
                column.append(&button);
            }
            popover.set_child(Some(&column));
            orbit_app::popup::open(&popover, &row, Some((x, y)));
        });
        row.add_controller(menu);
    }

    /// A line between groups of places.
    fn separator(&self) {
        let row = gtk::ListBoxRow::builder()
            .child(&gtk::Separator::new(gtk::Orientation::Horizontal))
            .activatable(false)
            .selectable(false)
            .css_classes(["places-separator"])
            .build();
        self.list.append(&row);
    }

    fn header(&self, title: &str) {
        let label = gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .css_classes(["places-header"])
            .build();
        let row = gtk::ListBoxRow::builder()
            .child(&label)
            .activatable(false)
            .selectable(false)
            .build();
        self.list.append(&row);
    }

    fn place(&self, icon: &str, name: &str, to: Location) -> gtk::ListBoxRow {
        let row = self.row_with_icon(&gio::ThemedIcon::new(&format!("{icon}-symbolic")).upcast(), name);
        self.take_drops(&row, &to);
        let go = self.go.clone();
        self.on_click(&row, move || go(to.clone()));
        row
    }

    fn row_with_icon(&self, icon: &gio::Icon, name: &str) -> gtk::ListBoxRow {
        let content = gtk::Box::builder().spacing(10).build();
        content.append(&gtk::Image::builder().gicon(icon).pixel_size(16).build());
        content.append(
            &gtk::Label::builder()
                .label(name)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
        let row = gtk::ListBoxRow::builder()
            .child(&content)
            .css_classes(["place"])
            .build();
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
            file.mount_enclosing_volume(
                gio::MountMountFlags::NONE,
                Some(&op),
                gio::Cancellable::NONE,
                move |r| match r {
                    Ok(()) => go(Location::of(&f)),
                    // Mounted already: just go.
                    Err(e) if e.matches(gio::IOErrorEnum::AlreadyMounted) => go(Location::of(&f)),
                    Err(e) => log::warn!("connect {uri}: {e}"),
                },
            );
            w.close();
        });
        window.present();
    }
}

/// A disk, partition, pendrive, or a share connected to through GVfs
/// (smb://, sftp://…) — not a folder mounted somewhere in the tree: NFS or
/// CIFS from fstab (UDisks gives those a "network" volume), bind mounts
/// (no volume, no drive).
pub fn is_volume(mount: &gio::Mount) -> bool {
    if !mount.root().has_uri_scheme("file") || mount.drive().is_some() {
        return true;
    }
    mount.volume().is_some_and(|v| !is_network(&v))
}

/// A network filesystem from fstab, as UDisks marks it.
pub fn is_network(volume: &gio::Volume) -> bool {
    volume.identifier("class").as_deref() == Some("network")
}

fn bookmarks_file() -> PathBuf {
    glib::user_config_dir().join("gtk-3.0").join("bookmarks")
}

/// The GTK bookmarks, a line each: "uri" or "uri name".
fn read_bookmarks() -> Vec<String> {
    std::fs::read_to_string(bookmarks_file())
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

fn write_bookmarks(lines: &[String]) -> std::io::Result<()> {
    let file = bookmarks_file();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    // Whole or not at all: GTK dialogs read it too.
    let tmp = file.with_extension("orbit-files.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, file)
}

/// A bookmark line: its URI, and the name given to it, if any.
fn split(line: &str) -> (&str, Option<&str>) {
    match line.split_once(' ') {
        Some((uri, name)) if !name.trim().is_empty() => (uri, Some(name)),
        Some((uri, _)) => (uri, None),
        None => (line, None),
    }
}

/// A folder worth a bookmark: not "Computer", nor the trash.
fn bookmarkable(file: &gio::File) -> bool {
    !file.has_uri_scheme("computer") && !file.has_uri_scheme("trash")
}

/// `lines` with the one at `from` taken to `to`, the others making room.
fn moved(mut lines: Vec<String>, from: usize, to: usize) -> Vec<String> {
    if from < lines.len() && to < lines.len() && from != to {
        let line = lines.remove(from);
        lines.insert(to, line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn moving_a_bookmark_takes_the_place_of_another() {
        let l = v(&["a", "b", "c", "d"]);
        assert_eq!(moved(l.clone(), 0, 2), v(&["b", "c", "a", "d"]));
        assert_eq!(moved(l.clone(), 3, 1), v(&["a", "d", "b", "c"]));
        assert_eq!(moved(l.clone(), 1, 1), l);
        assert_eq!(moved(l.clone(), 9, 1), l);
    }

    #[test]
    fn a_bookmark_line_has_a_uri_and_maybe_a_name() {
        assert_eq!(split("file:///home/a"), ("file:///home/a", None));
        assert_eq!(split("file:///home/a Meu nome"), ("file:///home/a", Some("Meu nome")));
        assert_eq!(split("file:///home/a "), ("file:///home/a", None));
    }
}
