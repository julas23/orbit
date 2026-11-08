//! Keyboard, pointer and wheel: the core's own shortcuts (`keys`: Mod+Return,
//! Mod+Q…) are taken before the focused client sees them; the
//! window under the pointer takes the keyboard (`focus_follows_mouse`), as
//! does whatever is clicked.
//!
//! The virtual pointer and keyboard (wlr-virtual-pointer,
//! virtual-keyboard: orbit-remote-server's remote mouse and keys) come in
//! as the real ones do; their keys are read with the seat's keymap. Only
//! orbit-remote-server gets them (`remote_server`), and its keyboard does
//! not reach Ctrl+Alt+Backspace or Ctrl+Alt+F1…F12: a remote user should
//! not end the session, or switch the server's console under its owner.
//!
//! A virtual machine or remote desktop that inhibits the shortcuts
//! (`protocols`) gets them all but Ctrl+Alt+Backspace and the consoles.
//! A pointer lock holds the pointer still (only relative motion goes
//! out); a confinement keeps it over its surface. Touchpad gestures go to
//! the window under the pointer, but three-finger swipes, which walk the
//! areas of the grid. A touch screen
//! touches the surfaces under the finger; a tablet's pen moves and clicks
//! as a mouse.
//!
//! Ported from Smithay's smallvil `src/input.rs` @ 24647fc (MIT, Copyright
//! (c) 2017 Victor Berger and Victoria Brekenfeld); shortcuts held back from
//! clients as anvil `src/input_handler.rs` @ 24647fc does (same licence).

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, GestureBeginEvent, GestureEndEvent,
        GesturePinchUpdateEvent, GestureSwipeUpdateEvent, InputBackend, InputEvent, KeyState, KeyboardKeyEvent,
        PointerAxisEvent, PointerButtonEvent, PointerMotionEvent, TabletToolEvent, TabletToolTipEvent,
        TabletToolTipState, TouchEvent,
    },
    desktop::{WindowSurfaceType, layer_map_for_output},
    input::{
        keyboard::{FilterResult, KeyboardSource, Keysym},
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::{
        shell::wlr_layer::Layer as WlrLayer,
        virtual_keyboard::{VirtualKeyboardBackend, VirtualKeyboardHandler},
        virtual_pointer::{VirtualPointerBackend, VirtualPointerHandler},
    },
};

use crate::{config::Modifier, state::Orbit};

/// A swipe of three or more fingers, the core's: where it has gone.
#[derive(Debug, Clone, Copy, Default)]
pub struct Swipe {
    pub dx: f64,
    pub dy: f64,
}

/// How far a swipe goes, in the touchpad's units, to count.
const SWIPE: f64 = 120.0;

/// What a shortcut does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// A command of `keys.yaml` (`command`).
    Command(String),
    /// Ctrl+Alt+F1…F12: another virtual terminal (on the screen only).
    Vt(i32),
    /// Ctrl+Alt+Backspace: out, whatever the config says (as in X).
    Emergency,
}

impl Orbit {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        // A click that wakes the screensaver is not the hidden window's;
        // nor is its release.
        if let InputEvent::PointerButton { event, .. } = &event {
            let code = event.button_code();
            if event.state() == ButtonState::Pressed && self.saver_swallows() && !self.locked() {
                self.touched();
                self.saver_buttons.push(code);
                return;
            }
            if event.state() == ButtonState::Released
                && let Some(i) = self.saver_buttons.iter().position(|c| *c == code)
            {
                self.saver_buttons.remove(i);
                return;
            }
        }
        if matches!(
            event,
            InputEvent::Keyboard { .. }
                | InputEvent::PointerMotion { .. }
                | InputEvent::PointerMotionAbsolute { .. }
                | InputEvent::PointerButton { .. }
                | InputEvent::PointerAxis { .. }
                | InputEvent::GestureSwipeBegin { .. }
                | InputEvent::GesturePinchBegin { .. }
                | InputEvent::GestureHoldBegin { .. }
                | InputEvent::TouchDown { .. }
                | InputEvent::TabletToolProximity { .. }
                | InputEvent::TabletToolTip { .. }
        ) {
            self.touched();
        }
        match event {
            InputEvent::Keyboard { event, .. } => self.key::<I>(event, KeyboardSource::MAIN, false),
            // A mouse or touchpad: it moves the pointer by so much.
            InputEvent::PointerMotion { event, .. } => self.relative_motion::<I>(event),
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let Some(output) = self.output() else { return };
                let Some(geo) = self.space.output_geometry(&output) else {
                    return;
                };
                let pos = event.position_transformed(geo.size) + geo.loc.to_f64();
                self.pointer_to(pos, Event::time(&event));
            }
            // Locked, clicks reach the greet alone (the pointer's focus
            // is never another client's), and the wheel no one.
            InputEvent::PointerButton { event, .. } if self.locked() => {
                let Some(pointer) = self.seat.get_pointer() else { return };
                let serial = SERIAL_COUNTER.next_serial();
                pointer.button(
                    self,
                    &ButtonEvent {
                        button: event.button_code(),
                        state: event.state(),
                        serial,
                        time: Event::time(&event),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerAxis { .. } if self.locked() => {}
            InputEvent::PointerButton { event, .. } => {
                let Some(pointer) = self.seat.get_pointer() else { return };
                let serial = SERIAL_COUNTER.next_serial();
                // Mod + left button on a window moves it, Mod + right
                // button resizes it; the click is not the window's.
                if event.state() == ButtonState::Pressed
                    && matches!(event.button_code(), crate::grab::BTN_LEFT | crate::resize::BTN_RIGHT)
                    && !pointer.is_grabbed()
                    && self.modifier_held()
                    && let Some(window) = self.window_under(pointer.current_location())
                {
                    if event.button_code() == crate::grab::BTN_LEFT {
                        self.start_move(window, serial, None);
                    } else {
                        self.start_resize(window, serial, None, None);
                    }
                    return;
                }
                // A right click on the bare wallpaper: the desktop's menu.
                if event.state() == ButtonState::Pressed
                    && event.button_code() == crate::resize::BTN_RIGHT
                    && !pointer.is_grabbed()
                    && self.desktop_menu(pointer.current_location())
                {
                    return;
                }
                if event.state() == ButtonState::Pressed && !pointer.is_grabbed() {
                    self.focus_at(pointer.current_location());
                }
                pointer.button(
                    self,
                    &ButtonEvent {
                        button: event.button_code(),
                        state: event.state(),
                        serial,
                        time: Event::time(&event),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerAxis { event, .. } => self.wheel::<I>(event),
            _ if self.locked() => {}
            InputEvent::GestureSwipeBegin { event, .. } => {
                let pointer = self.seat.get_pointer();
                if self.config.gestures && event.fingers() >= 3 {
                    self.extra.swipe = Some(Swipe::default());
                } else if let Some(p) = pointer {
                    p.gesture_swipe_begin(
                        self,
                        &smithay::input::pointer::GestureSwipeBeginEvent {
                            serial: SERIAL_COUNTER.next_serial(),
                            time: Event::time(&event),
                            fingers: event.fingers(),
                        },
                    );
                }
            }
            InputEvent::GestureSwipeUpdate { event, .. } => match &mut self.extra.swipe {
                Some(swipe) => {
                    swipe.dx += event.delta_x();
                    swipe.dy += event.delta_y();
                }
                None => {
                    if let Some(p) = self.seat.get_pointer() {
                        p.gesture_swipe_update(
                            self,
                            &smithay::input::pointer::GestureSwipeUpdateEvent {
                                time: Event::time(&event),
                                delta: event.delta(),
                            },
                        );
                    }
                }
            },
            InputEvent::GestureSwipeEnd { event, .. } => match self.extra.swipe.take() {
                Some(swipe) if !event.cancelled() => self.swiped(swipe),
                Some(_) => {}
                None => {
                    if let Some(p) = self.seat.get_pointer() {
                        p.gesture_swipe_end(
                            self,
                            &smithay::input::pointer::GestureSwipeEndEvent {
                                serial: SERIAL_COUNTER.next_serial(),
                                time: Event::time(&event),
                                cancelled: event.cancelled(),
                            },
                        );
                    }
                }
            },
            InputEvent::GesturePinchBegin { event, .. } => {
                if let Some(p) = self.seat.get_pointer() {
                    p.gesture_pinch_begin(
                        self,
                        &smithay::input::pointer::GesturePinchBeginEvent {
                            serial: SERIAL_COUNTER.next_serial(),
                            time: Event::time(&event),
                            fingers: event.fingers(),
                        },
                    );
                }
            }
            InputEvent::GesturePinchUpdate { event, .. } => {
                if let Some(p) = self.seat.get_pointer() {
                    p.gesture_pinch_update(
                        self,
                        &smithay::input::pointer::GesturePinchUpdateEvent {
                            time: Event::time(&event),
                            delta: event.delta(),
                            scale: event.scale(),
                            rotation: event.rotation(),
                        },
                    );
                }
            }
            InputEvent::GesturePinchEnd { event, .. } => {
                if let Some(p) = self.seat.get_pointer() {
                    p.gesture_pinch_end(
                        self,
                        &smithay::input::pointer::GesturePinchEndEvent {
                            serial: SERIAL_COUNTER.next_serial(),
                            time: Event::time(&event),
                            cancelled: event.cancelled(),
                        },
                    );
                }
            }
            InputEvent::GestureHoldBegin { event, .. } => {
                if let Some(p) = self.seat.get_pointer() {
                    p.gesture_hold_begin(
                        self,
                        &smithay::input::pointer::GestureHoldBeginEvent {
                            serial: SERIAL_COUNTER.next_serial(),
                            time: Event::time(&event),
                            fingers: event.fingers(),
                        },
                    );
                }
            }
            InputEvent::GestureHoldEnd { event, .. } => {
                if let Some(p) = self.seat.get_pointer() {
                    p.gesture_hold_end(
                        self,
                        &smithay::input::pointer::GestureHoldEndEvent {
                            serial: SERIAL_COUNTER.next_serial(),
                            time: Event::time(&event),
                            cancelled: event.cancelled(),
                        },
                    );
                }
            }
            InputEvent::TouchDown { event, .. } => {
                let Some(pos) = self.touch_position(&event) else { return };
                self.focus_at(pos);
                let under = self.surface_under(pos);
                if let Some(touch) = self.seat.get_touch() {
                    touch.down(
                        self,
                        under,
                        &smithay::input::touch::DownEvent {
                            slot: event.slot(),
                            location: pos,
                            serial: SERIAL_COUNTER.next_serial(),
                            time: Event::time(&event),
                        },
                    );
                }
            }
            InputEvent::TouchMotion { event, .. } => {
                let Some(pos) = self.touch_position(&event) else { return };
                let under = self.surface_under(pos);
                if let Some(touch) = self.seat.get_touch() {
                    touch.motion(
                        self,
                        under,
                        &smithay::input::touch::MotionEvent {
                            slot: event.slot(),
                            location: pos,
                            time: Event::time(&event),
                        },
                    );
                }
            }
            InputEvent::TouchUp { event, .. } => {
                if let Some(touch) = self.seat.get_touch() {
                    touch.up(
                        self,
                        &smithay::input::touch::UpEvent {
                            slot: event.slot(),
                            serial: SERIAL_COUNTER.next_serial(),
                            time: Event::time(&event),
                        },
                    );
                }
            }
            InputEvent::TouchCancel { .. } => {
                if let Some(touch) = self.seat.get_touch() {
                    touch.cancel(self);
                }
            }
            InputEvent::TouchFrame { .. } => {
                if let Some(touch) = self.seat.get_touch() {
                    touch.frame(self);
                }
            }
            // A pen: the pointer goes where it points, the tip clicks.
            InputEvent::TabletToolAxis { event, .. } => {
                if let Some(pos) = self.tablet_position(&event) {
                    self.pointer_to(pos, Event::time(&event));
                }
            }
            InputEvent::TabletToolProximity { event, .. } => {
                if let Some(pos) = self.tablet_position(&event) {
                    self.pointer_to(pos, Event::time(&event));
                }
            }
            InputEvent::TabletToolTip { event, .. } => {
                let Some(pointer) = self.seat.get_pointer() else { return };
                let pressed = event.tip_state() == TabletToolTipState::Down;
                if pressed && !pointer.is_grabbed() {
                    self.focus_at(pointer.current_location());
                }
                pointer.button(
                    self,
                    &ButtonEvent {
                        button: crate::grab::BTN_LEFT,
                        state: if pressed {
                            ButtonState::Pressed
                        } else {
                            ButtonState::Released
                        },
                        serial: SERIAL_COUNTER.next_serial(),
                        time: Event::time(&event),
                    },
                );
                pointer.frame(self);
            }
            _ => {}
        }
    }

    /// A three-finger swipe ended: it walks the areas of the grid, the way
    /// the fingers push (as on a phone: left brings the area on the right).
    fn swiped(&mut self, swipe: Swipe) {
        let command = if swipe.dx.abs() >= swipe.dy.abs() {
            if swipe.dx.abs() < SWIPE {
                return;
            }
            if swipe.dx < 0.0 {
                "workspace right"
            } else {
                "workspace left"
            }
        } else {
            if swipe.dy.abs() < SWIPE {
                return;
            }
            if swipe.dy < 0.0 {
                "workspace down"
            } else {
                "workspace up"
            }
        };
        self.run(Action::Command(command.into()));
    }

    /// Where a touch is, on the touch screen's monitor: the laptop's own
    /// panel, or else the first.
    fn touch_position<E: AbsolutePositionEvent<I>, I: InputBackend>(&self, event: &E) -> Option<Point<f64, Logical>> {
        let output = self
            .space
            .outputs()
            .find(|o| o.name().starts_with("eDP"))
            .or_else(|| self.space.outputs().next())?
            .clone();
        let geo = self.space.output_geometry(&output)?;
        Some(event.position_transformed(geo.size) + geo.loc.to_f64())
    }

    fn tablet_position<E: TabletToolEvent<I>, I: InputBackend>(&self, event: &E) -> Option<Point<f64, Logical>> {
        let output = self.output()?;
        let geo = self.space.output_geometry(&output)?;
        Some(event.position_transformed(geo.size) + geo.loc.to_f64())
    }

    /// A mouse or touchpad moved: relative motion for those who listen
    /// (games); the pointer moves, unless a lock holds it, or within its
    /// confinement.
    fn relative_motion<I: InputBackend>(&mut self, event: I::PointerMotionEvent) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        let here = pointer.current_location();
        let under = if self.locked() {
            self.face_surface_under(here)
        } else {
            self.surface_under(here)
        };
        pointer.relative_motion(
            self,
            under.clone(),
            &smithay::input::pointer::RelativeMotionEvent {
                delta: event.delta(),
                delta_unaccel: event.delta_unaccel(),
                time: Event::time(&event),
            },
        );
        let constraint = under
            .as_ref()
            .filter(|_| !self.locked())
            .and_then(|(surface, loc)| crate::protocols::constraint_at(surface, &pointer, here - *loc));
        let mut pos = self.clamp(here + event.delta());
        match constraint {
            Some((true, _)) => {
                pointer.frame(self);
                return;
            }
            Some((false, region)) => {
                let (surface, loc) = under.clone().expect("a constraint has a surface");
                let inside = |p: Point<f64, Logical>| {
                    region.as_ref().is_none_or(|r| r.contains((p - loc).to_i32_round()))
                        && self.surface_under(p).is_some_and(|(s, _)| s == surface)
                };
                if !inside(Point::from((pos.x, here.y))) {
                    pos.x = here.x;
                }
                if !inside(Point::from((pos.x, pos.y))) {
                    pos.y = here.y;
                }
            }
            None => {}
        }
        self.pointer_to(pos, Event::time(&event));
        // A constraint waiting where the pointer arrived starts now.
        if !self.locked()
            && let Some((surface, loc)) = self.surface_under(pos)
        {
            self.activate_constraint(&surface, pos - loc, &pointer);
        }
    }

    /// A key; `remote` when it comes from a virtual keyboard.
    fn key<I: InputBackend>(&mut self, event: I::KeyboardKeyEvent, source: KeyboardSource, remote: bool) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        let code = event.key_code();
        let pressed = event.state() == KeyState::Pressed;
        let action = keyboard.input_from_source::<Option<Action>, _>(
            source,
            self,
            code,
            event.state(),
            serial,
            Event::time(&event),
            |state, mods, handle| {
                // A key that wakes the screensaver is not the hidden
                // window's (its release neither).
                if pressed && state.saver_swallows() && !state.locked() {
                    state.suppressed.push(code);
                    return FilterResult::Intercept(None);
                }
                // Escape while switching windows: back as it was.
                if pressed && state.switcher.is_some() && handle.modified_sym() == Keysym::Escape {
                    state.suppressed.push(code);
                    state.switch_cancel();
                    return FilterResult::Intercept(None);
                }
                if !pressed {
                    // The release of a key a shortcut took is not the client's.
                    if let Some(i) = state.suppressed.iter().position(|c| *c == code) {
                        state.suppressed.remove(i);
                        return FilterResult::Intercept(None);
                    }
                    return FilterResult::Forward;
                }
                // Ctrl+Alt+Fn: the keymap gives XF86Switch_VT_n.
                let vt = handle.modified_sym().raw();
                let vt = (Keysym::XF86_Switch_VT_1.raw()..=Keysym::XF86_Switch_VT_12.raw())
                    .contains(&vt)
                    .then(|| (vt - Keysym::XF86_Switch_VT_1.raw() + 1) as i32);
                let emergency = mods.ctrl && mods.alt && handle.modified_sym() == Keysym::BackSpace;
                if (emergency || vt.is_some()) && remote {
                    tracing::info!(
                        "a remote keyboard's Ctrl+Alt+{}: held back",
                        if emergency {
                            "Backspace".into()
                        } else {
                            format!("F{}", vt.unwrap_or(0))
                        }
                    );
                    state.suppressed.push(code);
                    return FilterResult::Intercept(None);
                }
                if emergency {
                    state.suppressed.push(code);
                    return FilterResult::Intercept(Some(Action::Emergency));
                }
                if let Some(vt) = vt {
                    state.suppressed.push(code);
                    return FilterResult::Intercept(Some(Action::Vt(vt)));
                }
                // Locked, no shortcut but the two above, and no client but
                // the greet.
                if state.locked() {
                    return if state.face_has_keyboard() {
                        FilterResult::Forward
                    } else {
                        FilterResult::Intercept(None)
                    };
                }
                // A virtual machine or a remote desktop takes every key.
                if state.shortcuts_inhibited() && state.switcher.is_none() {
                    return FilterResult::Forward;
                }
                let sym = handle
                    .raw_latin_sym_or_raw_current_sym()
                    .unwrap_or_else(|| handle.modified_sym());
                match state.keys.command(mods, sym) {
                    Some(command) => {
                        state.suppressed.push(code);
                        FilterResult::Intercept(Some(Action::Command(command.to_owned())))
                    }
                    None => FilterResult::Forward,
                }
            },
        );
        if let Some(Some(action)) = action {
            self.run(action);
        }
        // A switch settles once its modifier is let go (`switcher`).
        let mods = keyboard.modifier_state();
        if self.switcher.is_some() && !(mods.alt || mods.logo || mods.ctrl) {
            self.switch_finish();
        }
    }

    fn run(&mut self, action: Action) {
        match action {
            Action::Command(command) => {
                for result in crate::command::run(self, &command) {
                    if result["success"] == false {
                        tracing::warn!("shortcut {command}: {}", result["error"].as_str().unwrap_or_default());
                    }
                }
            }
            Action::Emergency => {
                tracing::warn!("Ctrl+Alt+Backspace: leaving");
                self.loop_signal.stop();
            }
            Action::Vt(n) => {
                if let crate::state::Backend::Drm(drm) = &mut self.backend {
                    drm.change_vt(n);
                }
            }
        }
    }

    /// Whether the core's modifier is held now.
    fn modifier_held(&self) -> bool {
        let Some(mods) = self.seat.get_keyboard().map(|k| k.modifier_state()) else {
            return false;
        };
        match self.config.modifier {
            Modifier::Super => mods.logo,
            Modifier::Alt => mods.alt,
            Modifier::Ctrl => mods.ctrl,
        }
    }

    /// `pos` kept on the screens: inside the output it is over, or the
    /// nearest one.
    fn clamp(&self, pos: Point<f64, Logical>) -> Point<f64, Logical> {
        let outputs: Vec<_> = self
            .space
            .outputs()
            .filter_map(|o| self.space.output_geometry(o))
            .collect();
        if outputs.iter().any(|g| g.to_f64().contains(pos)) || outputs.is_empty() {
            return pos;
        }
        outputs
            .iter()
            .map(|g| {
                let g = g.to_f64();
                Point::from((
                    pos.x.clamp(g.loc.x, g.loc.x + g.size.w - 1.0),
                    pos.y.clamp(g.loc.y, g.loc.y + g.size.h - 1.0),
                ))
            })
            .min_by(|a: &Point<f64, Logical>, b| {
                let d = |p: &Point<f64, Logical>| (p.x - pos.x).powi(2) + (p.y - pos.y).powi(2);
                d(a).total_cmp(&d(b))
            })
            .unwrap_or(pos)
    }

    /// The window after (or before) the focused one, wrapping around.
    pub fn neighbour(&self, forward: bool) -> Option<smithay::desktop::Window> {
        let windows = self.tiled();
        let n = windows.len();
        if n == 0 {
            return None;
        }
        let i = self
            .focused_window()
            .and_then(|w| windows.iter().position(|x| x == &w))
            .unwrap_or(0);
        let j = if forward { (i + 1) % n } else { (i + n - 1) % n };
        windows.get(j).cloned()
    }

    fn pointer_to(&mut self, pos: Point<f64, Logical>, time: smithay::backend::input::InputTime) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        let serial = SERIAL_COUNTER.next_serial();
        if self.locked()
            && let Some(output) = self.output_at(pos)
        {
            self.face_follow(&output);
        }
        let under = if self.locked() {
            self.face_surface_under(pos)
        } else {
            self.surface_under(pos)
        };
        if !self.locked()
            && self.config.focus_follows_mouse
            && !pointer.is_grabbed()
            && self.exclusive_layer().is_none()
        {
            let window = self.window_under(pos);
            let over_layer = self.layer_at(pos, &[WlrLayer::Overlay, WlrLayer::Top]);
            // A dialog keeps the keyboard when the pointer leaves it for
            // another window of its program — its parent, bigger and all
            // around it, or another of the program's windows; a click moves
            // it (but to a modal one, see `focus_at`). A window with a modal
            // dialog open hands the keyboard to it.
            let focused = self.focused_window();
            let kept = focused.as_ref().is_some_and(|f| {
                crate::floating::floats(f)
                    && crate::floating::is_dialog(f)
                    && window.as_ref().is_some_and(|w| self.same_app(f, w))
            });
            let window = window.map(|w| self.modal_for(&w).unwrap_or(w));
            if !over_layer && window.is_some() && window != focused && !kept {
                self.focus_window(window);
            }
        }
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pos,
                serial,
                time,
            },
        );
        pointer.frame(self);
        // On the screen the core draws the pointer: it moved, so a frame.
        if matches!(self.backend, crate::state::Backend::Drm(_)) {
            self.request_render();
        }
    }

    /// A click gives the keyboard to what was clicked: a layer surface that
    /// takes it (a menu, a search field) or a window.
    fn focus_at(&mut self, pos: Point<f64, Logical>) {
        if self.exclusive_layer().is_some() {
            return;
        }
        let Some(output) = self.output_at(pos) else { return };
        let Some(out) = self.space.output_geometry(&output) else {
            return;
        };
        let layer = {
            let map = layer_map_for_output(&output);
            let rel = pos - out.loc.to_f64();
            [WlrLayer::Overlay, WlrLayer::Top, WlrLayer::Bottom, WlrLayer::Background]
                .into_iter()
                .find_map(|l| map.layer_under(l, rel).cloned())
        };
        match layer {
            Some(layer) if layer.can_receive_keyboard_focus() => self.focus_layer(&layer),
            Some(layer) if matches!(layer.layer(), WlrLayer::Overlay | WlrLayer::Top) => {}
            _ => {
                if let Some(window) = self.window_under(pos) {
                    // Behind a modal dialog, the dialog takes it.
                    let window = self.modal_for(&window).unwrap_or(window);
                    self.focus_window(Some(window));
                }
            }
        }
    }

    /// Opens the desktop's menu (`desktop_menu`) when nothing is at `pos`
    /// but the wallpaper — the core's, or a background layer's (a video
    /// wallpaper, swaybg): true when it was started.
    fn desktop_menu(&self, pos: Point<f64, Logical>) -> bool {
        let command = self.config.desktop_menu.trim();
        if command.is_empty() {
            return false;
        }
        let Some(output) = self.output_at(pos) else {
            return false;
        };
        let Some(geo) = self.space.output_geometry(&output) else {
            return false;
        };
        let at = pos - geo.loc.to_f64();
        if let Some((surface, _)) = self.surface_under(pos) {
            let map = layer_map_for_output(&output);
            let background = map
                .layer_under(smithay::wayland::shell::wlr_layer::Layer::Background, at)
                .is_some_and(|l| l.wl_surface() == &surface);
            if !background {
                return false;
            }
        }
        self.spawn(&format!("{command} {} {} {}", at.x as i32, at.y as i32, output.name()));
        true
    }

    /// The window under `pos` that can be focused and moved: not an X menu
    /// or tooltip.
    fn window_under(&self, pos: Point<f64, Logical>) -> Option<smithay::desktop::Window> {
        let (window, _) = self.space.element_under(pos)?;
        self.all_windows().any(|w| w == window).then(|| window.clone())
    }

    fn layer_at(&self, pos: Point<f64, Logical>, layers: &[WlrLayer]) -> bool {
        let Some(output) = self.output_at(pos) else {
            return false;
        };
        let Some(out) = self.space.output_geometry(&output) else {
            return false;
        };
        let map = layer_map_for_output(&output);
        let rel = pos - out.loc.to_f64();
        layers.iter().any(|l| {
            map.layer_under(*l, rel).is_some_and(|s| {
                s.surface_under(
                    rel - map.layer_geometry(s).map(|g| g.loc).unwrap_or_default().to_f64(),
                    WindowSurfaceType::ALL,
                )
                .is_some()
            })
        })
    }

    fn wheel<I: InputBackend>(&mut self, event: I::PointerAxisEvent) {
        let source = event.source();
        let amount = |axis| {
            event
                .amount(axis)
                .unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0)
        };
        let mut frame = AxisFrame::new(Event::time(&event)).source(source);
        for axis in [Axis::Horizontal, Axis::Vertical] {
            let value = amount(axis);
            if value != 0.0 {
                frame = frame.value(axis, value);
                if let Some(discrete) = event.amount_v120(axis) {
                    frame = frame.v120(axis, discrete as i32);
                }
            }
            if source == AxisSource::Finger && event.amount(axis) == Some(0.0) {
                frame = frame.stop(axis);
            }
        }
        let Some(pointer) = self.seat.get_pointer() else { return };
        pointer.axis(self, frame);
        pointer.frame(self);
    }
}

impl VirtualPointerHandler for Orbit {
    fn process_virtual_pointer_event(&mut self, event: InputEvent<VirtualPointerBackend>) {
        // Absolute, over the monitor the pointer was made for, if one.
        if let InputEvent::PointerMotionAbsolute { event } = &event
            && let Some(output) = event.device().output()
            && let Some(geo) = self.space.output_geometry(&output)
        {
            self.touched();
            let pos = event.position_transformed(geo.size) + geo.loc.to_f64();
            self.pointer_to(pos, Event::time(event));
            return;
        }
        self.process_input_event(event);
    }
}

impl VirtualKeyboardHandler for Orbit {
    fn process_virtual_keyboard_event(&mut self, event: InputEvent<VirtualKeyboardBackend>) {
        // Keys only: modifiers come as their keys; keymaps are the seat's.
        if let InputEvent::Keyboard { event } = event {
            let device = Event::device(&event);
            if device.keymap().is_some() {
                tracing::debug!("a virtual keyboard's own keymap: its keys are read with the seat's");
            }
            self.touched();
            self.key::<VirtualKeyboardBackend>(event, device.source(), true);
        }
    }
}

/// Whether `client` is orbit-remote-server, the only one given the virtual
/// pointer and keyboard: its program, by the process on the other end of
/// the socket (" (deleted)" after an upgrade replaced the file).
pub fn remote_server(
    dh: &smithay::reexports::wayland_server::DisplayHandle,
    client: &smithay::reexports::wayland_server::Client,
) -> bool {
    let Ok(credentials) = client.get_credentials(dh) else {
        return false;
    };
    let allowed = std::fs::read_link(format!("/proc/{}/exe", credentials.pid))
        .ok()
        .and_then(|exe| exe.file_name().map(|n| n.to_string_lossy().into_owned()))
        .is_some_and(|name| name == "orbit-remote-server" || name == "orbit-remote-server (deleted)");
    if !allowed {
        tracing::debug!(
            "pid {}: no virtual pointer or keyboard (only orbit-remote-server's)",
            credentials.pid
        );
    }
    allowed
}
