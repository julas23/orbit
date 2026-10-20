//! Cut, copy and paste of files, in the formats other file managers use:
//! GNOME's `x-special/gnome-copied-files` (which carries "cut" or "copy"),
//! `text/uri-list`, and GDK's file list — so files go between this and
//! Nautilus, Thunar, Nemo, PCManFM, or a browser's upload.

use std::path::PathBuf;

use gtk::{gdk, gio, glib, prelude::*};

const GNOME: &str = "x-special/gnome-copied-files";

pub fn put(widget: &impl IsA<gtk::Widget>, paths: &[PathBuf], cut: bool) {
    let files: Vec<gio::File> = paths.iter().map(gio::File::for_path).collect();
    let uris: Vec<String> = files.iter().map(|f| f.uri().to_string()).collect();
    let gnome = format!("{}\n{}", if cut { "cut" } else { "copy" }, uris.join("\n"));
    let providers = [
        gdk::ContentProvider::for_bytes(GNOME, &glib::Bytes::from_owned(gnome.into_bytes())),
        gdk::ContentProvider::for_bytes("text/uri-list", &glib::Bytes::from_owned(format!("{}\r\n", uris.join("\r\n")).into_bytes())),
        gdk::ContentProvider::for_value(&gdk::FileList::from_array(&files).to_value()),
        gdk::ContentProvider::for_bytes(
            "text/plain;charset=utf-8",
            &glib::Bytes::from_owned(paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n").into_bytes()),
        ),
    ];
    let union = gdk::ContentProvider::new_union(&providers);
    if let Err(err) = widget.clipboard().set_content(Some(&union)) {
        log::warn!("clipboard: {err}");
    }
}

/// What the clipboard holds: files, and whether they were cut.
pub async fn take(widget: &impl IsA<gtk::Widget>) -> Option<(Vec<PathBuf>, bool)> {
    let clipboard = widget.clipboard();
    let formats = clipboard.formats();
    if formats.contain_mime_type(GNOME)
        && let Ok((stream, _)) = clipboard.read_future(&[GNOME], glib::Priority::DEFAULT).await
    {
        let text = read_all(&stream).await?;
        let mut lines = text.lines();
        let cut = lines.next().is_some_and(|l| l.trim() == "cut");
        let paths = lines.filter_map(|u| gio::File::for_uri(u.trim()).path()).collect::<Vec<_>>();
        return (!paths.is_empty()).then_some((paths, cut));
    }
    if formats.contains_type(gdk::FileList::static_type())
        && let Ok(value) = clipboard.read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT).await
        && let Ok(list) = value.get::<gdk::FileList>()
    {
        let paths: Vec<PathBuf> = list.files().iter().filter_map(|f| f.path()).collect();
        return (!paths.is_empty()).then_some((paths, false));
    }
    None
}

async fn read_all(stream: &gio::InputStream) -> Option<String> {
    let mut bytes = Vec::new();
    loop {
        let chunk = stream.read_bytes_future(64 * 1024, glib::Priority::DEFAULT).await.ok()?;
        if chunk.is_empty() {
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).ok()
}
