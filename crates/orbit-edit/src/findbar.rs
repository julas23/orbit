//! Find and replace, under the tabs: GtkSourceView's search, every match
//! marked, with case, whole words and regular expressions (and `\1` in
//! the replacement) to switch on.

use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, glib, prelude::*};
use orbit_app::i18n::{number, tr, tr_args};
use sourceview5::prelude::*;

use crate::doc::Doc;

pub struct FindBar {
    pub widget: gtk::Revealer,
    find: gtk::SearchEntry,
    replace: gtk::Entry,
    replace_row: gtk::Box,
    count: gtk::Label,
    case: gtk::ToggleButton,
    words: gtk::ToggleButton,
    regex: gtk::ToggleButton,
    doc: RefCell<Option<Rc<Doc>>>,
    /// The count's handler on that document's search.
    watch: RefCell<Option<glib::SignalHandlerId>>,
}

impl FindBar {
    pub fn new() -> Rc<Self> {
        let find = gtk::SearchEntry::builder()
            .placeholder_text(tr("edit-find"))
            .width_chars(30)
            .build();
        let replace = gtk::Entry::builder()
            .placeholder_text(tr("edit-replace-with"))
            .width_chars(30)
            .build();
        let toggle = |label: &str, tip: &str| {
            gtk::ToggleButton::builder()
                .label(label)
                .tooltip_text(tr(tip))
                .focus_on_click(false)
                .css_classes(["option"])
                .build()
        };
        let case = toggle("Aa", "edit-find-case");
        let words = toggle("\\b", "edit-find-words");
        let regex = toggle(".*", "edit-find-regex");
        let button = |icon: &str, tip: &str| {
            gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(tr(tip))
                .focus_on_click(false)
                .build()
        };
        let previous = button("go-up-symbolic", "edit-find-previous");
        let next = button("go-down-symbolic", "edit-find-next");
        let count = gtk::Label::builder().width_chars(12).css_classes(["count"]).build();
        let close = button("window-close-symbolic", "edit-find-close");
        let find_row = gtk::Box::builder().spacing(5).build();
        for w in [
            find.upcast_ref::<gtk::Widget>(),
            previous.upcast_ref(),
            next.upcast_ref(),
            case.upcast_ref(),
            words.upcast_ref(),
            regex.upcast_ref(),
            count.upcast_ref(),
        ] {
            find_row.append(w);
        }
        find_row.append(&gtk::Box::builder().hexpand(true).build());
        find_row.append(&close);
        let replace_one = gtk::Button::builder().label(tr("edit-replace")).focus_on_click(false).build();
        let replace_all = gtk::Button::builder().label(tr("edit-replace-all")).focus_on_click(false).build();
        let replace_row = gtk::Box::builder().spacing(5).build();
        replace_row.append(&replace);
        replace_row.append(&replace_one);
        replace_row.append(&replace_all);
        let rows = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(5)
            .css_classes(["findbar"])
            .build();
        rows.append(&find_row);
        rows.append(&replace_row);
        let widget = gtk::Revealer::builder()
            .child(&rows)
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .build();

        let bar = Rc::new(Self {
            widget,
            find,
            replace,
            replace_row,
            count,
            case,
            words,
            regex,
            doc: RefCell::default(),
            watch: RefCell::default(),
        });
        let weak = Rc::downgrade(&bar);
        bar.find.connect_search_changed(move |_| {
            if let Some(b) = weak.upgrade() {
                b.apply();
                b.step(true, true);
            }
        });
        for toggle in [&bar.case, &bar.words, &bar.regex] {
            let weak = Rc::downgrade(&bar);
            toggle.connect_toggled(move |_| {
                if let Some(b) = weak.upgrade() {
                    b.apply();
                }
            });
        }
        let weak = Rc::downgrade(&bar);
        bar.find.connect_activate(move |_| {
            if let Some(b) = weak.upgrade() {
                b.step(true, false);
            }
        });
        let weak = Rc::downgrade(&bar);
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(b) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            match key {
                gdk::Key::Return | gdk::Key::KP_Enter if state.contains(gdk::ModifierType::SHIFT_MASK) => {
                    b.step(false, false);
                    glib::Propagation::Stop
                }
                gdk::Key::Escape => {
                    b.hide();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        rows.add_controller(keys);
        let act = |button: &gtk::Button, f: fn(&FindBar)| {
            let weak = Rc::downgrade(&bar);
            button.connect_clicked(move |_| {
                if let Some(b) = weak.upgrade() {
                    f(&b);
                }
            });
        };
        act(&previous, |b| b.step(false, false));
        act(&next, |b| b.step(true, false));
        act(&close, |b| b.hide());
        act(&replace_one, |b| b.replace_one());
        act(&replace_all, |b| b.replace_all());
        let weak = Rc::downgrade(&bar);
        bar.replace.connect_activate(move |_| {
            if let Some(b) = weak.upgrade() {
                b.replace_one();
            }
        });
        bar
    }

    pub fn is_shown(&self) -> bool {
        self.widget.reveals_child()
    }

    /// Shows the bar for `doc`, with the replace row if `replace`; the
    /// selection, if any, is what is searched for.
    pub fn show(&self, doc: &Rc<Doc>, replace: bool) {
        self.set_doc(Some(doc));
        if let Some((s, e)) = doc.buffer.selection_bounds()
            && s.line() == e.line()
        {
            self.find.set_text(&doc.buffer.text(&s, &e, false));
        }
        self.replace_row.set_visible(replace);
        self.widget.set_reveal_child(true);
        self.find.grab_focus();
        self.apply();
    }

    pub fn hide(&self) {
        self.widget.set_reveal_child(false);
        if let Some(doc) = self.doc.borrow().as_ref() {
            doc.search.settings().set_search_text(None);
            doc.view.grab_focus();
        }
    }

    /// Follows the tab shown: its search takes the bar's settings.
    pub fn set_doc(&self, doc: Option<&Rc<Doc>>) {
        if let Some(old) = self.doc.borrow().as_ref() {
            old.search.settings().set_search_text(None);
            if let Some(id) = self.watch.borrow_mut().take() {
                old.search.disconnect(id);
            }
        }
        *self.doc.borrow_mut() = doc.cloned();
        if let Some(doc) = doc {
            let count = self.count.downgrade();
            let id = doc.search.connect_occurrences_count_notify(move |search| {
                if let Some(label) = count.upgrade() {
                    // Where the selected match falls, if one is selected.
                    let buffer = search.buffer();
                    let position = buffer
                        .selection_bounds()
                        .map(|(s, e)| search.occurrence_position(&s, &e))
                        .filter(|p| *p > 0);
                    label.set_label(&counter(position, search.occurrences_count()));
                }
            });
            *self.watch.borrow_mut() = Some(id);
            if self.is_shown() {
                self.apply();
            }
        }
    }

    fn apply(&self) {
        let Some(doc) = self.doc.borrow().clone() else { return };
        let settings = doc.search.settings();
        let text = self.find.text();
        settings.set_case_sensitive(self.case.is_active());
        settings.set_at_word_boundaries(self.words.is_active());
        settings.set_regex_enabled(self.regex.is_active());
        settings.set_search_text((!text.is_empty()).then_some(text.as_str()));
        if text.is_empty() {
            self.count.set_label("");
        }
        self.find.remove_css_class("error");
        if doc.search.regex_error().is_some() {
            self.find.add_css_class("error");
        }
    }

    /// Selects the next match (the previous one), from the cursor; `again`
    /// starts from the selection's start, so the match being typed stays.
    fn step(&self, forward: bool, again: bool) {
        let Some(doc) = self.doc.borrow().clone() else { return };
        let buffer = &doc.buffer;
        let (start, end) = buffer.selection_bounds().unwrap_or_else(|| {
            let at = buffer.iter_at_mark(&buffer.get_insert());
            (at, at)
        });
        let found = if forward {
            doc.search.forward(if again { &start } else { &end })
        } else {
            doc.search.backward(&start)
        };
        if let Some((s, e, _)) = found {
            buffer.select_range(&s, &e);
            doc.view.scroll_to_iter(&mut s.clone(), 0.1, false, 0.0, 0.0);
            let position = doc.search.occurrence_position(&s, &e);
            self.count.set_label(&counter(Some(position), doc.search.occurrences_count()));
        } else if !self.find.text().is_empty() {
            self.count.set_label(&counter(None, 0));
        }
    }

    fn replace_one(&self) {
        let Some(doc) = self.doc.borrow().clone() else { return };
        let with = self.replace.text();
        if let Some((mut s, mut e)) = doc.buffer.selection_bounds()
            && doc.search.occurrence_position(&s, &e) > 0
            && let Err(err) = doc.search.replace(&mut s, &mut e, &with)
        {
            log::warn!("replace: {err}");
        }
        self.step(true, false);
    }

    fn replace_all(&self) {
        let Some(doc) = self.doc.borrow().clone() else { return };
        let n = doc.search.occurrences_count().max(0);
        match doc.search.replace_all(&self.replace.text()) {
            Ok(()) => self.count.set_label(&tr_args("edit-replaced", &[("n", number(n as f64))])),
            Err(err) => log::warn!("replace all: {err}"),
        }
    }
}

/// "3 de 12", "12 encontrados", "nada".
fn counter(position: Option<i32>, total: i32) -> String {
    match (position, total) {
        (_, t) if t < 0 => String::new(),
        (_, 0) => tr("edit-find-none"),
        (Some(p), t) if p > 0 => tr_args(
            "edit-find-position",
            &[("position", number(p as f64)), ("total", number(t as f64))],
        ),
        (_, t) => tr_args("edit-find-count", &[("total", number(t as f64))]),
    }
}
