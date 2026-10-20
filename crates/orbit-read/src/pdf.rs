//! A PDF: its pages one under another, scrolled through, drawn by poppler.
//!
//! Each page is a drawing area of the page's size at the zoom. GTK draws
//! every page, seen or not, so only the pages on screen (and their
//! neighbours) are rendered — the rest get a blank sheet, and are drawn
//! again when scrolled into view; rendered pages are kept for a while, so
//! scrolling back is free. Dragging over a page selects its text; clicking a link follows
//! it. The search goes through the pages a few at a time, in the idle
//! moments, so a long document never freezes the window.
//!
//! Next to the pages, a sidebar shows the outline, when the document has
//! one, and thumbnails of the pages.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    rc::Rc,
};

use gtk::{cairo, gdk, gio, glib, prelude::*};
use orbit_app::i18n::{number, tr, tr_args};

use crate::actions::{self, Target};

/// Pages kept rendered.
const CACHE: usize = 12;
/// Between pages, and around them.
const GAP: i32 = 10;
/// Thumbnails' width.
const THUMB: f64 = 110.0;
/// Pages searched per idle moment.
const SEARCH_STEP: usize = 4;

type Listener = Box<dyn Fn()>;
/// Thumbnails drawn blank, off screen, to draw again in view.
type Blanks = Rc<RefCell<Vec<(gtk::DrawingArea, Rc<Cell<bool>>)>>>;
/// Corners, in points from the page's top-left.
type Corners = (f64, f64, f64, f64);

struct Page {
    page: poppler::Page,
    /// In points.
    width: f64,
    height: f64,
    area: gtk::DrawingArea,
    links: RefCell<Option<Vec<actions::Link>>>,
    /// Drawn blank, off screen: to draw again when it comes into view.
    blank: Cell<bool>,
}

#[derive(Default)]
struct Search {
    query: String,
    /// Per page, the hits as (x1, y1, x2, y2) points from the top-left.
    hits: HashMap<usize, Vec<(f64, f64, f64, f64)>>,
    /// Pages searched so far.
    searched: usize,
    /// The hit shown: page and index.
    current: Option<(usize, usize)>,
    running: Option<glib::SourceId>,
}

pub struct PdfView {
    pub widget: gtk::Paned,
    scrolled: gtk::ScrolledWindow,
    pages_box: gtk::Box,
    sidebar: gtk::Box,
    document: poppler::Document,
    pages: Vec<Page>,
    zoom: Cell<f64>,
    fit_width: Cell<bool>,
    cache: RefCell<VecDeque<(usize, f64, cairo::ImageSurface)>>,
    search: RefCell<Search>,
    /// The selection: page, and the corners in points.
    selection: RefCell<Option<(usize, Corners)>>,
    drag_from: Cell<(f64, f64)>,
    on_change: RefCell<Option<Listener>>,
}

impl PdfView {
    pub fn open(path: &std::path::Path, password: Option<&str>) -> Result<Rc<Self>, glib::Error> {
        let uri = gio::File::for_path(path).uri();
        let document = poppler::Document::from_file(&uri, password)?;
        let pages_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(GAP)
            .halign(gtk::Align::Center)
            .margin_top(GAP)
            .margin_bottom(GAP)
            .css_classes(["pages"])
            .build();
        let mut pages = Vec::new();
        for i in 0..document.n_pages() {
            let Some(page) = document.page(i) else { continue };
            let (width, height) = page.size();
            let area = gtk::DrawingArea::builder()
                .css_classes(["page"])
                .halign(gtk::Align::Center)
                .build();
            pages_box.append(&area);
            pages.push(Page {
                page,
                width,
                height,
                area,
                links: RefCell::default(),
                blank: Cell::new(false),
            });
        }
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&pages_box)
            .hexpand(true)
            .vexpand(true)
            .css_classes(["document"])
            .build();
        let sidebar = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(170)
            .css_classes(["sidebar"])
            .build();
        let widget = gtk::Paned::builder()
            .start_child(&sidebar)
            .end_child(&scrolled)
            .shrink_start_child(false)
            .resize_start_child(false)
            .position(200)
            .build();
        sidebar.set_visible(false);

        let view = Rc::new(Self {
            widget,
            scrolled,
            pages_box,
            sidebar,
            document,
            pages,
            zoom: Cell::new(1.0),
            fit_width: Cell::new(true),
            cache: RefCell::default(),
            search: RefCell::default(),
            selection: RefCell::default(),
            drag_from: Cell::new((0.0, 0.0)),
            on_change: RefCell::default(),
        });
        view.connect();
        view.build_sidebar();
        view.apply_zoom();
        Ok(view)
    }

    /// Called when the page shown or the zoom changes.
    pub fn connect_change(&self, f: impl Fn() + 'static) {
        *self.on_change.borrow_mut() = Some(Box::new(f));
    }

    fn changed(&self) {
        if let Some(f) = self.on_change.borrow().as_ref() {
            f();
        }
    }

    pub fn title(&self) -> Option<String> {
        self.document
            .title()
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
    }

    pub fn n_pages(&self) -> usize {
        self.pages.len()
    }

    // ---------------------------------------------------------- drawing

    fn connect(self: &Rc<Self>) {
        for (i, page) in self.pages.iter().enumerate() {
            let weak = Rc::downgrade(self);
            page.area.set_draw_func(move |area, cr, _, _| {
                if let Some(v) = weak.upgrade() {
                    v.draw(i, area, cr);
                }
            });

            // Dragging selects text; a click follows a link.
            let drag = gtk::GestureDrag::new();
            let weak = Rc::downgrade(self);
            drag.connect_drag_begin(move |_, x, y| {
                if let Some(v) = weak.upgrade() {
                    v.drag_from.set((x, y));
                    *v.selection.borrow_mut() = None;
                    v.redraw_all();
                }
            });
            let weak = Rc::downgrade(self);
            drag.connect_drag_update(move |_, dx, dy| {
                let Some(v) = weak.upgrade() else { return };
                let (x, y) = v.drag_from.get();
                let z = v.zoom.get();
                *v.selection.borrow_mut() =
                    Some((i, (x / z, y / z, (x + dx) / z, (y + dy) / z)));
                v.pages[i].area.queue_draw();
            });
            let weak = Rc::downgrade(self);
            drag.connect_drag_end(move |_, dx, dy| {
                let Some(v) = weak.upgrade() else { return };
                if dx.abs() < 3.0 && dy.abs() < 3.0 {
                    *v.selection.borrow_mut() = None;
                    let (x, y) = v.drag_from.get();
                    if let Some(target) = v.link_at(i, x, y) {
                        v.follow(&target);
                    }
                    v.pages[i].area.queue_draw();
                }
            });
            page.area.add_controller(drag);

            // The hand over links.
            let motion = gtk::EventControllerMotion::new();
            let weak = Rc::downgrade(self);
            motion.connect_motion(move |m, x, y| {
                let (Some(v), Some(area)) = (weak.upgrade(), m.widget()) else { return };
                let over = v.link_at(i, x, y).is_some();
                area.set_cursor_from_name(Some(if over { "pointer" } else { "text" }));
            });
            page.area.add_controller(motion);
        }

        // Ctrl + wheel zooms.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        scroll.connect_scroll(move |c, _, dy| {
            let Some(v) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if !c.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                return glib::Propagation::Proceed;
            }
            v.zoom_by(if dy < 0.0 { 1.1 } else { 1.0 / 1.1 });
            glib::Propagation::Stop
        });
        self.scrolled.add_controller(scroll);

        // Fitted to the width, the zoom follows the window — after the
        // layout that changed the width, not inside it; scrolling changes
        // the page shown.
        let weak = Rc::downgrade(self);
        self.scrolled.hadjustment().connect_page_size_notify(move |_| {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(v) = weak.upgrade()
                    && v.fit_width.get()
                {
                    v.apply_zoom();
                }
            });
        });
        let weak = Rc::downgrade(self);
        self.scrolled.vadjustment().connect_value_changed(move |_| {
            if let Some(v) = weak.upgrade() {
                v.uncover();
                v.changed();
            }
        });
        let weak = Rc::downgrade(self);
        self.scrolled.vadjustment().connect_page_size_notify(move |_| {
            if let Some(v) = weak.upgrade() {
                v.uncover();
            }
        });
    }

    /// Where each page starts, down the pages, in pixels.
    fn tops(&self) -> impl Iterator<Item = f64> + '_ {
        let zoom = self.zoom.get();
        self.pages.iter().scan(f64::from(GAP), move |y, page| {
            let top = *y;
            *y += (page.height * zoom).round() + f64::from(GAP);
            Some(top)
        })
    }

    /// The pages on screen, with one more above and below.
    fn on_screen(&self) -> std::ops::Range<usize> {
        let vadj = self.scrolled.vadjustment();
        let (from, to) = (vadj.value(), vadj.value() + vadj.page_size().max(1.0));
        let zoom = self.zoom.get();
        let mut first = None;
        let mut last = 0;
        for (i, top) in self.tops().enumerate() {
            let bottom = top + self.pages[i].height * zoom;
            if bottom >= from && top <= to {
                first.get_or_insert(i);
                last = i;
            } else if top > to {
                break;
            }
        }
        let first = first.unwrap_or(0);
        first.saturating_sub(1)..(last + 2).min(self.pages.len())
    }

    /// Draws the pages that came into view blank.
    fn uncover(&self) {
        for i in self.on_screen() {
            if self.pages[i].blank.replace(false) {
                self.pages[i].area.queue_draw();
            }
        }
    }

    fn draw(&self, i: usize, area: &gtk::DrawingArea, cr: &cairo::Context) {
        let page = &self.pages[i];
        if !self.on_screen().contains(&i) {
            page.blank.set(true);
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.paint().ok();
            return;
        }
        let zoom = self.zoom.get();
        let scale = f64::from(area.scale_factor());
        let Some(surface) = self.rendered(i, zoom, scale) else {
            return;
        };
        cr.save().ok();
        cr.scale(1.0 / scale, 1.0 / scale);
        cr.set_source_surface(&surface, 0.0, 0.0).ok();
        cr.paint().ok();
        cr.restore().ok();

        // Search hits, the current one stronger.
        let search = self.search.borrow();
        if let Some(hits) = search.hits.get(&i) {
            for (n, (x1, y1, x2, y2)) in hits.iter().enumerate() {
                let current = search.current == Some((i, n));
                cr.set_source_rgba(1.0, if current { 0.55 } else { 0.85 }, 0.0, if current { 0.5 } else { 0.3 });
                cr.rectangle(x1 * zoom, y1 * zoom, (x2 - x1) * zoom, (y2 - y1) * zoom);
                cr.fill().ok();
            }
        }
        drop(search);

        // The selection, over the glyphs it takes.
        if let Some((p, (x1, y1, x2, y2))) = *self.selection.borrow()
            && p == i
        {
            let mut rect = poppler::Rectangle::new();
            rect.set_x1(x1.min(x2));
            rect.set_y1(y1.min(y2));
            rect.set_x2(x1.max(x2));
            rect.set_y2(y1.max(y2));
            if let Some(region) =
                page.page
                    .selected_region(zoom, poppler::SelectionStyle::Glyph, &mut rect)
            {
                let accent = orbit_app::theme::accent();
                cr.set_source_rgba(
                    accent.red().into(),
                    accent.green().into(),
                    accent.blue().into(),
                    0.35,
                );
                for n in 0..region.num_rectangles() {
                    let r = region.rectangle(n);
                    cr.rectangle(f64::from(r.x()), f64::from(r.y()), f64::from(r.width()), f64::from(r.height()));
                }
                cr.fill().ok();
            }
        }
    }

    /// Page `i` rendered at `zoom` for a screen of `scale`, from the cache
    /// or now.
    fn rendered(&self, i: usize, zoom: f64, scale: f64) -> Option<cairo::ImageSurface> {
        let mut cache = self.cache.borrow_mut();
        if let Some(pos) = cache.iter().position(|(p, z, _)| *p == i && *z == zoom * scale) {
            let entry = cache.remove(pos)?;
            let surface = entry.2.clone();
            cache.push_back(entry);
            return Some(surface);
        }
        let page = &self.pages[i];
        let k = zoom * scale;
        log::debug!("rendering page {} at {:.2}", i + 1, k);
        let surface = cairo::ImageSurface::create(
            cairo::Format::Rgb24,
            (page.width * k).ceil() as i32,
            (page.height * k).ceil() as i32,
        )
        .ok()?;
        {
            let cr = cairo::Context::new(&surface).ok()?;
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.paint().ok();
            cr.scale(k, k);
            page.page.render(&cr);
        }
        cache.push_back((i, k, surface.clone()));
        while cache.len() > CACHE {
            cache.pop_front();
        }
        Some(surface)
    }

    fn redraw_all(&self) {
        for page in &self.pages {
            page.area.queue_draw();
        }
    }

    // ------------------------------------------------------------- zoom

    pub fn zoom(&self) -> f64 {
        self.zoom.get()
    }

    pub fn zoom_by(&self, factor: f64) {
        self.fit_width.set(false);
        self.set_zoom((self.zoom.get() * factor).clamp(0.1, 8.0));
    }

    pub fn fit_width(&self) {
        self.fit_width.set(true);
        self.apply_zoom();
    }

    fn apply_zoom(&self) {
        if self.fit_width.get() {
            let width = self.scrolled.hadjustment().page_size();
            let widest = self.pages.iter().map(|p| p.width).fold(1.0, f64::max);
            if width > 0.0 {
                self.set_zoom(((width - f64::from(2 * GAP) - 20.0) / widest).clamp(0.1, 8.0));
                return;
            }
        }
        self.set_zoom(self.zoom.get());
    }

    fn set_zoom(&self, zoom: f64) {
        let vadj = self.scrolled.vadjustment();
        // Keep the same place in the document.
        let fraction = if vadj.upper() > 0.0 {
            vadj.value() / vadj.upper()
        } else {
            0.0
        };
        let changed = (zoom - self.zoom.get()).abs() > 1e-6;
        self.zoom.set(zoom);
        for page in &self.pages {
            page.area.set_content_width((page.width * zoom).round() as i32);
            page.area.set_content_height((page.height * zoom).round() as i32);
        }
        if changed {
            self.cache.borrow_mut().clear();
            let vadj = vadj.clone();
            glib::idle_add_local_once(move || vadj.set_value(fraction * vadj.upper()));
        }
        self.changed();
    }

    // ------------------------------------------------------------ pages

    /// The page at the top third of the view.
    pub fn current_page(&self) -> usize {
        let vadj = self.scrolled.vadjustment();
        let y = vadj.value() + vadj.page_size() / 3.0;
        self.tops().take_while(|top| *top <= y).count().saturating_sub(1)
    }

    /// Scrolls to page `i`, `top` points down it.
    pub fn go_to(&self, i: usize, top: Option<f64>) {
        let Some(page_top) = self.tops().nth(i) else { return };
        let vadj = self.scrolled.vadjustment();
        let y = page_top + top.unwrap_or(0.0) * self.zoom.get() - f64::from(GAP);
        let place = move || vadj.set_value(y.max(0.0));
        // Right after opening, the pages are not laid out yet.
        if self.pages_box.height() == 0 {
            glib::idle_add_local_once(place);
        } else {
            place();
        }
    }

    fn link_at(&self, i: usize, x: f64, y: f64) -> Option<Target> {
        let page = &self.pages[i];
        let zoom = self.zoom.get();
        let (px, py) = (x / zoom, y / zoom);
        let mut links = page.links.borrow_mut();
        let links = links.get_or_insert_with(|| actions::links(&self.document, &page.page, page.height));
        links
            .iter()
            .find(|l| (l.area.0..=l.area.2).contains(&px) && (l.area.1..=l.area.3).contains(&py))
            .map(|l| l.target.clone())
    }

    fn follow(&self, target: &Target) {
        match target {
            Target::Page(i, top) => self.go_to(*i, *top),
            Target::Uri(uri) => {
                if let Err(err) = gio::AppInfo::launch_default_for_uri(uri, None::<&gio::AppLaunchContext>) {
                    log::warn!("opening {uri}: {err}");
                }
            }
        }
    }

    // ----------------------------------------------------------- sidebar

    pub fn show_sidebar(&self, shown: bool) {
        self.sidebar.set_visible(shown);
    }

    fn build_sidebar(self: &Rc<Self>) {
        let stack = gtk::Stack::builder().vexpand(true).build();
        let outline = actions::outline(&self.document);
        if !outline.is_empty() {
            let list = gtk::ListBox::builder().css_classes(["outline"]).build();
            for entry in &outline {
                let label = gtk::Label::builder()
                    .label(entry.title.as_str())
                    .xalign(0.0)
                    .wrap(true)
                    .margin_start(10 * entry.depth as i32)
                    .build();
                list.append(&label);
            }
            let targets: Vec<Option<Target>> = outline.into_iter().map(|e| e.target).collect();
            let weak = Rc::downgrade(self);
            list.connect_row_activated(move |_, row| {
                if let (Some(v), Some(Some(target))) = (weak.upgrade(), targets.get(row.index() as usize)) {
                    v.follow(target);
                }
            });
            let scroll = gtk::ScrolledWindow::builder()
                .child(&list)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .build();
            stack.add_titled(&scroll, Some("outline"), &tr("read-outline"));
        }

        let thumbs = gtk::ListBox::builder().css_classes(["thumbs"]).build();
        let scroll = gtk::ScrolledWindow::builder()
            .child(&thumbs)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        // As with the pages: only those in view are rendered.
        let blanks: Blanks = Rc::default();
        for (i, page) in self.pages.iter().enumerate() {
            let k = THUMB / page.width;
            let area = gtk::DrawingArea::builder()
                .content_width(THUMB as i32)
                .content_height((page.height * k).round() as i32)
                .halign(gtk::Align::Center)
                .css_classes(["thumb"])
                .build();
            let cached: RefCell<Option<cairo::ImageSurface>> = RefCell::default();
            let source = page.page.clone();
            let blank = Rc::new(Cell::new(false));
            blanks.borrow_mut().push((area.clone(), blank.clone()));
            let viewport = scroll.downgrade();
            area.set_draw_func(move |area, cr, w, h| {
                let scale = area.scale_factor();
                let mut cached = cached.borrow_mut();
                if cached.is_none() {
                    if !viewport.upgrade().is_some_and(|s| in_view(area, &s)) {
                        blank.set(true);
                        cr.set_source_rgb(1.0, 1.0, 1.0);
                        cr.paint().ok();
                        return;
                    }
                    *cached = thumbnail(&source, w * scale, h * scale);
                }
                if let Some(surface) = cached.as_ref() {
                    cr.scale(1.0 / f64::from(scale), 1.0 / f64::from(scale));
                    cr.set_source_surface(surface, 0.0, 0.0).ok();
                    cr.paint().ok();
                }
            });
            let item = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(5)
                .build();
            item.append(&area);
            item.append(&gtk::Label::builder().label((i + 1).to_string()).css_classes(["number"]).build());
            thumbs.append(&item);
        }
        let weak = Rc::downgrade(self);
        thumbs.connect_row_activated(move |_, row| {
            if let Some(v) = weak.upgrade() {
                v.go_to(row.index() as usize, None);
            }
        });
        let uncover = {
            let scroll = scroll.downgrade();
            move || {
                let Some(scroll) = scroll.upgrade() else { return };
                for (area, blank) in blanks.borrow().iter() {
                    if blank.get() && in_view(area, &scroll) {
                        blank.set(false);
                        area.queue_draw();
                    }
                }
            }
        };
        let again = uncover.clone();
        scroll.vadjustment().connect_value_changed(move |_| again());
        scroll.vadjustment().connect_page_size_notify(move |_| uncover());
        stack.add_titled(&scroll, Some("pages"), &tr("read-pages"));

        if stack.pages().n_items() > 1 {
            let switcher = gtk::StackSwitcher::builder()
                .stack(&stack)
                .halign(gtk::Align::Center)
                .css_classes(["switcher"])
                .build();
            self.sidebar.append(&switcher);
        }
        self.sidebar.append(&stack);
    }

    // ------------------------------------------------------------ search

    /// Starts searching for `query`, from the page shown.
    pub fn search(self: &Rc<Self>, query: &str, on_progress: impl Fn(String) + 'static) {
        if let Some(id) = self.search.borrow_mut().running.take() {
            id.remove();
        }
        *self.search.borrow_mut() = Search {
            query: query.to_owned(),
            ..Search::default()
        };
        self.redraw_all();
        if query.trim().is_empty() {
            on_progress(String::new());
            return;
        }
        let start = self.current_page();
        let n = self.pages.len();
        let weak = Rc::downgrade(self);
        let id = glib::idle_add_local(move || {
            let Some(v) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let mut search = v.search.borrow_mut();
            for _ in 0..SEARCH_STEP {
                if search.searched >= n {
                    break;
                }
                let i = (start + search.searched) % n;
                let page = &v.pages[i];
                let mut hits: Vec<(f64, f64, f64, f64)> = page
                    .page
                    .find_text(&search.query)
                    .iter()
                    // Counted from the page's bottom; flipped to its top.
                    .map(|r| (r.x1(), page.height - r.y2(), r.x2(), page.height - r.y1()))
                    .collect();
                hits.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)));
                if !hits.is_empty() {
                    search.hits.insert(i, hits);
                    page.area.queue_draw();
                    if search.current.is_none() {
                        search.current = Some((i, 0));
                        let top = search.hits[&i][0].1;
                        drop(search);
                        v.go_to(i, Some((top - 40.0).max(0.0)));
                        search = v.search.borrow_mut();
                    }
                }
                search.searched += 1;
            }
            let total: usize = search.hits.values().map(Vec::len).sum();
            let done = search.searched >= n;
            let position = search.current.map(|c| v.ordinal(&search, c));
            drop(search);
            on_progress(counter(position, total, done));
            if done {
                v.search.borrow_mut().running = None;
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        self.search.borrow_mut().running = Some(id);
    }

    /// Goes to the next hit (or the previous one); returns the counter.
    pub fn search_step(&self, forward: bool) -> String {
        let mut search = self.search.borrow_mut();
        let mut order: Vec<(usize, usize)> = search
            .hits
            .iter()
            .flat_map(|(p, hits)| (0..hits.len()).map(move |n| (*p, n)))
            .collect();
        order.sort();
        if order.is_empty() {
            return counter(None, 0, search.running.is_none());
        }
        let at = search
            .current
            .and_then(|c| order.iter().position(|o| *o == c))
            .unwrap_or(0);
        let next = if forward {
            (at + 1) % order.len()
        } else {
            (at + order.len() - 1) % order.len()
        };
        let (page, n) = order[next];
        search.current = Some((page, n));
        let top = search.hits[&page][n].1;
        let total = order.len();
        let done = search.running.is_none();
        drop(search);
        self.redraw_all();
        self.go_to(page, Some((top - 40.0).max(0.0)));
        counter(Some(next + 1), total, done)
    }

    /// The position of hit `c` among all found so far, from 1.
    fn ordinal(&self, search: &Search, c: (usize, usize)) -> usize {
        search
            .hits
            .iter()
            .filter(|(p, _)| **p < c.0)
            .map(|(_, h)| h.len())
            .sum::<usize>()
            + c.1
            + 1
    }

    // ------------------------------------------------------- copy, print

    /// The selected text, if any.
    pub fn selected_text(&self) -> Option<String> {
        let (i, (x1, y1, x2, y2)) = (*self.selection.borrow())?;
        let mut rect = poppler::Rectangle::new();
        rect.set_x1(x1.min(x2));
        rect.set_y1(y1.min(y2));
        rect.set_x2(x1.max(x2));
        rect.set_y2(y1.max(y2));
        self.pages[i]
            .page
            .selected_text(poppler::SelectionStyle::Glyph, &mut rect)
            .map(|t| t.to_string())
            .filter(|t| !t.is_empty())
    }

    pub fn print(&self, window: &gtk::Window) {
        let operation = gtk::PrintOperation::builder()
            .n_pages(self.pages.len() as i32)
            .use_full_page(false)
            .unit(gtk::Unit::Points)
            .build();
        if let Some(title) = self.title() {
            operation.set_job_name(&title);
        }
        let pages: Vec<poppler::Page> = self.pages.iter().map(|p| p.page.clone()).collect();
        operation.connect_draw_page(move |_, context, n| {
            let Some(page) = pages.get(n as usize) else { return };
            let (w, h) = page.size();
            let cr = context.cairo_context();
            let k = (context.width() / w).min(context.height() / h);
            cr.scale(k, k);
            page.render_for_printing(&cr);
        });
        if let Err(err) = operation.run(gtk::PrintOperationAction::PrintDialog, Some(window)) {
            log::warn!("printing: {err}");
        }
    }
}

/// Whether `area` is within `scroll`'s view, or near it.
fn in_view(area: &gtk::DrawingArea, scroll: &gtk::ScrolledWindow) -> bool {
    area.compute_bounds(scroll).is_some_and(|b| {
        let (top, bottom) = (f64::from(b.y()), f64::from(b.y() + b.height()));
        bottom >= -200.0 && top <= f64::from(scroll.height()) + 200.0
    })
}

/// "3 de 27", "27 encontrados…", "nada encontrado".
pub fn counter(position: Option<usize>, total: usize, done: bool) -> String {
    match (position, total) {
        (_, 0) if done => tr("read-search-none"),
        (_, 0) => tr("read-search-running"),
        (Some(p), t) => tr_args(
            if done { "read-search-position" } else { "read-search-position-running" },
            &[("position", number(p as f64)), ("total", number(t as f64))],
        ),
        (None, t) => tr_args("read-search-count", &[("total", number(t as f64))]),
    }
}

fn thumbnail(page: &poppler::Page, w: i32, h: i32) -> Option<cairo::ImageSurface> {
    let surface = cairo::ImageSurface::create(cairo::Format::Rgb24, w.max(1), h.max(1)).ok()?;
    let cr = cairo::Context::new(&surface).ok()?;
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.paint().ok();
    let (pw, _) = page.size();
    let k = f64::from(w) / pw;
    cr.scale(k, k);
    page.render(&cr);
    drop(cr);
    Some(surface)
}
