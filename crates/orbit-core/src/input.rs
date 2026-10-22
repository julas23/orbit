//! Keyboard, pointer and wheel: the core's own shortcuts (`keys`: Mod+Return,
//! Mod+Q…) are taken before the focused client sees them; the
//! window under the pointer takes the keyboard (`focus_follows_mouse`), as
//! does whatever is clicked.
//!
//! Ported from Smithay's smallvil `src/input.rs` @ 24647fc (MIT, Copyright
//! (c) 2017 Victor Berger and Victoria Brekenfeld); shortcuts held back from
//! clients as anvil `src/input_handler.rs` @ 24647fc does (same licence).

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent, KeyState,
        KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    desktop::{WindowSurfaceType, layer_map_for_output},
    input::{
        keyboard::{FilterResult, Keysym},
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::shell::wlr_layer::Layer as WlrLayer,
};

use crate::{config::Modifier, state::Orbit};

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
        if matches!(
            event,
            InputEvent::Keyboard { .. }
                | InputEvent::PointerMotion { .. }
                | InputEvent::PointerMotionAbsolute { .. }
                | InputEvent::PointerButton { .. }
                | InputEvent::PointerAxis { .. }
        ) {
            self.touched();
        }
        match event {
            InputEvent::Keyboard { event, .. } => self.key::<I>(event),
            // A mouse or touchpad: it moves the pointer by so much.
            InputEvent::PointerMotion { event, .. } => {
                let Some(pointer) = self.seat.get_pointer() else { return };
                let pos = self.clamp(pointer.current_location() + event.delta());
                self.pointer_to(pos, Event::time(&event));
            }
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let Some(output) = self.output() else { return };
                let Some(geo) = self.space.output_geometry(&output) else { return };
                let pos = event.position_transformed(geo.size) + geo.loc.to_f64();
                self.pointer_to(pos, Event::time(&event));
            }
            // Locked, clicks reach the greet alone (the pointer's focus
            // is never another client's), and the wheel no one.
            InputEvent::PointerButton { event, .. } if self.locked() => {
                let Some(pointer) = self.seat.get_pointer() else { return };
                let serial = SERIAL_COUNTER.next_serial();
                pointer.button(self, &ButtonEvent { button: event.button_code(), state: event.state(), serial, time: Event::time(&event) });
                pointer.frame(self);
            }
            InputEvent::PointerAxis { .. } if self.locked() => {}
            InputEvent::PointerButton { event, .. } => {
                let Some(pointer) = self.seat.get_pointer() else { return };
                let serial = SERIAL_COUNTER.next_serial();
                // Mod + left button on a window moves it; the click is not
                // the window's.
                if event.state() == ButtonState::Pressed
                    && event.button_code() == crate::grab::BTN_LEFT
                    && !pointer.is_grabbed()
                    && self.modifier_held()
                    && let Some(window) = self.window_under(pointer.current_location())
                {
                    self.start_move(window, serial, None);
                    return;
                }
                if event.state() == ButtonState::Pressed && !pointer.is_grabbed() {
                    self.focus_at(pointer.current_location());
                }
                pointer.button(self, &ButtonEvent { button: event.button_code(), state: event.state(), serial, time: Event::time(&event) });
                pointer.frame(self);
            }
            InputEvent::PointerAxis { event, .. } => self.wheel::<I>(event),
            _ => {}
        }
    }

    fn key<I: InputBackend>(&mut self, event: I::KeyboardKeyEvent) {
        let Some(keyboard) = self.seat.get_keyboard() else { return };
        let serial = SERIAL_COUNTER.next_serial();
        let code = event.key_code();
        let pressed = event.state() == KeyState::Pressed;
        let action = keyboard.input::<Option<Action>, _>(self, code, event.state(), serial, Event::time(&event), |state, mods, handle| {
            if !pressed {
                // The release of a key a shortcut took is not the client's.
                if let Some(i) = state.suppressed.iter().position(|c| *c == code) {
                    state.suppressed.remove(i);
                    return FilterResult::Intercept(None);
                }
                return FilterResult::Forward;
            }
            if mods.ctrl && mods.alt && handle.modified_sym() == Keysym::BackSpace {
                state.suppressed.push(code);
                return FilterResult::Intercept(Some(Action::Emergency));
            }
            // Ctrl+Alt+Fn: the keymap gives XF86Switch_VT_n.
            let vt = handle.modified_sym().raw();
            if (Keysym::XF86_Switch_VT_1.raw()..=Keysym::XF86_Switch_VT_12.raw()).contains(&vt) {
                state.suppressed.push(code);
                return FilterResult::Intercept(Some(Action::Vt((vt - Keysym::XF86_Switch_VT_1.raw() + 1) as i32)));
            }
            // Locked, no shortcut but the two above, and no client but
            // the greet.
            if state.locked() {
                return if state.face_has_keyboard() { FilterResult::Forward } else { FilterResult::Intercept(None) };
            }
            let sym = handle.raw_latin_sym_or_raw_current_sym().unwrap_or_else(|| handle.modified_sym());
            match state.keys.command(mods, sym) {
                Some(command) => {
                    state.suppressed.push(code);
                    FilterResult::Intercept(Some(Action::Command(command.to_owned())))
                }
                None => FilterResult::Forward,
            }
        });
        if let Some(Some(action)) = action {
            self.run(action);
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
        let Some(mods) = self.seat.get_keyboard().map(|k| k.modifier_state()) else { return false };
        match self.config.modifier {
            Modifier::Super => mods.logo,
            Modifier::Alt => mods.alt,
            Modifier::Ctrl => mods.ctrl,
        }
    }

    /// `pos` kept on the screens: inside the output it is over, or the
    /// nearest one.
    fn clamp(&self, pos: Point<f64, Logical>) -> Point<f64, Logical> {
        let outputs: Vec<_> = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).collect();
        if outputs.iter().any(|g| g.to_f64().contains(pos)) || outputs.is_empty() {
            return pos;
        }
        outputs
            .iter()
            .map(|g| {
                let g = g.to_f64();
                Point::from((pos.x.clamp(g.loc.x, g.loc.x + g.size.w - 1.0), pos.y.clamp(g.loc.y, g.loc.y + g.size.h - 1.0)))
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
        let i = self.focused_window().and_then(|w| windows.iter().position(|x| x == &w)).unwrap_or(0);
        let j = if forward { (i + 1) % n } else { (i + n - 1) % n };
        windows.get(j).cloned()
    }

    /// Swaps the focused window with its neighbour on the workspace.
    pub fn move_focused(&mut self, forward: bool) {
        let Some(window) = self.focused_window() else { return };
        let ws = self.current_ws();
        let Some(i) = ws.windows.iter().position(|w| w == &window) else { return };
        let j = if forward { i + 1 } else { i.wrapping_sub(1) };
        if j < ws.windows.len() {
            ws.windows.swap(i, j);
            self.arrange();
            self.window_event("move", &window);
        }
    }

    fn pointer_to(&mut self, pos: Point<f64, Logical>, time: smithay::backend::input::InputTime) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        let serial = SERIAL_COUNTER.next_serial();
        if self.locked()
            && let Some(output) = self.output_at(pos)
        {
            self.face_follow(&output);
        }
        let under = if self.locked() { self.face_surface_under(pos) } else { self.surface_under(pos) };
        if !self.locked() && self.config.focus_follows_mouse && !pointer.is_grabbed() && self.exclusive_layer().is_none() {
            let window = self.window_under(pos);
            let over_layer = self.layer_at(pos, &[WlrLayer::Overlay, WlrLayer::Top]);
            if !over_layer && window.is_some() && window != self.focused_window() {
                self.focus_window(window);
            }
        }
        pointer.motion(self, under, &MotionEvent { location: pos, serial, time });
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
        let Some(out) = self.space.output_geometry(&output) else { return };
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
                    self.focus_window(Some(window));
                }
            }
        }
    }

    /// The window under `pos` that can be focused and moved: not an X menu
    /// or tooltip.
    fn window_under(&self, pos: Point<f64, Logical>) -> Option<smithay::desktop::Window> {
        let (window, _) = self.space.element_under(pos)?;
        self.all_windows().any(|w| w == window).then(|| window.clone())
    }

    fn layer_at(&self, pos: Point<f64, Logical>, layers: &[WlrLayer]) -> bool {
        let Some(output) = self.output_at(pos) else { return false };
        let Some(out) = self.space.output_geometry(&output) else { return false };
        let map = layer_map_for_output(&output);
        let rel = pos - out.loc.to_f64();
        layers.iter().any(|l| {
            map.layer_under(*l, rel)
                .is_some_and(|s| s.surface_under(rel - map.layer_geometry(s).map(|g| g.loc).unwrap_or_default().to_f64(), WindowSurfaceType::ALL).is_some())
        })
    }

    fn wheel<I: InputBackend>(&mut self, event: I::PointerAxisEvent) {
        let source = event.source();
        let amount = |axis| event.amount(axis).unwrap_or_else(|| event.amount_v120(axis).unwrap_or(0.0) * 15.0 / 120.0);
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
