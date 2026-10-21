//! Monitors module: how many monitors there are and where they sit, from
//! the compositor's IPC (`get_outputs`, `output` events). Same shape and
//! sizes as the status modules.
//!
//!   first line   how many monitors
//!   second line  where the others are from the main one ("HDMI-A-1 below")
//!   Full         each monitor's mode
//!
//! The main monitor is the laptop's own panel, or the first one.
//!
//! Left click, the quick part: for each other monitor, four arrows put it
//! above, below, left or right of the main one, centred on it. Right click,
//! the full part: the layout drawn to scale; a monitor dragged and let go
//! snaps to the nearest side of another, and Apply sends it. Both send
//! `output NAME pos X Y` for every monitor, which orbit-core remembers for
//! the next session.

use std::{cell::RefCell, rc::Rc};

use gtk::prelude::*;
use serde::Deserialize;

use orbit_app::i18n::{number, text, tr, tr_args};

use crate::{board::Form, status::Row, sway};

/// None of its own options yet; the file may be empty.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MonitorsConfig {}

#[derive(Debug, Clone, PartialEq)]
struct Monitor {
    name: String,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    /// Hz.
    refresh: f64,
}

struct Module {
    row: Rc<Row>,
    monitors: RefCell<Vec<Monitor>>,
}

pub fn new(_config: &MonitorsConfig, form: Form) -> gtk::Widget {
    let row = Row::new("monitors", form);
    let module = Rc::new(Module {
        row: row.clone(),
        monitors: RefCell::default(),
    });
    module.refresh();
    if sway::available() {
        let weak = Rc::downgrade(&module);
        sway::subscribe(&["output"], move || {
            if let Some(m) = weak.upgrade() {
                m.refresh();
            }
        });
    } else {
        log::warn!("monitors: no compositor IPC");
    }

    let click = gtk::GestureClick::builder().button(0).build();
    {
        let module = module.clone();
        click.connect_pressed(move |gesture, _, _, _| match gesture.current_button() {
            1 => module.quick(),
            3 => module.editor(),
            _ => {}
        });
    }
    row.root.add_controller(click);
    row.root.set_cursor_from_name(Some("pointer"));
    // The subscription holds a weak reference; the widget keeps the module.
    row.root.connect_destroy(move |_| {
        let _ = &module;
    });
    row.root.clone()
}

impl Module {
    fn refresh(&self) {
        let monitors = read();
        self.show(&monitors);
        *self.monitors.borrow_mut() = monitors;
    }

    fn show(&self, monitors: &[Monitor]) {
        let hint = tr("monitors-tooltip");
        let count = tr_args(
            "monitors-count",
            &[("count", number(monitors.len() as u32))],
        );
        let glyph = if monitors.len() > 1 {
            "\u{f037a}"
        } else {
            "\u{f0379}"
        }; // 󰍺 󰍹
        let main = main_index(monitors);
        let places: Vec<String> = monitors
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != main)
            .map(|(_, m)| side_of(m, &monitors[main.unwrap_or(0)]))
            .collect();
        let second = places
            .first()
            .cloned()
            .unwrap_or_else(|| monitors.first().map(|m| m.name.clone()).unwrap_or_default());
        let modes: Vec<String> = monitors.iter().map(mode).collect();
        let mut lines = vec![count.as_str(), second.as_str()];
        lines.extend(modes.iter().take(2).map(String::as_str));
        let tooltip = modes
            .iter()
            .chain(places.iter())
            .cloned()
            .chain([hint])
            .collect::<Vec<_>>()
            .join("\n");
        self.row.show(glyph, &lines, &tooltip);
    }

    /// The quick part: four arrows for each monitor but the main one.
    fn quick(self: &Rc<Self>) {
        let monitors = self.monitors.borrow().clone();
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(5)
            .css_classes(["monitors-quick"])
            .build();
        let main = main_index(&monitors);
        let others: Vec<usize> = (0..monitors.len()).filter(|i| Some(*i) != main).collect();
        if others.is_empty() {
            list.append(&gtk::Label::new(Some(&tr("monitors-single"))));
        }
        let popover = gtk::Popover::builder().child(&list).build();
        for i in others {
            let line = gtk::Box::builder().spacing(5).build();
            let name = gtk::Label::builder()
                .label(&monitors[i].name)
                .xalign(0.0)
                .hexpand(true)
                .build();
            line.append(&name);
            for (glyph, side, tip) in [
                ("\u{f005d}", Side::Above, "monitors-put-above"), // 󰁝
                ("\u{f0045}", Side::Below, "monitors-put-below"), // 󰁅
                ("\u{f004d}", Side::Left, "monitors-put-left"),   // 󰁍
                ("\u{f0054}", Side::Right, "monitors-put-right"), // 󰁔
            ] {
                let button = gtk::Button::builder()
                    .label(glyph)
                    .tooltip_text(tr_args(
                        tip,
                        &[("main", text(monitors[main.unwrap_or(0)].name.clone()))],
                    ))
                    .css_classes(["arrow"])
                    .build();
                let (monitors, popover) = (monitors.clone(), popover.downgrade());
                button.connect_clicked(move |_| {
                    let mut layout = monitors.clone();
                    let main = main_index(&layout).unwrap_or(0);
                    let anchor = layout[main].clone();
                    put(&mut layout[i], &anchor, side, Align::Middle);
                    apply(&layout);
                    if let Some(p) = popover.upgrade() {
                        p.popdown();
                    }
                });
                line.append(&button);
            }
            list.append(&line);
        }
        orbit_app::popup::open(&popover, &self.row.root, None);
    }

    /// The full part: the layout to scale, monitors moved by dragging.
    fn editor(self: &Rc<Self>) {
        let layout = Rc::new(RefCell::new(self.monitors.borrow().clone()));
        let area = gtk::DrawingArea::builder()
            .content_width(EDITOR_W)
            .content_height(EDITOR_H)
            .css_classes(["monitors-editor"])
            .build();
        // While dragging: the monitor and where it was when the drag began.
        let dragging: Rc<RefCell<Option<(usize, i32, i32)>>> = Rc::default();
        // The scale and offset of the last drawing, to map the pointer back.
        let view: Rc<RefCell<View>> = Rc::default();
        {
            let (layout, dragging, view) = (layout.clone(), dragging.clone(), view.clone());
            area.set_draw_func(move |area, cr, w, h| {
                let v = View::fit(&layout.borrow(), w, h);
                draw(
                    area,
                    cr,
                    &layout.borrow(),
                    &v,
                    dragging.borrow().map(|d| d.0),
                );
                *view.borrow_mut() = v;
            });
        }
        let drag = gtk::GestureDrag::new();
        {
            let (layout, dragging, view) = (layout.clone(), dragging.clone(), view.clone());
            drag.connect_drag_begin(move |_, x, y| {
                let (lx, ly) = view.borrow().to_layout(x, y);
                let hit = layout
                    .borrow()
                    .iter()
                    .position(|m| lx >= m.x && lx < m.x + m.w && ly >= m.y && ly < m.y + m.h);
                *dragging.borrow_mut() = hit.map(|i| {
                    let m = &layout.borrow()[i];
                    (i, m.x, m.y)
                });
            });
        }
        {
            let (layout, dragging, view, area) = (
                layout.clone(),
                dragging.clone(),
                view.clone(),
                area.downgrade(),
            );
            drag.connect_drag_update(move |_, dx, dy| {
                let Some((i, x0, y0)) = *dragging.borrow() else {
                    return;
                };
                let scale = view.borrow().scale;
                let mut layout = layout.borrow_mut();
                layout[i].x = x0 + (dx / scale) as i32;
                layout[i].y = y0 + (dy / scale) as i32;
                if let Some(area) = area.upgrade() {
                    area.queue_draw();
                }
            });
        }
        {
            let (layout, dragging, area) = (layout.clone(), dragging.clone(), area.downgrade());
            drag.connect_drag_end(move |_, _, _| {
                let Some((i, _, _)) = dragging.borrow_mut().take() else {
                    return;
                };
                snap(&mut layout.borrow_mut(), i);
                if let Some(area) = area.upgrade() {
                    area.queue_draw();
                }
            });
        }
        area.add_controller(drag);

        let hint = gtk::Label::builder()
            .label(tr("monitors-editor-hint"))
            .css_classes(["hint"])
            .xalign(0.0)
            .hexpand(true)
            .build();
        let apply_button = gtk::Button::builder()
            .label(tr("monitors-apply"))
            .css_classes(["apply"])
            .build();
        let bottom = gtk::Box::builder().spacing(10).build();
        bottom.append(&hint);
        bottom.append(&apply_button);
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["monitors-full"])
            .build();
        card.append(&area);
        card.append(&bottom);
        let popover = gtk::Popover::builder().child(&card).build();
        {
            let popover = popover.downgrade();
            apply_button.connect_clicked(move |_| {
                apply(&layout.borrow());
                if let Some(p) = popover.upgrade() {
                    p.popdown();
                }
            });
        }
        orbit_app::popup::open(&popover, &self.row.root, None);
    }
}

const EDITOR_W: i32 = 300;
const EDITOR_H: i32 = 200;

/// The monitors the compositor has, active ones only.
fn read() -> Vec<Monitor> {
    let outputs: Vec<serde_json::Value> = sway::Connection::open()
        .and_then(|mut c| c.query(sway::GET_OUTPUTS))
        .unwrap_or_default();
    outputs
        .iter()
        .filter(|o| o["active"].as_bool().unwrap_or(true))
        .filter_map(|o| {
            let rect = &o["rect"];
            let n = |v: &serde_json::Value| v.as_i64().map(|v| v as i32);
            Some(Monitor {
                name: o["name"].as_str()?.to_owned(),
                x: n(&rect["x"])?,
                y: n(&rect["y"])?,
                w: n(&rect["width"])?,
                h: n(&rect["height"])?,
                refresh: o["current_mode"]["refresh"].as_f64().unwrap_or(0.0) / 1000.0,
            })
        })
        .collect()
}

/// The laptop's own panel, or the first monitor.
fn main_index(monitors: &[Monitor]) -> Option<usize> {
    if monitors.is_empty() {
        return None;
    }
    let internal = |m: &Monitor| ["eDP", "LVDS", "DSI"].iter().any(|p| m.name.starts_with(p));
    Some(monitors.iter().position(internal).unwrap_or(0))
}

/// "HDMI-A-1 1920×1080 60 Hz".
fn mode(m: &Monitor) -> String {
    format!("{} {}×{} {:.0} Hz", m.name, m.w, m.h, m.refresh)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Above,
    Below,
    Left,
    Right,
}

/// Where along the side of the other monitor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Align {
    Start,
    Middle,
    End,
}

/// "HDMI-A-1 below", from the centres of `m` and `main`.
fn side_of(m: &Monitor, main: &Monitor) -> String {
    let (dx, dy) = (
        (m.x + m.w / 2) - (main.x + main.w / 2),
        (m.y + m.h / 2) - (main.y + main.h / 2),
    );
    let id = if dy.abs() >= dx.abs() {
        if dy > 0 {
            "monitors-below"
        } else {
            "monitors-above"
        }
    } else if dx > 0 {
        "monitors-right"
    } else {
        "monitors-left"
    };
    tr_args(id, &[("name", text(m.name.clone()))])
}

/// Moves `m` against `side` of `anchor`, aligned `align` along it.
fn put(m: &mut Monitor, anchor: &Monitor, side: Side, align: Align) {
    let along = |len: i32, alen: i32, at: i32| match align {
        Align::Start => at,
        Align::Middle => at + (alen - len) / 2,
        Align::End => at + alen - len,
    };
    match side {
        Side::Above | Side::Below => {
            m.x = along(m.w, anchor.w, anchor.x);
            m.y = if side == Side::Above {
                anchor.y - m.h
            } else {
                anchor.y + anchor.h
            };
        }
        Side::Left | Side::Right => {
            m.y = along(m.h, anchor.h, anchor.y);
            m.x = if side == Side::Left {
                anchor.x - m.w
            } else {
                anchor.x + anchor.w
            };
        }
    }
}

/// Puts monitor `i` at the place against another monitor nearest to where
/// it was let go.
fn snap(layout: &mut [Monitor], i: usize) {
    let dropped = layout[i].clone();
    let mut best: Option<(i64, Monitor)> = None;
    for (j, anchor) in layout.iter().enumerate() {
        if j == i {
            continue;
        }
        for side in [Side::Above, Side::Below, Side::Left, Side::Right] {
            for align in [Align::Start, Align::Middle, Align::End] {
                let mut place = dropped.clone();
                put(&mut place, anchor, side, align);
                let overlaps = layout.iter().enumerate().any(|(k, o)| {
                    k != i
                        && place.x < o.x + o.w
                        && o.x < place.x + place.w
                        && place.y < o.y + o.h
                        && o.y < place.y + place.h
                });
                if overlaps {
                    continue;
                }
                let (dx, dy) = (
                    i64::from(place.x - dropped.x),
                    i64::from(place.y - dropped.y),
                );
                let distance = dx * dx + dy * dy;
                if best.as_ref().is_none_or(|(d, _)| distance < *d) {
                    best = Some((distance, place));
                }
            }
        }
    }
    if let Some((_, place)) = best {
        layout[i] = place;
    }
}

/// Sends the layout, moved so that its top-left corner is at 0, 0.
fn apply(layout: &[Monitor]) {
    let (min_x, min_y) = (
        layout.iter().map(|m| m.x).min().unwrap_or(0),
        layout.iter().map(|m| m.y).min().unwrap_or(0),
    );
    let command = layout
        .iter()
        .map(|m| format!("output {} pos {} {}", m.name, m.x - min_x, m.y - min_y))
        .collect::<Vec<_>>()
        .join("; ");
    let sent = sway::Connection::open().and_then(|mut c| c.request(sway::RUN_COMMAND, &command));
    let ok = sent
        .ok()
        .and_then(|r| serde_json::from_slice::<Vec<sway::CommandResult>>(&r).ok())
        .is_some_and(|r| r.iter().all(|r| r.success));
    if !ok {
        log::warn!("monitors: the compositor did not take {command:?}");
    }
}

/// How the layout maps onto the editor's drawing.
#[derive(Clone, Copy, Debug)]
struct View {
    scale: f64,
    dx: f64,
    dy: f64,
}

impl Default for View {
    fn default() -> Self {
        Self {
            scale: 1.0,
            dx: 0.0,
            dy: 0.0,
        }
    }
}

impl View {
    /// Fits the layout's bounds, with room around them to drag into.
    fn fit(layout: &[Monitor], w: i32, h: i32) -> Self {
        if layout.is_empty() {
            return Self::default();
        }
        let x0 = layout.iter().map(|m| m.x).min().unwrap_or(0);
        let y0 = layout.iter().map(|m| m.y).min().unwrap_or(0);
        let x1 = layout.iter().map(|m| m.x + m.w).max().unwrap_or(1);
        let y1 = layout.iter().map(|m| m.y + m.h).max().unwrap_or(1);
        // Half a monitor of room on each side.
        let pad = layout.iter().map(|m| m.w.max(m.h)).max().unwrap_or(0) as f64 / 2.0;
        let (bw, bh) = (
            f64::from(x1 - x0) + 2.0 * pad,
            f64::from(y1 - y0) + 2.0 * pad,
        );
        let scale = (f64::from(w) / bw).min(f64::from(h) / bh);
        Self {
            scale,
            dx: (f64::from(w) - f64::from(x1 - x0) * scale) / 2.0 - f64::from(x0) * scale,
            dy: (f64::from(h) - f64::from(y1 - y0) * scale) / 2.0 - f64::from(y0) * scale,
        }
    }

    fn to_screen(self, x: i32, y: i32) -> (f64, f64) {
        (
            f64::from(x) * self.scale + self.dx,
            f64::from(y) * self.scale + self.dy,
        )
    }

    fn to_layout(self, x: f64, y: f64) -> (i32, i32) {
        (
            ((x - self.dx) / self.scale) as i32,
            ((y - self.dy) / self.scale) as i32,
        )
    }
}

fn draw(
    area: &gtk::DrawingArea,
    cr: &gtk::cairo::Context,
    layout: &[Monitor],
    view: &View,
    held: Option<usize>,
) {
    // The area's CSS colour is the accent; the text takes the popover's.
    let accent = area.color();
    let fg = area.parent().map_or(accent, |p| p.color());
    let rgba = |c: &gtk::gdk::RGBA, a: f64| {
        (
            f64::from(c.red()),
            f64::from(c.green()),
            f64::from(c.blue()),
            a,
        )
    };
    cr.select_font_face(
        "monospace",
        gtk::cairo::FontSlant::Normal,
        gtk::cairo::FontWeight::Bold,
    );
    cr.set_font_size(10.0);
    for (i, m) in layout.iter().enumerate() {
        let (x, y) = view.to_screen(m.x, m.y);
        let (w, h) = (f64::from(m.w) * view.scale, f64::from(m.h) * view.scale);
        let fill = if Some(i) == held { 0.55 } else { 0.30 };
        let (r, g, b, a) = rgba(&accent, fill);
        cr.set_source_rgba(r, g, b, a);
        cr.rectangle(x + 1.0, y + 1.0, w - 2.0, h - 2.0);
        let _ = cr.fill_preserve();
        let (r, g, b, a) = rgba(&accent, 1.0);
        cr.set_source_rgba(r, g, b, a);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        let (r, g, b, a) = rgba(&fg, 1.0);
        cr.set_source_rgba(r, g, b, a);
        for (k, line) in [m.name.clone(), format!("{}×{}", m.w, m.h)]
            .iter()
            .enumerate()
        {
            cr.move_to(x + 5.0, y + 14.0 + 12.0 * k as f64);
            let _ = cr.show_text(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(name: &str, x: i32, y: i32, w: i32, h: i32) -> Monitor {
        Monitor {
            name: name.into(),
            x,
            y,
            w,
            h,
            refresh: 60.0,
        }
    }

    #[test]
    fn put_centres_along_the_side() {
        let laptop = monitor("eDP-1", 0, 0, 1920, 1080);
        let mut big = monitor("HDMI-A-1", 0, 0, 2560, 1440);
        put(&mut big, &laptop, Side::Below, Align::Middle);
        assert_eq!((big.x, big.y), (-320, 1080));
        put(&mut big, &laptop, Side::Right, Align::Start);
        assert_eq!((big.x, big.y), (1920, 0));
    }

    #[test]
    fn a_dropped_monitor_snaps_to_the_nearest_side() {
        let mut layout = vec![
            monitor("eDP-1", 0, 0, 1920, 1080),
            monitor("HDMI-A-1", 1700, 300, 1920, 1080),
        ];
        snap(&mut layout, 1);
        assert_eq!((layout[1].x, layout[1].y), (1920, 0));
        // Dropped on top of the other: still beside it, never over it.
        layout[1].x = 100;
        layout[1].y = 900;
        snap(&mut layout, 1);
        assert_eq!((layout[1].x, layout[1].y), (0, 1080));
    }
}
