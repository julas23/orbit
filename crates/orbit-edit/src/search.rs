//! Ctrl+Shift+F: the text searched in every file of the folder — the same
//! files Ctrl+P sees (no hidden ones, no build or dependency folders, no
//! binaries) — on a thread, each match a line in the list; a click opens
//! the file there. Case and regular expressions can be switched on.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use gtk::{gdk, glib, prelude::*};
use orbit_app::i18n::{number, tr, tr_args};

/// Matches shown, at most.
const MOST: usize = 2000;
/// Files larger than this are left out.
const LARGEST: u64 = 4 << 20;

pub struct Found {
    pub path: PathBuf,
    /// From 0.
    pub line: i32,
    pub column: i32,
    pub text: String,
}

type Open = Rc<dyn Fn(&Path, i32, i32)>;

pub fn open(parent: &gtk::Window, root: &Path, initial: Option<String>, on_open: impl Fn(&Path, i32, i32) + 'static) {
    let window = gtk::Window::builder()
        .transient_for(parent)
        .title(tr("edit-search-files"))
        .default_width(760)
        .default_height(520)
        .css_classes(["orbit-edit", "search-files"])
        .build();
    let entry = gtk::SearchEntry::builder()
        .placeholder_text(tr("edit-search-files"))
        .hexpand(true)
        .build();
    let case = gtk::ToggleButton::builder()
        .label("Aa")
        .tooltip_text(tr("edit-find-case"))
        .css_classes(["option"])
        .build();
    let regex = gtk::ToggleButton::builder()
        .label(".*")
        .tooltip_text(tr("edit-find-regex"))
        .css_classes(["option"])
        .build();
    let row = gtk::Box::builder().spacing(5).build();
    row.append(&entry);
    row.append(&case);
    row.append(&regex);
    let status = gtk::Label::builder().xalign(0.0).css_classes(["status-text"]).build();
    let list = gtk::ListBox::builder().css_classes(["results"]).build();
    let scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(5)
        .css_classes(["quick-open-box"])
        .build();
    content.append(&row);
    content.append(&status);
    content.append(&scroll);
    window.set_child(Some(&content));

    let on_open: Open = Rc::new(on_open);
    let root = root.to_path_buf();
    let found: Rc<RefCell<Vec<Found>>> = Rc::default();
    let stop: Rc<RefCell<Option<Arc<AtomicBool>>>> = Rc::default();
    let wait: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();

    let run = {
        let (entry, case, regex, list, status, found, stop, root) = (
            entry.clone(),
            case.clone(),
            regex.clone(),
            list.clone(),
            status.clone(),
            found.clone(),
            stop.clone(),
            root.clone(),
        );
        move || {
            if let Some(s) = stop.borrow_mut().take() {
                s.store(true, Ordering::Relaxed);
            }
            while let Some(row) = list.first_child() {
                list.remove(&row);
            }
            found.borrow_mut().clear();
            let query = entry.text().to_string();
            if query.is_empty() {
                status.set_label("");
                return;
            }
            let pattern = if regex.is_active() {
                query.clone()
            } else {
                regex::escape(&query)
            };
            let Ok(re) = regex::RegexBuilder::new(&pattern)
                .case_insensitive(!case.is_active())
                .build()
            else {
                status.set_label(&tr("edit-search-bad-regex"));
                return;
            };
            status.set_label(&tr("edit-search-running"));
            let cancel = Arc::new(AtomicBool::new(false));
            *stop.borrow_mut() = Some(cancel.clone());
            let (tx, rx) = async_channel::unbounded::<Vec<Found>>();
            let walk_root = root.clone();
            std::thread::spawn(move || {
                let mut files = Vec::new();
                walk(&walk_root, &mut files, &cancel);
                let mut total = 0;
                for file in files {
                    if cancel.load(Ordering::Relaxed) || total >= MOST {
                        break;
                    }
                    let hits = search_file(&file, &re, MOST - total);
                    total += hits.len();
                    if !hits.is_empty() && tx.send_blocking(hits).is_err() {
                        break;
                    }
                }
            });
            let (list, status, found, root) = (list.clone(), status.clone(), found.clone(), root.clone());
            glib::spawn_future_local(async move {
                while let Ok(hits) = rx.recv().await {
                    for hit in hits {
                        list.append(&result_row(&root, &hit));
                        found.borrow_mut().push(hit);
                    }
                }
                let n = found.borrow().len();
                status.set_label(&if n == 0 {
                    tr("edit-find-none")
                } else {
                    tr_args("edit-find-count", &[("total", number(n as f64))])
                });
            });
        }
    };
    let run = Rc::new(run);
    let later = {
        let run = run.clone();
        let wait = wait.clone();
        move || {
            if let Some(id) = wait.borrow_mut().take() {
                id.remove();
            }
            let run = run.clone();
            let wait2 = wait.clone();
            *wait.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(300), move || {
                wait2.borrow_mut().take();
                run();
            }));
        }
    };
    let later = Rc::new(later);
    {
        let later = later.clone();
        entry.connect_search_changed(move |_| later());
    }
    for b in [&case, &regex] {
        let run = run.clone();
        b.connect_toggled(move |_| run());
    }
    {
        let found = found.clone();
        let on_open = on_open.clone();
        list.connect_row_activated(move |_, row| {
            if let Some(hit) = found.borrow().get(row.index() as usize) {
                on_open(&hit.path, hit.line, hit.column);
            }
        });
    }
    {
        let list = list.clone();
        entry.connect_activate(move |_| {
            if let Some(row) = list.row_at_index(0) {
                row.activate();
            }
        });
    }
    let keys = gtk::EventControllerKey::new();
    {
        let window = window.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                window.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    window.add_controller(keys);
    {
        let stop = stop.clone();
        window.connect_close_request(move |_| {
            if let Some(s) = stop.borrow_mut().take() {
                s.store(true, Ordering::Relaxed);
            }
            glib::Propagation::Proceed
        });
    }
    if let Some(text) = initial.filter(|t| !t.contains('\n')) {
        entry.set_text(&text);
    }
    window.present();
    entry.grab_focus();
}

fn result_row(root: &Path, hit: &Found) -> gtk::Widget {
    let place = format!(
        "{}:{}",
        hit.path.strip_prefix(root).unwrap_or(&hit.path).display(),
        hit.line + 1
    );
    let row = gtk::Box::builder().spacing(10).build();
    row.append(
        &gtk::Label::builder()
            .label(&place)
            .xalign(0.0)
            .width_chars(30)
            .max_width_chars(40)
            .ellipsize(gtk::pango::EllipsizeMode::Start)
            .css_classes(["search-place"])
            .build(),
    );
    row.append(
        &gtk::Label::builder()
            .label(hit.text.trim())
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["search-text"])
            .build(),
    );
    row.upcast()
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>, cancel: &AtomicBool) {
    if cancel.load(Ordering::Relaxed) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || crate::sidebar::SKIPPED.contains(&name.as_str()) {
            continue;
        }
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(&entry.path(), files, cancel),
            Ok(t) if t.is_file() => files.push(entry.path()),
            _ => {}
        }
    }
}

fn search_file(path: &Path, re: &regex::Regex, most: usize) -> Vec<Found> {
    if std::fs::metadata(path).map_or(true, |m| m.len() > LARGEST) {
        return Vec::new();
    }
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    if bytes.iter().take(8000).any(|&b| b == 0) {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if let Some(m) = re.find(line) {
            out.push(Found {
                path: path.to_path_buf(),
                line: n as i32,
                column: line[..m.start()].chars().count() as i32,
                text: line.chars().take(300).collect(),
            });
            if out.len() >= most {
                break;
            }
        }
    }
    out
}
