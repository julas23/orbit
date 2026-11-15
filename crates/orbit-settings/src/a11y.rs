//! The Accessibility page (docs/PLANO-SETTINGS.md §3.3): what the core
//! does with the keys, the pointer and the picture (core.yaml's
//! `accessibility:`, the core's `a11y.rs`, so it works at the lock too),
//! what the applications read from GNOME's settings (text size, high
//! contrast, less motion), the on-screen keyboard (wvkbd, which the core
//! lets type) and the screen reader (Orca).
//!
//! `orbit-settings screen-reader on|off|toggle` turns Orca on or off: GNOME's
//! `screen-reader-enabled`, Orca started or stopped, and kept in core.yaml's
//! `startup:` (before the greet, so it reads the login too). Mod+Alt+S runs
//! it (the core's default shortcuts).

use std::path::Path;

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::tr;
use orbit_common::ConfigDir;
use orbit_services::Privileged;
use serde_yaml_ng::Value;

use crate::{apps_page, core};

const INTERFACE: &str = "org.gnome.desktop.interface";
const A11Y_INTERFACE: &str = "org.gnome.desktop.a11y.interface";
const A11Y_APPS: &str = "org.gnome.desktop.a11y.applications";
/// The screen reader, and the on-screen keyboard the core lets type.
const ORCA: &str = "/usr/bin/orca";
const KEYBOARD: &str = "/usr/bin/wvkbd-mobintl";
/// Their lines in core.yaml: Orca in `startup:`, the keyboard in `login:`.
const ORCA_LINE: &str = "orca --replace";
const KEYBOARD_LINE: &str = "wvkbd-mobintl -L 280";

const TEXT_SIZES: [f64; 5] = [1.0, 1.25, 1.5, 1.75, 2.0];
const ZOOMS: [f64; 5] = [1.0, 1.5, 2.0, 3.0, 4.0];
const SLOW: [u64; 5] = [0, 150, 300, 500, 1000];
const BOUNCE: [u64; 4] = [0, 100, 300, 500];
const CURSOR_SIZES: [i32; 5] = [24, 32, 48, 64, 96];

fn schema(id: &str) -> Option<gio::Settings> {
    gio::SettingsSchemaSource::default()?.lookup(id, true)?;
    Some(gio::Settings::new(id))
}

/// A key of core.yaml's `accessibility:`.
fn get(dir: &ConfigDir, key: &str) -> Option<Value> {
    core::get(dir, "accessibility").and_then(|a| a.get(key).cloned())
}

/// One key of `accessibility:` written (the others kept), the core told.
fn set(dir: &ConfigDir, key: &str, value: Value) -> anyhow::Result<()> {
    let mut section = core::get(dir, "accessibility").and_then(|v| v.as_mapping().cloned()).unwrap_or_default();
    section.insert(key.into(), value);
    core::save(dir, "accessibility", Value::Mapping(section))?;
    core::command("reload")
}

fn told(message: &gtk::Label, result: anyhow::Result<()>) {
    crate::say(message, &result.err().map(|e| format!("{e:#}")).unwrap_or_default());
}

/// Whether a process named `name` (its 15-letter `comm`) of this user runs.
fn running(name: &str) -> bool {
    std::process::Command::new("pgrep").args(["-x", "-u", &uid(), name]).output().is_ok_and(|o| o.status.success())
}

fn uid() -> String {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }.to_string()
}

/// `line` run apart from Orbit-Settings (it outlives the window).
fn spawn(line: &str) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    std::process::Command::new("sh").args(["-c", line]).process_group(0).stdin(std::process::Stdio::null()).spawn().map(|_| ())
}

fn stop(name: &str) {
    let _ = std::process::Command::new("pkill").args(["-x", "-u", &uid(), name]).status();
}

/// The screen reader on or off, now and from the next session on.
pub fn screen_reader(dir: &ConfigDir, on: bool) -> anyhow::Result<()> {
    anyhow::ensure!(!on || Path::new(ORCA).exists(), "{ORCA} is not installed");
    if let Some(s) = schema(A11Y_APPS) {
        s.set_boolean("screen-reader-enabled", on)?;
        gio::Settings::sync();
    }
    apps_page::ensure_in(dir, "startup", ORCA_LINE, on)?;
    if on && !running("orca") {
        spawn(ORCA_LINE)?;
    } else if !on {
        stop("orca");
    }
    Ok(())
}

/// `orbit-settings screen-reader on|off|toggle`.
pub fn run_screen_reader(config: std::path::PathBuf, what: Option<&str>) -> glib::ExitCode {
    let dir = ConfigDir::new(&config);
    let on = match what {
        Some("on") => true,
        Some("off") => false,
        Some("toggle") | None => !running("orca"),
        Some(other) => {
            eprintln!("screen-reader: on, off or toggle, not {other}");
            return glib::ExitCode::FAILURE;
        }
    };
    match screen_reader(&dir, on) {
        Ok(()) => glib::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("screen-reader: {e:#}");
            glib::ExitCode::FAILURE
        }
    }
}

fn switch(active: bool) -> gtk::Switch {
    gtk::Switch::builder().active(active).halign(gtk::Align::End).valign(gtk::Align::Center).build()
}

fn dim(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["dim"]).build()
}

/// A dropdown of `labels` on the one nearest `now`.
fn choice(values: &[f64], labels: Vec<String>, now: f64) -> gtk::DropDown {
    let drop = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let near = values
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - now).abs().total_cmp(&(b.1 - now).abs()))
        .map_or(0, |(i, _)| i);
    drop.set_selected(near as u32);
    drop
}

fn ms_labels(values: &[u64]) -> Vec<String> {
    values.iter().map(|v| if *v == 0 { tr("settings-a11y-off") } else { format!("{v} ms") }).collect()
}

/// A row with a switch for an `accessibility:` key.
fn core_switch(dir: &ConfigDir, key: &'static str, label: &str, tip: &str, message: &gtk::Label) -> gtk::Box {
    let s = switch(get(dir, key).and_then(|v| v.as_bool()).unwrap_or(false));
    let (dir, message) = (dir.clone(), message.clone());
    s.connect_active_notify(move |s| told(&message, set(&dir, key, s.is_active().into())));
    let row = crate::row(&tr(label), &s);
    row.set_tooltip_text(Some(&tr(tip)));
    row
}

/// A program's row: its switch when installed, else a button installing
/// `package` through the helper.
fn program_row(label: &str, binary: &'static str, package: &'static str, on: bool, toggled: impl Fn(bool) -> anyhow::Result<()> + 'static, message: &gtk::Label) -> gtk::Box {
    let s = switch(on);
    s.set_sensitive(Path::new(binary).exists());
    let message2 = message.clone();
    s.connect_active_notify(move |s| told(&message2, toggled(s.is_active())));
    let controls = gtk::Box::builder().spacing(8).build();
    if !Path::new(binary).exists() {
        let install = gtk::Button::with_label(&tr("settings-a11y-install"));
        let (message, s2) = (message.clone(), s.clone());
        install.connect_clicked(move |b| {
            b.set_sensitive(false);
            crate::say(&message, &tr("settings-installing"));
            let (b, message, s) = (b.clone(), message.clone(), s2.clone());
            crate::helper::run(vec![Privileged::Install(package.into())], |_| {}, move |error| {
                let ok = Path::new(binary).exists();
                s.set_sensitive(ok);
                b.set_visible(!ok);
                b.set_sensitive(true);
                crate::say(&message, &error.unwrap_or_default());
            });
        });
        controls.append(&install);
    }
    controls.append(&s);
    crate::row(&tr(label), &controls)
}

pub fn page(dir: &ConfigDir) -> gtk::Widget {
    let message = crate::message();
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();

    // Seeing.
    let seeing = crate::power_card(&tr("settings-a11y-seeing"));
    if let Some(gs) = schema(INTERFACE) {
        let labels = TEXT_SIZES.iter().map(|v| format!("{:.0}%", v * 100.0)).collect();
        let size = choice(&TEXT_SIZES, labels, gs.double("text-scaling-factor"));
        let (g, m) = (gs.clone(), message.clone());
        size.connect_selected_notify(move |d| {
            let v = TEXT_SIZES.get(d.selected() as usize).copied().unwrap_or(1.0);
            told(&m, g.set_double("text-scaling-factor", v).map_err(Into::into));
        });
        seeing.append(&crate::row(&tr("settings-a11y-text-size"), &size));
        let calm = switch(!gs.boolean("enable-animations"));
        let (g, m, d) = (gs.clone(), message.clone(), dir.clone());
        calm.connect_active_notify(move |s| {
            let result = g
                .set_boolean("enable-animations", !s.is_active())
                .map_err(anyhow::Error::from)
                .and_then(|()| core::save(&d, "animations", (!s.is_active()).into()))
                .and_then(|()| core::command("reload"));
            told(&m, result);
        });
        let row = crate::row(&tr("settings-a11y-less-motion"), &calm);
        row.set_tooltip_text(Some(&tr("settings-a11y-less-motion-tip")));
        seeing.append(&row);
        let labels = CURSOR_SIZES.iter().map(|v| format!("{v} px")).collect();
        let cursor = choice(&CURSOR_SIZES.map(f64::from), labels, f64::from(gs.int("cursor-size")));
        let (g, m, d) = (gs.clone(), message.clone(), dir.clone());
        cursor.connect_selected_notify(move |c| {
            let px = CURSOR_SIZES.get(c.selected() as usize).copied().unwrap_or(24);
            let result = g
                .set_int("cursor-size", px)
                .map_err(anyhow::Error::from)
                .and_then(|()| core::save(&d, "cursor_size", i64::from(px).into()))
                .and_then(|()| core::command("reload"));
            told(&m, result);
        });
        seeing.append(&crate::row(&tr("settings-a11y-cursor-size"), &cursor));
    }
    if let Some(gs) = schema(A11Y_INTERFACE) {
        let contrast = switch(gs.boolean("high-contrast"));
        let m = message.clone();
        contrast.connect_active_notify(move |s| told(&m, gs.set_boolean("high-contrast", s.is_active()).map_err(Into::into)));
        let row = crate::row(&tr("settings-a11y-contrast"), &contrast);
        row.set_tooltip_text(Some(&tr("settings-a11y-contrast-tip")));
        seeing.append(&row);
    }
    let labels = ZOOMS.iter().map(|z| if *z == 1.0 { tr("settings-a11y-off") } else { format!("{z}×") }).collect();
    let zoom = choice(&ZOOMS, labels, get(dir, "zoom").and_then(|v| v.as_f64()).unwrap_or(1.0));
    let (d, m) = (dir.clone(), message.clone());
    zoom.connect_selected_notify(move |z| {
        let v = ZOOMS.get(z.selected() as usize).copied().unwrap_or(1.0);
        told(&m, set(&d, "zoom", v.into()));
    });
    seeing.append(&crate::row(&tr("settings-a11y-zoom"), &zoom));
    seeing.append(&dim(&tr("settings-a11y-zoom-tip")));
    body.append(&seeing);

    // Hearing.
    let hearing = crate::power_card(&tr("settings-a11y-hearing"));
    hearing.append(&core_switch(dir, "visual_bell", "settings-a11y-visual-bell", "settings-a11y-visual-bell-tip", &message));
    body.append(&hearing);

    // Typing.
    let typing = crate::power_card(&tr("settings-a11y-typing"));
    typing.append(&core_switch(dir, "sticky_keys", "settings-a11y-sticky", "settings-a11y-sticky-tip", &message));
    for (key, values, label, tip) in [
        ("slow_keys", &SLOW[..], "settings-a11y-slow", "settings-a11y-slow-tip"),
        ("bounce_keys", &BOUNCE[..], "settings-a11y-bounce", "settings-a11y-bounce-tip"),
    ] {
        let now = get(dir, key).and_then(|v| v.as_u64()).unwrap_or(0);
        let as_f64: Vec<f64> = values.iter().map(|v| *v as f64).collect();
        let drop = choice(&as_f64, ms_labels(values), now as f64);
        let (d, m, values) = (dir.clone(), message.clone(), values.to_vec());
        drop.connect_selected_notify(move |x| {
            let v = values.get(x.selected() as usize).copied().unwrap_or(0);
            told(&m, set(&d, key, v.into()));
        });
        let row = crate::row(&tr(label), &drop);
        row.set_tooltip_text(Some(&tr(tip)));
        typing.append(&row);
    }
    let d = dir.clone();
    let keyboard_on = apps_page::login_items(dir).iter().any(|l| l == KEYBOARD_LINE) || running("wvkbd-mobintl");
    typing.append(&program_row(
        "settings-a11y-osk",
        KEYBOARD,
        "wvkbd",
        keyboard_on,
        move |on| {
            apps_page::ensure_login(&d, KEYBOARD_LINE, on)?;
            if on && !running("wvkbd-mobintl") {
                spawn(KEYBOARD_LINE)?;
            } else if !on {
                stop("wvkbd-mobintl");
            }
            Ok(())
        },
        &message,
    ));
    body.append(&typing);

    // Pointing.
    let pointing = crate::power_card(&tr("settings-a11y-pointing"));
    pointing.append(&core_switch(dir, "mouse_keys", "settings-a11y-mouse-keys", "settings-a11y-mouse-keys-tip", &message));
    pointing.append(&dim(&tr("settings-a11y-mouse-keys-how")));
    body.append(&pointing);

    // The screen reader.
    let reader = crate::power_card(&tr("settings-a11y-reader"));
    let on = schema(A11Y_APPS).is_some_and(|s| s.boolean("screen-reader-enabled")) || running("orca");
    let d = dir.clone();
    reader.append(&program_row("settings-a11y-reader-on", ORCA, "orca", on, move |on| screen_reader(&d, on), &message));
    reader.append(&dim(&tr("settings-a11y-reader-tip")));
    body.append(&reader);

    body.append(&message);
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .build()
        .upcast()
}
