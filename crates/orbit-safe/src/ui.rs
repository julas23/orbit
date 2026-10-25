//! The window: closed, a card that asks what `/etc/pam.d/orbit-safe` asks
//! (the password, a finger, a code); open, the keyring's passwords or the
//! one-time codes, a search, and the chosen item beside the list.
//!
//! It closes again after a while without use, when the session locks, and
//! with the Lock button. A copied secret is marked as one (password
//! managers' hint: orbit-clip leaves it out of the history) and wiped from
//! the clipboard after a while, if it is still there.

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap},
    rc::Rc,
    sync::mpsc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gtk::{gdk, glib, prelude::*};
use orbit_app::i18n::{number, text, tr, tr_args};
use orbit_common::ConfigDir;
use serde::Deserialize;
use zeroize::Zeroizing;

use crate::{
    pam,
    secrets::{Item, PASSWORD_SCHEMA, Service, TOTP_SCHEMA},
    totp::{self, Totp},
};

/// `~/.config/orbit/safe.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct Config {
    /// Minutes without use before the safe closes; 0 never.
    lock_after_minutes: u64,
    /// Seconds a copied secret stays in the clipboard; 0 forever.
    clear_clipboard_seconds: u32,
    /// Close when the session locks (orbit-core's `lock` event).
    lock_with_session: bool,
    /// Length of a generated password.
    generated_length: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config { lock_after_minutes: 5, clear_clipboard_seconds: 30, lock_with_session: true, generated_length: 20 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Passwords,
    Codes,
}

/// A code on screen, refreshed every second.
struct LiveCode {
    totp: Totp,
    code: gtk::Label,
    left: gtk::Label,
}

struct Safe {
    config: Config,
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    // Closed.
    ask: gtk::Label,
    answer: gtk::Entry,
    said: gtk::Label,
    answers: RefCell<Option<mpsc::Sender<String>>>,
    asking: Cell<bool>,
    // Open.
    service: RefCell<Option<Rc<Service>>>,
    items: RefCell<Vec<Item>>,
    codes: RefCell<HashMap<String, Totp>>,
    mode: Cell<Mode>,
    search: gtk::SearchEntry,
    list: gtk::ListBox,
    details: gtk::Box,
    status: gtk::Label,
    live: RefCell<Vec<LiveCode>>,
    last_use: Cell<Instant>,
    copied: RefCell<Option<gdk::ContentProvider>>,
}

pub fn build(app: &gtk::Application, dir: &ConfigDir) -> anyhow::Result<gtk::ApplicationWindow> {
    let config: Config = dir.app("safe")?;
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(tr("safe-title"))
        .css_classes(["orbit-safe"])
        .default_width(800)
        .default_height(520)
        .build();

    // Closed: the card.
    let icon = gtk::Image::builder().icon_name("security-high-symbolic").pixel_size(50).css_classes(["icon"]).build();
    let title = gtk::Label::builder().label(tr("safe-title")).css_classes(["title"]).build();
    let ask = gtk::Label::builder().wrap(true).max_width_chars(36).justify(gtk::Justification::Center).css_classes(["ask"]).build();
    let answer = gtk::Entry::builder().visibility(false).sensitive(false).css_classes(["answer"]).build();
    let said = gtk::Label::builder().wrap(true).max_width_chars(36).justify(gtk::Justification::Center).css_classes(["said"]).build();
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["card"])
        .build();
    for w in [icon.upcast_ref::<gtk::Widget>(), title.upcast_ref(), ask.upcast_ref(), answer.upcast_ref(), said.upcast_ref()] {
        card.append(w);
    }

    // Open: the bar, the list and the item.
    let passwords_tab = gtk::ToggleButton::builder().label(tr("safe-passwords")).active(true).build();
    let codes_tab = gtk::ToggleButton::builder().label(tr("safe-codes")).group(&passwords_tab).build();
    let tabs = gtk::Box::builder().css_classes(["linked", "tabs"]).build();
    tabs.append(&passwords_tab);
    tabs.append(&codes_tab);
    let search = gtk::SearchEntry::builder().placeholder_text(tr("safe-search")).hexpand(true).build();
    let new = gtk::Button::builder().icon_name("list-add-symbolic").tooltip_text(tr("safe-new")).build();
    let lock = gtk::Button::builder().icon_name("system-lock-screen-symbolic").tooltip_text(tr("safe-lock")).build();
    let bar = gtk::Box::builder().spacing(10).css_classes(["bar"]).build();
    for w in [tabs.upcast_ref::<gtk::Widget>(), search.upcast_ref(), new.upcast_ref(), lock.upcast_ref()] {
        bar.append(w);
    }
    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::Browse).css_classes(["items"]).build();
    let scrolled = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&list).vexpand(true).build();
    scrolled.set_size_request(300, -1);
    let details = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).hexpand(true).css_classes(["details"]).build();
    let body = gtk::Box::builder().spacing(10).vexpand(true).build();
    body.append(&scrolled);
    body.append(&details);
    let status = gtk::Label::builder().xalign(0.0).css_classes(["status"]).build();
    let open = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(10).css_classes(["page"]).build();
    open.append(&bar);
    open.append(&body);
    open.append(&status);

    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
    stack.add_named(&card, Some("closed"));
    stack.add_named(&open, Some("open"));
    window.set_child(Some(&stack));

    let safe = Rc::new(Safe {
        config,
        window: window.clone(),
        stack,
        ask,
        answer: answer.clone(),
        said,
        answers: RefCell::new(None),
        asking: Cell::new(false),
        service: RefCell::new(None),
        items: RefCell::new(Vec::new()),
        codes: RefCell::new(HashMap::new()),
        mode: Cell::new(Mode::Passwords),
        search: search.clone(),
        list: list.clone(),
        details,
        status,
        live: RefCell::new(Vec::new()),
        last_use: Cell::new(Instant::now()),
        copied: RefCell::new(None),
    });

    {
        let safe = safe.clone();
        answer.connect_activate(move |entry| safe.reply(entry));
    }
    {
        let safe = safe.clone();
        passwords_tab.connect_toggled(move |tab| {
            safe.mode.set(if tab.is_active() { Mode::Passwords } else { Mode::Codes });
            safe.rebuild();
        });
    }
    {
        let safe = safe.clone();
        search.connect_search_changed(move |_| safe.rebuild());
    }
    {
        let safe = safe.clone();
        list.connect_row_selected(move |_, row| {
            if let Some(path) = row.map(|r| r.widget_name()) {
                safe.show(&path);
            }
        });
    }
    {
        let safe = safe.clone();
        new.connect_clicked(move |_| {
            safe.list.unselect_all();
            safe.show_new();
        });
    }
    {
        let safe = safe.clone();
        lock.connect_clicked(move |_| safe.close());
    }
    safe.watch_use();
    safe.follow_session();
    {
        let safe = safe.clone();
        glib::timeout_add_seconds_local(1, move || {
            safe.tick();
            glib::ControlFlow::Continue
        });
    }
    safe.authenticate();
    Ok(window)
}

impl Safe {
    // ------------------------------------------------------------ closed

    /// Starts a PAM conversation and shows it on the card.
    fn authenticate(self: &Rc<Self>) {
        if self.asking.replace(true) {
            return;
        }
        let (events, answers) = pam::start();
        *self.answers.borrow_mut() = Some(answers);
        self.ask.set_label(&tr("safe-waiting"));
        let safe = self.clone();
        glib::spawn_future_local(async move {
            while let Ok(event) = events.recv().await {
                match event {
                    pam::Event::Ask { text, echo } => {
                        safe.ask.set_label(&prompt_text(&text));
                        safe.answer.set_visibility(echo);
                        safe.answer.set_sensitive(true);
                        safe.answer.grab_focus();
                    }
                    pam::Event::Say { text, error } => {
                        safe.said.set_label(&text);
                        set_class(&safe.said, "error", error);
                    }
                    pam::Event::Done(result) => {
                        safe.asking.set(false);
                        safe.answers.borrow_mut().take();
                        safe.answer.set_sensitive(false);
                        match result {
                            Ok(()) => {
                                safe.said.set_label("");
                                safe.open();
                            }
                            Err(reason) => {
                                log::info!("safe: not opened: {reason}");
                                safe.said.set_label(&tr("safe-refused"));
                                set_class(&safe.said, "error", true);
                                safe.authenticate();
                            }
                        }
                        break;
                    }
                }
            }
        });
    }

    fn reply(&self, entry: &gtk::Entry) {
        let Some(answers) = self.answers.borrow().clone() else { return };
        let answer = Zeroizing::new(entry.text().to_string());
        entry.set_text("");
        entry.set_sensitive(false);
        self.said.set_label("");
        let _ = answers.send(answer.to_string());
    }

    /// Back to the card: what was open is dropped.
    fn close(self: &Rc<Self>) {
        if self.stack.visible_child_name().as_deref() == Some("closed") {
            return;
        }
        self.items.borrow_mut().clear();
        self.codes.borrow_mut().clear();
        self.live.borrow_mut().clear();
        self.service.borrow_mut().take();
        clear(&self.list);
        clear_box(&self.details);
        self.search.set_text("");
        self.status.set_label("");
        self.stack.set_visible_child_name("closed");
        self.authenticate();
    }

    // ------------------------------------------------------------ open

    fn open(self: &Rc<Self>) {
        self.stack.set_visible_child_name("open");
        self.last_use.set(Instant::now());
        self.search.grab_focus();
        self.reload(None);
    }

    /// Reads the keyring again, then selects `select` (an item's path).
    fn reload(self: &Rc<Self>, select: Option<String>) {
        let safe = self.clone();
        glib::spawn_future_local(async move {
            let service = match safe.service().await {
                Ok(service) => service,
                Err(err) => return safe.report(&err),
            };
            match load(&service).await {
                Ok((items, codes)) => {
                    *safe.items.borrow_mut() = items;
                    *safe.codes.borrow_mut() = codes;
                    safe.rebuild();
                    safe.select(select.as_deref());
                }
                Err(err) => safe.report(&err),
            }
        });
    }

    async fn service(&self) -> anyhow::Result<Rc<Service>> {
        if let Some(service) = self.service.borrow().clone() {
            return Ok(service);
        }
        let service = Rc::new(Service::connect().await?);
        *self.service.borrow_mut() = Some(service.clone());
        Ok(service)
    }

    fn report(&self, err: &anyhow::Error) {
        log::warn!("safe: {err:#}");
        self.status.set_label(&format!("{err:#}"));
        set_class(&self.status, "error", true);
    }

    fn note(&self, message: &str) {
        self.status.set_label(message);
        set_class(&self.status, "error", false);
    }

    /// Fills the list from what is loaded, for the mode and the search.
    fn rebuild(self: &Rc<Self>) {
        let selected = self.list.selected_row().map(|r| r.widget_name().to_string());
        clear(&self.list);
        self.live.borrow_mut().clear();
        let query = self.search.text().to_lowercase();
        let codes = self.mode.get() == Mode::Codes;
        let items = self.items.borrow();
        let mut shown = 0;
        for item in items.iter().filter(|i| i.is_totp() == codes) {
            let haystack = [Some(item.label.as_str()), item.username(), item.url(), Some(item.collection.as_str())]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            if !query.is_empty() && !haystack.contains(&query) {
                continue;
            }
            let row = if codes { self.code_row(item) } else { password_row(item) };
            row.set_widget_name(&item.path);
            self.list.append(&row);
            shown += 1;
        }
        if shown == 0 {
            let empty = if !query.is_empty() {
                tr("safe-no-match")
            } else if codes {
                tr("safe-no-codes")
            } else {
                tr("safe-no-passwords")
            };
            self.list.set_placeholder(Some(&gtk::Label::builder().label(empty).css_classes(["placeholder"]).build()));
        }
        drop(items);
        self.tick();
        self.select(selected.as_deref());
    }

    fn select(&self, path: Option<&str>) {
        let Some(path) = path else { return };
        let mut child = self.list.first_child();
        while let Some(widget) = child {
            if let Ok(row) = widget.clone().downcast::<gtk::ListBoxRow>()
                && row.widget_name() == path
            {
                self.list.select_row(Some(&row));
                return;
            }
            child = widget.next_sibling();
        }
    }

    fn code_row(&self, item: &Item) -> gtk::ListBoxRow {
        let name = gtk::Label::builder().label(&item.label).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build();
        let code = gtk::Label::builder().css_classes(["code"]).build();
        let left = gtk::Label::builder().css_classes(["meta"]).width_chars(3).build();
        let line = gtk::Box::builder().spacing(10).build();
        line.append(&name);
        line.append(&code);
        line.append(&left);
        if let Some(totp) = self.codes.borrow().get(&item.path) {
            self.live.borrow_mut().push(LiveCode { totp: totp.clone(), code, left });
        }
        gtk::ListBoxRow::builder().child(&line).build()
    }

    /// Every second: the codes on screen, and closing after a while unused.
    fn tick(self: &Rc<Self>) {
        let now = unix_now();
        for live in self.live.borrow().iter() {
            live.code.set_label(&spaced(&live.totp.code(now)));
            let left = live.totp.remaining(now);
            live.left.set_label(&format!("{left}s"));
            set_class(&live.left, "ending", left <= 5);
        }
        let minutes = self.config.lock_after_minutes;
        if minutes > 0
            && self.stack.visible_child_name().as_deref() == Some("open")
            && self.last_use.get().elapsed() >= Duration::from_secs(minutes * 60)
        {
            log::info!("safe: closed after {minutes} min without use");
            self.close();
        }
    }

    // ------------------------------------------------------------ the item

    fn show(self: &Rc<Self>, path: &str) {
        let Some(item) = self.items.borrow().iter().find(|i| i.path == path).cloned() else { return };
        clear_box(&self.details);
        let form = Form::new(&self.details, item.is_totp(), item.ours());
        form.title.set_text(&item.label);
        if item.is_totp() {
            form.first.set_text(item.attribute(&["issuer"]).unwrap_or_default());
            form.second.set_text(item.attribute(&["account"]).unwrap_or_default());
        } else {
            form.first.set_text(item.username().unwrap_or_default());
            form.second.set_text(item.url().unwrap_or_default());
        }
        if !item.ours() {
            let grid = gtk::Grid::builder().column_spacing(10).row_spacing(5).css_classes(["attributes"]).build();
            for (i, (name, value)) in item.attributes.iter().enumerate() {
                grid.attach(&gtk::Label::builder().label(name).xalign(0.0).css_classes(["meta"]).build(), 0, i as i32, 1, 1);
                grid.attach(
                    &gtk::Label::builder().label(value).xalign(0.0).selectable(true).wrap(true).wrap_mode(gtk::pango::WrapMode::WordChar).build(),
                    1,
                    i as i32,
                    1,
                    1,
                );
            }
            let scrolled = gtk::ScrolledWindow::builder().child(&grid).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
            self.details.append(&scrolled);
        }
        let meta = gtk::Label::builder()
            .label(tr_args("safe-in-collection", &[("collection", text(item.collection.clone()))]))
            .xalign(0.0)
            .css_classes(["meta"])
            .build();
        self.details.append(&meta);

        // The secret, read when the item is chosen.
        let loaded: Rc<RefCell<Option<Zeroizing<String>>>> = Rc::new(RefCell::new(None));
        {
            let safe = self.clone();
            let loaded = loaded.clone();
            let secret = form.secret.clone();
            let path = item.path.clone();
            glib::spawn_future_local(async move {
                let Ok(service) = safe.service().await else { return };
                match service.secret(&path).await {
                    Ok(value) => {
                        let value = Zeroizing::new(String::from_utf8_lossy(&value).into_owned());
                        secret.set_text(&value);
                        *loaded.borrow_mut() = Some(value);
                    }
                    Err(err) => safe.report(&err),
                }
            });
        }

        let buttons = gtk::Box::builder().spacing(10).css_classes(["buttons"]).build();
        let copy = gtk::Button::builder().label(tr(if item.is_totp() { "safe-copy-code" } else { "safe-copy-password" })).build();
        let copy_user = gtk::Button::builder().label(tr("safe-copy-username")).visible(!item.is_totp() && item.username().is_some()).build();
        let save = gtk::Button::builder().label(tr("safe-save")).css_classes(["suggested"]).build();
        let delete = gtk::Button::builder().label(tr("safe-delete")).css_classes(["destructive"]).hexpand(true).halign(gtk::Align::End).build();
        for b in [&copy, &copy_user, &save, &delete] {
            buttons.append(b);
        }
        self.details.append(&buttons);

        {
            let safe = self.clone();
            let secret = form.secret.clone();
            let totp = item.is_totp();
            let path = item.path.clone();
            copy.connect_clicked(move |_| {
                if totp {
                    if let Some(t) = safe.codes.borrow().get(&path) {
                        safe.copy(&t.code(unix_now()));
                    }
                } else {
                    safe.copy(&secret.text());
                }
            });
        }
        {
            let safe = self.clone();
            let user = form.first.clone();
            copy_user.connect_clicked(move |_| safe.copy_plain(&user.text()));
        }
        {
            let safe = self.clone();
            let item = item.clone();
            let form = form.clone();
            save.connect_clicked(move |_| safe.save(&item, &form, loaded.borrow().as_deref().map(String::as_str)));
        }
        {
            let safe = self.clone();
            let path = item.path.clone();
            delete.connect_clicked(move |button| {
                // Twice: the first click asks.
                if !button.has_css_class("confirm") {
                    button.add_css_class("confirm");
                    button.set_label(&tr("safe-delete-confirm"));
                    return;
                }
                let safe = safe.clone();
                let path = path.clone();
                glib::spawn_future_local(async move {
                    let Ok(service) = safe.service().await else { return };
                    match service.delete(&path).await {
                        Ok(()) => {
                            clear_box(&safe.details);
                            safe.note(&tr("safe-deleted"));
                            safe.reload(None);
                        }
                        Err(err) => safe.report(&err),
                    }
                });
            });
        }
    }

    fn save(self: &Rc<Self>, item: &Item, form: &Form, before: Option<&str>) {
        let label = form.title.text().to_string();
        let first = form.first.text().to_string();
        let second = form.second.text().to_string();
        let secret = Zeroizing::new(form.secret.text().to_string());
        let mut attributes = item.attributes.clone();
        let mut new_secret = (Some(secret.as_str()) != before).then(|| secret.clone());
        if item.is_totp() {
            if new_secret.is_some() {
                let Some(totp) = parse_key(&secret) else {
                    return self.report(&anyhow::anyhow!(tr("safe-bad-key")));
                };
                totp_attributes(&mut attributes, &totp, &first, &second);
                new_secret = Some(Zeroizing::new(totp::base32_encode(&totp.key)));
            } else {
                attributes.insert("issuer".into(), first);
                attributes.insert("account".into(), second);
            }
        } else if item.ours() {
            attributes.insert("username".into(), first);
            attributes.insert("url".into(), second);
        }
        let attributes_changed = item.ours() && attributes != item.attributes;
        let label_changed = label != item.label;
        let safe = self.clone();
        let path = item.path.clone();
        glib::spawn_future_local(async move {
            let Ok(service) = safe.service().await else { return };
            let result = async {
                if label_changed {
                    service.set_label(&path, &label).await?;
                }
                if attributes_changed {
                    service.set_attributes(&path, &attributes).await?;
                }
                if let Some(secret) = &new_secret {
                    service.set_secret(&path, secret.as_bytes()).await?;
                }
                anyhow::Ok(())
            }
            .await;
            match result {
                Ok(()) => {
                    safe.note(&tr("safe-saved"));
                    safe.reload(Some(path));
                }
                Err(err) => safe.report(&err),
            }
        });
    }

    /// The form for a new password or code, in the current mode.
    fn show_new(self: &Rc<Self>) {
        clear_box(&self.details);
        let codes = self.mode.get() == Mode::Codes;
        let form = Form::new(&self.details, codes, true);
        let buttons = gtk::Box::builder().spacing(10).css_classes(["buttons"]).build();
        if !codes {
            let generate = gtk::Button::builder().label(tr("safe-generate")).build();
            let secret = form.secret.clone();
            let length = self.config.generated_length;
            generate.connect_clicked(move |_| {
                secret.set_text(&generate_password(length));
                secret.set_show_peek_icon(true);
            });
            buttons.append(&generate);
        }
        let save = gtk::Button::builder().label(tr("safe-save")).css_classes(["suggested"]).hexpand(true).halign(gtk::Align::End).build();
        buttons.append(&save);
        self.details.append(&buttons);
        form.title.grab_focus();
        let safe = self.clone();
        save.connect_clicked(move |_| safe.create(&form, codes));
    }

    fn create(self: &Rc<Self>, form: &Form, codes: bool) {
        let mut label = form.title.text().to_string();
        let first = form.first.text().to_string();
        let second = form.second.text().to_string();
        let mut secret = Zeroizing::new(form.secret.text().to_string());
        let mut attributes = BTreeMap::new();
        if codes {
            let Some(totp) = parse_key(&secret) else {
                return self.report(&anyhow::anyhow!(tr("safe-bad-key")));
            };
            totp_attributes(&mut attributes, &totp, &first, &second);
            if label.trim().is_empty() {
                label = code_label(&attributes);
            }
            secret = Zeroizing::new(totp::base32_encode(&totp.key));
        } else {
            attributes.insert("xdg:schema".into(), PASSWORD_SCHEMA.into());
            attributes.insert("username".into(), first);
            attributes.insert("url".into(), second);
            if label.trim().is_empty() {
                label = attributes["url"].clone();
            }
        }
        if label.trim().is_empty() {
            return self.report(&anyhow::anyhow!(tr("safe-needs-title")));
        }
        let safe = self.clone();
        glib::spawn_future_local(async move {
            let Ok(service) = safe.service().await else { return };
            let result = async {
                let collection = service.default_collection().await?;
                service.create(&collection, &label, &attributes, secret.as_bytes()).await
            }
            .await;
            match result {
                Ok(path) => {
                    safe.note(&tr("safe-saved"));
                    safe.reload(Some(path));
                }
                Err(err) => safe.report(&err),
            }
        });
    }

    // ------------------------------------------------------------ clipboard

    /// Copies a secret: marked as one, wiped after a while.
    fn copy(&self, value: &str) {
        if value.is_empty() {
            return;
        }
        let bytes = glib::Bytes::from(value.as_bytes());
        let provider = gdk::ContentProvider::new_union(&[
            gdk::ContentProvider::for_bytes("text/plain;charset=utf-8", &bytes),
            gdk::ContentProvider::for_bytes("text/plain", &bytes),
            gdk::ContentProvider::for_bytes("UTF8_STRING", &bytes),
            gdk::ContentProvider::for_bytes("x-kde-passwordManagerHint", &glib::Bytes::from_static(b"secret")),
        ]);
        let clipboard = self.window.clipboard();
        if let Err(err) = clipboard.set_content(Some(&provider)) {
            log::warn!("safe: copying: {err}");
            return;
        }
        *self.copied.borrow_mut() = Some(provider.clone());
        let seconds = self.config.clear_clipboard_seconds;
        if seconds == 0 {
            self.note(&tr("safe-copied"));
            return;
        }
        self.note(&tr_args("safe-copied-for", &[("seconds", number(seconds))]));
        let status = self.status.clone();
        glib::timeout_add_seconds_local_once(seconds, move || {
            if clipboard.content().as_ref() == Some(&provider) {
                let _ = clipboard.set_content(None::<&gdk::ContentProvider>);
                status.set_label(&tr("safe-clipboard-cleared"));
            }
        });
    }

    /// Copies what is not a secret (the user name).
    fn copy_plain(&self, value: &str) {
        self.window.clipboard().set_text(value);
        self.note(&tr("safe-copied"));
    }

    // ------------------------------------------------------------ closing

    /// Any key, click or movement in the window counts as use.
    fn watch_use(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let safe = self.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            safe.last_use.set(Instant::now());
            if key == gdk::Key::Escape && safe.stack.visible_child_name().as_deref() == Some("open") && safe.search.text().is_empty() {
                safe.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.window.add_controller(keys);
        let motion = gtk::EventControllerMotion::new();
        let safe = self.clone();
        motion.connect_motion(move |_, _, _| safe.last_use.set(Instant::now()));
        self.window.add_controller(motion);
    }

    /// Closes when the session locks (orbit-core's `lock` event).
    fn follow_session(self: &Rc<Self>) {
        if !self.config.lock_with_session || !orbit_ipc::available() {
            return;
        }
        let (sender, locked) = async_channel::unbounded::<()>();
        let spawned = std::thread::Builder::new().name("lock".into()).spawn(move || {
            let Ok(mut ipc) = orbit_ipc::Connection::open() else { return };
            let Some(lock) = orbit_ipc::event_type("lock") else { return };
            if ipc.request(orbit_ipc::SUBSCRIBE, r#"["lock"]"#).is_err() {
                return;
            }
            while let Ok((kind, body)) = ipc.receive() {
                let change: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                if kind == lock && change["change"] == "locked" && sender.send_blocking(()).is_err() {
                    return;
                }
            }
        });
        if let Err(err) = spawned {
            log::warn!("safe: not following the session's lock: {err}");
            return;
        }
        let safe = self.clone();
        glib::spawn_future_local(async move {
            while locked.recv().await.is_ok() {
                log::info!("safe: closed with the session");
                safe.close();
            }
        });
    }
}

/// The item's fields: title, then user name and address (a password) or
/// issuer and account (a code), then the secret.
#[derive(Clone)]
struct Form {
    title: gtk::Entry,
    first: gtk::Entry,
    second: gtk::Entry,
    secret: gtk::PasswordEntry,
}

impl Form {
    fn new(into: &gtk::Box, codes: bool, editable: bool) -> Self {
        let grid = gtk::Grid::builder().column_spacing(10).row_spacing(10).build();
        let field = |row: i32, name: &str, entry: &gtk::Widget| {
            grid.attach(&gtk::Label::builder().label(tr(name)).xalign(1.0).css_classes(["field"]).build(), 0, row, 1, 1);
            entry.set_hexpand(true);
            grid.attach(entry, 1, row, 1, 1);
        };
        let title = gtk::Entry::new();
        let first = gtk::Entry::builder().editable(editable).build();
        let second = gtk::Entry::builder().editable(editable).build();
        let secret = gtk::PasswordEntry::builder().show_peek_icon(true).build();
        field(0, "safe-field-title", title.upcast_ref());
        if codes {
            field(1, "safe-field-issuer", first.upcast_ref());
            field(2, "safe-field-account", second.upcast_ref());
            secret.set_placeholder_text(Some(&tr("safe-key-hint")));
            field(3, "safe-field-key", secret.upcast_ref());
        } else {
            field(1, "safe-field-username", first.upcast_ref());
            field(2, "safe-field-url", second.upcast_ref());
            field(3, "safe-field-password", secret.upcast_ref());
        }
        into.append(&grid);
        Form { title, first, second, secret }
    }
}

fn password_row(item: &Item) -> gtk::ListBoxRow {
    let name = gtk::Label::builder().label(&item.label).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
    let meta = [item.username(), item.url()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    let meta = if meta.is_empty() { item.collection.clone() } else { meta };
    let meta = gtk::Label::builder().label(meta).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["meta"]).build();
    let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    column.append(&name);
    column.append(&meta);
    gtk::ListBoxRow::builder().child(&column).build()
}

/// Every unlockable collection's items, and the codes' keys.
async fn load(service: &Service) -> anyhow::Result<(Vec<Item>, HashMap<String, Totp>)> {
    let mut items = Vec::new();
    for collection in service.collections().await? {
        if collection.locked && !service.unlock(std::slice::from_ref(&collection.path)).await? {
            log::info!("safe: {} stays locked", collection.label);
            continue;
        }
        items.extend(service.items(&collection).await?);
    }
    items.sort_by_key(|i| i.label.to_lowercase());
    let mut codes = HashMap::new();
    for item in items.iter().filter(|i| i.is_totp()) {
        let key = service.secret(&item.path).await?;
        let Some(mut totp) = Totp::from_base32(&String::from_utf8_lossy(&key)) else {
            log::warn!("safe: {}: not a base32 key", item.label);
            continue;
        };
        totp.digits = item.attribute(&["digits"]).and_then(|d| d.parse().ok()).unwrap_or(6);
        totp.period = item.attribute(&["period"]).and_then(|p| p.parse().ok()).filter(|p| *p > 0).unwrap_or(30);
        totp.algorithm = item.attribute(&["algorithm"]).and_then(totp::Algorithm::parse).unwrap_or(totp::Algorithm::Sha1);
        codes.insert(item.path.clone(), totp);
    }
    Ok((items, codes))
}

/// A key as typed (base32) or an `otpauth://` link.
fn parse_key(text: &str) -> Option<Totp> {
    let text = text.trim();
    if text.starts_with("otpauth://") { Totp::from_uri(text) } else { Totp::from_base32(text) }
}

/// A code's attributes: the link's issuer and account unless typed.
fn totp_attributes(attributes: &mut BTreeMap<String, String>, totp: &Totp, issuer: &str, account: &str) {
    let pick = |typed: &str, from_link: &str| if typed.trim().is_empty() { from_link.to_owned() } else { typed.trim().to_owned() };
    attributes.insert("xdg:schema".into(), TOTP_SCHEMA.into());
    attributes.insert("issuer".into(), pick(issuer, &totp.issuer));
    attributes.insert("account".into(), pick(account, &totp.account));
    attributes.insert("digits".into(), totp.digits.to_string());
    attributes.insert("period".into(), totp.period.to_string());
    attributes.insert("algorithm".into(), totp.algorithm.name().into());
}

fn code_label(attributes: &BTreeMap<String, String>) -> String {
    match (attributes["issuer"].as_str(), attributes["account"].as_str()) {
        ("", account) => account.to_owned(),
        (issuer, "") => issuer.to_owned(),
        (issuer, account) => format!("{issuer} ({account})"),
    }
}

/// Letters, digits and a few signs, from the system's random source.
fn generate_password(length: usize) -> String {
    const SET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789-_.!@#%+=";
    let mut out = String::with_capacity(length);
    let mut byte = [0u8; 1];
    while out.len() < length.max(8) {
        if getrandom::getrandom(&mut byte).is_err() {
            break;
        }
        // Rejection sampling: no bias towards the first signs.
        let limit = 256 - 256 % SET.len();
        if (byte[0] as usize) < limit {
            out.push(SET[byte[0] as usize % SET.len()] as char);
        }
    }
    out
}

/// PAM's question in the window's words, when it is the usual one.
fn prompt_text(text: &str) -> String {
    let plain = text.trim_end_matches(':').trim();
    if plain.eq_ignore_ascii_case("password") || plain.is_empty() { tr("safe-password") } else { plain.to_owned() }
}

/// "123456" as "123 456", as authenticator apps show it.
fn spaced(code: &str) -> String {
    let half = code.len().div_ceil(2);
    format!("{} {}", &code[..half], &code[half..])
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default()
}

fn set_class(widget: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on { widget.add_css_class(class) } else { widget.remove_css_class(class) }
}

fn clear(list: &gtk::ListBox) {
    list.remove_all();
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_split_in_two() {
        assert_eq!(spaced("123456"), "123 456");
        assert_eq!(spaced("12345678"), "1234 5678");
    }

    #[test]
    fn generated_passwords_have_the_length() {
        let password = generate_password(20);
        assert_eq!(password.len(), 20);
        assert!(password.bytes().all(|b| b.is_ascii_graphic()));
    }
}
