//! Renaming: one file in place (F2), or many at once, as Thunar's bulk
//! renamer does — find and replace (with regular expressions and their
//! groups), numbering from a pattern ("Viagem ###" → Viagem 001…), text
//! put before or after the name, and upper or lower case. Every new name
//! shows beside the old before anything changes; names that would clash
//! are marked and stop it. Extensions stay as they are.

use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{glib, prelude::*};
use orbit_app::i18n::{text, tr, tr_args};

#[derive(Debug, Clone)]
pub enum Rule {
    Replace { find: String, with: String, regex: bool, case: bool },
    Number { pattern: String, start: u32 },
    Insert { before: String, after: String },
    Case(Case),
}

#[derive(Debug, Clone, Copy)]
pub enum Case {
    Lower,
    Upper,
    Title,
}

/// `name` split into stem and extension (".tar.gz" kept whole).
fn split(name: &str) -> (&str, &str) {
    for double in [".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst"] {
        if name.to_lowercase().ends_with(double) && name.len() > double.len() {
            let i = name.len() - double.len();
            return (&name[..i], &name[i..]);
        }
    }
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// The new names, in order; `Err` for a bad regular expression.
pub fn apply(names: &[String], rule: &Rule) -> Result<Vec<String>, String> {
    let regex = match rule {
        Rule::Replace { find, regex: true, case, .. } if !find.is_empty() => Some(
            regex::RegexBuilder::new(find)
                .case_insensitive(!case)
                .build()
                .map_err(|e| e.to_string())?,
        ),
        _ => None,
    };
    Ok(names
        .iter()
        .enumerate()
        .map(|(n, name)| {
            let (stem, ext) = split(name);
            let stem = match rule {
                Rule::Replace { find, .. } if find.is_empty() => stem.to_owned(),
                Rule::Replace { with, .. } if regex.is_some() => {
                    regex.as_ref().expect("built").replace_all(stem, with.as_str()).into_owned()
                }
                Rule::Replace { find, with, case: true, .. } => stem.replace(find.as_str(), with),
                Rule::Replace { find, with, case: false, .. } => replace_any_case(stem, find, with),
                Rule::Number { pattern, start } => number(pattern, start + n as u32, stem),
                Rule::Insert { before, after } => format!("{before}{stem}{after}"),
                Rule::Case(Case::Lower) => stem.to_lowercase(),
                Rule::Case(Case::Upper) => stem.to_uppercase(),
                Rule::Case(Case::Title) => title(stem),
            };
            format!("{stem}{ext}")
        })
        .collect())
}

/// "Viagem ###" with 7 → "Viagem 007"; "*" stands for the old name.
fn number(pattern: &str, value: u32, old: &str) -> String {
    let width = pattern.chars().filter(|&c| c == '#').count();
    let digits = format!("{value:0width$}", width = width.max(1));
    let mut out = String::new();
    let mut put = false;
    for c in pattern.chars() {
        match c {
            '#' if !put => {
                out.push_str(&digits);
                put = true;
            }
            '#' => {}
            '*' => out.push_str(old),
            c => out.push(c),
        }
    }
    if !put {
        out.push_str(&digits);
    }
    out
}

fn replace_any_case(stem: &str, find: &str, with: &str) -> String {
    let lower = stem.to_lowercase();
    let wanted = find.to_lowercase();
    let mut out = String::new();
    let mut at = 0;
    while let Some(i) = lower[at..].find(&wanted) {
        out.push_str(&stem[at..at + i]);
        out.push_str(with);
        at += i + wanted.len();
    }
    out.push_str(&stem[at..]);
    out
}

fn title(stem: &str) -> String {
    stem.split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(first) => first.to_uppercase().chain(c.flat_map(char::to_lowercase)).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Renames `files` to `names` in two steps — every one to a temporary
/// name, then to its new one — so swapping names cannot collide.
pub fn rename_all(files: &[PathBuf], names: &[String]) -> Vec<String> {
    let mut errors = Vec::new();
    let mut moved = Vec::new();
    for (n, file) in files.iter().enumerate() {
        let temp = file.with_file_name(format!(".orbit-rename-{}-{n}", std::process::id()));
        match std::fs::rename(file, &temp) {
            Ok(()) => moved.push((temp, file.with_file_name(&names[n]), file.clone())),
            Err(err) => errors.push(format!("{}: {err}", file.display())),
        }
    }
    for (temp, new, old) in moved {
        if new.symlink_metadata().is_ok() || std::fs::rename(&temp, &new).is_err() {
            // Back to its old name rather than lost.
            let _ = std::fs::rename(&temp, &old);
            errors.push(format!("{}: {}", new.display(), tr("files-rename-exists")));
        }
    }
    errors
}

// ------------------------------------------------------------- dialogs

/// F2 on one file: a small window with its name, the stem selected.
pub fn single(parent: &gtk::Window, file: &Path) {
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let entry = gtk::Entry::builder().text(&name).width_chars(40).build();
    let window = small_window(parent, &tr("files-rename"), &entry);
    let stem = split(&name).0.chars().count() as i32;
    entry.grab_focus();
    entry.select_region(0, if file.is_dir() { -1 } else { stem });
    let file = file.to_path_buf();
    let w = window.clone();
    entry.connect_activate(move |e| {
        let new = e.text().to_string();
        if !new.is_empty() && new != name && !new.contains('/') {
            let to = file.with_file_name(&new);
            if to.symlink_metadata().is_ok() {
                e.add_css_class("error");
                return;
            }
            if let Err(err) = std::fs::rename(&file, &to) {
                log::warn!("rename: {err}");
            }
        }
        w.close();
    });
    window.present();
}

fn small_window(parent: &gtk::Window, title: &str, child: &impl IsA<gtk::Widget>) -> gtk::Window {
    let window = gtk::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title(title)
        .resizable(false)
        .css_classes(["orbit-files-dialog"])
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(["dialog-box"])
        .build();
    content.append(&gtk::Label::builder().label(title).xalign(0.0).css_classes(["dialog-title"]).build());
    content.append(child);
    window.set_child(Some(&content));
    let keys = gtk::EventControllerKey::new();
    let w = window.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            w.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    window
}

/// The bulk renamer, for `files`.
pub fn bulk(parent: &gtk::Window, files: Vec<PathBuf>) {
    let names: Vec<String> = files
        .iter()
        .map(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
        .collect();
    let mode = gtk::DropDown::from_strings(&[
        &tr("files-rename-replace"),
        &tr("files-rename-number"),
        &tr("files-rename-insert"),
        &tr("files-rename-case"),
    ]);
    let first = gtk::Entry::new();
    let second = gtk::Entry::new();
    let regex = gtk::CheckButton::with_label(&tr("files-rename-regex"));
    let case = gtk::CheckButton::with_label(&tr("files-rename-match-case"));
    let start = gtk::SpinButton::with_range(0.0, 1_000_000.0, 1.0);
    start.set_value(1.0);
    let cases = gtk::DropDown::from_strings(&[&tr("files-case-lower"), &tr("files-case-upper"), &tr("files-case-title")]);
    let options = gtk::Box::builder().spacing(10).build();
    for w in [first.upcast_ref::<gtk::Widget>(), second.upcast_ref(), regex.upcast_ref(), case.upcast_ref(), start.upcast_ref(), cases.upcast_ref()] {
        options.append(w);
    }
    let list = gtk::ListBox::builder().css_classes(["rename-preview"]).selection_mode(gtk::SelectionMode::None).build();
    let scroll = gtk::ScrolledWindow::builder().child(&list).min_content_height(300).min_content_width(600).build();
    let apply_button = gtk::Button::builder().label(tr("files-rename-apply")).css_classes(["suggested-action"]).build();
    let problem = gtk::Label::builder().xalign(0.0).hexpand(true).css_classes(["error-text"]).build();
    let bottom = gtk::Box::builder().spacing(10).build();
    bottom.append(&problem);
    bottom.append(&apply_button);
    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).build();
    content.append(&mode);
    content.append(&options);
    content.append(&scroll);
    content.append(&bottom);
    let window = small_window(parent, &tr_args("files-rename-many", &[("n", orbit_app::i18n::number(files.len() as f64))]), &content);
    window.set_resizable(true);

    struct State {
        names: Vec<String>,
        files: Vec<PathBuf>,
        new: std::cell::RefCell<Vec<String>>,
    }
    let state = Rc::new(State { names, files, new: Default::default() });
    let rule = {
        let (mode, first, second, regex, case, start, cases) =
            (mode.clone(), first.clone(), second.clone(), regex.clone(), case.clone(), start.clone(), cases.clone());
        move || match mode.selected() {
            0 => Rule::Replace { find: first.text().into(), with: second.text().into(), regex: regex.is_active(), case: case.is_active() },
            1 => Rule::Number { pattern: first.text().into(), start: start.value() as u32 },
            2 => Rule::Insert { before: first.text().into(), after: second.text().into() },
            _ => Rule::Case(match cases.selected() {
                0 => Case::Lower,
                1 => Case::Upper,
                _ => Case::Title,
            }),
        }
    };
    let refresh = {
        let (state, list, problem, apply_button) = (state.clone(), list.clone(), problem.clone(), apply_button.clone());
        let (mode, first, second, regex, case, start, cases) =
            (mode.clone(), first.clone(), second.clone(), regex.clone(), case.clone(), start.clone(), cases.clone());
        let rule = rule.clone();
        move || {
            let m = mode.selected();
            first.set_visible(m < 3);
            second.set_visible(m == 0 || m == 2);
            regex.set_visible(m == 0);
            case.set_visible(m == 0);
            start.set_visible(m == 1);
            cases.set_visible(m == 3);
            let (p1, p2) = match m {
                0 => (tr("files-rename-find"), tr("files-rename-with")),
                1 => (tr("files-rename-pattern"), String::new()),
                _ => (tr("files-rename-before"), tr("files-rename-after")),
            };
            first.set_placeholder_text(Some(&p1));
            second.set_placeholder_text(Some(&p2));
            while let Some(row) = list.first_child() {
                list.remove(&row);
            }
            match apply(&state.names, &rule()) {
                Ok(new) => {
                    let clash = |n: &String| new.iter().filter(|x| *x == n).count() > 1 || n.is_empty() || n.contains('/');
                    let mut bad = false;
                    for (old, n) in state.names.iter().zip(&new) {
                        let row = gtk::Box::builder().spacing(10).build();
                        row.append(&gtk::Label::builder().label(old).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::Middle).build());
                        row.append(&gtk::Label::new(Some("→")));
                        let label = gtk::Label::builder().label(n).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::Middle).build();
                        if clash(n) {
                            label.add_css_class("error-text");
                            bad = true;
                        } else if n != old {
                            label.add_css_class("changed");
                        }
                        row.append(&label);
                        list.append(&row);
                    }
                    problem.set_label(&if bad { tr("files-rename-clash") } else { String::new() });
                    apply_button.set_sensitive(!bad && new != state.names);
                    *state.new.borrow_mut() = new;
                }
                Err(err) => {
                    problem.set_label(&tr_args("files-rename-bad-regex", &[("reason", text(err))]));
                    apply_button.set_sensitive(false);
                }
            }
        }
    };
    let refresh = Rc::new(refresh);
    for e in [&first, &second] {
        let r = refresh.clone();
        e.connect_changed(move |_| r());
    }
    for c in [&regex, &case] {
        let r = refresh.clone();
        c.connect_toggled(move |_| r());
    }
    {
        let r = refresh.clone();
        start.connect_value_changed(move |_| r());
    }
    for d in [&mode, &cases] {
        let r = refresh.clone();
        d.connect_selected_notify(move |_| r());
    }
    let w = window.clone();
    apply_button.connect_clicked(move |_| {
        let new = state.new.borrow().clone();
        let errors = rename_all(&state.files, &new);
        if !errors.is_empty() {
            gtk::AlertDialog::builder().modal(true).message(tr("files-job-errors")).detail(errors.join("\n")).build().show(w.transient_for().as_ref());
        }
        w.close();
    });
    refresh();
    window.present();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn numbering_keeps_extensions() {
        let out = apply(&names(&["a.jpg", "b.JPG", "c.tar.gz"]), &Rule::Number { pattern: "Viagem ###".into(), start: 9 }).unwrap();
        assert_eq!(out, ["Viagem 009.jpg", "Viagem 010.JPG", "Viagem 011.tar.gz"]);
    }

    #[test]
    fn replace_plain_any_case_and_regex() {
        let n = names(&["IMG_001.jpg", "img_002.jpg"]);
        let plain = apply(&n, &Rule::Replace { find: "img_".into(), with: "foto-".into(), regex: false, case: false }).unwrap();
        assert_eq!(plain, ["foto-001.jpg", "foto-002.jpg"]);
        let re = apply(&n, &Rule::Replace { find: r"(?i)img_(\d+)".into(), with: "$1-foto".into(), regex: true, case: true }).unwrap();
        assert_eq!(re, ["001-foto.jpg", "002-foto.jpg"]);
        assert!(apply(&n, &Rule::Replace { find: "(".into(), with: String::new(), regex: true, case: true }).is_err());
    }

    #[test]
    fn insert_and_case() {
        let n = names(&["minha FOTO.png"]);
        assert_eq!(apply(&n, &Rule::Insert { before: "2026 ".into(), after: " (1)".into() }).unwrap(), ["2026 minha FOTO (1).png"]);
        assert_eq!(apply(&n, &Rule::Case(Case::Title)).unwrap(), ["Minha Foto.png"]);
    }

    #[test]
    fn swapping_names_does_not_collide() {
        let dir = std::env::temp_dir().join(format!("orbit-files-rename-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a"), b"A").unwrap();
        std::fs::write(dir.join("b"), b"B").unwrap();
        let errors = rename_all(&[dir.join("a"), dir.join("b")], &names(&["b", "a"]));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(std::fs::read(dir.join("a")).unwrap(), b"B");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
