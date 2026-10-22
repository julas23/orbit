//! The greet (PLANO-CORE §7.3 step 3): the one client that is shown while
//! the session is locked, and the only one that can unlock it.
//!
//! The core launches it (config `greet:`, through `sh -c`) on a connection
//! of its own — one end of a socket pair, handed down as WAYLAND_SOCKET —
//! so being the greet is having been launched as it, never something a
//! client claims. Only that connection sees the `orbit_greet_v1` global
//! (`orbit-protocols`), whose `unlock` request is the only way out of the
//! lock.
//!
//! The greet shows itself with an ordinary `xdg_toplevel`. Its window is
//! not tiled: locked, it fills the monitor with the pointer and has the
//! keyboard; unlocked, it is hidden and gets nothing. The greet may stay
//! running between locks; at each lock the core launches one if none is
//! running. If it dies while locked the screen stays black and it is
//! launched again, after a wait that grows while it keeps dying quickly.

use std::{
    os::{fd::AsRawFd, unix::net::UnixStream, unix::process::CommandExt},
    sync::Arc,
    time::{Duration, Instant},
};

use orbit_protocols::server::orbit_greet_v1::{self, OrbitGreetV1};
use smithay::{
    backend::renderer::{
        element::{Kind, surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree}},
        gles::GlesRenderer,
    },
    desktop::{PopupManager, Window, WindowSurfaceType},
    output::Output,
    reexports::{
        calloop::{
            RegistrationToken,
            ping::make_ping,
            timer::{TimeoutAction, Timer},
        },
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            Client, DataInit, DisplayHandle, New, Resource,
            backend::{ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{Logical, Point, Scale, SERIAL_COUNTER},
    wayland::{
        Dispatch2, GlobalDispatch2,
        compositor::with_states,
        seat::WaylandFocus,
        shell::xdg::{ToplevelSurface, XdgToplevelSurfaceData},
    },
};

use crate::{
    focus::KeyboardFocus,
    render::OutputElement,
    state::{ClientState, Orbit},
};

/// A greet that dies sooner than this after its launch died quickly: the
/// next launch waits.
const QUICK: Duration = Duration::from_secs(5);
/// The longest wait between launches.
const MAX_WAIT: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct Greet {
    /// The greet's connection, while it is up.
    client: Option<Client>,
    /// Its `orbit_greet_v1` objects.
    resources: Vec<OrbitGreetV1>,
    /// Its window, once it opened one, and the monitor it is placed on.
    pub window: Option<Window>,
    pub output: Option<String>,
    /// When it was launched, how many launches in a row died quickly, and
    /// the launch a timer is waiting to make.
    launched: Option<Instant>,
    failures: u32,
    waiting: Option<RegistrationToken>,
    /// The source that hears its connection close.
    gone: Option<RegistrationToken>,
}

/// The `orbit_greet_v1` global; shown to the greet's connection only.
pub struct GreetGlobal;

/// The data of a bound `orbit_greet_v1`.
pub struct GreetData;

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Orbit, OrbitGreetV1, GreetGlobal>(1, GreetGlobal);
}

/// Whether `client` is the greet the core launched.
fn is_greet_client(client: &Client) -> bool {
    client.get_data::<ClientState>().is_some_and(|c| c.greet.is_some())
}

impl GlobalDispatch2<OrbitGreetV1, Orbit> for GreetGlobal {
    fn bind(&self, state: &mut Orbit, _handle: &DisplayHandle, _client: &Client, resource: New<OrbitGreetV1>, data_init: &mut DataInit<'_, Orbit>) {
        let resource = data_init.init(resource, GreetData);
        if state.greet_locked() {
            resource.locked();
        } else {
            resource.unlocked();
        }
        state.greet.resources.push(resource);
    }

    fn can_view(&self, client: &Client) -> bool {
        is_greet_client(client)
    }
}

impl Dispatch2<OrbitGreetV1, Orbit> for GreetData {
    fn request(&self, state: &mut Orbit, client: &Client, resource: &OrbitGreetV1, request: orbit_greet_v1::Request, _dh: &DisplayHandle, _data_init: &mut DataInit<'_, Orbit>) {
        match request {
            // The global is the greet's alone; checked again all the same.
            orbit_greet_v1::Request::Unlock if is_greet_client(client) => state.unlock(),
            orbit_greet_v1::Request::Unlock => tracing::warn!("unlock from a client that is not the greet"),
            orbit_greet_v1::Request::Destroy => state.greet.resources.retain(|r| r != resource),
            _ => {}
        }
    }

    fn destroyed(&self, state: &mut Orbit, _client: ClientId, resource: &OrbitGreetV1) {
        state.greet.resources.retain(|r| r != resource);
    }
}

impl Orbit {
    /// Launches the greet, unless one is up or a launch is waiting.
    pub fn launch_greet(&mut self) {
        if self.greet.client.is_some() || self.greet.waiting.is_some() {
            return;
        }
        let command = self.config.greet.clone();
        let (ours, theirs) = match UnixStream::pair() {
            Ok(pair) => pair,
            Err(err) => {
                tracing::error!("greet: no socket pair: {err}");
                return;
            }
        };
        // Its connection closing (the greet died or quit) is heard here.
        let (ping, source) = match make_ping() {
            Ok(p) => p,
            Err(err) => {
                tracing::error!("greet: {err}");
                return;
            }
        };
        let gone = self.loop_handle.insert_source(source, |(), &mut (), state| state.greet_gone());
        let gone = match gone {
            Ok(token) => token,
            Err(err) => {
                tracing::error!("greet: {err}");
                return;
            }
        };
        let data = ClientState { greet: Some(ping), ..Default::default() };
        let client = match self.display_handle.insert_client(ours, Arc::new(data)) {
            Ok(c) => c,
            Err(err) => {
                tracing::error!("greet: {err}");
                self.loop_handle.remove(gone);
                return;
            }
        };
        self.greet.client = Some(client.clone());
        self.greet.gone = Some(gone);
        self.greet.launched = Some(Instant::now());

        let fd = theirs.as_raw_fd();
        let mut child = std::process::Command::new("sh");
        child
            .arg("-c")
            .arg(&command)
            .env("WAYLAND_SOCKET", fd.to_string())
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .env("GDK_BACKEND", "wayland")
            .env("XDG_SESSION_TYPE", "wayland")
            .env("XCURSOR_THEME", &self.config.cursor_theme)
            .env("XCURSOR_SIZE", self.config.cursor_size.to_string());
        // Safety: only clears close-on-exec of a descriptor the child
        // inherits; nothing is allocated between fork and exec.
        unsafe {
            child.pre_exec(move || {
                let fd = std::os::fd::BorrowedFd::borrow_raw(fd);
                smithay::reexports::rustix::io::fcntl_setfd(fd, smithay::reexports::rustix::io::FdFlags::empty())?;
                Ok(())
            });
        }
        match child.spawn() {
            Ok(mut c) => {
                tracing::info!("greet launched: {command}");
                std::thread::spawn(move || c.wait());
            }
            Err(err) => {
                tracing::error!("greet {command}: {err}");
                // Its connection goes, and with it comes the next try.
                self.display_handle.backend_handle().kill_client(client.id(), DisconnectReason::ConnectionClosed);
            }
        }
        // The greet's end lives on in the child only.
        drop(theirs);
    }

    /// The greet's connection closed. Locked, another is launched, at
    /// once after a greet that lived, or after a wait that doubles while
    /// they keep dying quickly.
    fn greet_gone(&mut self) {
        if let Some(token) = self.greet.gone.take() {
            self.loop_handle.remove(token);
        }
        self.greet.client = None;
        self.greet.resources.clear();
        self.greet.window = None;
        let quick = self.greet.launched.take().is_some_and(|t| t.elapsed() < QUICK);
        self.greet.failures = if quick { self.greet.failures + 1 } else { 0 };
        if !self.locked() {
            tracing::info!("greet quit");
            return;
        }
        // Nothing of it may linger on the black screen.
        self.request_render();
        let wait = if self.greet.failures == 0 { Duration::ZERO } else { MAX_WAIT.min(Duration::from_secs(1 << self.greet.failures.min(5))) };
        tracing::warn!("greet died while locked; launching it again in {wait:?}");
        let token = self.loop_handle.insert_source(Timer::from_duration(wait), |_, _, state| {
            state.greet.waiting = None;
            if state.locked() {
                state.launch_greet();
            }
            TimeoutAction::Drop
        });
        match token {
            Ok(token) => self.greet.waiting = Some(token),
            Err(err) => tracing::error!("greet: {err}"),
        }
    }

    /// Whether `surface` belongs to the greet.
    pub fn is_greet(&self, surface: &WlSurface) -> bool {
        self.display_handle.get_client(surface.id()).is_ok_and(|c| is_greet_client(&c))
    }

    /// The greet's window, if its main surface is `surface`.
    pub fn greet_window(&self, surface: &WlSurface) -> Option<Window> {
        self.greet.window.clone().filter(|w| w.wl_surface().as_deref() == Some(surface))
    }

    /// A toplevel of the greet: its window, never tiled.
    pub fn new_greet_window(&mut self, surface: ToplevelSurface) {
        if self.greet.window.is_some() {
            tracing::warn!("the greet opened a second window; only the first is shown");
            return;
        }
        surface.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Fullscreen);
            state.states.set(xdg_toplevel::State::Activated);
        });
        self.greet.window = Some(Window::new_wayland_window(surface));
        self.place_greet();
        if self.locked() {
            self.focus_greet();
        }
    }

    /// Fills the monitor with the pointer with the greet's window.
    pub fn place_greet(&mut self) {
        let Some(window) = self.greet.window.clone() else { return };
        let Some(toplevel) = window.toplevel() else { return };
        let Some(output) = self.pointer_output().or_else(|| self.output()) else { return };
        let Some(geo) = self.space.output_geometry(&output) else { return };
        self.greet.output = Some(output.name());
        toplevel.with_pending_state(|state| state.size = Some(geo.size));
        let sent = with_states(toplevel.wl_surface(), |states| {
            states.data_map.get::<XdgToplevelSurfaceData>().is_some_and(|d| d.lock().unwrap().initial_configure_sent)
        });
        if sent {
            toplevel.send_pending_configure();
        }
        self.request_render();
    }

    /// The pointer went to `output` while locked: the greet follows it.
    pub fn greet_follow(&mut self, output: &Output) {
        if self.greet.output.as_deref() != Some(&*output.name()) {
            self.place_greet();
        }
    }

    /// Gives the greet the keyboard and the pointer.
    pub fn focus_greet(&mut self) {
        let Some(surface) = self.greet.window.as_ref().and_then(|w| w.wl_surface()).map(|s| s.into_owned()) else { return };
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, Some(KeyboardFocus::Surface(surface)), SERIAL_COUNTER.next_serial());
        }
        self.refresh_pointer();
    }

    /// Whether the keyboard is the greet's.
    pub fn greet_has_keyboard(&self) -> bool {
        matches!(self.seat.get_keyboard().and_then(|k| k.current_focus()), Some(KeyboardFocus::Surface(s)) if self.is_greet(&s))
    }

    /// The greet's surface under `pos`, and where its origin is.
    pub fn greet_surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        let window = self.greet.window.as_ref()?;
        let output = self.output_named(self.greet.output.as_deref()?)?;
        let out = self.space.output_geometry(&output)?;
        if !out.to_f64().contains(pos) {
            return None;
        }
        let origin = out.loc - window.geometry().loc;
        window.surface_under(pos - origin.to_f64(), WindowSurfaceType::ALL).map(|(s, p)| (s, (p + origin).to_f64()))
    }

    /// Tells the greet the session's state.
    pub fn greet_state(&self) {
        for resource in &self.greet.resources {
            if self.greet_locked() {
                resource.locked();
            } else {
                resource.unlocked();
            }
        }
    }

    /// Lets the greet draw its next frame, when it is shown on `output`.
    pub fn greet_send_frame(&self, output: &Output, time: Duration) {
        if !self.locked() || self.greet.output.as_deref() != Some(&*output.name()) {
            return;
        }
        if let Some(window) = &self.greet.window {
            window.send_frame(output, time, Some(Duration::ZERO), |_, _| Some(output.clone()));
        }
    }
}

/// The greet's window and its menus, when it is shown on `output`, at
/// `alpha` (fading with the eyes).
pub fn elements(state: &Orbit, output: &Output, renderer: &mut GlesRenderer, scale: Scale<f64>, alpha: f32, elements: &mut Vec<OutputElement>) {
    if state.greet.output.as_deref() != Some(&*output.name()) {
        return;
    }
    let Some(window) = &state.greet.window else { return };
    let Some(surface) = window.wl_surface() else { return };
    let geometry = window.geometry();
    let at = (Point::<i32, Logical>::default() - geometry.loc).to_physical_precise_round(scale);
    for (popup, offset) in PopupManager::popups_for_surface(&surface) {
        let offset = (geometry.loc + offset - popup.geometry().loc).to_physical_precise_round(scale);
        elements.extend(
            render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<GlesRenderer>>(renderer, popup.wl_surface(), at + offset, scale, alpha, Kind::Unspecified)
                .into_iter()
                .map(OutputElement::Surface),
        );
    }
    elements.extend(
        render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<GlesRenderer>>(renderer, &surface, at, scale, alpha, Kind::Unspecified)
            .into_iter()
            .map(OutputElement::Surface),
    );
}
