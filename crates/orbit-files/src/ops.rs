//! File operations — copy, move, trash, delete — each on a thread of its
//! own, with a row in the jobs bar: what it is doing, how far along, and a
//! button to stop it. When a file is already where it is going, the job
//! asks: skip, keep both, or replace — for this one or for all.
//!
//! Copies go in 1 MiB pieces, so progress shows and stopping is prompt;
//! moves within a disk are renames (instant); across disks, a copy and a
//! removal. Pasting into the folder a file came from makes "name (2)".

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::{number, text, tr, tr_args};

const CHUNK: usize = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Copy,
    Move,
    Trash,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Skip,
    SkipAll,
    KeepBoth,
    Replace,
    ReplaceAll,
}

enum Report {
    Progress { done: u64, total: u64, name: String },
    Conflict(PathBuf, async_channel::Sender<Answer>),
    Finished(Vec<String>),
}

/// The bar of running jobs, along the window's bottom.
pub struct Jobs {
    pub widget: gtk::Box,
    window: glib::WeakRef<gtk::Window>,
}

impl Jobs {
    pub fn new(window: &gtk::Window) -> Rc<Self> {
        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["jobs"])
            .visible(false)
            .build();
        Rc::new(Self {
            widget,
            window: window.downgrade(),
        })
    }

    /// Starts `kind` on `sources`, into `target` (copy, move).
    pub fn start(self: &Rc<Self>, kind: Kind, sources: Vec<PathBuf>, target: Option<PathBuf>) {
        if sources.is_empty() {
            return;
        }
        let label = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).hexpand(true).build();
        let bar = gtk::ProgressBar::builder().hexpand(true).build();
        let stop = gtk::Button::builder()
            .icon_name("process-stop-symbolic")
            .tooltip_text(tr("files-job-stop"))
            .css_classes(["flat"])
            .build();
        let row = gtk::Box::builder().spacing(10).css_classes(["job"]).build();
        let texts = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
        texts.append(&label);
        texts.append(&bar);
        row.append(&texts);
        row.append(&stop);
        self.widget.append(&row);
        self.widget.set_visible(true);

        let what = tr_args(
            match kind {
                Kind::Copy => "files-job-copy",
                Kind::Move => "files-job-move",
                Kind::Trash => "files-job-trash",
                Kind::Delete => "files-job-delete",
            },
            &[("n", number(sources.len() as f64))],
        );
        label.set_label(&what);
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let cancel = cancel.clone();
            stop.connect_clicked(move |_| cancel.store(true, Ordering::Relaxed));
        }
        let (tx, rx) = async_channel::unbounded();
        std::thread::Builder::new()
            .name("orbit-files-job".into())
            .spawn(move || {
                let errors = run(kind, &sources, target.as_deref(), &tx, &cancel);
                let _ = tx.send_blocking(Report::Finished(errors));
            })
            .ok();
        let jobs = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(report) = rx.recv().await {
                match report {
                    Report::Progress { done, total, name } => {
                        bar.set_fraction(if total > 0 { done as f64 / total as f64 } else { 0.0 });
                        label.set_label(&format!("{what} — {name}"));
                    }
                    Report::Conflict(path, reply) => {
                        let answer = match jobs.upgrade() {
                            Some(j) => j.ask(&path).await,
                            None => Answer::Skip,
                        };
                        let _ = reply.send(answer).await;
                    }
                    Report::Finished(errors) => {
                        if let Some(j) = jobs.upgrade() {
                            j.widget.remove(&row);
                            j.widget.set_visible(j.widget.first_child().is_some());
                            if !errors.is_empty() {
                                j.error(&errors);
                            }
                        }
                        return;
                    }
                }
            }
        });
    }

    /// Runs an outside program (an archiver) as a job: `what` while it
    /// runs, its error output if it fails.
    pub fn command(self: &Rc<Self>, what: String, argv: Vec<String>, dir: PathBuf) {
        let label = gtk::Label::builder().label(&what).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).hexpand(true).build();
        let bar = gtk::ProgressBar::builder().hexpand(true).build();
        let row = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["job"]).build();
        row.append(&label);
        row.append(&bar);
        self.widget.append(&row);
        self.widget.set_visible(true);
        let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::STDERR_PIPE | gio::SubprocessFlags::STDOUT_SILENCE);
        launcher.set_cwd(&dir);
        let args: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
        let jobs = Rc::downgrade(self);
        let process = match launcher.spawn(&args) {
            Ok(p) => p,
            Err(err) => {
                self.widget.remove(&row);
                self.error(&[err.message().to_owned()]);
                return;
            }
        };
        let pulse = glib::timeout_add_local(std::time::Duration::from_millis(150), {
            let bar = bar.clone();
            move || {
                bar.pulse();
                glib::ControlFlow::Continue
            }
        });
        glib::spawn_future_local(async move {
            let output = process.communicate_utf8_future(None).await;
            pulse.remove();
            let Some(j) = jobs.upgrade() else { return };
            j.widget.remove(&row);
            j.widget.set_visible(j.widget.first_child().is_some());
            if !process.is_successful() {
                let detail = output.ok().and_then(|(_, err)| err).map(|e| e.to_string()).unwrap_or_default();
                j.error(&[format!("{what}\n{detail}")]);
            }
        });
    }

    async fn ask(&self, path: &Path) -> Answer {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dialog = gtk::AlertDialog::builder()
            .modal(true)
            .message(tr_args("files-conflict", &[("name", text(name))]))
            .detail(tr("files-conflict-detail"))
            .buttons([
                tr("files-conflict-skip"),
                tr("files-conflict-skip-all"),
                tr("files-conflict-keep"),
                tr("files-conflict-replace"),
                tr("files-conflict-replace-all"),
            ])
            .cancel_button(0)
            .default_button(2)
            .build();
        let window = self.window.upgrade();
        match dialog.choose_future(window.as_ref()).await {
            Ok(1) => Answer::SkipAll,
            Ok(2) => Answer::KeepBoth,
            Ok(3) => Answer::Replace,
            Ok(4) => Answer::ReplaceAll,
            _ => Answer::Skip,
        }
    }

    fn error(&self, errors: &[String]) {
        let mut detail = errors.iter().take(10).cloned().collect::<Vec<_>>().join("\n");
        if errors.len() > 10 {
            detail.push_str("\n…");
        }
        gtk::AlertDialog::builder()
            .modal(true)
            .message(tr("files-job-errors"))
            .detail(detail)
            .build()
            .show(self.window.upgrade().as_ref());
    }
}

struct Ctx<'a> {
    tx: &'a async_channel::Sender<Report>,
    cancel: &'a AtomicBool,
    done: u64,
    total: u64,
    errors: Vec<String>,
    /// SkipAll / ReplaceAll, once chosen.
    standing: Option<Answer>,
}

fn run(
    kind: Kind,
    sources: &[PathBuf],
    target: Option<&Path>,
    tx: &async_channel::Sender<Report>,
    cancel: &AtomicBool,
) -> Vec<String> {
    let mut ctx = Ctx {
        tx,
        cancel,
        done: 0,
        total: 0,
        errors: Vec::new(),
        standing: None,
    };
    match kind {
        Kind::Trash => {
            ctx.total = sources.len() as u64;
            for src in sources {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                ctx.report(src);
                if let Err(err) = gio::File::for_path(src).trash(gio::Cancellable::NONE) {
                    ctx.errors.push(format!("{}: {}", src.display(), err.message()));
                }
                ctx.done += 1;
            }
        }
        Kind::Delete => {
            ctx.total = sources.len() as u64;
            for src in sources {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                ctx.report(src);
                let removed = if src.is_dir() && !src.is_symlink() {
                    std::fs::remove_dir_all(src)
                } else {
                    std::fs::remove_file(src)
                };
                if let Err(err) = removed {
                    ctx.errors.push(format!("{}: {err}", src.display()));
                }
                ctx.done += 1;
            }
        }
        Kind::Copy | Kind::Move => {
            let Some(target) = target else { return ctx.errors };
            ctx.total = sources.iter().map(|s| size_of(s)).sum();
            for src in sources {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let Some(name) = src.file_name() else { continue };
                // Into itself: refused, as every file manager does.
                if target.starts_with(src) {
                    ctx.errors.push(tr_args("files-into-itself", &[("name", text(src.display().to_string()))]));
                    continue;
                }
                let mut dest = target.join(name);
                if src.parent() == Some(target) {
                    if kind == Kind::Move {
                        continue;
                    }
                    dest = unique(&dest);
                } else if dest.symlink_metadata().is_ok() {
                    match ctx.conflict(&dest) {
                        Answer::Skip | Answer::SkipAll => {
                            ctx.done += size_of(src);
                            continue;
                        }
                        Answer::KeepBoth => dest = unique(&dest),
                        Answer::Replace | Answer::ReplaceAll => {
                            let removed = if dest.is_dir() && !dest.is_symlink() {
                                std::fs::remove_dir_all(&dest)
                            } else {
                                std::fs::remove_file(&dest)
                            };
                            if let Err(err) = removed {
                                ctx.errors.push(format!("{}: {err}", dest.display()));
                                continue;
                            }
                        }
                    }
                }
                if kind == Kind::Move && std::fs::rename(src, &dest).is_ok() {
                    ctx.done += size_of(&dest);
                    ctx.report(&dest);
                    continue;
                }
                let copied = ctx.copy(src, &dest);
                if copied && kind == Kind::Move && !cancel.load(Ordering::Relaxed) {
                    let removed = if src.is_dir() && !src.is_symlink() {
                        std::fs::remove_dir_all(src)
                    } else {
                        std::fs::remove_file(src)
                    };
                    if let Err(err) = removed {
                        ctx.errors.push(format!("{}: {err}", src.display()));
                    }
                }
            }
        }
    }
    ctx.errors
}

impl Ctx<'_> {
    fn report(&self, path: &Path) {
        let _ = self.tx.send_blocking(Report::Progress {
            done: self.done,
            total: self.total,
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        });
    }

    fn conflict(&mut self, dest: &Path) -> Answer {
        if let Some(standing) = self.standing {
            return standing;
        }
        let (reply, answer) = async_channel::bounded(1);
        if self.tx.send_blocking(Report::Conflict(dest.to_path_buf(), reply)).is_err() {
            return Answer::Skip;
        }
        let a = answer.recv_blocking().unwrap_or(Answer::Skip);
        if matches!(a, Answer::SkipAll | Answer::ReplaceAll) {
            self.standing = Some(a);
        }
        a
    }

    /// Copies `src` to `dest`, folders whole; whether it all went.
    fn copy(&mut self, src: &Path, dest: &Path) -> bool {
        if self.cancel.load(Ordering::Relaxed) {
            return false;
        }
        let Ok(meta) = src.symlink_metadata() else {
            self.errors.push(format!("{}: ?", src.display()));
            return false;
        };
        let result = if meta.file_type().is_symlink() {
            std::fs::read_link(src).and_then(|t| std::os::unix::fs::symlink(t, dest))
        } else if meta.is_dir() {
            if let Err(err) = std::fs::create_dir_all(dest) {
                Err(err)
            } else {
                let mut all = true;
                if let Ok(entries) = std::fs::read_dir(src) {
                    for entry in entries.filter_map(Result::ok) {
                        all &= self.copy(&entry.path(), &dest.join(entry.file_name()));
                    }
                }
                let _ = std::fs::set_permissions(dest, meta.permissions());
                return all;
            }
        } else {
            self.copy_file(src, dest, &meta)
        };
        match result {
            Ok(()) => true,
            Err(err) => {
                self.errors.push(format!("{}: {err}", src.display()));
                false
            }
        }
    }

    fn copy_file(&mut self, src: &Path, dest: &Path, meta: &std::fs::Metadata) -> std::io::Result<()> {
        let mut from = std::fs::File::open(src)?;
        let mut to = std::fs::File::create(dest)?;
        let mut buffer = vec![0u8; CHUNK];
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                drop(to);
                let _ = std::fs::remove_file(dest);
                return Ok(());
            }
            let n = from.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            to.write_all(&buffer[..n])?;
            self.done += n as u64;
            self.report(src);
        }
        to.set_permissions(meta.permissions())?;
        if let Ok(modified) = meta.modified() {
            let _ = to.set_modified(modified);
        }
        Ok(())
    }
}

/// Bytes under `path`.
fn size_of(path: &Path) -> u64 {
    let Ok(meta) = path.symlink_metadata() else { return 0 };
    if meta.is_dir() {
        std::fs::read_dir(path)
            .map(|d| d.filter_map(Result::ok).map(|e| size_of(&e.path())).sum())
            .unwrap_or(0)
    } else {
        meta.len()
    }
}

/// `name (2).ext`, `name (3).ext`… — the first free.
pub fn unique(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && !path.is_dir() => (name[..i].to_owned(), name[i..].to_owned()),
        _ => (name.clone(), String::new()),
    };
    (2..)
        .map(|n| parent.join(format!("{stem} ({n}){ext}")))
        .find(|p| p.symlink_metadata().is_err())
        .expect("a free name")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_folders_and_names_duplicates() {
        let root = std::env::temp_dir().join(format!("orbit-files-ops-{}", std::process::id()));
        let src = root.join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), b"hello").unwrap();
        std::fs::write(src.join("sub/b.txt"), vec![7u8; 3 * CHUNK + 5]).unwrap();
        let dest = root.join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        let (tx, rx) = async_channel::unbounded();
        let cancel = AtomicBool::new(false);
        let errors = run(Kind::Copy, std::slice::from_ref(&src), Some(&dest), &tx, &cancel);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(std::fs::read(dest.join("src/a.txt")).unwrap(), b"hello");
        assert_eq!(std::fs::metadata(dest.join("src/sub/b.txt")).unwrap().len(), (3 * CHUNK + 5) as u64);
        assert!(rx.try_recv().is_ok());
        // Into the same folder: a second copy beside it.
        let errors = run(Kind::Copy, &[src.join("a.txt")], Some(&src), &tx, &cancel);
        assert!(errors.is_empty());
        assert!(src.join("a (2).txt").is_file());
        // Moving within a disk: a rename.
        let errors = run(Kind::Move, &[src.join("a (2).txt")], Some(&dest), &tx, &cancel);
        assert!(errors.is_empty());
        assert!(dest.join("a (2).txt").is_file() && !src.join("a (2).txt").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
