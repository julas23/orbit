//! Orbit-Term: a terminal whose prompt is part of the window, not of the
//! output.
//!
//!   ┌ [date time][{exit}][<<git>>][user@host][dir] ──┬ outputs ┐
//!   │ ➤ command                                      │ ll      │
//!   ├────────────────────────────────────────────────┤ df -h   │
//!   │ the last command's output: never wrapped,      │ …  (50) │
//!   │ scrolled both ways, in its colours             │         │
//!   ├ status: command · exit code · time ────────────┤         │
//!   └────────────────────────────────────────────────┴─────────┘
//!
//! The parts above the command line follow the user's bash prompt
//! (`prompt`); the list of outputs starts as narrow as it goes, a mere
//! index (its edge can be dragged).
//!
//! Commands run in one shell session (`session`): `cd`, `export` and the
//! user's aliases hold from one to the next. Each output replaces the last;
//! the ones that worked are kept, the last 50 listed on the right
//! (`history`), a click showing one again. An interactive program (vim,
//! htop, ssh, sudo…, the `interactive` list of `term.yaml`, or any command
//! run with Ctrl+Enter) runs in a terminal (VTE) that takes the output's
//! place and the keyboard, until it ends.
//!
//! Keys in the prompt: Enter runs, Ctrl+Enter runs as interactive, Tab
//! completes (`complete`; the keyboard never leaves the prompt), ↑ ↓ walk
//! the commands run before (or the completions, when listed), Ctrl+C
//! interrupts (twice: kills), Ctrl+L clears the output.

mod ansi;
mod complete;
mod history;
mod interactive;
mod prompt;
mod session;

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::Instant,
};

use gtk::{gdk, glib, prelude::*};
use orbit_app::i18n::{text, tr, tr_args};
use orbit_common::ConfigDir;
use serde::Deserialize;
use vte4::prelude::*;

use crate::{
    history::History,
    session::{Event, Session},
};

/// `term.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TermConfig {
    /// Pango font description of the output and the prompt.
    pub font: String,
    /// Programs that run in the terminal (their first word); added to the
    /// built-in list.
    pub interactive: Vec<String>,
}

impl Default for TermConfig {
    fn default() -> Self {
        Self { font: "Monospace 11".into(), interactive: Vec::new() }
    }
}

fn main() -> glib::ExitCode {
    orbit_app::run_app(orbit_app::App {
        name: "orbit-term",
        app_id: "io.github.julas23.Orbit.Term",
        css: include_str!("../../../data/term.css"),
        build,
        resident: false,
        open: None,
    })
}

/// The command running, until both its streams have ended.
struct Running {
    command: String,
    started: Instant,
    stdout: Vec<u8>,
    wrote_errors: bool,
    done: Option<(i32, PathBuf)>,
    errors_done: bool,
    /// The second Ctrl+C kills.
    interrupted: bool,
}

/// The completions listed under the prompt.
struct Listed {
    /// Where the completed word starts in the prompt (in characters).
    start: i32,
    candidates: Vec<complete::Candidate>,
    /// The one put in the prompt, if any yet.
    at: Option<usize>,
}

struct Term {
    config: TermConfig,
    state: PathBuf,
    window: gtk::ApplicationWindow,
    cwd_label: gtk::Label,
    clock: gtk::Label,
    exit: gtk::Label,
    git: gtk::Label,
    choices: gtk::Popover,
    choice_list: gtk::ListBox,
    listed: RefCell<Option<Listed>>,
    /// The prompt is being changed by completion, not by typing.
    completing: Cell<bool>,
    commands: Rc<RefCell<(Vec<String>, Option<Instant>)>>,
    entry: gtk::Entry,
    stack: gtk::Stack,
    view: gtk::TextView,
    scroller: gtk::ScrolledWindow,
    terminal: vte4::Terminal,
    status: gtk::Label,
    list: gtk::ListBox,
    writer: RefCell<ansi::Writer>,
    out_reader: RefCell<ansi::Reader>,
    err_reader: RefCell<ansi::Reader>,
    session: RefCell<Option<Session>>,
    /// The session has run its start (the user's files).
    ready: Cell<bool>,
    running: RefCell<Option<Running>>,
    /// An interactive program is running in the terminal.
    in_terminal: RefCell<Option<(String, Instant)>>,
    cwd: RefCell<PathBuf>,
    history: RefCell<History>,
    /// Where ↑ and ↓ are in the commands, and what was typed before.
    browsing: Cell<Option<usize>>,
    typed: RefCell<String>,
}

fn build(app: &gtk::Application, dir: &ConfigDir) -> anyhow::Result<gtk::ApplicationWindow> {
    let config: TermConfig = dir.app("term")?;
    let state = history::state_dir();
    std::fs::create_dir_all(&state)?;
    let font = gtk::pango::FontDescription::from_string(&config.font);

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Orbit-Term")
        .default_width(1100)
        .default_height(680)
        .css_classes(["orbit-term"])
        .build();

    // The prompt: the parts of the user's bash prompt, then the command.
    let part = |class: &str| gtk::Label::builder().css_classes(["part", class]).build();
    let (clock, exit, git) = (part("time"), part("exit"), part("git"));
    let user = part("user");
    user.set_label(&prompt::user_host());
    if prompt::root() {
        user.add_css_class("root");
    }
    let cwd_label = part("dir");
    exit.set_label("{0}");
    git.set_visible(false);
    let parts = gtk::Box::builder().css_classes(["parts"]).spacing(4).build();
    for label in [&clock, &exit, &git, &user, &cwd_label] {
        parts.append(label);
    }
    let arrow = gtk::Label::builder().label("➤").css_classes(["arrow"]).build();
    let entry = gtk::Entry::builder().hexpand(true).css_classes(["command"]).build();
    let line = gtk::Box::builder().spacing(8).build();
    line.append(&arrow);
    line.append(&entry);
    let prompt = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["prompt"]).spacing(6).build();
    prompt.append(&parts);
    prompt.append(&line);

    // The completions, listed under the command line; the keyboard stays
    // in the prompt.
    let choice_list = gtk::ListBox::builder().css_classes(["choices"]).selection_mode(gtk::SelectionMode::Single).build();
    let choices = gtk::Popover::builder()
        .child(
            &gtk::ScrolledWindow::builder()
                .child(&choice_list)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .propagate_natural_height(true)
                .max_content_height(300)
                .min_content_width(220)
                .build(),
        )
        .autohide(false)
        .has_arrow(false)
        .can_focus(false)
        .position(gtk::PositionType::Bottom)
        .halign(gtk::Align::Start)
        .build();
    choices.set_parent(&entry);

    // The output: text, or the terminal of an interactive program.
    let view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::None)
        .left_margin(10)
        .right_margin(10)
        .top_margin(6)
        .bottom_margin(6)
        .css_classes(["output"])
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&view)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .build();
    let terminal = vte4::Terminal::builder().vexpand(true).hexpand(true).build();
    interactive::style(&terminal, &font);
    let stack = gtk::Stack::builder().vexpand(true).build();
    stack.add_named(&scroller, Some("text"));
    stack.add_named(&terminal, Some("terminal"));

    let status = gtk::Label::builder().xalign(0.0).css_classes(["status"]).ellipsize(gtk::pango::EllipsizeMode::End).build();
    let left = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    left.append(&prompt);
    left.append(&stack);
    left.append(&status);

    // The history.
    let list = gtk::ListBox::builder().css_classes(["history"]).selection_mode(gtk::SelectionMode::Single).build();
    let right = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["side"]).build();
    right.append(&gtk::Label::builder().label(tr("term-history")).xalign(0.0).css_classes(["title"]).build());
    right.append(
        &gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build(),
    );
    // The outputs' list starts as narrow as its minimum: past the end, the
    // position stops there.
    let paned = gtk::Paned::builder()
        .start_child(&left)
        .end_child(&right)
        .resize_end_child(false)
        .shrink_end_child(false)
        .position(100_000)
        .build();
    window.set_child(Some(&paned));

    let css = gtk::CssProvider::new();
    css.load_from_string(&format!(
        ".orbit-term .output, .orbit-term .prompt entry, .orbit-term .arrow, .orbit-term .choices {{ font: {}; }}",
        font_css(&font)
    ));
    gtk::style_context_add_provider_for_display(&gtk::prelude::WidgetExt::display(&window), &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()));
    let term = Rc::new(Term {
        config,
        state: state.clone(),
        window: window.clone(),
        cwd_label,
        clock,
        exit,
        git,
        choices,
        choice_list,
        listed: RefCell::new(None),
        completing: Cell::new(false),
        commands: Rc::new(RefCell::new((Vec::new(), None))),
        entry,
        stack,
        writer: RefCell::new(ansi::Writer::new(&view.buffer())),
        view,
        scroller,
        terminal,
        status,
        list,
        out_reader: RefCell::default(),
        err_reader: RefCell::default(),
        session: RefCell::new(None),
        ready: Cell::new(false),
        running: RefCell::new(None),
        in_terminal: RefCell::new(None),
        cwd: RefCell::new(cwd),
        history: RefCell::new(History::load(&state)),
        browsing: Cell::new(None),
        typed: RefCell::default(),
    });
    term.show_cwd();
    term.tick();
    term.refresh_git();
    term.refresh_commands();
    {
        let weak = Rc::downgrade(&term);
        glib::timeout_add_seconds_local(1, move || match weak.upgrade() {
            Some(term) => {
                term.tick();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
    }
    {
        // Typing closes the list of completions.
        let weak = Rc::downgrade(&term);
        term.entry.connect_changed(move |_| {
            if let Some(term) = weak.upgrade()
                && !term.completing.get()
            {
                term.close_choices();
            }
        });
    }
    {
        let weak = Rc::downgrade(&term);
        term.choice_list.connect_row_activated(move |_, row| {
            if let Some(term) = weak.upgrade() {
                term.choose(row.index() as usize);
                term.close_choices();
                term.entry.grab_focus();
            }
        });
    }
    term.fill_list();
    term.start_session();
    {
        // Tab, as an action: what the screenshot test presses.
        let action = gtk::gio::SimpleAction::new("complete", None);
        let weak = Rc::downgrade(&term);
        action.connect_activate(move |_, _| {
            if let Some(term) = weak.upgrade() {
                term.tab(false);
            }
        });
        window.add_action(&action);
    }
    keys(&term);
    {
        let term = term.clone();
        term.entry.clone().connect_activate(move |_| term.submit(false));
    }
    {
        let weak = Rc::downgrade(&term);
        term.list.connect_row_activated(move |_, row| {
            if let Some(term) = weak.upgrade() {
                term.show_output(row.index() as usize);
            }
        });
    }
    {
        let weak = Rc::downgrade(&term);
        term.terminal.connect_child_exited(move |_, status| {
            if let Some(term) = weak.upgrade() {
                term.terminal_done(status);
            }
        });
    }
    // The window holds the terminal state for good.
    let keep = term.clone();
    window.connect_destroy(move |_| {
        keep.session.replace(None);
    });
    term.entry.grab_focus();
    Ok(window)
}

/// A font description as a CSS `font` value.
fn font_css(font: &gtk::pango::FontDescription) -> String {
    let size = f64::from(font.size()) / f64::from(gtk::pango::SCALE);
    let family = font.family().map(|f| f.to_string()).unwrap_or_else(|| "monospace".into());
    format!("{}pt \"{family}\"", if size > 0.0 { size } else { 11.0 })
}

impl Term {
    fn start_session(self: &Rc<Self>) {
        let (tx, rx) = async_channel::unbounded();
        match Session::start(&self.cwd.borrow(), &self.state, tx) {
            Ok(session) => {
                self.session.replace(Some(session));
                self.ready.set(false);
            }
            Err(err) => {
                self.set_status(&format!("{}: {err}", tr("term-no-shell")), true);
                return;
            }
        }
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(event) = rx.recv().await {
                let Some(term) = weak.upgrade() else { return };
                if term.event(event) {
                    return;
                }
            }
        });
    }

    /// Handles a session event; `true` when the session is gone.
    fn event(self: &Rc<Self>, event: Event) -> bool {
        if !self.ready.get() {
            // The start: the user's files may print; nothing of it shows.
            match event {
                Event::Done { cwd, .. } if !cwd.as_os_str().is_empty() => {
                    self.cwd.replace(cwd);
                    self.show_cwd();
                }
                Event::ErrDone => self.ready.set(true),
                Event::Gone => {
                    self.set_status(&tr("term-no-shell"), true);
                    return true;
                }
                _ => {}
            }
            return false;
        }
        match event {
            Event::Out(bytes) => {
                if let Some(run) = self.running.borrow_mut().as_mut() {
                    run.stdout.extend_from_slice(&bytes);
                }
                self.writer.borrow_mut().write(&mut self.out_reader.borrow_mut(), &bytes, false);
                self.follow();
            }
            Event::Err(bytes) => {
                if let Some(run) = self.running.borrow_mut().as_mut() {
                    run.wrote_errors |= !bytes.is_empty();
                }
                self.writer.borrow_mut().write(&mut self.err_reader.borrow_mut(), &bytes, true);
                self.follow();
            }
            Event::Done { code, cwd } => {
                if let Some(run) = self.running.borrow_mut().as_mut() {
                    run.done = Some((code, cwd));
                }
                self.maybe_finish();
            }
            Event::ErrDone => {
                if let Some(run) = self.running.borrow_mut().as_mut() {
                    run.errors_done = true;
                }
                self.maybe_finish();
            }
            Event::Gone => {
                // `exit`, or bash killed: a new session where this one was.
                if let Some(run) = self.running.borrow_mut().as_mut() {
                    run.done.get_or_insert((0, self.cwd.borrow().clone()));
                    run.errors_done = true;
                }
                self.maybe_finish();
                self.session.replace(None);
                self.start_session();
                return true;
            }
        }
        false
    }

    /// Runs what is in the prompt; `force` runs it in the terminal.
    fn submit(self: &Rc<Self>, force: bool) {
        let command = self.entry.text().to_string();
        if command.trim().is_empty() || self.busy() {
            return;
        }
        self.entry.set_text("");
        self.browsing.set(None);
        self.history.borrow_mut().add_command(&command);
        if force || interactive::wanted(&command, &self.config.interactive) {
            self.run_in_terminal(command);
            return;
        }
        if !self.ready.get() {
            self.set_status(&tr("term-starting"), false);
            return;
        }
        self.stack.set_visible_child_name("text");
        self.writer.borrow_mut().clear();
        *self.out_reader.borrow_mut() = ansi::Reader::default();
        *self.err_reader.borrow_mut() = ansi::Reader::default();
        self.list.unselect_all();
        let (cols, rows) = self.grid();
        let sent = match self.session.borrow_mut().as_mut() {
            Some(session) => session.run(&command, cols, rows),
            None => return,
        };
        if let Err(err) = sent {
            self.set_status(&format!("{}: {err}", tr("term-no-shell")), true);
            return;
        }
        self.set_status(&tr_args("term-running", &[("command", text(command.clone()))]), false);
        self.running.replace(Some(Running {
            command,
            started: Instant::now(),
            stdout: Vec::new(),
            wrote_errors: false,
            done: None,
            errors_done: false,
            interrupted: false,
        }));
        self.entry.add_css_class("busy");
    }

    fn busy(&self) -> bool {
        self.running.borrow().is_some() || self.in_terminal.borrow().is_some()
    }

    /// The command is over once both streams have ended.
    fn maybe_finish(self: &Rc<Self>) {
        let ready = self.running.borrow().as_ref().is_some_and(|r| r.done.is_some() && r.errors_done);
        if !ready {
            return;
        }
        let run = self.running.take().expect("running");
        let (code, cwd) = run.done.expect("done");
        if !cwd.as_os_str().is_empty() {
            self.cwd.replace(cwd);
        }
        self.show_cwd();
        self.show_exit(code);
        self.refresh_git();
        self.entry.remove_css_class("busy");
        let seconds = run.started.elapsed().as_secs_f64();
        let failed = !history::worked(code, run.wrote_errors);
        self.set_status(&self.summary(&run.command, code, run.wrote_errors, seconds), failed);
        if !failed && !run.stdout.is_empty() {
            self.history.borrow_mut().add_output(history::Output {
                command: run.command,
                cwd: session::tilde(&self.cwd.borrow()),
                time: now(),
                seconds,
                text: String::from_utf8_lossy(&run.stdout).into_owned(),
            });
            self.fill_list();
        }
        self.entry.grab_focus();
    }

    fn summary(&self, command: &str, code: i32, wrote_errors: bool, seconds: f64) -> String {
        let mut parts = vec![command.to_string(), tr_args("term-exit", &[("code", text(code.to_string()))]), format!("{seconds:.2} s")];
        if wrote_errors {
            parts.push(tr("term-stderr"));
        }
        parts.join("  ·  ")
    }

    fn run_in_terminal(self: &Rc<Self>, command: String) {
        self.stack.set_visible_child_name("terminal");
        self.list.unselect_all();
        let env_file = self.session.borrow().as_ref().map(|s| s.env_file.clone());
        interactive::spawn(&self.terminal, &command, &self.cwd.borrow(), env_file.as_deref());
        self.set_status(&tr_args("term-interactive", &[("command", text(command.clone()))]), false);
        self.in_terminal.replace(Some((command, Instant::now())));
        self.entry.add_css_class("busy");
        // The keyboard goes to the program.
        self.terminal.grab_focus();
    }

    /// The interactive program ended (`status` as from waitpid).
    fn terminal_done(self: &Rc<Self>, status: i32) {
        let Some((command, started)) = self.in_terminal.take() else { return };
        let code = if status & 0x7f == 0 { (status >> 8) & 0xff } else { 128 + (status & 0x7f) };
        self.show_exit(code);
        self.refresh_git();
        self.entry.remove_css_class("busy");
        self.set_status(&self.summary(&command, code, false, started.elapsed().as_secs_f64()), code != 0);
        // What it left on screen stays until the next command.
        self.entry.grab_focus();
    }

    /// Ctrl+C while a command runs.
    fn interrupt(&self) -> bool {
        let mut running = self.running.borrow_mut();
        let Some(run) = running.as_mut() else { return false };
        if let Some(session) = self.session.borrow().as_ref() {
            session.interrupt(run.interrupted);
        }
        run.interrupted = true;
        true
    }

    /// Characters across and lines down the output box shows.
    fn grid(&self) -> (u16, u16) {
        let layout = self.view.create_pango_layout(Some("MMMMMMMMMM"));
        let (w, h) = layout.pixel_size();
        let (cw, ch) = ((f64::from(w) / 10.0).max(1.0), f64::from(h).max(1.0));
        let cols = ((f64::from(self.scroller.width()) - 20.0) / cw).floor().clamp(20.0, 1000.0);
        let rows = (f64::from(self.scroller.height()) / ch).floor().clamp(5.0, 500.0);
        (cols as u16, rows as u16)
    }

    /// Keeps the newest output in sight while it comes.
    fn follow(&self) {
        let buffer = self.view.buffer();
        let mark = buffer.create_mark(None, &buffer.end_iter(), false);
        self.view.scroll_mark_onscreen(&mark);
        buffer.delete_mark(&mark);
    }

    /// The date and time part, as bash's `\d \t`.
    fn tick(&self) {
        if let Some(now) = glib::DateTime::now_local().ok().and_then(|t| t.format("%a %b %d %H:%M:%S").ok()) {
            self.clock.set_label(&now);
        }
    }

    /// The exit part: `{code}`, red when it is not 0.
    fn show_exit(&self, code: i32) {
        self.exit.set_label(&format!("{{{code}}}"));
        if code == 0 {
            self.exit.remove_css_class("error");
        } else {
            self.exit.add_css_class("error");
        }
    }

    /// The git part for the current directory, read off the GTK thread.
    fn refresh_git(self: &Rc<Self>) {
        let cwd = self.cwd.borrow().clone();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let git = gtk::gio::spawn_blocking(move || prompt::git(&cwd)).await.unwrap_or_default();
            let Some(term) = weak.upgrade() else { return };
            term.git.set_label(&format!("<<{git}>>"));
            term.git.set_visible(!git.is_empty());
        });
    }

    /// The commands Tab completes, read again when a minute old.
    fn refresh_commands(&self) {
        let stale = self.commands.borrow().1.is_none_or(|at| at.elapsed().as_secs() > 60);
        if !stale {
            return;
        }
        self.commands.borrow_mut().1 = Some(Instant::now());
        let commands = self.commands.clone();
        glib::spawn_future_local(async move {
            if let Ok(list) = gtk::gio::spawn_blocking(complete::commands).await {
                commands.borrow_mut().0 = list;
            }
        });
    }

    /// Tab (`back`: Shift+Tab): completes the word before the cursor, or
    /// goes through the completions listed.
    fn tab(&self, back: bool) {
        if self.listed.borrow().is_some() {
            self.step_choice(if back { -1 } else { 1 });
            return;
        }
        self.refresh_commands();
        let text = self.entry.text().to_string();
        let cursor = self.entry.position().max(0) as usize;
        let byte = text.char_indices().nth(cursor).map_or(text.len(), |(i, _)| i);
        let before = &text[..byte];
        let (start, candidates) = complete::complete(before, &self.cwd.borrow(), &self.commands.borrow().0);
        let word = &before[start..];
        let start = before[..start].chars().count() as i32;
        match candidates.as_slice() {
            [] => self.entry.error_bell(),
            [one] => {
                let tail = if one.dir { "" } else { " " };
                self.replace_word(start, cursor as i32, &format!("{}{tail}", one.insert));
            }
            _ => {
                let shared = complete::shared(&candidates);
                if shared.len() > word.len() {
                    self.replace_word(start, cursor as i32, &shared);
                } else {
                    self.open_choices(start, candidates);
                }
            }
        }
    }

    /// Puts `text` in the prompt from character `start` to `end`, the
    /// cursor after it.
    fn replace_word(&self, start: i32, end: i32, text: &str) {
        self.completing.set(true);
        self.entry.delete_text(start, end);
        let mut at = start;
        self.entry.insert_text(text, &mut at);
        self.entry.set_position(at);
        self.completing.set(false);
    }

    fn open_choices(&self, start: i32, candidates: Vec<complete::Candidate>) {
        while let Some(row) = self.choice_list.row_at_index(0) {
            self.choice_list.remove(&row);
        }
        for candidate in &candidates {
            let label = gtk::Label::builder().label(&candidate.show).xalign(0.0).build();
            if candidate.dir {
                label.add_css_class("dir");
            }
            self.choice_list.append(&label);
        }
        self.listed.replace(Some(Listed { start, candidates, at: None }));
        // Under the word being completed.
        let before: String = self.entry.text().chars().take(start as usize).collect();
        let x = self.entry.create_pango_layout(Some(&before)).pixel_size().0;
        self.choices.set_pointing_to(Some(&gdk::Rectangle::new(x, self.entry.height(), 1, 1)));
        self.choices.popup();
    }

    /// The next (or previous) completion, put in the prompt.
    fn step_choice(&self, step: i32) {
        let (n, at) = match self.listed.borrow().as_ref() {
            Some(l) => (l.candidates.len() as i32, l.at),
            None => return,
        };
        let next = match at {
            None if step > 0 => 0,
            None => n - 1,
            Some(i) => (i as i32 + step).rem_euclid(n),
        };
        self.choose(next as usize);
    }

    /// Puts completion `i` in the prompt, the list staying open.
    fn choose(&self, i: usize) {
        let (start, insert) = {
            let listed = self.listed.borrow();
            let Some(l) = listed.as_ref() else { return };
            let Some(c) = l.candidates.get(i) else { return };
            (l.start, c.insert.clone())
        };
        self.replace_word(start, self.entry.position(), &insert);
        if let Some(l) = self.listed.borrow_mut().as_mut() {
            l.at = Some(i);
        }
        if let Some(row) = self.choice_list.row_at_index(i as i32) {
            self.choice_list.select_row(Some(&row));
        }
    }

    /// Enter with the list open: keeps the completion chosen, with the space
    /// a file takes after it.
    fn accept_choice(&self) {
        let chosen = self.listed.borrow().as_ref().and_then(|l| l.at.and_then(|i| l.candidates.get(i).cloned()));
        self.close_choices();
        if let Some(c) = chosen
            && !c.dir
        {
            let mut at = self.entry.position();
            self.completing.set(true);
            self.entry.insert_text(" ", &mut at);
            self.entry.set_position(at);
            self.completing.set(false);
        }
    }

    fn close_choices(&self) {
        if self.listed.take().is_some() {
            self.choices.popdown();
        }
    }

    fn show_cwd(&self) {
        let cwd = session::tilde(&self.cwd.borrow());
        self.cwd_label.set_label(&cwd);
        self.window.set_title(Some(&format!("{cwd} — Orbit-Term")));
    }

    fn set_status(&self, message: &str, error: bool) {
        self.status.set_label(message);
        if error {
            self.status.add_css_class("error");
        } else {
            self.status.remove_css_class("error");
        }
    }

    /// The list of kept outputs, newest first.
    fn fill_list(&self) {
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        for output in &self.history.borrow().outputs {
            let row = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["entry"]).build();
            row.append(
                &gtk::Label::builder()
                    .label(&output.command)
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .css_classes(["command"])
                    .build(),
            );
            row.append(
                &gtk::Label::builder()
                    .label(format!("{}  ·  {}", output.time, output.cwd))
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::Middle)
                    .css_classes(["when"])
                    .build(),
            );
            row.set_tooltip_text(Some(&output.command));
            self.list.append(&row);
        }
    }

    /// Shows kept output `i` in the output box.
    fn show_output(&self, i: usize) {
        if self.busy() {
            return;
        }
        let Some(output) = self.history.borrow().outputs.get(i).cloned() else { return };
        self.stack.set_visible_child_name("text");
        let mut writer = self.writer.borrow_mut();
        writer.clear();
        writer.write(&mut ansi::Reader::default(), output.text.as_bytes(), false);
        drop(writer);
        self.view.buffer().place_cursor(&self.view.buffer().start_iter());
        self.scroller.vadjustment().set_value(0.0);
        self.set_status(
            &tr_args(
                "term-from-history",
                &[("command", text(output.command)), ("time", text(output.time)), ("cwd", text(output.cwd))],
            ),
            false,
        );
    }

    /// ↑ (`back`) or ↓ through the commands run before.
    fn browse(&self, back: bool) {
        let history = self.history.borrow();
        let n = history.commands.len();
        if n == 0 {
            return;
        }
        let at = match (self.browsing.get(), back) {
            (None, true) => {
                self.typed.replace(self.entry.text().to_string());
                Some(n - 1)
            }
            (None, false) => return,
            (Some(i), true) => Some(i.saturating_sub(1)),
            (Some(i), false) if i + 1 < n => Some(i + 1),
            (Some(_), false) => None,
        };
        self.browsing.set(at);
        let text = match at {
            Some(i) => history.commands[i].clone(),
            None => self.typed.borrow().clone(),
        };
        self.entry.set_text(&text);
        self.entry.set_position(-1);
    }
}

fn keys(term: &Rc<Term>) {
    let keys = gtk::EventControllerKey::new();
    // Before the entry handles them (↑ ↓ would move the focus).
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(term);
    keys.connect_key_pressed(move |_, key, _, mods| {
        let Some(term) = weak.upgrade() else { return glib::Propagation::Proceed };
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let listing = term.listed.borrow().is_some();
        match key {
            // Tab never takes the keyboard out of the prompt.
            gdk::Key::Tab | gdk::Key::KP_Tab => term.tab(false),
            gdk::Key::ISO_Left_Tab => term.tab(true),
            gdk::Key::Up if listing => term.step_choice(-1),
            gdk::Key::Down if listing => term.step_choice(1),
            gdk::Key::Return | gdk::Key::KP_Enter if listing => term.accept_choice(),
            gdk::Key::Escape if listing => term.close_choices(),
            gdk::Key::Up => term.browse(true),
            gdk::Key::Down => term.browse(false),
            gdk::Key::Return | gdk::Key::KP_Enter if ctrl => term.submit(true),
            gdk::Key::c | gdk::Key::C if ctrl && term.entry.selection_bounds().is_none() => {
                if !term.interrupt() {
                    return glib::Propagation::Proceed;
                }
            }
            gdk::Key::l | gdk::Key::L if ctrl => {
                if !term.busy() {
                    term.writer.borrow_mut().clear();
                    term.stack.set_visible_child_name("text");
                }
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    term.entry.add_controller(keys);
}

/// Local time, `2026-10-21 18:02`.
fn now() -> String {
    glib::DateTime::now_local()
        .and_then(|t| t.format("%Y-%m-%d %H:%M"))
        .map(|s| s.to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn pump(ms: u64) {
        let until = Instant::now() + std::time::Duration::from_millis(ms);
        while Instant::now() < until {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn shot(window: &gtk::ApplicationWindow, path: &str) {
        let paintable = gtk::WidgetPaintable::new(Some(window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, f64::from(window.width()), f64::from(window.height()));
        match snapshot.to_node() {
            Some(node) => window.renderer().unwrap().render_texture(&node, None).save_to_png(path).unwrap(),
            None => eprintln!("{path}: nothing drawn"),
        }
    }

    /// Runs commands in a window and saves what it looks like to
    /// `$ORBIT_TERM_LOOK-N.png`. On a display of its own, and with a state
    /// directory of its own (the outputs it keeps are not the user's):
    /// `gtk4-broadwayd :5 &`, then `GDK_BACKEND=broadway
    /// BROADWAY_DISPLAY=:5 XDG_STATE_HOME=/tmp/s ORBIT_TERM_LOOK=/tmp/t
    /// cargo test -p orbit-term look -- --ignored`.
    #[test]
    #[ignore]
    fn look() {
        let Ok(base) = std::env::var("ORBIT_TERM_LOOK") else { return };
        assert!(std::env::var_os("XDG_STATE_HOME").is_some(), "give the test a state directory of its own");
        gtk::init().unwrap();
        let display = gdk::Display::default().unwrap();
        let _theme = orbit_app::theme::install(&display);
        let css = gtk::CssProvider::new();
        css.load_from_string(&format!("{}\n{}", include_str!("../../../data/style.css"), include_str!("../../../data/term.css")));
        gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        let app = gtk::Application::new(None::<&str>, Default::default());
        let window = build(&app, &ConfigDir::new(Path::new("/nonexistent"))).unwrap();
        window.present();
        pump(1500);
        let entry = gtk::prelude::GtkWindowExt::focus(&window).and_then(|w| w.downcast::<gtk::Text>().ok()).map(|t| t.parent().unwrap().downcast::<gtk::Entry>().unwrap());
        let entry = entry.expect("the prompt has the keyboard");
        let mut n = 0;
        let mut run = |cmd: &str, wait: u64| {
            entry.set_text(cmd);
            entry.emit_activate();
            pump(wait);
            n += 1;
            shot(&window, &format!("{base}-{n}.png"));
        };
        run("cd ~/Vault/GIT/orbit && ll", 2500);
        run("ls /nonexistent", 1200);
        // Completion: "git ch" + Tab puts in "che"; Tab again lists.
        entry.set_text("git ch");
        entry.set_position(-1);
        pump(3000);
        let _ = WidgetExt::activate_action(&window, "win.complete", None);
        pump(200);
        assert_eq!(entry.text(), "git che");
        let _ = WidgetExt::activate_action(&window, "win.complete", None);
        pump(800);
        shot(&window, &format!("{base}-tab.png"));
        eprintln!("prompt: {:?}", entry.text());
        let mut child = entry.first_child();
        while let Some(widget) = child {
            if let Some(popover) = widget.downcast_ref::<gtk::Popover>() {
                let paintable = gtk::WidgetPaintable::new(Some(popover));
                let snapshot = gtk::Snapshot::new();
                paintable.snapshot(&snapshot, f64::from(popover.width()), f64::from(popover.height()));
                if let Some(node) = snapshot.to_node() {
                    window.renderer().unwrap().render_texture(&node, None).save_to_png(format!("{base}-choices.png")).unwrap();
                }
            }
            child = widget.next_sibling();
        }
    }
}
