//! The remote mouse and keys: what the browser sends on its WebSocket,
//! given to orbit-core as a virtual pointer (wlr-virtual-pointer, made for
//! the monitor being shared, so positions are fractions of it) and a
//! virtual keyboard (virtual-keyboard, with the seat's own keymap: the
//! browser sends where a key is, `KeyboardEvent.code`, not what it types).
//!
//! The core lets go of what is still held when they are destroyed, as
//! when the browser goes away.

use std::time::Instant;

use anyhow::{Context, bail};
use serde_json::Value;
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_keyboard::KeymapFormat,
        wl_output, wl_pointer,
        wl_registry,
        wl_seat::WlSeat,
    },
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1, zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1, zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

/// Positions go to the core as fractions of this.
const EXTENT: u32 = 65_535;
/// The browser's wheel: about 100 pixels a notch; Wayland's: 15 a notch.
const WHEEL_SCALE: f64 = 15.0 / 100.0;

pub struct Input {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    pointer: ZwlrVirtualPointerV1,
    keyboard: ZwpVirtualKeyboardV1,
    start: Instant,
}

impl Input {
    /// A pointer over the monitor named `output`, and a keyboard.
    pub fn open(output: &str) -> anyhow::Result<Self> {
        let conn = Connection::connect_to_env().context("connecting to the Wayland display")?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
        let qh = queue.handle();
        let pointers: ZwlrVirtualPointerManagerV1 =
            globals.bind(&qh, 2..=2, ()).context("the compositor has no wlr-virtual-pointer version 2")?;
        let keyboards: ZwpVirtualKeyboardManagerV1 =
            globals.bind(&qh, 1..=1, ()).context("the compositor has no virtual-keyboard")?;
        let seat: WlSeat = globals.bind(&qh, 1..=8, ()).context("no seat")?;
        let mut state = State::default();
        let outputs: Vec<wl_output::WlOutput> = globals.contents().with_list(|list| {
            list.iter()
                .filter(|g| g.interface == "wl_output")
                .enumerate()
                .map(|(index, g)| {
                    state.names.push(String::new());
                    globals.registry().bind(g.name, g.version.min(4), &qh, index)
                })
                .collect()
        });
        queue.roundtrip(&mut state)?;
        let Some(index) = state.names.iter().position(|n| n == output) else {
            bail!("no monitor {output} (there are {:?})", state.names);
        };
        let pointer = pointers.create_virtual_pointer_with_output(Some(&seat), Some(&outputs[index]), &qh, ());
        let keyboard = keyboards.create_virtual_keyboard(&seat, &qh, ());
        // No keymap of its own: the seat's.
        let none = std::fs::File::open("/dev/null")?;
        keyboard.keymap(KeymapFormat::NoKeymap as u32, std::os::fd::AsFd::as_fd(&none), 0);
        queue.roundtrip(&mut state).context("the virtual pointer and keyboard")?;
        log::info!("remote: the remote mouse and keys are on {output}");
        Ok(Self { conn, queue, state, pointer, keyboard, start: Instant::now() })
    }

    /// One message from the browser (`move`, `button`, `wheel`, `key`);
    /// false if it was not one of those.
    pub fn handle(&mut self, message: &Value) -> anyhow::Result<bool> {
        let time = self.start.elapsed().as_millis() as u32;
        let number = |name: &str| message[name].as_f64().unwrap_or(0.0);
        let down = message["down"].as_bool().unwrap_or(false);
        match message["type"].as_str() {
            Some("move") => {
                let at = |v: f64| (v.clamp(0.0, 1.0) * f64::from(EXTENT)).round() as u32;
                self.pointer.motion_absolute(time, at(number("x")), at(number("y")), EXTENT, EXTENT);
                self.pointer.frame();
            }
            Some("button") => {
                let Some(button) = button(message["button"].as_u64().unwrap_or(0)) else { return Ok(true) };
                let state = if down { wl_pointer::ButtonState::Pressed } else { wl_pointer::ButtonState::Released };
                self.pointer.button(time, button, state);
                self.pointer.frame();
            }
            Some("wheel") => {
                self.pointer.axis_source(wl_pointer::AxisSource::Wheel);
                for (axis, delta) in [(wl_pointer::Axis::VerticalScroll, number("dy")), (wl_pointer::Axis::HorizontalScroll, number("dx"))] {
                    if delta == 0.0 {
                        continue;
                    }
                    let notches = (delta / 100.0).round() as i32;
                    if notches == 0 {
                        self.pointer.axis(time, axis, delta * WHEEL_SCALE);
                    } else {
                        self.pointer.axis_discrete(time, axis, delta * WHEEL_SCALE, notches);
                    }
                }
                self.pointer.frame();
            }
            Some("key") => {
                // The station sends evdev's own code; the browser, the key's name.
                let key = message["evdev"].as_u64().and_then(|k| u32::try_from(k).ok()).or_else(|| message["code"].as_str().and_then(evdev));
                let Some(key) = key else { return Ok(true) };
                self.keyboard.key(time, key, u32::from(down));
            }
            _ => return Ok(false),
        }
        self.conn.flush()?;
        // Nothing is expected back, but a protocol error would come so.
        self.queue.dispatch_pending(&mut self.state)?;
        Ok(true)
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        self.pointer.destroy();
        self.keyboard.destroy();
        let _ = self.conn.flush();
    }
}

/// The browser's `MouseEvent.button` as evdev's.
fn button(browser: u64) -> Option<u32> {
    Some(match browser {
        0 => 0x110, // BTN_LEFT
        1 => 0x112, // BTN_MIDDLE
        2 => 0x111, // BTN_RIGHT
        3 => 0x113, // BTN_SIDE (back)
        4 => 0x114, // BTN_EXTRA (forward)
        _ => return None,
    })
}

/// A key's place on the keyboard (`KeyboardEvent.code`, the UI Events
/// names) as its evdev code (linux/input-event-codes.h).
fn evdev(code: &str) -> Option<u32> {
    if let Some(n) = code.strip_prefix("F").and_then(|n| n.parse::<u32>().ok()) {
        return match n {
            1..=10 => Some(58 + n),
            11 => Some(87),
            12 => Some(88),
            13..=24 => Some(183 + n - 13),
            _ => None,
        };
    }
    Some(match code {
        "Escape" => 1,
        "Digit1" => 2,
        "Digit2" => 3,
        "Digit3" => 4,
        "Digit4" => 5,
        "Digit5" => 6,
        "Digit6" => 7,
        "Digit7" => 8,
        "Digit8" => 9,
        "Digit9" => 10,
        "Digit0" => 11,
        "Minus" => 12,
        "Equal" => 13,
        "Backspace" => 14,
        "Tab" => 15,
        "KeyQ" => 16,
        "KeyW" => 17,
        "KeyE" => 18,
        "KeyR" => 19,
        "KeyT" => 20,
        "KeyY" => 21,
        "KeyU" => 22,
        "KeyI" => 23,
        "KeyO" => 24,
        "KeyP" => 25,
        "BracketLeft" => 26,
        "BracketRight" => 27,
        "Enter" => 28,
        "ControlLeft" => 29,
        "KeyA" => 30,
        "KeyS" => 31,
        "KeyD" => 32,
        "KeyF" => 33,
        "KeyG" => 34,
        "KeyH" => 35,
        "KeyJ" => 36,
        "KeyK" => 37,
        "KeyL" => 38,
        "Semicolon" => 39,
        "Quote" => 40,
        "Backquote" => 41,
        "ShiftLeft" => 42,
        "Backslash" => 43,
        "KeyZ" => 44,
        "KeyX" => 45,
        "KeyC" => 46,
        "KeyV" => 47,
        "KeyB" => 48,
        "KeyN" => 49,
        "KeyM" => 50,
        "Comma" => 51,
        "Period" => 52,
        "Slash" => 53,
        "ShiftRight" => 54,
        "NumpadMultiply" => 55,
        "AltLeft" => 56,
        "Space" => 57,
        "CapsLock" => 58,
        "NumLock" => 69,
        "ScrollLock" => 70,
        "Numpad7" => 71,
        "Numpad8" => 72,
        "Numpad9" => 73,
        "NumpadSubtract" => 74,
        "Numpad4" => 75,
        "Numpad5" => 76,
        "Numpad6" => 77,
        "NumpadAdd" => 78,
        "Numpad1" => 79,
        "Numpad2" => 80,
        "Numpad3" => 81,
        "Numpad0" => 82,
        "NumpadDecimal" => 83,
        "IntlBackslash" => 86,
        "IntlRo" => 89,
        "NumpadEnter" => 96,
        "ControlRight" => 97,
        "NumpadDivide" => 98,
        "PrintScreen" => 99,
        "AltRight" => 100,
        "Home" => 102,
        "ArrowUp" => 103,
        "PageUp" => 104,
        "ArrowLeft" => 105,
        "ArrowRight" => 106,
        "End" => 107,
        "ArrowDown" => 108,
        "PageDown" => 109,
        "Insert" => 110,
        "Delete" => 111,
        "NumpadEqual" => 117,
        "Pause" => 119,
        "NumpadComma" => 121,
        "IntlYen" => 124,
        "MetaLeft" => 125,
        "MetaRight" => 126,
        "ContextMenu" => 127,
        _ => return None,
    })
}

#[derive(Default)]
struct State {
    names: Vec<String>,
}

impl Dispatch<wl_output::WlOutput, usize> for State {
    fn event(state: &mut Self, _: &wl_output::WlOutput, event: wl_output::Event, index: &usize, _: &Connection, _: &QueueHandle<Self>) {
        if let wl_output::Event::Name { name } = event {
            state.names[*index] = name;
        }
    }
}

macro_rules! ignore {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for State {
            fn event(_: &mut Self, _: &$t, _: <$t as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
ignore!(ZwlrVirtualPointerManagerV1, ZwlrVirtualPointerV1, ZwpVirtualKeyboardManagerV1, ZwpVirtualKeyboardV1, WlSeat);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_by_place() {
        assert_eq!(evdev("KeyA"), Some(30));
        assert_eq!(evdev("F1"), Some(59));
        assert_eq!(evdev("F12"), Some(88));
        assert_eq!(evdev("F13"), Some(183));
        assert_eq!(evdev("IntlRo"), Some(89));
        assert_eq!(evdev("Fn"), None);
    }
}
