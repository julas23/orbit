//! orbitctl: commands and queries to Orbit-Core, as `swaymsg` is to sway
//! (and it talks to sway too). Every action of the core is a command, so
//! shortcuts and scripts are just this (PLANO-CORE §3, bspwm's `bspc`).
//!
//!   orbitctl workspace number 3
//!   orbitctl -t get_tree
//!   orbitctl -t subscribe -m '["window"]'
//!
//! `-t TYPE` picks the message (`command` by default), `-r` prints the raw
//! JSON reply, `-m` keeps printing the events subscribed to.

use std::process::ExitCode;

use orbit_ipc::{Connection, RUN_COMMAND, SUBSCRIBE, message_type};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut kind = RUN_COMMAND;
    let (mut raw, mut monitor) = (false, false);
    let mut words: Vec<String> = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-t" | "--type" => {
                let Some(name) = args.next() else { return usage("-t needs a type") };
                match message_type(&name) {
                    Some(t) => kind = t,
                    None => return usage(&format!("unknown type: {name}")),
                }
            }
            "-r" | "--raw" => raw = true,
            "-m" | "--monitor" => monitor = true,
            "-h" | "--help" => {
                println!("Usage: orbitctl [-t TYPE] [-r] [-m] [MESSAGE…]");
                return ExitCode::SUCCESS;
            }
            "--" => words.extend(args.by_ref()),
            _ => words.push(arg),
        }
    }
    let payload = words.join(" ");
    let mut conn = match Connection::open() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("orbitctl: {err}");
            return ExitCode::FAILURE;
        }
    };
    let reply = match conn.request(kind, &payload) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("orbitctl: {err}");
            return ExitCode::FAILURE;
        }
    };
    let ok = print(&reply, raw, kind);
    if kind == SUBSCRIBE && monitor {
        loop {
            match conn.receive() {
                Ok((_, body)) => {
                    print(&body, raw, 0);
                }
                Err(err) => {
                    eprintln!("orbitctl: {err}");
                    return ExitCode::FAILURE;
                }
            }
        }
    }
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// Prints a reply; whether it reports success (a command's every part).
fn print(body: &[u8], raw: bool, kind: u32) -> bool {
    let value: serde_json::Value = serde_json::from_slice(body).unwrap_or(serde_json::Value::Null);
    let failures: Vec<&str> = match &value {
        serde_json::Value::Array(results) if kind == RUN_COMMAND => results
            .iter()
            .filter(|r| r["success"] == false)
            .map(|r| r["error"].as_str().unwrap_or("failed"))
            .collect(),
        _ => Vec::new(),
    };
    if raw {
        println!("{}", String::from_utf8_lossy(body));
    } else if kind == RUN_COMMAND {
        for error in &failures {
            eprintln!("Error: {error}");
        }
    } else {
        println!("{}", serde_json::to_string_pretty(&value).unwrap_or_default());
    }
    failures.is_empty() && value["success"] != false
}

fn usage(error: &str) -> ExitCode {
    eprintln!("orbitctl: {error}\nUsage: orbitctl [-t TYPE] [-r] [-m] [MESSAGE…]");
    ExitCode::from(2)
}
