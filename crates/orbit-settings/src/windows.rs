//! The Windows and areas page (docs/PLANO-SETTINGS.md §3.3), core.yaml's
//! keys for how windows are laid out and look: the layout a new area takes
//! (and the master layout's share and count), the gaps, border and corners,
//! the border's colours, shadows, animations, focus following the mouse,
//! the areas' grid, and the window rules (what a program's windows do when
//! they open). Each change is written at once and the core reloaded.

use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, prelude::*};
use orbit_app::i18n::{tr, tr_args};
use orbit_common::ConfigDir;
use serde_yaml_ng::{Mapping, Value};

use crate::core;

const LAYOUTS: [(&str, &str); 4] = [
    ("columns", "settings-win-columns"),
    ("master", "settings-win-master"),
    ("monocle", "settings-win-monocle"),
    ("bsp", "settings-win-bsp"),
];
const SHADOWS: [(&str, &str); 3] = [
    ("floating", "settings-win-shadows-floating"),
    ("all", "settings-win-shadows-all"),
    ("none", "settings-win-shadows-none"),
];

/// `#rrggbb` from GTK's colour.
fn hex(rgba: &gdk::RGBA) -> String {
    let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", c(rgba.red()), c(rgba.green()), c(rgba.blue()))
}

/// A regular expression matching `text` exactly (an app_id as written).
fn exact(text: &str) -> String {
    let mut out = String::from("^");
    for ch in text.chars() {
        if "\\.+*?()|[]{}^$".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('$');
    out
}

/// The app_ids of the windows open now (orbit-core's tree).
fn open_apps() -> Vec<String> {
    let Ok(body) = orbit_ipc::Connection::open().and_then(|mut c| c.request(orbit_ipc::GET_TREE, "")) else {
        return Vec::new();
    };
    let Ok(tree) = serde_json::from_slice::<serde_json::Value>(&body) else { return Vec::new() };
    let mut found = Vec::new();
    fn walk(node: &serde_json::Value, found: &mut Vec<String>) {
        let id = node["app_id"].as_str().or_else(|| node["window_properties"]["class"].as_str());
        if let Some(id) = id.filter(|i| !i.is_empty())
            && !found.iter().any(|f| f == id)
        {
            found.push(id.to_owned());
        }
        for key in ["nodes", "floating_nodes"] {
            for child in node[key].as_array().into_iter().flatten() {
                walk(child, found);
            }
        }
    }
    walk(&tree, &mut found);
    found.sort();
    found
}

/// A rule, said in a line: "firefox: floating, area 3".
fn describe(rule: &Mapping) -> String {
    let who = rule
        .get("app_id")
        .or_else(|| rule.get("title"))
        .and_then(Value::as_str)
        .map(|s| s.trim_start_matches('^').trim_end_matches('$').replace('\\', ""))
        .unwrap_or_else(|| "*".into());
    let mut what = Vec::new();
    let flag = |key: &str| rule.get(key).and_then(Value::as_bool) == Some(true);
    if flag("floating") {
        what.push(tr("settings-win-rule-floating"));
    }
    if let Some(n) = rule.get("workspace").and_then(Value::as_i64) {
        what.push(tr_args("settings-win-rule-area", &[("area", n.into())]));
    }
    if flag("fullscreen") {
        what.push(tr("settings-win-rule-fullscreen"));
    }
    if flag("maximized") {
        what.push(tr("settings-win-rule-maximized"));
    }
    if rule.get("focus").and_then(Value::as_bool) == Some(false) {
        what.push(tr("settings-win-rule-nofocus"));
    }
    format!("{who}: {}", what.join(", "))
}

struct Page {
    dir: ConfigDir,
    message: gtk::Label,
    rules: gtk::Box,
    master: RefCell<Vec<gtk::Box>>,
}

impl Page {
    fn set(&self, key: &str, value: Value) {
        let result = core::save(&self.dir, key, value).and_then(|()| core::command("reload"));
        crate::say(&self.message, &result.err().map(|e| e.to_string()).unwrap_or_default());
    }

    fn number(&self, key: &str, default: f64) -> f64 {
        core::get(&self.dir, key).and_then(|v| v.as_f64()).unwrap_or(default)
    }

    fn spin(self: &Rc<Self>, key: &'static str, label: &str, range: (f64, f64), default: f64) -> gtk::Box {
        let spin = gtk::SpinButton::with_range(range.0, range.1, 1.0);
        spin.set_value(self.number(key, default));
        let page = self.clone();
        spin.connect_value_changed(move |s| page.set(key, (s.value() as i64).into()));
        crate::row(&tr(label), &spin)
    }

    fn switch(self: &Rc<Self>, key: &'static str, label: &str, default: bool) -> gtk::Box {
        let on = core::get(&self.dir, key).and_then(|v| v.as_bool()).unwrap_or(default);
        let switch = gtk::Switch::builder().active(on).halign(gtk::Align::End).build();
        let page = self.clone();
        switch.connect_active_notify(move |s| page.set(key, s.is_active().into()));
        crate::row(&tr(label), &switch)
    }

    fn choice(self: &Rc<Self>, key: &'static str, label: &str, choices: &'static [(&'static str, &'static str)], default: &str) -> gtk::Box {
        let labels: Vec<String> = choices.iter().map(|(_, l)| tr(l)).collect();
        let dropdown = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
        let now = core::get(&self.dir, key).and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| default.into());
        dropdown.set_selected(choices.iter().position(|(v, _)| *v == now).unwrap_or(0) as u32);
        let page = self.clone();
        dropdown.connect_selected_notify(move |d| {
            if let Some((value, _)) = choices.get(d.selected() as usize) {
                page.set(key, (*value).into());
                if key == "layout" {
                    for row in page.master.borrow().iter() {
                        row.set_visible(*value == "master");
                    }
                }
            }
        });
        crate::row(&tr(label), &dropdown)
    }

    fn color(self: &Rc<Self>, key: &'static str, label: &str, default: &str) -> gtk::Box {
        let now = core::get(&self.dir, key).and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| default.into());
        let button = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::builder().with_alpha(false).build()));
        if let Ok(rgba) = gdk::RGBA::parse(&now) {
            button.set_rgba(&rgba);
        }
        let page = self.clone();
        button.connect_rgba_notify(move |b| page.set(key, hex(&b.rgba()).into()));
        crate::row(&tr(label), &button)
    }

    fn rules_list(&self) -> Vec<Value> {
        core::get(&self.dir, "rules").and_then(|v| v.as_sequence().cloned()).unwrap_or_default()
    }

    fn fill_rules(self: &Rc<Self>) {
        while let Some(child) = self.rules.first_child() {
            self.rules.remove(&child);
        }
        let rules = self.rules_list();
        if rules.is_empty() {
            self.rules.append(&gtk::Label::builder().label(tr("settings-win-no-rules")).xalign(0.0).css_classes(["dim"]).build());
        }
        for (i, rule) in rules.iter().enumerate() {
            let line = gtk::Box::builder().spacing(10).build();
            let text = rule.as_mapping().map(describe).unwrap_or_default();
            line.append(&gtk::Label::builder().label(text).xalign(0.0).hexpand(true).wrap(true).build());
            let remove = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text(tr("settings-kb-remove"))
                .css_classes(["flat"])
                .build();
            let page = self.clone();
            remove.connect_clicked(move |_| {
                let mut rules = page.rules_list();
                if i < rules.len() {
                    rules.remove(i);
                }
                page.set("rules", Value::Sequence(rules));
                page.fill_rules();
            });
            line.append(&remove);
            self.rules.append(&line);
        }
    }

    /// A dialog for a new rule: which program (one open now, or typed) and
    /// what its windows do.
    fn add_rule(self: &Rc<Self>, button: &gtk::Button) {
        let window = gtk::Window::builder()
            .title(tr("settings-win-add-rule"))
            .modal(true)
            .default_width(460)
            .css_classes(["orbit-settings"])
            .build();
        if let Some(parent) = button.root().and_downcast::<gtk::Window>() {
            window.set_transient_for(Some(&parent));
        }
        // The program: typed, or picked among the ones open now.
        let apps = open_apps();
        let app = gtk::Entry::builder().hexpand(true).placeholder_text(tr("settings-win-program-tip")).build();
        let open = gtk::DropDown::from_strings(&apps.iter().map(String::as_str).collect::<Vec<_>>());
        open.set_selected(gtk::INVALID_LIST_POSITION);
        open.set_visible(!apps.is_empty());
        open.connect_selected_notify({
            let app = app.clone();
            move |d| {
                if let Some(id) = apps.get(d.selected() as usize) {
                    app.set_text(id);
                }
            }
        });
        let floating = gtk::CheckButton::with_label(&tr("settings-win-rule-floating"));
        let fullscreen = gtk::CheckButton::with_label(&tr("settings-win-rule-fullscreen"));
        let maximized = gtk::CheckButton::with_label(&tr("settings-win-rule-maximized"));
        let nofocus = gtk::CheckButton::with_label(&tr("settings-win-rule-nofocus"));
        let area = gtk::SpinButton::with_range(0.0, 99.0, 1.0);
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["page"])
            .build();
        body.append(&crate::row(&tr("settings-win-program"), &app));
        body.append(&crate::row(&tr("settings-win-open-now"), &open));
        body.append(&floating);
        body.append(&fullscreen);
        body.append(&maximized);
        body.append(&nofocus);
        body.append(&crate::row(&tr("settings-win-area-of"), &area));
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
                let id = app.text().trim().to_owned();
                if id.is_empty() {
                    return;
                }
                let mut rule = Mapping::new();
                rule.insert("app_id".into(), exact(&id).into());
                for (check, key, value) in [
                    (&floating, "floating", true),
                    (&fullscreen, "fullscreen", true),
                    (&maximized, "maximized", true),
                    (&nofocus, "focus", false),
                ] {
                    if check.is_active() {
                        rule.insert(key.into(), value.into());
                    }
                }
                if area.value() > 0.0 {
                    rule.insert("workspace".into(), (area.value() as i64).into());
                }
                let mut rules = page.rules_list();
                rules.push(Value::Mapping(rule));
                page.set("rules", Value::Sequence(rules));
                page.fill_rules();
                window.close();
            }
        });
        window.present();
    }
}

pub fn page(dir: &ConfigDir) -> gtk::Widget {
    let page = Rc::new(Page {
        dir: dir.clone(),
        message: crate::message(),
        rules: gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).build(),
        master: RefCell::default(),
    });
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();

    let layout = crate::power_card(&tr("settings-win-layout-card"));
    layout.append(&page.choice("layout", "settings-win-layout", &LAYOUTS, "columns"));
    let ratio = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.2, 0.8, 0.05);
    ratio.set_value(page.number("master_ratio", 0.55));
    ratio.set_hexpand(true);
    ratio.connect_value_changed({
        let page = page.clone();
        move |s| page.set("master_ratio", ((s.value() * 100.0).round() / 100.0).into())
    });
    let ratio_row = crate::row(&tr("settings-win-master-ratio"), &ratio);
    let count_row = page.spin("master_count", "settings-win-master-count", (1.0, 6.0), 1.0);
    let is_master = core::get(dir, "layout").and_then(|v| v.as_str().map(str::to_owned)).as_deref() == Some("master");
    for row in [&ratio_row, &count_row] {
        row.set_visible(is_master);
        layout.append(row);
    }
    *page.master.borrow_mut() = vec![ratio_row, count_row];
    layout.append(&page.switch("focus_follows_mouse", "settings-win-focus-follows", true));
    body.append(&layout);

    let look = crate::power_card(&tr("settings-win-look"));
    look.append(&page.spin("gaps", "settings-win-gaps", (0.0, 40.0), 6.0));
    look.append(&page.spin("border", "settings-win-border", (0.0, 10.0), 2.0));
    look.append(&page.spin("corner_radius", "settings-win-corners", (0.0, 24.0), 10.0));
    look.append(&page.color("focused_color", "settings-win-focused", "#4682b4"));
    look.append(&page.color("unfocused_color", "settings-win-unfocused", "#4c566a"));
    look.append(&page.choice("shadows", "settings-win-shadows", &SHADOWS, "floating"));
    look.append(&page.switch("animations", "settings-win-animations", true));
    body.append(&look);

    let areas = crate::power_card(&tr("settings-win-areas"));
    areas.append(&page.spin("areas", "settings-win-areas-count", (1.0, 16.0), 4.0));
    areas.append(&page.spin("area_columns", "settings-win-areas-columns", (1.0, 8.0), 2.0));
    body.append(&areas);

    let rules = crate::power_card(&tr("settings-win-rules"));
    rules.append(&gtk::Label::builder().label(tr("settings-win-rules-tip")).xalign(0.0).wrap(true).css_classes(["dim"]).build());
    rules.append(&page.rules);
    let add = gtk::Button::with_label(&tr("settings-win-add-rule"));
    add.set_halign(gtk::Align::End);
    add.connect_clicked({
        let page = page.clone();
        move |b| page.add_rule(b)
    });
    rules.append(&add);
    page.fill_rules();
    body.append(&rules);
    body.append(&page.message);
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build()
        .upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_patterns() {
        assert_eq!(exact("firefox"), "^firefox$");
        assert_eq!(exact("org.gnome.Nautilus"), "^org\\.gnome\\.Nautilus$");
    }

    #[test]
    fn colours() {
        let rgba = gdk::RGBA::parse("#4682b4").unwrap();
        assert_eq!(hex(&rgba), "#4682b4");
    }
}
