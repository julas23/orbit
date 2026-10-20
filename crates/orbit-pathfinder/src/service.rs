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
use orbit_app::i18n::{text, tr_args};
use webkit6::prelude::*;

use crate::{config, cookies, privacy};

type Hook<T> = RefCell<Option<Box<dyn Fn(T)>>>;
type PageHook = Box<dyn Fn(&Service)>;

/// What every service shares: where storage goes, and the hooks back into
/// the window.
pub struct Env {
    pub app: gtk::Application,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub user_agent: Option<String>,
    pub external_browser: Option<String>,
    /// A count changed: the window adds them up again.
    pub changed: Hook<()>,
    /// The page shown changed its title or address: the toolbar follows.
    pub page_changed: RefCell<Option<PageHook>>,
}

impl Env {
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
const ICON: i32 = 30;

/// The site's icon, kept in the service's folder for the next start.
const ICON_FILE: &str = "icon.png";

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
}

impl Service {
    pub fn new(cfg: config::Service, env: &Rc<Env>, zoom: f64) -> Rc<Self> {
        let image = gtk::Image::builder().pixel_size(ICON).css_classes(["service-icon"]).build();
        match &cfg.icon {
            Some(icon) if icon.contains('/') => {
                let file = gio::File::for_path(orbit_common::config::expand_home(std::path::Path::new(icon)));
                image.set_from_gicon(&gio::FileIcon::new(&file));
            }
            Some(icon) => image.set_icon_name(Some(icon)),
            // Until the site's icon arrives.
            // The site's icon from the last run, so the button looks right
            // before the page loads; a placeholder the first time.
            None => match Some(env.data.join("services").join(cfg.key()).join(ICON_FILE)).filter(|p| p.is_file()) {
                Some(saved) => image.set_from_gicon(&gio::FileIcon::new(&gio::File::for_path(saved))),
                None => {
                    image.add_css_class("placeholder");
                    image.set_icon_name(Some("web-browser-symbolic"));
                }
            },
        }
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
        Rc::new(Self {
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
        })
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
        // Taken first: ending the process fires the page's signals at once,
        // and they look at the view.
        let view = self.view.borrow_mut().take();
        if let Some(view) = view {
            view.terminate_web_process();
        }
    }

    /// Writes the session cookies of a loaded service (see `cookies`),
    /// then calls `done`.
    pub fn save_cookies(&self, done: impl FnOnce() + 'static) {
        let manager = self.view().and_then(|v| v.network_session()).and_then(|s| s.cookie_manager());
        match (manager, self.env.upgrade()) {
            (Some(manager), Some(env)) => cookies::save(&manager, &env.data.join("services").join(&self.key), done),
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
                let _ = std::fs::remove_dir_all(env.data.join("services").join(&self.key));
                let _ = std::fs::remove_dir_all(env.cache.join("services").join(&self.key));
            }
            return;
        };
        if let Some(env) = self.env.upgrade() {
            cookies::forget(&env.data.join("services").join(&self.key));
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
        if n == self.unread.get() {
            return;
        }
        self.unread.set(n);
        self.badge.set_visible(n > 0);
        self.badge.set_label(&if n > 99 { "99+".to_owned() } else { n.to_string() });
        if let Some(env) = self.env.upgrade()
            && let Some(changed) = env.changed.borrow().as_ref()
        {
            changed(());
        }
    }

    fn make_view(self: &Rc<Self>) -> webkit6::WebView {
        let env = self.env.upgrade().expect("the window outlives its services");
        let cfg = self.cfg.borrow().clone();
        let data = env.data.join("services").join(&self.key);
        let cache = env.cache.join("services").join(&self.key);
        // WebKit sizes its storage quota from the disk the folder is on; a
        // folder not there yet reads as a disk of no size.
        for dir in [&data, &cache] {
            if let Err(err) = std::fs::create_dir_all(dir) {
                log::warn!("{}: {err}", dir.display());
            }
        }
        // Logins live here: the owner's alone.
        for dir in [&env.data, &data] {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        let session = webkit6::NetworkSession::new(data.to_str(), cache.to_str());
        // Privacy, the same for every service (docs/PLANO-PATHFINDER.md §3).
        session.set_itp_enabled(true);
        session.set_tls_errors_policy(webkit6::TLSErrorsPolicy::Fail);
        if let Some(manager) = session.cookie_manager() {
            manager.set_accept_policy(webkit6::CookieAcceptPolicy::NoThirdParty);
            cookies::persist(&manager, &data);
        }
        if let Some(manager) = session.website_data_manager() {
            manager.set_favicons_enabled(true);
        }
        if let Some(proxy) = &cfg.proxy {
            let settings = webkit6::NetworkProxySettings::new(Some(proxy), &[]);
            session.set_proxy_settings(webkit6::NetworkProxyMode::Custom, Some(&settings));
        }
        session.connect_download_started(glib::clone!(
            #[weak(rename_to = env)]
            env,
            move |_, download| watch_download(&env, download)
        ));

        // A context of its own: its own web process, and the notification
        // permission granted from the start (WhatsApp and Teams ask the page,
        // not the user, whether they may notify).
        let context = webkit6::WebContext::new();
        if cfg.notifications {
            let origin = cfg.url.clone();
            context.connect_initialize_notification_permissions(move |context| {
                let origin = webkit6::SecurityOrigin::for_uri(&origin);
                context.initialize_notification_permissions(&[&origin], &[]);
            });
        }

        let settings = webkit6::Settings::new();
        settings.set_enable_webrtc(cfg.media);
        settings.set_enable_media_stream(cfg.media);
        settings.set_enable_developer_extras(true);
        settings.set_enable_site_specific_quirks(true);
        if let Some(agent) = cfg.user_agent.as_ref().or(env.user_agent.as_ref()) {
            settings.set_user_agent(Some(agent));
        }

        let view = webkit6::WebView::builder()
            .network_session(&session)
            .web_context(&context)
            .settings(&settings)
            .hexpand(true)
            .vexpand(true)
            .build();
        // The window's colour while the page loads, not a white flash.
        view.set_background_color(&gdk::RGBA::new(0.106, 0.122, 0.153, 1.0));
        view.set_is_muted(cfg.muted);
        view.set_zoom_level(self.zoom.get());
        self.connect(&view, &env);
        // The session cookies of the last run go back first: the login.
        let load = glib::clone!(
            #[weak]
            view,
            move || view.load_uri(&cfg.url)
        );
        match session.cookie_manager() {
            Some(manager) => cookies::restore(&manager, &data, load),
            None => load(),
        }
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
                service.set_unread(privacy::unread(&title).unwrap_or(0));
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
                    && let Some(texture) = view.favicon()
                {
                    service.image.remove_css_class("placeholder");
                    // As an icon, so it is drawn at the icon size, whatever
                    // the size the site sent.
                    service.image.set_from_gicon(&texture);
                    if let Some(env) = service.env.upgrade() {
                        let file = env.data.join("services").join(&service.key).join(ICON_FILE);
                        if let Err(err) = texture.save_to_png(&file) {
                            log::debug!("{}: {err}", file.display());
                        }
                    }
                }
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
                        if privacy::stays_inside(&uri, &cfg.url, &cfg.domains) {
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
        view.connect_create(|view, _| {
            let popup = webkit6::WebView::builder().related_view(view).build();
            let window = gtk::Window::builder()
                .default_width(900)
                .default_height(700)
                .child(&popup)
                .build();
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
            Some(popup.upcast())
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
    }
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
