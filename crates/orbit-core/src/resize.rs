//! Resizing a window with the pointer: Mod + right button anywhere on it
//! (as sway's `floating_modifier`), or the edges of the client's own
//! frame. A floating window grows or shrinks by the corner nearest the
//! pointer (or the edges the client names), the opposite corner staying
//! where it is; a snapped one floats first, where it was. A tiled window
//! moves the line between its column and the neighbouring one on that
//! side (their `weight`s). Maximized and fullscreen windows keep their size.

use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, Focus, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
        GestureSwipeUpdateEvent, GrabStartData, MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::{wayland_protocols::xdg::shell::server::xdg_toplevel, wayland_server::protocol::wl_surface::WlSurface},
    utils::{Logical, Point, Serial},
    xwayland::xwm,
};

use crate::{state::Orbit, tile::Rect, workspace::info};

/// The right button (linux/input-event-codes.h).
pub const BTN_RIGHT: u32 = 0x111;

/// No window or column is made narrower or shorter than this.
const MIN: i32 = 64;

/// The edges of a window that follow the pointer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Edges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

impl Edges {
    /// The corner of `rect` nearest `at`.
    fn nearest((x, y, w, h): Rect, at: Point<f64, Logical>) -> Self {
        let right = at.x >= x as f64 + w as f64 / 2.0;
        let bottom = at.y >= y as f64 + h as f64 / 2.0;
        Self { left: !right, right, top: !bottom, bottom }
    }
}

impl From<xdg_toplevel::ResizeEdge> for Edges {
    fn from(edge: xdg_toplevel::ResizeEdge) -> Self {
        let bits = u32::from(edge);
        Self { top: bits & 1 != 0, bottom: bits & 2 != 0, left: bits & 4 != 0, right: bits & 8 != 0 }
    }
}

impl From<xwm::ResizeEdge> for Edges {
    fn from(edge: xwm::ResizeEdge) -> Self {
        use xwm::ResizeEdge::*;
        Self {
            left: matches!(edge, Left | TopLeft | BottomLeft),
            right: matches!(edge, Right | TopRight | BottomRight),
            top: matches!(edge, Top | TopLeft | TopRight),
            bottom: matches!(edge, Bottom | BottomLeft | BottomRight),
        }
    }
}

enum Kind {
    /// Its rectangle when the grab began and the edges that move.
    Floating { from: Rect, edges: Edges },
    /// The column on the other side of the line that moves (on the left
    /// or not), both widths and weights when the grab began.
    Tiled { other: Window, on_left: bool, widths: (i32, i32), weights: (f64, f64) },
}

pub struct ResizeGrab {
    start_data: GrabStartData<Orbit>,
    window: Window,
    kind: Kind,
}

/// Tells a Wayland window it is being resized (or no longer is).
fn set_resizing(window: &Window, on: bool) {
    let Some(toplevel) = window.toplevel() else { return };
    toplevel.with_pending_state(|state| {
        if on {
            state.states.set(xdg_toplevel::State::Resizing);
        } else {
            state.states.unset(xdg_toplevel::State::Resizing);
        }
    });
    toplevel.send_pending_configure();
}

impl Orbit {
    /// Starts resizing `window` with the pointer, by `edges` (`None`: the
    /// corner nearest the pointer).
    pub fn start_resize(&mut self, window: Window, serial: Serial, start_data: Option<GrabStartData<Orbit>>, edges: Option<Edges>) {
        if crate::fullscreen::is_fullscreen(&window) || crate::fullscreen::is_maximized(&window) {
            return;
        }
        let Some(pointer) = self.seat.get_pointer() else { return };
        let location = pointer.current_location();
        let rect = info(&window).borrow().rect;
        let kind = if crate::floating::floats(&window) {
            {
                let mut info = info(&window).borrow_mut();
                if info.snap.take().is_some() {
                    info.floating = true;
                    info.float_at = Some((rect.0, rect.1));
                    info.float_size = Some((rect.2, rect.3));
                }
            }
            Kind::Floating { from: rect, edges: edges.unwrap_or_else(|| Edges::nearest(rect, location)) }
        } else {
            let Some(kind) = self.tiled_kind(&window, rect, location, edges) else { return };
            kind
        };
        let start_data = start_data.unwrap_or(GrabStartData { focus: None, button: BTN_RIGHT, location });
        self.space.raise_element(&window, true);
        self.focus_window(Some(window.clone()));
        set_resizing(&window, true);
        pointer.set_grab(self, ResizeGrab { start_data, window, kind }, serial, Focus::Clear);
    }

    /// A tiled window's line to move: on the side of `edges`, or the one
    /// nearest the pointer (the other when there is no column there).
    fn tiled_kind(&self, window: &Window, rect: Rect, at: Point<f64, Logical>, edges: Option<Edges>) -> Option<Kind> {
        let tiled: Vec<Window> = self
            .windows_of(self.workspace_of(window)?)
            .into_iter()
            .filter(|w| !crate::floating::floats(w) && !crate::fullscreen::is_fullscreen(w) && !crate::fullscreen::is_maximized(w))
            .collect();
        let i = tiled.iter().position(|w| w == window)?;
        let left = (i > 0).then(|| tiled[i - 1].clone());
        let right = tiled.get(i + 1).cloned();
        let (other, on_left) = match edges {
            Some(e) if e.left => (left?, true),
            Some(e) if e.right => (right?, false),
            Some(_) => return None,
            None if Edges::nearest(rect, at).left => left.map(|w| (w, true)).or(right.map(|w| (w, false)))?,
            None => right.map(|w| (w, false)).or(left.map(|w| (w, true)))?,
        };
        let (a, b) = (info(window).borrow(), info(&other).borrow());
        let (widths, weights) = ((a.rect.2, b.rect.2), (a.weight, b.weight));
        drop((a, b));
        Some(Kind::Tiled { other, on_left, widths, weights })
    }
}

impl PointerGrab<Orbit> for ResizeGrab {
    fn motion(&mut self, data: &mut Orbit, handle: &mut PointerInnerHandle<'_, Orbit>, _focus: Option<(WlSurface, Point<f64, Logical>)>, event: &MotionEvent) {
        // While the grab is active, no client has pointer focus.
        handle.motion(data, None, event);
        let delta = event.location - self.start_data.location;
        let (dx, dy) = (delta.x.round() as i32, delta.y.round() as i32);
        match &self.kind {
            Kind::Floating { from, edges } => {
                let (mut x, mut y, mut w, mut h) = *from;
                if edges.right {
                    w = (from.2 + dx).max(MIN);
                } else if edges.left {
                    w = (from.2 - dx).max(MIN);
                    x = from.0 + from.2 - w;
                }
                if edges.bottom {
                    h = (from.3 + dy).max(MIN);
                } else if edges.top {
                    h = (from.3 - dy).max(MIN);
                    y = from.1 + from.3 - h;
                }
                let mut info = info(&self.window).borrow_mut();
                info.float_at = Some((x, y));
                info.float_size = Some((w, h));
            }
            Kind::Tiled { other, on_left, widths: (a, b), weights: (ka, kb) } => {
                let both = a + b;
                if both < 2 * MIN {
                    return;
                }
                // Away from the other column, the window grows.
                let grow = if *on_left { -dx } else { dx };
                let width = (a + grow).clamp(MIN, both - MIN);
                let weight = (ka + kb) * width as f64 / both as f64;
                info(&self.window).borrow_mut().weight = weight;
                info(other).borrow_mut().weight = ka + kb - weight;
            }
        }
        data.arrange();
    }

    fn relative_motion(&mut self, data: &mut Orbit, handle: &mut PointerInnerHandle<'_, Orbit>, focus: Option<(WlSurface, Point<f64, Logical>)>, event: &RelativeMotionEvent) {
        handle.relative_motion(data, focus, event);
    }

    fn button(&mut self, data: &mut Orbit, handle: &mut PointerInnerHandle<'_, Orbit>, event: &ButtonEvent) {
        handle.button(data, event);
        if !handle.current_pressed().contains(&self.start_data.button) {
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
        set_resizing(&self.window, false);
        data.arrange();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_corner() {
        let rect = (100, 100, 200, 100);
        assert_eq!(Edges::nearest(rect, (120.0, 110.0).into()), Edges { left: true, right: false, top: true, bottom: false });
        assert_eq!(Edges::nearest(rect, (280.0, 190.0).into()), Edges { left: false, right: true, top: false, bottom: true });
    }

    #[test]
    fn client_edges() {
        assert_eq!(Edges::from(xdg_toplevel::ResizeEdge::TopRight), Edges { left: false, right: true, top: true, bottom: false });
        assert_eq!(Edges::from(xwm::ResizeEdge::BottomLeft), Edges { left: true, right: false, top: false, bottom: true });
    }
}
