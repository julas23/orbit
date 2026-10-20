//! One service of the sidebar: its button, and the page it shows — a web
//! view with storage of its own (cookies, logins, cache), so no service
//! sees another's.
//!
//! The view is made on the first click, or at start for a service kept
//! always active. From the page come the unread count (read from its
//! title), its icon (the site's favicon), notifications (handed to the
//! desktop's daemon) and downloads (into the Downloads folder). New
//! windows stay inside when they belong to the service — its own site, a
//! login page — and go to the external browser otherwise; tracking
//! parameters come off every link either way.

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::{Rc, Weak},
};

use gtk::{gdk, gio, glib, prelude::*};
use orbit_app::i18n::{number, text, tr_args};
use webkit6::prelude::*;

use crate::{config, cookies, privacy};

type Hook<T> = RefCell<Option<Box<dyn Fn(T)>>>;
type Waiting = Rc<RefCell<Option<Vec<Box<dyn FnOnce()>>>>>;

/// The storage one or more services share: one network session (cookies,
/// cache, logins on disk) and one web context.
pub struct Profile {
    pub session: webkit6::NetworkSession,
    context: webkit6::WebContext,
    /// Pages waiting for the session cookies to be put back; `None` once
    /// they are.
    waiting: Waiting,
}

impl Profile {
    fn when_ready(&self, f: impl FnOnce() + 'static) {
        match self.waiting.borrow_mut().as_mut() {
            Some(list) => list.push(Box::new(f)),
            None => f(),
        }
    }
}
type PageHook = Box<dyn Fn(&Service)>;
type Watching = Box<dyn Fn(&Service) -> bool>;

/// What every service shares: where storage goes, and the hooks back into
/// the window.
pub struct Env {
    pub app: gtk::Application,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub user_agent: Option<String>,
    pub external_browser: Option<String>,
    /// The profiles made so far, by name.
    pub profiles: RefCell<std::collections::HashMap<String, Rc<Profile>>>,
    /// The sites allowed to notify, by profile (the window fills it in from
    /// the config).
    pub notify_origins: RefCell<std::collections::HashMap<String, Vec<String>>>,
    /// A count changed: the window adds them up again.
    pub changed: Hook<()>,
    /// The page shown changed its title or address: the toolbar follows.
    pub page_changed: RefCell<Option<PageHook>>,
    /// Whether the service is what the user is looking at (the window
    /// active, the service shown): then a new unread count tells nothing.
    pub watching: RefCell<Option<Watching>>,
}

impl Env {
    /// A profile's folder.
    pub fn profile_dir(&self, profile: &str) -> PathBuf {
        self.data.join("services").join(profile)
    }

    /// Where a service's icon is kept for the next start.
    pub fn icon_file(&self, key: &str) -> PathBuf {
        self.data.join("icons").join(format!("{key}.png"))
    }

    /// The profile of `cfg`, made the first time.
    fn profile(self: &Rc<Self>, cfg: &config::Service) -> Rc<Profile> {
        let name = cfg.profile();
        if let Some(p) = self.profiles.borrow().get(&name) {
            return p.clone();
        }
        let data = self.profile_dir(&name);
        let cache = self.cache.join("services").join(&name);
        // WebKit sizes its storage quota from the disk the folder is on; a
        // folder not there yet reads as a disk of no size.
        for dir in [&data, &cache] {
            if let Err(err) = std::fs::create_dir_all(dir) {
                log::warn!("{}: {err}", dir.display());
            }
        }
        // Logins live here: the owner's alone.
        for dir in [&self.data, &data] {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        let session = webkit6::NetworkSession::new(data.to_str(), cache.to_str());
        // Privacy, the same for every profile (docs/PLANO-PATHFINDER.md §3).
        session.set_itp_enabled(true);
        session.set_tls_errors_policy(webkit6::TLSErrorsPolicy::Fail);
        if let Some(manager) = session.website_data_manager() {
            manager.set_favicons_enabled(true);
        }
        if let Some(proxy) = &cfg.proxy {
            let settings = webkit6::NetworkProxySettings::new(Some(proxy), &[]);
            session.set_proxy_settings(webkit6::NetworkProxyMode::Custom, Some(&settings));
        }
        session.connect_download_started(glib::clone!(
            #[weak(rename_to = env)]
            self,
            move |_, download| watch_download(&env, download)
        ));

        // A context of its own: its own web processes, and the notification
        // permission granted from the start (WhatsApp and Teams ask the page,
        // not the user, whether they may notify).
        let context = webkit6::WebContext::new();
        context.connect_initialize_notification_permissions(glib::clone!(
            #[weak(rename_to = env)]
            self,
            #[strong]
            name,
            move |context| {
                let urls = env.notify_origins.borrow().get(&name).cloned().unwrap_or_default();
                let origins: Vec<webkit6::SecurityOrigin> = urls.iter().map(|u| webkit6::SecurityOrigin::for_uri(u)).collect();
                let refs: Vec<&webkit6::SecurityOrigin> = origins.iter().collect();
                context.initialize_notification_permissions(&refs, &[]);
            }
        ));

        let waiting: Waiting = Rc::new(RefCell::new(Some(Vec::new())));
        let profile = Rc::new(Profile { session: session.clone(), context, waiting: waiting.clone() });
        self.profiles.borrow_mut().insert(name, profile.clone());
        let ready = move || {
            let list = waiting.borrow_mut().take().unwrap_or_default();
            for f in list {
                f();
            }
        };
        match session.cookie_manager() {
            Some(manager) => {
                manager.set_accept_policy(webkit6::CookieAcceptPolicy::NoThirdParty);
                cookies::persist(&manager, &data);
                // The session cookies of the last run go back first.
                cookies::restore(&manager, &data, ready);
            }
            None => ready(),
        }
        profile
    }

    /// Opens `url` in the external browser — the configured one, or the
    /// system's default.
    pub fn open_external(&self, url: &str) {
        let url = privacy::clean(url);
        if let Some(command) = &self.external_browser {
            let quoted = glib::shell_quote(&url).to_string_lossy().into_owned();
            let line = if command.contains("%u") { command.replace("%u", &quoted) } else { format!("{command} {quoted}") };
            match glib::shell_parse_argv(&line) {
                Ok(argv) => {
                    let args: Vec<&std::ffi::OsStr> = argv.iter().map(|a| a.as_os_str()).collect();
                    if let Err(err) = gio::Subprocess::newv(&args, gio::SubprocessFlags::NONE) {
                        log::warn!("{command}: {err}");
                    }
                }
                Err(err) => log::warn!("external_browser: {err}"),
            }
            return;
        }
        if let Err(err) = gio::AppInfo::launch_default_for_uri(&url, None::<&gio::AppLaunchContext>) {
            log::warn!("opening {url}: {err}");
        }
    }
}

/// The size of a service's icon on its button.
const ICON: i32 = 22;


pub struct Service {
    pub cfg: RefCell<config::Service>,
    pub key: String,
    pub button: gtk::Button,
    image: gtk::Image,
    badge: gtk::Label,
    view: RefCell<Option<webkit6::WebView>>,
    unread: Cell<u32>,
    /// The page's last notification: clicking the desktop's tells the page,
    /// which then opens the conversation.
    notification: RefCell<Option<webkit6::Notification>>,
    zoom: Cell<f64>,
    env: Weak<Env>,
    /// A count was read since the page was made (the first is not news).
    counted: Cell<bool>,
    /// When the page last sent a notification of its own.
    last_page_notification: Cell<Option<std::time::Instant>>,
    /// Where the page goes when it is made, instead of the service's
    /// address (an address typed before the browser had a page).
    pending: RefCell<Option<String>>,
    /// A better icon than the favicon is in place (the page's touch icon,
    /// or the one the config names): favicons no longer replace it.
    good_icon: Cell<bool>,
}

impl Service {
    pub fn new(cfg: config::Service, env: &Rc<Env>, zoom: f64) -> Rc<Self> {
        let image = gtk::Image::builder().pixel_size(ICON).css_classes(["service-icon"]).build();
        let badge = gtk::Label::builder()
            .css_classes(["badge"])
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .visible(false)
            .build();
        let overlay = gtk::Overlay::builder().child(&image).build();
        overlay.add_overlay(&badge);
        let button = gtk::Button::builder()
            .child(&overlay)
            .tooltip_text(&cfg.name)
            .css_classes(["service", "flat"])
            .build();
        let service = Rc::new(Self {
            key: cfg.key(),
            cfg: RefCell::new(cfg),
            button,
            image,
            badge,
            view: RefCell::new(None),
            unread: Cell::new(0),
            notification: RefCell::new(None),
            zoom: Cell::new(zoom),
            env: Rc::downgrade(env),
            good_icon: Cell::new(false),
            pending: RefCell::new(None),
            counted: Cell::new(false),
            last_page_notification: Cell::new(None),
        });
        service.apply_icon();
        service
    }

    /// The name of its storage (see `config::Service::profile`).
    pub fn profile(&self) -> String {
        self.cfg.borrow().profile()
    }

    pub fn name(&self) -> String {
        self.cfg.borrow().name.clone()
    }

    pub fn unread(&self) -> u32 {
        self.unread.get()
    }

    pub fn view(&self) -> Option<webkit6::WebView> {
        self.view.borrow().clone()
    }

    pub fn is_loaded(&self) -> bool {
        self.view.borrow().is_some()
    }

    pub fn zoom(&self) -> f64 {
        self.zoom.get()
    }

    pub fn set_zoom(&self, zoom: f64) {
        let zoom = zoom.clamp(0.3, 3.0);
        self.zoom.set(zoom);
        if let Some(view) = self.view() {
            view.set_zoom_level(zoom);
        }
    }

    /// Puts on the button the icon the config names — an address (the copy
    /// kept from the last download until the new one arrives), a file or a
    /// theme icon — or, with none, the site's icon from the last run (a
    /// placeholder the first time).
    pub fn apply_icon(self: &Rc<Self>) {
        let Some(env) = self.env.upgrade() else { return };
        let icon = self.cfg.borrow().icon.clone();
        let cached = Some(env.icon_file(&self.key)).filter(|p| p.is_file());
        self.image.remove_css_class("placeholder");
        self.good_icon.set(false);
        let placeholder = || {
            self.image.add_css_class("placeholder");
            self.image.set_icon_name(Some("web-browser-symbolic"));
        };
        match icon {
            Some(url) if url.starts_with("https://") || url.starts_with("http://") => {
                match cached {
                    Some(file) => self.image.set_from_gicon(&gio::FileIcon::new(&gio::File::for_path(file))),
                    None => placeholder(),
                }
                self.fetch_icon(url);
            }
            Some(path) if path.contains('/') => {
                let file = gio::File::for_path(orbit_common::config::expand_home(std::path::Path::new(&path)));
                self.image.set_from_gicon(&gio::FileIcon::new(&file));
            }
            Some(name) => self.image.set_icon_name(Some(&name)),
            None => match cached {
                Some(file) => self.image.set_from_gicon(&gio::FileIcon::new(&gio::File::for_path(file))),
                None => placeholder(),
            },
        }
    }

    /// Opens `url` in the service's page — straight away when the page
    /// exists, as its first address when it is made.
    pub fn open(&self, url: &str) {
        match self.view() {
            Some(view) => view.load_uri(url),
            None => *self.pending.borrow_mut() = Some(url.to_owned()),
        }
    }

    /// Downloads the icon at `url` (the config's, or the page's touch icon)
    /// and puts it on the button, keeping a copy for the next start.
    fn fetch_icon(self: &Rc<Self>, url: String) {
        let weak = Rc::downgrade(self);
        crate::ui::background(
            move || fetch(&url),
            move |bytes| {
                let Some(service) = weak.upgrade() else { return };
                match bytes.and_then(|b| gdk::Texture::from_bytes(&glib::Bytes::from_owned(b)).map_err(|e| e.to_string())) {
                    Ok(texture) => {
                        service.good_icon.set(true);
                        service.show_icon(&texture);
                    }
                    Err(err) => log::debug!("icon of {}: {err}", service.key),
                }
            },
        );
    }

    fn show_icon(&self, texture: &gdk::Texture) {
        self.image.remove_css_class("placeholder");
        // As an icon, so it is drawn at the icon size, whatever the size the
        // site sent.
        self.image.set_from_gicon(texture);
        if let Some(env) = self.env.upgrade() {
            let file = env.icon_file(&self.key);
            let _ = std::fs::create_dir_all(env.data.join("icons"));
            if let Err(err) = texture.save_to_png(&file) {
                log::debug!("{}: {err}", file.display());
            }
        }
    }

    /// Tells the page its notification was clicked.
    pub fn notification_clicked(&self) {
        if let Some(n) = self.notification.borrow_mut().take() {
            n.clicked();
        }
    }

    /// The page, made now if it does not exist yet.
    pub fn ensure_view(self: &Rc<Self>) -> webkit6::WebView {
        if let Some(view) = self.view() {
            return view;
        }
        let view = self.make_view();
        *self.view.borrow_mut() = Some(view.clone());
        view
    }

    /// Forgets the page (removing a service); its storage stays on disk.
    pub fn drop_view(&self) {
        // The page closes when the view goes; its process may be another
        // page's too, in a shared profile, so it is not ended here.
        let view = self.view.borrow_mut().take();
        if let Some(view) = view {
            view.try_close();
        }
    }

    /// Writes the session cookies of a loaded service (see `cookies`),
    /// then calls `done`.
    pub fn save_cookies(&self, done: impl FnOnce() + 'static) {
        let manager = self.view().and_then(|v| v.network_session()).and_then(|s| s.cookie_manager());
        match (manager, self.env.upgrade()) {
            (Some(manager), Some(env)) => cookies::save(&manager, &env.profile_dir(&self.profile()), done),
            _ => done(),
        }
    }

    pub fn reload(&self) {
        if let Some(view) = self.view() {
            view.reload();
        }
    }

    pub fn set_muted(&self, muted: bool) {
        self.cfg.borrow_mut().muted = muted;
        if let Some(view) = self.view() {
            view.set_is_muted(muted);
        }
    }

    /// Wipes the service's cookies, logins and cache, then reloads it.
    pub fn clear_data(&self) {
        let Some(view) = self.view() else {
            // Not loaded: the folders go.
            if let Some(env) = self.env.upgrade() {
                let _ = std::fs::remove_dir_all(env.profile_dir(&self.profile()));
                let _ = std::fs::remove_dir_all(env.cache.join("services").join(self.profile()));
            }
            return;
        };
        if let Some(env) = self.env.upgrade() {
            cookies::forget(&env.profile_dir(&self.profile()));
        }
        let Some(manager) = view.network_session().and_then(|s| s.website_data_manager()) else { return };
        let guard = glib::thread_guard::ThreadGuard::new(view.clone());
        manager.clear(webkit6::WebsiteDataTypes::ALL, glib::TimeSpan::from_seconds(0), gio::Cancellable::NONE, move |result| {
            if let Err(err) = result {
                log::warn!("clearing data: {err}");
            }
            guard.get_ref().reload();
        });
    }

    fn set_unread(&self, n: u32) {
        let before = self.unread.get();
        if n == before {
            return;
        }
        self.unread.set(n);
        // The first count of a page just loaded is no news.
        if self.counted.replace(true) && n > before {
            self.notify_unread(n);
        }
        self.badge.set_visible(n > 0);
        self.badge.set_label(&if n > 99 { "99+".to_owned() } else { n.to_string() });
        if let Some(env) = self.env.upgrade()
            && let Some(changed) = env.changed.borrow().as_ref()
        {
            changed(());
        }
    }

    fn make_view(self: &Rc<Self>) -> webkit6::WebView {
        self.counted.set(false);
        let env = self.env.upgrade().expect("the window outlives its services");
        let cfg = self.cfg.borrow().clone();
        let profile = env.profile(&cfg);

        let settings = webkit6::Settings::new();
        settings.set_enable_webrtc(cfg.media);
        settings.set_enable_media_stream(cfg.media);
        settings.set_enable_developer_extras(true);
        settings.set_enable_site_specific_quirks(true);
        match cfg.user_agent.as_ref().or(env.user_agent.as_ref()) {
            Some(agent) => settings.set_user_agent(Some(agent)),
            // WebKit's own, as Safari on a Mac: sites know Safari on macOS
            // and turn "Safari on Linux" away (Outlook answers it with
            // OwaBasicUnsupportedException). The engine is the same.
            None => {
                let agent = settings.user_agent().map(|a| mac_agent(&a)).unwrap_or_default();
                if !agent.is_empty() {
                    settings.set_user_agent(Some(&agent));
                }
            }
        }

        let view = webkit6::WebView::builder()
            .network_session(&profile.session)
            .web_context(&profile.context)
            .settings(&settings)
            .hexpand(true)
            .vexpand(true)
            .build();
        // The window's colour while the page loads, not a white flash.
        view.set_background_color(&gdk::RGBA::new(0.106, 0.122, 0.153, 1.0));
        view.set_is_muted(cfg.muted);
        view.set_zoom_level(self.zoom.get());
        self.connect(&view, &env);
        // Loaded once the profile's session cookies are back: the login.
        let first = self.pending.borrow_mut().take().unwrap_or(cfg.url);
        profile.when_ready(glib::clone!(
            #[weak]
            view,
            move || view.load_uri(&first)
        ));
        view
    }

    fn connect(self: &Rc<Self>, view: &webkit6::WebView, env: &Rc<Env>) {
        let weak = Rc::downgrade(self);
        view.connect_title_notify(glib::clone!(
            #[strong]
            weak,
            move |view| {
                let Some(service) = weak.upgrade() else { return };
                let title = view.title().unwrap_or_default();
                if !service.cfg.borrow().browser {
                    service.set_unread(privacy::unread(&title).unwrap_or(0));
                }
                service.page_changed();
            }
        ));
        view.connect_uri_notify(glib::clone!(
            #[strong]
            weak,
            move |_| {
                if let Some(service) = weak.upgrade() {
                    service.page_changed();
                }
            }
        ));
        view.connect_favicon_notify(glib::clone!(
            #[strong]
            weak,
            move |view| {
                let Some(service) = weak.upgrade() else { return };
                if service.cfg.borrow().icon.is_none()
                    && !service.good_icon.get()
                    && let Some(texture) = view.favicon()
                {
                    service.show_icon(&texture);
                }
            }
        ));
        // Loaded: a better icon than the favicon, when the page has one —
        // its touch icon (large, sharp on the button), or the icon it
        // declares when WebKit gave none (an SVG on another server).
        view.connect_load_changed(glib::clone!(
            #[strong]
            weak,
            move |view, event| {
                let Some(service) = weak.upgrade() else { return };
                if event != webkit6::LoadEvent::Finished || service.good_icon.get() || service.cfg.borrow().icon.is_some() {
                    return;
                }
                let has_favicon = view.favicon().is_some();
                let weak = Rc::downgrade(&service);
                view.call_async_javascript_function(
                    PAGE_ICON,
                    Some(&glib::VariantDict::new(None).end()),
                    Some("orbit-pathfinder"),
                    None,
                    gio::Cancellable::NONE,
                    move |result| {
                        let Some(service) = weak.upgrade() else { return };
                        let Ok(value) = result else { return };
                        let found = value.to_str();
                        let (kind, url) = found.split_once(' ').unwrap_or(("", ""));
                        if url.starts_with("https://") && (kind == "touch" || !has_favicon) && !service.good_icon.get() {
                            // Once a run: further pages need not ask again.
                            service.good_icon.set(true);
                            service.fetch_icon(url.to_owned());
                        }
                    },
                );
            }
        ));
        view.connect_show_notification(glib::clone!(
            #[strong]
            weak,
            move |view, notification| {
                let Some(service) = weak.upgrade() else { return false };
                service.notify(view, notification);
                true
            }
        ));
        view.connect_permission_request(glib::clone!(
            #[strong]
            weak,
            move |_, request| {
                let Some(service) = weak.upgrade() else { return false };
                let cfg = service.cfg.borrow();
                let allow = if request.is::<webkit6::NotificationPermissionRequest>() {
                    cfg.notifications
                } else if request.is::<webkit6::UserMediaPermissionRequest>()
                    || request.is::<webkit6::DeviceInfoPermissionRequest>()
                {
                    cfg.media
                } else {
                    // Clipboard reads (paste in a chat) are allowed; location,
                    // pointer lock, storage across sites and the rest are not.
                    request.is::<webkit6::ClipboardPermissionRequest>()
                };
                if allow { request.allow() } else { request.deny() }
                true
            }
        ));
        view.connect_decide_policy(glib::clone!(
            #[strong]
            weak,
            #[weak]
            env,
            #[upgrade_or]
            false,
            move |view, decision, kind| {
                let Some(service) = weak.upgrade() else { return false };
                let Some(nav) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>() else { return false };
                let Some(action) = nav.navigation_action() else { return false };
                let uri = action.request().and_then(|r| r.uri()).map(|u| u.to_string()).unwrap_or_default();
                match kind {
                    webkit6::PolicyDecisionType::NewWindowAction => {
                        let cfg = service.cfg.borrow();
                        if cfg.browser || privacy::stays_inside(&uri, &cfg.url, &cfg.domains) {
                            // A link ("Sign in", a message): opened right here,
                            // as a tab would be. A script's window (a login
                            // that reports back to the page) stays a window.
                            if action.navigation_type() == webkit6::NavigationType::LinkClicked && !uri.is_empty() {
                                decision.ignore();
                                view.load_uri(&privacy::clean(&uri));
                                return true;
                            }
                            return false;
                        }
                        decision.ignore();
                        env.open_external(&uri);
                        true
                    }
                    webkit6::PolicyDecisionType::NavigationAction
                        if action.is_user_gesture() && action.navigation_type() == webkit6::NavigationType::LinkClicked =>
                    {
                        let clean = privacy::clean(&uri);
                        if clean == uri {
                            return false;
                        }
                        decision.ignore();
                        view.load_uri(&clean);
                        true
                    }
                    _ => false,
                }
            }
        ));
        // Windows that stay inside (logins, previews) open as small windows
        // sharing the service's storage.
        view.connect_create(glib::clone!(
            #[strong]
            weak,
            move |view, _| Some(popup(view, weak.clone()).upcast())
        ));
        // Why a page did not load, for the log: the address's host and the
        // error (never the full address, which may carry tokens).
        view.connect_load_failed(|_, _, uri, error| {
            if !error.matches(webkit6::NetworkError::Cancelled) {
                log::warn!("load failed: {}: {error}", privacy::host(uri).unwrap_or_default());
            }
            false
        });
        view.connect_load_failed_with_tls_errors(|_, uri, _, errors| {
            log::warn!("TLS error: {}: {errors:?}", privacy::host(uri).unwrap_or_default());
            false
        });
        // A page that crashed (or ran out of memory) comes back.
        view.connect_web_process_terminated(|view, reason| {
            if reason != webkit6::WebProcessTerminationReason::TerminatedByApi {
                log::warn!("web process ended ({reason:?}); reloading");
                view.reload();
            }
        });
    }

    fn page_changed(&self) {
        if let Some(env) = self.env.upgrade()
            && let Some(f) = env.page_changed.borrow().as_ref()
        {
            f(self);
        }
    }

    /// The unread count went up and the page said nothing (Gmail without its
    /// desktop notifications, most sites): the desktop's notification centre
    /// hears of it — one notification per service, replaced as the count
    /// changes, and none when the page itself just notified or the user is
    /// looking at the service.
    fn notify_unread(&self, n: u32) {
        let Some(env) = self.env.upgrade() else { return };
        let cfg = self.cfg.borrow();
        if cfg.browser || !cfg.notifications || cfg.muted {
            return;
        }
        if self.last_page_notification.get().is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(20)) {
            return;
        }
        if env.watching.borrow().as_ref().is_some_and(|w| w(self)) {
            return;
        }
        let note = gio::Notification::new(&cfg.name);
        note.set_body(Some(&tr_args("pathfinder-unread", &[("count", number(n))])));
        if let Some(icon) = self.image.gicon() {
            note.set_icon(&icon);
        }
        note.set_default_action_and_target_value("app.service", Some(&self.key.to_variant()));
        env.app.send_notification(Some(&format!("{}-unread", self.key)), &note);
    }

    /// Hands a page's notification to the desktop.
    fn notify(&self, view: &webkit6::WebView, notification: &webkit6::Notification) {
        let Some(env) = self.env.upgrade() else { return };
        let title = notification.title().map(|t| t.to_string()).unwrap_or_else(|| self.name());
        let n = gio::Notification::new(&title);
        if let Some(body) = notification.body() {
            n.set_body(Some(&body));
        }
        if let Some(texture) = view.favicon() {
            n.set_icon(&texture);
        }
        n.set_default_action_and_target_value("app.service", Some(&self.key.to_variant()));
        let id = format!("{}-{}", self.key, notification.id());
        env.app.send_notification(Some(&id), &n);
        notification.connect_closed(glib::clone!(
            #[weak(rename_to = app)]
            env.app,
            move |_| app.withdraw_notification(&id)
        ));
        *self.notification.borrow_mut() = Some(notification.clone());
        self.last_page_notification.set(Some(std::time::Instant::now()));
    }
}

/// The page's best icon: "touch <url>" for its touch icon, "icon <url>"
/// for the icon it declares, "" for none.
const PAGE_ICON: &str = r#"
const touch = document.querySelector('link[rel~="apple-touch-icon"], link[rel~="apple-touch-icon-precomposed"]');
if (touch && touch.href) return 'touch ' + touch.href;
const icon = document.querySelector('link[rel~="icon"]');
return icon && icon.href ? 'icon ' + icon.href : '';
"#;

/// An icon's bytes (two megabytes at most).
fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(15))).build().into();
    agent
        .get(url)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .with_config()
        .limit(2 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| e.to_string())
}

/// "X11; Linux x86_64" → "Macintosh; Intel Mac OS X 10_15_7" (what Safari
/// sends) in WebKit's own user agent, so its version keeps up with WebKit.
fn mac_agent(agent: &str) -> String {
    match (agent.find('('), agent.find(')')) {
        (Some(open), Some(close)) if open < close => {
            format!("{}(Macintosh; Intel Mac OS X 10_15_7){}", &agent[..open], &agent[close + 1..])
        }
        _ => agent.to_owned(),
    }
}

/// A window a page's script opened (a login that reports back to it): a
/// window of its own, sharing the service's storage. Once it lands back on
/// the service's own address — the login done — it closes, and the service
/// goes there.
fn popup(view: &webkit6::WebView, service: Weak<Service>) -> webkit6::WebView {
    let popup = webkit6::WebView::builder().related_view(view).build();
    let window = gtk::Window::builder().default_width(1000).default_height(760).child(&popup).build();
    if let Some(parent) = view.root().and_downcast::<gtk::Window>() {
        window.set_transient_for(Some(&parent));
    }
    popup.connect_ready_to_show(glib::clone!(
        #[weak]
        window,
        move |_| window.present()
    ));
    popup.connect_close(glib::clone!(
        #[weak]
        window,
        move |_| window.destroy()
    ));
    popup.connect_title_notify(glib::clone!(
        #[weak]
        window,
        move |p| window.set_title(p.title().as_deref())
    ));
    popup.connect_load_changed(glib::clone!(
        #[weak]
        window,
        #[weak]
        view,
        move |p, event| {
            if event != webkit6::LoadEvent::Committed {
                return;
            }
            let (Some(uri), Some(service)) = (p.uri(), service.upgrade()) else { return };
            let home = privacy::host(&service.cfg.borrow().url);
            if privacy::host(&uri).is_some() && privacy::host(&uri) == home {
                view.load_uri(&uri);
                window.destroy();
            }
        }
    ));
    popup
}

/// Downloads go to the Downloads folder, under a name not yet taken, and
/// say so when done.
fn watch_download(env: &Rc<Env>, download: &webkit6::Download) {
    download.connect_decide_destination(|download, suggested| {
        let dir = glib::user_special_dir(glib::UserDirectory::Downloads).unwrap_or_else(glib::home_dir);
        let name: String = suggested.chars().map(|c| if c == '/' { '_' } else { c }).collect();
        let name = if name.trim().is_empty() || name.starts_with('.') { format!("download{name}") } else { name };
        let path = unique(dir.join(name));
        download.set_destination(&path.to_string_lossy());
        true
    });
    download.connect_finished(glib::clone!(
        #[weak(rename_to = app)]
        env.app,
        move |download| {
            let Some(dest) = download.destination() else { return };
            let name = PathBuf::from(dest.as_str()).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let n = gio::Notification::new(&tr_args("pathfinder-downloaded", &[("name", text(name))]));
            n.set_icon(&gio::ThemedIcon::new("folder-download"));
            app.send_notification(None, &n);
        }
    ));
    download.connect_failed(|_, err| log::warn!("download: {err}"));
}

/// `path`, or "name (2).ext" and so on when it exists.
fn unique(path: PathBuf) -> PathBuf {
    if path.symlink_metadata().is_err() {
        return path;
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..)
        .map(|n| path.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|p| p.symlink_metadata().is_err())
        .expect("some name is free")
}

#[cfg(test)]
mod tests {
    #[test]
    fn makes_mac_agents() {
        let linux = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";
        assert_eq!(
            super::mac_agent(linux),
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15"
        );
    }
}
