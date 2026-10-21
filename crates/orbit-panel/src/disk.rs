//! Disk module: how full a filesystem is, as a pie — used in red, free in
//! blue — with the numbers beside it.
//!
//!   Mini    (2 × 1)  the pie and the used percentage
//!   Normal  (4 × 1)  the pie, the mount point and used %, used of total
//!                    and free
//!   Full    (4 × 2)  a larger pie; the mount point and used percentage,
//!                    total, used and free (the type in the tooltip)
//!
//! Each entry in `panel.yaml` may show its own filesystem
//! (`- disk: { size: full, path: /home }`, picked in the panel's editor
//! from `mounts`); `disk.yaml`'s `path` is the default.
//!
//! Read through GIO (`filesystem::size` and `filesystem::free`, off the GTK
//! thread so a slow mount cannot freeze the panel) every `interval`
//! seconds.

use std::{cell::Cell, f64::consts::PI, rc::Rc};

use gtk::{cairo, gio, glib, prelude::*};
use orbit_app::i18n::tr;
use serde::Deserialize;

use crate::{board::Form, meters::short};

/// `modules/disk.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiskConfig {
    /// Any path on the filesystem to show.
    pub path: String,
    /// Seconds between readings.
    pub interval: u32,
}

impl Default for DiskConfig {
    fn default() -> Self {
        Self {
            path: "/".into(),
            interval: 30,
        }
    }
}

const USED: (f64, f64, f64) = (0.86, 0.22, 0.20);
const FREE: (f64, f64, f64) = (0.27, 0.51, 0.71);

struct Module {
    form: Form,
    path: String,
    pie: gtk::DrawingArea,
    /// Used, of 1.
    fraction: Rc<Cell<f64>>,
    lines: Vec<gtk::Label>,
}

pub fn new(config: &DiskConfig, form: Form) -> gtk::Widget {
    let side = match form {
        Form::Full => 54,
        _ => 24,
    };
    let fraction = Rc::new(Cell::new(0.0));
    let pie = gtk::DrawingArea::builder()
        .content_width(side)
        .content_height(side)
        .valign(gtk::Align::Center)
        .build();
    {
        let fraction = fraction.clone();
        pie.set_draw_func(move |_, cr, w, h| {
            if let Err(err) = draw_pie(cr, fraction.get(), f64::from(w), f64::from(h)) {
                log::warn!("disk: drawing: {err}");
            }
        });
    }
    let count = match form {
        Form::Full => 4,
        Form::Normal => 2,
        _ => 1,
    };
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    let lines: Vec<gtk::Label> = (0..count)
        .map(|i| {
            let label = gtk::Label::builder()
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes([if i == 0 && form == Form::Full { "head" } else { "line" }])
                .build();
            text.append(&label);
            label
        })
        .collect();
    let root = gtk::Box::builder()
        .css_classes(["disk", form.class()])
        .spacing(if form == Form::Full { 8 } else { 5 })
        .build();
    root.append(&pie);
    root.append(&text);
    if form == Form::Line {
        root.set_size_request(70, 24);
    }

    let module = Rc::new(Module {
        form,
        path: config.path.clone(),
        pie,
        fraction,
        lines,
    });
    module.refresh();
    let weak = root.downgrade();
    glib::timeout_add_seconds_local(config.interval.max(1), move || {
        if weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        module.refresh();
        glib::ControlFlow::Continue
    });
    root.upcast()
}

impl Module {
    fn refresh(self: &Rc<Self>) {
        let file = gio::File::for_path(orbit_common::config::expand_home(self.path.as_ref()));
        let module = self.clone();
        glib::spawn_future_local(async move {
            let attrs = "filesystem::size,filesystem::free,filesystem::type";
            match file.query_filesystem_info_future(attrs, glib::Priority::DEFAULT).await {
                Ok(info) => {
                    let total = info.attribute_uint64("filesystem::size") as f64;
                    let free = info.attribute_uint64("filesystem::free") as f64;
                    let kind = info.attribute_string("filesystem::type").map(|t| t.to_string());
                    module.show(total, free, kind.as_deref());
                }
                Err(err) => log::warn!("disk: {}: {err}", module.path),
            }
        });
    }

    fn show(&self, total: f64, free: f64, kind: Option<&str>) {
        let place = match kind {
            Some(kind) => format!("{} ({kind})", self.path),
            None => self.path.clone(),
        };
        let used = (total - free).max(0.0);
        let fraction = if total > 0.0 { used / total } else { 0.0 };
        let percent = fraction * 100.0;
        self.fraction.set(fraction);
        self.pie.queue_draw();
        let lines: Vec<String> = match self.form {
            Form::Full => vec![
                format!("{} · {percent:.0}%", self.path),
                format!("{} {}", tr("disk-total"), short(total)),
                format!("{} {}", tr("disk-used"), short(used)),
                format!("{} {}", tr("disk-free"), short(free)),
            ],
            // Its place first: there may be more than one disk.
            Form::Normal => vec![
                format!("{} · {percent:.0}%", self.path),
                format!("{} / {} · {} {}", short(used), short(total), tr("disk-free"), short(free)),
            ],
            _ => vec![format!("{percent:.0}%")],
        };
        for (label, text) in self.lines.iter().zip(lines) {
            label.set_label(&text);
        }
        self.pie.set_tooltip_text(Some(&format!(
            "{place}\n{} {}\n{} {} ({percent:.0}%)\n{} {}",
            tr("disk-total"),
            short(total),
            tr("disk-used"),
            short(used),
            tr("disk-free"),
            short(free)
        )));
    }
}

/// Where the disks' filesystems are mounted (devices under /dev, but
/// snaps' and AppImages' images), in /proc/self/mounts' order.
pub fn mounts() -> Vec<String> {
    let text = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let mut mounts: Vec<String> = Vec::new();
    for line in text.lines() {
        let mut f = line.split_whitespace();
        let (Some(device), Some(point), Some(kind)) = (f.next(), f.next(), f.next()) else { continue };
        if !device.starts_with("/dev/") || device.starts_with("/dev/loop") || kind == "squashfs" {
            continue;
        }
        let point = unescape(point);
        if !mounts.contains(&point) {
            mounts.push(point);
        }
    }
    mounts
}

/// /proc/mounts writes space, tab, newline and backslash as `\ooo`.
fn unescape(field: &str) -> String {
    let mut out = String::new();
    let mut chars = field.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let digits: String = chars.clone().take(3).collect();
            if digits.len() == 3
                && let Ok(byte) = u8::from_str_radix(&digits, 8)
            {
                out.push(byte as char);
                chars.nth(2);
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// Used from 12 o'clock clockwise in red, the rest in blue.
fn draw_pie(cr: &cairo::Context, fraction: f64, w: f64, h: f64) -> Result<(), cairo::Error> {
    let (cx, cy) = (w / 2.0, h / 2.0);
    let r = w.min(h) / 2.0 - 0.5;
    let start = -PI / 2.0;
    let turn = start + 2.0 * PI * fraction.clamp(0.0, 1.0);
    cr.set_source_rgb(FREE.0, FREE.1, FREE.2);
    cr.arc(cx, cy, r, 0.0, 2.0 * PI);
    cr.fill()?;
    if fraction > 0.0 {
        cr.set_source_rgb(USED.0, USED.1, USED.2);
        cr.move_to(cx, cy);
        cr.arc(cx, cy, r, start, turn);
        cr.close_path();
        cr.fill()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_points_are_unescaped() {
        assert_eq!(unescape("/mnt/my\\040disk"), "/mnt/my disk");
        assert_eq!(unescape("/home"), "/home");
        assert!(mounts().contains(&"/".to_string()));
    }
}
