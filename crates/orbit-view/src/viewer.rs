//! The window: the image over all of it, the controls over the image —
//! shown while the pointer moves, gone a moment after it stops — and the
//! information panel along the right edge.
//!
//! Opening an image lists its folder, so the arrow keys, the buttons and
//! the strip of thumbnails go through the folder; the list follows files
//! added, removed or renamed there.

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use gtk::{gdk, gio, glib, prelude::*};
use orbit_app::i18n::{text, tr, tr_args};
use serde::Deserialize;

use crate::{
    canvas::{self, Canvas},
    decode::{self, Event, Failure, Info},
    edit, folder,
    info::Panel,
    thumbs::Strip,
};

/// How long the controls stay after the pointer stops.
const CONTROLS_FOR: Duration = Duration::from_secs(2);
const TOAST_FOR: Duration = Duration::from_secs(3);
/// Frames asking for less are shown this long, as browsers do.
const MIN_DELAY: Duration = Duration::from_millis(20);
const SLOW_DELAY: Duration = Duration::from_millis(100);

/// `~/.config/orbit/view.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Seconds each image stays in the slideshow.
    pub slideshow_seconds: u32,
    /// The strip of thumbnails under the image.
    pub thumbnails: bool,
    /// The information panel open from the start.
    pub info: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            slideshow_seconds: 5,
            thumbnails: true,
            info: false,
        }
    }
}

pub struct Viewer {
    window: gtk::ApplicationWindow,
    config: Config,
    supported: HashSet<String>,
    editable: HashSet<String>,
    canvas: Rc<Canvas>,
    strip: Rc<Strip>,
    panel: Panel,
    /// The image, or the message when there is none to show.
    stack: gtk::Stack,
    message: gtk::Label,
    top: gtk::Revealer,
    bottom: gtk::Revealer,
    toast_revealer: gtk::Revealer,
    toast_label: gtk::Label,
    name: gtk::Label,
    zoom: gtk::Label,
    position: gtk::Label,
    buttons: Buttons,

    images: RefCell<Vec<PathBuf>>,
    index: Cell<Option<usize>>,
    dir: RefCell<Option<(PathBuf, gio::FileMonitor)>>,
    /// Bumped on every load: what an older load still sends is dropped.
    generation: Cell<u64>,
    info: RefCell<Option<Info>>,
    /// The pointer is on the controls: they stay.
    on_controls: Cell<bool>,
    hide_controls: RefCell<Option<glib::SourceId>>,
    hide_toast: RefCell<Option<glib::SourceId>>,
    slideshow: RefCell<Option<Slideshow>>,
    relist: RefCell<Option<glib::SourceId>>,
}

struct Slideshow {
    timer: glib::SourceId,
    was_fullscreen: bool,
}

struct Buttons {
    previous: gtk::Button,
    next: gtk::Button,
    save: gtk::Button,
    info: gtk::ToggleButton,
    trash: gtk::Button,
    /// Those that need an image.
    for_image: Vec<gtk::Widget>,
}

impl Viewer {
    pub fn new(app: &gtk::Application, config: Config) -> Rc<Self> {
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title(tr("view-title"))
            .default_width(1000)
            .default_height(700)
            .css_classes(["orbit-view"])
            .build();
        // No title bar: the controls carry the window's buttons.
        window.set_titlebar(Some(&gtk::Box::builder().visible(false).build()));

        let canvas = Canvas::new();
        let strip = Strip::new();
        let panel = Panel::new();

        let message = gtk::Label::builder()
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["message"])
            .build();
        let open = gtk::Button::builder()
            .label(tr("view-open"))
            .halign(gtk::Align::Center)
            .css_classes(["open"])
            .build();
        let page = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(15)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .build();
        page.append(&gtk::Image::builder().icon_name("image-x-generic-symbolic").pixel_size(60).css_classes(["message-icon"]).build());
        page.append(&message);
        page.append(&open);
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .hexpand(true)
            .vexpand(true)
            .build();
        stack.add_named(&canvas.widget, Some("image"));
        stack.add_named(&page, Some("message"));

        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.append(&stack);
        body.append(&panel.widget);

        // ---- the bar along the top
        let button = |icon: &str, tip: &str, key: &str| {
            gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(shortcut(tip, key))
                .focus_on_click(false)
                .build()
        };
        let previous = button("go-previous-symbolic", "view-previous", "←");
        let next = button("go-next-symbolic", "view-next", "→");
        let name = gtk::Label::builder()
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .hexpand(true)
            .xalign(0.0)
            .css_classes(["name"])
            .build();
        let position = gtk::Label::builder().css_classes(["position"]).build();
        let zoom_out = button("zoom-out-symbolic", "view-zoom-out", "−");
        let zoom = gtk::Label::builder()
            .width_chars(5)
            .css_classes(["zoom"])
            .build();
        let zoom_in = button("zoom-in-symbolic", "view-zoom-in", "+");
        let fit = button("zoom-fit-best-symbolic", "view-fit", "0");
        let actual = button("zoom-original-symbolic", "view-actual-size", "1");
        let turn_left = button("object-rotate-left-symbolic", "view-turn-left", "Shift+R");
        let turn_right = button("object-rotate-right-symbolic", "view-turn-right", "R");
        let mirror_h = button("object-flip-horizontal-symbolic", "view-mirror-horizontal", "H");
        let mirror_v = button("object-flip-vertical-symbolic", "view-mirror-vertical", "V");
        let save = button("document-save-symbolic", "view-save", "Ctrl+S");
        let copy = button("edit-copy-symbolic", "view-copy", "Ctrl+C");
        let open_button = button("document-open-symbolic", "view-open", "Ctrl+O");
        let slideshow = button("media-playback-start-symbolic", "view-slideshow", "F5");
        let fullscreen = button("view-fullscreen-symbolic", "view-fullscreen", "F11");
        let trash = button("user-trash-symbolic", "view-trash", "Delete");
        let info = gtk::ToggleButton::builder()
            .icon_name("dialog-information-symbolic")
            .tooltip_text(shortcut("view-info", "I"))
            .focus_on_click(false)
            .build();
        let close = gtk::WindowControls::builder()
            .side(gtk::PackType::End)
            .decoration_layout(":minimize,maximize,close")
            .build();
        let group = |widgets: &[&gtk::Widget]| {
            let b = gtk::Box::builder().css_classes(["group"]).build();
            for w in widgets {
                b.append(*w);
            }
            b
        };
        let bar = gtk::Box::builder()
            .spacing(10)
            .css_classes(["bar"])
            .build();
        bar.append(&group(&[previous.upcast_ref(), next.upcast_ref()]));
        bar.append(&position);
        bar.append(&name);
        bar.append(&group(&[
            zoom_out.upcast_ref(),
            zoom.upcast_ref(),
            zoom_in.upcast_ref(),
            fit.upcast_ref(),
            actual.upcast_ref(),
        ]));
        bar.append(&group(&[
            turn_left.upcast_ref(),
            turn_right.upcast_ref(),
            mirror_h.upcast_ref(),
            mirror_v.upcast_ref(),
            save.upcast_ref(),
        ]));
        bar.append(&group(&[
            copy.upcast_ref(),
            info.upcast_ref(),
            slideshow.upcast_ref(),
            fullscreen.upcast_ref(),
            trash.upcast_ref(),
            open_button.upcast_ref(),
        ]));
        bar.append(&close);
        // Dragging the bar moves the window.
        let handle = gtk::WindowHandle::builder().child(&bar).build();
        let top = gtk::Revealer::builder()
            .child(&handle)
            .transition_type(gtk::RevealerTransitionType::Crossfade)
            .valign(gtk::Align::Start)
            .reveal_child(true)
            .build();
        let bottom = gtk::Revealer::builder()
            .child(&strip.widget)
            .transition_type(gtk::RevealerTransitionType::Crossfade)
            .valign(gtk::Align::End)
            .reveal_child(config.thumbnails)
            .build();
        let toast_label = gtk::Label::builder().wrap(true).build();
        let toast_box = gtk::Box::builder().css_classes(["toast"]).build();
        toast_box.append(&toast_label);
        let toast = gtk::Revealer::builder()
            .child(&toast_box)
            .transition_type(gtk::RevealerTransitionType::Crossfade)
            .valign(gtk::Align::End)
            .halign(gtk::Align::Center)
            .margin_bottom(110)
            .can_target(false)
            .build();

        let overlay = gtk::Overlay::builder().child(&body).build();
        overlay.add_overlay(&top);
        overlay.add_overlay(&bottom);
        overlay.add_overlay(&toast);
        window.set_child(Some(&overlay));

        let for_image: Vec<gtk::Widget> = [
            zoom_out.upcast_ref::<gtk::Widget>(),
            zoom_in.upcast_ref(),
            fit.upcast_ref(),
            actual.upcast_ref(),
            turn_left.upcast_ref(),
            turn_right.upcast_ref(),
            mirror_h.upcast_ref(),
            mirror_v.upcast_ref(),
            copy.upcast_ref(),
            slideshow.upcast_ref(),
            trash.upcast_ref(),
            info.upcast_ref(),
        ]
        .into_iter()
        .cloned()
        .collect();

        let viewer = Rc::new(Self {
            window,
            supported: decode::supported_types(),
            editable: edit::editable_types(),
            canvas,
            strip,
            panel,
            stack,
            message,
            top,
            bottom,
            toast_revealer: toast,
            toast_label,
            name,
            zoom,
            position,
            buttons: Buttons {
                previous: previous.clone(),
                next: next.clone(),
                save: save.clone(),
                info: info.clone(),
                trash: trash.clone(),
                for_image,
            },
            images: RefCell::default(),
            index: Cell::new(None),
            dir: RefCell::default(),
            generation: Cell::new(0),
            info: RefCell::default(),
            on_controls: Cell::new(false),
            hide_controls: RefCell::default(),
            hide_toast: RefCell::default(),
            slideshow: RefCell::default(),
            relist: RefCell::default(),
            config,
        });

        let act = |button: &gtk::Button, f: fn(&Rc<Viewer>)| {
            let weak = Rc::downgrade(&viewer);
            button.connect_clicked(move |_| {
                if let Some(v) = weak.upgrade() {
                    f(&v);
                }
            });
        };
        act(&previous, |v| v.step(-1));
        act(&next, |v| v.step(1));
        act(&zoom_out, |v| v.canvas.zoom_by(1.0 / canvas::STEP));
        act(&zoom_in, |v| v.canvas.zoom_by(canvas::STEP));
        act(&fit, |v| v.canvas.fit());
        act(&actual, |v| v.canvas.zoom_to(1.0, None));
        act(&turn_left, |v| v.transform(|c| c.turn(false)));
        act(&turn_right, |v| v.transform(|c| c.turn(true)));
        act(&mirror_h, |v| v.transform(|c| c.mirror(true)));
        act(&mirror_v, |v| v.transform(|c| c.mirror(false)));
        act(&save, |v| v.save());
        act(&copy, |v| v.copy());
        act(&open_button, |v| v.choose());
        act(&open, |v| v.choose());
        act(&slideshow, |v| v.toggle_slideshow());
        act(&fullscreen, |v| v.toggle_fullscreen());
        act(&trash, |v| v.trash());
        {
            let weak = Rc::downgrade(&viewer);
            info.connect_toggled(move |b| {
                if let Some(v) = weak.upgrade() {
                    v.panel.widget.set_reveal_child(b.is_active());
                }
            });
        }
        info.set_active(viewer.config.info);
        {
            let weak = Rc::downgrade(&viewer);
            viewer.canvas.connect_zoom(move |z| {
                if let Some(v) = weak.upgrade() {
                    v.zoom.set_label(&format!("{:.0}%", z * 100.0));
                }
            });
        }
        {
            let weak = Rc::downgrade(&viewer);
            viewer.strip.connect_pick(move |i| {
                if let Some(v) = weak.upgrade() {
                    v.show(i);
                }
            });
        }
        viewer.connect_controls(&overlay);
        viewer.connect_keys();
        viewer.connect_drop(&overlay);
        viewer.show_message(&tr("view-empty"));
        viewer
    }

    pub fn window(&self) -> &gtk::ApplicationWindow {
        &self.window
    }

    // ------------------------------------------------------------ files

    /// Opens `path`, and lists its folder.
    pub fn open(self: &Rc<Self>, path: &Path) {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let Some(dir) = path.parent().map(Path::to_path_buf) else {
            return;
        };
        let images = folder::images(&dir, &self.supported, Some(&path));
        let index = images.iter().position(|p| *p == path);
        *self.images.borrow_mut() = images;
        self.watch(&dir);
        self.strip.set(&self.images.borrow(), index);
        if let Some(index) = index {
            self.show(index);
        }
    }

    fn watch(self: &Rc<Self>, dir: &Path) {
        if self.dir.borrow().as_ref().is_some_and(|(d, _)| d == dir) {
            return;
        }
        let monitor = gio::File::for_path(dir)
            .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE);
        match monitor {
            Ok(monitor) => {
                let weak = Rc::downgrade(self);
                monitor.connect_changed(move |_, file, _, event| {
                    let Some(v) = weak.upgrade() else { return };
                    if event == gio::FileMonitorEvent::ChangesDoneHint
                        && let Some(path) = file.path()
                    {
                        v.strip.invalidate(&path);
                    }
                    v.relist_soon();
                });
                *self.dir.borrow_mut() = Some((dir.to_path_buf(), monitor));
            }
            Err(err) => {
                log::warn!("watching {}: {err}", dir.display());
                *self.dir.borrow_mut() = None;
            }
        }
    }

    /// Lists the folder again in a moment: changes come in bursts.
    fn relist_soon(self: &Rc<Self>) {
        if let Some(id) = self.relist.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(300), move || {
            if let Some(v) = weak.upgrade() {
                v.relist.borrow_mut().take();
                v.relist();
            }
        });
        *self.relist.borrow_mut() = Some(id);
    }

    fn relist(self: &Rc<Self>) {
        let Some(dir) = self.dir.borrow().as_ref().map(|(d, _)| d.clone()) else {
            return;
        };
        let current = self.current();
        let keep = current.as_deref().filter(|p| p.exists());
        let images = folder::images(&dir, &self.supported, keep);
        let index = match (&current, keep) {
            (Some(path), Some(_)) => images.iter().position(|p| p == path),
            // The current file went: its neighbour takes its place.
            (Some(_), None) => self.index.get().map(|i| i.min(images.len().saturating_sub(1))),
            _ => None,
        }
        .filter(|_| !images.is_empty());
        *self.images.borrow_mut() = images;
        self.strip.set(&self.images.borrow(), index);
        let gone = current.is_some() && keep.is_none();
        match index {
            Some(i) if gone => self.show(i),
            Some(i) => {
                self.index.set(Some(i));
                self.update_position();
            }
            None if gone => {
                self.index.set(None);
                self.clear();
            }
            None => {}
        }
    }

    fn current(&self) -> Option<PathBuf> {
        self.index.get().and_then(|i| self.images.borrow().get(i).cloned())
    }

    fn step(self: &Rc<Self>, by: isize) {
        let len = self.images.borrow().len();
        let Some(i) = self.index.get() else { return };
        let next = i as isize + by;
        if (0..len as isize).contains(&next) {
            self.show(next as usize);
        }
    }

    fn show(self: &Rc<Self>, index: usize) {
        let Some(path) = self.images.borrow().get(index).cloned() else {
            return;
        };
        self.index.set(Some(index));
        self.strip.select(Some(index));
        self.update_position();
        self.load(path);
    }

    fn update_position(&self) {
        let len = self.images.borrow().len();
        let index = self.index.get();
        self.position.set_label(&match index {
            Some(i) if len > 1 => format!("{} / {len}", i + 1),
            _ => String::new(),
        });
        self.buttons.previous.set_sensitive(index.is_some_and(|i| i > 0));
        self.buttons.next.set_sensitive(index.is_some_and(|i| i + 1 < len));
    }

    fn load(self: &Rc<Self>, path: PathBuf) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        *self.info.borrow_mut() = None;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.name.set_label(&name);
        self.window.set_title(Some(&name));
        self.update_save();

        let rx = decode::load(path.clone());
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let current = |weak: &std::rc::Weak<Viewer>| {
                weak.upgrade().filter(|v| v.generation.get() == generation)
            };
            let mut first = true;
            let mut cache: Vec<(gdk::Texture, Duration)> = Vec::new();
            let mut kept = 0usize;
            while let Ok(event) = rx.recv().await {
                let Some(v) = current(&weak) else { return };
                match event {
                    Event::Opened(info) => *v.info.borrow_mut() = Some(info),
                    Event::Frame(frame) => {
                        let delay = frame.delay.map(|d| if d < MIN_DELAY { SLOW_DELAY } else { d });
                        let bytes = frame.data.len();
                        let texture = frame.texture();
                        v.shown(&path, &texture, first);
                        first = false;
                        let Some(delay) = delay else { return };
                        kept += bytes;
                        if kept <= decode::ANIMATION_CACHE {
                            cache.push((texture, delay));
                        } else {
                            cache.clear();
                        }
                        drop(v);
                        glib::timeout_future(delay).await;
                    }
                    Event::Looped => {
                        drop(v);
                        // The whole animation is here: play it from memory.
                        for (texture, delay) in cache.iter().cycle() {
                            let Some(v) = current(&weak) else { return };
                            v.canvas.set_image(texture, false);
                            drop(v);
                            glib::timeout_future(*delay).await;
                        }
                        return;
                    }
                    Event::Failed(failure) => {
                        v.failed(&path, failure);
                        return;
                    }
                }
            }
        });
    }

    /// A frame of `path` arrived; the `first` one replaces what was shown.
    fn shown(&self, path: &Path, texture: &gdk::Texture, first: bool) {
        self.canvas.set_image(texture, first);
        if !first {
            return;
        }
        self.stack.set_visible_child_name("image");
        for w in &self.buttons.for_image {
            w.set_sensitive(true);
        }
        self.panel.set(
            path,
            Some((texture.width(), texture.height())),
            self.info.borrow().as_ref(),
        );
        self.update_save();
    }

    fn failed(&self, path: &Path, failure: Failure) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let reason = match failure {
            Failure::UnknownFormat => tr("view-unknown-format"),
            Failure::Other(reason) => reason,
        };
        self.show_message(&tr_args(
            "view-failed",
            &[("name", text(name)), ("reason", text(reason))],
        ));
        // The file is still there: it can go to the trash.
        self.buttons.trash.set_sensitive(true);
        self.panel.set(path, None, None);
    }

    fn show_message(&self, message: &str) {
        self.canvas.clear();
        self.message.set_label(message);
        self.stack.set_visible_child_name("message");
        for w in &self.buttons.for_image {
            w.set_sensitive(false);
        }
        self.buttons.save.set_sensitive(false);
        self.reveal_controls(true);
    }

    fn clear(&self) {
        self.generation.set(self.generation.get() + 1);
        self.name.set_label("");
        self.window.set_title(Some(&tr("view-title")));
        self.update_position();
        self.show_message(&tr("view-empty"));
    }

    fn choose(self: &Rc<Self>) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(&tr("view-filter-images")));
        for mime in &self.supported {
            filter.add_mime_type(mime);
        }
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title(tr("view-open"))
            .filters(&filters)
            .default_filter(&filter)
            .modal(true)
            .build();
        if let Some(dir) = self.dir.borrow().as_ref() {
            dialog.set_initial_folder(Some(&gio::File::for_path(&dir.0)));
        }
        let weak = Rc::downgrade(self);
        dialog.open(Some(&self.window), gio::Cancellable::NONE, move |result| {
            if let (Some(v), Ok(file)) = (weak.upgrade(), result)
                && let Some(path) = file.path()
            {
                v.open(&path);
            }
        });
    }

    // ---------------------------------------------------------- editing

    /// Turns or mirrors the image; saving makes it last.
    fn transform(&self, f: impl Fn(&Canvas)) {
        f(&self.canvas);
        self.update_save();
    }

    fn update_save(&self) {
        let changed = self.canvas.orientation() != (0, false);
        let editable = self
            .info
            .borrow()
            .as_ref()
            .is_some_and(|i| self.editable.contains(&i.mime));
        let save = &self.buttons.save;
        save.set_sensitive(changed && editable);
        if changed && !editable {
            save.set_tooltip_text(Some(&tr("view-save-unsupported")));
        } else {
            save.set_tooltip_text(Some(&shortcut("view-save", "Ctrl+S")));
        }
        if changed && editable {
            save.add_css_class("pending");
        } else {
            save.remove_css_class("pending");
        }
    }

    fn save(self: &Rc<Self>) {
        let Some(path) = self.current() else { return };
        if !self.buttons.save.is_sensitive() {
            return;
        }
        let (turns, mirrored) = self.canvas.orientation();
        self.buttons.save.set_sensitive(false);
        let rx = edit::save(path.clone(), turns, mirrored);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = rx.recv().await.unwrap_or_else(|_| Err("no answer".into()));
            let Some(v) = weak.upgrade() else { return };
            match result {
                Ok(()) => {
                    v.toast(&tr("view-saved"));
                    v.strip.invalidate(&path);
                    if v.current().as_deref() == Some(path.as_path()) {
                        v.load(path);
                    }
                }
                Err(err) => {
                    v.toast(&tr_args("view-save-failed", &[("reason", text(err))]));
                    v.update_save();
                }
            }
        });
    }

    fn copy(self: &Rc<Self>) {
        if let Some(texture) = self.canvas.texture() {
            self.window.clipboard().set_texture(&texture);
            self.toast(&tr("view-copied"));
        }
    }

    fn trash(self: &Rc<Self>) {
        let Some(path) = self.current() else { return };
        let weak = Rc::downgrade(self);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        gio::File::for_path(&path).trash_async(
            glib::Priority::DEFAULT,
            gio::Cancellable::NONE,
            move |result| {
                let Some(v) = weak.upgrade() else { return };
                match result {
                    Ok(()) => {
                        v.toast(&tr_args("view-trashed", &[("name", text(name))]));
                        // The folder's monitor would notice too; no need to wait.
                        v.relist();
                    }
                    Err(err) => v.toast(&tr_args(
                        "view-trash-failed",
                        &[("reason", text(err.message().to_owned()))],
                    )),
                }
            },
        );
    }

    // ------------------------------------------------- screen and controls

    fn toggle_fullscreen(&self) {
        if self.window.is_fullscreen() {
            self.window.unfullscreen();
        } else {
            self.window.fullscreen();
        }
    }

    fn toggle_slideshow(self: &Rc<Self>) {
        if let Some(show) = self.slideshow.borrow_mut().take() {
            show.timer.remove();
            if !show.was_fullscreen {
                self.window.unfullscreen();
            }
            self.bottom.set_reveal_child(self.config.thumbnails);
            self.reveal_controls(true);
            return;
        }
        if self.current().is_none() {
            return;
        }
        let was_fullscreen = self.window.is_fullscreen();
        self.window.fullscreen();
        self.reveal_controls(false);
        self.bottom.set_reveal_child(false);
        let weak = Rc::downgrade(self);
        let seconds = self.config.slideshow_seconds.max(1);
        let timer = glib::timeout_add_seconds_local(seconds, move || {
            let Some(v) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let len = v.images.borrow().len();
            // Round and round.
            if let Some(i) = v.index.get()
                && len > 1
            {
                v.show((i + 1) % len);
            }
            glib::ControlFlow::Continue
        });
        *self.slideshow.borrow_mut() = Some(Slideshow {
            timer,
            was_fullscreen,
        });
    }

    fn in_slideshow(&self) -> bool {
        self.slideshow.borrow().is_some()
    }

    /// The controls appear on any movement of the pointer and go a moment
    /// after it stops, unless it is on them.
    fn connect_controls(self: &Rc<Self>, overlay: &gtk::Overlay) {
        let motion = gtk::EventControllerMotion::new();
        let last = Cell::new((f64::NAN, f64::NAN));
        let weak = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            // Some compositors repeat the position when nothing moved.
            if last.get() == (x, y) {
                return;
            }
            last.set((x, y));
            if let Some(v) = weak.upgrade()
                && !v.in_slideshow()
            {
                v.reveal_controls(true);
                v.hide_controls_later();
            }
        });
        overlay.add_controller(motion);
        for revealer in [&self.top, &self.bottom] {
            let motion = gtk::EventControllerMotion::new();
            let weak = Rc::downgrade(self);
            motion.connect_enter(move |_, _, _| {
                if let Some(v) = weak.upgrade() {
                    v.on_controls.set(true);
                }
            });
            let weak = Rc::downgrade(self);
            motion.connect_leave(move |_| {
                if let Some(v) = weak.upgrade() {
                    v.on_controls.set(false);
                    v.hide_controls_later();
                }
            });
            revealer.add_controller(motion);
        }
    }

    fn reveal_controls(&self, shown: bool) {
        self.top.set_reveal_child(shown);
        self.top.set_can_target(shown);
        let strip = shown && self.config.thumbnails && !self.in_slideshow();
        self.bottom.set_reveal_child(strip);
        self.bottom.set_can_target(strip);
        self.window
            .set_cursor_from_name(if shown || !self.window.is_fullscreen() { None } else { Some("none") });
    }

    fn hide_controls_later(self: &Rc<Self>) {
        if let Some(id) = self.hide_controls.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(CONTROLS_FOR, move || {
            let Some(v) = weak.upgrade() else { return };
            v.hide_controls.borrow_mut().take();
            // With no image there is nothing to uncover.
            if !v.on_controls.get() && v.canvas.texture().is_some() {
                v.reveal_controls(false);
            }
        });
        *self.hide_controls.borrow_mut() = Some(id);
    }

    fn toast(self: &Rc<Self>, message: &str) {
        self.toast_label.set_label(message);
        self.toast_revealer.set_reveal_child(true);
        if let Some(id) = self.hide_toast.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(TOAST_FOR, move || {
            if let Some(v) = weak.upgrade() {
                v.hide_toast.borrow_mut().take();
                v.toast_revealer.set_reveal_child(false);
            }
        });
        *self.hide_toast.borrow_mut() = Some(id);
    }

    fn connect_keys(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        // Ahead of the scrolled window, which would take the arrows.
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(v) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let has = v.canvas.texture().is_some();
            use gdk::Key;
            match key {
                Key::Escape if v.in_slideshow() => v.toggle_slideshow(),
                Key::Escape if v.window.is_fullscreen() => v.window.unfullscreen(),
                Key::F5 => v.toggle_slideshow(),
                Key::F11 | Key::f if !ctrl => v.toggle_fullscreen(),
                Key::o if ctrl => v.choose(),
                Key::w | Key::q if ctrl => v.window.close(),
                Key::Right | Key::Page_Down | Key::space => v.step(1),
                Key::Left | Key::Page_Up | Key::BackSpace => v.step(-1),
                Key::Home => {
                    if !v.images.borrow().is_empty() {
                        v.show(0);
                    }
                }
                Key::End => {
                    let len = v.images.borrow().len();
                    if len > 0 {
                        v.show(len - 1);
                    }
                }
                Key::Delete | Key::KP_Delete => v.trash(),
                Key::i if !ctrl => v.buttons.info.set_active(!v.buttons.info.is_active()),
                _ if !has => return glib::Propagation::Proceed,
                Key::plus | Key::equal | Key::KP_Add => v.canvas.zoom_by(canvas::STEP),
                Key::minus | Key::KP_Subtract => v.canvas.zoom_by(1.0 / canvas::STEP),
                Key::_0 | Key::KP_0 => v.canvas.fit(),
                Key::_1 | Key::KP_1 => v.canvas.zoom_to(1.0, None),
                Key::r if !ctrl => v.transform(|c| c.turn(true)),
                Key::R if !ctrl => v.transform(|c| c.turn(false)),
                Key::h if !ctrl => v.transform(|c| c.mirror(true)),
                Key::v if !ctrl => v.transform(|c| c.mirror(false)),
                Key::s if ctrl => v.save(),
                Key::c if ctrl => v.copy(),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.window.add_controller(keys);
    }

    /// Dropping an image file on the window opens it.
    fn connect_drop(self: &Rc<Self>, overlay: &gtk::Overlay) {
        let target = gtk::DropTarget::new(gio::File::static_type(), gdk::DragAction::COPY);
        let weak = Rc::downgrade(self);
        target.connect_drop(move |_, value, _, _| {
            let (Some(v), Ok(file)) = (weak.upgrade(), value.get::<gio::File>()) else {
                return false;
            };
            match file.path() {
                Some(path) => {
                    v.open(&path);
                    true
                }
                None => false,
            }
        });
        overlay.add_controller(target);
    }
}

/// A button's tooltip: what it does, and its key.
fn shortcut(id: &str, key: &str) -> String {
    format!("{} ({key})", tr(id))
}
