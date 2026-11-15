//! The Panel page (docs/PLANO-SETTINGS.md §3.3): panel.yaml's edge,
//! monitor, reserved space and layout grid, and its modules in the three
//! parts (moved up, down, to another part, removed, added from the list
//! the panel's own editor offers). The panel edits itself in place too (a
//! right click on an empty spot), which is where modules are dragged and
//! sized; this page reaches the same file another way, and from the
//! search. The panel reads its file at start: after a change it is started
//! again (`orbit-session`, which starts the dock again too).

use std::{cell::RefCell, rc::Rc};

use gtk::prelude::*;
use orbit_app::i18n::tr;
use orbit_common::{
    ConfigDir,
    config::{Edge, MonitorSpec, ModuleEntry, PANEL_MODULES, PANEL_REPEATABLE, PanelConfig},
};

use crate::core;

const EDGES: [(Edge, &str, &str); 4] = [
    (Edge::Left, "left", "settings-edge-left"),
    (Edge::Right, "right", "settings-edge-right"),
    (Edge::Top, "top", "settings-edge-top"),
    (Edge::Bottom, "bottom", "settings-edge-bottom"),
];
const PARTS: [(&str, &str); 3] = [
    ("start", "settings-panel-start"),
    ("center", "settings-panel-center"),
    ("end", "settings-panel-end"),
];

/// Starts the panel and the dock again, to read their files.
pub fn restart() {
    if let Err(e) = std::process::Command::new("orbit-session").spawn() {
        log::warn!("orbit-session: {e}");
    }
}

struct Page {
    dir: ConfigDir,
    /// The three parts' modules, as the page edits them.
    parts: RefCell<[Vec<ModuleEntry>; 3]>,
    lists: [gtk::Box; 3],
    message: gtk::Label,
}

impl Page {
    fn save_keys(&self, key: &str, line: String) {
        let result = self.dir.save_app_keys("panel", &[key], &line).map(|()| restart());
        crate::say(&self.message, &result.err().map(|e| e.to_string()).unwrap_or_default());
    }

    fn save_modules(&self) {
        let parts = self.parts.borrow();
        let result = self.dir.save_panel_modules(&parts[0], &parts[1], &parts[2], false).map(|()| restart());
        crate::say(&self.message, &result.err().map(|e| e.to_string()).unwrap_or_default());
    }

    fn fill(self: &Rc<Self>) {
        let parts = self.parts.borrow().clone();
        for (p, list) in self.lists.iter().enumerate() {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            if parts[p].is_empty() {
                list.append(&gtk::Label::builder().label(tr("settings-panel-empty")).xalign(0.0).css_classes(["dim"]).build());
            }
            let count = parts[p].len();
            for (i, entry) in parts[p].iter().enumerate() {
                let line = gtk::Box::builder().spacing(5).build();
                let mut name = tr(&format!("panel-module-{}", entry.name));
                if let Some(path) = &entry.path {
                    name.push_str(&format!(" ({path})"));
                }
                line.append(&gtk::Label::builder().label(name).xalign(0.0).hexpand(true).build());
                for (icon, enabled, step) in [("go-up-symbolic", i > 0, -1i32), ("go-down-symbolic", i + 1 < count, 1)] {
                    let b = gtk::Button::builder().icon_name(icon).sensitive(enabled).css_classes(["flat"]).build();
                    let page = self.clone();
                    b.connect_clicked(move |_| {
                        page.parts.borrow_mut()[p].swap(i, (i as i32 + step) as usize);
                        page.save_modules();
                        page.fill();
                    });
                    line.append(&b);
                }
                let labels: Vec<String> = PARTS.iter().map(|(id, _)| tr(&format!("settings-panel-{id}-short"))).collect();
                let part = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
                part.set_selected(p as u32);
                part.set_tooltip_text(Some(&tr("settings-panel-part")));
                let page = self.clone();
                part.connect_selected_notify(move |d| {
                    let to = d.selected() as usize;
                    if to == p || to >= 3 {
                        return;
                    }
                    let moved = page.parts.borrow_mut()[p].remove(i);
                    page.parts.borrow_mut()[to].push(moved);
                    page.save_modules();
                    page.fill();
                });
                line.append(&part);
                let remove = gtk::Button::builder()
                    .icon_name("user-trash-symbolic")
                    .tooltip_text(tr("settings-kb-remove"))
                    .css_classes(["flat"])
                    .build();
                let page = self.clone();
                remove.connect_clicked(move |_| {
                    page.parts.borrow_mut()[p].remove(i);
                    page.save_modules();
                    page.fill();
                });
                line.append(&remove);
                list.append(&line);
            }
        }
    }
}

pub fn page(dir: &ConfigDir) -> gtk::Widget {
    let config: PanelConfig = dir.panel().unwrap_or_else(|e| {
        log::warn!("panel.yaml: {e}");
        PanelConfig::default()
    });
    let page = Rc::new(Page {
        dir: dir.clone(),
        parts: RefCell::new([config.start.clone(), config.center.clone(), config.end.clone()]),
        lists: std::array::from_fn(|_| gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).build()),
        message: crate::message(),
    });
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();

    let general = crate::power_card(&tr("settings-panel-general"));
    let labels: Vec<String> = EDGES.iter().map(|(_, _, l)| tr(l)).collect();
    let edge = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    edge.set_selected(EDGES.iter().position(|(e, _, _)| *e == config.edge).unwrap_or(0) as u32);
    edge.connect_selected_notify({
        let page = page.clone();
        move |d| {
            if let Some((_, name, _)) = EDGES.get(d.selected() as usize) {
                page.save_keys("edge", format!("edge: {name}\n"));
            }
        }
    });
    general.append(&crate::row(&tr("settings-panel-edge"), &edge));

    let names: Vec<String> = core::outputs().map(|o| o.into_iter().map(|o| o.name).collect()).unwrap_or_default();
    let mut labels = vec![tr("settings-panel-first-monitor")];
    labels.extend(names.iter().cloned());
    let monitor = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let selected = match &config.monitor {
        MonitorSpec::Name(n) => names.iter().position(|x| x == n).map_or(0, |i| i + 1),
        MonitorSpec::Index(i) => {
            if *i == 0 {
                0
            } else {
                (*i).min(names.len().saturating_sub(1)) + 1
            }
        }
    };
    monitor.set_selected(selected as u32);
    monitor.connect_selected_notify({
        let page = page.clone();
        move |d| {
            let line = match d.selected() as usize {
                0 => "monitor: 0\n".to_owned(),
                i => names.get(i - 1).map(|n| format!("monitor: {n}\n")).unwrap_or_default(),
            };
            if !line.is_empty() {
                page.save_keys("monitor", line);
            }
        }
    });
    general.append(&crate::row(&tr("settings-panel-monitor"), &monitor));

    for (key, label, value) in [
        ("reserve", "settings-panel-reserve", config.reserve),
        ("maintenance", "settings-panel-grid", config.maintenance),
    ] {
        let switch = gtk::Switch::builder().active(value).halign(gtk::Align::End).build();
        let page = page.clone();
        switch.connect_active_notify(move |s| page.save_keys(key, format!("{key}: {}\n", s.is_active())));
        general.append(&crate::row(&tr(label), &switch));
    }
    body.append(&general);

    let modules = crate::power_card(&tr("settings-panel-modules"));
    modules.append(&gtk::Label::builder().label(tr("settings-panel-modules-tip")).xalign(0.0).wrap(true).css_classes(["dim"]).build());
    for (p, (_, label)) in PARTS.iter().enumerate() {
        modules.append(&gtk::Label::builder().label(tr(label)).xalign(0.0).css_classes(["name"]).build());
        modules.append(&page.lists[p]);
    }
    let labels: Vec<String> = PANEL_MODULES.iter().map(|m| tr(&format!("panel-module-{m}"))).collect();
    let add_pick = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let add = gtk::Button::with_label(&tr("settings-panel-add"));
    add.connect_clicked({
        let page = page.clone();
        let add_pick = add_pick.clone();
        move |_| {
            let Some(name) = PANEL_MODULES.get(add_pick.selected() as usize) else { return };
            let present = page.parts.borrow().iter().flatten().any(|e| e.name == *name);
            if present && !PANEL_REPEATABLE.contains(name) {
                crate::say(&page.message, &tr("settings-panel-already"));
                return;
            }
            page.parts.borrow_mut()[2].push(ModuleEntry { name: (*name).to_owned(), size: None, path: None });
            page.save_modules();
            page.fill();
        }
    });
    let add_line = gtk::Box::builder().spacing(10).halign(gtk::Align::End).build();
    add_line.append(&add_pick);
    add_line.append(&add);
    modules.append(&add_line);
    page.fill();
    body.append(&modules);
    body.append(&page.message);
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .build()
        .upcast()
}
