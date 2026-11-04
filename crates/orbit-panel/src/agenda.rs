//! The agenda: the whole month over every window, with the events of the
//! calendars in `modules/calendar.yaml` (iCalendar links, see `ics.rs`).
//! Opened by a left click on the calendar or the clock; Esc, the close
//! button or a click outside the card closes it.
//!
//! The month on the left, a cell per day with its first events; the day
//! picked (today at first) on the right with all of its events. The arrows,
//! Page Up/Down or the wheel change the month; "Today" comes back.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

use chrono::{Datelike, Local, NaiveDate, TimeZone};
use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use orbit_app::i18n::{text, tr, tr_args};

use crate::{
    calendar::{CalendarConfig, WeekStart},
    ics::{self, Occurrence},
};

/// The colours of calendars that name none.
const COLOURS: &[&str] = &["#4285f4", "#33b679", "#e67c73", "#f6bf26", "#8e24aa", "#f4511e", "#039be5"];
/// Events written in a day's cell; the rest are counted.
const IN_CELL: usize = 4;

thread_local! {
    static OPEN: RefCell<glib::WeakRef<gtk::Window>> = RefCell::default();
    /// Each calendar's text and when it was fetched.
    static TEXTS: RefCell<HashMap<String, (String, Option<Instant>)>> = RefCell::default();
}

pub fn colour(config: &CalendarConfig, index: usize) -> String {
    config
        .calendars
        .get(index)
        .and_then(|c| c.colour.clone())
        .unwrap_or_else(|| COLOURS[index % COLOURS.len()].to_string())
}

struct Agenda {
    config: CalendarConfig,
    window: gtk::Window,
    /// The first day of the month shown.
    month: Cell<NaiveDate>,
    picked: Cell<NaiveDate>,
    title: gtk::Label,
    grid: gtk::Grid,
    day_title: gtk::Label,
    day_list: gtk::Box,
    status: gtk::Label,
    events: RefCell<Vec<Occurrence>>,
}

/// Opens the agenda on `anchor`'s monitor (or brings it back).
pub fn open(anchor: &gtk::Widget, config: &CalendarConfig) {
    if let Some(window) = OPEN.with(|o| o.borrow().upgrade()) {
        window.present();
        return;
    }
    let window = gtk::Window::builder()
        .title(tr("agenda-title"))
        .css_classes(["orbit", "orbit-agenda"])
        .decorated(false)
        .build();
    let monitor = anchor
        .native()
        .and_then(|n| n.surface())
        .and_then(|s| gdk::Display::default()?.monitor_at_surface(&s));
    if gtk4_layer_shell::is_supported() {
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_namespace(Some("orbit-agenda"));
        window.set_monitor(monitor.as_ref());
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(edge, true);
        }
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
    } else {
        window.set_default_size(1200, 800);
    }

    let today = Local::now().date_naive();
    let agenda = Rc::new(Agenda {
        config: config.clone(),
        window: window.clone(),
        month: Cell::new(today.with_day(1).unwrap_or(today)),
        picked: Cell::new(today),
        title: gtk::Label::builder().css_classes(["agenda-month"]).build(),
        grid: gtk::Grid::builder()
            .column_homogeneous(true)
            .row_homogeneous(true)
            .hexpand(true)
            .vexpand(true)
            .css_classes(["agenda-grid"])
            .build(),
        day_title: gtk::Label::builder().css_classes(["agenda-day-title"]).xalign(0.0).build(),
        day_list: gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .build(),
        status: gtk::Label::builder().css_classes(["agenda-status"]).xalign(0.0).hexpand(true).build(),
        events: RefCell::default(),
    });
    window.set_child(Some(&layout(&agenda)));
    keys(&agenda);
    OPEN.with(|o| o.borrow().set(Some(&window)));
    agenda.show();
    agenda.load(false);
    window.present();
}

fn button(icon: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .css_classes(["flat", "agenda-button"])
        .build()
}

fn layout(agenda: &Rc<Agenda>) -> gtk::Widget {
    let prev = button("go-previous-symbolic", &tr("agenda-previous"));
    let next = button("go-next-symbolic", &tr("agenda-next"));
    let today = gtk::Button::builder()
        .label(tr("agenda-today"))
        .css_classes(["flat", "agenda-button"])
        .build();
    let refresh = button("view-refresh-symbolic", &tr("agenda-refresh"));
    let close = button("window-close-symbolic", &tr("agenda-close"));
    let head = gtk::Box::builder().spacing(6).css_classes(["agenda-head"]).build();
    head.append(&prev);
    head.append(&agenda.title);
    head.append(&next);
    head.append(&today);
    head.append(&gtk::Box::builder().hexpand(true).build());
    head.append(&refresh);
    head.append(&close);

    let weak = Rc::downgrade(agenda);
    let on = move |f: fn(&Rc<Agenda>)| {
        let weak = weak.clone();
        move |_: &gtk::Button| {
            if let Some(a) = weak.upgrade() {
                f(&a);
            }
        }
    };
    prev.connect_clicked(on(|a| a.step(-1)));
    next.connect_clicked(on(|a| a.step(1)));
    today.connect_clicked(on(|a| {
        let today = Local::now().date_naive();
        a.month.set(today.with_day(1).unwrap_or(today));
        a.picked.set(today);
        a.show();
    }));
    refresh.connect_clicked(on(|a| a.load(true)));
    close.connect_clicked(on(|a| a.window.close()));

    // The weekday names.
    let names = gtk::Grid::builder().column_homogeneous(true).css_classes(["agenda-names"]).build();
    for col in 0..7 {
        let weekday = weekday_of(agenda.config.week_start, col);
        let mut classes = vec!["agenda-name"];
        if weekday >= 6 {
            classes.push("weekend");
        }
        names.attach(
            &gtk::Label::builder().label(weekday_name(weekday)).css_classes(classes).build(),
            col as i32,
            0,
            1,
            1,
        );
    }
    let month = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
    month.append(&names);
    month.append(&agenda.grid);
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    let weak = Rc::downgrade(agenda);
    wheel.connect_scroll(move |_, _, dy| {
        if let Some(a) = weak.upgrade() {
            a.step(if dy > 0.0 { 1 } else { -1 });
        }
        glib::Propagation::Stop
    });
    month.add_controller(wheel);

    let side = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .width_request(340)
        .css_classes(["agenda-side"])
        .build();
    side.append(&agenda.day_title);
    side.append(
        &gtk::ScrolledWindow::builder()
            .child(&agenda.day_list)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build(),
    );

    let body = gtk::Box::builder().spacing(16).vexpand(true).build();
    body.append(&month);
    body.append(&side);

    let legend = gtk::Box::builder().spacing(14).css_classes(["agenda-legend"]).build();
    legend.append(&agenda.status);
    for (i, cal) in agenda.config.calendars.iter().enumerate() {
        legend.append(
            &gtk::Label::builder()
                .use_markup(true)
                .label(format!(
                    "<span foreground=\"{}\">●</span> {}",
                    colour(&agenda.config, i),
                    glib::markup_escape_text(&cal.name)
                ))
                .build(),
        );
    }

    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .css_classes(["agenda-card"])
        .build();
    card.append(&head);
    card.append(&body);
    card.append(&legend);

    // A click on the dimmed backdrop, outside the card, closes.
    let backdrop = gtk::Box::builder().css_classes(["agenda-backdrop"]).build();
    card.set_hexpand(true);
    backdrop.append(&card);
    let click = gtk::GestureClick::builder().button(0).build();
    let weak_card = card.downgrade();
    let weak = Rc::downgrade(agenda);
    click.connect_released(move |gesture, _, x, y| {
        let (Some(card), Some(a), Some(backdrop)) = (weak_card.upgrade(), weak.upgrade(), gesture.widget()) else {
            return;
        };
        let inside = backdrop
            .compute_point(&card, &gtk::graphene::Point::new(x as f32, y as f32))
            .is_some_and(|p| card.contains(f64::from(p.x()), f64::from(p.y())));
        if !inside {
            a.window.close();
        }
    });
    backdrop.add_controller(click);
    backdrop.upcast()
}

fn keys(agenda: &Rc<Agenda>) {
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(agenda);
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(a) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let move_day = |days: i64| {
            let day = a.picked.get() + chrono::Duration::days(days);
            a.picked.set(day);
            a.month.set(day.with_day(1).unwrap_or(day));
            a.show();
        };
        match key {
            gdk::Key::Escape => a.window.close(),
            gdk::Key::Page_Up => a.step(-1),
            gdk::Key::Page_Down => a.step(1),
            gdk::Key::Left => move_day(-1),
            gdk::Key::Right => move_day(1),
            gdk::Key::Up => move_day(-7),
            gdk::Key::Down => move_day(7),
            gdk::Key::t | gdk::Key::T | gdk::Key::Home => move_day((Local::now().date_naive() - a.picked.get()).num_days()),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    agenda.window.add_controller(keys);
}

/// ISO weekday (1 Monday … 7 Sunday) of column `col`.
fn weekday_of(start: WeekStart, col: usize) -> u32 {
    match start {
        WeekStart::Monday => col as u32 + 1,
        WeekStart::Sunday if col == 0 => 7,
        WeekStart::Sunday => col as u32,
    }
}

/// The weekday's name in the locale, "segunda-feira" → "Segunda".
fn weekday_name(weekday: u32) -> String {
    // 1 January 2024 was a Monday.
    glib::DateTime::from_local(2024, 1, weekday as i32, 12, 0, 0.0)
        .ok()
        .and_then(|d| d.format("%A").ok())
        .map(|name| {
            let name = name.split('-').next().unwrap_or_default().to_string();
            let mut chars = name.chars();
            chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
        })
        .unwrap_or_default()
}

fn glib_date(day: NaiveDate) -> Option<glib::DateTime> {
    glib::DateTime::from_local(day.year(), day.month() as i32, day.day() as i32, 12, 0, 0.0).ok()
}

fn hm(time: chrono::DateTime<Local>) -> String {
    time.format("%H:%M").to_string()
}

impl Agenda {
    fn step(self: &Rc<Self>, months: i32) {
        let m = self.month.get();
        let total = m.year() * 12 + m.month0() as i32 + months;
        if let Some(first) = NaiveDate::from_ymd_opt(total.div_euclid(12), total.rem_euclid(12) as u32 + 1, 1) {
            self.month.set(first);
            // The same day of the new month, or its last.
            let day = self.picked.get().day().min(days_in(first));
            self.picked.set(first.with_day(day).unwrap_or(first));
            self.show();
            self.load(false);
        }
    }

    /// The six weeks on screen: their first day and the day after the last.
    fn span(&self) -> (NaiveDate, NaiveDate) {
        let first = self.month.get();
        let col = (0..7)
            .find(|&c| weekday_of(self.config.week_start, c) == first.weekday().number_from_monday())
            .unwrap_or(0);
        let start = first - chrono::Duration::days(col as i64);
        (start, start + chrono::Duration::days(42))
    }

    /// The events of the calendars, from the copies kept at once, then
    /// from the network when they are older than `refresh` (or `force`).
    fn load(self: &Rc<Self>, force: bool) {
        self.collect();
        let stale = Duration::from_secs(u64::from(self.config.refresh.max(1)) * 60);
        let wanted: Vec<String> = self
            .config
            .calendars
            .iter()
            .map(|c| c.url.clone())
            .filter(|url| {
                force
                    || TEXTS.with(|t| {
                        t.borrow()
                            .get(url)
                            .and_then(|(_, at)| *at)
                            .is_none_or(|at| at.elapsed() > stale)
                    })
            })
            .collect();
        if wanted.is_empty() {
            return;
        }
        self.status.set_label(&tr("agenda-loading"));
        let (tx, rx) = async_channel::unbounded();
        std::thread::spawn(move || {
            for url in wanted {
                let result = ics::fetch(&url);
                let _ = tx.send_blocking((url, result));
            }
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let mut errors = Vec::new();
            while let Ok((url, result)) = rx.recv().await {
                match result {
                    Ok(text) => TEXTS.with(|t| {
                        t.borrow_mut().insert(url, (text, Some(Instant::now())));
                    }),
                    Err(err) => {
                        log::warn!("calendar: {url}: {err}");
                        errors.push(err);
                    }
                }
            }
            if let Some(a) = weak.upgrade() {
                a.status.set_label(&if errors.is_empty() {
                    String::new()
                } else {
                    tr_args("agenda-failed", &[("count", text(errors.len().to_string()))])
                });
                a.collect();
            }
        });
    }

    /// The events of the weeks on screen, from the texts at hand.
    fn collect(self: &Rc<Self>) {
        let (start, end) = self.span();
        let at = |day: NaiveDate| {
            Local
                .from_local_datetime(&day.and_time(chrono::NaiveTime::MIN))
                .earliest()
                .unwrap_or_else(Local::now)
        };
        let (from, to) = (at(start), at(end));
        let mut events = Vec::new();
        for (i, cal) in self.config.calendars.iter().enumerate() {
            let text = TEXTS.with(|t| t.borrow().get(&cal.url).map(|(text, _)| text.clone())).or_else(|| {
                let text = ics::cached(&cal.url)?;
                TEXTS.with(|t| t.borrow_mut().insert(cal.url.clone(), (text.clone(), None)));
                Some(text)
            });
            if let Some(text) = text {
                events.extend(ics::occurrences(&text, i, from, to));
            }
        }
        // All-day first, then by time.
        events.sort_by_key(|e| (!e.all_day, e.start));
        *self.events.borrow_mut() = events;
        if self.config.calendars.is_empty() {
            self.status.set_label(&tr("agenda-no-calendars"));
        }
        self.show();
    }

    fn show(self: &Rc<Self>) {
        let month = self.month.get();
        self.title.set_label(
            &glib_date(month)
                .and_then(|d| d.format("%B %Y").ok())
                .map(|s| capitalize(&s))
                .unwrap_or_default(),
        );
        while let Some(child) = self.grid.first_child() {
            self.grid.remove(&child);
        }
        let (start, _) = self.span();
        let today = Local::now().date_naive();
        let events = self.events.borrow();
        for i in 0..42 {
            let day = start + chrono::Duration::days(i);
            let cell = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(2)
                .css_classes(["agenda-cell"])
                .build();
            if day.month() != month.month() {
                cell.add_css_class("other");
            }
            if day == today {
                cell.add_css_class("today");
            }
            if day == self.picked.get() {
                cell.add_css_class("picked");
            }
            if day.weekday().number_from_monday() >= 6 {
                cell.add_css_class("weekend");
            }
            cell.append(&gtk::Label::builder().label(day.day().to_string()).css_classes(["agenda-number"]).xalign(0.0).build());
            let on_day: Vec<&Occurrence> = events.iter().filter(|e| e.on(day)).collect();
            for event in on_day.iter().take(IN_CELL) {
                let time = if event.all_day { String::new() } else { format!("{} ", hm(event.start)) };
                cell.append(
                    &gtk::Label::builder()
                        .use_markup(true)
                        .label(format!(
                            "<span foreground=\"{}\">●</span> {}{}",
                            colour(&self.config, event.calendar),
                            time,
                            glib::markup_escape_text(&event.summary)
                        ))
                        .css_classes(["agenda-event"])
                        .xalign(0.0)
                        .ellipsize(gtk::pango::EllipsizeMode::End)
                        .build(),
                );
            }
            if on_day.len() > IN_CELL {
                cell.append(
                    &gtk::Label::builder()
                        .label(tr_args("agenda-more", &[("count", text((on_day.len() - IN_CELL).to_string()))]))
                        .css_classes(["agenda-more"])
                        .xalign(0.0)
                        .build(),
                );
            }
            let click = gtk::GestureClick::new();
            let weak = Rc::downgrade(self);
            click.connect_released(move |_, _, _, _| {
                if let Some(a) = weak.upgrade() {
                    a.picked.set(day);
                    if day.month() != a.month.get().month() {
                        a.month.set(day.with_day(1).unwrap_or(day));
                        a.collect();
                        a.load(false);
                    } else {
                        a.show();
                    }
                }
            });
            cell.add_controller(click);
            self.grid.attach(&cell, (i % 7) as i32, (i / 7) as i32, 1, 1);
        }
        let picked: Vec<Occurrence> = events.iter().filter(|e| e.on(self.picked.get())).cloned().collect();
        drop(events);
        self.show_day(&picked);
    }

    /// The day picked, every event of it.
    fn show_day(&self, events: &[Occurrence]) {
        let day = self.picked.get();
        self.day_title.set_label(
            &glib_date(day)
                .and_then(|d| d.format("%A, %-d %B").ok())
                .map(|s| capitalize(&s))
                .unwrap_or_default(),
        );
        while let Some(child) = self.day_list.first_child() {
            self.day_list.remove(&child);
        }
        if events.is_empty() {
            self.day_list.append(&gtk::Label::builder().label(tr("agenda-free")).css_classes(["dim"]).xalign(0.0).build());
        }
        for event in events {
            let row = gtk::Box::builder().spacing(10).css_classes(["agenda-item"]).build();
            let bar = gtk::Label::builder()
                .use_markup(true)
                .label(format!("<span foreground=\"{}\">▍</span>", colour(&self.config, event.calendar)))
                .valign(gtk::Align::Start)
                .build();
            let text_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
            let when = if event.all_day {
                tr("agenda-all-day")
            } else {
                format!("{} – {}", hm(event.start), hm(event.end))
            };
            text_box.append(&gtk::Label::builder().label(when).css_classes(["agenda-when"]).xalign(0.0).build());
            text_box.append(
                &gtk::Label::builder()
                    .label(&event.summary)
                    .css_classes(["agenda-summary"])
                    .xalign(0.0)
                    .wrap(true)
                    .build(),
            );
            if !event.location.is_empty() {
                text_box.append(
                    &gtk::Label::builder()
                        .label(format!("\u{f034e} {}", event.location))
                        .css_classes(["dim"])
                        .xalign(0.0)
                        .wrap(true)
                        .build(),
                );
            }
            if let Some(cal) = self.config.calendars.get(event.calendar) {
                text_box.append(&gtk::Label::builder().label(&cal.name).css_classes(["dim"]).xalign(0.0).build());
            }
            row.append(&bar);
            row.append(&text_box);
            self.day_list.append(&row);
        }
    }
}

fn days_in(first: NaiveDate) -> u32 {
    let next = if first.month() == 12 {
        NaiveDate::from_ymd_opt(first.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(first.year(), first.month() + 1, 1)
    };
    next.map_or(31, |n| (n - first).num_days() as u32)
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Forgets the copies in memory, so the next opening fetches again.
pub fn forget() {
    TEXTS.with(|t| t.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The agenda with a sample calendar, drawn to `$ORBIT_AGENDA_LOOK`
    /// (PNG); on a display of its own, as main.rs's `look`.
    #[test]
    #[ignore]
    fn agenda_look() {
        let Ok(path) = std::env::var("ORBIT_AGENDA_LOOK") else {
            return;
        };
        gtk::init().expect("a display");
        let display = gdk::Display::default().unwrap();
        let _theme = orbit_app::theme::install(&display);
        let css = gtk::CssProvider::new();
        css.load_from_string(include_str!("../../../data/style.css"));
        gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        let now = Local::now();
        let day = |d: i64, h: u32| {
            (now.date_naive() + chrono::Duration::days(d))
                .and_hms_opt(h, 0, 0)
                .unwrap()
                .format("%Y%m%dT%H%M%S")
                .to_string()
        };
        let ics = format!(
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:1\nDTSTART:{}\nDTEND:{}\nRRULE:FREQ=WEEKLY;COUNT=6\nSUMMARY:Reunião de equipe\nLOCATION:Sala 2\nEND:VEVENT\nBEGIN:VEVENT\nUID:2\nDTSTART:{}\nDTEND:{}\nSUMMARY:Dentista\nEND:VEVENT\nBEGIN:VEVENT\nUID:3\nDTSTART;VALUE=DATE:{}\nDTEND;VALUE=DATE:{}\nSUMMARY:Viagem\nEND:VEVENT\nEND:VCALENDAR\n",
            day(0, 9),
            day(0, 10),
            day(0, 15),
            day(0, 16),
            &day(3, 0)[..8],
            &day(5, 0)[..8],
        );
        let url = "https://example.invalid/test.ics".to_string();
        TEXTS.with(|t| t.borrow_mut().insert(url.clone(), (ics, Some(Instant::now()))));
        let config = CalendarConfig {
            calendars: vec![ics::Source { name: "Pessoal".into(), url, colour: None }],
            ..Default::default()
        };
        let anchor = gtk::Window::new();
        let label = gtk::Label::new(Some("x"));
        anchor.set_child(Some(&label));
        open(label.upcast_ref(), &config);
        let window = OPEN.with(|o| o.borrow().upgrade()).unwrap();
        window.set_default_size(1400, 860);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < until {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let (w, h) = (window.width(), window.height());
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(w), f64::from(h));
        let node = snapshot.to_node().unwrap();
        let texture = window.renderer().unwrap().render_texture(&node, None);
        texture.save_to_png(&path).unwrap();
    }
}
