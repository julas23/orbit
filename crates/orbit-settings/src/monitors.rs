//! The monitors page: each monitor's resolution, refresh rate, scale and
//! rotation. Apply sets them at once (`output NAME mode|scale|transform`)
//! and asks whether to keep them: kept, they go to `core.yaml` under
//! `HOST/NAME`; not answered in 15 seconds — a mode the monitor cannot
//! show leaves it dark — they go back to what they were.
//!
//! Where each monitor sits is the panel's monitors module's (drag them).

use std::{cell::RefCell, rc::Rc};

use gtk::{glib, prelude::*};
use orbit_app::i18n::{tr, tr_args};
use orbit_common::ConfigDir;

use crate::core::{self, Mode, Output};

const SCALES: &[f64] = &[1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];
const TURNS: &[&str] = &["normal", "90", "180", "270"];
/// Seconds before a change not kept goes back.
const KEEP: u32 = 15;
/// The choices for switching the monitors off (minutes; 0: never).
const OFF_MINUTES: &[u64] = &[0, 1, 2, 5, 10, 15, 30, 60];

/// What one monitor is set to.
#[derive(Debug, Clone, PartialEq)]
struct Setting {
    mode: Option<Mode>,
    scale: f64,
    transform: String,
}

impl Setting {
    fn of(output: &Output) -> Self {
        Self {
            mode: output.current_mode,
            scale: output.scale,
            transform: if output.transform.is_empty() {
                "normal".into()
            } else {
                output.transform.clone()
            },
        }
    }

    /// The core's commands that take `name` from `from` to this.
    fn commands(&self, name: &str, from: &Setting) -> Vec<String> {
        let mut commands = Vec::new();
        if let Some(mode) = self.mode
            && Some(mode) != from.mode
        {
            commands.push(format!("output {name} mode {}", mode.spec()));
        }
        if (self.scale - from.scale).abs() > 1e-6 {
            commands.push(format!("output {name} scale {}", self.scale));
        }
        if self.transform != from.transform {
            commands.push(format!("output {name} transform {}", self.transform));
        }
        commands
    }
}

/// One monitor's controls.
struct Row {
    output: Output,
    sizes: Vec<(i32, i32)>,
    /// The rates (mHz) of the size chosen, fastest first.
    rates: RefCell<Vec<i32>>,
    scales: Vec<f64>,
    /// The turns offered: the four, and the monitor's own if another.
    turns: Vec<String>,
    size: gtk::DropDown,
    rate: gtk::DropDown,
    scale: gtk::DropDown,
    turn: gtk::DropDown,
}

impl Row {
    fn new(output: Output) -> Rc<Self> {
        let mut sizes: Vec<(i32, i32)> = output.modes.iter().map(|m| (m.width, m.height)).collect();
        sizes.sort_by_key(|(w, h)| std::cmp::Reverse((*w as i64) * (*h as i64)));
        sizes.dedup();
        let preferred = output.modes.iter().find(|m| m.preferred).map(|m| (m.width, m.height));
        let labels: Vec<String> = sizes
            .iter()
            .map(|&(w, h)| {
                let text = format!("{w} × {h}");
                if Some((w, h)) == preferred {
                    tr_args("settings-recommended", &[("what", text.into())])
                } else {
                    text
                }
            })
            .collect();
        let size = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
        let mut scales = SCALES.to_vec();
        if !scales.iter().any(|s| (s - output.scale).abs() < 1e-6) {
            scales.push(output.scale);
            scales.sort_by(f64::total_cmp);
        }
        let percent: Vec<String> = scales.iter().map(|s| format!("{} %", (s * 100.0).round())).collect();
        let scale = gtk::DropDown::from_strings(&percent.iter().map(String::as_str).collect::<Vec<_>>());
        let mut turns: Vec<String> = TURNS.iter().map(|t| (*t).to_owned()).collect();
        let own = Setting::of(&output).transform;
        if !turns.contains(&own) {
            turns.push(own);
        }
        let labels: Vec<String> = turns
            .iter()
            .map(|t| match TURNS.contains(&t.as_str()) {
                true => tr(&format!("settings-turn-{t}")),
                false => t.clone(),
            })
            .collect();
        let turn = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
        let row = Rc::new(Self {
            output,
            sizes,
            rates: RefCell::default(),
            scales,
            turns,
            size,
            rate: gtk::DropDown::from_strings(&[]),
            scale,
            turn,
        });
        row.show(&Setting::of(&row.output));
        row.size.connect_selected_notify({
            let row = Rc::downgrade(&row);
            move |_| {
                if let Some(row) = row.upgrade() {
                    let keep = row.rates.borrow().get(row.rate.selected() as usize).copied();
                    row.fill_rates(keep);
                }
            }
        });
        row
    }

    /// The controls set to `setting`.
    fn show(&self, setting: &Setting) {
        if let Some(mode) = setting.mode
            && let Some(i) = self.sizes.iter().position(|s| *s == (mode.width, mode.height))
        {
            self.size.set_selected(i as u32);
        }
        self.fill_rates(setting.mode.map(|m| m.refresh));
        let scale = self.scales.iter().position(|s| (s - setting.scale).abs() < 1e-6);
        self.scale.set_selected(scale.unwrap_or(0) as u32);
        self.turn
            .set_selected(self.turns.iter().position(|t| *t == setting.transform).unwrap_or(0) as u32);
    }

    /// The rates of the size chosen, `near` (mHz) picked, else the fastest.
    fn fill_rates(&self, near: Option<i32>) {
        let Some(&(w, h)) = self.sizes.get(self.size.selected() as usize) else {
            return;
        };
        let mut rates: Vec<i32> = self
            .output
            .modes
            .iter()
            .filter(|m| (m.width, m.height) == (w, h))
            .map(|m| m.refresh)
            .collect();
        rates.sort_by_key(|r| std::cmp::Reverse(*r));
        rates.dedup();
        let labels: Vec<String> = rates.iter().map(|r| format!("{} Hz", core::hz(*r))).collect();
        self.rate.set_model(Some(&gtk::StringList::new(
            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        )));
        let pick = near
            .and_then(|near| (0..rates.len()).min_by_key(|i| (rates[*i] - near).abs()))
            .unwrap_or(0);
        self.rate.set_selected(pick as u32);
        *self.rates.borrow_mut() = rates;
    }

    /// What the controls say.
    fn setting(&self) -> Setting {
        let size = self.sizes.get(self.size.selected() as usize).copied();
        let rate = self.rates.borrow().get(self.rate.selected() as usize).copied();
        let mode = size.zip(rate).and_then(|((w, h), r)| {
            self.output
                .modes
                .iter()
                .find(|m| (m.width, m.height, m.refresh) == (w, h, r))
                .copied()
        });
        Setting {
            mode: mode.or(self.output.current_mode),
            scale: self.scales.get(self.scale.selected() as usize).copied().unwrap_or(1.0),
            transform: self
                .turns
                .get(self.turn.selected() as usize)
                .cloned()
                .unwrap_or_else(|| "normal".into()),
        }
    }

    fn widget(&self) -> gtk::Widget {
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["card"])
            .build();
        let product = [self.output.make.as_str(), self.output.model.as_str()]
            .iter()
            .filter(|s| !s.is_empty() && **s != "Unknown")
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        let title = if product.is_empty() {
            self.output.name.clone()
        } else {
            format!("{} · {product}", self.output.name)
        };
        card.append(
            &gtk::Label::builder()
                .label(title)
                .xalign(0.0)
                .css_classes(["heading"])
                .build(),
        );
        card.append(&crate::row(&tr("settings-resolution"), &self.size));
        card.append(&crate::row(&tr("settings-refresh"), &self.rate));
        card.append(&crate::row(&tr("settings-scale"), &self.scale));
        card.append(&crate::row(&tr("settings-rotation"), &self.turn));
        card.upcast()
    }
}

struct Page {
    dir: ConfigDir,
    list: gtk::Box,
    rows: RefCell<Vec<Rc<Row>>>,
    apply: gtk::Button,
    /// The question after Apply, and its countdown.
    keep_bar: gtk::Box,
    keep_label: gtk::Label,
    /// What each changed monitor was before Apply, while not yet kept.
    before: RefCell<Vec<(String, Setting, Setting)>>,
    countdown: RefCell<Option<glib::SourceId>>,
    message: gtk::Label,
}

pub fn page(dir: &ConfigDir) -> gtk::Widget {
    let keep_label = gtk::Label::builder().xalign(0.0).hexpand(true).build();
    let keep = gtk::Button::with_label(&tr("settings-keep"));
    keep.add_css_class("suggested-action");
    let revert = gtk::Button::with_label(&tr("settings-revert"));
    let keep_bar = gtk::Box::builder()
        .spacing(10)
        .visible(false)
        .css_classes(["keep"])
        .build();
    keep_bar.append(&keep_label);
    keep_bar.append(&revert);
    keep_bar.append(&keep);
    let page = Rc::new(Page {
        dir: dir.clone(),
        list: gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(20)
            .build(),
        rows: RefCell::default(),
        apply: gtk::Button::with_label(&tr("settings-apply")),
        keep_bar,
        keep_label,
        before: RefCell::default(),
        countdown: RefCell::default(),
        message: crate::message(),
    });
    page.apply.add_css_class("suggested-action");
    page.apply.set_halign(gtk::Align::End);
    page.apply.connect_clicked({
        let page = page.clone();
        move |_| page.apply()
    });
    keep.connect_clicked({
        let page = page.clone();
        move |_| page.keep()
    });
    revert.connect_clicked({
        let page = page.clone();
        move |_| page.revert()
    });
    page.refresh();

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();
    body.append(&page.list);
    let foot = gtk::Box::builder().spacing(10).build();
    foot.append(
        &gtk::Label::builder()
            .label(tr("settings-where"))
            .xalign(0.0)
            .wrap(true)
            .hexpand(true)
            .css_classes(["dim"])
            .build(),
    );
    foot.append(&page.apply);
    body.append(&foot);
    body.append(&page.keep_bar);
    body.append(&power_card(&page));
    body.append(&page.message);
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build()
        .upcast()
}

/// When the monitors switch off with no input (`idle_off`, this machine's):
/// written at once and the core reloads, as the screensaver.
fn power_card(page: &Rc<Page>) -> gtk::Box {
    let labels: Vec<String> = OFF_MINUTES
        .iter()
        .map(|m| match m {
            0 => tr("settings-saver-never"),
            m => tr_args("settings-saver-minutes", &[("minutes", (*m as i64).into())]),
        })
        .collect();
    let off = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    off.set_tooltip_text(Some(&tr("settings-off-tip")));
    let seconds = core::idle_off(&page.dir);
    // The file's seconds, to the nearest choice.
    let near = OFF_MINUTES
        .iter()
        .enumerate()
        .min_by_key(|(_, m)| (**m * 60).abs_diff(seconds))
        .map_or(0, |(i, _)| i);
    off.set_selected(near as u32);
    off.connect_selected_notify({
        let page = page.clone();
        move |dd| {
            let Some(minutes) = OFF_MINUTES.get(dd.selected() as usize) else {
                return;
            };
            if minutes * 60 == core::idle_off(&page.dir) {
                return;
            }
            let told = core::save_idle_off(&page.dir, minutes * 60).and_then(|()| core::command("reload"));
            match told {
                Ok(()) => crate::say(&page.message, ""),
                Err(err) => crate::say(&page.message, &format!("{}: {err:#}", tr("settings-not-told"))),
            }
        }
    });
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(["card"])
        .build();
    card.append(&crate::row(&tr("settings-off"), &off));
    card
}

impl Page {
    /// The monitors as the core has them now.
    fn refresh(&self) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let outputs = match core::outputs() {
            Ok(outputs) => outputs,
            Err(err) => {
                crate::say(&self.message, &format!("{}: {err:#}", tr("settings-not-told")));
                Vec::new()
            }
        };
        let rows: Vec<Rc<Row>> = outputs.into_iter().map(Row::new).collect();
        for row in &rows {
            self.list.append(&row.widget());
        }
        *self.rows.borrow_mut() = rows;
    }

    /// Sends `commands`; the first error, said.
    fn send(&self, commands: &[String]) -> bool {
        for command in commands {
            if let Err(err) = core::command(command) {
                crate::say(&self.message, &format!("{command}: {err:#}"));
                return false;
            }
        }
        true
    }

    fn apply(self: &Rc<Self>) {
        let changes: Vec<(String, Setting, Setting)> = self
            .rows
            .borrow()
            .iter()
            .map(|row| (row.output.name.clone(), Setting::of(&row.output), row.setting()))
            .filter(|(_, from, to)| from != to)
            .collect();
        if changes.is_empty() {
            return;
        }
        crate::say(&self.message, "");
        let commands: Vec<String> = changes
            .iter()
            .flat_map(|(name, from, to)| to.commands(name, from))
            .collect();
        *self.before.borrow_mut() = changes;
        if !self.send(&commands) {
            self.revert();
            return;
        }
        self.apply.set_sensitive(false);
        self.keep_bar.set_visible(true);
        let left = std::cell::Cell::new(KEEP);
        self.keep_label
            .set_label(&tr_args("settings-keep-question", &[("seconds", KEEP.into())]));
        let page = Rc::downgrade(self);
        let id = glib::timeout_add_seconds_local(1, move || {
            let Some(page) = page.upgrade() else {
                return glib::ControlFlow::Break;
            };
            left.set(left.get() - 1);
            if left.get() == 0 {
                page.countdown.borrow_mut().take();
                page.revert();
                return glib::ControlFlow::Break;
            }
            page.keep_label
                .set_label(&tr_args("settings-keep-question", &[("seconds", left.get().into())]));
            glib::ControlFlow::Continue
        });
        *self.countdown.borrow_mut() = Some(id);
    }

    fn stop_countdown(&self) {
        if let Some(id) = self.countdown.borrow_mut().take() {
            id.remove();
        }
        self.keep_bar.set_visible(false);
        self.apply.set_sensitive(true);
    }

    /// The changes written to core.yaml, for the next sessions.
    fn keep(&self) {
        self.stop_countdown();
        for (name, _, to) in self.before.borrow_mut().drain(..) {
            let saved = core::save_output(&self.dir, &name, |entry| {
                if let Some(mode) = to.mode {
                    entry.insert("mode".into(), mode.spec().into());
                }
                entry.insert("scale".into(), to.scale.into());
                if to.transform == "normal" {
                    entry.remove("transform");
                } else {
                    entry.insert("transform".into(), to.transform.clone().into());
                }
            });
            if let Err(err) = saved {
                crate::say(&self.message, &format!("{err:#}"));
            }
        }
        self.refresh();
    }

    /// Back to what the monitors were before Apply.
    fn revert(&self) {
        self.stop_countdown();
        let back: Vec<String> = self
            .before
            .borrow_mut()
            .drain(..)
            .flat_map(|(name, from, to)| from.commands(&name, &to))
            .collect();
        self.send(&back);
        self.refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_only_for_what_changed() {
        let mode = |w, h, r| Mode {
            width: w,
            height: h,
            refresh: r,
            preferred: false,
        };
        let from = Setting {
            mode: Some(mode(1920, 1080, 60000)),
            scale: 1.0,
            transform: "normal".into(),
        };
        let to = Setting {
            mode: Some(mode(2560, 1440, 143_980)),
            ..from.clone()
        };
        assert_eq!(to.commands("DP-1", &from), ["output DP-1 mode 2560x1440@143.98"]);
        assert_eq!(from.commands("DP-1", &to), ["output DP-1 mode 1920x1080@60"]);
        let turned = Setting {
            scale: 1.5,
            transform: "90".into(),
            ..from.clone()
        };
        assert_eq!(
            turned.commands("DP-1", &from),
            ["output DP-1 scale 1.5", "output DP-1 transform 90"]
        );
        assert!(from.commands("DP-1", &from).is_empty());
    }
}
