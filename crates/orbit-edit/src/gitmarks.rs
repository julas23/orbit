//! The git marks in the margin: a green bar beside lines added since the
//! last commit, blue beside the ones changed, a red notch where lines were
//! taken out. The text is compared with the committed file (`git show
//! HEAD:file`) as it is typed, not only once saved.

use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use gtk::{gdk, glib, graphene, prelude::*, subclass::prelude::*};
use sourceview5::subclass::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    Added,
    Changed,
    /// Lines taken out just above this one.
    Removed,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Renderer {
        pub marks: RefCell<HashMap<u32, Mark>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Renderer {
        const NAME: &'static str = "OrbitEditGitRenderer";
        type Type = super::Renderer;
        type ParentType = sourceview5::GutterRenderer;
    }

    impl ObjectImpl for Renderer {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().set_size_request(4, -1);
        }
    }
    impl WidgetImpl for Renderer {}
    impl GutterRendererImpl for Renderer {
        fn snapshot_line(&self, snapshot: &gtk::Snapshot, lines: &sourceview5::GutterLines, line: u32) {
            let Some(mark) = self.marks.borrow().get(&line).copied() else {
                return;
            };
            let (y, height) = lines.line_yrange(line, sourceview5::GutterRendererAlignmentMode::Cell);
            let (color, rect) = match mark {
                Mark::Added => ("#a3be8c", graphene::Rect::new(0.0, y as f32, 3.0, height as f32)),
                Mark::Changed => ("#81a1c1", graphene::Rect::new(0.0, y as f32, 3.0, height as f32)),
                Mark::Removed => ("#bf616a", graphene::Rect::new(0.0, y as f32 - 1.0, 4.0, 3.0)),
            };
            snapshot.append_color(&gdk::RGBA::parse(color).expect("a colour"), &rect);
        }
    }
}

glib::wrapper! {
    pub struct Renderer(ObjectSubclass<imp::Renderer>)
        @extends sourceview5::GutterRenderer, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

pub struct GitMarks {
    renderer: Renderer,
    buffer: sourceview5::Buffer,
    /// The committed text of the file; None outside a repository, or for
    /// a file never committed.
    base: RefCell<Option<String>>,
    path: RefCell<Option<PathBuf>>,
    soon: RefCell<Option<glib::SourceId>>,
}

impl GitMarks {
    pub fn new(view: &sourceview5::View) -> Rc<Self> {
        let renderer: Renderer = glib::Object::new();
        sourceview5::prelude::ViewExt::gutter(view, gtk::TextWindowType::Left).insert(&renderer, 5);
        let buffer = view
            .buffer()
            .downcast::<sourceview5::Buffer>()
            .expect("a source buffer");
        let marks = Rc::new(Self {
            renderer,
            buffer,
            base: RefCell::default(),
            path: RefCell::default(),
            soon: RefCell::default(),
        });
        let weak = Rc::downgrade(&marks);
        marks.buffer.connect_changed(move |_| {
            if let Some(m) = weak.upgrade() {
                m.compare_soon();
            }
        });
        marks
    }

    /// Reads the committed file again (after opening or saving `path`),
    /// on a thread, then compares.
    pub fn refresh(self: &Rc<Self>, path: Option<&Path>) {
        *self.path.borrow_mut() = path.map(Path::to_path_buf);
        let Some(path) = path.map(Path::to_path_buf) else {
            *self.base.borrow_mut() = None;
            self.compare();
            return;
        };
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(committed(&path));
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            if let (Ok(base), Some(m)) = (rx.recv().await, weak.upgrade()) {
                *m.base.borrow_mut() = base;
                m.compare();
            }
        });
    }

    fn compare_soon(self: &Rc<Self>) {
        if self.base.borrow().is_none() {
            return;
        }
        if let Some(id) = self.soon.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(400), move || {
            if let Some(m) = weak.upgrade() {
                m.soon.borrow_mut().take();
                m.compare();
            }
        });
        *self.soon.borrow_mut() = Some(id);
    }

    fn compare(&self) {
        let marks = match self.base.borrow().as_deref() {
            Some(base) => {
                let (start, end) = self.buffer.bounds();
                diff(base, &self.buffer.text(&start, &end, true))
            }
            None => HashMap::new(),
        };
        *self.renderer.imp().marks.borrow_mut() = marks;
        self.renderer.queue_draw();
    }
}

/// The file as last committed, read with git in its folder.
fn committed(path: &Path) -> Option<String> {
    let dir = path.parent()?;
    let name = path.file_name()?.to_string_lossy();
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("show")
        .arg(format!("HEAD:./{name}"))
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"))
}

/// The marks, by line of `now`, of how it differs from `base`.
pub fn diff(base: &str, now: &str) -> HashMap<u32, Mark> {
    let mut marks = HashMap::new();
    let diff = similar::TextDiff::from_lines(base, now);
    for op in diff.ops() {
        match *op {
            similar::DiffOp::Insert { new_index, new_len, .. } => {
                for l in new_index..new_index + new_len {
                    marks.insert(l as u32, Mark::Added);
                }
            }
            similar::DiffOp::Replace { new_index, new_len, .. } => {
                for l in new_index..new_index + new_len {
                    marks.insert(l as u32, Mark::Changed);
                }
            }
            similar::DiffOp::Delete { new_index, .. } => {
                marks.entry(new_index as u32).or_insert(Mark::Removed);
            }
            similar::DiffOp::Equal { .. } => {}
        }
    }
    marks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks() {
        let m = diff("a\nb\nc\n", "a\nB\nc\nd\n");
        assert_eq!(m.get(&1), Some(&Mark::Changed));
        assert_eq!(m.get(&3), Some(&Mark::Added));
        assert_eq!(m.get(&0), None);
        let m = diff("a\nb\nc\n", "a\nc\n");
        assert_eq!(m.get(&1), Some(&Mark::Removed));
    }
}
