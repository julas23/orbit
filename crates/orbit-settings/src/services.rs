//! The Services page: Orbit's services, a switch each, in groups
//! (docs/PLANO-DISTRO.md §12.2), over `orbit-services` — the same catalogue
//! and helper as `orbitctl services`.
//!
//! A switch turned asks the helper (by pkexec: polkit asks for the password)
//! for the steps that need root, in a thread, the row saying what it is
//! doing; a step on trial shows a bar to keep it, undone by itself when the
//! countdown ends. Turning off what lets things in, and removing, ask first.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::mpsc,
    time::Duration,
};

use gtk::{glib, prelude::*};
use orbit_app::i18n::{tr, tr_args};
use orbit_services::{Action, Event, Group, Kind, Plan, Port, Run, SERVICES, Scope, Service, State};

/// A message from the thread running the helper.
enum Msg {
    Event(Event),
    /// The helper ended: whether it was authorised and succeeded.
    Ended { authorised: bool, ok: bool },
    Failed(String),
}

struct Row {
    service: &'static Service,
    switch: gtk::Switch,
    state: gtk::Label,
    status: gtk::Label,
    remove: gtk::Button,
    keep_bar: gtk::Box,
    keep_label: gtk::Label,
    keep_button: gtk::Button,
    undo_button: gtk::Button,
    /// The firewall's: what each file opens.
    files: Option<gtk::Box>,
    /// The switch is being set to the state read, not by the user.
    updating: Cell<bool>,
    busy: Cell<bool>,
    /// The keep bar's buttons' handlers for the trial under way.
    trial_handlers: RefCell<Vec<(gtk::Button, glib::SignalHandlerId)>>,
}

pub fn page() -> gtk::Widget {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();
    for group in Group::ALL {
        let services: Vec<&'static Service> = SERVICES.iter().filter(|s| s.group == group).collect();
        if services.is_empty() {
            continue;
        }
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(15)
            .css_classes(["card"])
            .build();
        card.append(
            &gtk::Label::builder()
                .label(tr(&format!("settings-svc-group-{}", group.id())))
                .xalign(0.0)
                .css_classes(["heading"])
                .build(),
        );
        for service in services {
            card.append(&row(service));
        }
        body.append(&card);
    }
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .build()
        .upcast()
}

fn row(service: &'static Service) -> gtk::Box {
    let switch = gtk::Switch::builder().valign(gtk::Align::Center).build();
    let remove = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text(tr("settings-svc-remove"))
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .visible(false)
        .build();
    let state = label(&["dim"]);
    let status = label(&["dim"]);
    status.set_visible(false);
    let keep_label = label(&[]);
    keep_label.set_hexpand(true);
    let keep_button = gtk::Button::with_label(&tr("settings-keep"));
    keep_button.add_css_class("suggested-action");
    let undo_button = gtk::Button::with_label(&tr("settings-svc-undo"));
    let keep_bar = gtk::Box::builder()
        .spacing(10)
        .visible(false)
        .css_classes(["keep"])
        .build();
    keep_bar.append(&keep_label);
    keep_bar.append(&undo_button);
    keep_bar.append(&keep_button);

    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(3)
        .hexpand(true)
        .build();
    text.append(
        &gtk::Label::builder()
            .label(tr(&format!("settings-svc-{}", service.id)))
            .xalign(0.0)
            .css_classes(["name"])
            .build(),
    );
    text.append(&label_text(&tr(&format!("settings-svc-{}-summary", service.id)), &[]));
    let opens = opens(&orbit_services::ports(service));
    if !opens.is_empty() {
        text.append(&label_text(&opens, &["dim"]));
    }
    text.append(&state);
    text.append(&status);

    let line = gtk::Box::builder().spacing(10).build();
    line.append(&text);
    line.append(&remove);
    line.append(&switch);

    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(["service"])
        .build();
    outer.append(&line);
    outer.append(&keep_bar);

    let files = (service.id == "firewall").then(|| {
        let files = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(5)
            .build();
        let expander = gtk::Expander::builder()
            .label(tr("settings-svc-open-files"))
            .child(&files)
            .build();
        outer.append(&expander);
        files
    });

    let row = Rc::new(Row {
        service,
        switch,
        state,
        status,
        remove,
        keep_bar,
        keep_label,
        keep_button,
        undo_button,
        files,
        updating: Cell::new(false),
        busy: Cell::new(false),
        trial_handlers: RefCell::default(),
    });
    row.switch.connect_active_notify({
        let row = row.clone();
        move |switch| {
            if row.updating.get() || row.busy.get() {
                return;
            }
            row.ask(if switch.is_active() { Action::On } else { Action::Off });
        }
    });
    row.remove.connect_clicked({
        let row = row.clone();
        move |_| row.ask(Action::Remove)
    });
    row.refresh();
    outer
}

fn label(classes: &[&str]) -> gtk::Label {
    gtk::Label::builder().xalign(0.0).wrap(true).css_classes(classes.to_vec()).build()
}

fn label_text(text: &str, classes: &[&str]) -> gtk::Label {
    let l = label(classes);
    l.set_label(text);
    l
}

/// "Opens 7311/tcp to the local network", from the service's firewall file.
fn opens(ports: &[Port]) -> String {
    let words = |scope| -> String {
        ports
            .iter()
            .filter(|p| p.scope == scope)
            .map(port_label)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut lines = Vec::new();
    for (scope, message) in [(Scope::Lan, "settings-svc-opens-lan"), (Scope::Anyone, "settings-svc-opens-anyone")] {
        let list = words(scope);
        if !list.is_empty() {
            lines.push(tr_args(message, &[("ports", list.into())]));
        }
    }
    lines.join("\n")
}

fn port_label(port: &Port) -> String {
    let base = format!("{}/{}", port.port, port.proto);
    if port.source {
        tr_args("settings-svc-answers", &[("port", base.into())])
    } else {
        base
    }
}

impl Row {
    /// Shows the service's state as it is now.
    fn refresh(self: &Rc<Self>) {
        let state = orbit_services::state(self.service);
        self.updating.set(true);
        self.switch.set_active(state.on);
        self.updating.set(false);
        self.state.set_label(&state_text(self.service, &state));
        self.remove
            .set_visible(self.service.removable && state.installed.is_some());
        if let Some(files) = &self.files {
            while let Some(child) = files.first_child() {
                files.remove(&child);
            }
            for (path, ports) in orbit_services::firewall_files() {
                files.append(&label_text(&path.display().to_string(), &["dim"]));
                for port in ports {
                    let scope = tr(match port.scope {
                        Scope::Lan => "settings-svc-scope-lan",
                        Scope::Anyone => "settings-svc-scope-anyone",
                    });
                    let note = if port.note.is_empty() { String::new() } else { format!(" — {}", port.note) };
                    files.append(&label_text(&format!("    {} ({scope}){note}", port_label(&port)), &[]));
                }
            }
        }
    }

    fn say(&self, text: &str) {
        self.status.set_label(text);
        self.status.set_visible(!text.is_empty());
    }

    /// Asks first what needs asking, then does `action`.
    fn ask(self: &Rc<Self>, action: Action) {
        let name = tr(&format!("settings-svc-{}", self.service.id));
        let question = match action {
            Action::Remove => Some((
                tr_args("settings-svc-remove-question", &[("name", name.clone().into())]),
                tr("settings-svc-remove-detail"),
                tr("settings-svc-remove"),
            )),
            Action::Off if self.service.warn_off.is_some() => Some((
                tr_args("settings-svc-off-question", &[("name", name.clone().into())]),
                tr(&format!("settings-svc-{}-warn", self.service.id)),
                tr("settings-svc-turn-off"),
            )),
            _ => None,
        };
        let Some((message, detail, yes)) = question else {
            self.act(action);
            return;
        };
        let dialog = gtk::AlertDialog::builder()
            .message(message)
            .detail(detail)
            .buttons([tr("settings-svc-cancel"), yes])
            .cancel_button(0)
            .default_button(0)
            .modal(true)
            .build();
        let window = self.switch.root().and_downcast::<gtk::Window>();
        let row = self.clone();
        dialog.choose(window.as_ref(), gtk::gio::Cancellable::NONE, move |answer| {
            if answer == Ok(1) {
                row.act(action);
            } else {
                row.refresh();
            }
        });
    }

    fn act(self: &Rc<Self>, action: Action) {
        let state = orbit_services::state(self.service);
        let plan = match orbit_services::plan(self.service, &state, action) {
            Ok(plan) => plan,
            Err(e) => {
                self.say(&e);
                self.refresh();
                return;
            }
        };
        self.busy.set(true);
        self.switch.set_sensitive(false);
        self.remove.set_sensitive(false);
        self.say(&tr("settings-svc-working"));
        // Off: the session's program stops before its package goes.
        if plan.session == Some(false)
            && let Err(e) = self.session(false)
        {
            self.done(Some(e));
            return;
        }
        if plan.privileged.is_empty() {
            let error = if plan.session == Some(true) { self.session(true).err() } else { None };
            self.done(error);
            return;
        }
        self.privileged(plan);
    }

    /// The session's `start:` set, and its program started or stopped now.
    fn session(&self, on: bool) -> Result<(), String> {
        let Kind::Session { config, .. } = self.service.kind else {
            return Ok(());
        };
        orbit_services::set_session_flag(config, "start", on).map_err(|e| e.to_string())?;
        orbit_services::session_now(self.service, on).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Runs the helper on the plan's steps, in a thread.
    fn privileged(self: &Rc<Self>, plan: Plan) {
        let (tx, rx) = async_channel::unbounded::<Msg>();
        let (keep_tx, keep_rx) = mpsc::channel::<bool>();
        let steps = plan.privileged.clone();
        let trial = plan.trial;
        std::thread::spawn(move || {
            let mut run = match Run::spawn(&steps, trial) {
                Ok(run) => run,
                Err(e) => {
                    let _ = tx.send_blocking(Msg::Failed(format!("{}: {e}", orbit_services::HELPER)));
                    return;
                }
            };
            let mut failed = false;
            while let Some(event) = run.next_event() {
                let seconds = match &event {
                    Event::Trial(s) => Some(*s),
                    Event::Error(_) | Event::Undone => {
                        failed = true;
                        None
                    }
                    _ => None,
                };
                let _ = tx.send_blocking(Msg::Event(event));
                if let Some(s) = seconds {
                    match keep_rx.recv_timeout(Duration::from_secs(u64::from(s) + 2)) {
                        Ok(true) => run.keep(),
                        Ok(false) => run.undo(),
                        // The helper undoes it by itself.
                        Err(_) => {}
                    }
                }
            }
            let msg = match run.wait() {
                Ok(status) => Msg::Ended {
                    authorised: !matches!(status.code(), Some(126 | 127)),
                    ok: status.success() && !failed,
                },
                Err(e) => Msg::Failed(e.to_string()),
            };
            let _ = tx.send_blocking(msg);
        });
        let row = self.clone();
        glib::spawn_future_local(async move {
            let countdown: Rc<Cell<Option<glib::SourceId>>> = Rc::default();
            while let Ok(msg) = rx.recv().await {
                match msg {
                    Msg::Event(Event::Step(_)) => row.say(&tr("settings-svc-working")),
                    Msg::Event(Event::Progress(p, text)) => row.say(&format!("{p}% {text}")),
                    Msg::Event(Event::Trial(seconds)) => row.trial(seconds, &keep_tx, &countdown),
                    Msg::Event(Event::Kept) => row.keep_bar.set_visible(false),
                    Msg::Event(Event::Undone) => {
                        row.keep_bar.set_visible(false);
                        if let Some(id) = countdown.take() {
                            id.remove();
                        }
                        row.say(&tr("settings-svc-undone"));
                    }
                    Msg::Event(Event::Error(e)) => row.say(&e),
                    Msg::Event(Event::Done | Event::Info(_)) => {}
                    Msg::Ended { authorised, ok } => {
                        if let Some(id) = countdown.take() {
                            id.remove();
                        }
                        row.keep_bar.set_visible(false);
                        let error = if !authorised {
                            Some(tr("settings-svc-cancelled"))
                        } else if ok && plan.session == Some(true) {
                            row.session(true).err()
                        } else if ok {
                            None
                        } else {
                            Some(row.status.label().to_string())
                        };
                        row.done(error);
                        return;
                    }
                    Msg::Failed(e) => {
                        row.done(Some(e));
                        return;
                    }
                }
            }
        });
    }

    /// The bar to keep what was applied on trial, counting down.
    fn trial(self: &Rc<Self>, seconds: u32, keep: &mpsc::Sender<bool>, countdown: &Rc<Cell<Option<glib::SourceId>>>) {
        let left = Rc::new(Cell::new(seconds));
        let show = {
            let label = self.keep_label.clone();
            let left = left.clone();
            move || label.set_label(&tr_args("settings-svc-keep-question", &[("seconds", i64::from(left.get()).into())]))
        };
        show();
        self.keep_bar.set_visible(true);
        self.say("");
        let id = glib::timeout_add_seconds_local(1, move || {
            left.set(left.get().saturating_sub(1));
            show();
            if left.get() == 0 { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
        });
        countdown.set(Some(id));
        for (button, id) in self.trial_handlers.take() {
            button.disconnect(id);
        }
        for (button, answer) in [(&self.keep_button, true), (&self.undo_button, false)] {
            let keep = keep.clone();
            let bar = self.keep_bar.clone();
            let id = button.connect_clicked(move |_| {
                let _ = keep.send(answer);
                bar.set_visible(false);
            });
            self.trial_handlers.borrow_mut().push((button.clone(), id));
        }
    }

    fn done(self: &Rc<Self>, error: Option<String>) {
        self.busy.set(false);
        self.switch.set_sensitive(true);
        self.remove.set_sensitive(true);
        self.say(error.as_deref().unwrap_or_default());
        self.refresh();
    }
}

fn state_text(service: &Service, state: &State) -> String {
    tr(match (&state.installed, state.on, state.running) {
        (None, _, _) => "settings-svc-not-installed",
        (Some(_), true, true) => "settings-svc-on",
        (Some(_), true, false) if matches!(service.kind, Kind::Session { .. }) => "settings-svc-on-next",
        (Some(_), true, false) => "settings-svc-on-idle",
        (Some(_), false, true) => "settings-svc-off-running",
        (Some(_), false, false) => "settings-svc-off",
    })
}
