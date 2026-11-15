//! The Appearance page (docs/PLANO-SETTINGS.md §3.3): light or dark, the
//! accent colour, the GTK theme, the icons, the cursor and its size, the
//! fonts (interface, documents, monospace), the text's scale, smoothing and
//! hinting. GSettings' org.gnome.desktop.interface, which the portal hands
//! to every application (Orbit's own follow it live); the cursor goes to
//! core.yaml too, as the core draws it. The wallpaper has its own page.

use std::{collections::BTreeSet, path::PathBuf, rc::Rc};

use gtk::{gio, glib, pango, prelude::*};
use orbit_app::i18n::tr;
use orbit_common::ConfigDir;

use crate::core;

const SCHEMA: &str = "org.gnome.desktop.interface";
const SCHEMES: [(&str, &str); 3] = [
    ("default", "settings-look-auto"),
    ("prefer-light", "settings-look-light"),
    ("prefer-dark", "settings-look-dark"),
];
const ACCENTS: [(&str, &str); 9] = [
    ("blue", "#3584e4"),
    ("teal", "#2190a4"),
    ("green", "#3a944a"),
    ("yellow", "#c88800"),
    ("orange", "#ed5b00"),
    ("red", "#e62d42"),
    ("pink", "#d56199"),
    ("purple", "#9141ac"),
    ("slate", "#6f8396"),
];
const ANTIALIAS: [(&str, &str); 3] = [
    ("grayscale", "settings-look-aa-gray"),
    ("rgba", "settings-look-aa-rgba"),
    ("none", "settings-look-aa-none"),
];
const HINTING: [(&str, &str); 4] = [
    ("none", "settings-look-hint-none"),
    ("slight", "settings-look-hint-slight"),
    ("medium", "settings-look-hint-medium"),
    ("full", "settings-look-hint-full"),
];

/// The settings, when the schema is installed (GSettings aborts on a
/// missing one).
fn settings() -> Option<gio::Settings> {
    gio::SettingsSchemaSource::default()?.lookup(SCHEMA, true)?;
    Some(gio::Settings::new(SCHEMA))
}

/// The folders themes, icons and cursors are looked for in.
fn data_dirs(sub: &str) -> Vec<PathBuf> {
    let home = glib::home_dir();
    let mut dirs = vec![glib::user_data_dir().join(sub), home.join(format!(".{sub}"))];
    dirs.extend(glib::system_data_dirs().into_iter().map(|d| d.join(sub)));
    dirs
}

/// The names of the folders under `sub` that `keep` accepts, sorted.
fn names(sub: &str, keep: impl Fn(&std::path::Path) -> bool) -> Vec<String> {
    let mut found = BTreeSet::new();
    for dir in data_dirs(sub) {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if keep(&path) {
                found.insert(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    found.into_iter().collect()
}

fn gtk_themes() -> Vec<String> {
    names("themes", |p| p.join("gtk-4.0").is_dir() || p.join("gtk-3.0").is_dir())
}

/// Icon themes: an index.theme with folders of icons (not a cursor theme
/// alone).
fn icon_themes() -> Vec<String> {
    names("icons", |p| {
        std::fs::read_to_string(p.join("index.theme"))
            .is_ok_and(|t| t.lines().any(|l| l.starts_with("Directories=") && l.len() > "Directories=".len()))
    })
}

fn cursor_themes() -> Vec<String> {
    names("icons", |p| p.join("cursors").is_dir())
}

struct Page {
    dir: ConfigDir,
    gs: gio::Settings,
    message: gtk::Label,
}

impl Page {
    fn dropdown_of(self: &Rc<Self>, key: &'static str, values: Vec<String>, labels: Vec<String>) -> gtk::DropDown {
        let dropdown = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
        let now = self.gs.string(key);
        dropdown.set_selected(values.iter().position(|v| *v == now.as_str()).map_or(gtk::INVALID_LIST_POSITION, |i| i as u32));
        let page = self.clone();
        dropdown.connect_selected_notify(move |d| {
            if let Some(value) = values.get(d.selected() as usize) {
                page.write(key, value);
            }
        });
        dropdown
    }

    fn write(&self, key: &str, value: &str) {
        let result = self.gs.set_string(key, value).map_err(|e| e.to_string());
        crate::say(&self.message, &result.err().unwrap_or_default());
    }

    /// A font chooser for `key` ("Inter 11").
    fn font(self: &Rc<Self>, key: &'static str) -> gtk::FontDialogButton {
        let button = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
        button.set_font_desc(&pango::FontDescription::from_string(&self.gs.string(key)));
        let page = self.clone();
        button.connect_font_desc_notify(move |b| {
            if let Some(desc) = b.font_desc() {
                page.write(key, &desc.to_string());
            }
        });
        button
    }
}

pub fn page(dir: &ConfigDir) -> gtk::Widget {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();
    let Some(gs) = settings() else {
        body.append(&gtk::Label::builder().label(tr("settings-look-no-schema")).xalign(0.0).wrap(true).build());
        return body.upcast();
    };
    let page = Rc::new(Page {
        dir: dir.clone(),
        gs,
        message: crate::message(),
    });

    let style = crate::power_card(&tr("settings-look-style"));
    let schemes = page.dropdown_of(
        "color-scheme",
        SCHEMES.iter().map(|(v, _)| (*v).to_owned()).collect(),
        SCHEMES.iter().map(|(_, l)| tr(l)).collect(),
    );
    style.append(&crate::row(&tr("settings-look-scheme"), &schemes));
    // The accent: a swatch each, the chosen one marked.
    let swatches = gtk::Box::builder().spacing(6).halign(gtk::Align::End).build();
    let accent = page.gs.string("accent-color");
    let mut first: Option<gtk::ToggleButton> = None;
    for (name, colour) in ACCENTS {
        let button = gtk::ToggleButton::builder()
            .tooltip_text(tr(&format!("settings-look-accent-{name}")))
            .active(accent == name)
            .css_classes(["swatch"])
            .build();
        let area = gtk::DrawingArea::builder().content_width(18).content_height(18).build();
        let rgba = gtk::gdk::RGBA::parse(colour).expect("a colour");
        area.set_draw_func(move |_, cr, w, h| {
            cr.set_source_rgb(rgba.red().into(), rgba.green().into(), rgba.blue().into());
            cr.arc(f64::from(w) / 2.0, f64::from(h) / 2.0, f64::from(w.min(h)) / 2.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        });
        button.set_child(Some(&area));
        match &first {
            Some(group) => button.set_group(Some(group)),
            None => first = Some(button.clone()),
        }
        let page = page.clone();
        button.connect_toggled(move |b| {
            if b.is_active() {
                page.write("accent-color", name);
            }
        });
        swatches.append(&button);
    }
    style.append(&crate::row(&tr("settings-look-accent"), &swatches));
    let themes = gtk_themes();
    style.append(&crate::row(&tr("settings-look-theme"), &page.dropdown_of("gtk-theme", themes.clone(), themes)));
    let icons = icon_themes();
    style.append(&crate::row(&tr("settings-look-icons"), &page.dropdown_of("icon-theme", icons.clone(), icons)));

    // The cursor: GSettings for the applications, core.yaml for the core,
    // which draws it.
    let cursors = cursor_themes();
    let cursor = gtk::DropDown::from_strings(&cursors.iter().map(String::as_str).collect::<Vec<_>>());
    let now = page.gs.string("cursor-theme");
    cursor.set_selected(cursors.iter().position(|c| *c == now.as_str()).map_or(gtk::INVALID_LIST_POSITION, |i| i as u32));
    cursor.connect_selected_notify({
        let page = page.clone();
        move |d| {
            let Some(name) = cursors.get(d.selected() as usize) else { return };
            page.write("cursor-theme", name);
            let told = core::save(&page.dir, "cursor_theme", name.clone().into()).and_then(|()| core::command("reload"));
            crate::say(&page.message, &told.err().map(|e| e.to_string()).unwrap_or_default());
        }
    });
    style.append(&crate::row(&tr("settings-look-cursor"), &cursor));
    let size = gtk::DropDown::from_strings(&["24", "32", "48", "64"]);
    let sizes = [24, 32, 48, 64];
    let now = page.gs.int("cursor-size");
    size.set_selected(sizes.iter().position(|s| *s == now).unwrap_or(0) as u32);
    size.connect_selected_notify({
        let page = page.clone();
        move |d| {
            let Some(px) = sizes.get(d.selected() as usize).copied() else { return };
            let _ = page.gs.set_int("cursor-size", px);
            let told = core::save(&page.dir, "cursor_size", i64::from(px).into()).and_then(|()| core::command("reload"));
            crate::say(&page.message, &told.err().map(|e| e.to_string()).unwrap_or_default());
        }
    });
    style.append(&crate::row(&tr("settings-look-cursor-size"), &size));
    body.append(&style);

    let fonts = crate::power_card(&tr("settings-look-fonts"));
    fonts.append(&crate::row(&tr("settings-look-font-ui"), &page.font("font-name")));
    fonts.append(&crate::row(&tr("settings-look-font-doc"), &page.font("document-font-name")));
    fonts.append(&crate::row(&tr("settings-look-font-mono"), &page.font("monospace-font-name")));
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.75, 2.0, 0.05);
    scale.set_value(page.gs.double("text-scaling-factor"));
    scale.set_hexpand(true);
    scale.set_draw_value(true);
    scale.set_digits(2);
    scale.set_value_pos(gtk::PositionType::Right);
    scale.add_mark(1.0, gtk::PositionType::Bottom, None);
    let pending: Rc<std::cell::Cell<Option<glib::SourceId>>> = Rc::default();
    scale.connect_value_changed({
        let page = page.clone();
        move |s| {
            if let Some(id) = pending.take() {
                id.remove();
            }
            let v = (s.value() * 100.0).round() / 100.0;
            let page = page.clone();
            let again = pending.clone();
            pending.set(Some(glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                again.set(None);
                let _ = page.gs.set_double("text-scaling-factor", v);
            })));
        }
    });
    fonts.append(&crate::row(&tr("settings-look-text-scale"), &scale));
    let aa = page.dropdown_of(
        "font-antialiasing",
        ANTIALIAS.iter().map(|(v, _)| (*v).to_owned()).collect(),
        ANTIALIAS.iter().map(|(_, l)| tr(l)).collect(),
    );
    fonts.append(&crate::row(&tr("settings-look-aa"), &aa));
    let hint = page.dropdown_of(
        "font-hinting",
        HINTING.iter().map(|(v, _)| (*v).to_owned()).collect(),
        HINTING.iter().map(|(_, l)| tr(l)).collect(),
    );
    fonts.append(&crate::row(&tr("settings-look-hint"), &hint));
    body.append(&fonts);
    body.append(&page.message);
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .build()
        .upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_lists_read() {
        // Whatever this machine has: the lists are sorted and without twins.
        for list in [gtk_themes(), icon_themes(), cursor_themes()] {
            let mut sorted = list.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(list, sorted);
        }
    }
}
