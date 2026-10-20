//! Choosing an area on the frozen screen.
//!
//! The screens are captured first; then each is covered by an overlay
//! showing that capture, dimmed, so what is chosen is exactly what was on
//! screen when the key was pressed — open menus and tooltips included.
//!
//! Dragging draws the rectangle, with its size in pixels; a magnifier by the
//! pointer helps hitting the exact pixel. Without dragging, the window under
//! the pointer is outlined and a click takes it. Escape or the right button
//! cancels.

use std::{cell::RefCell, rc::Rc};

use gtk::{cairo, gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::{capture::Screen, image::Rect};

/// Pixels of the screen magnified, each side, and how much.
const LOUPE_SPAN: f64 = 15.0;
const LOUPE_ZOOM: f64 = 8.0;

struct Selection {
    /// Where the drag started, global logical pixels.
    start: Option<(f64, f64)>,
    /// The pointer, global logical pixels.
    pointer: Option<(f64, f64)>,
    /// The window under the pointer.
    hover: Option<Rect>,
    finished: bool,
}

/// Called once with the choice.
type Done = Box<dyn FnOnce(Option<Rect>)>;

struct Overlay {
    screens: Rc<Vec<Screen>>,
    surfaces: Vec<Option<cairo::ImageSurface>>,
    windows: Vec<Rect>,
    selection: RefCell<Selection>,
    overlays: RefCell<Vec<gtk::Window>>,
    areas: RefCell<Vec<gtk::DrawingArea>>,
    accent: gdk::RGBA,
    done: RefCell<Option<Done>>,
}

/// Lets the user pick an area on `screens`; `done` gets it, in global
/// logical pixels, or None when cancelled.
pub fn choose(
    app: &gtk::Application,
    screens: Rc<Vec<Screen>>,
    windows: Vec<Rect>,
    done: impl FnOnce(Option<Rect>) + 'static,
) {
    let surfaces = screens
        .iter()
        .map(|s| s.image.surface().inspect_err(|e| log::warn!("{e:#}")).ok())
        .collect();
    let overlay = Rc::new(Overlay {
        screens: screens.clone(),
        surfaces,
        windows,
        selection: RefCell::new(Selection {
            start: None,
            pointer: None,
            hover: None,
            finished: false,
        }),
        overlays: RefCell::new(Vec::new()),
        areas: RefCell::new(Vec::new()),
        accent: orbit_app::theme::accent(),
        done: RefCell::new(Some(Box::new(done))),
    });

    let Some(display) = gdk::Display::default() else {
        overlay.finish(None);
        return;
    };
    let monitors = display.monitors();
    for (index, screen) in screens.iter().enumerate() {
        let monitor = (0..monitors.n_items())
            .filter_map(|i| monitors.item(i).and_downcast::<gdk::Monitor>())
            .find(|m| m.connector().as_deref() == Some(screen.name.as_str()));
        let Some(monitor) = monitor else {
            log::warn!("no monitor for {}", screen.name);
            continue;
        };
        let window = gtk::Window::builder()
            .application(app)
            .css_classes(["orbit-blink-overlay"])
            .decorated(false)
            .build();
        window.init_layer_shell();
        window.set_namespace(Some("orbit-blink"));
        window.set_layer(Layer::Overlay);
        window.set_monitor(Some(&monitor));
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(edge, true);
        }
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(if screen.focused {
            KeyboardMode::Exclusive
        } else {
            KeyboardMode::OnDemand
        });

        let area = gtk::DrawingArea::new();
        area.set_cursor_from_name(Some("crosshair"));
        {
            let overlay = overlay.clone();
            area.set_draw_func(move |_, cr, _, _| overlay.draw(index, cr));
        }
        let origin = (f64::from(screen.logical.x), f64::from(screen.logical.y));
        let global = move |x: f64, y: f64| (origin.0 + x, origin.1 + y);

        let motion = gtk::EventControllerMotion::new();
        {
            let overlay = overlay.clone();
            motion.connect_motion(move |_, x, y| overlay.moved(global(x, y)));
        }
        {
            // The pointer arriving counts too: the overlay appears under it.
            let overlay = overlay.clone();
            motion.connect_enter(move |_, x, y| overlay.moved(global(x, y)));
        }
        area.add_controller(motion);

        let drag = gtk::GestureDrag::builder().button(1).build();
        {
            let overlay = overlay.clone();
            drag.connect_drag_begin(move |_, x, y| {
                let p = global(x, y);
                overlay.selection.borrow_mut().start = Some(p);
                overlay.moved(p);
            });
        }
        {
            let overlay = overlay.clone();
            drag.connect_drag_update(move |gesture, dx, dy| {
                if let Some((x, y)) = gesture.start_point() {
                    overlay.moved(global(x + dx, y + dy));
                }
            });
        }
        {
            let overlay = overlay.clone();
            drag.connect_drag_end(move |gesture, dx, dy| {
                let Some((x, y)) = gesture.start_point() else {
                    return;
                };
                let start = global(x, y);
                let end = global(x + dx, y + dy);
                let rect = if dx.abs() < 3.0 && dy.abs() < 3.0 {
                    // A click: the window under the pointer.
                    overlay.selection.borrow().hover
                } else {
                    Some(overlay.clamp(start, end))
                };
                overlay.finish(rect);
            });
        }
        area.add_controller(drag);

        let cancel = gtk::GestureClick::builder().button(3).build();
        {
            let overlay = overlay.clone();
            cancel.connect_pressed(move |_, _, _, _| overlay.finish(None));
        }
        area.add_controller(cancel);

        let keys = gtk::EventControllerKey::new();
        {
            let overlay = overlay.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                match key {
                    gdk::Key::Escape => overlay.finish(None),
                    gdk::Key::Return | gdk::Key::KP_Enter => {
                        let hover = overlay.selection.borrow().hover;
                        overlay.finish(hover);
                    }
                    _ => return glib::Propagation::Proceed,
                }
                glib::Propagation::Stop
            });
        }
        window.add_controller(keys);

        window.set_child(Some(&area));
        window.present();
        overlay.overlays.borrow_mut().push(window);
        overlay.areas.borrow_mut().push(area);
    }
    if overlay.overlays.borrow().is_empty() {
        overlay.finish(None);
    }
}

impl Overlay {
    fn moved(&self, p: (f64, f64)) {
        {
            let mut sel = self.selection.borrow_mut();
            sel.pointer = Some(p);
            // The topmost window under the pointer (floating ones come last).
            sel.hover = self
                .windows
                .iter()
                .rev()
                .find(|w| w.contains(p.0, p.1))
                .copied();
        }
        for area in self.areas.borrow().iter() {
            area.queue_draw();
        }
    }

    /// The rectangle from `a` to `b`, within the screen where `a` is.
    fn clamp(&self, a: (f64, f64), b: (f64, f64)) -> Rect {
        let rect = Rect::from_corners(a, b);
        self.screens
            .iter()
            .find(|s| s.logical.contains(a.0, a.1))
            .and_then(|s| rect.intersect(s.logical))
            .unwrap_or(rect)
    }

    fn finish(&self, rect: Option<Rect>) {
        {
            let mut sel = self.selection.borrow_mut();
            if sel.finished {
                return;
            }
            sel.finished = true;
        }
        for window in self.overlays.borrow_mut().drain(..) {
            window.destroy();
        }
        if let Some(done) = self.done.borrow_mut().take() {
            done(rect);
        }
    }

    /// The current choice: the rectangle being dragged, else the window
    /// under the pointer.
    fn chosen(&self) -> Option<Rect> {
        let sel = self.selection.borrow();
        match (sel.start, sel.pointer) {
            (Some(a), Some(b)) => Some(self.clamp(a, b)),
            _ => sel.hover,
        }
    }

    fn draw(&self, index: usize, cr: &cairo::Context) {
        let screen = &self.screens[index];
        let origin = screen.logical;
        let scale = screen.scale();
        let (w, h) = (f64::from(origin.width), f64::from(origin.height));

        // The frozen screen.
        if let Some(surface) = &self.surfaces[index] {
            cr.save().ok();
            cr.scale(1.0 / scale, 1.0 / scale);
            cr.set_source_surface(surface, 0.0, 0.0).ok();
            cr.paint().ok();
            cr.restore().ok();
        }

        // Dimmed, but for the choice.
        let chosen = self.chosen().and_then(|r| r.intersect(origin));
        cr.set_fill_rule(cairo::FillRule::EvenOdd);
        cr.rectangle(0.0, 0.0, w, h);
        if let Some(r) = chosen {
            cr.rectangle(
                f64::from(r.x - origin.x),
                f64::from(r.y - origin.y),
                f64::from(r.width),
                f64::from(r.height),
            );
        }
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.45);
        cr.fill().ok();

        if let Some(r) = chosen {
            let (x, y) = (f64::from(r.x - origin.x), f64::from(r.y - origin.y));
            let a = self.accent;
            cr.set_source_rgba(a.red().into(), a.green().into(), a.blue().into(), 1.0);
            cr.set_line_width(2.0);
            cr.rectangle(x, y, f64::from(r.width), f64::from(r.height));
            cr.stroke().ok();
            // The size, in the capture's own pixels.
            let label = format!(
                "{} × {}",
                (f64::from(r.width) * scale).round(),
                (f64::from(r.height) * scale).round()
            );
            self.tag(cr, &label, x, y + f64::from(r.height) + 5.0, h);
        }

        // The magnifier, by the pointer, on the screen it is on.
        let pointer = self.selection.borrow().pointer;
        if let (Some((px, py)), Some(surface)) = (pointer, &self.surfaces[index])
            && origin.contains(px, py)
        {
            self.loupe(
                cr,
                surface,
                px - f64::from(origin.x),
                py - f64::from(origin.y),
                scale,
                w,
                h,
            );
        }
    }

    /// A small label on a dark background, kept inside the screen.
    fn tag(&self, cr: &cairo::Context, text: &str, x: f64, y: f64, screen_h: f64) {
        cr.select_font_face(
            "monospace",
            cairo::FontSlant::Normal,
            cairo::FontWeight::Bold,
        );
        cr.set_font_size(13.0);
        let Ok(ext) = cr.text_extents(text) else {
            return;
        };
        let (bw, bh) = (ext.width() + 10.0, 20.0);
        let y = if y + bh > screen_h { y - bh - 10.0 } else { y };
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.75);
        cr.rectangle(x, y, bw, bh);
        cr.fill().ok();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.move_to(x + 5.0, y + 14.0);
        cr.show_text(text).ok();
    }

    #[allow(clippy::too_many_arguments)]
    fn loupe(
        &self,
        cr: &cairo::Context,
        surface: &cairo::ImageSurface,
        px: f64,
        py: f64,
        scale: f64,
        w: f64,
        h: f64,
    ) {
        let side = LOUPE_SPAN * LOUPE_ZOOM;
        // Below and to the right of the pointer, or flipped near the edges.
        let bx = if px + 20.0 + side > w {
            px - 20.0 - side
        } else {
            px + 20.0
        };
        let by = if py + 20.0 + side > h {
            py - 20.0 - side
        } else {
            py + 20.0
        };
        cr.save().ok();
        cr.rectangle(bx, by, side, side);
        cr.clip();
        cr.translate(bx + side / 2.0, by + side / 2.0);
        cr.scale(LOUPE_ZOOM / scale, LOUPE_ZOOM / scale);
        cr.set_source_surface(surface, -px * scale, -py * scale)
            .ok();
        cr.source().set_filter(cairo::Filter::Nearest);
        cr.paint().ok();
        cr.restore().ok();
        // Crosshair on the pixel under the pointer, and a border.
        cr.set_source_rgba(1.0, 0.2, 0.2, 0.9);
        cr.set_line_width(1.0);
        let (cx, cy) = (bx + side / 2.0, by + side / 2.0);
        cr.rectangle(cx, cy, LOUPE_ZOOM, LOUPE_ZOOM);
        cr.stroke().ok();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.set_line_width(2.0);
        cr.rectangle(bx, by, side, side);
        cr.stroke().ok();
        // The pointer's position, in the capture's pixels.
        let label = format!("{}, {}", (px * scale).floor(), (py * scale).floor());
        self.tag(cr, &label, bx, by + side + 5.0, h);
    }
}
