//! The preview, Yazi's third column: an image, the start of a text file
//! (highlighted), a PDF's first page, a video or a song to play, a
//! folder's contents — or, for anything else, what is known of the file.
//! Images and text are read on a thread; a newer selection drops what an
//! older one was still reading.

use std::{cell::Cell, io::Read, path::PathBuf, rc::Rc};

use gtk::{gdk, gdk_pixbuf, gio, glib, prelude::*};
use orbit_app::i18n::tr;
use sourceview5::prelude::*;

use crate::entry::{self, Entry};

/// Read of a text file, at most.
const TEXT_BYTES: usize = 64 * 1024;
/// Entries of a folder listed, at most.
const FOLDER_ITEMS: usize = 300;

pub struct Preview {
    pub widget: gtk::Box,
    stack: gtk::Stack,
    picture: gtk::Picture,
    code: sourceview5::View,
    video: gtk::Video,
    folder: gtk::Label,
    icon: gtk::Image,
    info: gtk::Label,
    generation: Cell<u64>,
}

/// Pixels decoded on a thread.
struct Pixels {
    width: i32,
    height: i32,
    stride: usize,
    alpha: bool,
    data: Vec<u8>,
}

impl Preview {
    pub fn new() -> Rc<Self> {
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .build();
        let buffer = sourceview5::Buffer::new(None);
        let code = sourceview5::View::builder()
            .buffer(&buffer)
            .editable(false)
            .monospace(true)
            .cursor_visible(false)
            .show_line_numbers(true)
            .css_classes(["preview-code"])
            .build();
        let video = gtk::Video::builder().autoplay(false).build();
        let folder = gtk::Label::builder()
            .xalign(0.0)
            .yalign(0.0)
            .selectable(false)
            .css_classes(["preview-folder"])
            .build();
        let icon = gtk::Image::builder().pixel_size(128).build();
        let stack = gtk::Stack::builder().vexpand(true).hexpand(true).build();
        stack.add_named(&picture, Some("image"));
        stack.add_named(
            &gtk::ScrolledWindow::builder().child(&code).build(),
            Some("text"),
        );
        stack.add_named(&video, Some("media"));
        stack.add_named(&gtk::ScrolledWindow::builder().child(&folder).build(), Some("folder"));
        stack.add_named(&icon, Some("icon"));
        let info = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .css_classes(["preview-info"])
            .build();
        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(5)
            .css_classes(["preview"])
            .build();
        widget.append(&stack);
        widget.append(&info);
        if let Some(settings) = gtk::Settings::default() {
            let set = {
                let buffer = buffer.clone();
                move |dark: bool| {
                    let manager = sourceview5::StyleSchemeManager::default();
                    buffer.set_style_scheme(manager.scheme(if dark { "Adwaita-dark" } else { "Adwaita" }).as_ref());
                }
            };
            set(settings.is_gtk_application_prefer_dark_theme());
            settings.connect_gtk_application_prefer_dark_theme_notify(move |s| set(s.is_gtk_application_prefer_dark_theme()));
        }
        Rc::new(Self {
            widget,
            stack,
            picture,
            code,
            video,
            folder,
            icon,
            info,
            generation: Cell::new(0),
        })
    }

    pub fn show(self: &Rc<Self>, entry: Option<&Entry>) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        // Nothing plays once out of sight.
        self.video.set_file(None::<&gio::File>);
        let Some(e) = entry else {
            self.stack.set_visible_child_name("icon");
            self.icon.set_icon_name(None);
            self.info.set_label("");
            return;
        };
        self.info.set_label(&describe(e));
        let path = e.location.path().map(PathBuf::from);
        let mime = e.mime.as_str();
        match path {
            Some(path) if e.dir => self.folder(path, generation),
            Some(path) if mime.starts_with("image/") => self.image(path, e, generation),
            Some(path) if mime == "application/pdf" => self.pdf(&path, e),
            Some(_) if mime.starts_with("video/") || mime.starts_with("audio/") => {
                self.video.set_file(Some(&e.location.file()));
                self.stack.set_visible_child_name("media");
            }
            Some(path) if is_text(mime) => self.text(path, e, generation),
            _ => self.icon(e),
        }
    }

    fn icon(&self, e: &Entry) {
        self.icon.set_from_gicon(&entry::icon(e));
        self.stack.set_visible_child_name("icon");
    }

    fn folder(self: &Rc<Self>, path: PathBuf, generation: u64) {
        let weak = Rc::downgrade(self);
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let mut names: Vec<(bool, String)> = std::fs::read_dir(&path)
                .map(|d| {
                    d.filter_map(Result::ok)
                        .map(|e| (e.path().is_dir(), e.file_name().to_string_lossy().into_owned()))
                        .filter(|(_, n)| !n.starts_with('.'))
                        .take(FOLDER_ITEMS * 4)
                        .collect()
                })
                .unwrap_or_default();
            names.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| orbit_common::natural::natural(&a.1.to_lowercase(), &b.1.to_lowercase())));
            let _ = tx.send_blocking(names);
        });
        glib::spawn_future_local(async move {
            let Ok(names) = rx.recv().await else { return };
            let Some(p) = weak.upgrade().filter(|p| p.generation.get() == generation) else { return };
            let total = names.len();
            let mut lines: Vec<String> = names
                .into_iter()
                .take(FOLDER_ITEMS)
                .map(|(dir, n)| if dir { format!("📁 {n}") } else { format!("    {n}") })
                .collect();
            if total == 0 {
                lines.push(tr("files-preview-empty"));
            } else if total > FOLDER_ITEMS {
                lines.push("…".into());
            }
            p.folder.set_label(&lines.join("\n"));
            p.stack.set_visible_child_name("folder");
        });
    }

    fn image(self: &Rc<Self>, path: PathBuf, e: &Entry, generation: u64) {
        let weak = Rc::downgrade(self);
        let fallback = e.clone();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            // Shrunk while decoded: a 50-megapixel photo costs no more
            // than the preview needs.
            let pixels = gdk_pixbuf::Pixbuf::from_file_at_scale(&path, 1200, 1200, true)
                .ok()
                .and_then(|p| p.apply_embedded_orientation())
                .map(|p| Pixels {
                    width: p.width(),
                    height: p.height(),
                    stride: p.rowstride() as usize,
                    alpha: p.has_alpha(),
                    data: p.read_pixel_bytes().to_vec(),
                });
            let _ = tx.send_blocking(pixels);
        });
        glib::spawn_future_local(async move {
            let Ok(pixels) = rx.recv().await else { return };
            let Some(p) = weak.upgrade().filter(|p| p.generation.get() == generation) else { return };
            match pixels {
                Some(px) => {
                    let texture = gdk::MemoryTexture::new(
                        px.width,
                        px.height,
                        if px.alpha { gdk::MemoryFormat::R8g8b8a8 } else { gdk::MemoryFormat::R8g8b8 },
                        &glib::Bytes::from_owned(px.data),
                        px.stride,
                    );
                    p.picture.set_paintable(Some(&texture));
                    p.stack.set_visible_child_name("image");
                }
                None => p.icon(&fallback),
            }
        });
    }

    fn pdf(&self, path: &std::path::Path, e: &Entry) {
        let uri = gio::File::for_path(path).uri();
        let page = poppler::Document::from_file(&uri, None).ok().and_then(|d| d.page(0));
        let Some(page) = page else {
            self.icon(e);
            return;
        };
        let (w, h) = page.size();
        let k = 600.0 / w.max(1.0);
        let Ok(surface) = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, (w * k) as i32, (h * k) as i32) else {
            self.icon(e);
            return;
        };
        if let Ok(cr) = gtk::cairo::Context::new(&surface) {
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.paint().ok();
            cr.scale(k, k);
            page.render(&cr);
        }
        surface.flush();
        let (width, height, stride) = (surface.width(), surface.height(), surface.stride() as usize);
        let Ok(data) = surface.take_data() else {
            self.icon(e);
            return;
        };
        let texture = gdk::MemoryTexture::new(
            width,
            height,
            gdk::MemoryFormat::B8g8r8a8Premultiplied,
            &glib::Bytes::from(&data[..]),
            stride,
        );
        self.picture.set_paintable(Some(&texture));
        self.stack.set_visible_child_name("image");
    }

    fn text(self: &Rc<Self>, path: PathBuf, e: &Entry, generation: u64) {
        let weak = Rc::downgrade(self);
        let name = e.name.clone();
        let mime = e.mime.clone();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let read = std::fs::File::open(&path)
                .and_then(|f| f.take(TEXT_BYTES as u64).read_to_end(&mut bytes));
            let text = read.ok().map(|_| match std::str::from_utf8(&bytes) {
                Ok(t) => t.to_owned(),
                // Cut mid-character, or Latin-1.
                Err(e) if e.error_len().is_none() => String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned(),
                Err(_) => bytes.iter().map(|&b| b as char).collect(),
            });
            let _ = tx.send_blocking(text);
        });
        glib::spawn_future_local(async move {
            let Ok(Some(text)) = rx.recv().await else { return };
            let Some(p) = weak.upgrade().filter(|p| p.generation.get() == generation) else { return };
            let buffer = p.code.buffer().downcast::<sourceview5::Buffer>().expect("a source buffer");
            let language = sourceview5::LanguageManager::default().guess_language(Some(&name), Some(&mime));
            buffer.set_language(language.as_ref());
            buffer.set_text(&text);
            p.stack.set_visible_child_name("text");
        });
    }
}

fn is_text(mime: &str) -> bool {
    mime.starts_with("text/")
        || matches!(
            mime,
            "application/json"
                | "application/xml"
                | "application/javascript"
                | "application/x-shellscript"
                | "application/x-yaml"
                | "application/toml"
                | "application/x-desktop"
                | "application/sql"
        )
}

/// "photo.jpg — 2,4 MB — JPEG image — 20/10/2026 09:31"
fn describe(e: &Entry) -> String {
    let kind = gio::content_type_get_description(&e.mime).to_string();
    let mut parts = vec![e.name.clone()];
    if !e.dir {
        parts.push(entry::size_text(e.size));
    }
    parts.push(kind);
    parts.push(entry::date_text(e.modified));
    if e.link {
        parts.push(tr("files-preview-link"));
    }
    parts.join(" — ")
}
