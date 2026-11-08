//! The protocols the common compositors speak and programs count on,
//! besides the core's own (state.rs) — most of them Smithay's, wired here:
//!
//! - xdg-activation: a program, or a notification clicked, brings a window
//!   forward (from another workspace, or minimised);
//! - keyboard-shortcuts-inhibit: a virtual machine or a remote desktop
//!   takes every key while focused — the core's shortcuts too, but for
//!   Ctrl+Alt+Backspace and the consoles (`input`);
//! - pointer-constraints and relative-pointer: games, Blender and remote
//!   desktops hold the pointer (`input`);
//! - pointer-gestures: pinches and swipes reach the programs (`input`);
//! - ext-idle-notify: swayidle and the like learn when the user is away;
//! - text-input-v3 and input-method-v2: input methods (fcitx5, ibus) and
//!   on-screen keyboards, their candidate windows drawn as popups;
//! - xdg-foreign: a portal's dialog stays over the window that asked;
//! - ext-foreign-toplevel-list: the windows listed for any tool (and
//!   wlr-foreign-toplevel-management, which can also act on them, in
//!   `toplevels`);
//! - security-context: a sandbox (Flatpak) gets a socket whose clients see
//!   none of the privileged globals (the clipboard's history, screen
//!   copies, virtual input, monitors, gamma, the windows' list);
//! - single-pixel-buffer, content-type, alpha-modifier, presentation-time.

use smithay::{
    desktop::{PopupKind, PopupManager, Window},
    input::pointer::PointerHandle,
    reexports::wayland_server::{Client, DisplayHandle, protocol::wl_surface::WlSurface},
    utils::{Logical, Point, Rectangle},
    wayland::{
        alpha_modifier::AlphaModifierState,
        compositor::with_states,
        content_type::ContentTypeState,
        foreign_toplevel_list::{ForeignToplevelHandle, ForeignToplevelListHandler, ForeignToplevelListState},
        idle_notify::{IdleNotifierHandler, IdleNotifierState},
        input_method::{InputMethodHandler, InputMethodManagerState, PopupSurface},
        keyboard_shortcuts_inhibit::{
            KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor,
        },
        pointer_constraints::{PointerConstraint, PointerConstraintsState, with_pointer_constraint},
        pointer_gestures::PointerGesturesState,
        presentation::PresentationState,
        relative_pointer::RelativePointerManagerState,
        seat::WaylandFocus,
        security_context::{
            SecurityContext, SecurityContextHandler, SecurityContextListenerSource, SecurityContextState,
        },
        shell::xdg::XdgToplevelSurfaceData,
        single_pixel_buffer::SinglePixelBufferState,
        text_input::TextInputManagerState,
        xdg_activation::{XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData},
        xdg_foreign::{XdgForeignHandler, XdgForeignState},
    },
};

use crate::state::{ClientState, Orbit};

/// The states of these protocols, and what they keep.
pub struct Extra {
    pub shortcuts_inhibit: KeyboardShortcutsInhibitState,
    pub activation: XdgActivationState,
    pub idle_notifier: IdleNotifierState<Orbit>,
    pub foreign: XdgForeignState,
    pub toplevel_list: ForeignToplevelListState,
    /// Each window's entry in the list.
    pub toplevel_handles: std::collections::HashMap<Window, ForeignToplevelHandle>,
    /// Where a locked pointer asked to be left when let go.
    pub cursor_position_hint: Option<(WlSurface, Point<f64, Logical>)>,
    /// A swipe of three or more fingers going on: the core's (`input`).
    pub swipe: Option<crate::input::Swipe>,
    /// Globals with nothing to ask of the state: held so they stay
    /// advertised.
    _globals: (
        RelativePointerManagerState,
        PointerConstraintsState,
        PointerGesturesState,
        TextInputManagerState,
        InputMethodManagerState,
        SecurityContextState,
        PresentationState,
        SinglePixelBufferState,
        ContentTypeState,
        AlphaModifierState,
    ),
}

/// Whether `client` came through a sandbox's socket: then the privileged
/// globals are not offered to it.
pub fn sandboxed(client: &Client) -> bool {
    client
        .get_data::<ClientState>()
        .is_some_and(|d| d.security_context.is_some())
}

impl Extra {
    pub fn new(dh: &DisplayHandle, loop_handle: smithay::reexports::calloop::LoopHandle<'static, Orbit>) -> Self {
        Self {
            shortcuts_inhibit: KeyboardShortcutsInhibitState::new::<Orbit>(dh),
            activation: XdgActivationState::new::<Orbit>(dh),
            idle_notifier: IdleNotifierState::new(dh, loop_handle),
            foreign: XdgForeignState::new::<Orbit>(dh),
            toplevel_list: ForeignToplevelListState::new_with_filter::<Orbit>(dh, |c| !sandboxed(c)),
            toplevel_handles: Default::default(),
            cursor_position_hint: None,
            swipe: None,
            _globals: (
                RelativePointerManagerState::new::<Orbit>(dh),
                PointerConstraintsState::new::<Orbit>(dh),
                PointerGesturesState::new::<Orbit>(dh),
                TextInputManagerState::new::<Orbit>(dh),
                InputMethodManagerState::new::<Orbit, _>(dh, |c| !sandboxed(c)),
                // A sandbox may not make sandboxes of its own.
                SecurityContextState::new::<Orbit, _>(dh, |c| !sandboxed(c)),
                PresentationState::new::<Orbit>(dh, libc_clock_monotonic()),
                SinglePixelBufferState::new::<Orbit>(dh),
                ContentTypeState::new::<Orbit>(dh),
                AlphaModifierState::new::<Orbit>(dh),
            ),
        }
    }
}

/// CLOCK_MONOTONIC: the clock the frames' times are told in.
fn libc_clock_monotonic() -> u32 {
    1
}

/// A window's title and application id; for an X window, its class.
pub fn names(window: &Window) -> (String, String) {
    if let Some(x) = window.x11_surface() {
        return (x.title(), x.class());
    }
    let Some(toplevel) = window.toplevel() else {
        return Default::default();
    };
    with_states(toplevel.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .map(|d| {
                let d = d.lock().unwrap();
                (
                    d.title.clone().unwrap_or_default(),
                    d.app_id.clone().unwrap_or_default(),
                )
            })
            .unwrap_or_default()
    })
}

impl Orbit {
    /// A window came, changed its name, or went: the lists of windows
    /// (ext-foreign-toplevel-list, wlr-foreign-toplevel-management) follow.
    pub fn toplevels_changed(&mut self, change: &str, window: &Window) {
        match change {
            "close" => {
                if let Some(handle) = self.extra.toplevel_handles.remove(window) {
                    handle.send_closed();
                }
                self.toplevel_closed(window);
            }
            _ => {
                let (title, app_id) = names(window);
                match self.extra.toplevel_handles.get(window) {
                    Some(handle) if handle.title() != title || handle.app_id() != app_id => {
                        handle.send_title(&title);
                        handle.send_app_id(&app_id);
                        handle.send_done();
                    }
                    Some(_) => {}
                    None if change == "new" || change == "title" => {
                        let handle = self.extra.toplevel_list.new_toplevel::<Orbit>(&title, &app_id);
                        self.extra.toplevel_handles.insert(window.clone(), handle);
                    }
                    None => {}
                }
                self.toplevel_updated(window);
            }
        }
    }

    /// The window a surface belongs to, of any workspace or minimised.
    fn window_of_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.all_windows()
            .chain(self.scratchpad.iter())
            .find(|w| w.wl_surface().as_deref() == Some(surface))
            .cloned()
    }

    /// Whether the window with the keyboard asked for every key (a virtual
    /// machine, a remote desktop): then the core's shortcuts wait.
    pub fn shortcuts_inhibited(&self) -> bool {
        use smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitorSeat;
        let Some(keyboard) = self.seat.get_keyboard() else {
            return false;
        };
        keyboard
            .current_focus()
            .and_then(|f| f.wl_surface().map(|s| s.into_owned()))
            .and_then(|s| self.seat.keyboard_shortcuts_inhibitor_for_surface(&s))
            .is_some_and(|i| i.is_active())
    }

    /// The pointer moved onto `surface` at `at` (in its coordinates): a
    /// constraint waiting there starts.
    pub fn activate_constraint(
        &mut self,
        surface: &WlSurface,
        at: Point<f64, Logical>,
        pointer: &PointerHandle<Orbit>,
    ) {
        with_pointer_constraint(surface, pointer, |constraint| {
            if let Some(c) = constraint
                && !c.is_active()
                && c.region().is_none_or(|r| r.contains(at.to_i32_round()))
            {
                c.activate();
            }
        });
    }
}

impl KeyboardShortcutsInhibitHandler for Orbit {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.extra.shortcuts_inhibit
    }

    /// Granted: the program asked for it, and the keys that end the session
    /// or change the console are never held (`input`).
    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        inhibitor.activate();
    }
}

impl XdgActivationHandler for Orbit {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.extra.activation
    }

    /// A token is handed out to a program that has the keyboard (or just
    /// had it: the click or key that asked), and to the panel, dock and
    /// launchers, which ask without one.
    fn token_created(&mut self, _token: XdgActivationToken, data: XdgActivationTokenData) -> bool {
        let Some((serial, seat)) = data.serial else {
            return true;
        };
        let Some(keyboard) = self.seat.get_keyboard() else {
            return false;
        };
        smithay::input::Seat::from_resource(&seat) == Some(self.seat.clone())
            && keyboard.last_enter().is_none_or(|last| serial.is_no_older_than(&last))
    }

    /// The window comes forward — from another workspace or the
    /// scratchpad — with the keyboard, if the token is fresh.
    fn request_activation(&mut self, _token: XdgActivationToken, data: XdgActivationTokenData, surface: WlSurface) {
        if data.timestamp.elapsed().as_secs() >= 10 || self.locked() {
            return;
        }
        if let Some(window) = self.window_of_surface(&surface) {
            self.focus_anywhere(&window);
        }
    }
}

impl IdleNotifierHandler for Orbit {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.extra.idle_notifier
    }
}

impl XdgForeignHandler for Orbit {
    fn xdg_foreign_state(&mut self) -> &mut XdgForeignState {
        &mut self.extra.foreign
    }
}

impl ForeignToplevelListHandler for Orbit {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.extra.toplevel_list
    }
}

impl smithay::wayland::text_input::TextInputActivation for Orbit {}

impl InputMethodHandler for Orbit {
    /// The input method's candidate window: a popup of the window typed
    /// into, drawn with it.
    fn new_popup(&mut self, surface: PopupSurface) {
        if let Err(err) = self.popups.track_popup(PopupKind::from(surface)) {
            tracing::warn!("input method popup: {err}");
        }
    }

    fn popup_repositioned(&mut self, _surface: PopupSurface) {}

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        if let Some(parent) = surface.get_parent().map(|p| p.surface.clone()) {
            let _ = PopupManager::dismiss_popup(&parent, &PopupKind::from(surface));
        }
    }

    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        self.window_of_surface(parent)
            .and_then(|w| self.space.element_geometry(&w))
            .unwrap_or_default()
    }
}

impl SecurityContextHandler for Orbit {
    /// The sandbox's own socket: its clients are marked, so the privileged
    /// globals stay out of their sight.
    fn context_created(&mut self, source: SecurityContextListenerSource, context: SecurityContext) {
        let left = self.client_left_ping();
        let inserted = self.loop_handle.insert_source(source, move |stream, _, state| {
            let data = ClientState {
                security_context: Some(context.clone()),
                left: left.clone(),
                ..Default::default()
            };
            if let Err(err) = state.display_handle.insert_client(stream, std::sync::Arc::new(data)) {
                tracing::warn!("sandboxed client refused: {err}");
            }
        });
        if let Err(err) = inserted {
            tracing::warn!("security context: {err}");
        }
    }
}

/// Whether the constraint on `surface` is a lock (the pointer stays) or a
/// confinement (it stays inside), if one is active and the pointer, at
/// `at` in the surface, is inside its region.
pub fn constraint_at(
    surface: &WlSurface,
    pointer: &PointerHandle<Orbit>,
    at: Point<f64, Logical>,
) -> Option<(bool, Option<smithay::wayland::compositor::RegionAttributes>)> {
    with_pointer_constraint(surface, pointer, |constraint| {
        let c = constraint.filter(|c| c.is_active())?;
        if !c.region().is_none_or(|r| r.contains(at.to_i32_round())) {
            return None;
        }
        match &*c {
            PointerConstraint::Locked(_) => Some((true, None)),
            PointerConstraint::Confined(confined) => Some((false, confined.region().cloned())),
        }
    })
}
