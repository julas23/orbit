//! Several cursors, and the layer drawn over the text: the extra cursors,
//! their selections, and the indent guides.
//!
//! GTK's text view has one cursor. The others ("extra") are pairs of marks
//! — where the cursor is, where its selection starts — and every edit the
//! user makes at the real cursor is made again at each of them once the
//! user action ends: typed text (dead keys and input methods included,
//! since GTK has already composed it), Enter with its indentation,
//! Backspace, Delete, paste. Arrows move them all.
//!
//! Ctrl+D adds the next occurrence of the selection; Ctrl+click adds a
//! cursor; Alt+drag selects a column, a cursor per line; Escape or a plain
//! click goes back to one.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gdk, glib, prelude::*};
use sourceview5::prelude::*;

/// An extra cursor: where it is, and where its selection starts (the same
/// place when nothing is selected).
struct Caret {
    insert: gtk::TextMark,
    bound: gtk::TextMark,
}

/// An edit made at the real cursor, to repeat at the others.
#[derive(Debug, Clone)]
enum Edit {
    /// The selection was replaced (deleted).
    DeleteSelection,
    /// Characters from `from` to `to`, counted from the cursor (Backspace:
    /// −1..0; Delete: 0..1).
    Delete(i32, i32),
    Insert(String),
}

pub struct Cursors {
    view: sourceview5::View,
    buffer: gtk::TextBuffer,
    extra: RefCell<Vec<Caret>>,
    /// Edits at the real cursor during the current user action.
    pending: RefCell<Vec<Edit>>,
    /// Set while the edits are repeated, so they are not recorded again.
    replaying: Cell<bool>,
    pub layer: gtk::DrawingArea,
    column_from: Cell<Option<(i32, i32)>>,
}

impl Cursors {
    pub fn new(view: &sourceview5::View) -> Rc<Self> {
        let layer = gtk::DrawingArea::builder()
            .can_target(false)
            .hexpand(true)
            .vexpand(true)
            .build();
        let cursors = Rc::new(Self {
            buffer: view.buffer(),
            view: view.clone(),
            extra: RefCell::default(),
            pending: RefCell::default(),
            replaying: Cell::new(false),
            layer,
            column_from: Cell::new(None),
        });
        cursors.connect();
        cursors
    }

    pub fn has_extra(&self) -> bool {
        !self.extra.borrow().is_empty()
    }

    pub fn clear(&self) {
        for caret in self.extra.borrow_mut().drain(..) {
            self.buffer.delete_mark(&caret.insert);
            self.buffer.delete_mark(&caret.bound);
        }
        self.layer.queue_draw();
    }

    fn add(&self, insert: &gtk::TextIter, bound: &gtk::TextIter) {
        // Not twice at the same place, nor on the real cursor.
        let cursor = self.buffer.iter_at_mark(&self.buffer.get_insert());
        if insert.offset() == cursor.offset() {
            return;
        }
        if self
            .extra
            .borrow()
            .iter()
            .any(|c| self.buffer.iter_at_mark(&c.insert).offset() == insert.offset())
        {
            return;
        }
        self.extra.borrow_mut().push(Caret {
            insert: self.buffer.create_mark(None, insert, false),
            bound: self.buffer.create_mark(None, bound, true),
        });
        self.layer.queue_draw();
    }

    fn connect(self: &Rc<Self>) {
        // ---- recording the edits made at the real cursor
        let weak = Rc::downgrade(self);
        self.buffer.connect_delete_range(move |buffer, start, end| {
            let Some(c) = weak.upgrade() else { return };
            if c.replaying.get() || !c.has_extra() {
                return;
            }
            let selection = buffer.selection_bounds();
            let edit = match selection {
                Some((s, e)) if s.offset() == start.offset() && e.offset() == end.offset() => {
                    Edit::DeleteSelection
                }
                _ => {
                    let cursor = buffer.iter_at_mark(&buffer.get_insert()).offset();
                    Edit::Delete(start.offset() - cursor, end.offset() - cursor)
                }
            };
            c.pending.borrow_mut().push(edit);
        });
        let weak = Rc::downgrade(self);
        self.buffer.connect_insert_text(move |buffer, at, text| {
            let Some(c) = weak.upgrade() else { return };
            if c.replaying.get() || !c.has_extra() {
                return;
            }
            // Only what goes in at the cursor.
            if at.offset() == buffer.iter_at_mark(&buffer.get_insert()).offset() {
                c.pending.borrow_mut().push(Edit::Insert(text.to_owned()));
            }
        });
        let weak = Rc::downgrade(self);
        self.buffer.connect_end_user_action(move |_| {
            if let Some(c) = weak.upgrade() {
                let edits: Vec<Edit> = c.pending.borrow_mut().drain(..).collect();
                if !edits.is_empty() {
                    c.replay(&edits);
                }
            }
        });

        // ---- keys: the arrows move every cursor; Ctrl+D, copy, paste
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(c) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
            use gdk::Key;
            match key {
                Key::d | Key::D if ctrl && !shift => {
                    c.add_next_occurrence();
                    return glib::Propagation::Stop;
                }
                _ if !c.has_extra() => {}
                Key::Escape => c.clear(),
                Key::Left | Key::Right | Key::Up | Key::Down | Key::Home | Key::End if !ctrl => {
                    c.move_all(key, shift)
                }
                Key::c | Key::C if ctrl => {
                    c.copy();
                    return glib::Propagation::Stop;
                }
                Key::x | Key::X if ctrl => {
                    c.copy();
                    // Deleting the selections: as if Delete were pressed.
                    c.buffer.begin_user_action();
                    c.buffer.delete_selection(true, true);
                    c.buffer.end_user_action();
                    return glib::Propagation::Stop;
                }
                Key::v | Key::V if ctrl => {
                    c.paste();
                    return glib::Propagation::Stop;
                }
                _ => {}
            }
            glib::Propagation::Proceed
        });
        self.view.add_controller(keys);

        // ---- the mouse: Ctrl+click adds a cursor; a plain click, one again
        let click = gtk::GestureClick::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |gesture, _, x, y| {
            let Some(c) = weak.upgrade() else { return };
            let state = gesture.current_event_state();
            if state.contains(gdk::ModifierType::CONTROL_MASK) {
                if let Some(iter) = c.iter_at(x, y) {
                    c.add(&iter, &iter);
                }
                gesture.set_state(gtk::EventSequenceState::Claimed);
            } else if !state.contains(gdk::ModifierType::ALT_MASK) {
                c.clear();
            }
        });
        self.view.add_controller(click);

        // ---- Alt+drag: a column
        let drag = gtk::GestureDrag::new();
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        drag.connect_drag_begin(move |gesture, x, y| {
            let Some(c) = weak.upgrade() else { return };
            if !gesture.current_event_state().contains(gdk::ModifierType::ALT_MASK) {
                gesture.set_state(gtk::EventSequenceState::Denied);
                return;
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
            c.column_from.set(Some((x as i32, y as i32)));
            c.column(x, y, x, y);
        });
        let weak = Rc::downgrade(self);
        drag.connect_drag_update(move |_, dx, dy| {
            if let Some(c) = weak.upgrade()
                && let Some((x, y)) = c.column_from.get()
            {
                c.column(f64::from(x), f64::from(y), f64::from(x) + dx, f64::from(y) + dy);
            }
        });
        let weak = Rc::downgrade(self);
        drag.connect_drag_end(move |_, _, _| {
            if let Some(c) = weak.upgrade() {
                c.column_from.set(None);
            }
        });
        self.view.add_controller(drag);

        // ---- drawing
        let weak = Rc::downgrade(self);
        self.layer.set_draw_func(move |area, cr, _, _| {
            if let Some(c) = weak.upgrade() {
                c.draw(area, cr);
            }
        });
        let redraw = {
            let layer = self.layer.downgrade();
            move || {
                if let Some(l) = layer.upgrade() {
                    l.queue_draw();
                }
            }
        };
        let r = redraw.clone();
        self.buffer.connect_changed(move |_| r());
        let r = redraw.clone();
        self.buffer.connect_mark_set(move |_, _, _| r());
        let r = redraw.clone();
        self.view.vadjustment().expect("scrollable").connect_value_changed(move |_| r());
        let r = redraw;
        self.view.hadjustment().expect("scrollable").connect_value_changed(move |_| r());
    }

    /// The text position under (x, y), in the view's coordinates.
    fn iter_at(&self, x: f64, y: f64) -> Option<gtk::TextIter> {
        let (bx, by) = self
            .view
            .window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        self.view.iter_at_location(bx, by)
    }

    /// Repeats `edits` at every extra cursor, as one more user action.
    fn replay(&self, edits: &[Edit]) {
        self.replaying.set(true);
        self.buffer.begin_user_action();
        let carets: Vec<(gtk::TextMark, gtk::TextMark)> = self
            .extra
            .borrow()
            .iter()
            .map(|c| (c.insert.clone(), c.bound.clone()))
            .collect();
        for (insert, bound) in &carets {
            for edit in edits {
                let mut at = self.buffer.iter_at_mark(insert);
                match edit {
                    Edit::DeleteSelection => {
                        let mut other = self.buffer.iter_at_mark(bound);
                        if other.offset() != at.offset() {
                            self.buffer.delete(&mut at, &mut other);
                        }
                    }
                    Edit::Delete(from, to) => {
                        let mut start = self.buffer.iter_at_offset(at.offset() + from);
                        let mut end = self.buffer.iter_at_offset(at.offset() + to);
                        if start.offset() != end.offset() {
                            self.buffer.delete(&mut start, &mut end);
                        }
                    }
                    Edit::Insert(text) => self.buffer.insert(&mut at, text),
                }
                // A cursor has no selection once it has typed.
                let now = self.buffer.iter_at_mark(insert);
                self.buffer.move_mark(bound, &now);
            }
        }
        self.buffer.end_user_action();
        self.replaying.set(false);
        self.layer.queue_draw();
    }

    fn move_all(&self, key: gdk::Key, extend: bool) {
        let carets: Vec<(gtk::TextMark, gtk::TextMark)> = self
            .extra
            .borrow()
            .iter()
            .map(|c| (c.insert.clone(), c.bound.clone()))
            .collect();
        for (insert, bound) in carets {
            let mut at = self.buffer.iter_at_mark(&insert);
            match key {
                gdk::Key::Left => {
                    at.backward_cursor_position();
                }
                gdk::Key::Right => {
                    at.forward_cursor_position();
                }
                gdk::Key::Up | gdk::Key::Down => {
                    let column = at.line_offset();
                    let line = at.line() + if key == gdk::Key::Up { -1 } else { 1 };
                    if let Some(mut next) = self.buffer.iter_at_line(line) {
                        if !next.ends_line() {
                            next.forward_to_line_end();
                        }
                        let length = next.line_offset();
                        next.set_line_offset(column.min(length));
                        at = next;
                    }
                }
                gdk::Key::Home => at.set_line_offset(0),
                _ => {
                    if !at.ends_line() {
                        at.forward_to_line_end();
                    }
                }
            }
            self.buffer.move_mark(&insert, &at);
            if !extend {
                self.buffer.move_mark(&bound, &at);
            }
        }
        self.layer.queue_draw();
    }

    /// Selects the word at the cursor, or adds a cursor on the next
    /// occurrence of what is selected.
    fn add_next_occurrence(&self) {
        let Some((start, end)) = self.buffer.selection_bounds() else {
            let mut start = self.buffer.iter_at_mark(&self.buffer.get_insert());
            let mut end = start;
            if !start.starts_word() {
                start.backward_word_start();
            }
            if !end.ends_word() {
                end.forward_word_end();
            }
            if start.offset() != end.offset() {
                self.buffer.select_range(&end, &start);
            }
            return;
        };
        let wanted = self.buffer.text(&start, &end, false).to_string();
        // After the last cursor added, round to the start.
        let from = self
            .extra
            .borrow()
            .last()
            .map(|c| self.buffer.iter_at_mark(&c.insert))
            .unwrap_or(end);
        let flags = gtk::TextSearchFlags::TEXT_ONLY;
        let found = from
            .forward_search(&wanted, flags, None)
            .or_else(|| self.buffer.start_iter().forward_search(&wanted, flags, Some(&start)));
        if let Some((s, e)) = found
            && s.offset() != start.offset()
        {
            self.add(&e, &s);
            let mark = self.buffer.create_mark(None, &e, false);
            self.view.scroll_mark_onscreen(&mark);
            self.buffer.delete_mark(&mark);
        }
    }

    /// A column: from (x1, y1) to (x2, y2), a cursor per line.
    fn column(&self, x1: f64, y1: f64, x2: f64, y2: f64) {
        self.clear();
        let (Some(a), Some(b)) = (self.iter_at(x1, y1), self.iter_at(x1, y2)) else {
            return;
        };
        let (first, last) = (a.line().min(b.line()), a.line().max(b.line()));
        let (bx1, _) = self.view.window_to_buffer_coords(gtk::TextWindowType::Widget, x1 as i32, 0);
        let (bx2, _) = self.view.window_to_buffer_coords(gtk::TextWindowType::Widget, x2 as i32, 0);
        let mut primary = None;
        for line in first..=last {
            let Some(line_start) = self.buffer.iter_at_line(line) else { continue };
            let (y, _) = self.view.line_yrange(&line_start);
            let at = |bx: i32| {
                self.view
                    .iter_at_location(bx, y)
                    .filter(|i| i.line() == line)
                    .unwrap_or_else(|| {
                        let mut end = line_start;
                        if !end.ends_line() {
                            end.forward_to_line_end();
                        }
                        end
                    })
            };
            let (bound, insert) = (at(bx1), at(bx2));
            if primary.is_none() {
                self.buffer.select_range(&insert, &bound);
                primary = Some(());
            } else {
                self.add(&insert, &bound);
            }
        }
    }

    /// Every selection, in the order of the text, one per line.
    fn selections(&self) -> Vec<(gtk::TextIter, gtk::TextIter)> {
        let mut all: Vec<(gtk::TextIter, gtk::TextIter)> = self
            .extra
            .borrow()
            .iter()
            .map(|c| {
                let (a, b) = (self.buffer.iter_at_mark(&c.insert), self.buffer.iter_at_mark(&c.bound));
                if a.offset() <= b.offset() { (a, b) } else { (b, a) }
            })
            .collect();
        let (s, e) = self.buffer.selection_bounds().unwrap_or_else(|| {
            let at = self.buffer.iter_at_mark(&self.buffer.get_insert());
            (at, at)
        });
        all.push((s, e));
        all.sort_by_key(|(s, _)| s.offset());
        all
    }

    fn copy(&self) {
        let text: Vec<String> = self
            .selections()
            .iter()
            .map(|(s, e)| self.buffer.text(s, e, false).to_string())
            .collect();
        self.view.clipboard().set_text(&text.join("\n"));
    }

    /// Pastes: a line per cursor when the clipboard has as many lines as
    /// there are cursors, else all of it at each.
    fn paste(self: &Rc<Self>) {
        let clipboard = self.view.clipboard();
        let weak = Rc::downgrade(self);
        clipboard.read_text_async(gtk::gio::Cancellable::NONE, move |result| {
            let (Some(c), Ok(Some(text))) = (weak.upgrade(), result) else { return };
            let carets = c.extra.borrow().len() + 1;
            let lines: Vec<&str> = text.split('\n').collect();
            let per_cursor = lines.len() == carets;
            let marks: Vec<(gtk::TextMark, gtk::TextMark)> = c
                .selections()
                .iter()
                .map(|(s, e)| (c.buffer.create_mark(None, s, true), c.buffer.create_mark(None, e, false)))
                .collect();
            c.replaying.set(true);
            c.buffer.begin_user_action();
            for (n, (s, e)) in marks.iter().enumerate() {
                let (mut start, mut end) = (c.buffer.iter_at_mark(s), c.buffer.iter_at_mark(e));
                c.buffer.delete(&mut start, &mut end);
                let mut at = c.buffer.iter_at_mark(s);
                c.buffer.insert(&mut at, if per_cursor { lines[n] } else { &text });
                c.buffer.delete_mark(s);
                c.buffer.delete_mark(e);
            }
            c.buffer.end_user_action();
            c.replaying.set(false);
            // The cursors' selections are gone with the text.
            for caret in c.extra.borrow().iter() {
                let at = c.buffer.iter_at_mark(&caret.insert);
                c.buffer.move_mark(&caret.bound, &at);
            }
            c.layer.queue_draw();
        });
    }

    // ------------------------------------------------------------ drawing

    fn draw(&self, area: &gtk::DrawingArea, cr: &gtk::cairo::Context) {
        let to_layer = |bx: i32, by: i32| {
            let (wx, wy) = self
                .view
                .buffer_to_window_coords(gtk::TextWindowType::Widget, bx, by);
            self.view
                .compute_point(area, &gtk::graphene::Point::new(wx as f32, wy as f32))
                .map(|p| (f64::from(p.x()), f64::from(p.y())))
                .unwrap_or((f64::from(wx), f64::from(wy)))
        };
        // Not over the gutter (line numbers).
        let gutter = ViewExt::gutter(&self.view, gtk::TextWindowType::Left).width() as f64;
        cr.rectangle(gutter, 0.0, f64::from(area.width()), f64::from(area.height()));
        cr.clip();

        self.draw_guides(cr, &to_layer);

        let accent = orbit_app::theme::accent();
        let (r, g, b) = (accent.red().into(), accent.green().into(), accent.blue().into());
        for caret in self.extra.borrow().iter() {
            let (a, z) = (self.buffer.iter_at_mark(&caret.insert), self.buffer.iter_at_mark(&caret.bound));
            let (mut s, e) = if a.offset() <= z.offset() { (a, z) } else { (z, a) };
            // The selection, line by line.
            cr.set_source_rgba(r, g, b, 0.3);
            while s.offset() < e.offset() {
                let mut line_end = s;
                if !line_end.ends_line() {
                    line_end.forward_to_line_end();
                }
                let stop = if line_end.offset() < e.offset() { line_end } else { e };
                let (from, to) = (self.view.iter_location(&s), self.view.iter_location(&stop));
                let (x1, y1) = to_layer(from.x(), from.y());
                let (x2, _) = to_layer(to.x(), to.y());
                cr.rectangle(x1, y1, (x2 - x1).max(4.0), f64::from(from.height()));
                cr.fill().ok();
                if !s.forward_line() {
                    break;
                }
            }
            // The cursor.
            let place = self.view.iter_location(&a);
            let (x, y) = to_layer(place.x(), place.y());
            cr.set_source_rgba(r, g, b, 1.0);
            cr.rectangle(x - 1.0, y, 2.0, f64::from(place.height()));
            cr.fill().ok();
        }
    }

    /// A thin line at each level of indentation, down every indented block.
    fn draw_guides(&self, cr: &gtk::cairo::Context, to_layer: &dyn Fn(i32, i32) -> (f64, f64)) {
        let visible = self.view.visible_rect();
        // Past the text's end there is no position: the end, then.
        let top = self
            .view
            .iter_at_location(0, visible.y())
            .unwrap_or_else(|| self.buffer.start_iter());
        let bottom = self
            .view
            .iter_at_location(0, visible.y() + visible.height())
            .unwrap_or_else(|| self.buffer.end_iter());
        let tab = self.view.tab_width().max(1) as i32;
        let indent = match self.view.indent_width() {
            n if n > 0 => n,
            _ => tab,
        };
        let space = self
            .view
            .create_pango_layout(Some(" "))
            .pixel_size()
            .0
            .max(1);
        cr.set_source_rgba(0.5, 0.5, 0.5, 0.25);
        cr.set_line_width(1.0);
        // Blank lines take the deeper of the lines around them, so the
        // guides run through them.
        let levels = levels(&self.buffer, top.line(), bottom.line(), tab, indent);
        for (line, level) in levels {
            let Some(start) = self.buffer.iter_at_line(line) else { continue };
            let place = self.view.iter_location(&start);
            for k in 0..level {
                let bx = place.x() + k * indent * space;
                let (x, y) = to_layer(bx, place.y());
                cr.move_to(x.floor() + 0.5, y);
                cr.line_to(x.floor() + 0.5, y + f64::from(place.height()));
            }
        }
        cr.stroke().ok();
    }
}

/// Indentation of `line` in columns, or `None` for a blank line.
pub fn indentation(buffer: &gtk::TextBuffer, line: i32, tab: i32) -> Option<i32> {
    let mut at = buffer.iter_at_line(line)?;
    let mut column = 0;
    loop {
        match at.char() {
            ' ' => column += 1,
            '\t' => column += tab - column % tab,
            '\n' | '\r' | '\0' => return None,
            _ => return Some(column),
        }
        if !at.forward_char() || at.line() != line {
            return None;
        }
    }
}

/// The guide levels of lines `first..=last`.
fn levels(buffer: &gtk::TextBuffer, first: i32, last: i32, tab: i32, indent: i32) -> Vec<(i32, i32)> {
    let lines = buffer.line_count();
    let nearest = |line: i32, step: i32| {
        let mut n = line + step;
        let mut tries = 0;
        while (0..lines).contains(&n) && tries < 200 {
            if let Some(c) = indentation(buffer, n, tab) {
                return c;
            }
            n += step;
            tries += 1;
        }
        0
    };
    (first..=last)
        .map(|line| {
            let columns = indentation(buffer, line, tab)
                .unwrap_or_else(|| nearest(line, -1).min(nearest(line, 1)));
            (line, columns / indent)
        })
        .collect()
}
