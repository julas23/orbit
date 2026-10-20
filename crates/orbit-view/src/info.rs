//! The information panel, along the right edge: the file (name, folder,
//! format, size, dimensions, date) and, when the image carries EXIF, the
//! camera's (when taken, camera, exposure, place).

use std::path::Path;

use gtk::{glib, prelude::*};
use orbit_app::i18n::{text, tr, tr_args};

use crate::decode::Info;

pub struct Panel {
    pub widget: gtk::Revealer,
    rows: gtk::Box,
}

impl Panel {
    pub fn new() -> Self {
        let rows = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["info"])
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .child(&rows)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(false)
            .width_request(250)
            .build();
        let widget = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideLeft)
            .child(&scroll)
            .halign(gtk::Align::End)
            .build();
        Self { widget, rows }
    }

    /// Fills the panel for `path`, `size` pixels, with what was read.
    pub fn set(&self, path: &Path, size: Option<(i32, i32)>, info: Option<&Info>) {
        while let Some(child) = self.rows.first_child() {
            self.rows.remove(&child);
        }
        for (label, value, link) in rows(path, size, info) {
            let field = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(0)
                .build();
            field.append(
                &gtk::Label::builder()
                    .label(label)
                    .xalign(0.0)
                    .css_classes(["label"])
                    .build(),
            );
            let value_label = gtk::Label::builder()
                .xalign(0.0)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .selectable(true)
                .css_classes(["value"])
                .build();
            match link {
                Some(uri) => value_label.set_markup(&format!(
                    "<a href=\"{}\">{}</a>",
                    glib::markup_escape_text(&uri),
                    glib::markup_escape_text(&value)
                )),
                None => value_label.set_label(&value),
            }
            field.append(&value_label);
            self.rows.append(&field);
        }
    }
}

/// (label, value, link) for each line.
fn rows(
    path: &Path,
    size: Option<(i32, i32)>,
    info: Option<&Info>,
) -> Vec<(String, String, Option<String>)> {
    let mut rows = Vec::new();
    let mut add = |id: &str, value: String| rows.push((tr(id), value, None));
    if let Some(name) = path.file_name() {
        add("view-info-name", name.to_string_lossy().into_owned());
    }
    if let Some(dir) = path.parent() {
        add("view-info-folder", home_relative(dir));
    }
    if let Some(format) = info.and_then(|i| i.format.clone()) {
        add("view-info-format", format);
    }
    if let Some((w, h)) = size {
        add("view-info-dimensions", format!("{w} × {h}"));
    }
    if let Ok(meta) = std::fs::metadata(path) {
        add("view-info-file-size", glib::format_size(meta.len()).to_string());
        if let Some(when) = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| glib::DateTime::from_unix_local(d.as_secs() as i64).ok())
            .and_then(|d| d.format("%x %H:%M").ok())
        {
            add("view-info-modified", when.to_string());
        }
    }

    let Some(exif) = info
        .and_then(|i| i.exif.clone())
        .and_then(|data| gufo_exif::Exif::new(data).ok())
    else {
        return rows;
    };
    if let Some(taken) = exif.date_time_original().and_then(|d| local_date(&d)) {
        add("view-info-taken", taken);
    }
    let make = exif.make().map(|m| m.trim().to_owned()).filter(|m| !m.is_empty());
    let model = exif.model().map(|m| m.trim().to_owned()).filter(|m| !m.is_empty());
    let camera = match (make, model) {
        // Most models already start with the maker ("Canon EOS R6").
        (Some(make), Some(model))
            if model
                .to_lowercase()
                .starts_with(make.to_lowercase().split_whitespace().next().unwrap_or("")) =>
        {
            Some(model)
        }
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (one, other) => one.or(other),
    };
    if let Some(camera) = camera {
        add("view-info-camera", camera);
    }
    let mut exposure = Vec::new();
    if let Some((n, d)) = exif.exposure_time()
        && n > 0
        && d > 0
    {
        exposure.push(if n == 1 || d / n >= 2 {
            format!("1/{} s", (f64::from(d) / f64::from(n)).round())
        } else {
            format!("{} s", decimal(f64::from(n) / f64::from(d)))
        });
    }
    if let Some(f) = exif.f_number() {
        exposure.push(format!("f/{}", decimal(f64::from(f))));
    }
    if let Some(iso) = exif.iso_speed_rating() {
        exposure.push(format!("ISO {iso}"));
    }
    if let Some(mm) = exif.focal_length() {
        exposure.push(format!("{} mm", decimal(f64::from(mm))));
    }
    if !exposure.is_empty() {
        add("view-info-exposure", exposure.join(" · "));
    }
    if let Some(software) = exif.software().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()) {
        add("view-info-software", software);
    }
    if let Some(place) = exif.gps_location() {
        let (lat, lon) = (place.lat.0, place.lon.0);
        let shown = format!(
            "{} {}",
            coordinate(lat, "view-info-north", "view-info-south"),
            coordinate(lon, "view-info-east", "view-info-west")
        );
        let link = format!("https://www.openstreetmap.org/?mlat={lat:.6}&mlon={lon:.6}#map=16/{lat:.6}/{lon:.6}");
        rows.push((tr("view-info-place"), shown, Some(link)));
    }
    rows
}

/// `~/Pictures/2024` rather than `/home/me/Pictures/2024`.
fn home_relative(dir: &Path) -> String {
    match dir.strip_prefix(glib::home_dir()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => dir.display().to_string(),
    }
}

/// One decimal at most, with the language's decimal separator.
fn decimal(x: f64) -> String {
    let s = format!("{:.1}", x);
    let s = s.strip_suffix(".0").unwrap_or(&s).to_owned();
    let separator = tr("calc-decimal-separator");
    s.replace('.', &separator)
}

/// 23°33′02″ S.
fn coordinate(value: f64, positive: &str, negative: &str) -> String {
    let side = tr(if value >= 0.0 { positive } else { negative });
    let value = value.abs();
    let degrees = value.floor();
    let minutes = ((value - degrees) * 60.0).floor();
    let seconds = ((value - degrees) * 60.0 - minutes) * 60.0;
    tr_args(
        "view-info-coordinate",
        &[
            ("degrees", text(format!("{degrees:.0}"))),
            ("minutes", text(format!("{minutes:02.0}"))),
            ("seconds", text(format!("{:02.0}", seconds.floor()))),
            ("side", text(side)),
        ],
    )
}

/// The EXIF date in the language's format; it carries no time zone, or
/// one that is the camera's own, so it is shown as written.
fn local_date(date: &gufo_common::datetime::DateTime) -> Option<String> {
    use chrono::{Datelike, Timelike};
    let naive = match date {
        gufo_common::datetime::DateTime::FixedOffset(d) => d.naive_local(),
        gufo_common::datetime::DateTime::Naive(d) => *d,
    };
    let local = glib::DateTime::from_local(
        naive.year(),
        naive.month() as i32,
        naive.day() as i32,
        naive.hour() as i32,
        naive.minute() as i32,
        f64::from(naive.second()),
    )
    .ok()?;
    Some(local.format("%x %H:%M").ok()?.to_string())
}
