//! The compositor's state: the Wayland protocols it speaks, the windows
//! and the order they are tiled in, the layer surfaces (panel, dock,
//! notifications), focus, and when to draw.
//!
//! Ported from Smithay's smallvil `src/state.rs` @ 24647fc (MIT, Copyright
//! (c) 2017 Victor Berger and Victoria Brekenfeld), with the layer shell,
//! decorations and selections of anvil `src/state.rs` @ 24647fc (same
//! licence), and drawing on demand instead of every frame.

use std::{ffi::OsString, sync::Arc, time::Instant};

use smithay::{
    desktop::{LayerSurface, PopupManager, Space, Window, WindowSurfaceType, layer_map_for_output},
    input::{Seat, SeatState, keyboard::XkbConfig},
    output::Output,
    reexports::{
        calloop::{EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction, generic::Generic},
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            Display, DisplayHandle,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        output::OutputManagerState,
        selection::{
            data_device::DataDeviceState, primary_selection::PrimarySelectionState,
            wlr_data_control::DataControlState,
        },
        shell::{
            wlr_layer::{KeyboardInteractivity, Layer as WlrLayer, WlrLayerShellState},
            xdg::{XdgShellState, decoration::XdgDecorationState},
        },
        shm::ShmState,
        socket::ListeningSocketSource,
        viewporter::ViewporterState,
    },
};

use crate::{
    config::Config,
    ipc::Ipc,
    tile,
    workspace::{Workspace, info},
};

pub struct Orbit {
    pub start_time: Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, Orbit>,
    pub loop_signal: LoopSignal,
    pub config: Config,
    /// Where the config was read from, for `reload`.
    pub config_path: std::path::PathBuf,

    /// What is on screen: the shown workspace's windows (the space keeps
    /// the stacking order) and the layer surfaces.
    pub space: Space<Window>,
    /// The workspaces that exist, in number order (see `workspace`).
    pub workspaces: Vec<Workspace>,
    /// The number of the one shown, and of the one before
    /// (`workspace back_and_forth`).
    pub current: i32,
    pub previous_ws: Option<i32>,
    /// Windows off every workspace (the panel's "minimise").
    pub scratchpad: Vec<Window>,
    /// The id the next window gets (sway's `con_id`).
    pub next_window_id: u64,
    pub popups: PopupManager,
    /// The sway-compatible IPC: its socket and clients.
    pub ipc: Ipc,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub layer_shell_state: WlrLayerShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Orbit>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub data_control_state: DataControlState,
    pub seat: Seat<Self>,
    /// Globals with nothing to ask of the state: held so they stay
    /// advertised.
    _globals: (XdgDecorationState, OutputManagerState, ViewporterState),

    /// The nested window this compositor draws into.
    pub winit: Option<crate::winit::WinitData>,
    /// Something changed on screen since the last frame.
    pub needs_render: bool,
    /// The keyboard's layouts by name, and the active one (for the IPC).
    pub layouts: (Vec<String>, usize),
    /// Keys a shortcut took: their release is not sent to clients.
    pub suppressed: Vec<smithay::input::keyboard::Keycode>,
}

impl Orbit {
    pub fn new(event_loop: &mut EventLoop<'static, Self>, display: Display<Self>, config: Config, config_path: std::path::PathBuf) -> Self {
        let dh = display.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        // orbit-clip reads and sets the clipboard through it.
        let data_control_state = DataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_| true);
        let viewporter_state = ViewporterState::new::<Self>(&dh);

        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "seat0");
        let kb = &config.keyboard;
        if let Err(err) = seat.add_keyboard(xkb_config(kb), kb.repeat_delay, kb.repeat_rate) {
            tracing::error!("keyboard {}/{}: {err:?}; using us", kb.layout, kb.variant);
            seat.add_keyboard(XkbConfig::default(), kb.repeat_delay, kb.repeat_rate).expect("the default keymap");
        }
        seat.add_pointer();

        let socket_name = Self::listen(display, event_loop);

        Self {
            start_time: Instant::now(),
            socket_name,
            display_handle: dh,
            loop_handle: event_loop.handle(),
            loop_signal: event_loop.get_signal(),
            config,
            config_path,
            space: Space::default(),
            workspaces: vec![Workspace { num: 1, windows: Vec::new(), last_focus: None }],
            current: 1,
            previous_ws: None,
            scratchpad: Vec::new(),
            next_window_id: 100,
            popups: PopupManager::default(),
            ipc: Ipc::default(),
            compositor_state,
            xdg_shell_state,
            layer_shell_state,
            shm_state,
            seat_state,
            data_device_state,
            primary_selection_state,
            data_control_state,
            seat,
            _globals: (xdg_decoration_state, output_manager_state, viewporter_state),
            winit: None,
            needs_render: true,
            suppressed: Vec::new(),
            layouts: (Vec::new(), 0),
        }
    }

    /// The Wayland socket clients connect to, and the display's own events.
    fn listen(display: Display<Self>, event_loop: &mut EventLoop<'static, Self>) -> OsString {
        let socket = ListeningSocketSource::new_auto().expect("a Wayland socket");
        let name = socket.socket_name().to_os_string();
        let handle = event_loop.handle();
        handle
            .insert_source(socket, |stream, _, state| {
                if let Err(err) = state.display_handle.insert_client(stream, Arc::new(ClientState::default())) {
                    tracing::warn!("client refused: {err}");
                }
            })
            .expect("the socket in the event loop");
        handle
            .insert_source(Generic::new(display, Interest::READ, Mode::Level), |_, display, state| {
                // Safety: the display is not dropped while the loop runs.
                unsafe {
                    display.get_mut().dispatch_clients(state).unwrap();
                }
                Ok(PostAction::Continue)
            })
            .expect("the display in the event loop");
        name
    }

    /// Reads the config again (`reload`): keyboard, gaps, borders and
    /// colours apply at once; a bad file keeps the defaults (§1.5).
    pub fn reload_config(&mut self) {
        self.config = Config::load(&self.config_path);
        if let Some(keyboard) = self.seat.get_keyboard() {
            let kb = self.config.keyboard.clone();
            if let Err(err) = keyboard.set_xkb_config(self, xkb_config(&kb)) {
                tracing::error!("keyboard {}/{}: {err:?}", kb.layout, kb.variant);
            }
            keyboard.change_repeat_info(kb.repeat_rate, kb.repeat_delay);
        }
        self.read_layouts();
        self.arrange();
    }

    /// Notes the keyboard's layouts and which is active.
    pub fn read_layouts(&mut self) {
        let Some(keyboard) = self.seat.get_keyboard() else { return };
        self.layouts = keyboard.with_xkb_state(self, |ctx| {
            let xkb = ctx.xkb().lock().unwrap();
            let names: Vec<String> = xkb.layouts().map(|l| xkb.layout_name(l).to_owned()).collect();
            (names, xkb.active_layout().0 as usize)
        });
    }

    /// The one output, for now (multi-monitor is Phase 2).
    pub fn output(&self) -> Option<Output> {
        self.space.outputs().next().cloned()
    }

    /// Draw a new frame when the loop is next idle.
    pub fn request_render(&mut self) {
        self.needs_render = true;
        if let Some(winit) = &self.winit {
            winit.backend.window().request_redraw();
        }
    }

    /// The window of a toplevel surface, wherever it is.
    pub fn window_for(&self, surface: &WlSurface) -> Option<Window> {
        self.all_windows().find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == surface)).cloned()
    }

    /// The area windows are laid out in: the output without the parts the
    /// panel and dock reserve.
    pub fn usable_area(&self) -> Option<tile::Rect> {
        let output = self.output()?;
        let out = self.space.output_geometry(&output)?;
        let zone = {
            let mut map = layer_map_for_output(&output);
            map.arrange();
            map.non_exclusive_zone()
        };
        Some((out.loc.x + zone.loc.x, out.loc.y + zone.loc.y, zone.size.w, zone.size.h))
    }

    /// The layer surface (panel, dock…) whose main surface is `surface`.
    pub fn layer_for(&self, surface: &WlSurface) -> Option<LayerSurface> {
        self.space.outputs().find_map(|o| layer_map_for_output(o).layer_for_surface(surface, WindowSurfaceType::TOPLEVEL).cloned())
    }

    /// Lays the windows out over what the layer surfaces leave free.
    pub fn arrange(&mut self) {
        let Some(area) = self.usable_area() else { return };
        let windows = self.tiled();
        let rects = tile::columns(windows.len(), area, self.config.gaps, self.config.border);
        tracing::debug!(?area, ?rects, "arrange");
        for (window, (x, y, w, h)) in windows.into_iter().zip(rects) {
            info(&window).borrow_mut().rect = (x, y, w, h);
            let Some(toplevel) = window.toplevel() else { continue };
            toplevel.with_pending_state(|state| {
                state.size = Some((w, h).into());
                for edge in [
                    xdg_toplevel::State::TiledLeft,
                    xdg_toplevel::State::TiledRight,
                    xdg_toplevel::State::TiledTop,
                    xdg_toplevel::State::TiledBottom,
                ] {
                    state.states.set(edge);
                }
            });
            // Sends nothing when nothing changed.
            toplevel.send_pending_configure();
            self.space.map_element(window, (x, y), false);
        }
        self.request_render();
    }

    /// Gives `window` the keyboard (and the active look); `None` takes it
    /// from every window.
    pub fn focus_window(&mut self, window: Option<Window>) {
        let serial = SERIAL_COUNTER.next_serial();
        let before = self.focused_window();
        if let Some(w) = &window {
            self.current_ws().last_focus = Some(w.clone());
        }
        for w in &self.tiled() {
            let active = window.as_ref() == Some(w);
            if w.set_activated(active)
                && let Some(t) = w.toplevel()
            {
                t.send_pending_configure();
            }
        }
        let surface = window.as_ref().and_then(|w| w.toplevel()).map(|t| t.wl_surface().clone());
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, surface, serial);
        }
        if let Some(w) = &window
            && before.as_ref() != Some(w)
        {
            self.window_event("focus", w);
        }
        self.request_render();
    }

    /// Gives the keyboard to a layer surface that asks for it (a menu of
    /// the panel, the notification centre).
    pub fn focus_layer(&mut self, layer: &LayerSurface) {
        let serial = SERIAL_COUNTER.next_serial();
        let surface = layer.wl_surface().clone();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, Some(surface), serial);
        }
    }

    /// The window with the keyboard, if a window has it.
    pub fn focused_window(&self) -> Option<Window> {
        let focus = self.seat.get_keyboard()?.current_focus()?;
        self.window_for(&focus)
    }

    /// A layer surface that takes the keyboard for as long as it is there
    /// (a lock screen, a launcher), on top first.
    pub fn exclusive_layer(&self) -> Option<LayerSurface> {
        let output = self.output()?;
        let map = layer_map_for_output(&output);
        [WlrLayer::Overlay, WlrLayer::Top].into_iter().find_map(|layer| {
            map.layers_on(layer)
                .rev()
                .find(|l| l.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive)
                .cloned()
        })
    }

    /// The surface under `pos` and where its origin is: overlay and top
    /// layers, then windows, then bottom and background layers.
    pub fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        let output = self.output()?;
        let out = self.space.output_geometry(&output)?;
        let map = layer_map_for_output(&output);
        let in_layer = |layer: WlrLayer| {
            let rel = pos - out.loc.to_f64();
            map.layer_under(layer, rel).and_then(|l| {
                let loc = map.layer_geometry(l)?.loc;
                l.surface_under(rel - loc.to_f64(), WindowSurfaceType::ALL)
                    .map(|(s, p)| (s, (p + loc + out.loc).to_f64()))
            })
        };
        in_layer(WlrLayer::Overlay)
            .or_else(|| in_layer(WlrLayer::Top))
            .or_else(|| {
                self.space.element_under(pos).and_then(|(window, location)| {
                    window
                        .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                        .map(|(s, p)| (s, (p + location).to_f64()))
                })
            })
            .or_else(|| in_layer(WlrLayer::Bottom))
            .or_else(|| in_layer(WlrLayer::Background))
    }

    /// Runs `command` through the shell, as a client of this compositor.
    pub fn spawn(&self, command: &str) {
        let mut child = std::process::Command::new("sh");
        child.arg("-c").arg(command).env("WAYLAND_DISPLAY", &self.socket_name);
        // Clients find the core's IPC under every name they look for, and
        // never the outer sway's.
        match &self.ipc.path {
            Some(path) => {
                child.env("ORBIT_SOCK", path).env("SWAYSOCK", path).env("I3SOCK", path);
            }
            None => {
                child.env_remove("ORBIT_SOCK").env_remove("SWAYSOCK").env_remove("I3SOCK");
            }
        }
        match child.spawn() {
            Ok(mut c) => {
                // Reaped in the background, so it never lingers as a zombie.
                std::thread::spawn(move || c.wait());
            }
            Err(err) => tracing::warn!("{command}: {err}"),
        }
    }
}

/// The keymap the config asks for.
fn xkb_config(kb: &crate::config::Keyboard) -> XkbConfig<'_> {
    XkbConfig {
        rules: "",
        model: &kb.model,
        layout: &kb.layout,
        variant: &kb.variant,
        options: (!kb.options.is_empty()).then(|| kb.options.clone()),
    }
}

/// Data the compositor keeps for each client.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
