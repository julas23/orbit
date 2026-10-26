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
        calloop::{EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction, generic::Generic, ping::make_ping},
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
        seat::WaylandFocus,
        socket::ListeningSocketSource,
        viewporter::ViewporterState,
        virtual_keyboard::VirtualKeyboardManagerState,
        virtual_pointer::VirtualPointerManagerState,
        xwayland_shell::XWaylandShellState,
        idle_inhibit::IdleInhibitManagerState,
        cursor_shape::CursorShapeManagerState,
        session_lock::SessionLockManagerState,
    },
    xwayland::X11Wm,
};

use crate::{
    config::Config,
    focus::KeyboardFocus,
    ipc::Ipc,
    tile,
    workspace::info,
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
    /// The shortcuts, from `keys.yaml` beside the config (`keys`).
    pub keys: crate::keys::Keys,

    /// What is on screen: the shown workspace's windows (the space keeps
    /// the stacking order) and the layer surfaces.
    pub space: Space<Window>,
    /// The workspaces that exist, in number order (see `workspace`).
    pub workspaces: Vec<crate::workspace::Workspace>,
    /// The workspace each monitor shows, by connector name.
    pub visible: std::collections::HashMap<String, i32>,
    /// The number of the focused one, and of the one before
    /// (`workspace back_and_forth`).
    pub current: i32,
    pub previous_ws: Option<i32>,
    /// Windows off every workspace (the panel's "minimise").
    pub scratchpad: Vec<Window>,
    /// The workspace each of them left, where restoring puts it back.
    pub minimized_from: std::collections::HashMap<Window, i32>,
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
    _globals: (XdgDecorationState, OutputManagerState, ViewporterState, smithay::wayland::fractional_scale::FractionalScaleManagerState),

    /// Where frames go: a window of the host session, or the screen.
    pub backend: Backend,
    /// What the pointer looks like (a client's image, or the theme's), and
    /// the theme, when the core draws it.
    pub cursor_status: smithay::input::pointer::CursorImageStatus,
    pub cursor: crate::cursor::Cursor,
    /// Clients name a shape and the core draws it from its theme.
    pub cursor_shape_state: CursorShapeManagerState,
    /// Something changed on screen since the last frame.
    pub needs_render: bool,
    /// The keyboard's layouts by name, and the active one (for the IPC).
    pub layouts: (Vec<String>, usize),
    /// The programs that round the corners (`rounded`): compiled at the
    /// first frame; `Some(None)` when they failed to.
    pub shaders: Option<Option<crate::rounded::Shaders>>,
    /// The virtual monitors (`virtual_output`).
    pub virtuals: Vec<crate::virtual_output::Virtual>,
    /// Whether the config's `startup:` and `login:` run (not nested, by
    /// default).
    pub startup: bool,
    /// wlr-output-management's clients (`output_management`).
    pub output_management: crate::output_management::OutputManagement,
    /// On mains power, and when that was looked at (`effects`).
    pub mains: std::cell::Cell<(Option<Instant>, bool)>,
    /// The window Mod+drag is moving (`grab`).
    pub dragging: Option<Window>,
    /// Keys a shortcut took: their release is not sent to clients.
    pub suppressed: Vec<smithay::input::keyboard::Keycode>,
    /// Whether the session is shown (`lock`).
    pub lock: crate::lock::Lock,
    /// Screen copies waiting for something to change on their monitor.
    pub screencopy: crate::screencopy::Queue,
    /// XWayland's display number, from when it is spawned, and its window
    /// manager, once it is ready (`xwayland`).
    pub xdisplay: Option<u32>,
    pub xwm: Option<X11Wm>,
    pub xwayland_shell_state: XWaylandShellState,
    /// The `login:` commands ran (`lock`).
    pub logged_in: bool,
    /// The client shown while locked (`greet`).
    pub greet: crate::greet::Greet,
    /// ext-session-lock, and the locker of others holding the lock
    /// (`session_lock`).
    pub session_lock_state: SessionLockManagerState,
    pub locker: Option<crate::session_lock::Locker>,
    /// Input and its absence (`idle`).
    pub idle: crate::idle::Idle,
    pub idle_inhibit_state: IdleInhibitManagerState,
    /// logind's sleep inhibitor, held from `PrepareForSleep` until the
    /// lock is on screen (`sleep`).
    pub sleep_inhibitor: Option<std::os::fd::OwnedFd>,
    /// The textures of the eyes opening or closing (`eyes`).
    pub eyes_cache: crate::eyes::Cache,
    /// The blur's textures under a confirmation dialog (`render`).
    pub backdrop_cache: crate::eyes::Cache,
    /// Every window, the last to have the keyboard first, and the switch
    /// walking them (`switcher`).
    pub mru: Vec<Window>,
    pub switcher: Option<crate::switcher::Switcher>,
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
        // Monitors at 1.25, 1.5…: clients draw at the scale, not blurred up.
        let fractional_scale_state = smithay::wayland::fractional_scale::FractionalScaleManagerState::new::<Self>(&dh);
        // wlr-randr, kanshi and the like (`output_management`).
        let output_management = crate::output_management::OutputManagement::new(&dh);
        let xwayland_shell_state = XWaylandShellState::new::<Self>(&dh);
        let idle_inhibit_state = IdleInhibitManagerState::new::<Self>(&dh);
        let cursor_shape_state = CursorShapeManagerState::new::<Self>(&dh);
        crate::greet::init(&dh);
        // Orbit-Blink, grim: screenshots (`screencopy`).
        crate::screencopy::init(&dh);
        // orbit-remote-server: the remote mouse and keys (`input`).
        let handle = dh.clone();
        VirtualPointerManagerState::new::<Self, _>(&dh, move |client| crate::input::remote_server(&handle, client));
        let handle = dh.clone();
        VirtualKeyboardManagerState::new::<Self, _>(&dh, move |client| crate::input::remote_server(&handle, client));
        let session_lock_state = crate::session_lock::init(&dh);

        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "seat0");
        let kb = &config.keyboard;
        if let Err(err) = seat.add_keyboard(xkb_config(kb), kb.repeat_delay, kb.repeat_rate) {
            tracing::error!("keyboard {}/{}: {err:?}; using us", kb.layout, kb.variant);
            seat.add_keyboard(XkbConfig::default(), kb.repeat_delay, kb.repeat_rate).expect("the default keymap");
        }
        seat.add_pointer();

        let socket_name = Self::listen(display, event_loop);
        let cursor = crate::cursor::Cursor::load(&config.cursor_theme, config.cursor_size);
        let keys = crate::keys::Keys::load(&config_path, &config);

        Self {
            start_time: Instant::now(),
            socket_name,
            display_handle: dh,
            loop_handle: event_loop.handle(),
            loop_signal: event_loop.get_signal(),
            config,
            keys,
            config_path,
            space: Space::default(),
            workspaces: Vec::new(),
            visible: Default::default(),
            current: 1,
            previous_ws: None,
            scratchpad: Vec::new(),
            minimized_from: Default::default(),
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
            _globals: (xdg_decoration_state, output_manager_state, viewporter_state, fractional_scale_state),
            backend: Backend::None,
            cursor_status: smithay::input::pointer::CursorImageStatus::default_named(),
            cursor,
            cursor_shape_state,
            needs_render: true,
            suppressed: Vec::new(),
            dragging: None,
            shaders: None,
            virtuals: Vec::new(),
            startup: true,
            output_management,
            mains: std::cell::Cell::new((None, true)),
            layouts: (Vec::new(), 0),
            lock: crate::lock::Lock::default(),
            screencopy: Default::default(),
            xdisplay: None,
            xwm: None,
            xwayland_shell_state,
            logged_in: false,
            greet: Default::default(),
            session_lock_state,
            locker: None,
            idle: Default::default(),
            idle_inhibit_state,
            sleep_inhibitor: None,
            eyes_cache: Default::default(),
            backdrop_cache: Default::default(),
            mru: Vec::new(),
            switcher: None,
        }
    }

    /// The Wayland socket clients connect to, and the display's own events.
    fn listen(display: Display<Self>, event_loop: &mut EventLoop<'static, Self>) -> OsString {
        let socket = ListeningSocketSource::new_auto().expect("a Wayland socket");
        let name = socket.socket_name().to_os_string();
        let handle = event_loop.handle();
        // Any client leaving: was it the locker (`session_lock`)?
        let (left, source) = make_ping().expect("a ping");
        handle.insert_source(source, |(), &mut (), state| state.client_left()).expect("the ping in the event loop");
        handle
            .insert_source(socket, move |stream, _, state| {
                let data = ClientState { left: Some(left.clone()), ..Default::default() };
                if let Err(err) = state.display_handle.insert_client(stream, Arc::new(data)) {
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

    /// Reads the config again (`reload`): keyboard, gaps, borders,
    /// colours and the monitors' places and turns apply at once; a bad
    /// file keeps the defaults (§1.5).
    pub fn reload_config(&mut self) {
        self.config = Config::load(&self.config_path);
        self.keys = crate::keys::Keys::load(&self.config_path, &self.config);
        if let Some(keyboard) = self.seat.get_keyboard() {
            let kb = self.config.keyboard.clone();
            if let Err(err) = keyboard.set_xkb_config(self, xkb_config(&kb)) {
                tracing::error!("keyboard {}/{}: {err:?}", kb.layout, kb.variant);
            }
            keyboard.change_repeat_info(kb.repeat_rate, kb.repeat_delay);
        }
        self.read_layouts();
        self.place_outputs();
        self.renumber();
        self.arrange();
        self.output_event();
        self.arm_idle();
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

    /// The monitor of the focused workspace.
    pub fn output(&self) -> Option<Output> {
        self.output_named(&self.focused_output_name()).or_else(|| self.space.outputs().next().cloned())
    }

    /// Draw a new frame when the loop is next idle.
    pub fn request_render(&mut self) {
        self.needs_render = true;
        match &self.backend {
            Backend::Winit(winit) => winit.backend.window().request_redraw(),
            Backend::Drm(_) => crate::udev::schedule(self),
            Backend::None => {}
        }
    }

    /// The window of a toplevel surface (Wayland or X), wherever it is.
    pub fn window_for(&self, surface: &WlSurface) -> Option<Window> {
        self.all_windows().find(|w| w.wl_surface().as_deref() == Some(surface)).cloned()
    }

    /// A new window, Wayland or X: on the monitor under the pointer, in
    /// the workspace it shows (a dialog, on its parent's), with the
    /// keyboard.
    pub fn add_window(&mut self, window: Window) {
        info(&window).borrow_mut().id = self.next_window_id;
        self.next_window_id += 1;
        let num = self
            .parent_of(&window)
            .and_then(|p| self.workspace_of(&p))
            .or_else(|| self.pointer_output().and_then(|o| self.visible.get(&o.name()).copied()))
            .unwrap_or(self.current);
        let i = self.ensure_ws(num);
        self.workspaces[i].windows.push(window.clone());
        self.space.map_element(window.clone(), (0, 0), false);
        self.arrange();
        self.window_event("new", &window);
        self.focus_window(Some(window));
    }

    /// A window closed: the others take its room, and the keyboard goes to
    /// its neighbour if it had it.
    pub fn remove_window(&mut self, window: &Window) {
        let had_focus = self.focused_window().as_ref() == Some(window) || self.focused_window().is_none();
        let index = self.workspace_of(window).and_then(|n| self.windows_of(n).iter().position(|w| w == window)).unwrap_or(0);
        if self.dragging.as_ref() == Some(window) {
            self.dragging = None;
        }
        let Some(num) = self.forget(window) else { return };
        self.arrange();
        if had_focus {
            // The neighbour on the left, as i3 does; or the first.
            let next = self.windows_of(num).get(index.saturating_sub(1)).cloned();
            self.focus_window(next);
        }
    }

    /// Asks a window to close.
    pub fn close_window(&self, window: &Window) {
        if let Some(t) = window.toplevel() {
            t.send_close();
        } else if let Some(x) = window.x11_surface()
            && let Err(err) = x.close()
        {
            tracing::warn!("closing X window {}: {err}", x.window_id());
        }
    }

    /// What gets the keyboard when `window` has it.
    fn keyboard_target(window: &Window) -> Option<KeyboardFocus> {
        match window.x11_surface() {
            Some(x) => Some(KeyboardFocus::X11(x.clone())),
            None => window.toplevel().map(|t| KeyboardFocus::Surface(t.wl_surface().clone())),
        }
    }

    /// The area windows are laid out in on `output`: the monitor without
    /// the parts the panel and dock reserve.
    pub fn usable_area(&self, output: &Output) -> Option<tile::Rect> {
        let out = self.space.output_geometry(output)?;
        let zone = {
            let mut map = layer_map_for_output(output);
            map.arrange();
            map.non_exclusive_zone()
        };
        Some((out.loc.x + zone.loc.x, out.loc.y + zone.loc.y, zone.size.w, zone.size.h))
    }

    /// The layer surface (panel, dock…) whose main surface is `surface`.
    pub fn layer_for(&self, surface: &WlSurface) -> Option<LayerSurface> {
        self.space.outputs().find_map(|o| layer_map_for_output(o).layer_for_surface(surface, WindowSurfaceType::TOPLEVEL).cloned())
    }

    /// Lays the windows of each monitor out over what its layer surfaces
    /// leave free.
    pub fn arrange(&mut self) {
        let shown: Vec<(String, i32)> = self.visible.iter().map(|(o, n)| (o.clone(), *n)).collect();
        for (output, num) in shown {
            let Some(output) = self.output_named(&output) else { continue };
            let (Some(area), Some(full)) = (self.usable_area(&output), self.space.output_geometry(&output)) else { continue };
            self.arrange_on(area, (full.loc.x, full.loc.y, full.size.w, full.size.h), num);
        }
        self.request_render();
    }

    /// The windows of workspace `num` in columns over `area`, as wide as
    /// their weights; a maximized one over all of `area`, a fullscreen one
    /// over the whole monitor, `full`, above the others; the floating and
    /// snapped ones above them all, in the order they were stacked
    /// (`floating`).
    fn arrange_on(&mut self, area: tile::Rect, full: tile::Rect, num: i32) {
        let windows = self.windows_of(num);
        let (mut floating, tiled): (Vec<Window>, Vec<Window>) = windows.into_iter().partition(|w| {
            crate::floating::floats(w) && !crate::fullscreen::is_fullscreen(w) && !crate::fullscreen::is_maximized(w)
        });
        let weights: Vec<f64> = tiled.iter().map(|w| info(w).borrow().weight).collect();
        let layout = self.workspaces.iter().find(|w| w.num == num).map_or(self.config.layout, |w| w.layout);
        let masters = tile::Master { ratio: self.config.master_ratio, count: self.config.master_count };
        let rects = layout.rects(&weights, area, self.config.gaps, self.config.border, masters);
        let whole = tile::columns(1, area, self.config.gaps, self.config.border).first().copied().unwrap_or(area);
        tracing::debug!(num, ?area, ?rects, "arrange");
        // The space stacks in the order windows are mapped: the tiled,
        // then the maximized and fullscreen, then the floating.
        let mut placed: Vec<(Window, tile::Rect)> = tiled.into_iter().zip(rects).collect();
        // One at a time: the focused one over the others.
        let top = (layout == tile::Layout::Monocle).then(|| self.workspaces.iter().find(|w| w.num == num).and_then(|w| w.last_focus.clone())).flatten();
        placed.sort_by_key(|(w, _)| (crate::fullscreen::is_fullscreen(w) || crate::fullscreen::is_maximized(w), Some(w) == top.as_ref()));
        for (window, rect) in placed {
            let fullscreen = crate::fullscreen::is_fullscreen(&window);
            let maximized = crate::fullscreen::is_maximized(&window);
            let (x, y, w, h) = if fullscreen { full } else if maximized { whole } else { rect };
            let above = fullscreen || maximized || Some(&window) == top.as_ref();
            info(&window).borrow_mut().rect = (x, y, w, h);
            if let Some(surface) = window.x11_surface() {
                // X windows are placed in the root window's coordinates,
                // which are the core's.
                if let Err(err) = surface.configure(smithay::utils::Rectangle::new((x, y).into(), (w, h).into())) {
                    tracing::warn!("X window {}: {err}", surface.window_id());
                }
                if self.dragging.as_ref() != Some(&window) {
                    self.space.map_element(window, (x, y), above);
                }
                continue;
            }
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
            // The one being dragged follows the pointer until it is let go.
            if self.dragging.as_ref() != Some(&window) {
                self.space.map_element(window, (x, y), above);
            }
        }
        let stacked: Vec<Window> = self.space.elements().cloned().collect();
        floating.sort_by_key(|w| stacked.iter().position(|s| s == w));
        for window in floating {
            self.place_floating(&window, area, full);
        }
    }

    /// The focused workspace's layout: `layout`, or the next one.
    pub fn set_layout(&mut self, layout: Option<tile::Layout>) {
        let num = self.current;
        let i = self.ensure_ws(num);
        let ws = &mut self.workspaces[i];
        ws.layout = layout.unwrap_or_else(|| ws.layout.next());
        tracing::info!("workspace {num}: layout {}", ws.layout.name());
        self.arrange();
        self.workspace_event("layout", num, None);
    }

    /// Gives `window` the keyboard (and the active look); `None` takes it
    /// from every window.
    pub fn focus_window(&mut self, window: Option<Window>) {
        let serial = SERIAL_COUNTER.next_serial();
        let before = self.focused_window();
        if let Some(w) = &window {
            self.yield_fullscreen(w);
            // Floating, or one of a monocle's: it comes over the others.
            let monocle = self.workspace_of(w).and_then(|n| self.workspaces.iter().find(|ws| ws.num == n)).is_some_and(|ws| ws.layout == tile::Layout::Monocle);
            if crate::floating::floats(w) || monocle {
                self.space.raise_element(w, false);
            }
            // Its dialogs over it: a bigger parent focused (a click, the
            // pointer) never hides them.
            for dialog in self.dialogs_of(w) {
                self.space.raise_element(&dialog, false);
            }
        }
        if let Some(w) = &window
            && let Some(num) = self.workspace_of(w)
        {
            let i = self.ensure_ws(num);
            self.workspaces[i].last_focus = Some(w.clone());
            self.follow_focus(num);
        }
        // Locked, it is noted for the unlock and nothing more.
        if self.locked() {
            return;
        }
        for w in &self.all_windows().cloned().collect::<Vec<_>>() {
            let active = window.as_ref() == Some(w);
            if w.set_activated(active)
                && let Some(t) = w.toplevel()
            {
                t.send_pending_configure();
            }
        }
        // An X window goes on top of the other X windows, for the X server,
        // and its dialogs over it.
        let dialogs = window.as_ref().map(|w| self.dialogs_of(w)).unwrap_or_default();
        if let Some(xwm) = self.xwm.as_mut() {
            for x in window.iter().chain(&dialogs).filter_map(|w| w.x11_surface()) {
                if let Err(err) = xwm.raise_window(x) {
                    tracing::warn!("raising X window {}: {err}", x.window_id());
                }
            }
        }
        let target = window.as_ref().and_then(Self::keyboard_target);
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, target, serial);
        }
        if let Some(w) = &window {
            self.used(w);
            if before.as_ref() != Some(w) {
                self.window_event("focus", w);
            }
        }
        self.request_render();
    }

    /// Gives the keyboard to a layer surface that asks for it (a menu of
    /// the panel, the notification centre).
    pub fn focus_layer(&mut self, layer: &LayerSurface) {
        if self.locked() {
            return;
        }
        let serial = SERIAL_COUNTER.next_serial();
        let surface = layer.wl_surface().clone();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, Some(KeyboardFocus::Surface(surface)), serial);
        }
    }

    /// The window with the keyboard, if a window has it.
    pub fn focused_window(&self) -> Option<Window> {
        match self.seat.get_keyboard()?.current_focus()? {
            KeyboardFocus::X11(x) => self.x11_window(&x),
            KeyboardFocus::Surface(s) => self.window_for(&s),
        }
    }

    /// Kills the client of the focused window (SIGKILL), as sway's
    /// `$mod+Shift+q` did; the core itself is never the target.
    pub fn kill_focused(&mut self) {
        use smithay::reexports::{
            rustix::process::{Pid, Signal, kill_process},
            wayland_server::Resource,
        };
        let Some(window) = self.focused_window() else { return };
        // An X window's client is XWayland: the program is the window's own
        // process (_NET_WM_PID), never XWayland.
        let pid = match window.x11_surface() {
            Some(x) => x.pid().map(|pid| pid as i32).ok_or_else(|| "the X window gives no pid".to_string()),
            None => match window.toplevel().and_then(|t| t.wl_surface().client()) {
                Some(client) => client.get_credentials(&self.display_handle).map(|c| c.pid).map_err(|e| e.to_string()),
                None => return,
            },
        };
        match pid {
            Ok(pid) if pid > 0 && pid as u32 != std::process::id() => {
                tracing::info!("killing client {pid}");
                if let Some(Err(err)) = Pid::from_raw(pid).map(|p| kill_process(p, Signal::KILL)) {
                    tracing::warn!("kill {pid}: {err}");
                }
            }
            Ok(_) => {}
            Err(err) => tracing::warn!("no pid for the focused client: {err}"),
        }
    }

    /// A layer surface that takes the keyboard for as long as it is there
    /// (a lock screen, a launcher), on top first.
    pub fn exclusive_layer(&self) -> Option<LayerSurface> {
        self.space.outputs().find_map(|output| {
            let map = layer_map_for_output(output);
            [WlrLayer::Overlay, WlrLayer::Top].into_iter().find_map(|layer| {
                map.layers_on(layer)
                    .rev()
                    .find(|l| l.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive)
                    .cloned()
            })
        })
    }

    /// The surface under `pos` and where its origin is: overlay and top
    /// layers, then windows, then bottom and background layers.
    pub fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        let output = self.output_at(pos)?;
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
        // A fullscreen window: above it, the overlay, the top layer's
        // menus and X menus alone.
        if let Some(full) = self.fullscreen_on(&output) {
            let menu = |l: &LayerSurface| crate::fullscreen::above_fullscreen(l);
            let in_top = || {
                let rel = pos - out.loc.to_f64();
                map.layer_under(WlrLayer::Top, rel).filter(|l| menu(l)).and_then(|l| {
                    let loc = map.layer_geometry(l)?.loc;
                    l.surface_under(rel - loc.to_f64(), WindowSurfaceType::ALL).map(|(s, p)| (s, (p + loc + out.loc).to_f64()))
                })
            };
            let window = || {
                self.space
                    .element_under(pos)
                    .filter(|(w, _)| **w == full || w.x11_surface().is_some_and(|x| x.is_override_redirect()))
                    .or_else(|| Some((&full, self.space.element_location(&full)?)))
                    .and_then(|(window, location)| {
                        window.surface_under(pos - location.to_f64(), WindowSurfaceType::ALL).map(|(s, p)| (s, (p + location).to_f64()))
                    })
            };
            return in_layer(WlrLayer::Overlay).or_else(in_top).or_else(window);
        }
        // X menus over the top layer, as they are drawn.
        let x_menu = || {
            self.space
                .elements()
                .rev()
                .filter(|w| w.x11_surface().is_some_and(|x| x.is_override_redirect()))
                .find_map(|window| {
                    let location = self.space.element_location(window)?;
                    window.surface_under(pos - location.to_f64(), WindowSurfaceType::ALL).map(|(s, p)| (s, (p + location).to_f64()))
                })
        };
        in_layer(WlrLayer::Overlay)
            .or_else(x_menu)
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
        child
            .arg("-c")
            .arg(command)
            .env("WAYLAND_DISPLAY", &self.socket_name)
            .env("XDG_SESSION_TYPE", "wayland")
            .env("XCURSOR_THEME", &self.config.cursor_theme)
            .env("XCURSOR_SIZE", self.config.cursor_size.to_string());
        // X programs reach the core's XWayland, never the host's X.
        match self.xdisplay {
            Some(n) => child.env("DISPLAY", format!(":{n}")),
            None => child.env_remove("DISPLAY"),
        };
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

/// Where frames go.
pub enum Backend {
    /// Not set up yet, or taken out for a moment while drawing.
    None,
    /// A window inside another session (`winit`).
    Winit(Box<crate::winit::WinitData>),
    /// The screen itself (`udev`).
    Drm(Box<crate::udev::DrmData>),
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
    /// Set on the greet's connection only (`greet`): rung when it closes.
    pub greet: Option<smithay::reexports::calloop::ping::Ping>,
    /// Set on the socket's clients: rung when one closes (`session_lock`).
    pub left: Option<smithay::reexports::calloop::ping::Ping>,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {
        for ping in [&self.greet, &self.left].into_iter().flatten() {
            ping.ping();
        }
    }
}
