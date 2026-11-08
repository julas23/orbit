//! What a browser has besides the page: find in the page (Ctrl+F), the
//! downloads of this run (the button in the bar), the list that finds a
//! service, a tab, a bookmark or a page of the history by a few letters
//! (Ctrl+K, and Ctrl+H, Ctrl+Shift+O for the history and the bookmarks
//! alone), the suggestions under the address bar, reader mode, and the
//! private window (Ctrl+Shift+N: storage that is forgotten when it
//! closes).

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

use gtk::{gdk, gio, glib, prelude::*};
use orbit_app::i18n::{number, text, tr, tr_args};
use webkit6::prelude::*;

// ------------------------------------------------------------ find bar

pub struct FindBar {
    pub widget: gtk::Revealer,
    entry: gtk::SearchEntry,
    count: gtk::Label,
    case: gtk::ToggleButton,
    view: RefCell<Option<webkit6::WebView>>,
    handlers: RefCell<Vec<(webkit6::FindController, glib::SignalHandlerId)>>,
}

impl FindBar {
    pub fn new() -> Rc<Self> {
        let entry = gtk::SearchEntry::builder()
            .placeholder_text(tr("pathfinder-find"))
            .width_chars(30)
            .build();
        let button = |icon: &str, tip: &str| {
            gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(tr(tip))
                .css_classes(["flat"])
                .focus_on_click(false)
                .build()
        };
        let previous = button("go-up-symbolic", "pathfinder-find-previous");
        let next = button("go-down-symbolic", "pathfinder-find-next");
        let case = gtk::ToggleButton::builder()
            .label("Aa")
            .tooltip_text(tr("pathfinder-find-case"))
            .css_classes(["flat", "option"])
            .focus_on_click(false)
            .build();
        let count = gtk::Label::builder()
            .width_chars(12)
            .css_classes(["find-count"])
            .build();
        let close = button("window-close-symbolic", "pathfinder-find-close");
        let row = gtk::Box::builder().spacing(5).css_classes(["findbar"]).build();
        for w in [
            entry.upcast_ref::<gtk::Widget>(),
            previous.upcast_ref(),
            next.upcast_ref(),
            case.upcast_ref(),
            count.upcast_ref(),
        ] {
            row.append(w);
        }
        row.append(&gtk::Box::builder().hexpand(true).build());
        row.append(&close);
        let widget = gtk::Revealer::builder().child(&row).build();
        let bar = Rc::new(Self {
            widget,
            entry,
            count,
            case,
            view: RefCell::default(),
            handlers: RefCell::default(),
        });
        let weak = Rc::downgrade(&bar);
        bar.entry.connect_search_changed(move |_| {
            if let Some(b) = weak.upgrade() {
                b.search();
            }
        });
        let weak = Rc::downgrade(&bar);
        bar.case.connect_toggled(move |_| {
            if let Some(b) = weak.upgrade() {
                b.search();
            }
        });
        let weak = Rc::downgrade(&bar);
        bar.entry.connect_activate(move |_| {
            if let Some(b) = weak.upgrade() {
                b.step(true);
            }
        });
        let weak = Rc::downgrade(&bar);
        next.connect_clicked(move |_| {
            if let Some(b) = weak.upgrade() {
                b.step(true);
            }
        });
        let weak = Rc::downgrade(&bar);
        previous.connect_clicked(move |_| {
            if let Some(b) = weak.upgrade() {
                b.step(false);
            }
        });
        let weak = Rc::downgrade(&bar);
        close.connect_clicked(move |_| {
            if let Some(b) = weak.upgrade() {
                b.hide();
            }
        });
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&bar);
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(b) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            match key {
                gdk::Key::Escape => b.hide(),
                gdk::Key::Return | gdk::Key::KP_Enter if state.contains(gdk::ModifierType::SHIFT_MASK) => b.step(false),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        bar.entry.add_controller(keys);
        bar
    }

    pub fn shown(&self) -> bool {
        self.widget.reveals_child()
    }

    /// Ctrl+F: the bar over `view`, the text selected to type over.
    pub fn show(self: &Rc<Self>, view: &webkit6::WebView) {
        self.set_view(Some(view));
        self.widget.set_reveal_child(true);
        self.entry.grab_focus();
        self.entry.select_region(0, -1);
        self.search();
    }

    /// The page shown changed: the search follows it, while the bar shows.
    pub fn set_view(self: &Rc<Self>, view: Option<&webkit6::WebView>) {
        if self.view.borrow().as_ref() == view {
            return;
        }
        if let Some(old) = self.view.borrow_mut().take()
            && let Some(c) = old.find_controller()
        {
            c.search_finish();
        }
        for (c, h) in self.handlers.borrow_mut().drain(..) {
            c.disconnect(h);
        }
        *self.view.borrow_mut() = view.cloned();
        let Some(c) = view.and_then(|v| v.find_controller()) else {
            return;
        };
        let weak = Rc::downgrade(self);
        let counted = c.connect_counted_matches(move |_, n| {
            if let Some(b) = weak.upgrade() {
                b.count.set_label(&if n == 0 {
                    tr("pathfinder-find-none")
                } else {
                    tr_args("pathfinder-find-count", &[("n", number(f64::from(n)))])
                });
            }
        });
        let weak = Rc::downgrade(self);
        let failed = c.connect_failed_to_find_text(move |_| {
            if let Some(b) = weak.upgrade() {
                b.count.set_label(&tr("pathfinder-find-none"));
            }
        });
        self.handlers.borrow_mut().extend([(c.clone(), counted), (c, failed)]);
        if self.shown() {
            self.search();
        }
    }

    fn options(&self) -> u32 {
        let mut options = webkit6::FindOptions::WRAP_AROUND;
        if !self.case.is_active() {
            options |= webkit6::FindOptions::CASE_INSENSITIVE;
        }
        options.bits()
    }

    fn search(&self) {
        let Some(c) = self.view.borrow().as_ref().and_then(|v| v.find_controller()) else {
            return;
        };
        let text = self.entry.text();
        if text.is_empty() {
            c.search_finish();
            self.count.set_label("");
            return;
        }
        c.search(&text, self.options(), 1000);
        c.count_matches(&text, self.options(), 1000);
    }

    fn step(&self, forward: bool) {
        if let Some(c) = self.view.borrow().as_ref().and_then(|v| v.find_controller()) {
            if forward {
                c.search_next();
            } else {
                c.search_previous();
            }
        }
    }

    pub fn hide(&self) {
        if let Some(c) = self.view.borrow().as_ref().and_then(|v| v.find_controller()) {
            c.search_finish();
        }
        self.widget.set_reveal_child(false);
        if let Some(v) = self.view.borrow().as_ref() {
            v.grab_focus();
        }
    }
}

// ----------------------------------------------------------- downloads

/// The downloads of this run, in a popover under the button in the bar.
/// The button shows once there is one, and pulses while any runs.
pub struct Downloads {
    pub button: gtk::MenuButton,
    list: gtk::Box,
    running: Cell<u32>,
}

impl Downloads {
    pub fn new() -> Rc<Self> {
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(5)
            .css_classes(["downloads"])
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(400)
            .min_content_width(380)
            .build();
        let button = gtk::MenuButton::builder()
            .icon_name("folder-download-symbolic")
            .tooltip_text(tr("pathfinder-downloads"))
            .css_classes(["flat"])
            .visible(false)
            .popover(&gtk::Popover::builder().child(&scroll).build())
            .build();
        Rc::new(Self {
            button,
            list,
            running: Cell::new(0),
        })
    }

    pub fn add(self: &Rc<Self>, download: &webkit6::Download) {
        self.button.set_visible(true);
        self.running.set(self.running.get() + 1);
        self.button.add_css_class("downloading");
        let name = gtk::Label::builder()
            .label(tr("pathfinder-download-starting"))
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .build();
        let progress = gtk::ProgressBar::builder().hexpand(true).build();
        let action = gtk::Button::builder()
            .icon_name("process-stop-symbolic")
            .tooltip_text(tr("pathfinder-download-cancel"))
            .css_classes(["flat"])
            .valign(gtk::Align::Center)
            .build();
        let folder = gtk::Button::builder()
            .icon_name("folder-symbolic")
            .tooltip_text(tr("pathfinder-download-folder"))
            .css_classes(["flat"])
            .valign(gtk::Align::Center)
            .build();
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(3)
            .hexpand(true)
            .build();
        column.append(&name);
        column.append(&progress);
        let row = gtk::Box::builder().spacing(5).css_classes(["download"]).build();
        row.append(&column);
        row.append(&action);
        row.append(&folder);
        self.list.prepend(&row);

        let done = Rc::new(Cell::new(false));
        {
            let name = name.clone();
            download.connect_created_destination(move |_, dest| {
                let shown = PathBuf::from(dest)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                name.set_label(&shown);
            });
        }
        {
            let progress = progress.clone();
            download.connect_estimated_progress_notify(move |d| progress.set_fraction(d.estimated_progress()));
        }
        {
            let (done, d) = (done.clone(), download.clone());
            action.connect_clicked(move |_| {
                if done.get() {
                    if let Some(dest) = d.destination() {
                        let uri = gio::File::for_path(dest.as_str()).uri();
                        if let Err(err) = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>) {
                            log::warn!("{uri}: {err}");
                        }
                    }
                } else {
                    d.cancel();
                }
            });
        }
        {
            let d = download.clone();
            folder.connect_clicked(move |_| {
                let dir = d
                    .destination()
                    .and_then(|p| PathBuf::from(p.as_str()).parent().map(PathBuf::from))
                    .or_else(|| glib::user_special_dir(glib::UserDirectory::Downloads))
                    .unwrap_or_else(glib::home_dir);
                let uri = gio::File::for_path(dir).uri();
                if let Err(err) = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>) {
                    log::warn!("{uri}: {err}");
                }
            });
        }
        let weak = Rc::downgrade(self);
        let finish = move || {
            if let Some(d) = weak.upgrade() {
                d.running.set(d.running.get().saturating_sub(1));
                if d.running.get() == 0 {
                    d.button.remove_css_class("downloading");
                }
            }
        };
        let finish = Rc::new(finish);
        {
            let (finish, done, action, progress) = (finish.clone(), done.clone(), action.clone(), progress.clone());
            download.connect_finished(move |_| {
                if done.replace(true) {
                    return;
                }
                progress.set_fraction(1.0);
                progress.set_visible(false);
                action.set_icon_name("document-open-symbolic");
                action.set_tooltip_text(Some(&tr("pathfinder-download-open")));
                finish();
            });
        }
        {
            let (name, progress, action) = (name.clone(), progress.clone(), action.clone());
            download.connect_failed(move |_, err| {
                if done.replace(true) {
                    return;
                }
                progress.set_visible(false);
                action.set_visible(false);
                name.set_label(&tr_args(
                    "pathfinder-download-failed",
                    &[("reason", text(err.to_string()))],
                ));
                finish();
            });
        }
    }
}

// ---------------------------------------------------------- chooser

/// One line of the chooser.
pub struct Item {
    pub title: String,
    pub detail: String,
    pub icon: &'static str,
    pub run: Rc<dyn Fn()>,
    /// Delete takes it away (a page of the history, a bookmark).
    pub delete: Option<Rc<dyn Fn()>>,
}

impl Item {
    pub fn new(title: String, detail: String, icon: &'static str, run: impl Fn() + 'static) -> Self {
        Self {
            title,
            detail,
            icon,
            run: Rc::new(run),
            delete: None,
        }
    }
}

pub type Query = Rc<dyn Fn(&str) -> Vec<Item>>;

/// A list over the window that `query` fills as one types; Enter or a
/// click runs the item, Delete takes it away if it can be.
pub fn choose(parent: &gtk::Window, placeholder: &str, query: Query) {
    let window = gtk::Window::builder()
        .transient_for(parent)
        .modal(true)
        .decorated(false)
        .default_width(680)
        .default_height(460)
        .css_classes(["orbit-pathfinder", "chooser"])
        .build();
    let entry = gtk::SearchEntry::builder().placeholder_text(placeholder).build();
    let list = gtk::ListBox::builder().css_classes(["results"]).build();
    let scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(5)
        .css_classes(["chooser-box"])
        .build();
    content.append(&entry);
    content.append(&scroll);
    window.set_child(Some(&content));

    let items: Rc<RefCell<Vec<Item>>> = Rc::default();
    let refresh = {
        let (items, list, entry, query) = (items.clone(), list.clone(), entry.clone(), query.clone());
        move || {
            while let Some(row) = list.first_child() {
                list.remove(&row);
            }
            let found = query(&entry.text());
            for item in &found {
                let row = gtk::Box::builder().spacing(10).build();
                row.append(&gtk::Image::from_icon_name(item.icon));
                let column = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .hexpand(true)
                    .build();
                column.append(
                    &gtk::Label::builder()
                        .label(&item.title)
                        .xalign(0.0)
                        .ellipsize(gtk::pango::EllipsizeMode::End)
                        .build(),
                );
                if !item.detail.is_empty() {
                    column.append(
                        &gtk::Label::builder()
                            .label(&item.detail)
                            .xalign(0.0)
                            .ellipsize(gtk::pango::EllipsizeMode::Middle)
                            .css_classes(["detail"])
                            .build(),
                    );
                }
                row.append(&column);
                list.append(&row);
            }
            *items.borrow_mut() = found;
            if let Some(first) = list.row_at_index(0) {
                list.select_row(Some(&first));
            }
        }
    };
    let refresh = Rc::new(refresh);
    {
        let refresh = refresh.clone();
        entry.connect_search_changed(move |_| refresh());
    }
    let run = {
        let (window, items) = (window.clone(), items.clone());
        move |index: i32| {
            let picked = items.borrow().get(index as usize).map(|i| i.run.clone());
            window.close();
            if let Some(f) = picked {
                glib::idle_add_local_once(move || f());
            }
        }
    };
    let run = Rc::new(run);
    {
        let run = run.clone();
        list.connect_row_activated(move |_, row| run(row.index()));
    }
    {
        let (list, run) = (list.clone(), run.clone());
        entry.connect_activate(move |_| {
            if let Some(row) = list.selected_row() {
                run(row.index());
            }
        });
    }
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (window, list, items, refresh) = (window.clone(), list.clone(), items.clone(), refresh.clone());
        keys.connect_key_pressed(move |_, key, _, state| {
            let step = match key {
                gdk::Key::Escape => {
                    window.close();
                    return glib::Propagation::Stop;
                }
                gdk::Key::Delete if state.contains(gdk::ModifierType::SHIFT_MASK) || list.has_focus() => {
                    let at = list.selected_row().map_or(-1, |r| r.index());
                    let delete = items.borrow().get(at as usize).and_then(|i| i.delete.clone());
                    if let Some(f) = delete {
                        f();
                        refresh();
                        return glib::Propagation::Stop;
                    }
                    return glib::Propagation::Proceed;
                }
                gdk::Key::Down => 1,
                gdk::Key::Up => -1,
                _ => return glib::Propagation::Proceed,
            };
            let at = list.selected_row().map_or(0, |r| r.index());
            if let Some(row) = list.row_at_index((at + step).max(0)) {
                list.select_row(Some(&row));
                // The entry keeps the keys: typing goes on.
                row.grab_focus();
                GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
            }
            glib::Propagation::Stop
        });
    }
    window.add_controller(keys);
    refresh();
    window.present();
    entry.grab_focus();
}

// ------------------------------------------------------ suggestions

/// The pages offered under the address bar while one types: Up and Down
/// pick one, Enter opens it (or what was typed, with none picked).
pub struct Suggestions {
    popover: gtk::Popover,
    list: gtk::ListBox,
    urls: RefCell<Vec<String>>,
}

impl Suggestions {
    pub fn new(address: &gtk::Entry, open: impl Fn(&str) + 'static) -> Rc<Self> {
        let list = gtk::ListBox::builder().css_classes(["suggestions"]).build();
        let popover = gtk::Popover::builder()
            .child(&list)
            .has_arrow(false)
            .autohide(false)
            .can_focus(false)
            .position(gtk::PositionType::Bottom)
            .halign(gtk::Align::Start)
            .css_classes(["suggestions-popover"])
            .build();
        popover.set_parent(address);
        let s = Rc::new(Self {
            popover,
            list,
            urls: RefCell::default(),
        });
        let weak = Rc::downgrade(&s);
        s.list.connect_row_activated(move |_, row| {
            if let Some(s) = weak.upgrade() {
                let url = s.urls.borrow().get(row.index() as usize).cloned();
                s.hide();
                if let Some(url) = url {
                    open(&url);
                }
            }
        });
        s
    }

    pub fn show(&self, found: Vec<(String, String)>, width: i32) {
        while let Some(row) = self.list.first_child() {
            self.list.remove(&row);
        }
        if found.is_empty() {
            self.hide();
            return;
        }
        for (url, title) in &found {
            let row = gtk::Box::builder().spacing(10).build();
            row.append(
                &gtk::Label::builder()
                    .label(if title.is_empty() { url } else { title })
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .max_width_chars(60)
                    .build(),
            );
            row.append(
                &gtk::Label::builder()
                    .label(url)
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk::pango::EllipsizeMode::Middle)
                    .css_classes(["detail"])
                    .build(),
            );
            self.list.append(&row);
        }
        *self.urls.borrow_mut() = found.into_iter().map(|(u, _)| u).collect();
        self.list.set_size_request(width.max(300), -1);
        self.popover.popup();
    }

    pub fn hide(&self) {
        self.list.unselect_all();
        self.popover.popdown();
    }

    pub fn shown(&self) -> bool {
        self.popover.is_visible()
    }

    /// Up or Down moved the pick.
    pub fn step(&self, by: i32) {
        let n = self.urls.borrow().len() as i32;
        if n == 0 {
            return;
        }
        let at = self.list.selected_row().map_or(-1, |r| r.index());
        let next = at + by;
        if next < 0 || next >= n {
            self.list.unselect_all();
            return;
        }
        if let Some(row) = self.list.row_at_index(next) {
            self.list.select_row(Some(&row));
        }
    }

    /// The page picked, if one is.
    pub fn picked(&self) -> Option<String> {
        let at = self.list.selected_row()?.index();
        self.urls.borrow().get(at as usize).cloned()
    }
}

// ---------------------------------------------------------- reader

/// Reader mode: the page's article alone, in large clear type; run again,
/// the page as it was. The article is the block with the most text in
/// paragraphs (Readability's idea, much simplified).
pub const READER: &str = r#"
if (document.documentElement.dataset.orbitReader === '1') {
    location.reload();
    return 'off';
}
const candidates = document.querySelectorAll('article, main, [role=main], section, div');
let best = null, score = 0;
for (const el of candidates) {
    let t = 0;
    for (const p of el.querySelectorAll(':scope > p, :scope > div > p, :scope > section > p')) {
        t += (p.innerText || '').length;
    }
    if (el.tagName === 'ARTICLE' || el.getAttribute('role') === 'main') t *= 1.4;
    if (t > score) { score = t; best = el; }
}
if (!best || score < 400) return 'none';
const title = (document.querySelector('h1') || {}).innerText || document.title;
const article = best.cloneNode(true);
article.querySelectorAll('script, style, noscript, iframe, form, nav, aside, button, input, select, footer, header, svg, [aria-hidden=true], [role=navigation], [class*=share], [class*=comment], [class*=related], [class*=advert], [id*=ad-]').forEach(e => e.remove());
article.querySelectorAll('*').forEach(e => { e.removeAttribute('style'); e.removeAttribute('class'); });
const dark = window.matchMedia('(prefers-color-scheme: dark)').matches;
document.documentElement.dataset.orbitReader = '1';
const css = `
  html { background: ${dark ? '#1b1f27' : '#f6f4ee'}; }
  body { max-width: 720px; margin: 40px auto; padding: 0 24px 80px;
         font: 19px/1.65 Georgia, 'Noto Serif', serif; color: ${dark ? '#d8dee9' : '#2e3440'}; }
  h1 { font: 700 34px/1.25 'Noto Sans', sans-serif; margin-bottom: 30px; }
  h2, h3 { font-family: 'Noto Sans', sans-serif; line-height: 1.3; }
  img, video, figure { max-width: 100%; height: auto; }
  a { color: ${dark ? '#88c0d0' : '#2e6da4'}; }
  pre, code { font-size: 15px; background: ${dark ? '#2e3440' : '#ebe8df'}; padding: 2px 4px; border-radius: 4px; }
  pre { padding: 12px; overflow-x: auto; }
  blockquote { border-left: 3px solid #81a1c1; margin-left: 0; padding-left: 16px; opacity: .9; }`;
document.head.innerHTML = '';
const meta = document.createElement('meta'); meta.setAttribute('charset', 'utf-8'); document.head.append(meta);
const t = document.createElement('title'); t.textContent = document.title; document.head.append(t);
const style = document.createElement('style'); style.textContent = css; document.head.append(style);
document.body.innerHTML = '';
document.body.removeAttribute('class'); document.body.removeAttribute('style');
const h1 = document.createElement('h1'); h1.textContent = title; document.body.append(h1);
document.body.append(article);
window.scrollTo(0, 0);
return 'on';
"#;

// --------------------------------------------------- private window

/// Ctrl+Shift+N: a window of its own, on storage that lives in memory
/// and is gone when it closes — no history, no cookies kept.
pub fn private_window(app: &gtk::Application, url: &str, search: String) {
    let session = webkit6::NetworkSession::new_ephemeral();
    session.set_itp_enabled(true);
    session.set_tls_errors_policy(webkit6::TLSErrorsPolicy::Fail);
    if let Some(m) = session.cookie_manager() {
        m.set_accept_policy(webkit6::CookieAcceptPolicy::NoThirdParty);
    }
    let settings = webkit6::Settings::new();
    settings.set_enable_developer_extras(true);
    let view = webkit6::WebView::builder()
        .network_session(&session)
        .settings(&settings)
        .hexpand(true)
        .vexpand(true)
        .build();
    view.set_background_color(&gdk::RGBA::new(0.106, 0.122, 0.153, 1.0));
    let address = gtk::Entry::builder()
        .hexpand(true)
        .placeholder_text(tr("pathfinder-address-hint"))
        .input_purpose(gtk::InputPurpose::Url)
        .css_classes(["address"])
        .build();
    let button = |icon: &str, tip: &str| {
        gtk::Button::builder()
            .icon_name(icon)
            .tooltip_text(tr(tip))
            .css_classes(["flat"])
            .build()
    };
    let back = button("go-previous-symbolic", "pathfinder-back");
    let forward = button("go-next-symbolic", "pathfinder-forward");
    let reload = button("view-refresh-symbolic", "pathfinder-reload");
    let badge = gtk::Label::builder()
        .label(tr("pathfinder-private"))
        .css_classes(["private-badge"])
        .build();
    let bar = gtk::Box::builder().spacing(5).css_classes(["bar"]).build();
    for w in [
        back.upcast_ref::<gtk::Widget>(),
        forward.upcast_ref(),
        reload.upcast_ref(),
        address.upcast_ref(),
        badge.upcast_ref(),
    ] {
        bar.append(w);
    }
    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    content.append(&bar);
    content.append(&view);
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(tr("pathfinder-private"))
        .default_width(1200)
        .default_height(850)
        .css_classes(["orbit-pathfinder", "private"])
        .child(&content)
        .build();
    {
        let address = address.clone();
        view.connect_uri_notify(move |v| {
            if !address.has_focus() {
                address.set_text(v.uri().as_deref().unwrap_or_default());
            }
        });
    }
    {
        let window = window.clone();
        view.connect_title_notify(move |v| {
            let title = v.title().map(|t| t.to_string()).unwrap_or_default();
            window.set_title(Some(&format!("{title} — {}", tr("pathfinder-private"))));
        });
    }
    {
        let (back, forward) = (back.clone(), forward.clone());
        view.connect_load_changed(move |v, _| {
            back.set_sensitive(v.can_go_back());
            forward.set_sensitive(v.can_go_forward());
        });
    }
    {
        let view = view.clone();
        address.connect_activate(move |e| {
            let url = crate::privacy::typed(&e.text(), &search);
            if !url.is_empty() {
                view.load_uri(&url);
                view.grab_focus();
            }
        });
    }
    {
        let v = view.clone();
        back.connect_clicked(move |_| v.go_back());
        let v = view.clone();
        forward.connect_clicked(move |_| v.go_forward());
        let v = view.clone();
        reload.connect_clicked(move |_| v.reload());
    }
    // A link that opens a window opens here.
    view.connect_create(|v, action| {
        if let Some(uri) = action.clone().request().and_then(|r| r.uri()) {
            v.load_uri(&uri);
        }
        None
    });
    window.present();
    if url.is_empty() {
        address.grab_focus();
    } else {
        view.load_uri(url);
    }
}
