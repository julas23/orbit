//! The image on screen: fitted to the window, or zoomed — with the wheel or
//! a pinch, around the pointer — and panned by dragging; turned and
//! mirrored too.
//!
//! A `Picture` in a `ScrolledWindow`: fitted, the picture scales itself
//! down to the window; zoomed, it is given the zoomed size and the scrolled
//! window pans over it, with no scrollbars. Turning and mirroring render a
//! new texture on the GPU, so the picture always shows an upright image.

use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, glib, graphene, prelude::*};

const MIN_ZOOM: f64 = 0.02;
const MAX_ZOOM: f64 = 32.0;
/// One wheel step, and the zoom buttons.
pub const STEP: f64 = 1.25;

type Listener = Box<dyn Fn(f64)>;

pub struct Canvas {
    pub widget: gtk::ScrolledWindow,
    picture: gtk::Picture,
    state: RefCell<State>,
    on_zoom: RefCell<Option<Listener>>,
}

#[derive(Default)]
struct State {
    /// As decoded.
    source: Option<gdk::Texture>,
    /// As shown: turned and mirrored.
    shown: Option<gdk::Texture>,
    /// Quarter turns clockwise, applied after the mirroring.
    turns: u8,
    mirrored: bool,
    /// `None`: fitted to the window. Otherwise image pixels per screen pixel.
    zoom: Option<f64>,
    /// Where the pointer is, in the scrolled window.
    pointer: Option<(f64, f64)>,
    drag_from: (f64, f64),
    pinch_from: f64,
}

impl Canvas {
    pub fn new() -> Rc<Self> {
        let picture = gtk::Picture::builder()
            .can_shrink(true)
            .content_fit(gtk::ContentFit::ScaleDown)
            .build();
        let widget = gtk::ScrolledWindow::builder()
            .child(&picture)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .vexpand(true)
            .css_classes(["canvas"])
            .build();
        let canvas = Rc::new(Self {
            widget,
            picture,
            state: RefCell::default(),
            on_zoom: RefCell::default(),
        });
        canvas.connect();
        canvas
    }

    fn connect(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let motion = gtk::EventControllerMotion::new();
        {
            let weak = weak.clone();
            motion.connect_motion(move |_, x, y| {
                if let Some(c) = weak.upgrade() {
                    c.state.borrow_mut().pointer = Some((x, y));
                }
            });
        }
        {
            let weak = weak.clone();
            motion.connect_leave(move |_| {
                if let Some(c) = weak.upgrade() {
                    c.state.borrow_mut().pointer = None;
                }
            });
        }
        self.widget.add_controller(motion);

        // The wheel zooms (instead of scrolling).
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let weak = weak.clone();
            scroll.connect_scroll(move |controller, _, dy| {
                let Some(c) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                // A wheel notch is one step; a touchpad moves in pixels.
                let steps = match controller.unit() {
                    gdk::ScrollUnit::Wheel => dy,
                    _ => dy / 20.0,
                };
                let pointer = c.state.borrow().pointer;
                c.zoom_to(c.zoom() * STEP.powf(-steps), pointer);
                glib::Propagation::Stop
            });
        }
        self.widget.add_controller(scroll);

        // Dragging pans a zoomed image.
        let drag = gtk::GestureDrag::new();
        {
            let weak = weak.clone();
            drag.connect_drag_begin(move |_, _, _| {
                if let Some(c) = weak.upgrade() {
                    let from = (c.widget.hadjustment().value(), c.widget.vadjustment().value());
                    c.state.borrow_mut().drag_from = from;
                    if c.state.borrow().zoom.is_some() {
                        c.widget.set_cursor_from_name(Some("grabbing"));
                    }
                }
            });
        }
        {
            let weak = weak.clone();
            drag.connect_drag_update(move |_, dx, dy| {
                if let Some(c) = weak.upgrade() {
                    let (x, y) = c.state.borrow().drag_from;
                    c.widget.hadjustment().set_value(x - dx);
                    c.widget.vadjustment().set_value(y - dy);
                }
            });
        }
        {
            let weak = weak.clone();
            drag.connect_drag_end(move |_, _, _| {
                if let Some(c) = weak.upgrade() {
                    c.update_cursor();
                }
            });
        }
        self.widget.add_controller(drag);

        // Double-click: fitted ⇄ actual size, at the pointer.
        let click = gtk::GestureClick::new();
        {
            let weak = weak.clone();
            click.connect_pressed(move |_, n, x, y| {
                if n == 2
                    && let Some(c) = weak.upgrade()
                {
                    if c.state.borrow().zoom.is_some() {
                        c.fit();
                    } else {
                        c.zoom_to(1.0, Some((x, y)));
                    }
                }
            });
        }
        self.widget.add_controller(click);

        // Pinching, on a touchpad or a touch screen.
        let pinch = gtk::GestureZoom::new();
        {
            let weak = weak.clone();
            pinch.connect_begin(move |_, _| {
                if let Some(c) = weak.upgrade() {
                    let zoom = c.zoom();
                    c.state.borrow_mut().pinch_from = zoom;
                }
            });
        }
        {
            let weak = weak.clone();
            pinch.connect_scale_changed(move |gesture, scale| {
                if let Some(c) = weak.upgrade() {
                    let from = c.state.borrow().pinch_from;
                    c.zoom_to(from * scale, gesture.bounding_box_center());
                }
            });
        }
        self.widget.add_controller(pinch);

        // Fitted, the zoom follows the window's size.
        self.widget.hadjustment().connect_changed(move |_| {
            if let Some(c) = weak.upgrade()
                && c.state.borrow().zoom.is_none()
            {
                c.notify();
            }
        });
    }

    /// Called with the zoom whenever it changes.
    pub fn connect_zoom(&self, f: impl Fn(f64) + 'static) {
        *self.on_zoom.borrow_mut() = Some(Box::new(f));
    }

    /// Shows `texture`. A `fresh` image starts fitted, upright; a further
    /// frame of the same one keeps the zoom and orientation.
    pub fn set_image(&self, texture: &gdk::Texture, fresh: bool) {
        {
            let mut st = self.state.borrow_mut();
            if fresh {
                st.turns = 0;
                st.mirrored = false;
                st.zoom = None;
            }
            st.source = Some(texture.clone());
        }
        self.reorient();
        if fresh {
            self.apply();
        }
    }

    pub fn clear(&self) {
        *self.state.borrow_mut() = State::default();
        self.picture.set_paintable(None::<&gdk::Paintable>);
    }

    /// The image as shown, turned and mirrored.
    pub fn texture(&self) -> Option<gdk::Texture> {
        self.state.borrow().shown.clone()
    }

    /// Quarter turns clockwise, and whether mirrored (left to right) before.
    pub fn orientation(&self) -> (u8, bool) {
        let st = self.state.borrow();
        (st.turns, st.mirrored)
    }

    pub fn turn(&self, clockwise: bool) {
        {
            let mut st = self.state.borrow_mut();
            st.turns = (st.turns + if clockwise { 1 } else { 3 }) % 4;
            st.zoom = None;
        }
        self.reorient();
        self.apply();
    }

    /// Mirrors left to right (`horizontal`) or top to bottom, as seen.
    pub fn mirror(&self, horizontal: bool) {
        {
            let mut st = self.state.borrow_mut();
            // Mirroring after turning by r is turning by −r after mirroring;
            // top to bottom is left to right turned half a turn.
            st.mirrored = !st.mirrored;
            st.turns = (4 - st.turns + if horizontal { 0 } else { 2 }) % 4;
        }
        self.reorient();
    }

    fn reorient(&self) {
        let shown = {
            let st = self.state.borrow();
            st.source
                .as_ref()
                .map(|t| self.oriented(t, st.turns, st.mirrored))
        };
        self.picture.set_paintable(shown.as_ref());
        self.state.borrow_mut().shown = shown;
    }

    fn oriented(&self, texture: &gdk::Texture, turns: u8, mirrored: bool) -> gdk::Texture {
        if turns == 0 && !mirrored {
            return texture.clone();
        }
        let Some(renderer) = self.widget.native().and_then(|n| n.renderer()) else {
            return texture.clone();
        };
        let (w, h) = (texture.width() as f32, texture.height() as f32);
        let (ow, oh) = if turns % 2 == 1 { (h, w) } else { (w, h) };
        let snapshot = gtk::Snapshot::new();
        snapshot.translate(&graphene::Point::new(ow / 2.0, oh / 2.0));
        snapshot.rotate(90.0 * f32::from(turns));
        if mirrored {
            snapshot.scale(-1.0, 1.0);
        }
        snapshot.append_texture(texture, &graphene::Rect::new(-w / 2.0, -h / 2.0, w, h));
        match snapshot.to_node() {
            Some(node) => {
                renderer.render_texture(node, Some(&graphene::Rect::new(0.0, 0.0, ow, oh)))
            }
            None => texture.clone(),
        }
    }

    /// Image pixels per screen pixel now.
    pub fn zoom(&self) -> f64 {
        self.state.borrow().zoom.unwrap_or_else(|| self.fit_zoom())
    }

    pub fn is_fitted(&self) -> bool {
        self.state.borrow().zoom.is_none()
    }

    /// The zoom that fits the image in the window, never above 1.
    fn fit_zoom(&self) -> f64 {
        let Some((w, h)) = self.size() else {
            return 1.0;
        };
        let (vw, vh) = self.viewport();
        if vw <= 0.0 || vh <= 0.0 {
            return 1.0;
        }
        let s = f64::from(self.widget.scale_factor());
        (vw * s / w).min(vh * s / h).min(1.0)
    }

    pub fn fit(&self) {
        self.state.borrow_mut().zoom = None;
        self.apply();
    }

    /// Zooms to `zoom` keeping the image point under `anchor` (a point in
    /// the scrolled window; its centre by default) where it is. Below the
    /// fitted size, fits.
    pub fn zoom_to(&self, zoom: f64, anchor: Option<(f64, f64)>) {
        let Some((w, h)) = self.size() else { return };
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let fit = self.fit_zoom();
        if zoom <= fit + 1e-9 && zoom < self.zoom() {
            self.fit();
            return;
        }
        let (vw, vh) = self.viewport();
        let (ax, ay) = anchor.unwrap_or((vw / 2.0, vh / 2.0));
        let s = f64::from(self.widget.scale_factor());
        let (iw, ih) = (w / s, h / s);
        let hadj = self.widget.hadjustment();
        let vadj = self.widget.vadjustment();

        // The fraction of the image under the anchor, before.
        let old = self.zoom();
        let (ow, oh) = (iw * old, ih * old);
        let (ox, oy) = (
            ((vw - ow) / 2.0).max(0.0) - if self.is_fitted() { 0.0 } else { hadj.value() },
            ((vh - oh) / 2.0).max(0.0) - if self.is_fitted() { 0.0 } else { vadj.value() },
        );
        let fx = ((ax - ox) / ow).clamp(0.0, 1.0);
        let fy = ((ay - oy) / oh).clamp(0.0, 1.0);

        self.state.borrow_mut().zoom = Some(zoom);
        self.apply();

        // Set the scroll position now, ahead of the layout that would
        // otherwise clamp it to the old size.
        let (nw, nh) = (iw * zoom, ih * zoom);
        let (cw, ch) = (nw.max(vw), nh.max(vh));
        let x = (fx * nw + ((vw - nw) / 2.0).max(0.0) - ax).clamp(0.0, cw - vw);
        let y = (fy * nh + ((vh - nh) / 2.0).max(0.0) - ay).clamp(0.0, ch - vh);
        hadj.configure(x, 0.0, cw, vw * 0.1, vw * 0.9, vw);
        vadj.configure(y, 0.0, ch, vh * 0.1, vh * 0.9, vh);
    }

    /// Zooms by `factor` around the centre.
    pub fn zoom_by(&self, factor: f64) {
        self.zoom_to(self.zoom() * factor, None);
    }

    fn apply(&self) {
        let zoom = self.state.borrow().zoom;
        match (zoom, self.size()) {
            (Some(zoom), Some((w, h))) => {
                let s = f64::from(self.widget.scale_factor());
                self.picture.set_content_fit(gtk::ContentFit::Fill);
                self.picture.set_halign(gtk::Align::Center);
                self.picture.set_valign(gtk::Align::Center);
                self.picture.set_size_request(
                    (w * zoom / s).round() as i32,
                    (h * zoom / s).round() as i32,
                );
                self.widget
                    .set_policy(gtk::PolicyType::External, gtk::PolicyType::External);
            }
            _ => {
                self.picture.set_content_fit(gtk::ContentFit::ScaleDown);
                self.picture.set_halign(gtk::Align::Fill);
                self.picture.set_valign(gtk::Align::Fill);
                self.picture.set_size_request(-1, -1);
                self.widget
                    .set_policy(gtk::PolicyType::Never, gtk::PolicyType::Never);
            }
        }
        self.update_cursor();
        self.notify();
    }

    fn update_cursor(&self) {
        let zoomed = self.state.borrow().zoom.is_some();
        self.widget
            .set_cursor_from_name(if zoomed { Some("grab") } else { None });
    }

    fn notify(&self) {
        if self.state.borrow().shown.is_none() {
            return;
        }
        let zoom = self.zoom();
        if let Some(f) = self.on_zoom.borrow().as_ref() {
            f(zoom);
        }
    }

    /// The shown image's size in pixels.
    fn size(&self) -> Option<(f64, f64)> {
        self.state
            .borrow()
            .shown
            .as_ref()
            .map(|t| (f64::from(t.width()), f64::from(t.height())))
    }

    fn viewport(&self) -> (f64, f64) {
        (
            f64::from(self.widget.width()),
            f64::from(self.widget.height()),
        )
    }
}
