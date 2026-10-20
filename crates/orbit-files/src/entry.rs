//! A folder's entries, listed on a thread of their own and sent to the
//! window in batches, so the first files show at once and a folder of a
//! hundred thousand never freezes it — Yazi's way.
//!
//! Local folders (and network ones GVfs has mounted, which appear under
//! `/run/user/…/gvfs`) are read with plain system calls: one `readdir`
//! and one `lstat` per entry, nothing else. Places only GIO can read — the
//! trash, a network share not yet mounted — go through GIO. The type of a
//! file is guessed from its name alone, never by opening it.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::UNIX_EPOCH,
};

use gtk::{gio, glib, prelude::*};

/// Entries per batch sent to the window.
const BATCH: usize = 2000;

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    /// Where it is: a local path, or a URI for what GIO alone reaches.
    pub location: Location,
    pub dir: bool,
    pub link: bool,
    pub hidden: bool,
    pub size: u64,
    /// Seconds since 1970.
    pub modified: i64,
    pub mime: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Location {
    Path(PathBuf),
    Uri(String),
}

impl Location {
    pub fn file(&self) -> gio::File {
        match self {
            Location::Path(p) => gio::File::for_path(p),
            Location::Uri(u) => gio::File::for_uri(u),
        }
    }

    pub fn path(&self) -> Option<&Path> {
        match self {
            Location::Path(p) => Some(p),
            Location::Uri(_) => None,
        }
    }

    /// From a GIO file: its local path when it has one (GVfs gives
    /// mounted shares one).
    pub fn of(file: &gio::File) -> Self {
        match file.path() {
            Some(p) => Location::Path(p),
            None => Location::Uri(file.uri().to_string()),
        }
    }

    pub fn display(&self) -> String {
        match self {
            Location::Path(p) => p.display().to_string(),
            Location::Uri(u) => u.clone(),
        }
    }

    pub fn name(&self) -> String {
        match self {
            Location::Path(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "/".into()),
            Location::Uri(_) => self
                .file()
                .basename()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.display()),
        }
    }

    pub fn parent(&self) -> Option<Location> {
        match self {
            Location::Path(p) => p.parent().map(|p| Location::Path(p.to_path_buf())),
            Location::Uri(_) => self.file().parent().map(|f| Location::of(&f)),
        }
    }
}

pub enum Listing {
    Batch(Vec<Entry>),
    Done,
    Failed(String),
}

/// Lists `dir` on a thread; the entries arrive on the channel.
pub fn list(dir: Location) -> async_channel::Receiver<Listing> {
    let (tx, rx) = async_channel::unbounded();
    std::thread::Builder::new()
        .name("orbit-files-list".into())
        .spawn(move || {
            let result = match &dir {
                Location::Path(p) => list_local(p, &tx),
                Location::Uri(_) => list_gio(&dir.file(), &tx),
            };
            let _ = tx.send_blocking(match result {
                Ok(()) => Listing::Done,
                Err(err) => Listing::Failed(err),
            });
        })
        .ok();
    rx
}

fn list_local(dir: &Path, tx: &async_channel::Sender<Listing>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    let mut batch = Vec::with_capacity(BATCH);
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        let link = meta.file_type().is_symlink();
        // A link counts as what it points to.
        let target = if link { std::fs::metadata(&path).ok() } else { None };
        let dir = target.as_ref().map_or(meta.is_dir(), |t| t.is_dir());
        let size = target.as_ref().map_or(meta.len(), |t| t.len());
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64);
        batch.push(Entry {
            hidden: name.starts_with('.'),
            mime: if dir { "inode/directory".into() } else { mime_of(&name) },
            name,
            location: Location::Path(path),
            dir,
            link,
            size,
            modified,
        });
        if batch.len() >= BATCH {
            if tx.send_blocking(Listing::Batch(std::mem::take(&mut batch))).is_err() {
                return Ok(());
            }
            batch.reserve(BATCH);
        }
    }
    if !batch.is_empty() {
        let _ = tx.send_blocking(Listing::Batch(batch));
    }
    Ok(())
}

fn list_gio(dir: &gio::File, tx: &async_channel::Sender<Listing>) -> Result<(), String> {
    let children = dir
        .enumerate_children(
            "standard::name,standard::display-name,standard::type,standard::size,standard::content-type,standard::is-hidden,standard::is-symlink,time::modified",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )
        .map_err(|e| e.message().to_owned())?;
    let mut batch = Vec::new();
    while let Ok(Some(info)) = children.next_file(gio::Cancellable::NONE) {
        let dir_ = info.file_type() == gio::FileType::Directory;
        let child = children.child(&info);
        batch.push(Entry {
            name: info.display_name().to_string(),
            location: Location::of(&child),
            dir: dir_,
            link: info.is_symlink(),
            hidden: info.is_hidden(),
            size: info.size().max(0) as u64,
            modified: info.modification_date_time().map_or(0, |d| d.to_unix()),
            mime: info
                .content_type()
                .and_then(|c| gio::content_type_get_mime_type(&c))
                .map(|m| m.to_string())
                .unwrap_or_else(|| if dir_ { "inode/directory".into() } else { "application/octet-stream".into() }),
        });
        if batch.len() >= BATCH && tx.send_blocking(Listing::Batch(std::mem::take(&mut batch))).is_err() {
            return Ok(());
        }
    }
    if !batch.is_empty() {
        let _ = tx.send_blocking(Listing::Batch(batch));
    }
    Ok(())
}

/// The MIME type from the name — cached by extension, so a folder of
/// photos costs one guess.
pub fn mime_of(name: &str) -> String {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let key = match name.rfind('.') {
        Some(i) if i > 0 => name[i..].to_lowercase(),
        // No extension: the whole name decides (Makefile, README).
        _ => name.to_owned(),
    };
    let cache = CACHE.get_or_init(Default::default);
    if let Some(m) = cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return m;
    }
    let (kind, _) = gio::content_type_guess(Some(name), None);
    let mime = gio::content_type_get_mime_type(&kind)
        .map(|m| m.to_string())
        .unwrap_or_else(|| "application/octet-stream".into());
    if let Ok(mut c) = cache.lock() {
        c.insert(key, mime.clone());
    }
    mime
}

/// The icon for an entry: the folder's own for the special folders, else
/// its type's, cached.
pub fn icon(entry: &Entry) -> gio::Icon {
    thread_local! {
        static ICONS: std::cell::RefCell<HashMap<String, gio::Icon>> = Default::default();
    }
    if entry.dir {
        if let Some(path) = entry.location.path()
            && let Some(name) = special_folder_icon(path)
        {
            return gio::ThemedIcon::new(name).upcast();
        }
        return gio::ThemedIcon::from_names(&["folder"]).upcast();
    }
    ICONS.with(|icons| {
        icons
            .borrow_mut()
            .entry(entry.mime.clone())
            .or_insert_with(|| gio::content_type_get_icon(&entry.mime))
            .clone()
    })
}

/// "folder-documents" for ~/Documents and the like.
pub fn special_folder_icon(path: &Path) -> Option<&'static str> {
    use glib::UserDirectory as U;
    let home = glib::home_dir();
    if path == home {
        return Some("user-home");
    }
    [
        (U::Desktop, "user-desktop"),
        (U::Documents, "folder-documents"),
        (U::Downloads, "folder-download"),
        (U::Music, "folder-music"),
        (U::Pictures, "folder-pictures"),
        (U::Videos, "folder-videos"),
        (U::Templates, "folder-templates"),
        (U::PublicShare, "folder-publicshare"),
    ]
    .into_iter()
    .find(|(dir, _)| glib::user_special_dir(*dir).as_deref() == Some(path))
    .map(|(_, icon)| icon)
}

/// 1,2 MB — GLib's, in the user's language.
pub fn size_text(size: u64) -> String {
    glib::format_size(size).to_string()
}

/// The date the way the language writes it.
pub fn date_text(secs: i64) -> String {
    glib::DateTime::from_unix_local(secs)
        .ok()
        .and_then(|d| d.format("%x %H:%M").ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_a_folder_in_batches() {
        let dir = std::env::temp_dir().join(format!("orbit-files-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for n in 0..2500 {
            std::fs::write(dir.join(format!("f{n}.txt")), b"x").unwrap();
        }
        let rx = list(Location::Path(dir.clone()));
        let mut count = 0;
        let mut batches = 0;
        loop {
            match rx.recv_blocking().unwrap() {
                Listing::Batch(b) => {
                    batches += 1;
                    count += b.len();
                    assert!(b.iter().all(|e| e.dir == (e.name == "sub")));
                }
                Listing::Done => break,
                Listing::Failed(e) => panic!("{e}"),
            }
        }
        assert_eq!(count, 2501);
        assert!(batches >= 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn guesses_types_by_name() {
        assert_eq!(mime_of("a.PNG"), "image/png");
        assert_eq!(mime_of("b.png"), "image/png");
        assert_eq!(mime_of("notas.txt"), "text/plain");
    }
}
