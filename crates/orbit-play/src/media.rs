//! What a file is — video, audio, a Flash movie — and the other media in
//! its folder, in the order a person would sort them.

use std::path::{Path, PathBuf};

use gtk::gio;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Video or audio: GStreamer plays it.
    Media,
    /// A Flash movie: Ruffle plays it.
    Flash,
}

/// Subtitle files found next to a video of the same name.
const SUBTITLES: &[&str] = &["srt", "ass", "ssa", "vtt", "sub"];

pub fn kind_of(path: &Path) -> Option<Kind> {
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if extension == "swf" {
        return Some(Kind::Flash);
    }
    let (kind, _) = gio::content_type_guess(Some(path), None);
    let mime = gio::content_type_get_mime_type(&kind)
        .map(|m| m.to_string())
        .unwrap_or_default();
    if mime == "application/x-shockwave-flash" || mime == "application/vnd.adobe.flash.movie" {
        Some(Kind::Flash)
    } else if mime.starts_with("video/")
        || mime.starts_with("audio/")
        // Containers some systems name as applications.
        || matches!(
            mime.as_str(),
            "application/ogg" | "application/vnd.rn-realmedia" | "application/x-matroska" | "application/mxf"
        )
    {
        Some(Kind::Media)
    } else {
        None
    }
}

/// Whether `path` is audio by its type — an MP3's cover art counts as a
/// video stream to GStreamer, yet the file is a song.
pub fn is_audio(path: &Path) -> bool {
    let (kind, _) = gio::content_type_guess(Some(path), None);
    gio::content_type_get_mime_type(&kind).is_some_and(|m| m.starts_with("audio/"))
}

/// The media in `dir`, hidden files left out; `keep` stays in regardless.
pub fn folder(dir: &Path, keep: &Path) -> Vec<PathBuf> {
    let mut list: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p == keep
                        || (!p
                            .file_name()
                            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                            && kind_of(p).is_some()
                            && p.is_file())
                })
                .collect()
        })
        .unwrap_or_default();
    if !list.iter().any(|p| p == keep) {
        list.push(keep.to_path_buf());
    }
    list.sort_by(|a, b| orbit_common::natural::by_name(a, b));
    list
}

/// A subtitle file beside `video` with its name: `filme.srt`, or
/// `filme.pt-BR.srt` and the like.
pub fn subtitle_for(video: &Path) -> Option<PathBuf> {
    let stem = video.file_stem()?.to_string_lossy().into_owned();
    let dir = video.parent()?;
    for ext in SUBTITLES {
        let exact = dir.join(format!("{stem}.{ext}"));
        if exact.is_file() {
            return Some(exact);
        }
    }
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            name.starts_with(&format!("{stem}.")) && SUBTITLES.contains(&ext.as_str())
        })
        .collect();
    found.sort();
    found.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(kind_of(Path::new("old.swf")), Some(Kind::Flash));
        assert_eq!(kind_of(Path::new("movie.mkv")), Some(Kind::Media));
        assert_eq!(kind_of(Path::new("song.flac")), Some(Kind::Media));
        assert_eq!(kind_of(Path::new("clip.flv")), Some(Kind::Media));
        assert_eq!(kind_of(Path::new("notes.txt")), None);
    }

    #[test]
    fn subtitles_beside_the_video() {
        let dir = std::env::temp_dir().join(format!("orbit-play-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("filme.mkv");
        std::fs::write(&video, b"").unwrap();
        assert_eq!(subtitle_for(&video), None);
        std::fs::write(dir.join("filme.pt-BR.srt"), b"").unwrap();
        assert_eq!(subtitle_for(&video), Some(dir.join("filme.pt-BR.srt")));
        std::fs::write(dir.join("filme.srt"), b"").unwrap();
        assert_eq!(subtitle_for(&video), Some(dir.join("filme.srt")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
