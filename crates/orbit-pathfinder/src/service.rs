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
use orbit_app::i18n::{number, text, tr, tr_args};
use webkit6::prelude::*;

use crate::{config, cookies, privacy, recipe, trust};

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
    /// The browser's tabs changed (opened, closed, shown, retitled).
    pub tabs_changed: RefCell<Option<PageHook>>,
    /// The block lists, put on the pages of the services that block.
    pub shield: Option<Rc<crate::shield::Shield>>,
    /// The Ferdium recipes found.
    pub recipes: recipe::Index,
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
        // The local-network certificates the user trusted (see `trust`).
        trust::apply(&self.data, &session);
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
    /// The pages: one for a service, one per tab for the browser.
    views: RefCell<Vec<webkit6::WebView>>,
    /// What its pages carry besides the site's own: the block lists and
    /// its recipe.
    content: webkit6::UserContentManager,
    /// The recipe that reads its count, if any.
    recipe: Option<recipe::Recipe>,
    /// The recipe has given a count: the title's is no longer read.
    recipe_counts: Cell<bool>,
    /// Messages in places that matter less (muted chats, groups): a dot
    /// when there is nothing else.
    indirect: Cell<u32>,
    /// Which of `views` is shown.
    active: Cell<usize>,
    /// Tabs saved from the last run, opened when the browser first shows.
    restore: RefCell<(Vec<String>, usize)>,
    unread: Cell<u32>,
    /// When it was last shown or left: what hibernation counts from.
    last_used: Cell<std::time::Instant>,
    /// The address its page was on when it was unloaded, to come back to.
    resume: RefCell<Option<String>>,
    /// The page's last notification: clicking the desktop's tells the page,
    /// which then opens the conversation.
    notification: RefCell<Option<webkit6::Notification>>,
    zoom: Cell<f64>,
    env: Weak<Env>,
    /// The HTTPS address last retried over HTTP (once each).
    tried_http: RefCell<Option<String>>,
    /// A count was read since the page was made (the first is not news).
    counted: Cell<bool>,
    /// When the page last sent a notification of its own.
    last_page_notification: Cell<Option<std::time::Instant>>,
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
        let content = webkit6::UserContentManager::new();
        if cfg.block
            && let Some(shield) = &env.shield
        {
            shield.attach(&content);
        }
        let recipe = if cfg.browser { None } else { env.recipes.find(cfg.recipe.as_deref(), &cfg.url).cloned() };
        let script = recipe.as_ref().and_then(|r| r.script());
        if let Some(source) = &script {
            let script = webkit6::UserScript::for_world(
                source,
                webkit6::UserContentInjectedFrames::TopFrame,
                webkit6::UserScriptInjectionTime::End,
                recipe::WORLD,
                &[],
                &[],
            );
            content.add_script(&script);
            content.register_script_message_handler(recipe::HANDLER, Some(recipe::WORLD));
        }
        let service = Rc::new(Self {
            key: cfg.key(),
            cfg: RefCell::new(cfg),
            button,
            image,
            badge,
            views: RefCell::new(Vec::new()),
            content,
            recipe: recipe.clone(),
            recipe_counts: Cell::new(false),
            indirect: Cell::new(0),
            active: Cell::new(0),
            restore: RefCell::new((Vec::new(), 0)),
            unread: Cell::new(0),
            last_used: Cell::new(std::time::Instant::now()),
            resume: RefCell::new(None),
            notification: RefCell::new(None),
            zoom: Cell::new(zoom),
            env: Rc::downgrade(env),
            good_icon: Cell::new(false),
            counted: Cell::new(false),
            tried_http: RefCell::new(None),
            last_page_notification: Cell::new(None),
        });
        service.apply_icon();
        if script.is_some() {
            log::info!("{}: recipe {}", service.key, service.recipe.as_ref().map(|r| r.id.as_str()).unwrap_or_default());
            let weak = Rc::downgrade(&service);
            service.content.connect_script_message_received(Some(recipe::HANDLER), move |_, value| {
                if let Some(service) = weak.upgrade() {
                    service.recipe_message(&value.to_str());
                }
            });
        }
        service
    }

    /// What the recipe in the page says: a count, a link to open, a page
    /// script to run.
    fn recipe_message(&self, text: &str) {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(text) else { return };
        if let Some(counts) = message.get("badge").and_then(|b| b.as_array()) {
            let n = |i: usize| counts.get(i).and_then(|v| v.as_u64()).unwrap_or(0).min(u32::MAX as u64) as u32;
            self.recipe_counts.set(true);
            self.indirect.set(n(1));
            self.set_unread(n(0));
            self.show_badge();
        } else if let Some(url) = message.get("open").and_then(|u| u.as_str()) {
            let Some(env) = self.env.upgrade() else { return };
            let cfg = self.cfg.borrow();
            match self.view() {
                Some(view) if privacy::stays_inside(url, &cfg.url, &cfg.domains) => view.load_uri(&privacy::clean(url)),
                _ => env.open_external(url),
            }
        } else if let Some(name) = message.get("unsafe").and_then(|u| u.as_str())
            && let Some(source) = self.recipe.as_ref().and_then(|r| r.page_script(name))
            && let Some(view) = self.view()
        {
            // As the page's own script, where the recipe needs it.
            view.evaluate_javascript(&source, None, None, gio::Cancellable::NONE, |result| {
                if let Err(err) = result {
                    log::debug!("recipe page script: {err}");
                }
            });
        }
    }

    /// The count on the button: the unread messages, or a dot for the ones
    /// that matter less.
    fn show_badge(&self) {
        let (n, indirect) = (self.unread.get(), self.indirect.get());
        if n > 0 {
            self.badge.remove_css_class("indirect");
            self.badge.set_label(&if n > 99 { "99+".to_owned() } else { n.to_string() });
        } else {
            self.badge.add_css_class("indirect");
            self.badge.set_label("");
        }
        self.badge.set_visible(n > 0 || indirect > 0);
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

    /// The page shown (the active tab, for the browser).
    pub fn view(&self) -> Option<webkit6::WebView> {
        self.views.borrow().get(self.active.get()).cloned()
    }

    /// Every page (every tab).
    pub fn views(&self) -> Vec<webkit6::WebView> {
        self.views.borrow().clone()
    }

    pub fn active_tab(&self) -> usize {
        self.active.get()
    }

    pub fn is_loaded(&self) -> bool {
        !self.views.borrow().is_empty()
    }

    /// Shown, or just left: the time unused starts again.
    pub fn touch(&self) {
        self.last_used.set(std::time::Instant::now());
    }

    /// Whether its page may be unloaded after `minutes` unused: loaded, not
    /// the browser, not kept active, and not playing sound (music, a call).
    pub fn can_hibernate(&self, minutes: u32) -> bool {
        let cfg = self.cfg.borrow();
        self.is_loaded()
            && !cfg.browser
            && !cfg.always_active
            && self.last_used.get().elapsed() >= std::time::Duration::from_secs(minutes as u64 * 60)
            && !self.views.borrow().iter().any(|v| v.is_playing_audio())
    }

    /// Unloads the page — the window has taken it out of the stack — keeping
    /// where it was; the next click loads it there. The count stays as it
    /// was last read.
    pub fn hibernate(&self) {
        *self.resume.borrow_mut() = self.view().and_then(|v| v.uri()).map(|u| u.to_string()).filter(|u| u.starts_with("http"));
        self.drop_view();
        self.button.add_css_class("hibernated");
    }

    /// Tabs to open the first time the browser shows (the last run's).
    pub fn set_restore(&self, urls: Vec<String>, active: usize) {
        *self.restore.borrow_mut() = (urls, active);
    }

    /// The tabs' addresses, to save for the next run.
    pub fn tab_urls(&self) -> Vec<String> {
        if self.views.borrow().is_empty() {
            // Not opened this run: what was saved stays.
            return self.restore.borrow().0.clone();
        }
        self.views.borrow().iter().filter_map(|v| v.uri().map(|u| u.to_string())).filter(|u| u.starts_with("http")).collect()
    }

    pub fn zoom(&self) -> f64 {
        self.zoom.get()
    }

    pub fn set_zoom(&self, zoom: f64) {
        let zoom = zoom.clamp(0.3, 3.0);
        self.zoom.set(zoom);
        for view in self.views() {
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
    pub fn open(self: &Rc<Self>, url: &str) {
        match self.view() {
            Some(view) => view.load_uri(url),
            None => self.first_pages(Some(url.to_owned())),
        }
    }

    /// A new tab on `url`, shown when `show` (a middle click opens it
    /// behind).
    pub fn new_tab(self: &Rc<Self>, url: &str, show: bool) {
        if self.views.borrow().is_empty() {
            self.first_pages(Some(url.to_owned()));
        } else {
            let view = self.make_view(url.to_owned());
            self.views.borrow_mut().push(view);
            if show {
                self.active.set(self.views.borrow().len() - 1);
            }
        }
        self.tabs_changed();
    }

    /// A page a script opened (`window.open`), taken as a tab: it shares the
    /// opener's storage and process, as WebKit requires.
    fn adopt_tab(self: &Rc<Self>, opener: &webkit6::WebView) -> webkit6::WebView {
        let view = webkit6::WebView::builder().related_view(opener).user_content_manager(&self.content).hexpand(true).vexpand(true).build();
        if let Some(settings) = WebViewExt::settings(opener) {
            view.set_settings(&settings);
        }
        view.set_background_color(&gdk::RGBA::new(0.106, 0.122, 0.153, 1.0));
        view.set_zoom_level(self.zoom.get());
        if let Some(env) = self.env.upgrade() {
            self.connect(&view, &env);
        }
        self.views.borrow_mut().push(view.clone());
        self.active.set(self.views.borrow().len() - 1);
        self.tabs_changed();
        view
    }

    pub fn show_tab(&self, index: usize) {
        if index < self.views.borrow().len() {
            self.active.set(index);
            self.tabs_changed();
        }
    }

    /// Closes tab `index`, handing back its page for the window to drop; the
    /// last tab closed leaves a new one on the start page.
    pub fn close_tab(self: &Rc<Self>, index: usize) -> Option<webkit6::WebView> {
        let view = {
            let mut views = self.views.borrow_mut();
            if index >= views.len() {
                return None;
            }
            views.remove(index)
        };
        let len = self.views.borrow().len();
        if len == 0 {
            let home = self.cfg.borrow().url.clone();
            let fresh = self.make_view(home);
            self.views.borrow_mut().push(fresh);
            self.active.set(0);
        } else if self.active.get() >= len || self.active.get() > index {
            self.active.set(self.active.get().saturating_sub(1).min(len - 1));
        }
        view.try_close();
        self.tabs_changed();
        Some(view)
    }

    fn tabs_changed(&self) {
        if let Some(env) = self.env.upgrade()
            && let Some(f) = env.tabs_changed.borrow().as_ref()
        {
            f(self);
        }
    }

    /// The first pages: the last run's tabs (the browser), then `extra`, or
    /// else the service's address.
    fn first_pages(self: &Rc<Self>, extra: Option<String>) {
        self.button.remove_css_class("hibernated");
        let extra = extra.or_else(|| self.resume.borrow_mut().take());
        let (saved, saved_active) = std::mem::take(&mut *self.restore.borrow_mut());
        for url in &saved {
            let view = self.make_view(url.clone());
            self.views.borrow_mut().push(view);
        }
        self.active.set(saved_active.min(saved.len().saturating_sub(1)));
        if let Some(url) = extra {
            let view = self.make_view(url);
            self.views.borrow_mut().push(view);
            self.active.set(self.views.borrow().len() - 1);
        }
        if self.views.borrow().is_empty() {
            let url = self.cfg.borrow().url.clone();
            let view = self.make_view(url);
            self.views.borrow_mut().push(view);
            self.active.set(0);
        }
    }

    /// Asks whether to trust the certificate a local address showed; if so,
    /// every profile accepts it from now on, and the page loads.
    fn ask_trust(&self, view: &webkit6::WebView, uri: &str, host: &str, certificate: &gio::TlsCertificate) {
        let Some(env) = self.env.upgrade() else { return };
        let changed = trust::changed(&env.data, host, certificate);
        let mut detail = tr_args("pathfinder-cert-detail", &[("host", text(host.to_owned()))]);
        if changed {
            detail = format!("{}\n\n{detail}", tr_args("pathfinder-cert-changed", &[("host", text(host.to_owned()))]));
        }
        let subject = certificate.subject_name().map(|s| s.to_string()).unwrap_or_default();
        let until = certificate.not_valid_after().and_then(|d| d.format("%x").ok()).map(|d| d.to_string()).unwrap_or_default();
        detail = format!(
            "{detail}\n\n{}",
            tr_args(
                "pathfinder-cert-facts",
                &[("subject", text(subject)), ("until", text(until)), ("sha256", text(trust::fingerprint(certificate)))],
            )
        );
        let dialog = gtk::AlertDialog::builder()
            .message(tr("pathfinder-cert-title"))
            .detail(detail)
            .buttons([tr("pathfinder-cancel"), tr("pathfinder-cert-trust")])
            .cancel_button(0)
            .default_button(0)
            .modal(true)
            .build();
        let parent = view.root().and_downcast::<gtk::Window>();
        let (view, uri, host, certificate) = (view.clone(), uri.to_owned(), host.to_owned(), certificate.clone());
        let env = Rc::downgrade(&env);
        dialog.choose(parent.as_ref(), gio::Cancellable::NONE, move |answer| {
            if answer != Ok(1) {
                return;
            }
            let Some(env) = env.upgrade() else { return };
            trust::add(&env.data, &host, &certificate);
            for profile in env.profiles.borrow().values() {
                profile.session.allow_tls_certificate_for_host(&certificate, &host);
            }
            view.load_uri(&uri);
        });
    }

    /// In the browser, a local address that failed over HTTPS is tried once
    /// over HTTP (see `privacy::http_fallback`). Whether it was.
    fn try_http(&self, view: &webkit6::WebView, uri: &str) -> bool {
        if !self.cfg.borrow().browser {
            return false;
        }
        let Some(http) = privacy::http_fallback(uri) else { return false };
        if self.tried_http.borrow().as_deref() == Some(uri) {
            return false;
        }
        *self.tried_http.borrow_mut() = Some(uri.to_owned());
        view.load_uri(&http);
        true
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
        if !self.is_loaded() {
            self.first_pages(None);
        }
        self.view().expect("a page was just made")
    }

    /// Forgets the page (removing a service); its storage stays on disk.
    pub fn drop_view(&self) {
        // The page closes when the view goes; its process may be another
        // page's too, in a shared profile, so it is not ended here.
        let views = std::mem::take(&mut *self.views.borrow_mut());
        for view in views {
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
        for view in self.views() {
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
        self.show_badge();
        if let Some(env) = self.env.upgrade()
            && let Some(changed) = env.changed.borrow().as_ref()
        {
            changed(());
        }
    }

    fn make_view(self: &Rc<Self>, first: String) -> webkit6::WebView {
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
            .user_content_manager(&self.content)
            .hexpand(true)
            .vexpand(true)
            .build();
        // The window's colour while the page loads, not a white flash.
        view.set_background_color(&gdk::RGBA::new(0.106, 0.122, 0.153, 1.0));
        view.set_is_muted(cfg.muted);
        view.set_zoom_level(self.zoom.get());
        self.connect(&view, &env);
        // Loaded once the profile's session cookies are back (the login) and
        // the block lists are in force.
        let shield = env.shield.clone().filter(|_| cfg.block);
        profile.when_ready(glib::clone!(
            #[weak]
            view,
            move || match shield {
                Some(shield) => shield.when_ready(move || view.load_uri(&first)),
                None => view.load_uri(&first),
            }
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
                if service.cfg.borrow().browser {
                    service.tabs_changed();
                } else if !service.recipe_counts.get() {
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
                if service.cfg.borrow().browser {
                    service.tabs_changed();
                    return;
                }
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
                        // The browser: a link meant for a new window is a new
                        // tab (behind it, for a middle click).
                        if cfg.browser && action.navigation_type() == webkit6::NavigationType::LinkClicked && !uri.is_empty() {
                            decision.ignore();
                            let show = action.mouse_button() != 2;
                            drop(cfg);
                            service.new_tab(&privacy::clean(&uri), show);
                            return true;
                        }
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
        // A window a script opens: a tab, in the browser; a window of its own
        // elsewhere (a login that reports back to the service).
        view.connect_create(glib::clone!(
            #[strong]
            weak,
            move |view, _| {
                let service = weak.upgrade()?;
                if service.cfg.borrow().browser {
                    return Some(service.adopt_tab(view).upcast());
                }
                Some(popup(view, weak.clone()).upcast())
            }
        ));
        // A tab a script closes (window.close()).
        view.connect_close(glib::clone!(
            #[strong]
            weak,
            move |view| {
                let Some(service) = weak.upgrade() else { return };
                let index = service.views.borrow().iter().position(|v| v == view);
                if service.cfg.borrow().browser
                    && let Some(i) = index
                {
                    service.close_tab(i);
                }
            }
        ));
        // Why a page did not load, for the log: the address's host and the
        // error (never the full address, which may carry tokens).
        view.connect_load_failed(glib::clone!(
            #[strong]
            weak,
            move |view, _, uri, error| {
                if error.matches(webkit6::NetworkError::Cancelled) {
                    return false;
                }
                log::warn!("load failed: {}: {error}", privacy::host(uri).unwrap_or_default());
                if weak.upgrade().is_some_and(|s| s.try_http(view, uri)) {
                    return true;
                }
                error_page(view, uri, &tr("pathfinder-error-load"), error.message());
                true
            }
        ));
        // A certificate refused: on the local network, the user may trust it
        // (Proxmox, routers and NAS sign their own); elsewhere, refused.
        view.connect_load_failed_with_tls_errors(glib::clone!(
            #[strong]
            weak,
            move |view, uri, certificate, errors| {
                let host = privacy::host(uri).unwrap_or_default();
                log::warn!("TLS error: {host}: {errors:?}");
                let Some(service) = weak.upgrade() else { return false };
                if !privacy::is_local(&host) {
                    error_page(view, uri, &tr("pathfinder-error-tls"), &tr("pathfinder-error-tls-detail"));
                    return true;
                }
                service.ask_trust(view, uri, &host, certificate);
                true
            }
        ));
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

/// A page in place of one that did not load, readable on the dark window
/// (WebKit's own writes black on it) and in the user's language.
fn error_page(view: &webkit6::WebView, uri: &str, title: &str, detail: &str) {
    let host = privacy::host(uri).unwrap_or_else(|| uri.to_owned());
    let escape = |t: &str| t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let html = format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>{host}</title><style>
:root {{ color-scheme: dark light; }}
body {{ font: 15px system-ui, sans-serif; background: #1b1f27; color: #d8dee9; margin: 0; display: grid; place-items: center; min-height: 100vh; }}
@media (prefers-color-scheme: light) {{ body {{ background: #eceff4; color: #2e3440; }} }}
main {{ max-width: 560px; padding: 30px; }}
h1 {{ font-size: 20px; margin: 0 0 10px; }}
p {{ opacity: .8; line-height: 1.5; }}
code {{ opacity: .6; font-size: 13px; }}
</style></head><body><main><h1>{title}</h1><p>{detail}</p><code>{host}</code></main></body></html>"#,
        host = escape(&host),
        title = escape(title),
        detail = escape(detail),
    );
    view.load_alternate_html(&html, uri, None);
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
