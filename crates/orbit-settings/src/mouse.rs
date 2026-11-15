//! The Mouse and touchpad page (docs/PLANO-SETTINGS.md §3.1): core.yaml's
//! `mouse:` and `touchpad:` (libinput's settings, applied by the core to
//! each device as it is plugged and at `reload`), and `gestures`. The
//! touchpad's part shows when the machine has one (udev's
//! ID_INPUT_TOUCHPAD). Each change is written at once and the core
//! reloaded.

use std::rc::Rc;

use gtk::{glib, prelude::*};
use orbit_app::i18n::tr;
use orbit_common::ConfigDir;
use serde_yaml_ng::{Mapping, Value};

use crate::core;

/// Whether udev knows a touchpad on this machine.
fn has_touchpad() -> bool {
    std::fs::read_dir("/run/udev/data").is_ok_and(|entries| {
        entries.filter_map(|e| e.ok()).any(|e| {
            e.file_name().to_string_lossy().starts_with("c13:")
                && std::fs::read_to_string(e.path()).is_ok_and(|t| t.lines().any(|l| l == "E:ID_INPUT_TOUCHPAD=1"))
        })
    })
}

/// A setting of a device's block (`mouse` or `touchpad`).
struct Field {
    section: &'static str,
    key: &'static str,
    label: &'static str,
}

struct Page {
    dir: ConfigDir,
    message: gtk::Label,
}

impl Page {
    /// The block as core.yaml has it.
    fn block(&self, section: &str) -> Mapping {
        core::get(&self.dir, section).and_then(|v| v.as_mapping().cloned()).unwrap_or_default()
    }

    /// Sets `section.key`, keeping the block's other keys; the core reloads.
    fn set(&self, section: &str, key: &str, value: Value) {
        let mut block = self.block(section);
        block.insert(key.into(), value);
        let result = core::save(&self.dir, section, Value::Mapping(block)).and_then(|()| core::command("reload"));
        crate::say(&self.message, &result.err().map(|e| e.to_string()).unwrap_or_default());
    }

    fn switch(self: &Rc<Self>, field: Field, default: bool) -> gtk::Box {
        let on = self.block(field.section).get(field.key).and_then(Value::as_bool).unwrap_or(default);
        let switch = gtk::Switch::builder().active(on).halign(gtk::Align::End).build();
        let page = self.clone();
        switch.connect_active_notify(move |s| page.set(field.section, field.key, s.is_active().into()));
        crate::row(&tr(field.label), &switch)
    }

    /// libinput's speed, from -1 to 1, written when let go of.
    fn speed(self: &Rc<Self>, section: &'static str) -> gtk::Box {
        let value = self.block(section).get("speed").and_then(Value::as_f64).unwrap_or(0.0);
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, -1.0, 1.0, 0.1);
        scale.set_value(value);
        scale.set_hexpand(true);
        scale.add_mark(0.0, gtk::PositionType::Bottom, None);
        let page = self.clone();
        let pending: Rc<std::cell::Cell<Option<glib::SourceId>>> = Rc::default();
        scale.connect_value_changed(move |s| {
            if let Some(id) = pending.take() {
                id.remove();
            }
            let v = (s.value() * 10.0).round() / 10.0;
            let page = page.clone();
            let again = pending.clone();
            pending.set(Some(glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                again.set(None);
                page.set(section, "speed", v.into());
            })));
        });
        crate::row(&tr("settings-mouse-speed"), &scale)
    }

    fn choice(self: &Rc<Self>, field: Field, choices: &'static [(&'static str, &'static str)]) -> gtk::Box {
        let labels: Vec<String> = choices.iter().map(|(_, l)| tr(l)).collect();
        let dropdown = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
        let now = self.block(field.section).get(field.key).and_then(Value::as_str).map(str::to_owned);
        let index = now.and_then(|n| choices.iter().position(|(v, _)| *v == n)).unwrap_or(0);
        dropdown.set_selected(index as u32);
        let page = self.clone();
        dropdown.connect_selected_notify(move |d| {
            if let Some((value, _)) = choices.get(d.selected() as usize) {
                page.set(field.section, field.key, (*value).into());
            }
        });
        crate::row(&tr(field.label), &dropdown)
    }
}

pub fn page(dir: &ConfigDir) -> gtk::Widget {
    let page = Rc::new(Page {
        dir: dir.clone(),
        message: crate::message(),
    });
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();

    let mouse = crate::power_card(&tr("settings-mouse"));
    mouse.append(&page.speed("mouse"));
    mouse.append(&page.choice(
        Field { section: "mouse", key: "profile", label: "settings-mouse-profile" },
        &[("adaptive", "settings-mouse-adaptive"), ("flat", "settings-mouse-flat")],
    ));
    mouse.append(&page.switch(Field { section: "mouse", key: "natural_scroll", label: "settings-mouse-natural" }, false));
    mouse.append(&page.switch(Field { section: "mouse", key: "left_handed", label: "settings-mouse-left" }, false));
    body.append(&mouse);

    if has_touchpad() {
        let pad = crate::power_card(&tr("settings-touchpad"));
        pad.append(&page.speed("touchpad"));
        pad.append(&page.switch(Field { section: "touchpad", key: "tap", label: "settings-touchpad-tap" }, true));
        pad.append(&page.switch(Field { section: "touchpad", key: "natural_scroll", label: "settings-mouse-natural" }, true));
        pad.append(&page.switch(
            Field { section: "touchpad", key: "disable_while_typing", label: "settings-touchpad-typing" },
            true,
        ));
        pad.append(&page.choice(
            Field { section: "touchpad", key: "click", label: "settings-touchpad-click" },
            &[("clickfinger", "settings-touchpad-fingers"), ("areas", "settings-touchpad-areas")],
        ));
        pad.append(&page.choice(
            Field { section: "touchpad", key: "scroll", label: "settings-touchpad-scroll" },
            &[("two_finger", "settings-touchpad-two"), ("edge", "settings-touchpad-edge"), ("none", "settings-touchpad-noscroll")],
        ));
        pad.append(&page.switch(
            Field { section: "touchpad", key: "middle_emulation", label: "settings-touchpad-middle" },
            false,
        ));
        pad.append(&page.switch(Field { section: "touchpad", key: "left_handed", label: "settings-mouse-left" }, false));
        // `gestures` is core.yaml's own key, not the touchpad block's.
        let gestures = gtk::Switch::builder()
            .active(core::get(dir, "gestures").and_then(|v| v.as_bool()).unwrap_or(true))
            .halign(gtk::Align::End)
            .build();
        let dir = dir.clone();
        let message = page.message.clone();
        gestures.connect_active_notify(move |s| {
            let result = core::save(&dir, "gestures", s.is_active().into()).and_then(|()| core::command("reload"));
            crate::say(&message, &result.err().map(|e| e.to_string()).unwrap_or_default());
        });
        let row = crate::row(&tr("settings-touchpad-gestures"), &gestures);
        row.set_tooltip_text(Some(&tr("settings-touchpad-gestures-tip")));
        pad.append(&row);
        body.append(&pad);
    }
    body.append(&page.message);
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .build()
        .upcast()
}
