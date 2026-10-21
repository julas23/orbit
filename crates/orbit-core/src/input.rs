//! Keyboard, pointer and wheel: the core's own shortcuts (Mod+Return,
//! Mod+Shift+Q…) are taken before the focused client sees them; the
//! window under the pointer takes the keyboard (`focus_follows_mouse`), as
//! does whatever is clicked.
//!
//! Ported from Smithay's smallvil `src/input.rs` @ 24647fc (MIT, Copyright
//! (c) 2017 Victor Berger and Victoria Brekenfeld); shortcuts held back from
//! clients as anvil `src/input_handler.rs` @ 24647fc does (same licence).

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent, KeyState,
        KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
    },
    desktop::{WindowSurfaceType, layer_map_for_output},
    input::{
        keyboard::{FilterResult, Keysym, ModifiersState},
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::shell::wlr_layer::Layer as WlrLayer,
};

use crate::{config::Modifier, state::Orbit};

/// What a shortcut does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Terminal,
    Close,
    Quit,
    FocusNext,
    FocusPrevious,
    MoveNext,
    MovePrevious,
    Workspace(i32),
    MoveToWorkspace(i32),
}

/// The action of `sym` with `mods` held, when `modifier` is the core's.
pub fn shortcut(modifier: Modifier, mods: &ModifiersState, sym: Keysym) -> Option<Action> {
    let held = match modifier {
        Modifier::Super => mods.logo,
        Modifier::Alt => mods.alt,
        Modifier::Ctrl => mods.ctrl,
    };
    if !held {
        return None;
    }
    let sym = Keysym::from(xkb_lower(sym.raw()));
    // Mod+1…9, 0 = 10: show the workspace; with Shift, send the window there.
    if let Some(d) = char::from_u32(sym.raw()).and_then(|c| c.to_digit(10)) {
        let n = if d == 0 { 10 } else { d as i32 };
        return Some(if mods.shift { Action::MoveToWorkspace(n) } else { Action::Workspace(n) });
    }
    Some(match (mods.shift, sym) {
        (false, Keysym::Return) => Action::Terminal,
        (true, Keysym::q) => Action::Close,
        (true, Keysym::e) => Action::Quit,
        (false, Keysym::Right | Keysym::l | Keysym::Down | Keysym::j) => Action::FocusNext,
        (false, Keysym::Left | Keysym::h | Keysym::Up | Keysym::k) => Action::FocusPrevious,
        (true, Keysym::Right | Keysym::l | Keysym::Down | Keysym::j) => Action::MoveNext,
        (true, Keysym::Left | Keysym::h | Keysym::Up | Keysym::k) => Action::MovePrevious,
        _ => return None,
    })
}

/// A Latin capital's small letter ("Q" with Shift is still Q).
fn xkb_lower(raw: u32) -> u32 {
    if (u32::from(b'A')..=u32::from(b'Z')).contains(&raw) { raw + 32 } else { raw }
}

impl Orbit {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => self.key::<I>(event),
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let Some(output) = self.output() else { return };
                let Some(geo) = self.space.output_geometry(&output) else { return };
                let pos = event.position_transformed(geo.size) + geo.loc.to_f64();
                self.pointer_to(pos, Event::time(&event));
            }
            InputEvent::PointerButton { event, .. } => {
                let Some(pointer) = self.seat.get_pointer() else { return };
                let serial = SERIAL_COUNTER.next_serial();
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
        let modifier = self.config.modifier;
        let action = keyboard.input::<Option<Action>, _>(self, code, event.state(), serial, Event::time(&event), |state, mods, handle| {
            if !pressed {
                // The release of a key a shortcut took is not the client's.
                if let Some(i) = state.suppressed.iter().position(|c| *c == code) {
                    state.suppressed.remove(i);
                    return FilterResult::Intercept(None);
                }
                return FilterResult::Forward;
            }
            let sym = handle.raw_latin_sym_or_raw_current_sym().unwrap_or_else(|| handle.modified_sym());
            match shortcut(modifier, mods, sym) {
                Some(action) => {
                    state.suppressed.push(code);
                    FilterResult::Intercept(Some(action))
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
            Action::Terminal => {
                let terminal = self.config.terminal.clone();
                self.spawn(&terminal);
            }
            Action::Close => {
                if let Some(t) = self.focused_window().as_ref().and_then(|w| w.toplevel()) {
                    t.send_close();
                }
            }
            Action::Quit => self.loop_signal.stop(),
            Action::FocusNext | Action::FocusPrevious => {
                let next = self.neighbour(action == Action::FocusNext);
                if next.is_some() {
                    self.focus_window(next);
                }
            }
            Action::MoveNext | Action::MovePrevious => self.move_focused(action == Action::MoveNext),
            Action::Workspace(n) => self.switch_workspace(n),
            Action::MoveToWorkspace(n) => {
                if let Some(window) = self.focused_window() {
                    self.move_to_workspace(&window, n);
                }
            }
        }
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
        let under = self.surface_under(pos);
        if self.config.focus_follows_mouse && !pointer.is_grabbed() && self.exclusive_layer().is_none() {
            let window = self.space.element_under(pos).map(|(w, _)| w.clone());
            let over_layer = self.layer_at(pos, &[WlrLayer::Overlay, WlrLayer::Top]);
            if !over_layer && window.is_some() && window != self.focused_window() {
                self.focus_window(window);
            }
        }
        pointer.motion(self, under, &MotionEvent { location: pos, serial, time });
        pointer.frame(self);
    }

    /// A click gives the keyboard to what was clicked: a layer surface that
    /// takes it (a menu, a search field) or a window.
    fn focus_at(&mut self, pos: Point<f64, Logical>) {
        if self.exclusive_layer().is_some() {
            return;
        }
        let Some(output) = self.output() else { return };
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
                if let Some((window, _)) = self.space.element_under(pos).map(|(w, l)| (w.clone(), l)) {
                    self.focus_window(Some(window));
                }
            }
        }
    }

    fn layer_at(&self, pos: Point<f64, Logical>, layers: &[WlrLayer]) -> bool {
        let Some(output) = self.output() else { return false };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(logo: bool, shift: bool) -> ModifiersState {
        ModifiersState { logo, shift, ..Default::default() }
    }

    #[test]
    fn shortcuts() {
        assert_eq!(shortcut(Modifier::Super, &mods(true, false), Keysym::Return), Some(Action::Terminal));
        assert_eq!(shortcut(Modifier::Super, &mods(true, true), Keysym::Q), Some(Action::Close));
        assert_eq!(shortcut(Modifier::Super, &mods(false, true), Keysym::Q), None);
        assert_eq!(shortcut(Modifier::Super, &mods(true, false), Keysym::x), None);
        assert_eq!(shortcut(Modifier::Super, &mods(true, true), Keysym::Left), Some(Action::MovePrevious));
        let alt = ModifiersState { alt: true, ..Default::default() };
        assert_eq!(shortcut(Modifier::Alt, &alt, Keysym::Return), Some(Action::Terminal));
        assert_eq!(shortcut(Modifier::Super, &mods(true, false), Keysym::_3), Some(Action::Workspace(3)));
        assert_eq!(shortcut(Modifier::Super, &mods(true, true), Keysym::_0), Some(Action::MoveToWorkspace(10)));
    }
}
