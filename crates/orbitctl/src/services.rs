//! `orbitctl services`: Orbit-Settings' Services page in a terminal (for a
//! machine without a session, over SSH, or a script).
//!
//!   orbitctl services [--json]                 every service and its state
//!   orbitctl services on|off|remove ID [--yes] turn one on, off, or away
//!
//! What needs root goes through orbit-services-helper by pkexec, which asks
//! for the password here. A change that could cut the machine off (the
//! firewall) is kept only when confirmed within 30 s; `--yes` answers the
//! questions turning a service off asks, not that one.

use std::{
    io::{IsTerminal, Write},
    process::ExitCode,
    sync::mpsc,
    thread,
    time::Duration,
};

use orbit_services::{Action, Event, Group, Kind, Run, SERVICES, Scope};

pub fn main(args: &[String]) -> ExitCode {
    let json = args.iter().any(|a| a == "--json");
    let yes = args.iter().any(|a| a == "--yes" || a == "-y");
    let words: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| !a.starts_with('-'))
        .collect();
    match words.as_slice() {
        [] => list(json),
        [verb, id] => match Action::parse(verb) {
            Some(action) => act(action, id, yes),
            None => usage(&format!("unknown action: {verb}")),
        },
        _ => usage("a service is turned on, off or away one at a time"),
    }
}

fn usage(error: &str) -> ExitCode {
    eprintln!("orbitctl services: {error}\nUsage: orbitctl services [--json] | on|off|remove ID [--yes]");
    ExitCode::from(2)
}

fn list(json: bool) -> ExitCode {
    let rows = orbit_services::rows();
    if json {
        println!("{}", serde_json::to_string_pretty(&rows).unwrap_or_default());
        return ExitCode::SUCCESS;
    }
    for group in Group::ALL {
        let in_group: Vec<_> = rows.iter().filter(|r| r.group == group).collect();
        if in_group.is_empty() {
            continue;
        }
        println!("{}", group.title());
        for row in in_group {
            let state = match (&row.state.installed, row.state.on, row.state.running) {
                (None, _, _) => "not installed",
                (Some(_), true, true) => "on",
                (Some(_), true, false) => "on, not running",
                (Some(_), false, true) => "off, running",
                (Some(_), false, false) => "off",
            };
            let opens = if row.id == "firewall" {
                let files = orbit_services::firewall_files();
                let rules: usize = files.iter().map(|(_, p)| p.len()).sum();
                format!("{rules} openings in {} files (sudo orbit-firewall status)", files.len())
            } else {
                opens(&row.ports)
            };
            println!("  {:<10} {:<22} {:<16} {}", row.id, row.name, state, opens);
        }
    }
    ExitCode::SUCCESS
}

fn opens(ports: &[orbit_services::Port]) -> String {
    if ports.is_empty() {
        return String::new();
    }
    let lan: Vec<String> = ports.iter().filter(|p| p.scope == Scope::Lan).map(|p| p.label()).collect();
    let any: Vec<String> = ports.iter().filter(|p| p.scope == Scope::Anyone).map(|p| p.label()).collect();
    let mut parts = Vec::new();
    if !lan.is_empty() {
        parts.push(format!("{} (local network)", lan.join(", ")));
    }
    if !any.is_empty() {
        parts.push(format!("{} (anyone)", any.join(", ")));
    }
    format!("opens {}", parts.join("; "))
}

fn act(action: Action, id: &str, yes: bool) -> ExitCode {
    let Some(service) = orbit_services::find(id) else {
        let ids: Vec<&str> = SERVICES.iter().map(|s| s.id).collect();
        return usage(&format!("no service {id} (there are: {})", ids.join(", ")));
    };
    let state = orbit_services::state(service);
    let plan = match orbit_services::plan(service, &state, action) {
        Ok(plan) => plan,
        Err(e) => {
            eprintln!("orbitctl services: {e}");
            return ExitCode::FAILURE;
        }
    };
    if action != Action::On
        && let Some(warning) = service.warn_off
        && !yes
        && !ask(&format!("{warning} Go on?"))
    {
        return ExitCode::FAILURE;
    }
    // Off: the session's program stops before its package goes.
    if plan.session == Some(false) && !session(service, false) {
        return ExitCode::FAILURE;
    }
    if !plan.privileged.is_empty() && !privileged(&plan.privileged, plan.trial) {
        return ExitCode::FAILURE;
    }
    if plan.session == Some(true) && !session(service, true) {
        return ExitCode::FAILURE;
    }
    let now = orbit_services::state(service);
    println!(
        "{}: {}",
        service.name,
        match (&now.installed, now.on, now.running) {
            (None, _, _) => "not installed",
            (Some(_), true, true) => "on",
            (Some(_), true, false) if matches!(service.kind, Kind::Session { .. }) => "on (starts with the next session)",
            (Some(_), true, false) => "on, but not running",
            (Some(_), false, _) => "off",
        }
    );
    ExitCode::SUCCESS
}

/// The session's `start:` set, and the program started or stopped now.
fn session(service: &orbit_services::Service, on: bool) -> bool {
    let Kind::Session { config, .. } = service.kind else {
        return true;
    };
    if let Err(e) = orbit_services::set_session_flag(config, "start", on) {
        eprintln!("orbitctl services: {}: {e}", orbit_services::user_config(config).display());
        return false;
    }
    if let Err(e) = orbit_services::session_now(service, on) {
        eprintln!("orbitctl services: {e}");
        return false;
    }
    true
}

/// Runs the helper on `steps`, saying what it does; whether it worked.
fn privileged(steps: &[orbit_services::Privileged], trial: bool) -> bool {
    let mut run = match Run::spawn(steps, trial) {
        Ok(run) => run,
        Err(e) => {
            eprintln!("orbitctl services: {}: {e}", orbit_services::HELPER);
            return false;
        }
    };
    let mut failed = false;
    let mut last = u8::MAX;
    while let Some(event) = run.next_event() {
        match event {
            Event::Step(text) => println!("· {text}"),
            Event::Progress(p, text) => {
                if p / 10 != last / 10 {
                    println!("  {p:>3}% {text}");
                    last = p;
                }
            }
            Event::Trial(seconds) => match confirm_within(seconds) {
                Some(true) => run.keep(),
                Some(false) => run.undo(),
                None => println!("Not confirmed: undoing it."),
            },
            Event::Kept => println!("Kept."),
            Event::Undone => {
                println!("Undone.");
                failed = true;
            }
            Event::Error(e) => {
                eprintln!("Error: {e}");
                failed = true;
            }
            Event::Done | Event::Info(_) => {}
        }
    }
    match run.wait() {
        Ok(status) if status.code() == Some(126) || status.code() == Some(127) => {
            eprintln!("orbitctl services: not authorised");
            false
        }
        Ok(status) => status.success() && !failed,
        Err(e) => {
            eprintln!("orbitctl services: {e}");
            false
        }
    }
}

/// A yes/no question; no answer (not a terminal) is no.
fn ask(question: &str) -> bool {
    if !std::io::stdin().is_terminal() {
        eprintln!("{question} (answer with --yes)");
        return false;
    }
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    let _ = std::io::stdin().read_line(&mut answer);
    matches!(answer.trim(), "y" | "Y" | "yes" | "s" | "S" | "sim")
}

/// Whether the user keeps what was just applied (`Some(true)`), says no
/// (`Some(false)`), or does not answer within `seconds` (`None`).
fn confirm_within(seconds: u32) -> Option<bool> {
    if !std::io::stdin().is_terminal() {
        eprintln!("Applied on trial, but nobody here to keep it.");
        return Some(false);
    }
    print!("Applied. Keep it? [y/N] (undone in {seconds} s) ");
    let _ = std::io::stdout().flush();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut answer = String::new();
        let _ = std::io::stdin().read_line(&mut answer);
        let _ = tx.send(matches!(answer.trim(), "y" | "Y" | "yes" | "s" | "S" | "sim"));
    });
    let answer = rx.recv_timeout(Duration::from_secs(seconds.into())).ok();
    if answer.is_none() {
        println!();
    }
    answer
}
