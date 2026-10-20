//! The images in a folder, in the order a person would sort them (see
//! `orbit_common::natural`).

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use gtk::gio;

/// The images in `dir` that a loader can open (by their name's type),
/// hidden files left out; `keep` stays in even if its type is unknown.
pub fn images(dir: &Path, supported: &HashSet<String>, keep: Option<&Path>) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return keep.map(Path::to_path_buf).into_iter().collect();
    };
    let mut list: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            Some(p.as_path()) == keep
                || (!p
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                    && is_image(p, supported)
                    && p.is_file())
        })
        .collect();
    if let Some(keep) = keep
        && !list.iter().any(|p| p == keep)
    {
        list.push(keep.to_path_buf());
    }
    list.sort_by(|a, b| orbit_common::natural::by_name(a, b));
    list
}

/// Whether `path`'s name says it is a type a loader opens.
pub fn is_image(path: &Path, supported: &HashSet<String>) -> bool {
    let (kind, _) = gio::content_type_guess(Some(path), None);
    gio::content_type_get_mime_type(&kind).is_some_and(|mime| supported.contains(mime.as_str()))
}
