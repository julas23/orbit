//! The Keyboard page (docs/PLANO-SETTINGS.md §3.1): the layouts, in order
//! (added from XKB's list, searched; moved, removed), how to switch between
//! them, the Compose key, and how a held key repeats. Written to core.yaml's
//! `keyboard:` (layout and variant as XKB's comma lists), the core
//! reloaded at once. XKB options set by hand other than switching and
//! Compose are kept.

use std::{cell::RefCell, rc::Rc};

use gtk::{glib, prelude::*};
use orbit_app::i18n::tr;
use orbit_common::ConfigDir;
use serde_yaml_ng::{Mapping, Value};

use crate::core;

const XKB_LIST: &str = "/usr/share/X11/xkb/rules/evdev.lst";

/// The ways to switch layouts, as XKB options (empty: only the panel's
/// keyboard module). Win+Space is the launcher's.
const SWITCHES: [(&str, &str); 5] = [
    ("", "settings-kb-switch-panel"),
    ("grp:alt_shift_toggle", "settings-kb-switch-alt-shift"),
    ("grp:ctrl_shift_toggle", "settings-kb-switch-ctrl-shift"),
    ("grp:caps_toggle", "settings-kb-switch-caps"),
    ("grp:alt_caps_toggle", "settings-kb-switch-alt-caps"),
];
const COMPOSE: [(&str, &str); 5] = [
    ("", "settings-kb-compose-none"),
    ("compose:ralt", "settings-kb-compose-ralt"),
    ("compose:rwin", "settings-kb-compose-rwin"),
    ("compose:menu", "settings-kb-compose-menu"),
    ("compose:caps", "settings-kb-compose-caps"),
];

/// The keyboard as core.yaml has it (or, without it, as the system has it).
#[derive(Debug, Clone, PartialEq)]
struct Keyboard {
    /// (layout, variant) pairs, in order.
    layouts: Vec<(String, String)>,
    model: String,
    options: Vec<String>,
    repeat_delay: i64,
    repeat_rate: i64,
}

/// XKB's comma lists zipped: `us,br` + `alt-intl,` → (us, alt-intl), (br, "").
fn zip(layouts: &str, variants: &str) -> Vec<(String, String)> {
    let variants: Vec<&str> = variants.split(',').collect();
    layouts
        .split(',')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .enumerate()
        .map(|(i, l)| (l.to_owned(), variants.get(i).map(|v| v.trim().to_owned()).unwrap_or_default()))
        .collect()
}

/// /etc/default/keyboard's `KEY="value"`.
fn system_value(text: &str, key: &str) -> String {
    text.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
        .map(|v| v.trim().trim_matches('"').to_owned())
        .unwrap_or_default()
}

fn load(dir: &ConfigDir) -> Keyboard {
    let system = std::fs::read_to_string("/etc/default/keyboard").unwrap_or_default();
    let map = core::get(dir, "keyboard").and_then(|v| v.as_mapping().cloned()).unwrap_or_default();
    let field = |key: &str, system_key: &str| {
        map.get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| if map.is_empty() { system_value(&system, system_key) } else { String::new() })
    };
    let number = |key: &str, default: i64| map.get(key).and_then(Value::as_i64).unwrap_or(default);
    let mut layouts = zip(&field("layout", "XKBLAYOUT"), &field("variant", "XKBVARIANT"));
    if layouts.is_empty() {
        layouts.push(("us".into(), String::new()));
    }
    Keyboard {
        layouts,
        model: field("model", "XKBMODEL"),
        options: field("options", "XKBOPTIONS")
            .split(',')
            .map(str::trim)
            .filter(|o| !o.is_empty())
            .map(str::to_owned)
            .collect(),
        repeat_delay: number("repeat_delay", 200),
        repeat_rate: number("repeat_rate", 25),
    }
}

fn save(dir: &ConfigDir, kb: &Keyboard) -> anyhow::Result<()> {
    let mut map = Mapping::new();
    let layouts: Vec<&str> = kb.layouts.iter().map(|(l, _)| l.as_str()).collect();
    let variants: Vec<&str> = kb.layouts.iter().map(|(_, v)| v.as_str()).collect();
    map.insert("layout".into(), layouts.join(",").into());
    map.insert("variant".into(), variants.join(",").into());
    if !kb.model.is_empty() {
        map.insert("model".into(), kb.model.clone().into());
    }
    map.insert("options".into(), kb.options.join(",").into());
    map.insert("repeat_delay".into(), kb.repeat_delay.into());
    map.insert("repeat_rate".into(), kb.repeat_rate.into());
    core::save(dir, "keyboard", Value::Mapping(map))?;
    core::command("reload")
}

/// Every layout and variant XKB lists: (layout, variant, description).
fn xkb_layouts() -> Vec<(String, String, String)> {
    let text = std::fs::read_to_string(XKB_LIST).unwrap_or_default();
    let mut section = "";
    let mut found = Vec::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("! ") {
            section = match name.trim() {
                "layout" => "layout",
                "variant" => "variant",
                _ => "",
            };
            continue;
        }
        let line = line.trim();
        let Some((code, rest)) = line.split_once(char::is_whitespace) else { continue };
        let rest = rest.trim();
        match section {
            "layout" => found.push((code.to_owned(), String::new(), rest.to_owned())),
            "variant" => {
                if let Some((layout, description)) = rest.split_once(": ") {
                    found.push((layout.to_owned(), code.to_owned(), description.to_owned()));
                }
            }
            _ => {}
        }
    }
    found
}

fn describe(all: &[(String, String, String)], layout: &str, variant: &str) -> String {
    all.iter()
        .find(|(l, v, _)| l == layout && v == variant)
        .map(|(_, _, d)| d.clone())
        .unwrap_or_else(|| if variant.is_empty() { layout.to_owned() } else { format!("{layout} ({variant})") })
}

struct Page {
    dir: ConfigDir,
    kb: RefCell<Keyboard>,
    all: Vec<(String, String, String)>,
    list: gtk::Box,
    message: gtk::Label,
}

pub fn page(dir: &ConfigDir) -> gtk::Widget {
    let page = Rc::new(Page {
        dir: dir.clone(),
        kb: RefCell::new(load(dir)),
        all: xkb_layouts(),
        list: gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).build(),
        message: crate::message(),
    });
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();

    let layouts = crate::power_card(&tr("settings-kb-layouts"));
    layouts.append(&page.list);
    let add = gtk::Button::with_label(&tr("settings-kb-add"));
    add.set_halign(gtk::Align::End);
    layouts.append(&add);
    add.connect_clicked({
        let page = page.clone();
        move |b| page.add_dialog(b)
    });
    page.fill();

    let typing = crate::power_card(&tr("settings-kb-typing"));
    typing.append(&crate::row(&tr("settings-kb-switch"), &page.option_dropdown(&SWITCHES, "grp:")));
    typing.append(&crate::row(&tr("settings-kb-compose"), &page.option_dropdown(&COMPOSE, "compose:")));
    let (delay, rate) = {
        let kb = page.kb.borrow();
        (kb.repeat_delay, kb.repeat_rate)
    };
    typing.append(&crate::row(&tr("settings-kb-delay"), &page.scale(150.0, 1000.0, delay as f64, true)));
    typing.append(&crate::row(&tr("settings-kb-rate"), &page.scale(10.0, 60.0, rate as f64, false)));
    let test = gtk::Entry::builder().placeholder_text(tr("settings-kb-test")).build();
    typing.append(&test);

    body.append(&layouts);
    body.append(&typing);
    body.append(&page.message);
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build()
        .upcast()
}

impl Page {
    fn write(&self) {
        let result = save(&self.dir, &self.kb.borrow());
        crate::say(&self.message, &result.err().map(|e| e.to_string()).unwrap_or_default());
    }

    /// The layouts' rows again.
    fn fill(self: &Rc<Self>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let layouts = self.kb.borrow().layouts.clone();
        let count = layouts.len();
        for (i, (layout, variant)) in layouts.iter().enumerate() {
            let line = gtk::Box::builder().spacing(5).build();
            line.append(
                &gtk::Label::builder()
                    .label(describe(&self.all, layout, variant))
                    .xalign(0.0)
                    .hexpand(true)
                    .build(),
            );
            for (icon, tooltip, enabled, step) in [
                ("go-up-symbolic", "settings-kb-up", i > 0, -1i32),
                ("go-down-symbolic", "settings-kb-down", i + 1 < count, 1),
            ] {
                let button = gtk::Button::builder()
                    .icon_name(icon)
                    .tooltip_text(tr(tooltip))
                    .sensitive(enabled)
                    .css_classes(["flat"])
                    .build();
                let page = self.clone();
                button.connect_clicked(move |_| {
                    let j = (i as i32 + step) as usize;
                    page.kb.borrow_mut().layouts.swap(i, j);
                    page.write();
                    page.fill();
                });
                line.append(&button);
            }
            let remove = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text(tr("settings-kb-remove"))
                .sensitive(count > 1)
                .css_classes(["flat"])
                .build();
            let page = self.clone();
            remove.connect_clicked(move |_| {
                page.kb.borrow_mut().layouts.remove(i);
                page.write();
                page.fill();
            });
            line.append(&remove);
            self.list.append(&line);
        }
    }

    /// A dropdown of one family of XKB options (`prefix`): the one set
    /// replaces any other of the family, the rest of the options kept.
    fn option_dropdown(self: &Rc<Self>, choices: &'static [(&'static str, &'static str)], prefix: &'static str) -> gtk::DropDown {
        let labels: Vec<String> = choices.iter().map(|(_, l)| tr(l)).collect();
        let dropdown = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
        let current = self.kb.borrow().options.iter().find(|o| o.starts_with(prefix)).cloned().unwrap_or_default();
        dropdown.set_selected(choices.iter().position(|(o, _)| *o == current).unwrap_or(0) as u32);
        let page = self.clone();
        dropdown.connect_selected_notify(move |d| {
            let Some((option, _)) = choices.get(d.selected() as usize) else { return };
            {
                let mut kb = page.kb.borrow_mut();
                kb.options.retain(|o| !o.starts_with(prefix));
                if !option.is_empty() {
                    kb.options.push((*option).to_owned());
                }
            }
            page.write();
        });
        dropdown
    }

    /// The repetition's delay (ms) or rate (per second), written when let
    /// go of (not at every step of a drag).
    fn scale(self: &Rc<Self>, min: f64, max: f64, value: f64, delay: bool) -> gtk::Scale {
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, 1.0);
        scale.set_value(value);
        scale.set_hexpand(true);
        scale.set_draw_value(true);
        scale.set_value_pos(gtk::PositionType::Right);
        let pending: Rc<std::cell::Cell<Option<glib::SourceId>>> = Rc::default();
        let page = self.clone();
        scale.connect_value_changed(move |s| {
            let v = s.value().round() as i64;
            {
                let mut kb = page.kb.borrow_mut();
                if delay {
                    kb.repeat_delay = v;
                } else {
                    kb.repeat_rate = v;
                }
            }
            if let Some(id) = pending.take() {
                id.remove();
            }
            let page = page.clone();
            let again = pending.clone();
            pending.set(Some(glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                again.set(None);
                page.write();
            })));
        });
        scale
    }

    /// A dialog with XKB's layouts and variants, searched by typing.
    fn add_dialog(self: &Rc<Self>, button: &gtk::Button) {
        let window = gtk::Window::builder()
            .title(tr("settings-kb-add"))
            .modal(true)
            .default_width(480)
            .css_classes(["orbit-settings"])
            .build();
        if let Some(parent) = button.root().and_downcast::<gtk::Window>() {
            window.set_transient_for(Some(&parent));
        }
        let labels: Vec<String> = self.all.iter().map(|(_, _, d)| d.clone()).collect();
        let model = gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>());
        let dropdown = gtk::DropDown::builder()
            .model(&model)
            .enable_search(true)
            .expression(gtk::PropertyExpression::new(gtk::StringObject::static_type(), None::<gtk::Expression>, "string"))
            .hexpand(true)
            .build();
        dropdown.set_search_match_mode(gtk::StringFilterMatchMode::Substring);
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["page"])
            .build();
        body.append(&dropdown);
        let cancel = gtk::Button::with_label(&tr("settings-svc-cancel"));
        let add = gtk::Button::with_label(&tr("settings-net-add"));
        add.add_css_class("suggested-action");
        let buttons = gtk::Box::builder().spacing(10).halign(gtk::Align::End).build();
        buttons.append(&cancel);
        buttons.append(&add);
        body.append(&buttons);
        window.set_child(Some(&body));
        cancel.connect_clicked({
            let window = window.clone();
            move |_| window.close()
        });
        let page = self.clone();
        add.connect_clicked({
            let window = window.clone();
            move |_| {
                if let Some((layout, variant, _)) = page.all.get(dropdown.selected() as usize).cloned() {
                    let added = {
                        let mut kb = page.kb.borrow_mut();
                        let pair = (layout, variant);
                        let new = !kb.layouts.contains(&pair);
                        if new {
                            kb.layouts.push(pair);
                        }
                        new
                    };
                    if added {
                        page.write();
                        page.fill();
                    }
                }
                window.close();
            }
        });
        window.present();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xkb_lists_zipped() {
        assert_eq!(
            zip("us,br", "alt-intl,"),
            vec![("us".into(), "alt-intl".into()), ("br".into(), String::new())]
        );
        assert_eq!(zip("de", ""), vec![("de".into(), String::new())]);
        assert!(zip("", "").is_empty());
    }

    #[test]
    fn system_keyboard() {
        let text = "XKBMODEL=\"pc105\"\nXKBLAYOUT=\"us,br\"\nXKBVARIANT=\",\"\n";
        assert_eq!(system_value(text, "XKBLAYOUT"), "us,br");
        assert_eq!(system_value(text, "XKBOPTIONS"), "");
    }

    #[test]
    fn the_xkb_list_reads() {
        let all = xkb_layouts();
        if all.is_empty() {
            return; // no xkb-data on this machine
        }
        assert!(all.iter().any(|(l, v, _)| l == "br" && v.is_empty()));
        assert!(all.iter().any(|(l, v, _)| l == "us" && v == "alt-intl"));
    }
}
