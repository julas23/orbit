//! The Dock page (docs/PLANO-SETTINGS.md §3.3): dock.yaml's edge, monitor,
//! alignment, icon size and reserved space, and its launchers and
//! separators (moved, removed, an installed application or a separator
//! added). The dock edits itself in place too; this page reaches the same
//! file another way. The dock reads its file at start: after a change it
//! is started again (`orbit-session`).

use std::{cell::RefCell, rc::Rc};

use gtk::{gio, prelude::*};
use orbit_app::i18n::tr;
use orbit_common::{
    ConfigDir,
    config::{Align, DockApp, DockConfig, DockEntry, Edge, MonitorSpec},
};

use crate::{core, panel_page::restart};

const EDGES: [(Edge, &str, &str); 4] = [
    (Edge::Bottom, "bottom", "settings-edge-bottom"),
    (Edge::Top, "top", "settings-edge-top"),
    (Edge::Left, "left", "settings-edge-left"),
    (Edge::Right, "right", "settings-edge-right"),
];
const ALIGNS: [(Align, &str, &str); 3] = [
    (Align::Start, "start", "settings-dock-align-start"),
    (Align::Center, "center", "settings-dock-align-center"),
    (Align::End, "end", "settings-dock-align-end"),
];

/// The installed application `app` launches, if it names one.
fn app_info(app: &DockApp) -> Option<gio_unix::DesktopAppInfo> {
    let desktop = app.desktop.as_deref()?;
    if desktop.starts_with('/') || desktop.starts_with('~') {
        let path = desktop.replacen('~', &gtk::glib::home_dir().to_string_lossy(), 1);
        gio_unix::DesktopAppInfo::from_filename(path)
    } else {
        gio_unix::DesktopAppInfo::new(desktop)
    }
}

struct Page {
    dir: ConfigDir,
    apps: RefCell<Vec<DockEntry>>,
    list: gtk::Box,
    message: gtk::Label,
}

impl Page {
    fn save_key(&self, key: &str, line: String) {
        let result = self.dir.save_app_keys("dock", &[key], &line).map(|()| restart());
        crate::say(&self.message, &result.err().map(|e| e.to_string()).unwrap_or_default());
    }

    fn save_apps(&self) {
        let result = self.dir.save_dock_apps(&self.apps.borrow(), false).map(|()| restart());
        crate::say(&self.message, &result.err().map(|e| e.to_string()).unwrap_or_default());
    }

    fn fill(self: &Rc<Self>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let apps = self.apps.borrow().clone();
        let count = apps.len();
        for (i, entry) in apps.iter().enumerate() {
            let line = gtk::Box::builder().spacing(10).build();
            match entry.app() {
                Some(app) => {
                    let info = app_info(app);
                    let name = app
                        .name
                        .clone()
                        .or_else(|| info.as_ref().map(|i| i.display_name().to_string()))
                        .or_else(|| app.exec.clone())
                        .unwrap_or_else(|| "?".into());
                    let image = match (&app.icon, info.as_ref().and_then(|i| i.icon())) {
                        (Some(icon), _) => gtk::Image::from_icon_name(icon),
                        (None, Some(gicon)) => gtk::Image::from_gicon(&gicon),
                        _ => gtk::Image::from_icon_name("application-x-executable"),
                    };
                    image.set_pixel_size(24);
                    line.append(&image);
                    line.append(&gtk::Label::builder().label(name).xalign(0.0).hexpand(true).build());
                }
                None => line.append(
                    &gtk::Label::builder()
                        .label(tr("settings-dock-separator"))
                        .xalign(0.0)
                        .hexpand(true)
                        .css_classes(["dim"])
                        .build(),
                ),
            }
            for (icon, enabled, step) in [("go-up-symbolic", i > 0, -1i32), ("go-down-symbolic", i + 1 < count, 1)] {
                let b = gtk::Button::builder().icon_name(icon).sensitive(enabled).css_classes(["flat"]).build();
                let page = self.clone();
                b.connect_clicked(move |_| {
                    page.apps.borrow_mut().swap(i, (i as i32 + step) as usize);
                    page.save_apps();
                    page.fill();
                });
                line.append(&b);
            }
            let remove = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text(tr("settings-kb-remove"))
                .css_classes(["flat"])
                .build();
            let page = self.clone();
            remove.connect_clicked(move |_| {
                page.apps.borrow_mut().remove(i);
                page.save_apps();
                page.fill();
            });
            line.append(&remove);
            self.list.append(&line);
        }
    }

    /// The installed applications, searched by typing; the one chosen is
    /// added at the end.
    fn add_dialog(self: &Rc<Self>, button: &gtk::Button) {
        let window = gtk::Window::builder()
            .title(tr("settings-dock-add"))
            .modal(true)
            .default_width(460)
            .css_classes(["orbit-settings"])
            .build();
        if let Some(parent) = button.root().and_downcast::<gtk::Window>() {
            window.set_transient_for(Some(&parent));
        }
        let mut apps: Vec<(String, String)> = gio::AppInfo::all()
            .into_iter()
            .filter(|a| a.should_show())
            .filter_map(|a| Some((a.id()?.to_string(), a.display_name().to_string())))
            .collect();
        apps.sort_by_key(|a| a.1.to_lowercase());
        let labels: Vec<&str> = apps.iter().map(|(_, n)| n.as_str()).collect();
        let model = gtk::StringList::new(&labels);
        let pick = gtk::DropDown::builder()
            .model(&model)
            .enable_search(true)
            .expression(gtk::PropertyExpression::new(gtk::StringObject::static_type(), None::<gtk::Expression>, "string"))
            .hexpand(true)
            .build();
        pick.set_search_match_mode(gtk::StringFilterMatchMode::Substring);
        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).css_classes(["page"]).build();
        body.append(&pick);
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
                if let Some((id, _)) = apps.get(pick.selected() as usize) {
                    page.apps.borrow_mut().push(DockEntry::App(DockApp {
                        desktop: Some(id.clone()),
                        name: None,
                        exec: None,
                        icon: None,
                        group: None,
                        show: true,
                    }));
                    page.save_apps();
                    page.fill();
                }
                window.close();
            }
        });
        window.present();
    }
}

pub fn page(dir: &ConfigDir) -> gtk::Widget {
    let config: DockConfig = dir.dock().unwrap_or_else(|e| {
        log::warn!("dock.yaml: {e}");
        DockConfig::default()
    });
    let page = Rc::new(Page {
        dir: dir.clone(),
        apps: RefCell::new(config.apps.clone()),
        list: gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(5).build(),
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
                page.save_key("edge", format!("edge: {name}\n"));
            }
        }
    });
    general.append(&crate::row(&tr("settings-panel-edge"), &edge));

    let labels: Vec<String> = ALIGNS.iter().map(|(_, _, l)| tr(l)).collect();
    let align = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    align.set_selected(ALIGNS.iter().position(|(a, _, _)| *a == config.align).unwrap_or(1) as u32);
    align.connect_selected_notify({
        let page = page.clone();
        move |d| {
            if let Some((_, name, _)) = ALIGNS.get(d.selected() as usize) {
                page.save_key("align", format!("align: {name}\n"));
            }
        }
    });
    general.append(&crate::row(&tr("settings-dock-align"), &align));

    let names: Vec<String> = core::outputs().map(|o| o.into_iter().map(|o| o.name).collect()).unwrap_or_default();
    let mut labels = vec![tr("settings-panel-first-monitor")];
    labels.extend(names.iter().cloned());
    let monitor = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let selected = match &config.monitor {
        MonitorSpec::Name(n) => names.iter().position(|x| x == n).map_or(0, |i| i + 1),
        MonitorSpec::Index(0) => 0,
        MonitorSpec::Index(i) => (*i).min(names.len().saturating_sub(1)) + 1,
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
                page.save_key("monitor", line);
            }
        }
    });
    general.append(&crate::row(&tr("settings-panel-monitor"), &monitor));

    let icons = gtk::SpinButton::with_range(16.0, 64.0, 2.0);
    icons.set_value(f64::from(config.icon_size));
    icons.connect_value_changed({
        let page = page.clone();
        move |s| page.save_key("icon_size", format!("icon_size: {}\n", s.value() as i64))
    });
    general.append(&crate::row(&tr("settings-dock-icon-size"), &icons));
    let reserve = gtk::Switch::builder().active(config.reserve).halign(gtk::Align::End).build();
    reserve.connect_active_notify({
        let page = page.clone();
        move |s| page.save_key("reserve", format!("reserve: {}\n", s.is_active()))
    });
    general.append(&crate::row(&tr("settings-panel-reserve"), &reserve));
    body.append(&general);

    let launchers = crate::power_card(&tr("settings-dock-launchers"));
    launchers.append(&gtk::Label::builder().label(tr("settings-dock-tip")).xalign(0.0).wrap(true).css_classes(["dim"]).build());
    launchers.append(&page.list);
    let separator = gtk::Button::with_label(&tr("settings-dock-add-separator"));
    separator.connect_clicked({
        let page = page.clone();
        move |_| {
            page.apps.borrow_mut().push(DockEntry::SEPARATOR);
            page.save_apps();
            page.fill();
        }
    });
    let add = gtk::Button::with_label(&tr("settings-dock-add"));
    add.add_css_class("suggested-action");
    add.connect_clicked({
        let page = page.clone();
        move |b| page.add_dialog(b)
    });
    let buttons = gtk::Box::builder().spacing(10).halign(gtk::Align::End).build();
    buttons.append(&separator);
    buttons.append(&add);
    launchers.append(&buttons);
    page.fill();
    body.append(&launchers);
    body.append(&page.message);
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .build()
        .upcast()
}
