//! Aigent module: Orbit-Aigent's sphere.
//!
//!   Mini  (2 × 1)  the sphere, with the spectrum along its middle
//!   Full  (4 × 3)  larger, two fainter rings echoing its edge, and the
//!                  agent in use under it
//!
//! The sphere's edge moves with the sound, as a spectrum around a circle,
//! and the spectrum runs across it too. White while the daemon waits (and
//! thinks, with a sweep around the edge), red while it listens, yellow
//! while it speaks. A new colour reaches the edge first and the centre a
//! moment later, so a change sweeps inwards as a gradient, in about a
//! quarter of a second.
//!
//! The frames come from the daemon's socket (`orbit_aigent::Frame`), read
//! on a thread of their own; the GTK thread draws on its frame clock only
//! while something still moves. Without the daemon the sphere is dim.
//!
//! Left click: listen (or stop); while it asks for a confirmation, yes.
//! Right click: the agents to choose from, and cancel.

use std::{
    cell::{Cell, RefCell},
    io::{BufRead, BufReader},
    os::unix::net::UnixStream,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use gtk::{cairo, gio, glib, prelude::*};
use orbit_aigent::{BANDS, Command, Frame, State};
use orbit_app::i18n::tr;
use serde::Deserialize;

use crate::board::Form;

/// `modules/aigent.yaml`: nothing to set yet.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AigentConfig {}

const WHITE: Rgb = Rgb(0.94, 0.95, 0.97);
const RED: Rgb = Rgb(0.92, 0.20, 0.18);
const YELLOW: Rgb = Rgb(0.98, 0.85, 0.20);
/// How fast the edge and the centre take a new colour: 63% of the way in
/// this many seconds.
const EDGE_TAU: f64 = 0.045;
const CORE_TAU: f64 = 0.10;
/// The same for the bands, up and down.
const RISE_TAU: f64 = 0.03;
const FALL_TAU: f64 = 0.12;
/// The whole sphere without the daemon.
const OFFLINE_ALPHA: f64 = 0.35;
const PAD: f64 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Rgb(f64, f64, f64);

impl Rgb {
    fn towards(self, to: Rgb, k: f64) -> Rgb {
        Rgb(
            self.0 + (to.0 - self.0) * k,
            self.1 + (to.1 - self.1) * k,
            self.2 + (to.2 - self.2) * k,
        )
    }

    fn near(self, other: Rgb) -> bool {
        (self.0 - other.0).abs() + (self.1 - other.1).abs() + (self.2 - other.2).abs() < 0.01
    }

    fn set(self, cr: &cairo::Context, alpha: f64) {
        cr.set_source_rgba(self.0, self.1, self.2, alpha);
    }

    fn stop(self, gradient: &cairo::Gradient, offset: f64, alpha: f64) {
        gradient.add_color_stop_rgba(offset, self.0, self.1, self.2, alpha);
    }
}

/// What a message from the socket thread says.
enum Message {
    Frame(Frame),
    /// The daemon is gone (or never was).
    Gone,
}

/// What is drawn, moving towards the last frame.
struct Look {
    frame: Frame,
    online: bool,
    edge: Rgb,
    core: Rgb,
    bands: [f64; BANDS],
    alpha: f64,
    /// Seconds, for the turning rings and the sweep.
    phase: f64,
    /// The frame clock's last time, in µs.
    last: Option<i64>,
}

impl Look {
    fn target(&self) -> Rgb {
        match self.frame.state {
            State::Listening | State::Confirm => RED,
            State::Speaking => YELLOW,
            State::Idle | State::Thinking => WHITE,
        }
    }

    /// Moves everything `dt` seconds on; whether anything still moves.
    fn step(&mut self, dt: f64) -> bool {
        let target = self.target();
        self.edge = self.edge.towards(target, 1.0 - (-dt / EDGE_TAU).exp());
        self.core = self.core.towards(target, 1.0 - (-dt / CORE_TAU).exp());
        let alpha = if self.online { 1.0 } else { OFFLINE_ALPHA };
        self.alpha += (alpha - self.alpha) * (1.0 - (-dt / CORE_TAU).exp());
        let mut resting = true;
        for (k, shown) in self.bands.iter_mut().enumerate() {
            let to = self.frame.bands.get(k).map_or(0.0, |v| f64::from(*v));
            let tau = if to > *shown { RISE_TAU } else { FALL_TAU };
            *shown += (to - *shown) * (1.0 - (-dt / tau).exp());
            if *shown < 1e-3 && to == 0.0 {
                *shown = 0.0;
            }
            resting &= *shown == 0.0;
        }
        self.phase += dt;
        let settled = self.edge.near(target) && self.core.near(target) && (self.alpha - alpha).abs() < 0.01;
        !(settled && resting && self.frame.state != State::Thinking)
    }
}

pub fn new(_config: &AigentConfig, form: Form) -> gtk::Widget {
    let area = gtk::DrawingArea::builder()
        .css_classes(["aigent", form.class()])
        .hexpand(true)
        .vexpand(true)
        .build();
    let look = Rc::new(RefCell::new(Look {
        frame: Frame::default(),
        online: false,
        edge: WHITE,
        core: WHITE,
        bands: [0.0; BANDS],
        alpha: OFFLINE_ALPHA,
        phase: 0.0,
        last: None,
    }));
    {
        let look = look.clone();
        area.set_draw_func(move |_, cr, w, h| {
            if let Err(err) = draw(cr, &look.borrow(), form, f64::from(w), f64::from(h)) {
                log::warn!("aigent: drawing: {err}");
            }
        });
    }
    let agent = gtk::Label::builder()
        .css_classes(["aigent-agent"])
        .valign(gtk::Align::End)
        .halign(gtk::Align::Center)
        .can_target(false)
        .build();
    let root: gtk::Widget = if form == Form::Full {
        let overlay = gtk::Overlay::builder().child(&area).css_classes(["aigent-card"]).build();
        overlay.add_overlay(&agent);
        overlay.upcast()
    } else {
        area.clone().upcast()
    };
    root.set_tooltip_text(Some(&tr("aigent-offline")));

    // The frame clock runs only while something moves.
    let ticking = Rc::new(Cell::new(false));
    let animate = {
        let look = look.clone();
        let ticking = ticking.clone();
        let area = area.clone();
        move || {
            if ticking.replace(true) {
                return;
            }
            look.borrow_mut().last = None;
            let look = look.clone();
            let ticking = ticking.clone();
            area.add_tick_callback(move |area, clock| {
                let now = clock.frame_time();
                let mut look = look.borrow_mut();
                let dt = look.last.map_or(0.0, |last| (now - last) as f64 / 1e6).min(0.1);
                look.last = Some(now);
                let moving = look.step(dt);
                area.queue_draw();
                if moving {
                    glib::ControlFlow::Continue
                } else {
                    ticking.set(false);
                    glib::ControlFlow::Break
                }
            });
        }
    };

    let latest: Latest = Arc::default();
    let (wake, woken) = async_channel::bounded::<()>(1);
    {
        let latest = latest.clone();
        std::thread::Builder::new()
            .name("aigent".into())
            .spawn(move || listen(&latest, &wake))
            .expect("a thread");
    }
    {
        let look = look.clone();
        let weak = root.downgrade();
        let agent = agent.clone();
        glib::spawn_future_local(async move {
            while woken.recv().await.is_ok() {
                let Some(root) = weak.upgrade() else { break };
                let Some(message) = latest.lock().unwrap().take() else { continue };
                let mut look = look.borrow_mut();
                match message {
                    Message::Frame(frame) => {
                        look.online = true;
                        agent.set_label(&frame.agent);
                        let tip = if frame.text.is_empty() {
                            format!("{} · {}", tr("panel-module-aigent"), frame.agent)
                        } else {
                            format!("{} · {}\n{}", tr("panel-module-aigent"), frame.agent, frame.text)
                        };
                        root.set_tooltip_text(Some(&tip));
                        look.frame = frame;
                    }
                    Message::Gone => {
                        look.online = false;
                        look.frame = Frame::default();
                        agent.set_label("");
                        root.set_tooltip_text(Some(&tr("aigent-offline")));
                    }
                }
                drop(look);
                animate();
            }
        });
    }

    let click = gtk::GestureClick::builder().button(0).build();
    {
        let look = look.clone();
        let root = root.downgrade();
        click.connect_released(move |gesture, _, x, y| {
            let Some(root) = root.upgrade() else { return };
            let look = look.borrow();
            if !look.online {
                return;
            }
            match gesture.current_button() {
                1 if look.frame.state == State::Confirm => send(&Command::Yes),
                1 => send(&Command::Toggle),
                3 => menu(&root, &look.frame, (x, y)),
                _ => {}
            }
        });
    }
    root.add_controller(click);
    root.add_css_class("clickable");
    root.set_cursor_from_name(Some("pointer"));
    root
}

/// The message not handled yet.
type Latest = Arc<Mutex<Option<Message>>>;

/// Reads the daemon's frames until the module is gone; connects again
/// every 2 s while the daemon is not there.
fn listen(latest: &Latest, wake: &async_channel::Sender<()>) {
    let tell = |message: Message| {
        latest.lock().unwrap().replace(message);
        !matches!(wake.try_send(()), Err(async_channel::TrySendError::Closed(())))
    };
    let mut online = true;
    loop {
        if let Ok(stream) = UnixStream::connect(orbit_aigent::socket_path()) {
            online = true;
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                match serde_json::from_str::<Frame>(&line) {
                    Ok(frame) => {
                        if !tell(Message::Frame(frame)) {
                            return;
                        }
                    }
                    Err(err) => log::warn!("aigent: the daemon sent {line:?}: {err}"),
                }
            }
        }
        // Said once, not every 2 s.
        if online {
            online = false;
            if !tell(Message::Gone) {
                return;
            }
        }
        if wake.is_closed() {
            return;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn send(command: &Command) {
    if let Err(err) = orbit_aigent::send(command) {
        log::warn!("aigent: sending {command:?}: {err}");
    }
}

/// The right-click menu: the agents, and cancel while busy.
fn menu(root: &gtk::Widget, frame: &Frame, at: (f64, f64)) {
    let model = gio::Menu::new();
    let group = gio::SimpleActionGroup::new();
    let agents = gio::Menu::new();
    let choose = gio::SimpleAction::new_stateful("agent", Some(glib::VariantTy::STRING), &frame.agent.to_variant());
    choose.connect_change_state(|_, value| {
        if let Some(name) = value.and_then(|v| v.get::<String>()) {
            send(&Command::Agent { name });
        }
    });
    group.add_action(&choose);
    for name in &frame.agents {
        let item = gio::MenuItem::new(Some(name), None);
        item.set_action_and_target_value(Some("aigent.agent"), Some(&name.to_variant()));
        agents.append_item(&item);
    }
    model.append_section(Some(&tr("aigent-agents")), &agents);
    if frame.state != State::Idle {
        let cancel = gio::SimpleAction::new("cancel", None);
        cancel.connect_activate(|_, _| send(&Command::Cancel));
        group.add_action(&cancel);
        model.append(Some(&tr("aigent-cancel")), Some("aigent.cancel"));
    }
    let popover = gtk::PopoverMenu::from_model(Some(&model));
    popover.insert_action_group("aigent", Some(&group));
    orbit_app::popup::open(&popover, root, Some(at));
}

/// The band at `u`, 0 to 1 along the bands, between the two nearest.
fn band_at(bands: &[f64; BANDS], u: f64) -> f64 {
    let x = u.clamp(0.0, 1.0) * (BANDS - 1) as f64;
    let i = x.floor() as usize;
    let j = (i + 1).min(BANDS - 1);
    bands[i] + (bands[j] - bands[i]) * (x - i as f64)
}

/// A closed smooth path through `points` (Catmull-Rom as Bézier curves).
fn closed_curve(cr: &cairo::Context, points: &[(f64, f64)]) {
    let n = points.len();
    let Some(&(x0, y0)) = points.first() else { return };
    cr.move_to(x0, y0);
    for i in 0..n {
        let p0 = points[(i + n - 1) % n];
        let (p1, p2) = (points[i], points[(i + 1) % n]);
        let p3 = points[(i + 2) % n];
        cr.curve_to(
            p1.0 + (p2.0 - p0.0) / 6.0,
            p1.1 + (p2.1 - p0.1) / 6.0,
            p2.0 - (p3.0 - p1.0) / 6.0,
            p2.1 - (p3.1 - p1.1) / 6.0,
            p2.0,
            p2.1,
        );
    }
    cr.close_path();
}

/// The edge around (`cx`, `cy`): radius `r`, pushed out by the bands up to
/// `amp`, the low bands at the sides, the high ones at the top and bottom,
/// the pattern turned by `turn` radians.
fn edge_points(look: &Look, cx: f64, cy: f64, r: f64, amp: f64, turn: f64, n: usize) -> Vec<(f64, f64)> {
    use std::f64::consts::{PI, TAU};
    // Still, it breathes a little.
    let breath = 0.4 * (look.phase * 1.6).sin();
    (0..n)
        .map(|j| {
            let theta = TAU * j as f64 / n as f64;
            // Folded into a quarter so the pattern is the same left and
            // right, top and bottom, as a spectrum mirrored around.
            let folded = (theta + turn).rem_euclid(PI);
            let u = 1.0 - (folded - PI / 2.0).abs() / (PI / 2.0);
            let radius = r + breath + amp * band_at(&look.bands, u);
            (cx + radius * theta.cos(), cy + radius * theta.sin())
        })
        .collect()
}

fn draw(cr: &cairo::Context, look: &Look, form: Form, w: f64, h: f64) -> Result<(), cairo::Error> {
    use std::f64::consts::TAU;
    let full = form == Form::Full;
    let (cx, cy) = (w / 2.0, if full { h / 2.0 - 6.0 } else { h / 2.0 });
    let r = if full { h * 0.27 } else { h * 0.33 };
    let amp = r * if full { 0.38 } else { 0.42 };
    let alpha = look.alpha;

    // The spectrum across, behind the sphere: mirrored up and down, the
    // low bands next to the sphere, fading out to the sides.
    let reach = (w / 2.0 - PAD).max(1.0);
    let room = if full { r * 1.05 } else { h / 2.0 - PAD };
    let per_side = if full { BANDS } else { BANDS / 2 };
    let step = reach / per_side as f64;
    let bar = (step * 0.6).max(1.0);
    for side in [-1.0, 1.0] {
        for k in 0..per_side {
            let u = k as f64 / (per_side - 1).max(1) as f64;
            let v = band_at(&look.bands, u);
            let x = cx + side * (k as f64 + 0.5) * step;
            let tall = (v * room).max(0.5);
            let fade = 1.0 - 0.75 * u;
            look.edge.set(cr, alpha * 0.7 * fade);
            cr.rectangle(x - bar / 2.0, cy - tall, bar, 2.0 * tall);
        }
        cr.fill()?;
    }

    // The body: lit from the top left, the centre's colour inside and the
    // edge's at the rim, so a new colour sweeps in from the edge.
    let body = cairo::RadialGradient::new(cx - r * 0.3, cy - r * 0.35, r * 0.05, cx, cy, r);
    look.core.stop(&body, 0.0, alpha * 0.95);
    look.core.stop(&body, 0.55, alpha * 0.55);
    look.edge.stop(&body, 1.0, alpha * 0.30);
    cr.set_source(&body)?;
    cr.arc(cx, cy, r, 0.0, TAU);
    cr.fill()?;

    let n = if full { 96 } else { 48 };
    let turn = look.phase * 0.35;
    if full {
        // Two fainter rings, turning the other ways.
        for (scale, offset, a, width) in [(0.6, 1.3 - turn * 2.0, 0.45, 1.0), (0.35, -2.1 + turn * 0.6, 0.3, 0.8)] {
            closed_curve(cr, &edge_points(look, cx, cy, r, amp * scale, offset, n));
            look.edge.set(cr, alpha * a);
            cr.set_line_width(width);
            cr.stroke()?;
        }
    }
    let points = edge_points(look, cx, cy, r, amp, turn, n);
    // A glow, then the line.
    closed_curve(cr, &points);
    look.edge.set(cr, alpha * 0.18);
    cr.set_line_width(if full { 5.0 } else { 3.0 });
    cr.stroke_preserve()?;
    look.edge.set(cr, alpha);
    cr.set_line_width(if full { 1.6 } else { 1.0 });
    cr.stroke()?;

    // Thinking: a bright arc running round the edge, its tail fading.
    if look.frame.state == State::Thinking {
        let head = look.phase * 3.2;
        let pieces = 12;
        let length = 1.4;
        for i in 0..pieces {
            let from = head - length * (i + 1) as f64 / pieces as f64;
            let to = head - length * i as f64 / pieces as f64;
            look.edge.set(cr, alpha * (1.0 - i as f64 / pieces as f64));
            cr.set_line_width(if full { 2.4 } else { 1.6 });
            cr.arc(cx, cy, r + 1.5, from, to);
            cr.stroke()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(state: State, bands: f32) -> Look {
        Look {
            frame: Frame {
                state,
                bands: (0..BANDS).map(|k| bands * (0.4 + 0.6 * ((k as f32) * 0.7).sin().abs())).collect(),
                ..Frame::default()
            },
            online: true,
            edge: WHITE,
            core: WHITE,
            bands: [0.0; BANDS],
            alpha: 1.0,
            phase: 0.0,
            last: None,
        }
    }

    #[test]
    fn a_new_colour_reaches_the_edge_first_and_settles_fast() {
        let mut l = look(State::Listening, 0.0);
        l.step(0.06);
        let red_edge = (l.edge.1 - RED.1).abs();
        let red_core = (l.core.1 - RED.1).abs();
        assert!(red_edge < red_core, "edge {:?}, core {:?}", l.edge, l.core);
        // About half a second later it is all red, and still.
        let mut moving = true;
        for _ in 0..30 {
            moving = l.step(1.0 / 60.0);
        }
        assert!(l.edge.near(RED) && l.core.near(RED), "{:?} {:?}", l.edge, l.core);
        assert!(!moving);
    }

    #[test]
    fn thinking_keeps_moving() {
        let mut l = look(State::Thinking, 0.0);
        for _ in 0..120 {
            l.step(1.0 / 60.0);
        }
        assert!(l.step(1.0 / 60.0));
    }

    #[test]
    fn the_edge_is_mirrored() {
        let mut l = look(State::Speaking, 0.8);
        for _ in 0..30 {
            l.step(1.0 / 60.0);
        }
        let points = edge_points(&l, 0.0, 0.0, 10.0, 4.0, 0.0, 8);
        let radius = |(x, y): (f64, f64)| x.hypot(y);
        // Right and left, top and bottom.
        assert!((radius(points[0]) - radius(points[4])).abs() < 1e-9);
        assert!((radius(points[2]) - radius(points[6])).abs() < 1e-9);
    }

    /// Mini and Full in each state, drawn to `$ORBIT_AIGENT_PPM`, 4× (a
    /// look without a panel): `ORBIT_AIGENT_PPM=x.ppm cargo test -p
    /// orbit-panel aigent::tests::draws -- --ignored`.
    #[test]
    #[ignore]
    fn draws_the_states() {
        let Ok(path) = std::env::var("ORBIT_AIGENT_PPM") else { return };
        let (scale, gap) = (4.0, 10.0);
        let states = [
            (State::Idle, 0.0, 0.0),
            (State::Listening, 0.9, 0.0),
            (State::Thinking, 0.0, 0.3),
            (State::Speaking, 0.9, 1.1),
        ];
        let (fw, fh, mw, mh) = (150.0, 110.0, 70.0, 30.0);
        let width = gap + states.len() as f64 * (fw + gap);
        let height = gap + fh + gap + mh + gap;
        let mut surface =
            cairo::ImageSurface::create(cairo::Format::ARgb32, (width * scale) as i32, (height * scale) as i32).unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        cr.scale(scale, scale);
        cr.set_source_rgb(0.08, 0.09, 0.11);
        cr.paint().unwrap();
        for (i, (state, loud, phase)) in states.into_iter().enumerate() {
            let mut l = look(state, loud);
            let target = l.target();
            (l.edge, l.core) = (target, target);
            for _ in 0..30 {
                l.step(1.0 / 60.0);
            }
            l.phase = phase;
            let x = gap + i as f64 * (fw + gap);
            for (form, w, h, y) in [(Form::Full, fw, fh, gap), (Form::Mini, mw, mh, gap + fh + gap)] {
                cr.save().unwrap();
                cr.translate(x, y);
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.05);
                cr.rectangle(0.0, 0.0, w, h);
                cr.fill().unwrap();
                draw(&cr, &l, form, w, h).unwrap();
                cr.restore().unwrap();
            }
        }
        drop(cr);
        surface.flush();
        let (w, h, stride) = (surface.width() as usize, surface.height() as usize, surface.stride() as usize);
        let data = surface.data().unwrap();
        let mut ppm = format!("P6 {w} {h} 255\n").into_bytes();
        for y in 0..h {
            for x in 0..w {
                let p = &data[y * stride + x * 4..][..4];
                ppm.extend([p[2], p[1], p[0]]);
            }
        }
        std::fs::write(path, ppm).unwrap();
    }
}
