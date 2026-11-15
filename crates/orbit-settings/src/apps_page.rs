//! The Default apps and startup page (docs/PLANO-SETTINGS.md §3.3): the
//! application for each kind of file and link (`mimeapps.list`, through
//! GIO, as every other program reads it), the terminal Mod+Return opens
//! (core.yaml `terminal:`), and what runs on entering (core.yaml
//! `login:`), edited line by line so the comments in that list stay.

use std::rc::Rc;

use gtk::{gio, prelude::*};
use orbit_app::i18n::tr;
use orbit_common::ConfigDir;
use serde_yaml_ng::Value;

use crate::core;

/// The kinds offered, each with the types its choice is made the default
/// for; the first names the applications offered.
const KINDS: [(&str, &[&str]); 8] = [
    ("settings-apps-browser", &["x-scheme-handler/http", "x-scheme-handler/https", "text/html"]),
    ("settings-apps-mail", &["x-scheme-handler/mailto"]),
    ("settings-apps-files", &["inode/directory"]),
    ("settings-apps-text", &["text/plain"]),
    ("settings-apps-pdf", &["application/pdf"]),
    ("settings-apps-images", &["image/png", "image/jpeg", "image/gif", "image/webp", "image/bmp", "image/tiff"]),
    ("settings-apps-music", &["audio/mpeg", "audio/flac", "audio/ogg", "audio/x-wav", "audio/mp4", "audio/x-vorbis+ogg"]),
    ("settings-apps-video", &["video/mp4", "video/x-matroska", "video/webm", "video/quicktime", "video/x-msvideo"]),
];

/// A dropdown of the applications for the first of `types`, on its
/// current default; choosing one makes it the default for each of `types`
/// it handles.
fn chooser(types: Vec<String>, message: &gtk::Label) -> gtk::DropDown {
    let mime = types[0].clone();
    let mut apps = gio::AppInfo::all_for_type(&mime);
    apps.sort_by_key(|a| a.display_name().to_lowercase());
    let current = gio::AppInfo::default_for_type(&mime, false).and_then(|a| a.id());
    let mut labels: Vec<String> = apps.iter().map(|a| a.display_name().to_string()).collect();
    let none = current.is_none() || apps.is_empty();
    if none {
        labels.insert(0, tr("settings-apps-none"));
    }
    let drop = crate::short_dropdown(&labels);
    let at = apps.iter().position(|a| a.id().is_some() && a.id() == current);
    drop.set_selected(match (none, at) {
        (false, Some(i)) => i as u32,
        _ => 0,
    });
    let message = message.clone();
    drop.connect_selected_notify(move |d| {
        let i = d.selected() as usize;
        let Some(app) = (if none { i.checked_sub(1) } else { Some(i) }).and_then(|i| apps.get(i)) else { return };
        let mut error = String::new();
        for t in &types {
            let handles = gio::AppInfo::all_for_type(t).iter().any(|a| a.id() == app.id());
            if (handles || *t == mime)
                && let Err(e) = app.set_as_default_for_type(t)
            {
                error = e.to_string();
            }
        }
        crate::say(&message, &error);
    });
    drop
}

/// The `login:` list as lines of core.yaml: each item's first and last line
/// (a long item may go on), and the item's indent.
#[derive(Debug, PartialEq)]
struct LoginBlock {
    /// The `login:` line.
    head: usize,
    items: Vec<(usize, usize)>,
    indent: String,
}

/// Where `login:` is written as a block list, one item per `- ` line;
/// `None` when it is not there, is written inline, or is not that simple.
fn login_block(text: &str) -> Option<LoginBlock> {
    let lines: Vec<&str> = text.lines().collect();
    let head = lines.iter().position(|l| l.trim_end() == "login:" || l.starts_with("login: #"))?;
    let mut items: Vec<(usize, usize)> = Vec::new();
    let mut indent: Option<String> = None;
    for (n, line) in lines.iter().enumerate().skip(head + 1) {
        let trimmed = line.trim_start();
        if !line.starts_with([' ', '\t']) && !trimmed.is_empty() && !trimmed.starts_with('#') {
            break;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let pad = &line[..line.len() - trimmed.len()];
        if trimmed.starts_with("- ") || trimmed == "-" {
            match &indent {
                Some(i) if i != pad => return None,
                Some(_) => {}
                None => indent = Some(pad.to_owned()),
            }
            items.push((n, n));
        } else if indent.as_ref().is_some_and(|i| pad.len() > i.len())
            && let Some(last) = items.last_mut()
        {
            last.1 = n;
        } else {
            return None;
        }
    }
    Some(LoginBlock { head, items, indent: indent.unwrap_or_else(|| "  ".into()) })
}

/// `text` with item `i` of `login:` removed, with the comment lines right
/// above it (they speak of it).
fn remove_login(text: &str, block: &LoginBlock, i: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let (first, last) = block.items[i];
    let mut from = first;
    while from > block.head + 1 && lines[from - 1].trim_start().starts_with('#') {
        from -= 1;
    }
    let mut out: Vec<&str> = lines[..from].to_vec();
    out.extend_from_slice(&lines[last + 1..]);
    out.join("\n") + "\n"
}

/// `text` with `command` added at the end of `login:`.
fn add_login(text: &str, block: &LoginBlock, command: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let after = block.items.last().map_or(block.head, |&(_, last)| last);
    let value = serde_yaml_ng::to_string(&Value::from(command)).unwrap_or_default();
    let item = format!("{}- {}", block.indent, value.trim_end());
    let mut out: Vec<&str> = lines[..=after].to_vec();
    out.push(&item);
    out.extend_from_slice(&lines[after + 1..]);
    out.join("\n") + "\n"
}

struct Page {
    dir: ConfigDir,
    list: gtk::Box,
    message: gtk::Label,
}

impl Page {
    fn text(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("core.yaml")).unwrap_or_default()
    }

    fn login(&self) -> Vec<String> {
        match core::get(&self.dir, "login") {
            Some(Value::Sequence(s)) => s.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
            _ => Vec::new(),
        }
    }

    fn write(&self, text: &str) -> anyhow::Result<()> {
        let path = self.dir.path().join("core.yaml");
        let temp = path.with_extension("yaml.new");
        std::fs::write(&temp, text)?;
        std::fs::rename(&temp, &path)?;
        Ok(())
    }

    /// The list changed as `edit` says: line by line when the file allows,
    /// the whole list written again otherwise.
    fn change(self: &Rc<Self>, line: impl FnOnce(&str, &LoginBlock) -> String, whole: impl FnOnce(&mut Vec<String>)) {
        let text = self.text();
        let now = self.login();
        let block = login_block(&text).filter(|b| b.items.len() == now.len() && !b.items.is_empty());
        let result = match block {
            Some(b) => {
                let new = line(&text, &b);
                // Checked: what the line editing gave reads as the list
                // should, or the whole list is written.
                let mut want = now.clone();
                whole(&mut want);
                let read: Option<Vec<String>> = serde_yaml_ng::from_str::<Value>(&new)
                    .ok()
                    .and_then(|v| v.get("login").cloned())
                    .and_then(|v| serde_yaml_ng::from_value(v).ok());
                if read.as_ref() == Some(&want) && !want.is_empty() {
                    self.write(&new)
                } else {
                    core::save(&self.dir, "login", Value::Sequence(want.into_iter().map(Value::from).collect()))
                }
            }
            None => {
                let mut want = now;
                whole(&mut want);
                core::save(&self.dir, "login", Value::Sequence(want.into_iter().map(Value::from).collect()))
            }
        };
        crate::say(&self.message, &result.err().map(|e| e.to_string()).unwrap_or_default());
        self.fill();
    }

    fn fill(self: &Rc<Self>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let items = self.login();
        if items.is_empty() {
            self.list.append(&gtk::Label::builder().label(tr("settings-panel-empty")).xalign(0.0).css_classes(["dim"]).build());
        }
        for (i, command) in items.iter().enumerate() {
            let line = gtk::Box::builder().spacing(10).build();
            line.append(
                &gtk::Label::builder()
                    .label(command)
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .tooltip_text(command)
                    .css_classes(["monospace"])
                    .build(),
            );
            let remove = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text(tr("settings-kb-remove"))
                .css_classes(["flat"])
                .build();
            let page = self.clone();
            remove.connect_clicked(move |_| {
                page.change(|text, b| remove_login(text, b, i), |l| {
                    if i < l.len() {
                        l.remove(i);
                    }
                })
            });
            line.append(&remove);
            self.list.append(&line);
        }
    }
}

/// The installed terminals: (command, name).
fn terminals() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = gio::AppInfo::all()
        .into_iter()
        .filter_map(|a| a.downcast::<gio_unix::DesktopAppInfo>().ok())
        .filter(|a| a.categories().is_some_and(|c| c.split(';').any(|c| c == "TerminalEmulator")))
        .filter_map(|a| {
            let exe = a.executable();
            let command = exe.file_name()?.to_string_lossy().into_owned();
            Some((command, a.display_name().to_string()))
        })
        .collect();
    out.sort_by_key(|t| t.1.to_lowercase());
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

pub fn page(dir: &ConfigDir) -> gtk::Widget {
    let page = Rc::new(Page {
        dir: dir.clone(),
        list: gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).build(),
        message: crate::message(),
    });
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();

    let defaults = crate::power_card(&tr("settings-apps-defaults"));
    for (label, types) in KINDS {
        defaults.append(&crate::row(&tr(label), &chooser(types.iter().map(|t| (*t).to_owned()).collect(), &page.message)));
    }

    // Any other type, by a file name or a type: `.md`, `text/markdown`.
    let other = gtk::Entry::builder().placeholder_text(tr("settings-apps-other-tip")).hexpand(true).build();
    let other_line = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).build();
    let other_choice = gtk::Box::builder().spacing(10).build();
    other_line.append(&gtk::Label::builder().label(tr("settings-apps-other")).xalign(0.0).build());
    other_line.append(&other);
    other_line.append(&other_choice);
    other.connect_activate({
        let message = page.message.clone();
        let other_choice = other_choice.clone();
        move |e| {
            while let Some(child) = other_choice.first_child() {
                other_choice.remove(&child);
            }
            let typed = e.text().trim().to_owned();
            if typed.is_empty() {
                return;
            }
            let mime = if typed.contains('/') {
                typed
            } else {
                let name = if typed.contains('.') { format!("x{typed}") } else { format!("x.{typed}") };
                gio::content_type_guess(Some(name.as_str()), None).0.to_string()
            };
            let description = gio::content_type_get_description(&mime).to_string();
            if gio::AppInfo::all_for_type(&mime).is_empty() {
                crate::say(&message, &tr("settings-apps-no-app"));
                return;
            }
            crate::say(&message, "");
            other_choice.append(
                &gtk::Label::builder()
                    .label(format!("{description} ({mime})"))
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build(),
            );
            other_choice.append(&chooser(vec![mime], &message));
        }
    });
    defaults.append(&other_line);
    body.append(&defaults);

    let session = crate::power_card(&tr("settings-apps-session"));
    let current = core::get(dir, "terminal").and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| "kitty".into());
    let mut found = terminals();
    if !found.iter().any(|(c, _)| *c == current) {
        found.insert(0, (current.clone(), current.clone()));
    }
    let labels: Vec<String> = found.iter().map(|(c, n)| format!("{n} ({c})")).collect();
    let terminal = crate::short_dropdown(&labels);
    terminal.set_selected(found.iter().position(|(c, _)| *c == current).unwrap_or(0) as u32);
    terminal.connect_selected_notify({
        let page = page.clone();
        move |d| {
            let Some((command, _)) = found.get(d.selected() as usize) else { return };
            let result = core::save(&page.dir, "terminal", Value::from(command.as_str())).and_then(|()| core::command("reload"));
            crate::say(&page.message, &result.err().map(|e| e.to_string()).unwrap_or_default());
        }
    });
    let row = crate::row(&tr("settings-apps-terminal"), &terminal);
    row.set_tooltip_text(Some(&tr("settings-apps-terminal-tip")));
    session.append(&row);
    body.append(&session);

    let login = crate::power_card(&tr("settings-apps-login"));
    login.append(&gtk::Label::builder().label(tr("settings-apps-login-tip")).xalign(0.0).wrap(true).css_classes(["dim"]).build());
    login.append(&page.list);
    let entry = gtk::Entry::builder().placeholder_text(tr("settings-apps-command")).hexpand(true).build();
    let here = gtk::CheckButton::with_label(&tr("settings-apps-here"));
    let delay = gtk::SpinButton::with_range(0.0, 600.0, 1.0);
    let add = gtk::Button::with_label(&tr("settings-net-add"));
    add.add_css_class("suggested-action");
    login.append(&entry);
    login.append(&here);
    login.append(&crate::row(&tr("settings-apps-delay"), &delay));
    add.set_halign(gtk::Align::End);
    login.append(&add);
    let go = {
        let page = page.clone();
        let entry = entry.clone();
        move || {
            let typed = entry.text().trim().to_owned();
            if typed.is_empty() {
                return;
            }
            let mut command = typed;
            let seconds = delay.value() as u32;
            if seconds > 0 {
                command = format!("sleep {seconds}; {command}");
            }
            if here.is_active() {
                command = format!("[ \"$(hostname)\" = {} ] && {{ {command}; }}", core::hostname());
            }
            let c = command.clone();
            page.change(|text, b| add_login(text, b, &c), |l| l.push(command));
            entry.set_text("");
        }
    };
    add.connect_clicked({
        let go = go.clone();
        move |_| go()
    });
    entry.connect_activate(move |_| go());
    page.fill();
    body.append(&login);
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

    const FILE: &str = "keyboard: { layout: us }\n#\n# about login\nlogin:\n  - udiskie --appindicator\n  - nm-applet --indicator\n  - '[ \"$(hostname)\" = julas ] || orbit-clip --background'\n  # blueman waits for the panel\n  - for _ in $(seq 1 50); do\n    sleep 0.1; done\n\n# Locks after\nidle_lock: 0\n";

    fn items(text: &str) -> Vec<String> {
        let v: Value = serde_yaml_ng::from_str(text).unwrap();
        serde_yaml_ng::from_value(v["login"].clone()).unwrap()
    }

    #[test]
    fn finds_the_items() {
        let b = login_block(FILE).unwrap();
        assert_eq!(b.head, 3);
        assert_eq!(b.items, vec![(4, 4), (5, 5), (6, 6), (8, 9)]);
        assert_eq!(b.indent, "  ");
        assert_eq!(items(FILE).len(), 4);
    }

    #[test]
    fn removes_with_its_comment() {
        let b = login_block(FILE).unwrap();
        let out = remove_login(FILE, &b, 3);
        assert!(!out.contains("blueman"));
        assert!(out.contains("# Locks after\nidle_lock: 0"));
        assert_eq!(items(&out).len(), 3);
        let out = remove_login(FILE, &b, 0);
        assert!(out.contains("# about login\nlogin:\n  - nm-applet"));
    }

    #[test]
    fn adds_at_the_end() {
        let b = login_block(FILE).unwrap();
        let out = add_login(FILE, &b, "[ \"$(hostname)\" = hp ] && { sleep 5; solaar; }");
        let got = items(&out);
        assert_eq!(got.len(), 5);
        assert_eq!(got[4], "[ \"$(hostname)\" = hp ] && { sleep 5; solaar; }");
        assert!(out.contains("# Locks after"));
    }

    #[test]
    fn leaves_inline_lists_alone() {
        assert_eq!(login_block("login: [nm-applet]\n"), None);
        assert_eq!(login_block("startup: []\n"), None);
    }
}
