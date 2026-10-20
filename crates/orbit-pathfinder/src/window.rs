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

/// The built-in browser's start page and search, when the file names none.
const DEFAULT_HOME: &str = "https://www.google.com/";
const DEFAULT_SEARCH: &str = "https://www.google.com/search?q=";
/// The browser's key: its storage, icon and zoom.
const BROWSER_ID: &str = "orbit-browser";

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
    /// The address bar: the page's address; what is typed opens in the
    /// browser.
    address: gtk::Entry,
    /// Typing in the address bar: the page's address does not overwrite it.
    editing: std::cell::Cell<bool>,
    /// The built-in browser, at the top of the sidebar.
    browser: Rc<Service>,
    back: gtk::Button,
    forward: gtk::Button,
    current: RefCell<Option<Rc<Service>>>,
    state: RefCell<State>,
    vault: Rc<crate::passwords::Vault>,
    key_button: gtk::Button,
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
            profiles: RefCell::new(HashMap::new()),
            notify_origins: RefCell::new(HashMap::new()),
            changed: RefCell::new(None),
            page_changed: RefCell::new(None),
            watching: RefCell::new(None),
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
            .width_request(40)
            .build();
        // "+" over the gear, one under the other: the sidebar stays narrow.
        let footer = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["footer"]).build();
        footer.append(&add);
        footer.append(&menu);
        // The browser, always there above the services.
        let top = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["group-services", "browser-slot"]).build();
        sidebar.append(&top);
        sidebar.append(&gtk::Separator::builder().css_classes(["group-separator"]).build());
        sidebar.append(&scroller);
        sidebar.append(&footer);

        let button = |icon: &str, tip: &str| {
            gtk::Button::builder().icon_name(icon).tooltip_text(tr(tip)).css_classes(["flat"]).build()
        };
        let back = button("go-previous-symbolic", "pathfinder-back");
        let forward = button("go-next-symbolic", "pathfinder-forward");
        let reload = button("view-refresh-symbolic", "pathfinder-reload");
        let outside = button("send-to-symbolic", "pathfinder-open-external");
        let key_button = button("changes-prevent-symbolic", "pathfinder-fill");
        let home = button("go-home-symbolic", "pathfinder-home");
        let address = gtk::Entry::builder()
            .hexpand(true)
            .placeholder_text(tr("pathfinder-address-hint"))
            .input_purpose(gtk::InputPurpose::Url)
            .css_classes(["address"])
            .build();
        let bar = gtk::Box::builder().spacing(5).css_classes(["bar"]).build();
        for w in [home.upcast_ref::<gtk::Widget>(), back.upcast_ref(), forward.upcast_ref(), reload.upcast_ref(), address.upcast_ref(), key_button.upcast_ref(), outside.upcast_ref()] {
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

        let state = State::load();
        let browser = {
            let mut cfg_b = config::Service::new(&tr("pathfinder-browser"), cfg.home.as_deref().unwrap_or(DEFAULT_HOME));
            cfg_b.id = Some(BROWSER_ID.into());
            cfg_b.icon = Some("web-browser-symbolic".into());
            cfg_b.browser = true;
            // Any site opens here: none may notify by just asking.
            cfg_b.notifications = false;
            let zoom = state.zoom.get(BROWSER_ID).copied().unwrap_or(1.0);
            Service::new(cfg_b, &env, zoom)
        };
        top.append(&browser.button);
        let this = Rc::new(Self {
            window,
            env,
            cfg_path: config::path(dir),
            cfg: RefCell::new(cfg),
            services: RefCell::new(Vec::new()),
            groups_box,
            groups: RefCell::new(Vec::new()),
            stack,
            address,
            editing: std::cell::Cell::new(false),
            browser,
            back,
            forward,
            current: RefCell::new(None),
            state: RefCell::new(state),
            vault: Rc::new(crate::passwords::Vault::default()),
            key_button,
        });

        {
            let weak = Rc::downgrade(&this);
            *this.env.changed.borrow_mut() = Some(Box::new(move |()| {
                if let Some(w) = weak.upgrade() {
                    w.update_counts();
                }
            }));
            let weak = Rc::downgrade(&this);
            *this.env.watching.borrow_mut() = Some(Box::new(move |service| {
                weak.upgrade().is_some_and(|w| {
                    w.window.is_active() && w.window.is_visible() && w.current.borrow().as_ref().is_some_and(|c| c.key == service.key)
                })
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
        this.wire(&this.browser.clone());
        home.connect_clicked(glib::clone!(#[weak] this, move |_| this.go_home()));
        this.address.connect_activate(glib::clone!(#[weak] this, move |e| this.go_to(&e.text())));
        {
            // Editing: the page's address waits; leaving puts it back.
            let focus = gtk::EventControllerFocus::new();
            focus.connect_enter(glib::clone!(#[weak] this, move |_| this.editing.set(true)));
            focus.connect_leave(glib::clone!(#[weak] this, move |_| {
                this.editing.set(false);
                this.update_bar();
            }));
            this.address.add_controller(focus);
            let keys = gtk::EventControllerKey::new();
            keys.connect_key_pressed(glib::clone!(
                #[weak]
                this,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, key, _, _| {
                    if key != gdk::Key::Escape {
                        return glib::Propagation::Proceed;
                    }
                    this.editing.set(false);
                    this.update_bar();
                    this.with_view(|v| {
                        v.grab_focus();
                    });
                    glib::Propagation::Stop
                }
            ));
            this.address.add_controller(keys);
        }
        this.back.connect_clicked(glib::clone!(#[weak] this, move |_| this.with_view(|v| v.go_back())));
        this.forward.connect_clicked(glib::clone!(#[weak] this, move |_| this.with_view(|v| v.go_forward())));
        reload.connect_clicked(glib::clone!(#[weak] this, move |_| this.with_view(|v| v.reload())));
        outside.connect_clicked(glib::clone!(#[weak] this, move |_| {
            if let Some(uri) = this.current_view().and_then(|v| v.uri()) {
                this.env.open_external(&uri);
            }
        }));
        add.connect_clicked(glib::clone!(#[weak] this, move |b| this.add_menu(b.upcast_ref())));
        this.key_button.connect_clicked(glib::clone!(#[weak] this, move |_| this.fill_login()));
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
        // The browser first (Ctrl+1), whatever the file holds.
        kept.insert(self.browser.key.clone(), self.browser.clone());
        self.services.borrow_mut().push(self.browser.clone());
        // Who may notify, by profile — read when a profile's first page
        // starts.
        {
            let mut origins = self.env.notify_origins.borrow_mut();
            origins.clear();
            for s in cfg.groups.iter().flat_map(|g| &g.services).filter(|s| s.notifications) {
                origins.entry(s.profile()).or_default().push(s.url.clone());
            }
        }
        let mut seps = 0;
        for (index, group) in cfg.groups.iter().enumerate() {
            let collapsed = group.name.as_ref().is_some_and(|n| self.state.borrow().collapsed.contains(n));
            let badge = gtk::Label::builder().css_classes(["badge", "group-badge"]).visible(false).build();
            let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["group-services"]).build();
            let revealer = gtk::Revealer::builder()
                .child(&list)
                .reveal_child(!collapsed)
                .transition_type(gtk::RevealerTransitionType::SlideDown)
                .build();
            // The same count `flatten` makes: a slot per heading or line.
            let separator = |widget: &gtk::Widget, this: &Rc<Self>, n: usize| {
                this.draggable(widget, format!("sep:{n}"));
                this.separator_menu(widget, n);
            };
            match &group.name {
                // A named group: a heading that folds it.
                Some(name) => {
                    let header = self.group_header(name, collapsed, &badge, &revealer);
                    separator(header.upcast_ref(), self, seps);
                    seps += 1;
                    self.groups_box.append(&header);
                }
                // Otherwise just a line between it and the one above, in a
                // slot tall enough to grab.
                None if index > 0 => {
                    let line = gtk::Separator::builder().css_classes(["group-separator"]).hexpand(true).valign(gtk::Align::Center).build();
                    let slot = gtk::Box::builder().css_classes(["separator-slot"]).build();
                    slot.append(&line);
                    separator(slot.upcast_ref(), self, seps);
                    seps += 1;
                    self.groups_box.append(&slot);
                }
                None => {}
            }
            let mut members = Vec::new();
            for cfg in &group.services {
                let key = cfg.key();
                let service = match old.get(&key) {
                    Some(s) => {
                        let icon_changed = s.cfg.borrow().icon != cfg.icon;
                        *s.cfg.borrow_mut() = cfg.clone();
                        s.button.set_tooltip_text(Some(&cfg.name));
                        if icon_changed {
                            // A cleared or new icon: the copy kept for the old one goes.
                            let _ = std::fs::remove_file(self.env.icon_file(&key));
                            s.apply_icon();
                        }
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

    /// The heading of a named group: arrow, name and — folded — the sum of
    /// its unread counts. A click folds or unfolds it.
    fn group_header(self: &Rc<Self>, group: &str, collapsed: bool, badge: &gtk::Label, revealer: &gtk::Revealer) -> gtk::Button {
        let name = gtk::Label::builder()
            .label(group)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(8)
            .hexpand(true)
            .xalign(0.0)
            .build();
        let arrow = gtk::Image::from_icon_name(if collapsed { "pan-end-symbolic" } else { "pan-down-symbolic" });
        let head = gtk::Box::builder().spacing(2).build();
        head.append(&arrow);
        head.append(&name);
        head.append(badge);
        let header = gtk::Button::builder()
            .child(&head)
            .tooltip_text(group)
            .css_classes(["flat", "group-header"])
            .build();
        let group = group.to_owned();
        header.connect_clicked(glib::clone!(
            #[weak(rename_to = this)]
            self,
            #[weak]
            revealer,
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
        header
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
        if service.cfg.borrow().browser {
            // Fixed in place, nothing to edit or remove.
            return;
        }
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
        self.draggable(service.button.upcast_ref(), format!("svc:{}", service.key));
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
            if !self.editing.get() {
                self.address.set_text("");
            }
            return;
        };
        if !self.editing.get() {
            self.address.set_text(view.uri().as_deref().unwrap_or_default());
        }
        self.address.set_tooltip_text(view.title().as_deref());
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
        let (key, this) = (service.key.clone(), self.clone());
        item(tr("pathfinder-edit"), Box::new(move || this.service_dialog(Some(key.clone()))));
        let (key, this) = (service.key.clone(), self.clone());
        item(tr("pathfinder-move-up"), Box::new(move || this.move_service(&key, true)));
        let (key, this) = (service.key.clone(), self.clone());
        item(tr("pathfinder-move-down"), Box::new(move || this.move_service(&key, false)));
        let (key, this) = (service.key.clone(), self.clone());
        item(tr("pathfinder-separator-below"), Box::new(move || this.edit_slots(|slots| {
            if let Some(i) = slots.iter().position(|s| matches!(s, Slot::Service(c) if c.key() == key)) {
                slots.insert(i + 1, Slot::Separator(None));
            }
        })));
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
                let sharing: Vec<String> =
                    this.services.borrow().iter().filter(|o| o.profile() == s.profile() && o.key != s.key).map(|o| o.name()).collect();
                let message = if sharing.is_empty() {
                    tr_args("pathfinder-clear-confirm", &[("name", text(s.name()))])
                } else {
                    tr_args("pathfinder-clear-confirm-shared", &[("name", text(s.name())), ("others", text(sharing.join(", ")))])
                };
                this.confirm(&message, &tr("pathfinder-clear"), move || s.clear_data());
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

    /// "+" (`editing` = None) or "Edit…" on a service: name, address, group,
    /// a profile to share a login with, and the two choices that matter at
    /// once.
    fn service_dialog(self: &Rc<Self>, editing: Option<String>) {
        let old: Option<(usize, config::Service)> = editing.as_ref().and_then(|key| {
            self.cfg.borrow().groups.iter().enumerate().find_map(|(gi, g)| g.services.iter().find(|s| s.key() == *key).map(|s| (gi, s.clone())))
        });
        let name = gtk::Entry::builder().hexpand(true).build();
        let url = gtk::Entry::builder().placeholder_text("https://").build();
        // Groups by name, or by their first service when unnamed; then "new".
        let mut choices: Vec<String> = self
            .cfg
            .borrow()
            .groups
            .iter()
            .enumerate()
            .map(|(i, g)| {
                g.name.clone().unwrap_or_else(|| match g.services.first() {
                    Some(first) => tr_args("pathfinder-group-n", &[("n", text((i + 1).to_string())), ("first", text(first.name.clone()))]),
                    None => tr_args("pathfinder-group-empty", &[("n", text((i + 1).to_string()))]),
                })
            })
            .collect();
        let existing = choices.len() as u32;
        choices.push(tr("pathfinder-new-group"));
        let group = gtk::DropDown::from_strings(&choices.iter().map(String::as_str).collect::<Vec<_>>());
        let profile = gtk::Entry::builder().placeholder_text(tr("pathfinder-profile-hint")).build();
        let icon = gtk::Entry::builder().placeholder_text(tr("pathfinder-icon-hint")).build();
        let always = gtk::CheckButton::with_label(&tr("pathfinder-always-active"));
        let media = gtk::CheckButton::with_label(&tr("pathfinder-media"));
        if let Some((gi, s)) = &old {
            name.set_text(&s.name);
            url.set_text(&s.url);
            group.set_selected(*gi as u32);
            profile.set_text(s.profile.as_deref().unwrap_or_default());
            icon.set_text(s.icon.as_deref().unwrap_or_default());
            always.set_active(s.always_active);
            media.set_active(s.media);
        }
        let button = if old.is_some() { "pathfinder-save" } else { "pathfinder-add-button" };
        let go = gtk::Button::builder().label(tr(button)).css_classes(["suggested-action"]).halign(gtk::Align::End).build();
        let grid = gtk::Grid::builder().row_spacing(10).column_spacing(10).css_classes(["dialog-box"]).build();
        for (row, (label, widget)) in [
            ("pathfinder-name", name.upcast_ref::<gtk::Widget>()),
            ("pathfinder-url", url.upcast_ref()),
            ("pathfinder-group", group.upcast_ref()),
            ("pathfinder-profile", profile.upcast_ref()),
            ("pathfinder-icon", icon.upcast_ref()),
        ]
        .into_iter()
        .enumerate()
        {
            grid.attach(&gtk::Label::builder().label(tr(label)).xalign(1.0).build(), 0, row as i32, 1, 1);
            grid.attach(widget, 1, row as i32, 1, 1);
        }
        grid.attach(&always, 1, 5, 1, 1);
        grid.attach(&media, 1, 6, 1, 1);
        grid.attach(&go, 1, 7, 1, 1);
        let title = if old.is_some() { "pathfinder-edit" } else { "pathfinder-add" };
        let dialog = gtk::Window::builder()
            .transient_for(&self.window)
            .modal(true)
            .title(tr(title))
            .resizable(false)
            .default_width(420)
            .css_classes(["orbit-pathfinder-dialog"])
            .child(&grid)
            .build();
        crate::ui::close_on_escape(dialog.upcast_ref());
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
            profile,
            #[weak]
            icon,
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
                // Editing keeps what the dialog does not show (domains, proxy…).
                let mut service = old.as_ref().map(|(_, s)| s.clone()).unwrap_or_else(|| config::Service::new(&label, &address));
                let old_key = old.as_ref().map(|(_, s)| s.key());
                let old_profile = old.as_ref().map(|(_, s)| s.profile());
                service.name = label;
                service.url = address;
                service.always_active = always.is_active();
                service.media = media.is_active();
                service.profile = Some(profile.text().trim().to_owned()).filter(|p| !p.is_empty());
                service.icon = Some(icon.text().trim().to_owned()).filter(|i| !i.is_empty());
                match &old_key {
                    // A new name must not move the service to new storage
                    // (its login) or lose its icon and zoom: it keeps its key.
                    Some(key) if service.key() != *key => service.id = Some(key.clone()),
                    Some(_) => {}
                    // Two of the same name get storage of their own.
                    None => {
                        let taken: Vec<String> = this.services.borrow().iter().map(|s| s.key.clone()).collect();
                        if taken.contains(&service.key()) {
                            let base = service.key();
                            let free = (2..).map(|n| format!("{base}-{n}")).find(|k| !taken.contains(k)).expect("free key");
                            service.id = Some(free);
                        }
                    }
                }
                let key = service.key();
                let new_url = service.url.clone();
                let profile_changed = old_profile.is_some_and(|p| p != service.profile());
                {
                    let mut cfg = this.cfg.borrow_mut();
                    let chosen = group.selected();
                    let same_group = old.as_ref().is_some_and(|(gi, _)| *gi as u32 == chosen);
                    let replaced = same_group
                        && cfg.groups.iter_mut().flat_map(|g| g.services.iter_mut()).find(|s| Some(s.key()) == old_key).map(|s| *s = service.clone()).is_some();
                    if !replaced {
                        if let Some(k) = &old_key {
                            for g in &mut cfg.groups {
                                g.services.retain(|s| s.key() != *k);
                            }
                        }
                        match cfg.groups.get_mut(chosen as usize).filter(|_| chosen < existing) {
                            Some(g) => g.services.push(service),
                            None => cfg.groups.push(config::Group { name: None, services: vec![service] }),
                        }
                    }
                    this.save(&cfg);
                }
                // A loaded page in another profile's storage is made again.
                if profile_changed {
                    let loaded = this.services.borrow().iter().find(|s| s.key == key).cloned();
                    if let Some(s) = loaded {
                        if let Some(view) = s.view() {
                            this.stack.remove(&view);
                        }
                        s.drop_view();
                    }
                }
                this.rebuild();
                let service = this.services.borrow().iter().find(|s| s.key == key).cloned();
                if let Some(s) = service {
                    if let Some(view) = s.view()
                        && old_key.is_some()
                        && view.uri().is_none_or(|u| crate::privacy::host(&u) != crate::privacy::host(&new_url))
                    {
                        view.load_uri(&new_url);
                    }
                    this.select(&s);
                }
                dialog.close();
            }
        ));
        {
            let run = run.clone();
            go.connect_clicked(move |_| run());
        }
        for entry in [&name, &url, &profile, &icon] {
            let run = run.clone();
            entry.connect_activate(move |_| run());
        }
        dialog.present();
        name.grab_focus();
    }

    /// "+": a service, or a separator (at the end of the sidebar, to drag
    /// where it goes).
    fn add_menu(self: &Rc<Self>, anchor: &gtk::Widget) {
        let popover = gtk::Popover::builder().has_arrow(false).position(gtk::PositionType::Right).build();
        popover.set_parent(anchor);
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["menu-list"]).build();
        for (label, separator) in [("pathfinder-add-service", false), ("pathfinder-add-separator", true)] {
            let item = gtk::Button::builder().label(tr(label)).css_classes(["flat", "menu-item"]).build();
            if let Some(l) = item.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            let (this, p) = (self.clone(), popover.clone());
            item.connect_clicked(move |_| {
                p.popdown();
                if separator {
                    let this = this.clone();
                    glib::idle_add_local_once(move || this.edit_slots(|slots| slots.push(Slot::Separator(None))));
                } else {
                    this.service_dialog(None);
                }
            });
            list.append(&item);
        }
        popover.set_child(Some(&list));
        popover.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        popover.popup();
    }

    /// Lets `widget` be dragged (as `id`, "svc:<key>" or "sep:<n>") and
    /// take drops: what lands on its upper half goes before it, on its
    /// lower half after it.
    fn draggable(self: &Rc<Self>, widget: &gtk::Widget, id: String) {
        let source = gtk::DragSource::builder().actions(gdk::DragAction::MOVE).build();
        let value = id.clone();
        source.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&value.to_value())));
        source.connect_drag_begin(|source, _| {
            if let Some(widget) = source.widget() {
                let paintable = gtk::WidgetPaintable::new(Some(&widget));
                source.set_icon(Some(&paintable), widget.width() / 2, widget.height() / 2);
            }
        });
        widget.add_controller(source);

        let target = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
        let mark = |w: &gtk::Widget, y: f64| {
            let after = y > w.height() as f64 / 2.0;
            w.remove_css_class(if after { "drop-before" } else { "drop-after" });
            w.add_css_class(if after { "drop-after" } else { "drop-before" });
        };
        target.connect_enter(move |t, _, y| {
            if let Some(w) = t.widget() {
                mark(&w, y);
            }
            gdk::DragAction::MOVE
        });
        target.connect_motion(move |t, _, y| {
            if let Some(w) = t.widget() {
                mark(&w, y);
            }
            gdk::DragAction::MOVE
        });
        target.connect_leave(|t| {
            if let Some(w) = t.widget() {
                w.remove_css_class("drop-before");
                w.remove_css_class("drop-after");
            }
        });
        let weak = Rc::downgrade(self);
        target.connect_drop(move |t, value, _, y| {
            let Some(w) = t.widget() else { return false };
            w.remove_css_class("drop-before");
            w.remove_css_class("drop-after");
            let (Some(this), Ok(from)) = (weak.upgrade(), value.get::<String>()) else { return false };
            if from == id {
                return false;
            }
            let after = y > w.height() as f64 / 2.0;
            let (from, to) = (from.clone(), id.clone());
            // After the drop finishes: the rebuild removes the widgets.
            glib::idle_add_local_once(move || {
                this.edit_slots(|slots| {
                    let find = |slots: &[Slot], id: &str| slots.iter().position(|s| s.id(slots) == id);
                    let Some(i) = find(slots, &from) else { return };
                    let ids: Vec<String> = slots.iter().map(|s| s.id(slots)).collect();
                    let moving = slots.remove(i);
                    // The target, found by the id it had before the move.
                    let Some(j) = ids.iter().position(|x| *x == to) else {
                        slots.insert(i, moving);
                        return;
                    };
                    let j = if j > i { j - 1 } else { j };
                    slots.insert(if after { j + 1 } else { j }, moving);
                })
            });
            true
        });
        widget.add_controller(target);
    }

    /// A right click on a separator: remove it (its group joins the one
    /// above).
    fn separator_menu(self: &Rc<Self>, widget: &gtk::Widget, n: usize) {
        let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let weak = Rc::downgrade(self);
        let anchor = widget.clone();
        click.connect_pressed(move |_, _, _, _| {
            let Some(this) = weak.upgrade() else { return };
            let popover = gtk::Popover::builder().has_arrow(false).position(gtk::PositionType::Right).build();
            popover.set_parent(&anchor);
            let remove = gtk::Button::builder().label(tr("pathfinder-separator-remove")).css_classes(["flat", "menu-item"]).build();
            let p = popover.clone();
            remove.connect_clicked(move |_| {
                p.popdown();
                let this = this.clone();
                glib::idle_add_local_once(move || {
                    this.edit_slots(|slots| {
                        let mut seen = 0;
                        slots.retain(|s| match s {
                            Slot::Separator(_) => {
                                seen += 1;
                                seen - 1 != n
                            }
                            Slot::Service(_) => true,
                        });
                    })
                });
            });
            popover.set_child(Some(&remove));
            popover.connect_closed(|p| {
                let p = p.clone();
                glib::idle_add_local_once(move || p.unparent());
            });
            popover.popup();
        });
        widget.add_controller(click);
    }

    /// Changes the sidebar as a flat list of services and separators, then
    /// saves it as groups again and rebuilds.
    fn edit_slots(self: &Rc<Self>, f: impl FnOnce(&mut Vec<Slot>)) {
        {
            let mut cfg = self.cfg.borrow_mut();
            let mut slots = flatten(&cfg.groups);
            f(&mut slots);
            cfg.groups = regroup(slots);
            self.save(&cfg);
        }
        self.rebuild();
    }

    /// Moves a service one place up or down; past the edge of its group, into
    /// the next group.
    fn move_service(self: &Rc<Self>, key: &str, up: bool) {
        {
            let mut cfg = self.cfg.borrow_mut();
            let Some((gi, si)) = cfg.groups.iter().enumerate().find_map(|(gi, g)| g.services.iter().position(|s| s.key() == key).map(|si| (gi, si))) else { return };
            let len = cfg.groups[gi].services.len();
            match (up, si) {
                (true, 0) if gi > 0 => {
                    let s = cfg.groups[gi].services.remove(0);
                    cfg.groups[gi - 1].services.push(s);
                }
                (true, 0) => return,
                (true, _) => cfg.groups[gi].services.swap(si, si - 1),
                (false, _) if si + 1 < len => cfg.groups[gi].services.swap(si, si + 1),
                (false, _) if gi + 1 < cfg.groups.len() => {
                    let s = cfg.groups[gi].services.remove(si);
                    cfg.groups[gi + 1].services.insert(0, s);
                }
                (false, _) => return,
            }
            self.save(&cfg);
        }
        self.rebuild();
    }

    fn settings_menu(self: &Rc<Self>) -> gtk::Popover {
        let popover = gtk::Popover::builder().has_arrow(false).build();
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["menu-list"]).build();
        let edit = gtk::Button::builder().label(tr("pathfinder-edit-config")).css_classes(["flat", "menu-item"]).build();
        let reload = gtk::Button::builder().label(tr("pathfinder-reload-config")).css_classes(["flat", "menu-item"]).build();
        let import = gtk::Button::builder().label(tr("pathfinder-import-title")).css_classes(["flat", "menu-item"]).build();
        for item in [&edit, &reload, &import] {
            if let Some(label) = item.child().and_downcast::<gtk::Label>() {
                label.set_xalign(0.0);
            }
            list.append(item);
        }
        import.connect_clicked(glib::clone!(
            #[weak(rename_to = this)]
            self,
            #[weak]
            popover,
            move |_| {
                popover.popdown();
                this.vault.import_dialog(this.window.upcast_ref());
            }
        ));
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
        add("fill", None, Box::new(|this, _| this.fill_login()));
        add("address", None, Box::new(|this, _| {
            this.address.grab_focus();
            this.address.select_region(0, -1);
        }));
        add("home", None, Box::new(|this, _| this.go_home()));
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
            ("win.fill", &["<Control><Shift>l"]),
            ("win.address", &["<Control>l", "F6"]),
            ("win.home", &["<Alt>Home"]),
        ] {
            app.set_accels_for_action(action, keys);
        }
    }

    /// Opens what was typed in the address bar — an address, or a search —
    /// in the browser, which comes forward.
    fn go_to(self: &Rc<Self>, text: &str) {
        let search = self.cfg.borrow().search.clone().unwrap_or_else(|| DEFAULT_SEARCH.into());
        let url = crate::privacy::typed(text, &search);
        if url.is_empty() {
            return;
        }
        self.editing.set(false);
        let browser = self.browser.clone();
        browser.open(&url);
        self.select(&browser);
    }

    /// Home: the browser, on its start page.
    fn go_home(self: &Rc<Self>) {
        let home = self.cfg.borrow().home.clone().unwrap_or_else(|| DEFAULT_HOME.into());
        let browser = self.browser.clone();
        browser.open(&home);
        self.select(&browser);
    }

    /// Fills the page's login form from Bitwarden.
    fn fill_login(&self) {
        if let Some(view) = self.current_view() {
            self.vault.fill(self.window.upcast_ref(), &view, self.key_button.upcast_ref());
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
        // One per profile: services sharing a profile share its cookies.
        let mut seen = std::collections::HashSet::new();
        let loaded: Vec<Rc<Service>> =
            self.services.borrow().iter().filter(|s| s.is_loaded() && seen.insert(s.profile())).cloned().collect();
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

/// The sidebar as a flat list: a separator where each group but the first
/// begins (and before a named first group), then its services.
#[derive(Debug, Clone)]
enum Slot {
    /// The group's name, when it has one.
    Separator(Option<String>),
    Service(config::Service),
}

impl Slot {
    /// "svc:<key>", or "sep:<n>" for the n-th separator in `slots`.
    fn id(&self, slots: &[Slot]) -> String {
        match self {
            Slot::Service(s) => format!("svc:{}", s.key()),
            Slot::Separator(_) => {
                let n = slots.iter().take_while(|s| !std::ptr::eq(*s, self)).filter(|s| matches!(s, Slot::Separator(_))).count();
                format!("sep:{n}")
            }
        }
    }
}

fn flatten(groups: &[config::Group]) -> Vec<Slot> {
    let mut slots = Vec::new();
    for (i, g) in groups.iter().enumerate() {
        if i > 0 || g.name.is_some() {
            slots.push(Slot::Separator(g.name.clone()));
        }
        slots.extend(g.services.iter().cloned().map(Slot::Service));
    }
    slots
}

/// Back to groups. A separator stays even with nothing after it yet (one
/// just added, to drag services under); only "Remove separator" takes it
/// away.
fn regroup(slots: Vec<Slot>) -> Vec<config::Group> {
    let mut groups: Vec<config::Group> = Vec::new();
    let mut current = config::Group::default();
    for slot in slots {
        match slot {
            Slot::Separator(name) => {
                let previous = std::mem::replace(&mut current, config::Group { name, services: Vec::new() });
                // Before a first separator there may be no group at all.
                if !(groups.is_empty() && previous.services.is_empty() && previous.name.is_none()) {
                    groups.push(previous);
                }
            }
            Slot::Service(s) => current.services.push(s),
        }
    }
    groups.push(current);
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(name: &str) -> config::Service {
        config::Service::new(name, "https://a.example/")
    }

    fn names(groups: &[config::Group]) -> Vec<Vec<String>> {
        groups.iter().map(|g| g.services.iter().map(|s| s.name.clone()).collect()).collect()
    }

    #[test]
    fn flattens_and_regroups() {
        let groups = vec![
            config::Group { name: None, services: vec![svc("A"), svc("B")] },
            config::Group { name: None, services: vec![svc("C")] },
        ];
        let mut slots = flatten(&groups);
        assert_eq!(slots.iter().map(|s| s.id(&slots)).collect::<Vec<_>>(), ["svc:a", "svc:b", "sep:0", "svc:c"]);
        // Moving the separator up one: B joins C's group.
        let sep = slots.remove(2);
        slots.insert(1, sep);
        assert_eq!(names(&regroup(slots.clone())), [vec!["A"], vec!["B", "C"]]);
        // A separator added at the end stays, with nothing under it yet.
        slots.push(Slot::Separator(None));
        let groups = regroup(slots);
        assert_eq!(names(&groups), [vec!["A"], vec!["B", "C"], vec![]]);
        // And flattens back to the same slots.
        assert_eq!(flatten(&groups).len(), 5);
    }
}
