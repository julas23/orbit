//! `orbit-settings housekeeping`: the trash and the temporary files
//! emptied of what is older than GNOME's privacy settings say
//! (`org.gnome.desktop.privacy`: `remove-old-trash-files`,
//! `remove-old-temp-files`, `old-files-age` in days), run by the user's
//! `orbit-housekeeping.timer`, which the Privacy page turns on and off.
//!
//! The trash goes by each item's deletion date (its `.trashinfo`), not
//! the file's own dates: a file last changed a year ago and trashed today
//! stays its days. The temporary files are this user's in /tmp and
//! /var/tmp, by their last change.

use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use gtk::{gio, glib, prelude::*};

const SCHEMA: &str = "org.gnome.desktop.privacy";
pub const TIMER: &str = "orbit-housekeeping.timer";

pub fn run() -> glib::ExitCode {
    let settings = gio::Settings::new(SCHEMA);
    let days = settings.uint("old-files-age");
    let age = Duration::from_secs(u64::from(days) * 86_400);
    if settings.boolean("remove-old-trash-files") {
        let trash = glib::user_data_dir().join("Trash");
        let gone = old_trash(&trash, age);
        log::info!("trash: {gone} items older than {days} days removed");
    }
    if settings.boolean("remove-old-temp-files") {
        // SAFETY: getuid has no preconditions and cannot fail.
        let uid = unsafe { libc::getuid() };
        let gone: usize = ["/tmp", "/var/tmp"].iter().map(|d| old_temp(Path::new(d), uid, age)).sum();
        log::info!("temporary files: {gone} older than {days} days removed");
    }
    glib::ExitCode::SUCCESS
}

/// When `info` (a `.trashinfo`) says its item was deleted.
fn deleted(info: &str) -> Option<SystemTime> {
    let date = info.lines().find_map(|l| l.strip_prefix("DeletionDate="))?.trim();
    let when = glib::DateTime::from_iso8601(date, Some(&glib::TimeZone::local())).ok()?;
    let seconds = u64::try_from(when.to_unix()).ok()?;
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
}

/// Removes the items of the trash at `trash` deleted longer than `age`
/// ago; how many.
fn old_trash(trash: &Path, age: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(trash.join("info")) else { return 0 };
    let now = SystemTime::now();
    let mut gone = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "trashinfo") {
            continue;
        }
        let Some(when) = std::fs::read_to_string(&path).ok().as_deref().and_then(deleted) else { continue };
        if now.duration_since(when).unwrap_or_default() < age {
            continue;
        }
        let item: PathBuf = trash.join("files").join(path.file_stem().unwrap_or_default());
        let removed = match std::fs::symlink_metadata(&item) {
            Ok(m) if m.is_dir() => std::fs::remove_dir_all(&item),
            Ok(_) => std::fs::remove_file(&item),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        };
        match removed.and_then(|()| std::fs::remove_file(&path)) {
            Ok(()) => gone += 1,
            Err(e) => log::warn!("{}: {e}", item.display()),
        }
    }
    gone
}

/// Removes the entries of `dir` that are `uid`'s and unchanged for longer
/// than `age`; how many.
fn old_temp(dir: &Path, uid: u32, age: Duration) -> usize {
    use std::os::unix::fs::MetadataExt;
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let now = SystemTime::now();
    let old = |t: std::io::Result<SystemTime>| t.ok().is_some_and(|t| now.duration_since(t).unwrap_or_default() >= age);
    let mut gone = 0;
    for entry in entries.flatten() {
        let Ok(meta) = entry.path().symlink_metadata() else { continue };
        let changed = SystemTime::UNIX_EPOCH + Duration::from_secs(u64::try_from(meta.ctime()).unwrap_or(0));
        if meta.uid() != uid || !old(meta.modified()) || !old(Ok(changed)) {
            continue;
        }
        let removed = if meta.is_dir() {
            std::fs::remove_dir_all(entry.path())
        } else {
            std::fs::remove_file(entry.path())
        };
        if removed.is_ok() {
            gone += 1;
        }
    }
    gone
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trash_goes_by_deletion_date() {
        let trash = std::env::temp_dir().join(format!("orbit-trash-test-{}", std::process::id()));
        std::fs::create_dir_all(trash.join("info")).unwrap();
        std::fs::create_dir_all(trash.join("files/old-dir")).unwrap();
        std::fs::write(trash.join("files/old-dir/inside"), "x").unwrap();
        std::fs::write(trash.join("files/new"), "x").unwrap();
        std::fs::write(trash.join("info/old-dir.trashinfo"), "[Trash Info]\nPath=/x/old-dir\nDeletionDate=2020-01-02T03:04:05\n").unwrap();
        let today = glib::DateTime::now_local().unwrap().format("%Y-%m-%dT%H:%M:%S").unwrap();
        std::fs::write(trash.join("info/new.trashinfo"), format!("[Trash Info]\nPath=/x/new\nDeletionDate={today}\n")).unwrap();
        assert_eq!(old_trash(&trash, Duration::from_secs(30 * 86_400)), 1);
        assert!(!trash.join("files/old-dir").exists());
        assert!(!trash.join("info/old-dir.trashinfo").exists());
        assert!(trash.join("files/new").exists());
        std::fs::remove_dir_all(&trash).unwrap();
    }
}
