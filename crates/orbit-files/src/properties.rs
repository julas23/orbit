//! Properties of what is selected (Alt+Enter) — or of the folder, with
//! nothing selected: name, kind, where it is, size (a folder's added up on
//! a thread, shown as it goes), the dates, the owner and the permissions,
//! which the owner can change here.
//!
//! Everything about the file is read off the main thread (GIO's
//! `query_info` future): over NFS, a look at a file being written waits
//! for it, and the window must not.

use std::{
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use gtk::{gdk, gio, glib, prelude::*};
use orbit_app::i18n::{number, text, tr, tr_args};

use crate::entry::{self, Entry};

const ATTRIBUTES: &str = "standard::*,time::*,owner::*,unix::*";

pub fn show(parent: &gtk::Window, entries: Vec<Entry>) {
    if entries.is_empty() {
        return;
    }
    let grid = gtk::Grid::builder().column_spacing(15).row_spacing(6).css_classes(["properties"]).build();
    let rows = Rc::new(Rows { grid: grid.clone(), next: std::cell::Cell::new(0) });

    let head = gtk::Box::builder().spacing(12).css_classes(["properties-head"]).build();
    let name = gtk::Label::builder()
        .xalign(0.0)
        .selectable(true)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .max_width_chars(40)
        .css_classes(["properties-name"])
        .build();
    let single = entries.len() == 1;
    if single {
        let e = &entries[0];
        head.append(&gtk::Image::builder().gicon(&entry::icon(e)).pixel_size(48).build());
        name.set_label(&e.name);
    } else {
        head.append(&gtk::Image::builder().icon_name("edit-select-all-symbolic").pixel_size(48).build());
        name.set_label(&tr_args("files-prop-items", &[("n", number(entries.len() as f64))]));
    }
    head.append(&name);

    // Where: the folder they are in, when they share one.
    let folders: Vec<_> = entries.iter().filter_map(|e| e.location.parent()).collect();
    if let Some(first) = folders.first()
        && folders.iter().all(|f| f == first)
    {
        rows.add(&tr("files-prop-where"), &value(&first.display()));
    }

    // Size: a file's at once; folders added up on a thread.
    let size = value("");
    rows.add(&tr("files-prop-size"), &size);
    let paths: Vec<PathBuf> = entries.iter().filter_map(|e| e.location.path().map(PathBuf::from)).collect();
    let stop = Arc::new(AtomicBool::new(false));
    if single && !entries[0].dir {
        size.set_label(&bytes_text(entries[0].size));
    } else if !paths.is_empty() {
        count(paths, size, stop.clone());
    }

    let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(15).build();
    body.append(&head);
    body.append(&grid);
    let window = gtk::Window::builder()
        .transient_for(parent)
        .css_classes(["orbit-files-dialog"])
        .title(tr("files-properties"))
        .resizable(false)
        .child(&crate::actions::padded("dialog-box", &body))
        .build();
    window.connect_close_request(move |_| {
        stop.store(true, Ordering::Relaxed);
        glib::Propagation::Proceed
    });
    let keys = gtk::EventControllerKey::new();
    let w = window.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape
            && let Some(w) = w.upgrade()
        {
            w.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    window.present();

    // The rest of one file: from GIO, off the main thread.
    if single {
        let e = entries[0].clone();
        let rows = rows.clone();
        glib::spawn_future_local(async move {
            let file = e.location.file();
            let Ok(info) = file
                .query_info_future(ATTRIBUTES, gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, glib::Priority::DEFAULT)
                .await
            else {
                return;
            };
            details(&rows, &e, &info);
        });
    } else {
        let kinds: std::collections::BTreeSet<String> =
            entries.iter().map(|e| gio::content_type_get_description(&e.mime).to_string()).collect();
        rows.add(&tr("files-prop-kind"), &value(&kinds.into_iter().collect::<Vec<_>>().join(", ")));
    }
}

/// The grid of a name and its value, a row at a time.
struct Rows {
    grid: gtk::Grid,
    next: std::cell::Cell<i32>,
}

impl Rows {
    fn add(&self, key: &str, value: &impl IsA<gtk::Widget>) {
        let row = self.next.get();
        self.next.set(row + 1);
        let key = gtk::Label::builder().label(key).xalign(1.0).yalign(0.0).css_classes(["dim"]).build();
        self.grid.attach(&key, 0, row, 1, 1);
        self.grid.attach(value, 1, row, 1, 1);
    }
}

fn value(s: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(s)
        .xalign(0.0)
        .selectable(true)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .max_width_chars(45)
        .build()
}

fn bytes_text(bytes: u64) -> String {
    tr_args(
        "files-prop-bytes",
        &[("size", text(entry::size_text(bytes))), ("bytes", number(bytes as f64))],
    )
}

fn details(rows: &Rows, e: &Entry, info: &gio::FileInfo) {
    let mime = if e.dir { "inode/directory".to_owned() } else { e.mime.clone() };
    rows.add(
        &tr("files-prop-kind"),
        &value(&format!("{} ({mime})", gio::content_type_get_description(&mime))),
    );
    if let Some(target) = info.symlink_target() {
        rows.add(&tr("files-prop-link"), &value(&target.display().to_string()));
    }
    for (key, attribute) in [
        ("files-prop-modified", "time::modified"),
        ("files-prop-accessed", "time::access"),
        ("files-prop-created", "time::created"),
    ] {
        let secs = info.attribute_uint64(attribute);
        if secs > 0 {
            rows.add(&tr(key), &value(&entry::date_text(secs as i64)));
        }
    }
    if info.has_attribute("owner::user") {
        let user = info.attribute_string("owner::user").map(|s| s.to_string()).unwrap_or_default();
        let group = info.attribute_string("owner::group").map(|s| s.to_string()).unwrap_or_default();
        rows.add(&tr("files-prop-owner"), &value(&format!("{user} : {group}")));
    }
    if info.has_attribute("unix::mode")
        && let Some(path) = e.location.path()
    {
        let mode = info.attribute_uint32("unix::mode") & 0o7777;
        let mine = info.attribute_uint32("unix::uid") == own_uid();
        rows.add(&tr("files-prop-permissions"), &permissions(path, mode, mine && !e.link));
    }
}

/// The nine boxes — read, write, run, for the owner, the group and the
/// others — and the mode in octal. Ticking one changes the file (the
/// owner's alone can).
fn permissions(path: &Path, mode: u32, editable: bool) -> gtk::Box {
    let grid = gtk::Grid::builder().column_spacing(10).row_spacing(2).build();
    for (col, key) in ["files-prop-read", "files-prop-write", "files-prop-execute"].into_iter().enumerate() {
        grid.attach(&gtk::Label::builder().label(tr(key)).css_classes(["dim"]).build(), col as i32 + 1, 0, 1, 1);
    }
    let octal = gtk::Label::builder().label(format!("{mode:04o}")).xalign(0.0).css_classes(["dim"]).build();
    let current = Rc::new(std::cell::Cell::new(mode));
    for (row, key) in ["files-prop-user", "files-prop-group", "files-prop-others"].into_iter().enumerate() {
        grid.attach(&gtk::Label::builder().label(tr(key)).xalign(0.0).build(), 0, row as i32 + 1, 1, 1);
        for col in 0..3 {
            let bit = 1u32 << ((2 - row) * 3 + (2 - col));
            let check = gtk::CheckButton::builder()
                .active(mode & bit != 0)
                .sensitive(editable)
                .halign(gtk::Align::Center)
                .build();
            let (path, current, octal) = (path.to_path_buf(), current.clone(), octal.clone());
            check.connect_toggled(move |c| {
                let mode = if c.is_active() { current.get() | bit } else { current.get() & !bit };
                current.set(mode);
                octal.set_label(&format!("{mode:04o}"));
                let path = path.clone();
                std::thread::spawn(move || {
                    use std::os::unix::fs::PermissionsExt;
                    if let Err(err) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)) {
                        log::warn!("chmod {}: {err}", path.display());
                    }
                });
            });
            grid.attach(&check, col as i32 + 1, row as i32 + 1, 1, 1);
        }
    }
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).build();
    b.append(&grid);
    b.append(&octal);
    b
}

fn own_uid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(u32::MAX)
}

/// Adds up `paths` on a thread — items and bytes, links not followed —
/// shown as it goes, until done or `stop`.
fn count(paths: Vec<PathBuf>, label: gtk::Label, stop: Arc<AtomicBool>) {
    label.set_label(&tr("files-prop-counting-start"));
    let (tx, rx) = async_channel::unbounded::<(u64, u64, bool)>();
    std::thread::Builder::new()
        .name("orbit-files-count".into())
        .spawn(move || {
            let mut state = (0u64, 0u64, Instant::now());
            // A folder asked about is not one of its items: what is in it is.
            for p in &paths {
                match std::fs::read_dir(p).ok().filter(|_| !p.is_symlink()) {
                    Some(dir) => dir.filter_map(Result::ok).for_each(|e| walk(&e.path(), &mut state, &tx, &stop)),
                    None => walk(p, &mut state, &tx, &stop),
                }
            }
            let _ = tx.send_blocking((state.0, state.1, true));
        })
        .ok();
    glib::spawn_future_local(async move {
        while let Ok((items, bytes, done)) = rx.recv().await {
            let args = [("n", number(items as f64)), ("size", text(entry::size_text(bytes)))];
            label.set_label(&tr_args(if done { "files-prop-contents" } else { "files-prop-counting" }, &args));
        }
    });
}

fn walk(path: &Path, state: &mut (u64, u64, Instant), tx: &async_channel::Sender<(u64, u64, bool)>, stop: &AtomicBool) {
    if stop.load(Ordering::Relaxed) {
        return;
    }
    let Ok(meta) = path.symlink_metadata() else { return };
    state.0 += 1;
    if meta.is_dir() {
        if let Ok(dir) = std::fs::read_dir(path) {
            for e in dir.filter_map(Result::ok) {
                walk(&e.path(), state, tx, stop);
            }
        }
    } else {
        state.1 += meta.len();
    }
    if state.2.elapsed() > Duration::from_millis(200) {
        state.2 = Instant::now();
        let _ = tx.send_blocking((state.0, state.1, false));
    }
}
