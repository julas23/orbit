//! The folder beside the tabs: its tree, folders first, hidden files and
//! the usual build and dependency folders left out; a click opens a file.
//! And Ctrl+P, to find a file of the folder by a few letters of its name.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{gdk, gio, glib, prelude::*};
use orbit_app::i18n::tr;

/// Folders nobody edits by hand.
const SKIPPED: &[&str] = &[".git", "node_modules", "target", "__pycache__", ".venv", "venv", "build", "dist"];
/// Files Ctrl+P looks through, at most.
const MOST_FILES: usize = 50_000;

type Opener = Box<dyn Fn(&Path)>;

pub struct Sidebar {
    pub widget: gtk::Box,
    title: gtk::Label,
    list: gtk::ListView,
    root: RefCell<Option<PathBuf>>,
    on_open: RefCell<Option<Opener>>,
}

impl Sidebar {
    pub fn new() -> Rc<Self> {
        let title = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .css_classes(["folder-title"])
            .build();
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
            let row = gtk::Box::builder().spacing(5).build();
            row.append(&gtk::Image::new());
            row.append(&gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build());
            item.set_child(Some(&gtk::TreeExpander::builder().child(&row).build()));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
            let (Some(expander), Some(row)) = (
                item.child().and_downcast::<gtk::TreeExpander>(),
                item.item().and_downcast::<gtk::TreeListRow>(),
            ) else {
                return;
            };
            expander.set_list_row(Some(&row));
            let Some(info) = row.item().and_downcast::<gio::FileInfo>() else { return };
            let Some(content) = expander.child() else { return };
            if let Some(image) = content.first_child().and_downcast::<gtk::Image>()
                && let Some(icon) = info.symbolic_icon()
            {
                image.set_from_gicon(&icon);
            }
            if let Some(label) = content.last_child().and_downcast::<gtk::Label>() {
                label.set_label(&info.display_name());
            }
        });
        let list = gtk::ListView::builder()
            .factory(&factory)
            .single_click_activate(true)
            .css_classes(["tree"])
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(200)
            .css_classes(["sidebar"])
            .build();
        widget.append(&title);
        widget.append(&scroll);

        let sidebar = Rc::new(Self {
            widget,
            title,
            list,
            root: RefCell::default(),
            on_open: RefCell::default(),
        });
        let weak = Rc::downgrade(&sidebar);
        sidebar.list.connect_activate(move |list, position| {
            let (Some(s), Some(row)) = (
                weak.upgrade(),
                list.model().and_then(|m| m.item(position)).and_downcast::<gtk::TreeListRow>(),
            ) else {
                return;
            };
            if row.is_expandable() {
                row.set_expanded(!row.is_expanded());
                return;
            }
            if let Some(path) = row
                .item()
                .and_downcast::<gio::FileInfo>()
                .and_then(|i| file_of(&i))
                .and_then(|f| f.path())
                && let Some(f) = s.on_open.borrow().as_ref()
            {
                f(&path);
            }
        });
        sidebar.set_root(None);
        sidebar
    }

    pub fn connect_open(&self, f: impl Fn(&Path) + 'static) {
        *self.on_open.borrow_mut() = Some(Box::new(f));
    }

    pub fn root(&self) -> Option<PathBuf> {
        self.root.borrow().clone()
    }

    pub fn set_root(&self, root: Option<&Path>) {
        *self.root.borrow_mut() = root.map(Path::to_path_buf);
        match root {
            Some(root) => {
                self.title.set_label(
                    &root
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| root.display().to_string()),
                );
                self.title.set_tooltip_text(Some(&root.display().to_string()));
                let tree = gtk::TreeListModel::new(folder(&gio::File::for_path(root)), false, false, |item| {
                    let info = item.downcast_ref::<gio::FileInfo>()?;
                    (info.file_type() == gio::FileType::Directory)
                        .then(|| file_of(info))
                        .flatten()
                        .map(|f| folder(&f))
                });
                self.list
                    .set_model(Some(&gtk::NoSelection::new(Some(tree))));
            }
            None => {
                self.title.set_label(&tr("edit-no-folder"));
                self.title.set_tooltip_text(None);
                self.list.set_model(None::<&gtk::NoSelection>);
            }
        }
    }
}

/// A folder's entries, shown ones only, folders first, by name.
fn folder(dir: &gio::File) -> gio::ListModel {
    let list = gtk::DirectoryList::new(
        Some("standard::name,standard::display-name,standard::type,standard::symbolic-icon,standard::is-hidden"),
        Some(dir),
    );
    let filter = gtk::CustomFilter::new(|item| {
        item.downcast_ref::<gio::FileInfo>().is_some_and(|i| {
            !i.is_hidden() && !SKIPPED.contains(&i.name().to_string_lossy().as_ref())
        })
    });
    let sorter = gtk::CustomSorter::new(|a, b| {
        let (Some(a), Some(b)) = (a.downcast_ref::<gio::FileInfo>(), b.downcast_ref::<gio::FileInfo>()) else {
            return gtk::Ordering::Equal;
        };
        let dir = |i: &gio::FileInfo| i.file_type() != gio::FileType::Directory;
        dir(a)
            .cmp(&dir(b))
            .then_with(|| orbit_common::natural::natural(&a.display_name().to_lowercase(), &b.display_name().to_lowercase()))
            .into()
    });
    let filtered = gtk::FilterListModel::new(Some(list), Some(filter));
    gtk::SortListModel::new(Some(filtered), Some(sorter)).upcast()
}

/// The file a directory listing's entry stands for.
fn file_of(info: &gio::FileInfo) -> Option<gio::File> {
    info.attribute_object("standard::file").and_downcast::<gio::File>()
}

// ------------------------------------------------------------ Ctrl+P

/// Finds a file of `root` by a few letters of its path and opens it.
pub fn quick_open(parent: &gtk::Window, root: &Path, open: impl Fn(&Path) + 'static) {
    let window = gtk::Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .default_width(600)
        .default_height(400)
        .css_classes(["quick-open"])
        .build();
    let entry = gtk::SearchEntry::builder().placeholder_text(tr("edit-quick-open")).build();
    let list = gtk::ListBox::builder().css_classes(["results"]).build();
    let scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(5)
        .css_classes(["quick-open-box"])
        .build();
    content.append(&entry);
    content.append(&scroll);
    window.set_child(Some(&content));

    let files: Rc<RefCell<Vec<String>>> = Rc::default();
    let root = root.to_path_buf();
    let open = Rc::new(open);
    let refresh = {
        let files = files.clone();
        let list = list.clone();
        let entry = entry.clone();
        move || {
            while let Some(row) = list.first_child() {
                list.remove(&row);
            }
            let query = entry.text().to_lowercase();
            let files = files.borrow();
            let mut scored: Vec<(i32, usize)> = files
                .iter()
                .enumerate()
                .filter_map(|(n, f)| score(&query, f).map(|s| (s, n)))
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| files[a.1].len().cmp(&files[b.1].len())));
            for file in scored.into_iter().take(50).map(|(_, n)| &files[n]) {
                let label = gtk::Label::builder()
                    .label(file.as_str())
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::Start)
                    .build();
                list.append(&label);
            }
            if let Some(first) = list.row_at_index(0) {
                list.select_row(Some(&first));
            }
        }
    };
    let refresh = Rc::new(refresh);
    {
        let refresh = refresh.clone();
        entry.connect_search_changed(move |_| refresh());
    }
    let choose = {
        let window = window.clone();
        let root = root.clone();
        let open = open.clone();
        move |row: &gtk::ListBoxRow| {
            if let Some(label) = row.child().and_downcast::<gtk::Label>() {
                open(&root.join(label.label().as_str()));
            }
            window.close();
        }
    };
    let choose = Rc::new(choose);
    {
        let choose = choose.clone();
        list.connect_row_activated(move |_, row| choose(row));
    }
    {
        let list = list.clone();
        let choose = choose.clone();
        entry.connect_activate(move |_| {
            if let Some(row) = list.selected_row() {
                choose(&row);
            }
        });
    }
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let window = window.clone();
        let list = list.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            let step = match key {
                gdk::Key::Escape => {
                    window.close();
                    return glib::Propagation::Stop;
                }
                gdk::Key::Down => 1,
                gdk::Key::Up => -1,
                _ => return glib::Propagation::Proceed,
            };
            let at = list.selected_row().map_or(0, |r| r.index());
            if let Some(row) = list.row_at_index((at + step).max(0)) {
                list.select_row(Some(&row));
                row.grab_focus();
                GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
            }
            glib::Propagation::Stop
        });
    }
    window.add_controller(keys);

    // The files, listed on a thread of their own.
    let (tx, rx) = async_channel::bounded::<Vec<String>>(1);
    let walk_root = root.clone();
    std::thread::spawn(move || {
        let mut found = Vec::new();
        walk(&walk_root, &walk_root, &mut found);
        let _ = tx.send_blocking(found);
    });
    glib::spawn_future_local(async move {
        if let Ok(found) = rx.recv().await {
            *files.borrow_mut() = found;
            refresh();
        }
    });
    window.present();
    entry.grab_focus();
}

fn walk(root: &Path, dir: &Path, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.filter_map(Result::ok) {
        if found.len() >= MOST_FILES {
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || SKIPPED.contains(&name.as_str()) {
            continue;
        }
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(root, &path, found),
            Ok(t) if t.is_file() => {
                if let Ok(rel) = path.strip_prefix(root) {
                    found.push(rel.to_string_lossy().into_owned());
                }
            }
            _ => {}
        }
    }
}

/// How well `query` matches `path`: its letters in order, closer together
/// and in the file's name scoring higher; `None` if they are not all there.
fn score(query: &str, path: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let lower = path.to_lowercase();
    let name_start = lower.rfind('/').map_or(0, |i| i + 1);
    let mut score = 0;
    let mut last: Option<usize> = None;
    let mut from = 0;
    for q in query.chars().filter(|c| !c.is_whitespace()) {
        let found = lower[from..].find(q)? + from;
        score += 1;
        if last.is_some_and(|l| l + 1 == found) {
            score += 5;
        }
        if found >= name_start {
            score += 3;
        }
        last = Some(found);
        from = found + q.len_utf8();
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_in_order() {
        assert!(score("mrs", "src/main.rs").is_some());
        assert!(score("xyz", "src/main.rs").is_none());
        // Together, and in the name, beats scattered.
        assert!(score("main", "src/main.rs") > score("main", "m/a/i/n.txt"));
    }
}
