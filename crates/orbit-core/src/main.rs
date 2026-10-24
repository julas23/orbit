//! Orbit-Core: the compositor of the Orbit desktop, on Smithay. For now
//! (PLANO-CORE Phase 1, its first step) it runs nested — a window inside the
//! current Wayland session — tiles windows side by side, hosts the panel
//! and dock (layer shell), speaks the clipboard protocols orbit-clip
//! uses, and runs X programs through its own XWayland (`xwayland`).
//!
//!   orbit-core [-c CONFIG] [--nested | --drm] [--locked] [--startup] [-- COMMAND…]
//!
//! Inside a Wayland or X session it runs nested, as a window; from a text
//! console it takes the screen (DRM/KMS, libinput, libseat), and logs to
//! `$XDG_STATE_HOME/orbit/core/core.log`. Ctrl+Alt+F1…F12 switch consoles,
//! Mod+L locks (and so do `idle_lock` seconds of no input, `idle`, and
//! the machine going to sleep, `sleep`),
//! Mod+Shift+E leaves.
//!
//! `--locked` starts with the session hidden behind the lock (the login,
//! PLANO-CORE §7), with the greet (`greet`) launched; the config's
//! `login:` waits for the first unlock. COMMAND (and the config's
//! `startup:`) run as clients of the core. Nested, `startup:` and `login:`
//! are left out unless `--startup` asks for them: they are the session's
//! own (the default ones hand the display to systemd and D-Bus, and
//! `orbit-session` takes the panel and dock of the session around it).
//! The plan is `docs/PLANO-CORE.md`.
//!
//! Files that hold code from other projects say so at the top, with the
//! project, file, commit and licence (PLANO-CORE §9).

mod cards;
mod command;
mod config;
mod cursor;
mod direction;
mod eyes;
mod focus;
mod floating;
mod fullscreen;
mod grab;
mod greet;
mod session_lock;
mod handlers;
mod idle;
mod input;
mod ipc;
mod keys;
mod lock;
mod output;
mod render;
mod resize;
mod rounded;
mod screencopy;
mod screensaver;
mod sleep;
mod state;
mod switcher;
mod tile;
mod udev;
mod virtual_output;
mod watchdog;
mod winit;
mod workspace;
mod xwayland;

use smithay::reexports::{calloop::EventLoop, wayland_server::Display};

use crate::{config::Config, state::Orbit};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    // Nested when there is a session to nest in.
    let mut drm = std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_none();
    let mut config_path = Config::path();
    let mut command: Vec<String> = Vec::new();
    let mut locked = false;
    let mut startup = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-c" | "--config" => config_path = args.next().map(Into::into).ok_or_else(|| anyhow::anyhow!("--config needs a file"))?,
            "--" => command = args.by_ref().collect(),
            "--nested" => drm = false,
            "--drm" => drm = true,
            "--locked" => locked = true,
            "--startup" => startup = Some(true),
            "-h" | "--help" => {
                println!("Usage: orbit-core [-c|--config FILE] [--nested|--drm] [--locked] [--startup] [-- COMMAND…]  (default config: {})", Config::path().display());
                return Ok(());
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    logging(drm);
    let config = Config::load(&config_path);

    let mut event_loop: EventLoop<Orbit> = EventLoop::try_new()?;
    let display: Display<Orbit> = Display::new()?;
    let mut state = Orbit::new(&mut event_loop, display, config, config_path);
    // Before the loop runs, so before any client is let in.
    if locked {
        state.lock();
    }
    state.read_layouts();
    if drm {
        udev::init(&mut event_loop, &mut state)?;
    } else {
        winit::init(&mut event_loop, &mut state)?;
    }
    state.arm_idle();
    // Nested, the host session's own screensaver has the name.
    if drm {
        state.serve_screensaver();
        state.lock_before_sleep();
        watchdog::start(&mut event_loop);
    }
    if let Err(err) = ipc::listen(&mut event_loop, &mut state) {
        tracing::error!("no IPC: {err:#}");
    }
    tracing::info!("listening on {:?}", state.socket_name);
    // Before any program starts, so each gets DISPLAY.
    state.start_xwayland();
    if drm {
        state.spawn(REFRESH_PORTALS);
    }

    state.startup = startup.unwrap_or(drm);
    if !state.startup {
        tracing::info!("nested: the config's startup and login left out (--startup runs them)");
    }
    for line in state.config.startup.clone().into_iter().filter(|_| state.startup) {
        state.spawn(&line);
    }
    if !state.locked() {
        state.log_in();
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

/// The user's systemd and D-Bus outlive a core (Ctrl+Alt+Backspace, a
/// crash): the portals still running hold the dead core's display, and the
/// one D-Bus restarts meanwhile fails on it while D-Bus waits two minutes
/// for its name. Every GTK program asks the portal for its settings at
/// start and waits too, the greet first. So the new display goes to
/// systemd, and the portals left over restart on it.
const REFRESH_PORTALS: &str = "dbus-update-activation-environment --systemd WAYLAND_DISPLAY DISPLAY ORBIT_SOCK XDG_CURRENT_DESKTOP XDG_SESSION_TYPE; \
    units=$(systemctl --user list-units --all --plain --no-legend --state=active,activating,failed 'xdg-desktop-portal*.service' | cut -d' ' -f1); \
    [ -z \"$units\" ] || systemctl --user restart $units";

/// Logs to stderr nested; on the screen, to a file (the console is hidden
/// behind it), `$XDG_STATE_HOME/orbit/core/core.log`, a new one each start
/// (the one before in `core.log.prev`).
fn logging(drm: bool) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    if drm {
        let dir = crate::config::state_dir();
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::rename(dir.join("core.log"), dir.join("core.log.prev"));
        if let Ok(file) = std::fs::File::create(dir.join("core.log")) {
            tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(std::sync::Mutex::new(file)).init();
            return;
        }
    }
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// `arg` as one word for `sh -c`.
fn shell_quote(arg: &str) -> String {
    if !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c)) {
        arg.to_owned()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}
