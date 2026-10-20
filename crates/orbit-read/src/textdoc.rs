//! Markdown and plain text. Markdown is shown laid out, with its source a
//! button away; plain text in a monospaced font. Both are read-only text
//! views: selecting and copying are GTK's own. The text keeps to a
//! comfortable width, centred, however wide the window.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{gdk, gio, glib, prelude::*};

use crate::markdown::{self, Rendered};

/// The widest the text runs, in pixels.
const MEASURE: f64 = 800.0;
/// The font size at 100%, in points.
const BASE_SIZE: f64 = 11.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Markdown,
    Text,
}

type Opener = Box<dyn Fn(&Path)>;

pub struct TextDoc {
    pub widget: gtk::Stack,
    pub kind: Kind,
    /// Markdown laid out (for plain text, unused).
    rendered: gtk::TextView,
    /// The source: Markdown's, or the plain text.
    source: gtk::TextView,
    rendered_scroll: gtk::ScrolledWindow,
    links: RefCell<Rendered>,
    base: PathBuf,
    scale: Cell<f64>,
    css: gtk::CssProvider,
    /// The matches of the search, and which one is shown.
    matches: RefCell<Vec<(gtk::TextMark, gtk::TextMark)>>,
    current: Cell<Option<usize>>,
    on_open: RefCell<Option<Opener>>,
}

impl TextDoc {
    pub fn open(path: &Path, kind: Kind) -> std::io::Result<Rc<Self>> {
        let text = read_text(path)?;
        let base = path.parent().map(Path::to_path_buf).unwrap_or_default();

        let view = |classes: &[&str]| {
            gtk::TextView::builder()
                .editable(false)
                .cursor_visible(false)
                .wrap_mode(gtk::WrapMode::WordChar)
                .top_margin(20)
                .bottom_margin(40)
                .left_margin(20)
                .right_margin(20)
                .css_classes(classes)
                .build()
        };
        let rendered = view(&["doc", "rendered"]);
        let source = view(&["doc", "source"]);
        source.buffer().set_text(&text);
        let scroll = |child: &gtk::TextView| {
            gtk::ScrolledWindow::builder()
                .child(child)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .hexpand(true)
                .vexpand(true)
                .build()
        };
        let rendered_scroll = scroll(&rendered);
        let source_scroll = scroll(&source);
        let widget = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .css_classes(["document"])
            .build();
        widget.add_named(&rendered_scroll, Some("rendered"));
        widget.add_named(&source_scroll, Some("source"));

        let links = if kind == Kind::Markdown {
            markdown::render(&rendered, &text, &base)
        } else {
            Rendered::default()
        };
        widget.set_visible_child_name(if kind == Kind::Markdown { "rendered" } else { "source" });

        let css = gtk::CssProvider::new();
        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().expect("a display"),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
        let doc = Rc::new(Self {
            widget,
            kind,
            rendered,
            source,
            rendered_scroll,
            links: RefCell::new(links),
            base,
            scale: Cell::new(1.0),
            css,
            matches: RefCell::default(),
            current: Cell::new(None),
            on_open: RefCell::default(),
        });
        doc.apply_scale();
        doc.connect(&source_scroll);
        Ok(doc)
    }

    /// Called with a document a link leads to (another Markdown file…).
    pub fn connect_open(&self, f: impl Fn(&Path) + 'static) {
        *self.on_open.borrow_mut() = Some(Box::new(f));
    }

    fn connect(self: &Rc<Self>, source_scroll: &gtk::ScrolledWindow) {
        // A comfortable measure, centred — set after the layout that
        // changed the width: changing margins inside it trips GTK's text
        // view.
        for (scroll, view) in [
            (self.rendered_scroll.clone(), self.rendered.clone()),
            (source_scroll.clone(), self.source.clone()),
        ] {
            scroll.hadjustment().connect_page_size_notify(move |adj| {
                let margin = ((adj.page_size() - MEASURE) / 2.0).max(20.0) as i32;
                if margin == view.left_margin() {
                    return;
                }
                let view = view.clone();
                glib::idle_add_local_once(move || {
                    view.set_left_margin(margin);
                    view.set_right_margin(margin);
                    markdown::indent(&view.buffer(), margin);
                });
            });
        }

        // Links: the hand over them, a click follows them.
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            if let Some(d) = weak.upgrade() {
                let over = d.link_at(x, y).is_some();
                d.rendered.set_cursor_from_name(Some(if over { "pointer" } else { "text" }));
            }
        });
        self.rendered.add_controller(motion);
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(self);
        click.connect_released(move |_, _, x, y| {
            let Some(d) = weak.upgrade() else { return };
            // Not at the end of a selection.
            if d.rendered.buffer().has_selection() {
                return;
            }
            if let Some(url) = d.link_at(x, y) {
                d.follow(&url);
            }
        });
        self.rendered.add_controller(click);

        // Ctrl + wheel changes the size.
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        wheel.connect_scroll(move |c, _, dy| {
            let Some(d) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if !c.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                return glib::Propagation::Proceed;
            }
            d.zoom_by(if dy < 0.0 { 1.1 } else { 1.0 / 1.1 });
            glib::Propagation::Stop
        });
        self.widget.add_controller(wheel);
    }

    fn link_at(&self, x: f64, y: f64) -> Option<String> {
        let (bx, by) = self
            .rendered
            .window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let iter = self.rendered.iter_at_location(bx, by)?;
        let links = self.links.borrow();
        iter.tags().iter().find_map(|t| links.links.get(t).cloned())
    }

    fn follow(&self, url: &str) {
        if let Some(anchor) = url.strip_prefix('#') {
            let wanted = markdown::percent_decode(anchor).to_lowercase();
            if let Some(mark) = self.links.borrow().anchors.get(&wanted) {
                self.rendered.scroll_to_mark(mark, 0.0, true, 0.0, 0.0);
            }
            return;
        }
        if !url.contains("://") && !url.starts_with("mailto:") {
            // A file next to this one: a document opens here.
            let path = self.base.join(markdown::percent_decode(url.split('#').next().unwrap_or(url)));
            if crate::kind_of(&path).is_some()
                && let Some(f) = self.on_open.borrow().as_ref()
            {
                f(&path);
                return;
            }
            let uri = gio::File::for_path(&path).uri();
            launch(&uri);
            return;
        }
        launch(url);
    }

    // -------------------------------------------------------- source, size

    pub fn showing_source(&self) -> bool {
        self.widget.visible_child_name().as_deref() == Some("source")
    }

    pub fn show_source(&self, shown: bool) {
        if self.kind == Kind::Markdown {
            self.widget
                .set_visible_child_name(if shown { "source" } else { "rendered" });
        }
    }

    pub fn scale(&self) -> f64 {
        self.scale.get()
    }

    pub fn zoom_by(&self, factor: f64) {
        self.scale.set((self.scale.get() * factor).clamp(0.5, 4.0));
        self.apply_scale();
    }

    pub fn reset_zoom(&self) {
        self.scale.set(1.0);
        self.apply_scale();
    }

    fn apply_scale(&self) {
        let size = BASE_SIZE * self.scale.get();
        self.css.load_from_string(&format!(
            "window.orbit-read textview.doc {{ font-size: {size:.1}pt; }}"
        ));
    }

    fn active(&self) -> &gtk::TextView {
        if self.showing_source() {
            &self.source
        } else {
            &self.rendered
        }
    }

    // ------------------------------------------------------------- search

    /// Marks every match of `query` and shows the first after the top of
    /// the view; returns the counter.
    pub fn search(&self, query: &str) -> String {
        let view = self.active().clone();
        let buffer = view.buffer();
        let (start, end) = buffer.bounds();
        for buf in [self.rendered.buffer(), self.source.buffer()] {
            ensure_search_tags(&buf);
            let (s, e) = buf.bounds();
            buf.remove_tag_by_name("match", &s, &e);
            buf.remove_tag_by_name("current", &s, &e);
        }
        self.matches.borrow_mut().clear();
        self.current.set(None);
        if query.is_empty() {
            return String::new();
        }
        ensure_search_tags(&buffer);
        let mut found = Vec::new();
        let mut at = start;
        while let Some((s, e)) = at.forward_search(
            query,
            gtk::TextSearchFlags::CASE_INSENSITIVE | gtk::TextSearchFlags::TEXT_ONLY,
            Some(&end),
        ) {
            buffer.apply_tag_by_name("match", &s, &e);
            found.push((
                buffer.create_mark(None, &s, true),
                buffer.create_mark(None, &e, false),
            ));
            at = e;
        }
        // The first at or below what is on screen.
        let top = view.visible_rect().y();
        let first = found
            .iter()
            .position(|(s, _)| view.iter_location(&buffer.iter_at_mark(s)).y() >= top)
            .unwrap_or(0);
        *self.matches.borrow_mut() = found;
        self.show_match(first)
    }

    pub fn search_step(&self, forward: bool) -> String {
        let n = self.matches.borrow().len();
        if n == 0 {
            return crate::pdf::counter(None, 0, true);
        }
        let at = self.current.get().unwrap_or(0);
        self.show_match(if forward { (at + 1) % n } else { (at + n - 1) % n })
    }

    fn show_match(&self, index: usize) -> String {
        let matches = self.matches.borrow();
        let total = matches.len();
        if total == 0 {
            return crate::pdf::counter(None, 0, true);
        }
        let view = self.active();
        let buffer = view.buffer();
        let (s, e) = buffer.bounds();
        buffer.remove_tag_by_name("current", &s, &e);
        let (start, end) = &matches[index];
        let (start, end) = (buffer.iter_at_mark(start), buffer.iter_at_mark(end));
        buffer.apply_tag_by_name("current", &start, &end);
        view.scroll_to_iter(&mut start.clone(), 0.1, true, 0.0, 0.3);
        self.current.set(Some(index));
        crate::pdf::counter(Some(index + 1), total, true)
    }
}

impl Drop for TextDoc {
    fn drop(&mut self) {
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_remove_provider_for_display(&display, &self.css);
        }
    }
}

fn ensure_search_tags(buffer: &gtk::TextBuffer) {
    let table = buffer.tag_table();
    if table.lookup("match").is_none() {
        table.add(
            &gtk::TextTag::builder()
                .name("match")
                .background_rgba(&gdk::RGBA::new(1.0, 0.85, 0.0, 0.35))
                .build(),
        );
        table.add(
            &gtk::TextTag::builder()
                .name("current")
                .background_rgba(&gdk::RGBA::new(1.0, 0.55, 0.0, 0.6))
                .build(),
        );
    }
}

fn launch(uri: &str) {
    if let Err(err) = gio::AppInfo::launch_default_for_uri(uri, None::<&gio::AppLaunchContext>) {
        log::warn!("opening {uri}: {err}");
    }
}

/// The file as text: UTF-8 (a byte-order mark dropped), or else Latin-1,
/// which every byte is.
fn read_text(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    Ok(match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin1_files_still_read() {
        let dir = std::env::temp_dir().join(format!("orbit-read-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, b"ol\xe1\n").unwrap();
        assert_eq!(read_text(&file).unwrap(), "olá\n");
        std::fs::write(&file, "\u{feff}olá".as_bytes()).unwrap();
        assert_eq!(read_text(&file).unwrap(), "olá");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
