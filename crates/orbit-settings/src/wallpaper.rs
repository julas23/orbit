//! The wallpaper page: a picture (from the folders where pictures usually
//! are, or any file), how it meets the monitor, and the colour under it;
//! the colour alone; or an effect in motion (aurora, ink, facets, …) with
//! its colours and speed, following the sound or not, and how long idle
//! before it shows alone (the screensaver). For every monitor, or one (an effect: every monitor
//! without a picture of its own). Each change is written to `core.yaml`
//! and the core reloads it at once.
//!
//! The preview draws what the core will: the monitor's shape, the colour,
//! and the picture laid as the mode says (center and tile at the monitor's
//! real pixels); for an effect, its colours (it moves on the desktop).

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{cairo, gdk, gdk_pixbuf::Pixbuf, gio, glib, prelude::*};
use orbit_app::i18n::{tr, tr_args};
use orbit_common::ConfigDir;

use crate::core::{self, Wallpaper};

/// What can be shown: the picture, the colour, the core's effects, a video
/// (the page offers them as `SHOWS` and a list of the effects).
/// `fire-blue` is the core's `fire` with the blue and white colours.
const KINDS: &[&str] = &[
    "picture",
    "color",
    "aurora",
    "ink",
    "facets",
    "fire",
    "fire-blue",
    "water",
    "borealis",
    "stars",
    "rain",
    "nebula",
    "galaxy",
    "logo",
    "scope",
    "video",
];
/// The screensaver's choices, in minutes; 0: never.
const SAVER_MINUTES: &[u64] = &[0, 1, 2, 5, 10, 15, 30];
/// "Show"'s video, the last; the effects are between the colour and it.
const VIDEO: usize = KINDS.len() - 1;
fn is_effect(kind: usize) -> bool {
    (2..VIDEO).contains(&kind)
}
/// "Show" offers these four; an effect is then chosen in a list of its own.
const SHOWS: &[&str] = &["picture", "color", "effect", "video"];
/// Each effect's own colours, as the core has them (`effects.rs`).
fn effect_colors(effect: &str) -> &'static [&'static str] {
    match effect {
        "aurora" => &["#0b1026", "#1d4e89", "#14b8a6", "#7c3aed", "#db2777"],
        "ink" => &["#070a14", "#1e3a8a", "#f97316", "#fde68a"],
        "facets" => &["#0f172a", "#334155", "#4682b4", "#8b5cf6"],
        "fire" => &["#050202", "#7f1d1d", "#dc2626", "#f97316", "#fde047"],
        "water" => &["#021526", "#03346e", "#0e7490", "#67e8f9", "#ecfeff"],
        "borealis" => &["#030712", "#4ade80", "#2dd4bf", "#818cf8", "#c084fc"],
        "stars" => &["#02030a", "#1e294f", "#93c5fd", "#fde68a", "#fca5a5"],
        "rain" => &["#050812", "#1e293b", "#f59e0b", "#ef4444", "#38bdf8"],
        "nebula" => &["#03020c", "#3b0764", "#be185d", "#f97316", "#fef3c7"],
        "galaxy" => &["#010208", "#0b2a6f", "#2563eb", "#7dd3fc", "#fff4d6"],
        "logo" => &["#05070d", "#4682b4", "#6ea8e0", "#d8dee9", "#ff9500"],
        "scope" => &["#020308", "#22d3ee", "#4ade80", "#facc15", "#f43f5e"],
        _ => &[],
    }
}
/// Blue fire: the colours `fire-blue` writes.
const BLUE_FIRE: &[&str] = &["#020308", "#0c1a4d", "#1d4ed8", "#38bdf8", "#f0f9ff"];
/// The core's modes (`wallpaper.rs`), in the order the list shows them.
const MODES: &[&str] = &["fill", "fit", "center", "stretch", "tile"];
/// Where pictures are looked for, a few folders deep.
const FOLDERS: &[&str] = &[
    "~/.wallpaper",
    "~/.wallpapers",
    "~/Wallpapers",
    "/usr/share/backgrounds",
    "/usr/share/wallpapers",
];
const EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp"];
/// At most so many thumbnails.
const MOST: usize = 400;
/// A thumbnail's size.
const THUMB: (i32, i32) = (192, 108);
/// The preview's width; its height follows the monitor.
const PREVIEW: i32 = 360;

struct Page {
    dir: ConfigDir,
    paper: RefCell<Wallpaper>,
    /// The monitor the picture is for; None: all of them.
    target: RefCell<Option<String>>,
    outputs: Vec<core::Output>,
    /// The picture shown in the preview, smaller, and its real size.
    picture: RefCell<Option<(Pixbuf, (i32, i32))>>,
    preview: gtk::DrawingArea,
    /// What to show (`SHOWS`), and which effect.
    kind: gtk::DropDown,
    effect: gtk::DropDown,
    /// Set while `show_kind` moves the two lists: not a choice of the user's.
    syncing: std::cell::Cell<bool>,
    mode: gtk::DropDown,
    color: gtk::ColorDialogButton,
    /// The effect's colours, one button each, and the speed.
    swatches: gtk::Box,
    speed: gtk::Scale,
    /// The rows shown for a picture, for a colour, for an effect.
    picture_rows: Vec<gtk::Widget>,
    mode_rows: Vec<gtk::Widget>,
    video_rows: Vec<gtk::Widget>,
    color_rows: Vec<gtk::Widget>,
    effect_rows: Vec<gtk::Widget>,
    message: gtk::Label,
    /// Each thumbnail's file and button, to mark the one in use.
    thumbs: RefCell<Vec<(PathBuf, gtk::Button)>>,
}

pub fn page(dir: &ConfigDir) -> gtk::Widget {
    let outputs = core::outputs().unwrap_or_else(|err| {
        log::warn!("the monitors: {err}");
        Vec::new()
    });
    let paper = core::wallpaper(dir);
    let preview = gtk::DrawingArea::builder()
        .content_width(PREVIEW)
        .content_height(PREVIEW * 9 / 16)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .css_classes(["preview"])
        .build();
    let shows: Vec<String> = SHOWS.iter().map(|k| tr(&format!("settings-kind-{k}"))).collect();
    let kind = gtk::DropDown::from_strings(&shows.iter().map(String::as_str).collect::<Vec<_>>());
    let effects: Vec<String> = KINDS[2..VIDEO]
        .iter()
        .map(|k| tr(&format!("settings-kind-{k}")))
        .collect();
    let effect = gtk::DropDown::from_strings(&effects.iter().map(String::as_str).collect::<Vec<_>>());
    let effect_row = crate::row(&tr("settings-effect"), &effect);
    let names: Vec<String> = MODES.iter().map(|m| tr(&format!("settings-mode-{m}"))).collect();
    let mode = gtk::DropDown::from_strings(&names.iter().map(String::as_str).collect::<Vec<_>>());
    mode.set_selected(MODES.iter().position(|m| *m == paper.mode).unwrap_or(0) as u32);
    let color = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
    if let Ok(rgba) = gdk::RGBA::parse(&paper.color) {
        color.set_rgba(&rgba);
    }
    let speed = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 3.0, 0.1);
    speed.set_value(paper.speed);
    speed.set_width_request(200);
    speed.add_mark(1.0, gtk::PositionType::Bottom, None);
    let swatches = gtk::Box::builder().spacing(5).build();
    let fewer = gtk::Button::from_icon_name("list-remove-symbolic");
    let more = gtk::Button::from_icon_name("list-add-symbolic");
    let own = gtk::Button::from_icon_name("edit-undo-symbolic");
    own.set_tooltip_text(Some(&tr("settings-own-colors")));
    let palette = gtk::Box::builder().spacing(5).build();
    palette.append(&swatches);
    palette.append(&fewer);
    palette.append(&more);
    palette.append(&own);
    let choose = gtk::Button::with_label(&tr("settings-choose"));
    choose.set_halign(gtk::Align::End);
    let choose_video = gtk::Button::with_label(&tr("settings-choose-video"));
    choose_video.set_halign(gtk::Align::End);
    let mode_row = crate::row(&tr("settings-mode"), &mode);
    let color_row = crate::row(&tr("settings-color"), &color);
    let colors_row = crate::row(&tr("settings-colors"), &palette);
    let speed_row = crate::row(&tr("settings-speed"), &speed);
    let sound = gtk::Switch::builder()
        .active(paper.sound)
        .halign(gtk::Align::Start)
        .tooltip_text(tr("settings-sound-tip"))
        .build();
    let sound_row = crate::row(&tr("settings-sound"), &sound);
    let minutes: Vec<String> = SAVER_MINUTES
        .iter()
        .map(|m| match m {
            0 => tr("settings-saver-never"),
            m => tr_args("settings-saver-minutes", &[("minutes", (*m as i64).into())]),
        })
        .collect();
    let saver = gtk::DropDown::from_strings(&minutes.iter().map(String::as_str).collect::<Vec<_>>());
    saver.set_tooltip_text(Some(&tr("settings-saver-tip")));
    // The file's seconds, to the nearest choice.
    let near = SAVER_MINUTES
        .iter()
        .enumerate()
        .min_by_key(|(_, m)| (**m * 60).abs_diff(paper.screensaver))
        .map_or(0, |(i, _)| i);
    saver.set_selected(near as u32);
    let saver_row = crate::row(&tr("settings-saver"), &saver);
    let page = Rc::new(Page {
        dir: dir.clone(),
        paper: RefCell::new(paper),
        target: RefCell::new(None),
        outputs,
        picture: RefCell::default(),
        preview,
        kind,
        effect,
        syncing: Default::default(),
        mode,
        color,
        swatches,
        speed,
        picture_rows: vec![choose.clone().upcast()],
        mode_rows: vec![mode_row.clone().upcast()],
        video_rows: vec![choose_video.clone().upcast()],
        color_rows: vec![color_row.clone().upcast()],
        effect_rows: vec![
            effect_row.clone().upcast(),
            colors_row.clone().upcast(),
            speed_row.clone().upcast(),
            sound_row.clone().upcast(),
            saver_row.clone().upcast(),
        ],
        message: crate::message(),
        thumbs: RefCell::default(),
    });
    page.show_kind(page.kind_now());
    page.show_rows();
    page.fill_swatches();

    let controls = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .hexpand(true)
        .build();
    if page.outputs.len() > 1 {
        let mut names = vec![tr("settings-all-monitors")];
        names.extend(page.outputs.iter().map(|o| o.name.clone()));
        let monitor = gtk::DropDown::from_strings(&names.iter().map(String::as_str).collect::<Vec<_>>());
        monitor.connect_selected_notify({
            let page = page.clone();
            move |dd| {
                let i = dd.selected() as usize;
                *page.target.borrow_mut() = i
                    .checked_sub(1)
                    .and_then(|i| page.outputs.get(i))
                    .map(|o| o.name.clone());
                page.show_kind(page.kind_now());
                page.load();
            }
        });
        controls.append(&crate::row(&tr("settings-monitor"), &monitor));
    }
    controls.append(&crate::row(&tr("settings-show"), &page.kind));
    controls.append(&effect_row);
    controls.append(&mode_row);
    controls.append(&color_row);
    controls.append(&colors_row);
    controls.append(&speed_row);
    controls.append(&sound_row);
    controls.append(&saver_row);
    controls.append(&choose);
    controls.append(&choose_video);
    controls.append(&page.message);
    fewer.connect_clicked({
        let page = page.clone();
        move |_| {
            let mut colors = page.colors();
            if colors.len() > 2 {
                colors.pop();
                page.set_colors(colors);
            }
        }
    });
    more.connect_clicked({
        let page = page.clone();
        move |_| {
            let mut colors = page.colors();
            if colors.len() < 5 {
                colors.push(colors.last().cloned().unwrap_or_else(|| "#4682b4".into()));
                page.set_colors(colors);
            }
        }
    });
    own.connect_clicked({
        let page = page.clone();
        move |_| page.set_colors(Vec::new())
    });
    page.speed.connect_value_changed({
        let page = page.clone();
        move |scale| {
            let speed = (scale.value() * 10.0).round() / 10.0;
            if (speed - page.paper.borrow().speed).abs() > 1e-6 {
                page.paper.borrow_mut().speed = speed;
                page.save(false);
            }
        }
    });

    sound.connect_active_notify({
        let page = page.clone();
        move |switch| {
            if switch.is_active() != page.paper.borrow().sound {
                page.paper.borrow_mut().sound = switch.is_active();
                page.save(false);
            }
        }
    });
    saver.connect_selected_notify({
        let page = page.clone();
        move |dd| {
            let Some(minutes) = SAVER_MINUTES.get(dd.selected() as usize) else {
                return;
            };
            if minutes * 60 != page.paper.borrow().screensaver {
                page.paper.borrow_mut().screensaver = minutes * 60;
                page.save(false);
            }
        }
    });

    let top = gtk::Box::builder().spacing(20).css_classes(["top"]).build();
    top.append(&page.preview);
    top.append(&controls);

    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .max_children_per_line(12)
        .row_spacing(10)
        .column_spacing(10)
        .valign(gtk::Align::Start)
        .css_classes(["thumbs"])
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .child(&grid)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();
    body.append(&top);
    body.append(
        &gtk::Label::builder()
            .label(tr("settings-pictures"))
            .xalign(0.0)
            .css_classes(["heading"])
            .build(),
    );
    body.append(&scroll);

    page.preview.set_draw_func({
        let page = Rc::downgrade(&page);
        move |_, cr, w, h| {
            if let Some(page) = page.upgrade() {
                page.draw(cr, w, h);
            }
        }
    });
    for dd in [&page.kind, &page.effect] {
        dd.connect_selected_notify({
            let page = page.clone();
            move |_| page.kind_chosen()
        });
    }
    page.mode.connect_selected_notify({
        let page = page.clone();
        move |dd| {
            let Some(mode) = MODES.get(dd.selected() as usize) else {
                return;
            };
            page.paper.borrow_mut().mode = (*mode).to_owned();
            page.save(false);
            page.preview.queue_draw();
        }
    });
    page.color.connect_rgba_notify({
        let page = page.clone();
        move |button| {
            let hex = hex_of(&button.rgba());
            if hex != page.paper.borrow().color {
                page.paper.borrow_mut().color = hex;
                page.save(false);
                page.preview.queue_draw();
            }
        }
    });
    choose_video.connect_clicked({
        let page = page.clone();
        move |button| page.choose_video(button)
    });
    choose.connect_clicked({
        let page = page.clone();
        move |button| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(&tr("settings-pictures")));
            filter.add_pixbuf_formats();
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder()
                .title(tr("settings-choose"))
                .filters(&filters)
                .modal(true)
                .build();
            let window = button.root().and_downcast::<gtk::Window>();
            let page = page.clone();
            dialog.open(window.as_ref(), None::<&gio::Cancellable>, move |file| {
                if let Some(path) = file.ok().and_then(|f| f.path()) {
                    page.set_image(short(&path));
                }
            });
        }
    });

    page.load();
    thumbnails(&page, &grid);
    body.upcast()
}

impl Page {
    /// "Show" or the effect changed: what they say now, written.
    fn kind_chosen(self: &Rc<Self>) {
        if self.syncing.get() {
            return;
        }
        let kind = self.chosen();
        self.show_rows();
        if kind == self.kind_now() {
            return;
        }
        if kind == VIDEO {
            self.choose_video(&self.kind);
            return;
        }
        if let Some(effect) = KINDS.get(kind).filter(|_| is_effect(kind)) {
            // An effect: for every monitor without a picture of its
            // own, so the one chosen loses its own.
            if let Some(name) = self.target.borrow().clone()
                && let Err(err) = core::drop_output_wallpaper(&self.dir, &name)
            {
                crate::say(&self.message, &format!("{err:#}"));
            }
            let mut paper = self.paper.borrow_mut();
            // Each effect's colours are its own: none carried over.
            paper.colors = match *effect {
                "fire-blue" => BLUE_FIRE.iter().map(|c| (*c).to_owned()).collect(),
                _ => Vec::new(),
            };
            paper.effect = effect.trim_end_matches("-blue").to_owned();
            paper.video.clear();
            drop(paper);
            self.save(false);
            self.fill_swatches();
            self.load();
            return;
        }
        let image = if kind == 1 {
            String::new()
        } else {
            // Back to a picture: the one there was, or the first shown.
            let first = self.thumbs.borrow().first().map(|(p, _)| p.clone());
            match (self.paper.borrow().image.clone(), first) {
                (old, _) if !old.is_empty() => old,
                (_, Some(path)) => short(&path),
                _ => return,
            }
        };
        self.set_image(image);
    }

    /// The index in `KINDS` the two lists say.
    fn chosen(&self) -> usize {
        match self.kind.selected() {
            0 => 0,
            1 => 1,
            2 => 2 + (self.effect.selected() as usize).min(VIDEO - 3),
            _ => VIDEO,
        }
    }

    /// The two lists set to `kind` (an index in `KINDS`).
    fn show_kind(&self, kind: usize) {
        self.syncing.set(true);
        if is_effect(kind) {
            self.effect.set_selected((kind - 2) as u32);
        }
        let show = match kind {
            0 | 1 => kind,
            k if k == VIDEO => 3,
            _ => 2,
        };
        self.kind.set_selected(show as u32);
        self.syncing.set(false);
        self.show_rows();
    }

    /// What "Show" says for the target now (an index of `KINDS`).
    fn kind_now(&self) -> usize {
        let own = self
            .target
            .borrow()
            .as_ref()
            .and_then(|name| core::output_wallpaper(&self.dir, name));
        let paper = self.paper.borrow();
        // A video is over every monitor, their own pictures too.
        if !paper.video.is_empty() {
            return VIDEO;
        }
        match own {
            Some(image) => usize::from(image.is_empty()),
            None => {
                let blue =
                    paper.effect == "fire" && paper.colors.iter().map(String::as_str).eq(BLUE_FIRE.iter().copied());
                let name = if blue { "fire-blue" } else { paper.effect.as_str() };
                match KINDS.iter().skip(2).position(|k| *k == name) {
                    Some(i) => i + 2,
                    None => usize::from(paper.image.is_empty()),
                }
            }
        }
    }

    fn show_rows(&self) {
        let kind = self.chosen();
        for row in &self.picture_rows {
            row.set_visible(kind == 0);
        }
        for row in &self.mode_rows {
            row.set_visible(kind == 0 || kind == VIDEO);
        }
        for row in &self.video_rows {
            row.set_visible(kind == VIDEO);
        }
        for row in &self.color_rows {
            row.set_visible(kind <= 1 || kind == VIDEO);
        }
        for row in &self.effect_rows {
            row.set_visible(is_effect(kind));
        }
    }

    /// The effect's colours: the file's, else its own.
    fn colors(&self) -> Vec<String> {
        let paper = self.paper.borrow();
        if paper.colors.len() >= 2 {
            return paper.colors.clone();
        }
        effect_colors(&paper.effect).iter().map(|c| (*c).to_owned()).collect()
    }

    /// `colors` written (empty: the effect's own), and the buttons made again.
    fn set_colors(self: &Rc<Self>, colors: Vec<String>) {
        self.paper.borrow_mut().colors = colors;
        self.save(false);
        self.fill_swatches();
        self.preview.queue_draw();
    }

    /// One button per colour of the effect.
    fn fill_swatches(self: &Rc<Self>) {
        while let Some(child) = self.swatches.first_child() {
            self.swatches.remove(&child);
        }
        for (i, hex) in self.colors().into_iter().enumerate() {
            let button = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
            if let Ok(rgba) = gdk::RGBA::parse(&hex) {
                button.set_rgba(&rgba);
            }
            button.connect_rgba_notify({
                let page = Rc::downgrade(self);
                move |button| {
                    let Some(page) = page.upgrade() else { return };
                    let mut colors = page.colors();
                    let hex = hex_of(&button.rgba());
                    if colors.get(i) != Some(&hex) && i < colors.len() {
                        colors[i] = hex;
                        page.paper.borrow_mut().colors = colors;
                        page.save(false);
                        page.preview.queue_draw();
                    }
                }
            });
            self.swatches.append(&button);
        }
    }

    /// A video asked for, then played over every monitor; nothing chosen,
    /// "Show" goes back to what there is.
    fn choose_video(self: &Rc<Self>, from: &impl IsA<gtk::Widget>) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(&tr("settings-videos")));
        filter.add_mime_type("video/*");
        filter.add_mime_type("image/gif");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title(tr("settings-choose-video"))
            .filters(&filters)
            .modal(true)
            .build();
        if let Some(videos) = glib::user_special_dir(glib::UserDirectory::Videos) {
            dialog.set_initial_folder(Some(&gio::File::for_path(videos)));
        }
        let window = from.root().and_downcast::<gtk::Window>();
        let page = self.clone();
        dialog.open(window.as_ref(), None::<&gio::Cancellable>, move |file| {
            if let Some(path) = file.ok().and_then(|f| f.path()) {
                page.paper.borrow_mut().video = short(&path);
                page.save(false);
            }
            page.show_kind(page.kind_now());
            page.show_rows();
            page.preview.queue_draw();
        });
    }

    /// The picture of the target (a monitor's own, or everyone's).
    fn image(&self) -> String {
        let global = self.paper.borrow().image.clone();
        match &*self.target.borrow() {
            Some(name) => core::output_wallpaper(&self.dir, name).unwrap_or(global),
            None => global,
        }
    }

    /// The size the target monitor shows, in pixels.
    fn monitor_size(&self) -> (i32, i32) {
        let target = self.target.borrow();
        let output = match &*target {
            Some(name) => self.outputs.iter().find(|o| &o.name == name),
            None => self.outputs.first(),
        };
        output
            .and_then(|o| {
                let m = o.current_mode?;
                Some(if o.transform.contains("90") || o.transform.contains("270") {
                    (m.height, m.width)
                } else {
                    (m.width, m.height)
                })
            })
            .unwrap_or((1920, 1080))
    }

    fn set_image(&self, image: String) {
        // Anything else chosen ends the video, which would cover it.
        if !self.paper.borrow().video.is_empty() {
            self.paper.borrow_mut().video.clear();
            self.save(false);
        }
        match self.target.borrow().clone() {
            Some(name) => {
                if let Err(err) = core::save_output(&self.dir, &name, |e| {
                    e.insert("wallpaper".into(), image.clone().into());
                }) {
                    crate::say(&self.message, &format!("{err:#}"));
                    return;
                }
                self.reload();
            }
            None => {
                // A picture (or the colour) for all: no effect over it.
                let mut paper = self.paper.borrow_mut();
                paper.image = image.clone();
                paper.effect = "none".into();
                drop(paper);
                self.save(true);
            }
        }
        self.show_kind(self.kind_now());
        self.load();
    }

    /// Writes `wallpaper:` (with `everywhere`, over the monitors' own
    /// pictures) and has the core read it.
    fn save(&self, everywhere: bool) {
        if let Err(err) = core::save_wallpaper(&self.dir, &self.paper.borrow(), everywhere) {
            crate::say(&self.message, &format!("{err:#}"));
            return;
        }
        self.reload();
    }

    fn reload(&self) {
        match core::command("reload") {
            Ok(()) => crate::say(&self.message, ""),
            Err(err) => crate::say(&self.message, &format!("{}: {err:#}", tr("settings-not-told"))),
        }
    }

    /// The target's picture into the preview, and the thumbnail in use marked.
    fn load(&self) {
        let image = self.image();
        let path = expand(&image);
        *self.picture.borrow_mut() = (!image.is_empty())
            .then(|| {
                let (_, w, h) = Pixbuf::file_info(&path)?;
                let pixbuf = Pixbuf::from_file_at_scale(&path, 1280, 1280, true).ok()?;
                Some((pixbuf, (w, h)))
            })
            .flatten();
        let (mw, mh) = self.monitor_size();
        self.preview
            .set_content_height((PREVIEW as f64 * mh as f64 / mw.max(1) as f64).round() as i32);
        for (file, button) in self.thumbs.borrow().iter() {
            if !image.is_empty() && *file == path {
                button.add_css_class("current");
            } else {
                button.remove_css_class("current");
            }
        }
        self.preview.queue_draw();
    }

    /// What the core will draw, at the preview's size.
    fn draw(&self, cr: &cairo::Context, w: i32, h: i32) {
        if self.kind_now() == VIDEO {
            // The colour around it, and the file's name.
            let paper = self.paper.borrow();
            if let Ok(c) = gdk::RGBA::parse(&paper.color) {
                cr.set_source_rgb(c.red().into(), c.green().into(), c.blue().into());
                let _ = cr.paint();
            }
            let name = Path::new(&paper.video)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            cr.set_source_rgb(0.9, 0.9, 0.9);
            cr.set_font_size(14.0);
            if let Ok(extents) = cr.text_extents(&name) {
                cr.move_to((w as f64 - extents.width()) / 2.0, h as f64 / 2.0);
                let _ = cr.show_text(&name);
            }
            return;
        }
        if is_effect(self.kind_now()) {
            // The effect's colours across, as it runs them.
            let gradient = cairo::LinearGradient::new(0.0, h as f64, w as f64, 0.0);
            let colors = self.colors();
            for (i, hex) in colors.iter().enumerate() {
                if let Ok(c) = gdk::RGBA::parse(hex) {
                    let at = i as f64 / (colors.len().max(2) - 1) as f64;
                    gradient.add_color_stop_rgb(at, c.red().into(), c.green().into(), c.blue().into());
                }
            }
            let _ = cr.set_source(&gradient);
            let _ = cr.paint();
            return;
        }
        let paper = self.paper.borrow();
        if let Ok(c) = gdk::RGBA::parse(&paper.color) {
            cr.set_source_rgb(c.red().into(), c.green().into(), c.blue().into());
            let _ = cr.paint();
        }
        let picture = self.picture.borrow();
        let Some((pixbuf, (pw, ph))) = picture.as_ref() else {
            return;
        };
        let (mw, mh) = self.monitor_size();
        let (mw, mh, pw, ph) = (mw as f64, mh as f64, *pw as f64, *ph as f64);
        // Preview pixels per monitor pixel, and monitor pixels per pixel
        // of the smaller picture.
        let s = w as f64 / mw;
        let k = pw / pixbuf.width() as f64;
        let (sx, sy) = match paper.mode.as_str() {
            "fit" => ((mw / pw).min(mh / ph), (mw / pw).min(mh / ph)),
            "center" | "tile" => (1.0, 1.0),
            "stretch" => (mw / pw, mh / ph),
            _ => ((mw / pw).max(mh / ph), (mw / pw).max(mh / ph)),
        };
        let (dw, dh) = (pw * sx * s, ph * sy * s);
        let (x, y) = if paper.mode == "tile" {
            (0.0, 0.0)
        } else {
            ((w as f64 - dw) / 2.0, (h as f64 - dh) / 2.0)
        };
        let _ = cr.save();
        cr.rectangle(0.0, 0.0, w as f64, h as f64);
        cr.clip();
        cr.translate(x, y);
        cr.scale(k * sx * s, k * sy * s);
        gdk::prelude::GdkCairoContextExt::set_source_pixbuf(cr, pixbuf, 0.0, 0.0);
        if paper.mode == "tile" {
            cr.source().set_extend(cairo::Extend::Repeat);
        }
        cr.source().set_filter(cairo::Filter::Good);
        let _ = cr.paint();
        let _ = cr.restore();
    }
}

fn hex_of(c: &gdk::RGBA) -> String {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(c.red()), byte(c.green()), byte(c.blue()))
}

/// The thumbnails, made one at a time while the loop is idle.
fn thumbnails(page: &Rc<Page>, grid: &gtk::FlowBox) {
    let mut files = Vec::new();
    let mut folders: Vec<PathBuf> = FOLDERS.iter().map(|f| expand(f)).collect();
    if let Some(pictures) = glib::user_special_dir(glib::UserDirectory::Pictures) {
        folders.insert(1, pictures.join("Wallpapers"));
        folders.insert(2, pictures);
    }
    for folder in folders {
        walk(&folder, 3, &mut files);
    }
    let current = expand(&page.image());
    if current.is_file() && !files.contains(&current) {
        files.insert(0, current);
    }
    files.truncate(MOST);
    let mut queue = files.into_iter();
    let (page, grid) = (Rc::downgrade(page), grid.clone());
    glib::idle_add_local(move || {
        let (Some(page), Some(path)) = (page.upgrade(), queue.next()) else {
            return glib::ControlFlow::Break;
        };
        let Some(pixbuf) = thumbnail(&path) else {
            return glib::ControlFlow::Continue;
        };
        #[allow(deprecated)]
        let texture = gdk::Texture::for_pixbuf(&pixbuf);
        let picture = gtk::Picture::builder()
            .paintable(&texture)
            .can_shrink(false)
            .width_request(THUMB.0)
            .height_request(THUMB.1)
            .build();
        let button = gtk::Button::builder()
            .child(&picture)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .tooltip_text(path.display().to_string())
            .css_classes(["thumb"])
            .build();
        if expand(&page.image()) == path {
            button.add_css_class("current");
        }
        button.connect_clicked({
            let (page, path) = (Rc::downgrade(&page), path.clone());
            move |_| {
                if let Some(page) = page.upgrade() {
                    page.set_image(short(&path));
                }
            }
        });
        grid.append(&button);
        page.thumbs.borrow_mut().push((path, button));
        glib::ControlFlow::Continue
    });
}

/// `path` made `THUMB` exactly, the overflow cut evenly (as `fill`).
fn thumbnail(path: &Path) -> Option<Pixbuf> {
    let (tw, th) = THUMB;
    let (_, w, h) = Pixbuf::file_info(path)?;
    let wider = w as i64 * th as i64 > h as i64 * tw as i64;
    let pixbuf = if wider {
        Pixbuf::from_file_at_scale(path, -1, th, true)
    } else {
        Pixbuf::from_file_at_scale(path, tw, -1, true)
    }
    .ok()?;
    let (pw, ph) = (pixbuf.width(), pixbuf.height());
    if pw < tw || ph < th {
        return Some(pixbuf);
    }
    Some(pixbuf.new_subpixbuf((pw - tw) / 2, (ph - th) / 2, tw, th))
}

/// The pictures in `dir` and its folders, `depth` deep, by name.
fn walk(dir: &Path, depth: usize, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if files.len() >= MOST {
            return;
        }
        if path.is_dir() {
            if depth > 0 {
                walk(&path, depth - 1, files);
            }
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
            && !files.contains(&path)
        {
            files.push(path);
        }
    }
}

fn home() -> PathBuf {
    glib::home_dir()
}

fn expand(path: &str) -> PathBuf {
    match path.strip_prefix('~') {
        Some(rest) => home().join(rest.trim_start_matches('/')),
        None => PathBuf::from(path),
    }
}

/// `path` with the home folder as `~`, as core.yaml is written by hand.
fn short(path: &Path) -> String {
    match path.strip_prefix(home()) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}
