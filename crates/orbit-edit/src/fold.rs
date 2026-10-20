//! Folding by indentation: a line followed by more indented ones starts a
//! block; the arrow beside it in the gutter folds the block away (an
//! invisible tag over it) and back. Ctrl+Shift+[ and ] do the same for the
//! block the cursor is in.

use std::{cell::RefCell, rc::Rc};

use gtk::prelude::*;
use sourceview5::prelude::*;

use crate::multi::indentation;

pub struct Folds {
    view: sourceview5::View,
    buffer: gtk::TextBuffer,
    tag: gtk::TextTag,
    renderer: sourceview5::GutterRendererText,
    /// Each folded block: its first line, and where the hidden text starts
    /// and ends — kept, so unfolding shows what was hidden even after
    /// edits around it.
    folded: RefCell<Vec<[gtk::TextMark; 3]>>,
}

impl Folds {
    pub fn new(view: &sourceview5::View) -> Rc<Self> {
        let buffer = view.buffer();
        let tag = gtk::TextTag::builder().invisible(true).build();
        buffer.tag_table().add(&tag);
        let renderer = sourceview5::GutterRendererText::builder()
            .xpad(5)
            .width_request(15)
            .css_classes(["fold"])
            .build();
        ViewExt::gutter(view, gtk::TextWindowType::Left).insert(&renderer, 10);
        let folds = Rc::new(Self {
            view: view.clone(),
            buffer,
            tag,
            renderer,
            folded: RefCell::default(),
        });
        folds.connect();
        folds
    }

    fn tab(&self) -> i32 {
        self.view.tab_width().max(1) as i32
    }

    fn connect(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.renderer.connect_query_data(move |renderer, _, line| {
            let Some(f) = weak.upgrade() else { return };
            let line = line as i32;
            let mark = if f.is_folded(line) {
                "▸"
            } else if f.block(line).is_some() {
                "▾"
            } else {
                ""
            };
            renderer.set_text(mark);
        });
        let weak = Rc::downgrade(self);
        self.renderer.connect_query_activatable(move |_, iter, _| {
            weak.upgrade()
                .is_some_and(|f| f.is_folded(iter.line()) || f.block(iter.line()).is_some())
        });
        let weak = Rc::downgrade(self);
        self.renderer.connect_activate(move |_, iter, _, _, _, _| {
            if let Some(f) = weak.upgrade() {
                f.toggle(iter.line());
            }
        });
    }

    /// The block `line` starts: its last line, if the lines after it are
    /// more indented.
    fn block(&self, line: i32) -> Option<i32> {
        let tab = self.tab();
        let own = indentation(&self.buffer, line, tab)?;
        let lines = self.buffer.line_count();
        let mut last = None;
        let mut n = line + 1;
        while n < lines {
            match indentation(&self.buffer, n, tab) {
                Some(c) if c > own => last = Some(n),
                Some(_) => break,
                None => {}
            }
            n += 1;
        }
        last
    }

    fn is_folded(&self, line: i32) -> bool {
        self.folded
            .borrow()
            .iter()
            .any(|[m, _, _]| self.buffer.iter_at_mark(m).line() == line)
    }

    pub fn toggle(&self, line: i32) {
        if self.is_folded(line) {
            self.unfold(line);
        } else {
            self.fold(line);
        }
    }

    fn range(&self, line: i32) -> Option<(gtk::TextIter, gtk::TextIter)> {
        let last = self.block(line)?;
        let mut start = self.buffer.iter_at_line(line)?;
        if !start.ends_line() {
            start.forward_to_line_end();
        }
        let mut end = self.buffer.iter_at_line(last)?;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        Some((start, end))
    }

    pub fn fold(&self, line: i32) {
        let Some((start, end)) = self.range(line) else { return };
        // The cursor would be lost inside: out to the folded line.
        let cursor = self.buffer.iter_at_mark(&self.buffer.get_insert());
        if cursor.offset() > start.offset() && cursor.offset() <= end.offset() {
            self.buffer.place_cursor(&start);
        }
        self.buffer.apply_tag(&self.tag, &start, &end);
        let at = self.buffer.iter_at_line(line).unwrap_or(start);
        self.folded.borrow_mut().push([
            self.buffer.create_mark(None, &at, true),
            self.buffer.create_mark(None, &start, true),
            self.buffer.create_mark(None, &end, false),
        ]);
        self.renderer.queue_draw();
    }

    pub fn unfold(&self, line: i32) {
        let mut folded = self.folded.borrow_mut();
        if let Some(pos) = folded
            .iter()
            .position(|[m, _, _]| self.buffer.iter_at_mark(m).line() == line)
        {
            let marks = folded.remove(pos);
            let (start, end) = (self.buffer.iter_at_mark(&marks[1]), self.buffer.iter_at_mark(&marks[2]));
            self.buffer.remove_tag(&self.tag, &start, &end);
            for mark in &marks {
                self.buffer.delete_mark(mark);
            }
        }
        drop(folded);
        self.renderer.queue_draw();
    }

    /// The line starting the block the cursor is in.
    fn enclosing(&self) -> Option<i32> {
        let cursor = self.buffer.iter_at_mark(&self.buffer.get_insert()).line();
        if self.block(cursor).is_some() {
            return Some(cursor);
        }
        let tab = self.tab();
        let own = indentation(&self.buffer, cursor, tab).unwrap_or(0);
        (0..cursor)
            .rev()
            .find(|&l| indentation(&self.buffer, l, tab).is_some_and(|c| c < own))
    }

    pub fn fold_here(&self) {
        if let Some(line) = self.enclosing() {
            self.fold(line);
        }
    }

    pub fn unfold_here(&self) {
        let cursor = self.buffer.iter_at_mark(&self.buffer.get_insert()).line();
        self.unfold(cursor);
    }
}
