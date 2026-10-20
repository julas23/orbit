//! The strip of thumbnails along the bottom: every image in the folder,
//! the current one selected; clicking one opens it.
//!
//! Thumbnails come from the shared cache of the freedesktop.org
//! specification (`~/.cache/thumbnails/normal`), which file managers read
//! and write too; those missing are made on two threads of their own —
//! decoded with glycin, shrunk, and saved to that cache.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    time::UNIX_EPOCH,
};

use gtk::{gdk, gdk_pixbuf, gio, glib, prelude::*};

use crate::{decode, pixels};

/// The "normal" size of the specification.
const SIZE: u32 = 128;
const WORKERS: usize = 2;

type Listener = Box<dyn Fn(usize)>;

pub struct Strip {
    pub widget: gtk::ScrolledWindow,
    view: gtk::ListView,
    store: gio::ListStore,
    selection: gtk::SingleSelection,
    /// Made or loaded so far.
    cache: RefCell<HashMap<PathBuf, gdk::Texture>>,
    /// Asked of the workers and not back yet.
    asked: RefCell<HashSet<PathBuf>>,
    /// The picture showing each path now.
    bound: RefCell<HashMap<PathBuf, glib::WeakRef<gtk::Picture>>>,
    requests: async_channel::Sender<PathBuf>,
    /// Set while the selection is moved from here, not by a click.
    syncing: Cell<bool>,
    on_pick: RefCell<Option<Listener>>,
}

impl Strip {
    pub fn new() -> Rc<Self> {
        let store = gio::ListStore::new::<gtk::StringObject>();
        let selection = gtk::SingleSelection::builder()
            .model(&store)
            .autoselect(false)
            .can_unselect(true)
            .build();
        let factory = gtk::SignalListItemFactory::new();
        let view = gtk::ListView::builder()
            .model(&selection)
            .factory(&factory)
            .orientation(gtk::Orientation::Horizontal)
            .single_click_activate(false)
            .css_classes(["strip"])
            .build();
        let widget = gtk::ScrolledWindow::builder()
            .child(&view)
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .css_classes(["strip-scroll"])
            .build();

        let (requests, queue) = async_channel::unbounded::<PathBuf>();
        let (done, results) = async_channel::unbounded();
        for _ in 0..WORKERS {
            let queue = queue.clone();
            let done = done.clone();
            let spawned = std::thread::Builder::new()
                .name("orbit-view-thumbs".into())
                .spawn(move || {
                    while let Ok(path) = queue.recv_blocking() {
                        let thumb = thumbnail(&path);
                        if done.send_blocking((path, thumb)).is_err() {
                            break;
                        }
                    }
                });
            if let Err(err) = spawned {
                log::warn!("starting a thumbnailer: {err}");
            }
        }

        let strip = Rc::new(Self {
            widget,
            view,
            store,
            selection,
            cache: RefCell::default(),
            asked: RefCell::default(),
            bound: RefCell::default(),
            requests,
            syncing: Cell::new(false),
            on_pick: RefCell::default(),
        });

        let weak = Rc::downgrade(&strip);
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
            let picture = gtk::Picture::builder()
                .can_shrink(true)
                .content_fit(gtk::ContentFit::Contain)
                .css_classes(["thumb"])
                .build();
            item.set_child(Some(&picture));
        });
        {
            let weak = weak.clone();
            factory.connect_bind(move |_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
                let (Some(strip), Some(picture), Some(path)) = (
                    weak.upgrade(),
                    item.child().and_downcast::<gtk::Picture>(),
                    item.item().and_downcast::<gtk::StringObject>(),
                ) else {
                    return;
                };
                let path = PathBuf::from(path.string().as_str());
                picture.set_tooltip_text(path.file_name().and_then(|n| n.to_str()));
                strip.show(&picture, &path);
            });
        }
        {
            let weak = weak.clone();
            factory.connect_unbind(move |_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
                if let (Some(strip), Some(path)) = (
                    weak.upgrade(),
                    item.item().and_downcast::<gtk::StringObject>(),
                ) {
                    strip
                        .bound
                        .borrow_mut()
                        .remove(Path::new(path.string().as_str()));
                }
            });
        }
        {
            let weak = weak.clone();
            strip.selection.connect_selected_notify(move |selection| {
                let Some(strip) = weak.upgrade() else { return };
                let index = selection.selected();
                if strip.syncing.get() || index == gtk::INVALID_LIST_POSITION {
                    return;
                }
                if let Some(f) = strip.on_pick.borrow().as_ref() {
                    f(index as usize);
                }
            });
        }
        glib::spawn_future_local(async move {
            while let Ok((path, thumb)) = results.recv().await {
                let Some(strip) = weak.upgrade() else { break };
                strip.asked.borrow_mut().remove(&path);
                let Some(thumb) = thumb else { continue };
                let texture = thumb.texture();
                if let Some(picture) = strip.bound.borrow().get(&path).and_then(|w| w.upgrade()) {
                    picture.set_paintable(Some(&texture));
                }
                strip.cache.borrow_mut().insert(path, texture);
            }
        });
        strip
    }

    /// Called with the index of an image picked in the strip.
    pub fn connect_pick(&self, f: impl Fn(usize) + 'static) {
        *self.on_pick.borrow_mut() = Some(Box::new(f));
    }

    /// Shows `images`, `current` selected.
    pub fn set(&self, images: &[PathBuf], current: Option<usize>) {
        self.syncing.set(true);
        let items: Vec<gtk::StringObject> = images
            .iter()
            .map(|p| gtk::StringObject::new(&p.to_string_lossy()))
            .collect();
        self.store.splice(0, self.store.n_items(), &items);
        // Thumbnails of files no longer listed go.
        self.cache
            .borrow_mut()
            .retain(|p, _| images.iter().any(|i| i == p));
        self.syncing.set(false);
        self.select(current);
    }

    /// Moves the selection to `index`, scrolled into view.
    pub fn select(&self, index: Option<usize>) {
        self.syncing.set(true);
        match index {
            Some(i) if (i as u32) < self.store.n_items() => {
                self.selection.set_selected(i as u32);
                self.view
                    .scroll_to(i as u32, gtk::ListScrollFlags::NONE, None);
            }
            _ => self.selection.set_selected(gtk::INVALID_LIST_POSITION),
        }
        self.syncing.set(false);
    }

    /// Makes `path`'s thumbnail again (the file changed).
    pub fn invalidate(&self, path: &Path) {
        self.cache.borrow_mut().remove(path);
        let picture = self.bound.borrow().get(path).and_then(|w| w.upgrade());
        if let Some(picture) = picture {
            self.show(&picture, path);
        }
    }

    fn show(&self, picture: &gtk::Picture, path: &Path) {
        self.bound
            .borrow_mut()
            .insert(path.to_path_buf(), picture.downgrade());
        if let Some(texture) = self.cache.borrow().get(path) {
            picture.set_paintable(Some(texture));
            return;
        }
        picture.set_paintable(None::<&gdk::Paintable>);
        if self.asked.borrow_mut().insert(path.to_path_buf()) {
            let _ = self.requests.send_blocking(path.to_path_buf());
        }
    }
}

/// A thumbnail's pixels, to cross from the workers to the GTK thread
/// (pixbufs and textures do not).
struct Thumb {
    width: i32,
    height: i32,
    stride: usize,
    alpha: bool,
    data: Vec<u8>,
}

impl Thumb {
    fn of(pixbuf: &gdk_pixbuf::Pixbuf) -> Option<Self> {
        if pixbuf.bits_per_sample() != 8 || pixbuf.n_channels() < 3 {
            return None;
        }
        Some(Self {
            width: pixbuf.width(),
            height: pixbuf.height(),
            stride: pixbuf.rowstride() as usize,
            alpha: pixbuf.has_alpha(),
            data: pixbuf.read_pixel_bytes().to_vec(),
        })
    }

    fn texture(self) -> gdk::Texture {
        let format = if self.alpha {
            gdk::MemoryFormat::R8g8b8a8
        } else {
            gdk::MemoryFormat::R8g8b8
        };
        gdk::MemoryTexture::new(
            self.width,
            self.height,
            format,
            &glib::Bytes::from_owned(self.data),
            self.stride,
        )
        .upcast()
    }
}

/// `path`'s thumbnail: from the shared cache when it is current, else made
/// and saved there.
fn thumbnail(path: &Path) -> Option<Thumb> {
    let uri = gio::File::for_path(path).uri();
    let mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs()
        .to_string();
    let dir = glib::user_cache_dir().join("thumbnails").join("normal");
    let md5 = glib::compute_checksum_for_string(glib::ChecksumType::Md5, uri.as_str())?;
    let cached = dir.join(format!("{md5}.png"));
    if let Ok(pixbuf) = gdk_pixbuf::Pixbuf::from_file(&cached)
        && pixbuf.option("tEXt::Thumb::MTime").as_deref() == Some(mtime.as_str())
    {
        return Thumb::of(&pixbuf);
    }

    let rx = decode::load(path.to_path_buf());
    let frame = loop {
        match rx.recv_blocking().ok()? {
            decode::Event::Frame(frame) => break frame,
            decode::Event::Opened(_) => continue,
            _ => return None,
        }
    };
    drop(rx);
    let (w, h, rgba) = pixels::thumbnail(&frame, SIZE)?;
    let pixbuf = gdk_pixbuf::Pixbuf::from_bytes(
        &glib::Bytes::from_owned(rgba),
        gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        w as i32,
        h as i32,
        (w * 4) as i32,
    );
    // Not for the cache's own files.
    if !path.starts_with(glib::user_cache_dir().join("thumbnails")) {
        save(&pixbuf, &dir, &cached, &uri, &mtime);
    }
    Thumb::of(&pixbuf)
}

/// Writes the thumbnail as the specification asks: private, with the
/// source's URI and modification time, renamed into place whole.
fn save(pixbuf: &gdk_pixbuf::Pixbuf, dir: &Path, target: &Path, uri: &str, mtime: &str) {
    use std::os::unix::fs::PermissionsExt;
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    let temporary = target.with_extension(format!("orbit-view-{}.png", std::process::id()));
    let saved = pixbuf.savev(
        &temporary,
        "png",
        &[("tEXt::Thumb::URI", uri), ("tEXt::Thumb::MTime", mtime)],
    );
    match saved {
        Ok(()) => {
            let _ = std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600));
            if let Err(err) = std::fs::rename(&temporary, target) {
                log::debug!("thumbnail {}: {err}", target.display());
                let _ = std::fs::remove_file(&temporary);
            }
        }
        Err(err) => log::debug!("thumbnail {}: {err}", target.display()),
    }
}
