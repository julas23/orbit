//! Moving a window with the pointer: Mod + left button anywhere on it (as
//! sway's `floating_modifier`), or the client's own title bar. The window
//! follows the pointer above the others; let go over another window, the
//! two swap places; over the empty part of a monitor, it goes to the
//! workspace that monitor shows.
//!
//! The grab is ported from Smithay's smallvil `src/grabs/move_grab.rs` @
//! 24647fc (MIT, Copyright (c) 2017 Victor Berger and Victoria Brekenfeld).

use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, Focus, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
        GestureSwipeUpdateEvent, GrabStartData, MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Serial},
};

use crate::{state::Orbit, workspace::info};

/// The left button (linux/input-event-codes.h).
pub const BTN_LEFT: u32 = 0x110;

pub struct MoveGrab {
    start_data: GrabStartData<Orbit>,
    window: Window,
    /// Where the window was when the grab began.
    from: Point<i32, Logical>,
    /// Where the pointer is now.
    at: Point<f64, Logical>,
}

impl Orbit {
    /// Starts moving `window` with the pointer, from where the pointer is.
    pub fn start_move(&mut self, window: Window, serial: Serial, start_data: Option<GrabStartData<Orbit>>) {
        let Some(pointer) = self.seat.get_pointer() else { return };
        let Some(from) = self.space.element_location(&window) else { return };
        let location = pointer.current_location();
        let start_data = start_data.unwrap_or(GrabStartData { focus: None, button: BTN_LEFT, location });
        self.dragging = Some(window.clone());
        self.space.raise_element(&window, true);
        self.focus_window(Some(window.clone()));
        pointer.set_grab(self, MoveGrab { start_data, window, from, at: location }, serial, Focus::Clear);
    }

    /// The window was let go at `pos`.
    fn drop_window(&mut self, window: &Window, pos: Point<f64, Logical>) {
        self.dragging = None;
        // It closed while it was being moved.
        if self.workspace_of(window).is_none() {
            self.arrange();
            return;
        }
        let shown: Vec<Window> = self.visible.values().flat_map(|n| self.windows_of(*n)).collect();
        let under = shown.iter().find(|w| {
            let (x, y, width, height) = info(w).borrow().rect;
            *w != window && pos.x >= x as f64 && pos.x < (x + width) as f64 && pos.y >= y as f64 && pos.y < (y + height) as f64
        });
        match under {
            Some(other) => {
                let other = other.clone();
                self.swap_windows(window, &other);
            }
            None => {
                let target = self.output_at(pos).and_then(|o| self.visible.get(&o.name()).copied());
                if let Some(num) = target.filter(|n| Some(*n) != self.workspace_of(window)) {
                    self.move_to_workspace(window, num);
                }
            }
        }
        self.arrange();
        self.focus_window(Some(window.clone()));
    }
}

impl PointerGrab<Orbit> for MoveGrab {
    fn motion(&mut self, data: &mut Orbit, handle: &mut PointerInnerHandle<'_, Orbit>, _focus: Option<(WlSurface, Point<f64, Logical>)>, event: &MotionEvent) {
        // While the grab is active, no client has pointer focus.
        handle.motion(data, None, event);
        self.at = event.location;
        let to = self.from.to_f64() + (event.location - self.start_data.location);
        data.space.map_element(self.window.clone(), to.to_i32_round(), true);
        data.request_render();
    }

    fn relative_motion(&mut self, data: &mut Orbit, handle: &mut PointerInnerHandle<'_, Orbit>, focus: Option<(WlSurface, Point<f64, Logical>)>, event: &RelativeMotionEvent) {
        handle.relative_motion(data, focus, event);
    }

    fn button(&mut self, data: &mut Orbit, handle: &mut PointerInnerHandle<'_, Orbit>, event: &ButtonEvent) {
        handle.button(data, event);
        if !handle.current_pressed().contains(&BTN_LEFT) {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(&mut self, data: &mut Orbit, handle: &mut PointerInnerHandle<'_, Orbit>, details: AxisFrame) {
        handle.axis(data, details)
    }

    fn frame(&mut self, data: &mut Orbit, handle: &mut PointerInnerHandle<'_, Orbit>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(&mut self, data: &mut Orbit, handle: &mut PointerInnerHandle<'_, Orbit>, event: &GestureSwipeBeginEvent) {
        handle.gesture_swipe_begin(data, event)
    }

    fn gesture_swipe_update(&mut self, data: &mut Orbit, handle: &mut PointerInnerHandle<'_, Orbit>, event: &GestureSwipeUpdateEvent) {
        handle.gesture_swipe_update(data, event)
    }

    fn gesture_swipe_end(&mut self, data: &mut Orbit, handle: &mut PointerInnerHandle<'_, Orbit>, event: &GestureSwipeEndEvent) {
        handle.gesture_swipe_end(data, event)
    }

    fn gesture_pinch_begin(&mut self, data: &mut Orbit, handle: &mut PointerInnerHandle<'_, Orbit>, event: &GesturePinchBeginEvent) {
        handle.gesture_pinch_begin(data, event)
    }

    fn gesture_pinch_update(&mut self, data: &mut Orbit, handle: &mut PointerInnerHandle<'_, Orbit>, event: &GesturePinchUpdateEvent) {
        handle.gesture_pinch_update(data, event)
    }

    fn gesture_pinch_end(&mut self, data: &mut Orbit, handle: &mut PointerInnerHandle<'_, Orbit>, event: &GesturePinchEndEvent) {
        handle.gesture_pinch_end(data, event)
    }

    fn gesture_hold_begin(&mut self, data: &mut Orbit, handle: &mut PointerInnerHandle<'_, Orbit>, event: &GestureHoldBeginEvent) {
        handle.gesture_hold_begin(data, event)
    }

    fn gesture_hold_end(&mut self, data: &mut Orbit, handle: &mut PointerInnerHandle<'_, Orbit>, event: &GestureHoldEndEvent) {
        handle.gesture_hold_end(data, event)
    }

    fn start_data(&self) -> &GrabStartData<Orbit> {
        &self.start_data
    }

    fn unset(&mut self, data: &mut Orbit) {
        let window = self.window.clone();
        data.drop_window(&window, self.at);
    }
}
