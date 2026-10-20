//! The window: the history, newest first, with a search field.
//!
//! Typing anywhere searches. Enter or a click puts the item back in the
//! clipboard and hides the window — then Ctrl+V pastes it where you were.
//! Delete removes the selected item; Escape hides the window.

use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use gtk::{gdk, gdk_pixbuf, glib, prelude::*};
use orbit_app::i18n::{number, tr, tr_args};
use orbit_common::ConfigDir;
use serde::Deserialize;

use crate::{
    history::{Content, History, Item, Store},
    wayland::{self, Command, Handle},
};

/// `~/.config/orbit/clip.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct Config {
    /// Items kept.
    limit: usize,
    /// Keep copied images too.
    images: bool,
    /// Keep the clipboard and the primary selection (mouse selection,
    /// middle-button paste) the same.
    sync_primary: bool,
    /// Skip copies that password managers mark as secret.
    ignore_secrets: bool,
    /// Largest copy kept, in megabytes.
    max_size_mb: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            limit: 200,
            images: true,
            sync_primary: true,
            ignore_secrets: true,
            max_size_mb: 20,
        }
    }
}

/// A thumbnail and the image's full size.
type Thumbnail = (gdk::Texture, i32, i32);

struct Clip {
    history: RefCell<History>,
    store: Store,
    watcher: Handle,
    window: gtk::ApplicationWindow,
    search: gtk::SearchEntry,
    list: gtk::ListBox,
    status: gtk::Label,
    thumbnails: RefCell<HashMap<u64, Option<Thumbnail>>>,
}

pub fn build(app: &gtk::Application, dir: &ConfigDir) -> anyhow::Result<gtk::ApplicationWindow> {
    let config: Config = dir.app("clip")?;
    let store = Store::new(&glib::user_data_dir().join("orbit").join("clip"));
    let history = store.load(config.limit);

    let (sender, receiver) = async_channel::unbounded();
    let watcher = wayland::spawn(
        sender,
        wayland::Options {
            sync_primary: config.sync_primary,
            ignore_secrets: config.ignore_secrets,
            max_bytes: config.max_size_mb.max(1) * 1024 * 1024,
        },
    )
    .context("watching the clipboard")?;

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(tr("clip-title"))
        .css_classes(["orbit-clip"])
        .resizable(false)
        .build();
    let search = gtk::SearchEntry::builder()
        .placeholder_text(tr("clip-search"))
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Browse)
        .css_classes(["items"])
        .build();
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&list)
        .vexpand(true)
        .build();
    // The window's fixed size.
    scrolled.set_size_request(450, 450);
    let status = gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["status"])
        .build();
    let clear = gtk::Button::builder()
        .label(tr("clip-clear"))
        .css_classes(["clear"])
        .focus_on_click(false)
        .build();
    let footer = gtk::Box::builder().spacing(10).build();
    footer.append(&status);
    footer.append(&clear);
    let page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(["page"])
        .build();
    page.append(&search);
    page.append(&scrolled);
    page.append(&footer);
    window.set_child(Some(&page));
    search.set_key_capture_widget(Some(&window));

    let clip = Rc::new(Clip {
        history: RefCell::new(history),
        store,
        watcher,
        window: window.clone(),
        search: search.clone(),
        list: list.clone(),
        status,
        thumbnails: RefCell::new(HashMap::new()),
    });
    clip.rebuild();

    // Copies from other applications.
    {
        let clip = clip.clone();
        let images = config.images;
        glib::spawn_future_local(async move {
            while let Ok(copied) = receiver.recv().await {
                if !images && matches!(copied.content, Content::Image { .. }) {
                    continue;
                }
                let changed = clip
                    .history
                    .borrow_mut()
                    .add(copied.content, copied.primary, now());
                if changed {
                    clip.save();
                    clip.rebuild();
                }
            }
        });
    }
    {
        let clip = clip.clone();
        list.connect_row_activated(move |_, row| clip.choose(row));
    }
    {
        let clip = clip.clone();
        search.connect_search_changed(move |_| clip.rebuild());
    }
    {
        let clip = clip.clone();
        search.connect_activate(move |_| {
            let row = clip
                .list
                .selected_row()
                .or_else(|| clip.list.row_at_index(0));
            if let Some(row) = row {
                clip.choose(&row);
            }
        });
    }
    {
        let clip = clip.clone();
        clear.connect_clicked(move |_| {
            clip.history.borrow_mut().clear();
            clip.save();
            clip.rebuild();
        });
    }
    {
        // Each time it is shown: an empty search, the newest item selected.
        let clip = clip.clone();
        window.connect_map(move |_| {
            clip.search.set_text("");
            clip.rebuild();
            clip.search.grab_focus();
        });
    }

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let clip = clip.clone();
        keys.connect_key_pressed(move |_, key, _, _| match key {
            gdk::Key::Escape => {
                clip.window.set_visible(false);
                glib::Propagation::Stop
            }
            gdk::Key::Delete | gdk::Key::KP_Delete => {
                if let Some(row) = clip.list.selected_row()
                    && let Some(id) = row_id(&row)
                {
                    let index = row.index();
                    clip.history.borrow_mut().remove(id);
                    clip.save();
                    clip.rebuild();
                    clip.select(index);
                }
                glib::Propagation::Stop
            }
            // Arrows move in the list even while typing in the search.
            gdk::Key::Down | gdk::Key::Up => {
                let index = clip.list.selected_row().map_or(-1, |r| r.index());
                clip.select(if key == gdk::Key::Down {
                    index + 1
                } else {
                    (index - 1).max(0)
                });
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
    }
    window.add_controller(keys);
    Ok(window)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn row_id(row: &gtk::ListBoxRow) -> Option<u64> {
    row.widget_name().parse().ok()
}

/// "now", "5 min ago", "3 h ago", "2 d ago".
fn ago(time: i64) -> String {
    let secs = (now() - time).max(0);
    let n = |v: i64| [("n", number(v as f64))];
    match secs {
        0..60 => tr("clip-ago-now"),
        60..3600 => tr_args("clip-ago-minutes", &n(secs / 60)),
        3600..86_400 => tr_args("clip-ago-hours", &n(secs / 3600)),
        _ => tr_args("clip-ago-days", &n(secs / 86_400)),
    }
}

/// A thumbnail at most 240 × 120, decoded at that size (a screenshot would
/// otherwise stay decoded at full size), and the image's own size.
fn thumbnail(bytes: &[u8]) -> Option<Thumbnail> {
    let loader = gdk_pixbuf::PixbufLoader::new();
    let size = Rc::new(std::cell::Cell::new((0, 0)));
    {
        let size = size.clone();
        loader.connect_size_prepared(move |loader, w, h| {
            size.set((w, h));
            let scale = (240.0 / f64::from(w)).min(120.0 / f64::from(h)).min(1.0);
            loader.set_size(
                ((f64::from(w) * scale) as i32).max(1),
                ((f64::from(h) * scale) as i32).max(1),
            );
        });
    }
    loader.write(bytes).ok()?;
    loader.close().ok()?;
    let pixbuf = loader.pixbuf()?;
    let (w, h) = size.get();
    log::debug!(
        "thumbnail {}x{} of {w}x{h}",
        pixbuf.width(),
        pixbuf.height()
    );
    Some((gdk::Texture::for_pixbuf(&pixbuf), w, h))
}

impl Clip {
    fn save(&self) {
        if let Err(err) = self.store.save(&self.history.borrow()) {
            log::warn!("saving the clipboard history: {err:#}");
        }
    }

    fn select(&self, index: i32) {
        if let Some(row) = self.list.row_at_index(index) {
            self.list.select_row(Some(&row));
            // Focusing the row scrolls the list to it; typing stays in the
            // search, with the cursor at its end.
            row.grab_focus();
            self.search.grab_focus();
            self.search.set_position(-1);
        }
    }

    /// Puts the row's item back in the clipboard and hides the window.
    fn choose(&self, row: &gtk::ListBoxRow) {
        let Some(id) = row_id(row) else { return };
        let Some(content) = self.history.borrow().get(id).map(|i| i.content.clone()) else {
            return;
        };
        self.history.borrow_mut().add(content.clone(), false, now());
        self.save();
        self.watcher.send(Command::Set(content));
        self.window.set_visible(false);
    }

    /// Fills the list from the history and the search.
    fn rebuild(&self) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let history = self.history.borrow();
        let query = self.search.text();
        let mut shown = 0;
        for item in history.search(&query) {
            self.list.append(&self.row(item));
            shown += 1;
        }
        let total = history.items().len();
        // Thumbnails of items no longer there go.
        self.thumbnails
            .borrow_mut()
            .retain(|id, _| history.get(*id).is_some());
        if shown == 0 {
            let text = if total == 0 {
                tr("clip-empty")
            } else {
                tr("clip-no-match")
            };
            let placeholder = gtk::Label::builder()
                .label(text)
                .css_classes(["placeholder"])
                .build();
            self.list.set_placeholder(Some(&placeholder));
        }
        self.status
            .set_label(&tr_args("clip-count", &[("count", number(total as f64))]));
        if let Some(row) = self.list.row_at_index(0) {
            self.list.select_row(Some(&row));
        }
    }

    fn row(&self, item: &Item) -> gtk::ListBoxRow {
        let body: gtk::Widget = match &item.content {
            Content::Text(text) => {
                // The first lines only; a huge copy would make a huge label.
                let preview: String = text.trim().chars().take(400).collect();
                gtk::Label::builder()
                    .label(preview)
                    .xalign(0.0)
                    .wrap(true)
                    .wrap_mode(gtk::pango::WrapMode::WordChar)
                    .lines(3)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .max_width_chars(1)
                    .hexpand(true)
                    .css_classes(["text"])
                    .build()
                    .upcast()
            }
            Content::Image { bytes, .. } => {
                let thumb = self
                    .thumbnails
                    .borrow_mut()
                    .entry(item.id)
                    .or_insert_with(|| thumbnail(bytes))
                    .clone();
                let column = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(5)
                    .hexpand(true)
                    .build();
                match thumb {
                    Some((texture, w, h)) => {
                        let picture = gtk::Picture::for_paintable(&texture);
                        picture.set_can_shrink(false);
                        // In a row of its own: a Picture given the column's
                        // whole width would scale the thumbnail up to fill it.
                        let holder = gtk::Box::builder().halign(gtk::Align::Start).build();
                        holder.append(&picture);
                        column.append(&holder);
                        column.append(
                            &gtk::Label::builder()
                                .label(tr_args(
                                    "clip-image",
                                    &[
                                        ("width", number(f64::from(w))),
                                        ("height", number(f64::from(h))),
                                    ],
                                ))
                                .xalign(0.0)
                                .css_classes(["meta"])
                                .build(),
                        );
                    }
                    None => column.append(&gtk::Label::new(Some(&tr("clip-image-unreadable")))),
                }
                column.upcast()
            }
        };
        let time = gtk::Label::builder()
            .label(ago(item.time))
            .valign(gtk::Align::Start)
            .css_classes(["meta"])
            .build();
        let line = gtk::Box::builder().spacing(10).build();
        line.append(&body);
        line.append(&time);
        let row = gtk::ListBoxRow::builder().child(&line).build();
        row.set_widget_name(&item.id.to_string());
        row
    }
}
