//! The Shortcuts page (docs/PLANO-SETTINGS.md §3.3): the core's modifier
//! (`Mod`: Super, Alt or Ctrl, core.yaml), and every shortcut in force —
//! the defaults (orbit-ipc's, the core's own list) with keys.yaml over
//! them — each with what it does and its combination. A combination is
//! changed by pressing the new one; one taken by another shortcut asks
//! first. A shortcut is restored to its default, removed (`none`), or added
//! (a combination and any command). keys.yaml is rewritten only where it
//! changes (comments and the rest kept), and the core reloads.

use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, gio, glib, prelude::*};
use orbit_app::i18n::{tr, tr_args};
use orbit_common::ConfigDir;
use orbit_ipc::keys::DEFAULT;
use serde_yaml_ng::{Mapping, Value};

use crate::core;

const MODIFIERS: [(&str, &str); 3] = [
    ("super", "settings-keys-mod-super"),
    ("alt", "settings-keys-mod-alt"),
    ("ctrl", "settings-keys-mod-ctrl"),
];

/// A combination as the core compares them: the modifiers held (logo,
/// alt, ctrl, shift, with `Mod` resolved) and the key in lowercase.
fn canon(combo: &str, modifier: &str) -> Option<([bool; 4], String)> {
    let mut parts: Vec<&str> = combo.split('+').map(str::trim).collect();
    let key = parts.pop().filter(|k| !k.is_empty())?.to_lowercase();
    let mut held = [false; 4];
    for part in parts {
        let i = match part.to_ascii_lowercase().as_str() {
            "mod" => match modifier {
                "alt" => 1,
                "ctrl" => 2,
                _ => 0,
            },
            "super" | "logo" | "mod4" => 0,
            "alt" | "mod1" => 1,
            "ctrl" | "control" => 2,
            "shift" => 3,
            _ => return None,
        };
        held[i] = true;
    }
    Some((held, key))
}

/// The shortcuts in force, in the defaults' order then the file's:
/// (combination, command, the default's command if it is one).
fn in_force(file: &Mapping, modifier: &str) -> Vec<(String, String, Option<String>)> {
    let mut list: Vec<(String, String, Option<String>)> =
        DEFAULT.iter().map(|(k, c)| ((*k).to_owned(), (*c).to_owned(), Some((*c).to_owned()))).collect();
    for (k, v) in file {
        let (Some(combo), Some(command)) = (k.as_str(), v.as_str()) else { continue };
        let key = canon(combo, modifier);
        match list.iter().position(|(c, _, _)| canon(c, modifier) == key) {
            Some(i) => list[i].1 = command.to_owned(),
            None => list.push((combo.to_owned(), command.to_owned(), None)),
        }
    }
    list.retain(|(_, command, _)| command.trim() != "none");
    list
}

/// What a command does, in words; the command itself when unknown.
fn describe(command: &str) -> String {
    let c = command.trim();
    let n = |prefix: &str| c.strip_prefix(prefix).and_then(|n| n.trim().parse::<i64>().ok());
    if let Some(n) = n("workspace number") {
        return tr_args("settings-keys-do-workspace", &[("area", n.into())]);
    }
    if let Some(n) = n("move container to workspace number") {
        return tr_args("settings-keys-do-move-to", &[("area", n.into())]);
    }
    let id = match c {
        "exec $terminal" => "settings-keys-do-terminal",
        "exec $launcher" => "settings-keys-do-launcher",
        "exec $run" => "settings-keys-do-run",
        "exec orbit-aigent toggle" => "settings-keys-do-aigent",
        "kill" => "settings-keys-do-close",
        "kill --force" => "settings-keys-do-kill",
        "exit" => "settings-keys-do-exit",
        "lock" => "settings-keys-do-lock",
        "fullscreen toggle" => "settings-keys-do-fullscreen",
        "floating toggle" => "settings-keys-do-floating",
        "layout next" => "settings-keys-do-layout",
        "switch workspace" => "settings-keys-do-switch",
        "switch all" => "settings-keys-do-switch-all",
        "switch cards" => "settings-keys-do-cards",
        "switch cards back" => "settings-keys-do-cards-back",
        "maximize" => "settings-keys-do-maximize",
        "restore" => "settings-keys-do-restore",
        "snap left" => "settings-keys-do-snap-left",
        "snap right" => "settings-keys-do-snap-right",
        "focus next" => "settings-keys-do-focus-next",
        "focus prev" => "settings-keys-do-focus-prev",
        _ => "",
    };
    if !id.is_empty() {
        return tr(id);
    }
    for (verb, id) in [
        ("focus ", "settings-keys-do-focus"),
        ("move ", "settings-keys-do-move"),
        ("workspace ", "settings-keys-do-area"),
    ] {
        if let Some(dir) = c.strip_prefix(verb)
            && ["left", "right", "up", "down"].contains(&dir)
        {
            return tr_args(id, &[("direction", tr(&format!("settings-keys-{dir}")).into())]);
        }
    }
    c.to_owned()
}

/// A combination as shown: `Mod` named after the modifier, letters large.
fn shown(combo: &str, modifier: &str) -> String {
    let name = |m: &str| tr(&format!("settings-keys-key-{m}"));
    combo
        .split('+')
        .map(|p| match p.trim() {
            "Mod" => name(modifier),
            k if k.chars().count() == 1 => k.to_uppercase(),
            k => k.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// The combination a key press makes, `Mod` for the core's modifier; `None`
/// for a modifier pressed alone.
fn combo_of(key: gdk::Key, state: gdk::ModifierType, modifier: &str) -> Option<String> {
    let name = key.to_lower().name()?.to_string();
    if name.contains("Shift") || name.contains("Control") || name.contains("Alt") || name.contains("Super") || name.contains("Meta") || name == "ISO_Level3_Shift" {
        return None;
    }
    let mut parts = Vec::new();
    let held = [
        (gdk::ModifierType::SUPER_MASK, "super", "Super"),
        (gdk::ModifierType::ALT_MASK, "alt", "Alt"),
        (gdk::ModifierType::CONTROL_MASK, "ctrl", "Ctrl"),
    ];
    for (mask, id, plain) in held {
        if state.contains(mask) {
            parts.push(if id == modifier { "Mod".to_owned() } else { plain.to_owned() });
        }
    }
    if state.contains(gdk::ModifierType::SHIFT_MASK) {
        parts.push("Shift".to_owned());
    }
    let key = if name.chars().count() == 1 { name.to_uppercase() } else { name };
    parts.push(key);
    Some(parts.join("+"))
}

struct Page {
    dir: ConfigDir,
    list: gtk::Box,
    message: gtk::Label,
    modifier: RefCell<String>,
}

pub fn page(dir: &ConfigDir) -> gtk::Widget {
    let modifier = core::get(dir, "modifier").and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| "super".into());
    let page = Rc::new(Page {
        dir: dir.clone(),
        list: gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).build(),
        message: crate::message(),
        modifier: RefCell::new(modifier.clone()),
    });
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();
    let top = crate::power_card(&tr("settings-keys-mod-card"));
    let labels: Vec<String> = MODIFIERS.iter().map(|(_, l)| tr(l)).collect();
    let picker = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    picker.set_selected(MODIFIERS.iter().position(|(m, _)| *m == modifier).unwrap_or(0) as u32);
    picker.connect_selected_notify({
        let page = page.clone();
        move |p| {
            let Some((m, _)) = MODIFIERS.get(p.selected() as usize) else { return };
            *page.modifier.borrow_mut() = (*m).to_owned();
            let result = core::save(&page.dir, "modifier", (*m).into()).and_then(|()| core::command("reload"));
            crate::say(&page.message, &result.err().map(|e| e.to_string()).unwrap_or_default());
            page.fill();
        }
    });
    let row = crate::row(&tr("settings-keys-mod"), &picker);
    row.set_tooltip_text(Some(&tr("settings-keys-mod-tip")));
    top.append(&row);
    body.append(&top);

    let card = crate::power_card(&tr("settings-keys"));
    card.append(&gtk::Label::builder().label(tr("settings-keys-tip")).xalign(0.0).wrap(true).css_classes(["dim"]).build());
    card.append(&page.list);
    let add = gtk::Button::with_label(&tr("settings-keys-add"));
    add.set_halign(gtk::Align::End);
    add.connect_clicked({
        let page = page.clone();
        move |b| page.add_dialog(b)
    });
    card.append(&add);
    body.append(&card);
    body.append(&page.message);
    page.fill();
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .build()
        .upcast()
}

impl Page {
    fn file(&self) -> Mapping {
        std::fs::read_to_string(self.dir.path().join("keys.yaml"))
            .ok()
            .and_then(|t| serde_yaml_ng::from_str::<Value>(&t).ok())
            .and_then(|v| v.as_mapping().cloned())
            .unwrap_or_default()
    }

    /// Writes `changes` (combination → command, `none`, or `None` to take
    /// the line away) into keys.yaml, then the core reloads.
    fn write(&self, changes: &[(String, Option<String>)]) {
        let mut block = Mapping::new();
        let mut keys = Vec::new();
        for (combo, command) in changes {
            keys.push(combo.as_str());
            if let Some(command) = command {
                block.insert(combo.clone().into(), command.clone().into());
            }
        }
        let text = if block.is_empty() { String::new() } else { serde_yaml_ng::to_string(&block).unwrap_or_default() };
        let result = self.dir.save_app_keys("keys", &keys, &text).and_then(|()| core::command("reload"));
        crate::say(&self.message, &result.err().map(|e| e.to_string()).unwrap_or_default());
    }

    /// The file's line for the shortcut whose combination is `combo`, as
    /// written there (it may be spelt otherwise than the default).
    fn file_key(&self, combo: &str) -> Option<String> {
        let m = self.modifier.borrow().clone();
        let key = canon(combo, &m);
        self.file().keys().filter_map(|k| k.as_str()).find(|k| canon(k, &m) == key).map(str::to_owned)
    }

    fn is_default(combo: &str, modifier: &str) -> bool {
        let key = canon(combo, modifier);
        DEFAULT.iter().any(|(k, _)| canon(k, modifier) == key)
    }

    fn fill(self: &Rc<Self>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let modifier = self.modifier.borrow().clone();
        for (combo, command, default) in in_force(&self.file(), &modifier) {
            let line = gtk::Box::builder().spacing(10).build();
            line.append(&gtk::Label::builder().label(describe(&command)).xalign(0.0).hexpand(true).wrap(true).build());
            // Something to restore: a default's command changed, or a
            // default's command moved to this combination.
            let changed = match &default {
                Some(d) => *d != command,
                None => DEFAULT.iter().any(|(_, d)| *d == command),
            };
            if changed {
                let restore = gtk::Button::builder()
                    .icon_name("edit-undo-symbolic")
                    .tooltip_text(tr("settings-keys-restore"))
                    .css_classes(["flat"])
                    .build();
                let page = self.clone();
                let combo = combo.clone();
                restore.connect_clicked(move |_| page.restore(&combo));
                line.append(&restore);
            }
            let remove = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text(tr("settings-keys-remove"))
                .css_classes(["flat"])
                .build();
            remove.connect_clicked({
                let page = self.clone();
                let combo = combo.clone();
                move |_| page.remove(&combo)
            });
            line.append(&remove);
            let button = gtk::Button::with_label(&shown(&combo, &modifier));
            button.add_css_class("shortcut");
            button.connect_clicked({
                let page = self.clone();
                let combo = combo.clone();
                let command = command.clone();
                move |b| {
                    let page = page.clone();
                    let old = combo.clone();
                    let command = command.clone();
                    page.clone().capture(b, move |new| page.change(&old, &new, &command));
                }
            });
            line.append(&button);
            self.list.append(&line);
        }
    }

    /// Moves `command` from `old` to `new`: the default's old combination
    /// turned off, a file line moved; a combination already taken asks.
    fn change(self: &Rc<Self>, old: &str, new: &str, command: &str) {
        let modifier = self.modifier.borrow().clone();
        if canon(old, &modifier) == canon(new, &modifier) {
            return;
        }
        let taken = in_force(&self.file(), &modifier)
            .into_iter()
            .find(|(c, _, _)| canon(c, &modifier) == canon(new, &modifier));
        let apply = {
            let page = self.clone();
            let (old, new, command) = (old.to_owned(), new.to_owned(), command.to_owned());
            move || {
                let mut changes = Vec::new();
                // The old combination (none for a new shortcut): off when it
                // is a default, gone when it was the file's.
                if !old.is_empty() {
                    let old_line = page.file_key(&old).unwrap_or(old.clone());
                    if Self::is_default(&old, &page.modifier.borrow()) {
                        changes.push((old_line, Some("none".to_owned())));
                    } else {
                        changes.push((old_line, None));
                    }
                }
                let new_line = page.file_key(&new).unwrap_or(new.clone());
                changes.push((new_line, Some(command.clone())));
                page.write(&changes);
                page.fill();
            }
        };
        match taken {
            Some((_, other, _)) => {
                let dialog = gtk::AlertDialog::builder()
                    .message(tr_args("settings-keys-taken", &[("keys", shown(new, &modifier).into()), ("what", describe(&other).into())]))
                    .buttons([tr("settings-svc-cancel"), tr("settings-keys-replace")])
                    .cancel_button(0)
                    .default_button(0)
                    .modal(true)
                    .build();
                let window = self.list.root().and_downcast::<gtk::Window>();
                dialog.choose(window.as_ref(), gio::Cancellable::NONE, move |answer| {
                    if answer == Ok(1) {
                        apply();
                    }
                });
            }
            None => apply(),
        }
    }

    fn remove(self: &Rc<Self>, combo: &str) {
        let line = self.file_key(combo).unwrap_or(combo.to_owned());
        let command = if Self::is_default(combo, &self.modifier.borrow()) { Some("none".to_owned()) } else { None };
        self.write(&[(line, command)]);
        self.fill();
    }

    /// Takes the file's line for `combo` away; and when its command is a
    /// default that moved, that default's combination comes back.
    fn restore(self: &Rc<Self>, combo: &str) {
        let modifier = self.modifier.borrow().clone();
        let mut changes = Vec::new();
        if let Some(line) = self.file_key(combo) {
            changes.push((line, None));
        }
        let file = self.file();
        let command = file
            .iter()
            .find(|(k, _)| k.as_str().is_some_and(|k| canon(k, &modifier) == canon(combo, &modifier)))
            .and_then(|(_, v)| v.as_str().map(str::to_owned));
        if let Some(command) = command
            && let Some((default_combo, _)) = DEFAULT.iter().find(|(_, c)| *c == command)
            && let Some(line) = self.file_key(default_combo)
        {
            changes.push((line, None));
        }
        if !changes.is_empty() {
            self.write(&changes);
        }
        self.fill();
    }

    /// Asks for a combination: the next one pressed (Esc cancels).
    fn capture(self: Rc<Self>, button: &gtk::Button, done: impl Fn(String) + 'static) {
        let window = gtk::Window::builder()
            .title(tr("settings-keys"))
            .modal(true)
            .default_width(360)
            .css_classes(["orbit-settings"])
            .build();
        if let Some(parent) = button.root().and_downcast::<gtk::Window>() {
            window.set_transient_for(Some(&parent));
        }
        let label = gtk::Label::builder().label(tr("settings-keys-press")).wrap(true).build();
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).css_classes(["page"]).build();
        body.append(&label);
        window.set_child(Some(&body));
        let keys = gtk::EventControllerKey::new();
        let modifier = self.modifier.borrow().clone();
        keys.connect_key_pressed({
            let window = window.clone();
            move |_, key, _, state| {
                if key == gdk::Key::Escape && state.is_empty() {
                    window.close();
                    return glib::Propagation::Stop;
                }
                if let Some(combo) = combo_of(key, state, &modifier) {
                    window.close();
                    done(combo);
                }
                glib::Propagation::Stop
            }
        });
        window.add_controller(keys);
        window.present();
    }

    /// A new shortcut: a combination pressed and a command typed.
    fn add_dialog(self: &Rc<Self>, button: &gtk::Button) {
        let window = gtk::Window::builder()
            .title(tr("settings-keys-add"))
            .modal(true)
            .default_width(460)
            .css_classes(["orbit-settings"])
            .build();
        if let Some(parent) = button.root().and_downcast::<gtk::Window>() {
            window.set_transient_for(Some(&parent));
        }
        let combo: Rc<RefCell<Option<String>>> = Rc::default();
        let pick = gtk::Button::with_label(&tr("settings-keys-choose"));
        let command = gtk::Entry::builder().placeholder_text(tr("settings-keys-command-tip")).hexpand(true).build();
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).css_classes(["page"]).build();
        body.append(&crate::row(&tr("settings-keys-combination"), &pick));
        body.append(&crate::row(&tr("settings-keys-command"), &command));
        let cancel = gtk::Button::with_label(&tr("settings-svc-cancel"));
        let add = gtk::Button::with_label(&tr("settings-net-add"));
        add.add_css_class("suggested-action");
        let buttons = gtk::Box::builder().spacing(10).halign(gtk::Align::End).build();
        buttons.append(&cancel);
        buttons.append(&add);
        body.append(&buttons);
        window.set_child(Some(&body));
        pick.connect_clicked({
            let page = self.clone();
            let combo = combo.clone();
            move |b| {
                let combo = combo.clone();
                let modifier = page.modifier.borrow().clone();
                let b2 = b.clone();
                page.clone().capture(b, move |c| {
                    b2.set_label(&shown(&c, &modifier));
                    *combo.borrow_mut() = Some(c);
                });
            }
        });
        cancel.connect_clicked({
            let window = window.clone();
            move |_| window.close()
        });
        add.connect_clicked({
            let page = self.clone();
            let window = window.clone();
            move |_| {
                let (Some(c), text) = (combo.borrow().clone(), command.text().trim().to_owned()) else { return };
                if text.is_empty() {
                    return;
                }
                window.close();
                // An application's name alone is run.
                let text = if text.contains(' ') || text.starts_with("exec") || DEFAULT.iter().any(|(_, d)| *d == text) {
                    text
                } else {
                    format!("exec {text}")
                };
                page.change("", &c, &text);
            }
        });
        window.present();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combinations_compare_as_the_core_does() {
        assert_eq!(canon("Mod+Return", "super"), canon("Super+return", "super"));
        assert_ne!(canon("Mod+Return", "alt"), canon("Super+Return", "alt"));
        assert_eq!(canon("mod+shift+q", "super"), canon("Mod+Shift+Q", "super"));
        assert!(canon("Hyper+X", "super").is_none());
    }

    #[test]
    fn the_file_over_the_defaults() {
        let file: Mapping = serde_yaml_ng::from_str("Mod+Return: exec foot\nMod+Shift+Return: exec kitty\nMod+Q: none\n").unwrap();
        let list = in_force(&file, "super");
        let terminal = list.iter().find(|(c, _, _)| c == "Mod+Return").unwrap();
        assert_eq!(terminal.1, "exec foot");
        assert_eq!(terminal.2.as_deref(), Some("exec $terminal"));
        assert!(list.iter().any(|(c, cmd, d)| c == "Mod+Shift+Return" && cmd == "exec kitty" && d.is_none()));
        assert!(!list.iter().any(|(c, _, _)| c == "Mod+Q"));
    }

    #[test]
    fn commands_said() {
        assert_eq!(describe("exec firefox"), "exec firefox");
        assert!(!describe("workspace number 3").contains("workspace number"));
    }
}
