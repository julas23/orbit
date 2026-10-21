//! Orbit-Core: the compositor of the Orbit desktop, on Smithay. For now
//! (PLANO-CORE Phase 1, its first step) it runs nested — a window inside the
//! current Wayland session — tiles windows side by side, hosts the panel
//! and dock (layer shell), and speaks the clipboard protocols orbit-clip
//! uses.
//!
//!   orbit-core [-c CONFIG] [-- COMMAND…]
//!
//! COMMAND (and the config's `startup:`) run as clients of the core. The
//! plan is `docs/PLANO-CORE.md`.
//!
//! Files that hold code from other projects say so at the top, with the
//! project, file, commit and licence (PLANO-CORE §9).

mod command;
mod config;
mod handlers;
mod input;
mod ipc;
mod state;
mod tile;
mod winit;
mod workspace;

use smithay::reexports::{calloop::EventLoop, wayland_server::Display};

use crate::{config::Config, state::Orbit};

fn main() -> anyhow::Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let mut args = std::env::args().skip(1);
    let mut config_path = Config::path();
    let mut command: Vec<String> = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-c" | "--config" => config_path = args.next().map(Into::into).ok_or_else(|| anyhow::anyhow!("--config needs a file"))?,
            "--" => command = args.by_ref().collect(),
            "-h" | "--help" => {
                println!("Usage: orbit-core [-c|--config FILE] [-- COMMAND…]  (default config: {})", Config::path().display());
                return Ok(());
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    let config = Config::load(&config_path);

    let mut event_loop: EventLoop<Orbit> = EventLoop::try_new()?;
    let display: Display<Orbit> = Display::new()?;
    let mut state = Orbit::new(&mut event_loop, display, config, config_path);
    state.read_layouts();
    winit::init(&mut event_loop, &mut state)?;
    if let Err(err) = ipc::listen(&mut event_loop, &mut state) {
        tracing::error!("no IPC: {err:#}");
    }
    tracing::info!("listening on {:?}", state.socket_name);

    for line in state.config.startup.clone() {
        state.spawn(&line);
    }
    if !command.is_empty() {
        let line = command.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ");
        state.spawn(&line);
    }

    // Replies go out at every turn of the loop: frames are drawn only on
    // change, so they cannot wait for the next one.
    event_loop.run(None, &mut state, |state| {
        let _ = state.display_handle.flush_clients();
        state.ipc.flush();
    })?;
    Ok(())
}

/// `arg` as one word for `sh -c`.
fn shell_quote(arg: &str) -> String {
    if !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c)) {
        arg.to_owned()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}
