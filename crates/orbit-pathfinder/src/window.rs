//! The window: the sidebar of services in groups on the left (Wavebox's
//! layout), the page of the service shown on the right under a thin bar
//! (back, forward, reload, title, open outside).
//!
//! Groups fold away; a folded group shows the sum of its unread counts.
//! A right click on a service offers reload, mute, open outside, clear its
//! data and remove; "+" at the bottom adds one. Ctrl+1…9 picks a service,
//! Ctrl+Tab cycles the ones loaded, Ctrl+R reloads, Alt+←/→ go back and
//! forward, Ctrl+plus/minus/0 zoom, F12 opens the web inspector.

use std::{
    cell::RefCell,
    collections::HashMap,
    path::PathBuf,
    rc::Rc,
};

use gtk::{gdk, gio, glib, prelude::*};
use orbit_app::i18n::{text, tr, tr_args};
use orbit_common::ConfigDir;
use webkit6::prelude::*;

use crate::{
    config::{self, Config},
    service::{Env, Service},
    state::State,
};

/// A group in the sidebar: its unread badge, its services and the revealer
/// that folds it.
type GroupRow = (gtk::Label, Vec<Rc<Service>>, gtk::Revealer);

/// What a shortcut does, given the window and the action's parameter.
type Shortcut = Box<dyn Fn(&Rc<Window>, Option<&glib::Variant>)>;

pub struct Window {
    window: gtk::ApplicationWindow,
    env: Rc<Env>,
    cfg_path: PathBuf,
    cfg: RefCell<Config>,
    /// In sidebar order.
    services: RefCell<Vec<Rc<Service>>>,
    groups_box: gtk::Box,
    /// Each group's unread badge, its services and whether it is folded.
    groups: RefCell<Vec<GroupRow>>,
    stack: gtk::Stack,
    title: gtk::Label,
    back: gtk::Button,
    forward: gtk::Button,
    current: RefCell<Option<Rc<Service>>>,
    state: RefCell<State>,
}

impl Window {
    pub fn new(app: &gtk::Application, dir: &ConfigDir) -> anyhow::Result<Rc<Self>> {
        let cfg: Config = dir.app("pathfinder")?;
        let base = |root: PathBuf| root.join("orbit").join("pathfinder");
        let env = Rc::new(Env {
            app: app.clone(),
            data: base(glib::user_data_dir()),
            cache: base(glib::user_cache_dir()),
            user_agent: cfg.user_agent.clone(),
            external_browser: cfg.external_browser.clone(),
            changed: RefCell::new(None),
            page_changed: RefCell::new(None),
        });

        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title(tr("pathfinder-title"))
            .default_width(1400)
            .default_height(900)
            .css_classes(["orbit-pathfinder"])
            .build();

        let groups_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["groups"]).build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&groups_box)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .hexpand(false)
            .build();
        let add = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text(tr("pathfinder-add"))
            .css_classes(["flat", "footer-button"])
            .build();
        let menu = gtk::MenuButton::builder()
            .icon_name("emblem-system-symbolic")
            .tooltip_text(tr("pathfinder-menu"))
            .css_classes(["flat", "footer-button"])
            .direction(gtk::ArrowType::Right)
            .build();
        // A fixed width: the group names inside would otherwise widen it.
        let sidebar = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["sidebar"])
            .hexpand(false)
            .width_request(70)
            .build();
        sidebar.append(&scroller);
        sidebar.append(&add);
        sidebar.append(&menu);

        let button = |icon: &str, tip: &str| {
            gtk::Button::builder().icon_name(icon).tooltip_text(tr(tip)).css_classes(["flat"]).build()
        };
        let back = button("go-previous-symbolic", "pathfinder-back");
        let forward = button("go-next-symbolic", "pathfinder-forward");
        let reload = button("view-refresh-symbolic", "pathfinder-reload");
        let outside = button("send-to-symbolic", "pathfinder-open-external");
        let title = gtk::Label::builder()
            .hexpand(true)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["page-title"])
            .build();
        let bar = gtk::Box::builder().spacing(5).css_classes(["bar"]).build();
        for w in [back.upcast_ref::<gtk::Widget>(), forward.upcast_ref(), reload.upcast_ref(), title.upcast_ref(), outside.upcast_ref()] {
            bar.append(w);
        }

        let stack = gtk::Stack::builder().hexpand(true).vexpand(true).build();
        let empty = gtk::Label::builder().label(tr("pathfinder-empty")).css_classes(["empty"]).build();
        stack.add_named(&empty, Some("empty"));

        let page = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        page.append(&bar);
        page.append(&stack);
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.append(&sidebar);
        root.append(&page);
        window.set_child(Some(&root));

        let this = Rc::new(Self {
            window,
            env,
            cfg_path: config::path(dir),
            cfg: RefCell::new(cfg),
            services: RefCell::new(Vec::new()),
            groups_box,
            groups: RefCell::new(Vec::new()),
            stack,
            title,
            back,
            forward,
            current: RefCell::new(None),
            state: RefCell::new(State::load()),
        });

        {
            let weak = Rc::downgrade(&this);
            *this.env.changed.borrow_mut() = Some(Box::new(move |()| {
                if let Some(w) = weak.upgrade() {
                    w.update_counts();
                }
            }));
            let weak = Rc::downgrade(&this);
            *this.env.page_changed.borrow_mut() = Some(Box::new(move |service| {
                if let Some(w) = weak.upgrade()
                    && w.current.borrow().as_ref().is_some_and(|c| c.key == service.key)
                {
                    w.update_bar();
                }
            }));
        }
        this.back.connect_clicked(glib::clone!(#[weak] this, move |_| this.with_view(|v| v.go_back())));
        this.forward.connect_clicked(glib::clone!(#[weak] this, move |_| this.with_view(|v| v.go_forward())));
        reload.connect_clicked(glib::clone!(#[weak] this, move |_| this.with_view(|v| v.reload())));
        outside.connect_clicked(glib::clone!(#[weak] this, move |_| {
            if let Some(uri) = this.current_view().and_then(|v| v.uri()) {
                this.env.open_external(&uri);
            }
        }));
        add.connect_clicked(glib::clone!(#[weak] this, move |_| this.add_dialog()));
        menu.set_popover(Some(&this.settings_menu()));
        this.install_keys(app);
        this.keep_logins();
        this.install_actions(app);

        this.rebuild();
        // Services always active load now, so their counts arrive.
        for service in this.services.borrow().iter() {
            if service.cfg.borrow().always_active {
                this.attach(service);
            }
        }
        let last = this.state.borrow().last.clone();
        let first = {
            let services = this.services.borrow();
            last.and_then(|k| services.iter().find(|s| s.key == k).cloned()).or_else(|| services.first().cloned())
        };
        if let Some(service) = first {
            this.select(&service);
        }
        Ok(this)
    }

    pub fn window(&self) -> &gtk::ApplicationWindow {
        &self.window
    }

    fn current_view(&self) -> Option<webkit6::WebView> {
        self.current.borrow().as_ref().and_then(|s| s.view())
    }

    fn with_view(&self, f: impl FnOnce(&webkit6::WebView)) {
        if let Some(view) = self.current_view() {
            f(&view);
        }
    }

    /// Builds the sidebar from the config, keeping the services (and their
    /// loaded pages) that are still there.
    fn rebuild(self: &Rc<Self>) {
        while let Some(child) = self.groups_box.first_child() {
            self.groups_box.remove(&child);
        }
        let old: HashMap<String, Rc<Service>> =
            self.services.borrow_mut().drain(..).map(|s| (s.key.clone(), s)).collect();
        let mut kept = HashMap::new();
        let mut groups = Vec::new();
        let cfg = self.cfg.borrow().clone();
        for group in &cfg.groups {
            let collapsed = self.state.borrow().collapsed.contains(&group.name);
            let name = gtk::Label::builder()
                .label(&group.name)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .max_width_chars(8)
                .hexpand(true)
                .xalign(0.0)
                .build();
            let arrow = gtk::Image::from_icon_name(if collapsed { "pan-end-symbolic" } else { "pan-down-symbolic" });
            let badge = gtk::Label::builder().css_classes(["badge", "group-badge"]).visible(false).build();
            let head = gtk::Box::builder().spacing(2).build();
            head.append(&arrow);
            head.append(&name);
            head.append(&badge);
            let header = gtk::Button::builder()
                .child(&head)
                .tooltip_text(&group.name)
                .css_classes(["flat", "group-header"])
                .build();
            let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["group-services"]).build();
            let revealer = gtk::Revealer::builder()
                .child(&list)
                .reveal_child(!collapsed)
                .transition_type(gtk::RevealerTransitionType::SlideDown)
                .build();
            let mut members = Vec::new();
            for cfg in &group.services {
                let key = cfg.key();
                let service = match old.get(&key) {
                    Some(s) => {
                        *s.cfg.borrow_mut() = cfg.clone();
                        s.clone()
                    }
                    None => {
                        let zoom = self.state.borrow().zoom.get(&key).copied().unwrap_or(1.0);
                        let s = Service::new(cfg.clone(), &self.env, zoom);
                        self.wire(&s);
                        s
                    }
                };
                if let Some(parent) = service.button.parent().and_downcast::<gtk::Box>() {
                    parent.remove(&service.button);
                }
                list.append(&service.button);
                kept.insert(key, service.clone());
                members.push(service.clone());
                self.services.borrow_mut().push(service);
            }
            header.connect_clicked(glib::clone!(
                #[weak(rename_to = this)]
                self,
                #[weak]
                revealer,
                #[weak]
                arrow,
                #[strong(rename_to = group)]
                group.name,
                move |_| {
                    let open = !revealer.reveals_child();
                    revealer.set_reveal_child(open);
                    arrow.set_icon_name(Some(if open { "pan-down-symbolic" } else { "pan-end-symbolic" }));
                    {
                        let mut state = this.state.borrow_mut();
                        state.collapsed.retain(|g| *g != group);
                        if !open {
                            state.collapsed.push(group.clone());
                        }
                        state.save();
                    }
                    this.update_counts();
                }
            ));
            self.groups_box.append(&header);
            self.groups_box.append(&revealer);
            groups.push((badge, members, revealer));
        }
        *self.groups.borrow_mut() = groups;
        // Services gone from the config: their pages go too.
        for (key, service) in old {
            if !kept.contains_key(&key) {
                if let Some(view) = service.view() {
                    self.stack.remove(&view);
                }
                service.drop_view();
                if self.current.borrow().as_ref().is_some_and(|c| c.key == key) {
                    *self.current.borrow_mut() = None;
                }
            }
        }
        if self.current.borrow().is_none() {
            match self.services.borrow().first().cloned() {
                Some(first) => self.select(&first),
                None => self.stack.set_visible_child_name("empty"),
            }
        }
        self.update_counts();
    }

    /// Clicks on a service's button.
    fn wire(self: &Rc<Self>, service: &Rc<Service>) {
        let weak = Rc::downgrade(service);
        service.button.connect_clicked(glib::clone!(
            #[weak(rename_to = this)]
            self,
            #[strong]
            weak,
            move |_| {
                if let Some(s) = weak.upgrade() {
                    this.select(&s);
                }
            }
        ));
        let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = this)]
            self,
            #[strong]
            weak,
            move |_, _, _, _| {
                if let Some(s) = weak.upgrade() {
                    this.context_menu(&s);
                }
            }
        ));
        service.button.add_controller(click);
        let middle = gtk::GestureClick::builder().button(gdk::BUTTON_MIDDLE).build();
        middle.connect_pressed(move |_, _, _, _| {
            if let Some(s) = weak.upgrade() {
                s.reload();
            }
        });
        service.button.add_controller(middle);
    }

    /// Makes the service's page and puts it in the stack.
    fn attach(&self, service: &Rc<Service>) -> webkit6::WebView {
        let view = service.ensure_view();
        if view.parent().is_none() {
            self.stack.add_named(&view, Some(&service.key));
        }
        view
    }

    fn select(self: &Rc<Self>, service: &Rc<Service>) {
        if let Some(old) = self.current.borrow().as_ref() {
            old.button.remove_css_class("active");
        }
        let view = self.attach(service);
        self.stack.set_visible_child(&view);
        service.button.add_css_class("active");
        *self.current.borrow_mut() = Some(service.clone());
        view.grab_focus();
        self.update_bar();
        let mut state = self.state.borrow_mut();
        if state.last.as_deref() != Some(&service.key) {
            state.last = Some(service.key.clone());
            state.save();
        }
    }

    fn update_bar(&self) {
        let Some(view) = self.current_view() else {
            self.title.set_label("");
            return;
        };
        let title = view.title().filter(|t| !t.is_empty()).or_else(|| view.uri()).unwrap_or_default();
        self.title.set_label(&title);
        self.title.set_tooltip_text(view.uri().as_deref());
        self.back.set_sensitive(view.can_go_back());
        self.forward.set_sensitive(view.can_go_forward());
    }

    /// Group badges (on folded groups) and the window title.
    fn update_counts(&self) {
        let mut total = 0;
        for (badge, members, revealer) in self.groups.borrow().iter() {
            let sum: u32 = members.iter().map(|s| s.unread()).sum();
            total += sum;
            badge.set_label(&if sum > 99 { "99+".to_owned() } else { sum.to_string() });
            badge.set_visible(sum > 0 && !revealer.reveals_child());
        }
        let title = tr("pathfinder-title");
        self.window.set_title(Some(&if total > 0 { format!("({total}) {title}") } else { title }));
    }

    fn context_menu(self: &Rc<Self>, service: &Rc<Service>) {
        let popover = gtk::Popover::builder().has_arrow(false).position(gtk::PositionType::Right).build();
        popover.set_parent(&service.button);
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["menu-list"]).build();
        let item = |label: String, f: Box<dyn Fn()>| {
            let b = gtk::Button::builder().label(label).css_classes(["flat", "menu-item"]).build();
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            let p = popover.clone();
            b.connect_clicked(move |_| {
                p.popdown();
                f();
            });
            list.append(&b);
        };
        let s = service.clone();
        item(tr("pathfinder-reload"), Box::new(move || s.reload()));
        let muted = service.cfg.borrow().muted;
        let (s, this) = (service.clone(), self.clone());
        item(
            tr(if muted { "pathfinder-unmute" } else { "pathfinder-mute" }),
            Box::new(move || {
                s.set_muted(!muted);
                this.store_service(&s);
            }),
        );
        let (s, this) = (service.clone(), self.clone());
        item(
            tr("pathfinder-open-external"),
            Box::new(move || {
                let uri = s.view().and_then(|v| v.uri()).map(|u| u.to_string()).unwrap_or_else(|| s.cfg.borrow().url.clone());
                this.env.open_external(&uri);
            }),
        );
        let (s, this) = (service.clone(), self.clone());
        item(
            tr("pathfinder-clear"),
            Box::new(move || {
                let s = s.clone();
                this.confirm(&tr_args("pathfinder-clear-confirm", &[("name", text(s.name()))]), &tr("pathfinder-clear"), move || s.clear_data());
            }),
        );
        let (s, this) = (service.clone(), self.clone());
        item(
            tr("pathfinder-remove"),
            Box::new(move || {
                let (s2, this2) = (s.clone(), this.clone());
                this.confirm(&tr_args("pathfinder-remove-confirm", &[("name", text(s.name()))]), &tr("pathfinder-remove"), move || this2.remove(&s2));
            }),
        );
        popover.set_child(Some(&list));
        popover.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        popover.popup();
    }

    fn confirm(&self, message: &str, action: &str, f: impl FnOnce() + 'static) {
        let dialog = gtk::AlertDialog::builder()
            .message(message)
            .buttons([tr("pathfinder-cancel"), action.to_owned()])
            .cancel_button(0)
            .default_button(0)
            .modal(true)
            .build();
        dialog.choose(Some(&self.window), gio::Cancellable::NONE, move |r| {
            if r == Ok(1) {
                f();
            }
        });
    }

    /// Writes a service's changed settings (mute) back to the config.
    fn store_service(&self, service: &Service) {
        let new = service.cfg.borrow().clone();
        let mut cfg = self.cfg.borrow_mut();
        for group in &mut cfg.groups {
            for s in &mut group.services {
                if s.key() == service.key {
                    *s = new.clone();
                }
            }
        }
        self.save(&cfg);
    }

    fn save(&self, cfg: &Config) {
        if let Err(err) = config::save(&self.cfg_path, cfg) {
            log::warn!("{}: {err:#}", self.cfg_path.display());
        }
    }

    fn remove(self: &Rc<Self>, service: &Rc<Service>) {
        {
            let mut cfg = self.cfg.borrow_mut();
            for group in &mut cfg.groups {
                group.services.retain(|s| s.key() != service.key);
            }
            self.save(&cfg);
        }
        self.rebuild();
    }

    /// "+": name, address, group and the two choices that matter at once.
    fn add_dialog(self: &Rc<Self>) {
        let name = gtk::Entry::builder().hexpand(true).build();
        let url = gtk::Entry::builder().placeholder_text("https://").build();
        let default_group = self.cfg.borrow().groups.first().map(|g| g.name.clone()).unwrap_or_else(|| tr("pathfinder-default-group"));
        let group = gtk::Entry::builder().text(&default_group).build();
        let always = gtk::CheckButton::with_label(&tr("pathfinder-always-active"));
        let media = gtk::CheckButton::with_label(&tr("pathfinder-media"));
        let go = gtk::Button::builder().label(tr("pathfinder-add-button")).css_classes(["suggested-action"]).halign(gtk::Align::End).build();
        let grid = gtk::Grid::builder().row_spacing(10).column_spacing(10).css_classes(["dialog-box"]).build();
        for (row, (label, widget)) in [
            ("pathfinder-name", name.upcast_ref::<gtk::Widget>()),
            ("pathfinder-url", url.upcast_ref()),
            ("pathfinder-group", group.upcast_ref()),
        ]
        .into_iter()
        .enumerate()
        {
            grid.attach(&gtk::Label::builder().label(tr(label)).xalign(1.0).build(), 0, row as i32, 1, 1);
            grid.attach(widget, 1, row as i32, 1, 1);
        }
        grid.attach(&always, 1, 3, 1, 1);
        grid.attach(&media, 1, 4, 1, 1);
        grid.attach(&go, 1, 5, 1, 1);
        let dialog = gtk::Window::builder()
            .transient_for(&self.window)
            .modal(true)
            .title(tr("pathfinder-add"))
            .resizable(false)
            .default_width(420)
            .css_classes(["orbit-pathfinder-dialog"])
            .child(&grid)
            .build();
        let run = Rc::new(glib::clone!(
            #[weak(rename_to = this)]
            self,
            #[weak]
            dialog,
            #[weak]
            name,
            #[weak]
            url,
            #[weak]
            group,
            #[weak]
            always,
            #[weak]
            media,
            move || {
                let mut address = url.text().trim().to_owned();
                if address.is_empty() {
                    url.grab_focus();
                    return;
                }
                if !address.contains("://") {
                    address = format!("https://{address}");
                }
                let mut label = name.text().trim().to_owned();
                if label.is_empty() {
                    label = crate::privacy::host(&address).unwrap_or_else(|| address.clone());
                }
                let mut service = config::Service::new(&label, &address);
                service.always_active = always.is_active();
                service.media = media.is_active();
                // Two of the same name get storage of their own.
                let taken: Vec<String> = this.services.borrow().iter().map(|s| s.key.clone()).collect();
                if taken.contains(&service.key()) {
                    let base = service.key();
                    let free = (2..).map(|n| format!("{base}-{n}")).find(|k| !taken.contains(k)).expect("free key");
                    service.id = Some(free);
                }
                let key = service.key();
                let group_name = Some(group.text().trim().to_owned()).filter(|g| !g.is_empty()).unwrap_or_else(|| tr("pathfinder-default-group"));
                {
                    let mut cfg = this.cfg.borrow_mut();
                    match cfg.groups.iter_mut().find(|g| g.name == group_name) {
                        Some(g) => g.services.push(service),
                        None => cfg.groups.push(config::Group { name: group_name, services: vec![service] }),
                    }
                    this.save(&cfg);
                }
                this.rebuild();
                let added = this.services.borrow().iter().find(|s| s.key == key).cloned();
                if let Some(s) = added {
                    this.select(&s);
                }
                dialog.close();
            }
        ));
        {
            let run = run.clone();
            go.connect_clicked(move |_| run());
        }
        for entry in [&name, &url, &group] {
            let run = run.clone();
            entry.connect_activate(move |_| run());
        }
        dialog.present();
        name.grab_focus();
    }

    fn settings_menu(self: &Rc<Self>) -> gtk::Popover {
        let popover = gtk::Popover::builder().has_arrow(false).build();
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["menu-list"]).build();
        let edit = gtk::Button::builder().label(tr("pathfinder-edit-config")).css_classes(["flat", "menu-item"]).build();
        let reload = gtk::Button::builder().label(tr("pathfinder-reload-config")).css_classes(["flat", "menu-item"]).build();
        list.append(&edit);
        list.append(&reload);
        popover.set_child(Some(&list));
        edit.connect_clicked(glib::clone!(
            #[weak(rename_to = this)]
            self,
            #[weak]
            popover,
            move |_| {
                popover.popdown();
                if !this.cfg_path.exists() {
                    this.save(&this.cfg.borrow());
                }
                let uri = gio::File::for_path(&this.cfg_path).uri();
                if let Err(err) = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>) {
                    log::warn!("{uri}: {err}");
                }
            }
        ));
        reload.connect_clicked(glib::clone!(
            #[weak(rename_to = this)]
            self,
            #[weak]
            popover,
            move |_| {
                popover.popdown();
                this.reload_config();
            }
        ));
        popover
    }

    fn reload_config(self: &Rc<Self>) {
        let text = std::fs::read_to_string(&self.cfg_path).unwrap_or_default();
        match serde_yaml_ng::from_str::<Config>(&text) {
            Ok(cfg) => {
                *self.cfg.borrow_mut() = cfg;
                self.rebuild();
            }
            Err(err) => log::warn!("{}: {err}", self.cfg_path.display()),
        }
    }

    fn install_actions(self: &Rc<Self>, app: &gtk::Application) {
        // A notification clicked: its service, in front.
        let action = gio::SimpleAction::new("service", Some(glib::VariantTy::STRING));
        action.connect_activate(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |_, param| {
                let Some(key) = param.and_then(|p| p.get::<String>()) else { return };
                let service = this.services.borrow().iter().find(|s| s.key == key).cloned();
                if let Some(s) = service {
                    this.select(&s);
                    s.notification_clicked();
                }
                this.window.present();
            }
        ));
        app.add_action(&action);
    }

    /// The shortcuts, as window actions with application accelerators:
    /// GTK runs those before the key reaches the page, which a key
    /// controller on the window does not see while a page has the focus.
    fn install_keys(self: &Rc<Self>, app: &gtk::Application) {
        let add = |name: &str, param: Option<&glib::VariantTy>, f: Shortcut| {
            let action = gio::SimpleAction::new(name, param);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, p| {
                if let Some(this) = weak.upgrade() {
                    f(&this, p);
                }
            });
            self.window.add_action(&action);
        };
        add("pick", Some(glib::VariantTy::INT32), Box::new(|this, p| {
            let n = p.and_then(|p| p.get::<i32>()).unwrap_or(1).max(1) as usize - 1;
            let s = this.services.borrow().get(n).cloned();
            if let Some(s) = s {
                this.select(&s);
            }
        }));
        add("next", None, Box::new(|this, _| this.cycle(1)));
        add("previous", None, Box::new(|this, _| this.cycle(-1)));
        add("reload", None, Box::new(|this, _| this.with_view(|v| v.reload())));
        add("reload-hard", None, Box::new(|this, _| this.with_view(|v| v.reload_bypass_cache())));
        add("back", None, Box::new(|this, _| this.with_view(|v| v.go_back())));
        add("forward", None, Box::new(|this, _| this.with_view(|v| v.go_forward())));
        add("zoom-in", None, Box::new(|this, _| this.zoom(Some(0.1))));
        add("zoom-out", None, Box::new(|this, _| this.zoom(Some(-0.1))));
        add("zoom-reset", None, Box::new(|this, _| this.zoom(None)));
        add("inspect", None, Box::new(|this, _| {
            this.with_view(|v| {
                if let Some(i) = v.inspector() {
                    i.show();
                }
            })
        }));
        for n in 1..=9 {
            app.set_accels_for_action(&format!("win.pick({n})"), &[&format!("<Control>{n}")]);
        }
        for (action, keys) in [
            ("win.next", &["<Control>Tab"][..]),
            ("win.previous", &["<Control><Shift>Tab", "<Control>ISO_Left_Tab"]),
            ("win.reload", &["<Control>r", "F5"]),
            ("win.reload-hard", &["<Control><Shift>r", "<Shift>F5"]),
            ("win.back", &["<Alt>Left"]),
            ("win.forward", &["<Alt>Right"]),
            ("win.zoom-in", &["<Control>plus", "<Control>equal", "<Control>KP_Add"]),
            ("win.zoom-out", &["<Control>minus", "<Control>KP_Subtract"]),
            ("win.zoom-reset", &["<Control>0", "<Control>KP_0"]),
            ("win.inspect", &["F12"]),
        ] {
            app.set_accels_for_action(action, keys);
        }
    }

    /// Session cookies are written when the window closes (which waits for
    /// them, a second at most) and every few minutes meanwhile.
    fn keep_logins(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(crate::cookies::SAVE_EVERY, move || {
            let Some(this) = weak.upgrade() else { return glib::ControlFlow::Break };
            this.save_cookies(|| {});
            glib::ControlFlow::Continue
        });
        let saved = Rc::new(std::cell::Cell::new(false));
        self.window.connect_close_request(glib::clone!(
            #[weak(rename_to = this)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |window| {
                if saved.get() {
                    return glib::Propagation::Proceed;
                }
                let finish = {
                    let (saved, window) = (saved.clone(), window.clone());
                    move || {
                        if !saved.replace(true) {
                            window.close();
                        }
                    }
                };
                let finish = Rc::new(finish);
                let f = finish.clone();
                this.save_cookies(move || f());
                // A cookie store that never answers must not keep the window.
                glib::timeout_add_local_once(std::time::Duration::from_secs(1), move || finish());
                glib::Propagation::Stop
            }
        ));
    }

    /// Writes every loaded service's session cookies, then calls `done`.
    fn save_cookies(&self, done: impl FnOnce() + 'static) {
        let loaded: Vec<Rc<Service>> = self.services.borrow().iter().filter(|s| s.is_loaded()).cloned().collect();
        if loaded.is_empty() {
            done();
            return;
        }
        let left = Rc::new(std::cell::Cell::new(loaded.len()));
        let done = Rc::new(std::cell::Cell::new(Some(Box::new(done) as Box<dyn FnOnce()>)));
        for service in loaded {
            let (left, done) = (left.clone(), done.clone());
            service.save_cookies(move || {
                left.set(left.get() - 1);
                if left.get() == 0
                    && let Some(done) = done.take()
                {
                    done();
                }
            });
        }
    }

    /// The next (or previous) service already loaded.
    fn cycle(self: &Rc<Self>, step: isize) {
        let services: Vec<Rc<Service>> = self.services.borrow().iter().filter(|s| s.is_loaded()).cloned().collect();
        if services.is_empty() {
            return;
        }
        let at = self
            .current
            .borrow()
            .as_ref()
            .and_then(|c| services.iter().position(|s| s.key == c.key))
            .unwrap_or(0) as isize;
        let next = (at + step).rem_euclid(services.len() as isize) as usize;
        self.select(&services[next]);
    }

    fn zoom(&self, step: Option<f64>) {
        let Some(service) = self.current.borrow().clone() else { return };
        let zoom = step.map_or(1.0, |s| ((service.zoom() + s) * 10.0).round() / 10.0);
        service.set_zoom(zoom);
        let mut state = self.state.borrow_mut();
        if (zoom - 1.0).abs() < f64::EPSILON {
            state.zoom.remove(&service.key);
        } else {
            state.zoom.insert(service.key.clone(), service.zoom());
        }
        state.save();
    }
}
