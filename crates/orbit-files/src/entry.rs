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
    /// The Unix mode, owner and group (0 where unknown).
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Location {
    Path(PathBuf),
    Uri(String),
    /// "Computer": the disks and volumes, with the space on each.
    Computer,
}

/// What "Computer" is typed as.
pub const COMPUTER_URI: &str = "computer:///";

impl Location {
    pub fn file(&self) -> gio::File {
        match self {
            Location::Path(p) => gio::File::for_path(p),
            Location::Uri(u) => gio::File::for_uri(u),
            Location::Computer => gio::File::for_uri(COMPUTER_URI),
        }
    }

    pub fn path(&self) -> Option<&Path> {
        match self {
            Location::Path(p) => Some(p),
            _ => None,
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
            Location::Computer => COMPUTER_URI.into(),
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
            Location::Computer => orbit_app::i18n::tr("files-computer"),
        }
    }

    pub fn parent(&self) -> Option<Location> {
        match self {
            Location::Path(p) => p.parent().map(|p| Location::Path(p.to_path_buf())),
            Location::Uri(_) => self.file().parent().map(|f| Location::of(&f)),
            Location::Computer => None,
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
                Location::Computer => Ok(()),
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
        let Some(e) = entry_for(&entry.path()) else { continue };
        batch.push(e);
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

/// The entry for one local path — what listing its folder would give.
/// One `lstat` (two for a link); call it off the main thread.
pub fn entry_for(path: &Path) -> Option<Entry> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let meta = std::fs::symlink_metadata(path).ok()?;
    let link = meta.file_type().is_symlink();
    // A link counts as what it points to.
    let target = if link { std::fs::metadata(path).ok() } else { None };
    use std::os::unix::fs::MetadataExt;
    let own = target.as_ref().unwrap_or(&meta);
    let (mode, uid, gid) = (own.mode(), own.uid(), own.gid());
    let dir = target.as_ref().map_or(meta.is_dir(), |t| t.is_dir());
    let size = target.as_ref().map_or(meta.len(), |t| t.len());
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);
    Some(Entry {
        hidden: name.starts_with('.'),
        mime: if dir { "inode/directory".into() } else { mime_of(&name) },
        name,
        location: Location::Path(path.to_path_buf()),
        dir,
        link,
        size,
        modified,
        mode,
        uid,
        gid,
    })
}

fn list_gio(dir: &gio::File, tx: &async_channel::Sender<Listing>) -> Result<(), String> {
    let children = dir
        .enumerate_children(
            "standard::name,standard::display-name,standard::type,standard::size,standard::content-type,standard::is-hidden,standard::is-symlink,time::modified,unix::mode,unix::uid,unix::gid",
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
            mode: info.attribute_uint32("unix::mode"),
            uid: info.attribute_uint32("unix::uid"),
            gid: info.attribute_uint32("unix::gid"),
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

// ------------------------------------------------------------ searching

/// Searches under `dir` for names holding `text` (any case) or, with
/// `contents`, files whose text holds it, on a thread; what is found
/// arrives as a listing does. Hidden folders are left out unless
/// `hidden`; links to folders are not followed. Setting `stop` ends it.
pub fn search(
    dir: PathBuf,
    text: String,
    contents: bool,
    hidden: bool,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> async_channel::Receiver<Listing> {
    let (tx, rx) = async_channel::unbounded();
    std::thread::Builder::new()
        .name("orbit-files-search".into())
        .spawn(move || {
            walk(&dir, &text.to_lowercase(), contents, hidden, &stop, &tx);
            let _ = tx.send_blocking(Listing::Done);
        })
        .ok();
    rx
}

/// At most this many found, so a search for "a" from / ends.
const MOST_FOUND: usize = 10_000;
/// Files larger than this are not read for their text.
const MOST_READ: u64 = 16 << 20;

fn walk(
    root: &Path,
    wanted: &str,
    contents: bool,
    hidden: bool,
    stop: &std::sync::atomic::AtomicBool,
    tx: &async_channel::Sender<Listing>,
) {
    use std::sync::atomic::Ordering;
    // Not the kernel's own trees.
    let skip = [Path::new("/proc"), Path::new("/sys"), Path::new("/dev"), Path::new("/run")];
    let mut dirs = vec![root.to_path_buf()];
    let mut batch = Vec::new();
    let mut found = 0;
    let mut sent = std::time::Instant::now();
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.filter_map(Result::ok) {
            if stop.load(Ordering::Relaxed) || tx.is_closed() {
                return;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if !hidden && name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() && !skip.contains(&path.as_path()) {
                dirs.push(path.clone());
            }
            let hit = if contents {
                kind.is_file() && holds(&path, wanted)
            } else {
                name.to_lowercase().contains(wanted)
            };
            if hit && let Some(e) = entry_for(&path) {
                batch.push(e);
                found += 1;
            }
            // What is found shows as it is found.
            if !batch.is_empty() && (batch.len() >= BATCH || sent.elapsed().as_millis() > 200) {
                if tx.send_blocking(Listing::Batch(std::mem::take(&mut batch))).is_err() {
                    return;
                }
                sent = std::time::Instant::now();
            }
            if found >= MOST_FOUND {
                let _ = tx.send_blocking(Listing::Batch(batch));
                return;
            }
        }
    }
    if !batch.is_empty() {
        let _ = tx.send_blocking(Listing::Batch(batch));
    }
}

/// Whether the text of `path` holds `wanted` (lowercase), any case. Only
/// text files, by their name's type, and not huge ones.
fn holds(path: &Path, wanted: &str) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if !crate::actions::is_text(&mime_of(&name), &name) {
        return false;
    }
    if std::fs::metadata(path).map_or(true, |m| m.len() > MOST_READ) {
        return false;
    }
    std::fs::read(path).is_ok_and(|bytes| String::from_utf8_lossy(&bytes).to_lowercase().contains(wanted))
}

// ------------------------------------------------- columns' text

/// `drwxr-xr-x`, as `ls -l` writes it.
pub fn mode_text(e: &Entry) -> String {
    if e.mode == 0 {
        return String::new();
    }
    let kind = if e.link {
        'l'
    } else if e.dir {
        'd'
    } else {
        '-'
    };
    let mut out = String::from(kind);
    for shift in [6, 3, 0] {
        let bits = (e.mode >> shift) & 7;
        out.push(if bits & 4 != 0 { 'r' } else { '-' });
        out.push(if bits & 2 != 0 { 'w' } else { '-' });
        out.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    out
}

/// The user's name for `uid` (from /etc/passwd), else the number.
pub fn user_name(uid: u32) -> String {
    id_name("/etc/passwd", uid)
}

/// The group's name for `gid` (from /etc/group), else the number.
pub fn group_name(gid: u32) -> String {
    id_name("/etc/group", gid)
}

fn id_name(file: &'static str, id: u32) -> String {
    thread_local! {
        static NAMES: std::cell::RefCell<HashMap<&'static str, HashMap<u32, String>>> = Default::default();
    }
    NAMES.with(|names| {
        let mut names = names.borrow_mut();
        let table = names.entry(file).or_insert_with(|| {
            std::fs::read_to_string(file)
                .unwrap_or_default()
                .lines()
                .filter_map(|l| {
                    let mut f = l.split(':');
                    let name = f.next()?;
                    let id = f.nth(1)?.parse().ok()?;
                    Some((id, name.to_owned()))
                })
                .collect()
        });
        table.get(&id).cloned().unwrap_or_else(|| id.to_string())
    })
}

/// Where an entry is, for search results: its folder, from home as `~`.
pub fn folder_text(e: &Entry) -> String {
    let Some(parent) = e.location.path().and_then(Path::parent) else {
        return String::new();
    };
    let home = glib::home_dir();
    match parent.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => parent.display().to_string(),
    }
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
    fn searches_names_and_contents() {
        let dir = std::env::temp_dir().join(format!("orbit-files-search-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join("a/b/Relatorio.txt"), b"nada aqui").unwrap();
        std::fs::write(dir.join("a/notas.txt"), b"o RELATORIO final").unwrap();
        std::fs::write(dir.join(".hidden/relatorio.md"), b"x").unwrap();
        let found = |contents, hidden| {
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let rx = search(dir.clone(), "relatorio".into(), contents, hidden, stop);
            let mut names = Vec::new();
            while let Ok(Listing::Batch(b)) = rx.recv_blocking() {
                names.extend(b.into_iter().map(|e| e.name));
            }
            names.sort();
            names
        };
        assert_eq!(found(false, false), ["Relatorio.txt"]);
        assert_eq!(found(false, true), ["Relatorio.txt", "relatorio.md"]);
        assert_eq!(found(true, false), ["notas.txt"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn writes_modes_as_ls_does() {
        let e = Entry {
            name: "x".into(),
            location: Location::Path("/x".into()),
            dir: true,
            link: false,
            hidden: false,
            size: 0,
            modified: 0,
            mime: String::new(),
            mode: 0o40755,
            uid: 0,
            gid: 0,
        };
        assert_eq!(mode_text(&e), "drwxr-xr-x");
        assert_eq!(user_name(0), "root");
    }

    #[test]
    fn guesses_types_by_name() {
        assert_eq!(mime_of("a.PNG"), "image/png");
        assert_eq!(mime_of("b.png"), "image/png");
        assert_eq!(mime_of("notas.txt"), "text/plain");
    }
}
