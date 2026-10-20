//! Thumbnails for the icon view: from the freedesktop.org cache
//! (`~/.cache/thumbnails/normal`), which other programs share, or made by
//! the thumbnailers the system has installed (`/usr/share/thumbnailers`:
//! images, videos, PDFs…) on three threads, as Nautilus does.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

use gtk::{gdk, gio, glib, prelude::*};

use crate::entry::Entry;

const WORKERS: usize = 3;
const SIZE: &str = "128";

struct Thumbnailers {
    /// MIME type → its command line, with %i %o %s %u.
    exec: HashMap<String, String>,
}

fn thumbnailers() -> &'static Thumbnailers {
    static T: OnceLock<Thumbnailers> = OnceLock::new();
    T.get_or_init(|| {
        let mut exec = HashMap::new();
        let dirs = [PathBuf::from("/usr/share/thumbnailers"), glib::user_data_dir().join("thumbnailers")];
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            for entry in entries.filter_map(Result::ok) {
                let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
                let field = |key: &str| {
                    text.lines()
                        .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
                        .map(str::trim)
                };
                // A thumbnailer whose program is gone is no use.
                if let Some(program) = field("TryExec")
                    && glib::find_program_in_path(program).is_none()
                {
                    continue;
                }
                let (Some(command), Some(types)) = (field("Exec"), field("MimeType")) else { continue };
                if glib::find_program_in_path(command.split_whitespace().next().unwrap_or("")).is_none() {
                    continue;
                }
                for mime in types.split(';').filter(|m| !m.is_empty()) {
                    exec.entry(mime.to_owned()).or_insert_with(|| command.to_owned());
                }
            }
        }
        Thumbnailers { exec }
    })
}

struct Queue {
    requests: async_channel::Sender<(String, PathBuf, String)>,
    /// URI → the images waiting for it.
    waiting: RefCell<HashMap<String, Vec<glib::WeakRef<gtk::Image>>>>,
    failed: RefCell<HashSet<String>>,
}

thread_local! {
    static QUEUE: Queue = start();
}

fn start() -> Queue {
    let (requests, rx) = async_channel::unbounded::<(String, PathBuf, String)>();
    let (done, results) = async_channel::unbounded::<(String, Option<PathBuf>)>();
    for _ in 0..WORKERS {
        let rx = rx.clone();
        let done = done.clone();
        std::thread::Builder::new()
            .name("orbit-files-thumbs".into())
            .spawn(move || {
                while let Ok((uri, path, mime)) = rx.recv_blocking() {
                    let made = make(&uri, &path, &mime);
                    if done.send_blocking((uri, made)).is_err() {
                        break;
                    }
                }
            })
            .ok();
    }
    glib::spawn_future_local(async move {
        while let Ok((uri, made)) = results.recv().await {
            QUEUE.with(|q| {
                let images = q.waiting.borrow_mut().remove(&uri).unwrap_or_default();
                match made {
                    Some(file) => {
                        for image in images.iter().filter_map(|w| w.upgrade()) {
                            if wants(&image, &uri) {
                                set(&image, &file);
                            }
                        }
                    }
                    None => {
                        q.failed.borrow_mut().insert(uri);
                    }
                }
            });
        }
    });
    Queue {
        requests,
        waiting: RefCell::default(),
        failed: RefCell::default(),
    }
}

/// Shows `entry`'s thumbnail in `image`, now if cached, else once made.
pub fn request(entry: &Entry, image: &gtk::Image) {
    let Some(path) = entry.location.path() else { return };
    if entry.dir || !(entry.mime.starts_with("image/") || entry.mime.starts_with("video/") || entry.mime == "application/pdf") {
        mark(image, "");
        return;
    }
    let uri = gio::File::for_path(path).uri().to_string();
    mark(image, &uri);
    let cached = cache_file(&uri);
    if is_fresh(&cached, path) {
        set(image, &cached);
        return;
    }
    if !thumbnailers().exec.contains_key(&entry.mime) {
        return;
    }
    QUEUE.with(|q| {
        if q.failed.borrow().contains(&uri) {
            return;
        }
        let mut waiting = q.waiting.borrow_mut();
        let first = !waiting.contains_key(&uri);
        waiting.entry(uri.clone()).or_default().push(image.downgrade());
        if first {
            let _ = q.requests.send_blocking((uri, path.to_path_buf(), entry.mime.clone()));
        }
    });
}

/// The image is recycled for other entries as the view scrolls: it keeps
/// the URI it now shows, so a late thumbnail goes nowhere wrong.
fn mark(image: &gtk::Image, uri: &str) {
    unsafe {
        image.set_data("orbit-thumb", uri.to_owned());
    }
}

fn wants(image: &gtk::Image, uri: &str) -> bool {
    unsafe { image.data::<String>("orbit-thumb").is_some_and(|u| u.as_ref() == uri) }
}

fn set(image: &gtk::Image, file: &Path) {
    if let Ok(texture) = gdk::Texture::from_filename(file) {
        image.set_paintable(Some(&texture));
    }
}

fn cache_file(uri: &str) -> PathBuf {
    let md5 = glib::compute_checksum_for_string(glib::ChecksumType::Md5, uri).unwrap_or_default();
    glib::user_cache_dir().join("thumbnails").join("normal").join(format!("{md5}.png"))
}

/// A cached thumbnail no older than the file.
fn is_fresh(thumb: &Path, file: &Path) -> bool {
    match (std::fs::metadata(thumb).and_then(|m| m.modified()), std::fs::metadata(file).and_then(|m| m.modified())) {
        (Ok(t), Ok(f)) => t >= f,
        _ => false,
    }
}

/// Runs the thumbnailer for `mime` on `path`; the thumbnail's file, if made.
fn make(uri: &str, path: &Path, mime: &str) -> Option<PathBuf> {
    let command = thumbnailers().exec.get(mime)?;
    let out = cache_file(uri);
    std::fs::create_dir_all(out.parent()?).ok()?;
    let argv: Vec<String> = command
        .split_whitespace()
        .map(|a| {
            a.replace("%i", &path.to_string_lossy())
                .replace("%o", &out.to_string_lossy())
                .replace("%s", SIZE)
                .replace("%u", uri)
        })
        .collect();
    let status = Command::new(&argv[0]).args(&argv[1..]).output().ok()?;
    (status.status.success() && out.is_file()).then_some(out)
}
