//! The advanced editor (docs/PLANO-SETTINGS.md §3.3, "Editor avançado"):
//! every YAML file of Orbit's (`~/.config/orbit/*.yaml` and
//! `modules/*.yaml`), raw, as XFCE's settings editor shows its own. The
//! text is saved as typed, so its comments stay; but only once it reads:
//! as YAML first, then as what reads it (core.yaml by `orbit-core
//! --check`, which a bad file would otherwise send back to the defaults;
//! the panel's and the dock's by their own types; keys.yaml as
//! combinations and commands). An error is said with its line, and the
//! cursor goes there. Saved, the core reads its files again; the panel, the
//! dock and the modules see theirs change.

use std::{cell::RefCell, path::PathBuf, rc::Rc};

use gtk::{glib, prelude::*};
use orbit_app::i18n::{tr, tr_args};
use orbit_common::ConfigDir;
use serde_yaml_ng::Value;

use crate::core;

/// The files, by name relative to the config folder (`modules/clock.yaml`).
fn files(dir: &ConfigDir) -> Vec<String> {
    let list = |sub: &str| -> Vec<String> {
        let folder = if sub.is_empty() { dir.path().to_path_buf() } else { dir.path().join(sub) };
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".yaml") && !n.starts_with('.'))
            .map(|n| if sub.is_empty() { n } else { format!("{sub}/{n}") })
            .collect();
        names.sort();
        names
    };
    // The core's first, then the others.
    let first = ["core.yaml", "keys.yaml", "panel.yaml", "dock.yaml"];
    let mut all = list("");
    all.sort_by_key(|n| first.iter().position(|f| f == n).unwrap_or(first.len()));
    all.extend(list("modules"));
    all
}

/// A problem with the text: what, and on which line (1-based) if known.
#[derive(Debug, PartialEq)]
struct Problem {
    text: String,
    line: Option<usize>,
}

/// The line where `key` starts at the top level, for an error that names
/// a field but no place.
fn line_of_key(text: &str, key: &str) -> Option<usize> {
    text.lines().position(|l| l.starts_with(&format!("{key}:"))).map(|i| i + 1)
}

/// The line serde names in its message (`at line N column M`), or of the
/// unknown field it names.
fn line_in(message: &str, text: &str) -> Option<usize> {
    if let Some(rest) = message.split(" at line ").nth(1) {
        return rest.split_whitespace().next().and_then(|n| n.parse().ok());
    }
    let field = message.strip_prefix("unknown field `")?.split('`').next()?;
    line_of_key(text, field)
}

fn problem(message: String, text: &str) -> Problem {
    let line = line_in(&message, text);
    Problem { text: message, line }
}

/// Whether `text` would read as file `name`.
fn check(name: &str, text: &str) -> Result<(), Problem> {
    if text.trim().is_empty() {
        return Ok(());
    }
    let value: Value = serde_yaml_ng::from_str(text).map_err(|e| problem(e.to_string(), text))?;
    match name {
        "core.yaml" => check_core(text),
        "panel.yaml" => serde_yaml_ng::from_str::<orbit_common::config::PanelConfig>(text).map(|_| ()).map_err(|e| problem(e.to_string(), text)),
        "dock.yaml" => serde_yaml_ng::from_str::<orbit_common::config::DockConfig>(text).map(|_| ()).map_err(|e| problem(e.to_string(), text)),
        "keys.yaml" => {
            let map = value.as_mapping().ok_or_else(|| problem(tr("settings-advanced-keys-map"), text))?;
            for (key, command) in map {
                if key.as_str().is_none() || !command.is_string() {
                    let shown = key.as_str().map_or_else(|| format!("{key:?}"), str::to_owned);
                    return Err(Problem {
                        text: tr_args("settings-advanced-keys-entry", &[("key", shown.clone().into())]),
                        line: line_of_key(text, &shown),
                    });
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// core.yaml read by the core itself (`orbit-core --check`), from a copy.
fn check_core(text: &str) -> Result<(), Problem> {
    let copy = glib::user_runtime_dir().join(format!("orbit-settings-check-{}.yaml", std::process::id()));
    std::fs::write(&copy, text).map_err(|e| Problem { text: e.to_string(), line: None })?;
    let out = std::process::Command::new("orbit-core").arg("--check").arg(&copy).output();
    let _ = std::fs::remove_file(&copy);
    match out {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(problem(String::from_utf8_lossy(&out.stdout).trim().to_owned(), text)),
        // No core to ask (or one without --check): the YAML alone.
        Err(_) => Ok(()),
    }
}

/// Writes `text` whole, through a file beside it, so a reader never sees
/// half of it.
fn write(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = path.with_file_name(format!(".{name}.orbit-settings"));
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, path)
}

struct Page {
    dir: ConfigDir,
    names: Vec<String>,
    pick: gtk::DropDown,
    view: gtk::TextView,
    status: gtk::Label,
    /// The file shown, and its text as last read or saved.
    shown: RefCell<Option<(String, String)>>,
}

impl Page {
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn text(&self) -> String {
        let buffer = self.view.buffer();
        buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string()
    }

    fn changed(&self) -> bool {
        self.shown.borrow().as_ref().is_some_and(|(_, saved)| *saved != self.text())
    }

    fn say(&self, text: &str, error: bool) {
        self.status.set_label(text);
        if error {
            self.status.add_css_class("error");
        } else {
            self.status.remove_css_class("error");
        }
    }

    /// File `name` into the view.
    fn open(&self, name: &str) {
        let text = std::fs::read_to_string(self.path(name)).unwrap_or_default();
        let buffer = self.view.buffer();
        buffer.set_text(&text);
        buffer.place_cursor(&buffer.start_iter());
        *self.shown.borrow_mut() = Some((name.to_owned(), text));
        self.say(&self.path(name).display().to_string(), false);
    }

    fn selected(&self) -> Option<String> {
        self.names.get(self.pick.selected() as usize).cloned()
    }

    fn save(&self) {
        let Some((name, _)) = self.shown.borrow().clone() else { return };
        let text = self.text();
        let buffer = self.view.buffer();
        buffer.remove_tag_by_name("error-line", &buffer.start_iter(), &buffer.end_iter());
        if let Err(p) = check(&name, &text) {
            let place = p.line.map(|l| tr_args("settings-advanced-line", &[("line", (l as i64).into())]));
            self.say(&format!("{}{}", place.map(|p| format!("{p}: ")).unwrap_or_default(), p.text), true);
            if let Some(line) = p.line
                && let Some(start) = buffer.iter_at_line(line.saturating_sub(1) as i32)
            {
                let mut end = start;
                end.forward_to_line_end();
                buffer.apply_tag_by_name("error-line", &start, &end);
                buffer.place_cursor(&start);
                self.view.scroll_to_iter(&mut start.clone(), 0.1, false, 0.0, 0.0);
                self.view.grab_focus();
            }
            return;
        }
        if let Err(e) = write(&self.path(&name), &text) {
            self.say(&e.to_string(), true);
            return;
        }
        *self.shown.borrow_mut() = Some((name.clone(), text));
        // The core reads core.yaml and keys.yaml when told; the rest is
        // watched by its readers.
        let told = if name == "core.yaml" || name == "keys.yaml" { core::command("reload").err() } else { None };
        match told {
            Some(e) => self.say(&format!("{}: {e:#}", tr("settings-not-told")), true),
            None => self.say(&tr("settings-advanced-saved"), false),
        }
    }
}

pub fn page(dir: &ConfigDir) -> gtk::Widget {
    let names = files(dir);
    let pick = crate::short_dropdown(&names);
    let view = gtk::TextView::builder()
        .monospace(true)
        .wrap_mode(gtk::WrapMode::None)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .vexpand(true)
        .hexpand(true)
        .build();
    let tag = gtk::TextTag::builder().name("error-line").underline(gtk::pango::Underline::Error).build();
    view.buffer().tag_table().add(&tag);
    let page = Rc::new(Page {
        dir: dir.clone(),
        names,
        pick,
        view,
        status: gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["dim"]).build(),
        shown: RefCell::new(None),
    });

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .css_classes(["page"])
        .build();
    let card = crate::power_card(&tr("settings-advanced"));
    card.set_vexpand(true);
    card.append(&gtk::Label::builder().label(tr("settings-advanced-tip")).xalign(0.0).wrap(true).css_classes(["dim"]).build());
    let revert = gtk::Button::with_label(&tr("settings-advanced-revert"));
    let save = gtk::Button::with_label(&tr("settings-advanced-save"));
    save.add_css_class("suggested-action");
    let top = gtk::Box::builder().spacing(10).build();
    top.append(&gtk::Label::builder().label(tr("settings-advanced-file")).xalign(0.0).build());
    top.append(&page.pick);
    top.append(&gtk::Box::builder().hexpand(true).build());
    top.append(&revert);
    top.append(&save);
    card.append(&top);
    let scroller = gtk::ScrolledWindow::builder().child(&page.view).vexpand(true).min_content_height(360).css_classes(["frame"]).build();
    card.append(&scroller);
    card.append(&page.status);
    body.append(&card);

    if let Some(first) = page.selected() {
        page.open(&first);
    }
    page.pick.connect_selected_notify({
        let page = page.clone();
        move |d| {
            let Some(name) = page.selected() else { return };
            if page.shown.borrow().as_ref().is_some_and(|(n, _)| *n == name) {
                return;
            }
            if !page.changed() {
                page.open(&name);
                return;
            }
            // Unsaved changes: asked first; kept, the list goes back.
            let ask = gtk::AlertDialog::builder()
                .message(tr("settings-advanced-discard"))
                .buttons([tr("settings-svc-cancel"), tr("settings-advanced-discard-yes")])
                .cancel_button(0)
                .default_button(0)
                .modal(true)
                .build();
            let window = d.root().and_downcast::<gtk::Window>();
            let page = page.clone();
            glib::spawn_future_local(async move {
                if ask.choose_future(window.as_ref()).await == Ok(1) {
                    page.open(&name);
                } else {
                    let back = page.shown.borrow().as_ref().and_then(|(n, _)| page.names.iter().position(|x| x == n));
                    if let Some(i) = back {
                        page.pick.set_selected(i as u32);
                    }
                }
            });
        }
    });
    revert.connect_clicked({
        let page = page.clone();
        move |_| {
            if let Some(name) = page.selected() {
                page.open(&name);
            }
        }
    });
    save.connect_clicked({
        let page = page.clone();
        move |_| page.save()
    });
    // Ctrl+S saves.
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let page = page.clone();
        move |_, key, _, mods| {
            if mods.contains(gtk::gdk::ModifierType::CONTROL_MASK) && key == gtk::gdk::Key::s {
                page.save();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    });
    page.view.add_controller(keys);
    // Shown again: what other pages wrote since, unless there are changes.
    body.connect_map({
        let page = page.clone();
        move |_| {
            let shown = page.shown.borrow().clone();
            if !page.changed()
                && let Some((name, _)) = shown
            {
                page.open(&name);
            }
        }
    });
    body.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problems_have_lines() {
        let p = check("x.yaml", "a: 1\nb: [\n").unwrap_err();
        assert!(p.line.is_some());
        assert_eq!(line_in("unknown field `gapz`, expected one of …", "terminal: x\ngapz: 3\n"), Some(2));
        assert_eq!(line_in("gaps: invalid type: sequence, expected i32 at line 4 column 7", ""), Some(4));
        assert!(check("keys.yaml", "Mod+Return: orbit-term\n").is_ok());
        let p = check("keys.yaml", "Mod+q: [kill]\n").unwrap_err();
        assert_eq!(p.line, Some(1));
        assert!(check("panel.yaml", "edgee: top\n").is_err() || check("panel.yaml", "edge: top\n").is_ok());
        assert!(check("anything.yaml", "").is_ok());
    }
}
