//! Accessibility (docs/PLANO-SETTINGS.md §3.3): what core.yaml's
//! `accessibility:` turns on, all of it off by default. It works locked
//! too: the keys go through here before the greet or any client sees them.
//!
//! ```yaml
//! accessibility:
//!   sticky_keys: false   # a modifier pressed and let go alone stays held
//!                        # for the next key; pressed twice, held until
//!                        # pressed again
//!   sticky_keys_two_off: true  # two modifiers pressed together turn
//!                        # sticky keys off until the config is read again
//!   slow_keys: 0         # ms a key is held before it counts (0: off); let
//!                        # go sooner, it never happened
//!   bounce_keys: 0       # ms after a key is let go in which the same key
//!                        # again is ignored (0: off)
//!   mouse_keys: false    # with Num Lock off, the keypad moves the pointer
//!   zoom: 1.0            # the magnifier: more than 1 draws every monitor
//!                        # that much larger around the pointer
//!   visual_bell: false   # a program's bell flashes its monitor
//! ```
//!
//! Mouse keys go as X's MouseKeys and GNOME's: 1–4 and 6–9 move the
//! pointer that way, faster the longer they are held; 5 clicks, + clicks
//! twice, 0 holds the button down and . lets it go; / * and - pick the
//! left, middle or right button for them. Only with Num Lock off: on, the
//! keypad types its digits, so the same keypad does both.
//!
//! The magnifier (`zoom`, and the `zoom` command for the moment) draws each
//! monitor larger around the pointer, so the pointer stays where it is
//! drawn and what is under it is what it points at; going to an edge
//! brings that edge in view. Screen copies are of the monitor as it is,
//! not zoomed.

use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use serde::Deserialize;
use smithay::{
    backend::{
        input::{ButtonState, InputTime, KeyState},
        renderer::{
            element::{Id, Kind, solid::SolidColorRenderElement},
            utils::CommitCounter,
        },
    },
    input::keyboard::{KeyboardSource, Keycode},
    output::Output,
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Physical, Point, Rectangle},
    wayland::xdg_system_bell::XdgSystemBellHandler,
};

use crate::state::Orbit;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Accessibility {
    pub sticky_keys: bool,
    pub sticky_keys_two_off: bool,
    pub slow_keys: u64,
    pub bounce_keys: u64,
    pub mouse_keys: bool,
    pub zoom: f64,
    pub visual_bell: bool,
}

impl Default for Accessibility {
    fn default() -> Self {
        Self {
            sticky_keys: false,
            sticky_keys_two_off: true,
            slow_keys: 0,
            bounce_keys: 0,
            mouse_keys: false,
            zoom: 1.0,
            visual_bell: false,
        }
    }
}

/// XKB's keycodes are the kernel's plus 8.
const fn xkb(evdev: u32) -> u32 {
    evdev + 8
}

/// The modifier keys, by keycode: Ctrl, Shift, Alt, AltGr, Super.
const MODIFIERS: [u32; 8] = [
    xkb(29),
    xkb(97),
    xkb(42),
    xkb(54),
    xkb(56),
    xkb(100),
    xkb(125),
    xkb(126),
];

fn is_modifier(code: u32) -> bool {
    MODIFIERS.contains(&code)
}

/// A modifier sticky keys is holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    /// Down for real; `used` once another key went with it.
    Down { used: bool },
    /// Let go alone: held for the next key.
    Latched,
    /// Pressed twice: held until pressed again.
    Locked,
    /// Pressed again while locked: let go with this press's release.
    Unlocking,
}

/// Sticky keys: what reaches the keyboard of the keys pressed.
#[derive(Debug, Default)]
pub struct Sticky {
    mods: HashMap<u32, Held>,
}

impl Sticky {
    /// The keys (and whether down) the keyboard gets for `code` going down
    /// or up, in order.
    pub fn key(&mut self, code: u32, pressed: bool) -> Vec<(u32, bool)> {
        if is_modifier(code) {
            let held = self.mods.get(&code).copied();
            return match (pressed, held) {
                (true, None) => {
                    self.mods.insert(code, Held::Down { used: false });
                    vec![(code, true)]
                }
                (true, Some(Held::Latched)) => {
                    self.mods.insert(code, Held::Locked);
                    Vec::new()
                }
                (true, Some(Held::Locked)) => {
                    self.mods.insert(code, Held::Unlocking);
                    Vec::new()
                }
                (true, Some(_)) => vec![(code, true)],
                (false, Some(Held::Down { used: false })) => {
                    self.mods.insert(code, Held::Latched);
                    Vec::new()
                }
                (false, Some(Held::Down { used: true } | Held::Unlocking)) => {
                    self.mods.remove(&code);
                    vec![(code, false)]
                }
                (false, Some(Held::Latched | Held::Locked)) => Vec::new(),
                (false, None) => vec![(code, false)],
            };
        }
        self.mark();
        if pressed {
            return vec![(code, true)];
        }
        let mut out = vec![(code, false)];
        out.extend(self.used());
        out
    }

    /// Whether a modifier other than `code` is down for real now (someone
    /// who can press two together, as GNOME's "two keys off" reads it).
    pub fn other_down(&self, code: u32) -> bool {
        self.mods.iter().any(|(c, h)| *c != code && matches!(h, Held::Down { .. }))
    }

    /// The modifiers held for real went with another key or a click.
    pub fn mark(&mut self) {
        for held in self.mods.values_mut() {
            if let Held::Down { used } = held {
                *used = true;
            }
        }
    }

    /// A key or a click that went with the modifiers is over: those held
    /// for real are used, the latched ones go (their releases).
    pub fn used(&mut self) -> Vec<(u32, bool)> {
        self.mark();
        let mut out = Vec::new();
        let latched: Vec<u32> = self
            .mods
            .iter()
            .filter(|(_, h)| **h == Held::Latched)
            .map(|(c, _)| *c)
            .collect();
        for c in latched {
            self.mods.remove(&c);
            out.push((c, false));
        }
        out
    }

    /// Lets go what it holds (sticky keys turned off): the releases of the
    /// latched and locked modifiers. One still down for real is let go by
    /// its own release.
    pub fn release_all(&mut self) -> Vec<(u32, bool)> {
        let out = self
            .mods
            .iter()
            .filter(|(_, h)| matches!(h, Held::Latched | Held::Locked))
            .map(|(c, _)| (*c, false))
            .collect();
        self.mods.clear();
        out
    }
}

/// Bounce keys: a key pressed again too soon after its release is ignored,
/// and so is that press's release.
#[derive(Debug, Default)]
pub struct Bounce {
    released: HashMap<u32, u64>,
    dropped: HashSet<u32>,
}

impl Bounce {
    /// Whether `code` going down or up at `now` (ms) passes, with a window
    /// of `ms`.
    pub fn pass(&mut self, code: u32, pressed: bool, now: u64, ms: u64) -> bool {
        if pressed {
            if self.released.get(&code).is_some_and(|t| now.saturating_sub(*t) < ms) {
                self.dropped.insert(code);
                return false;
            }
            return true;
        }
        if self.dropped.remove(&code) {
            return false;
        }
        self.released.insert(code, now);
        true
    }
}

/// Slow keys: a press counts once the key has been held long enough.
#[derive(Debug, Default)]
pub struct Slow {
    /// Keys down, not yet counted, with the token of their timer.
    pending: HashMap<u32, u64>,
    accepted: HashSet<u32>,
    next: u64,
}

impl Slow {
    /// `code` went down: the token its timer hands back to `fire`.
    pub fn press(&mut self, code: u32) -> u64 {
        self.next += 1;
        self.pending.insert(code, self.next);
        self.next
    }

    /// The timer of `code` ran out: whether the press counts now (it is
    /// still the same press, still down).
    pub fn fire(&mut self, code: u32, token: u64) -> bool {
        if self.pending.get(&code) != Some(&token) {
            return false;
        }
        self.pending.remove(&code);
        self.accepted.insert(code);
        true
    }

    /// `code` went up: whether the release is passed on (not for a press
    /// that never counted).
    pub fn release(&mut self, code: u32) -> bool {
        if self.pending.remove(&code).is_some() {
            return false;
        }
        self.accepted.remove(&code);
        true
    }
}

/// What a keypad key does with mouse keys on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mouse {
    /// Moving that way starts (or another direction joins in).
    Move,
    Click,
    DoubleClick,
    Press,
    Release,
    /// The button 5, + 0 and . use from now on.
    Button(u32),
    /// A key of the keypad's that does nothing here (or a release).
    Nothing,
}

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;

/// The keypad's direction keys: which way each goes.
fn direction(code: u32) -> Option<(i8, i8)> {
    Some(match code.checked_sub(8)? {
        71 => (-1, -1),
        72 => (0, -1),
        73 => (1, -1),
        75 => (-1, 0),
        77 => (1, 0),
        79 => (-1, 1),
        80 => (0, 1),
        81 => (1, 1),
        _ => return None,
    })
}

/// Mouse keys' state: the direction keys held and since when, the button.
#[derive(Debug)]
pub struct MouseKeys {
    held: Vec<u32>,
    pub since: Option<u64>,
    pub button: u32,
    /// Keys whose press was taken: their release is too.
    taken: HashSet<u32>,
    /// The movement's timer is running.
    pub ticking: bool,
}

impl Default for MouseKeys {
    fn default() -> Self {
        Self {
            held: Vec::new(),
            since: None,
            button: BTN_LEFT,
            taken: HashSet::new(),
            ticking: false,
        }
    }
}

/// Milliseconds before a held direction key starts gliding, how long it
/// takes to reach full speed, and that speed (pixels a second): GNOME's
/// defaults.
const MOUSE_DELAY: u64 = 160;
const MOUSE_ACCEL: u64 = 1200;
const MOUSE_MAX: f64 = 750.0;
/// The slowest glide, so it moves at once after the delay.
const MOUSE_MIN: f64 = 40.0;

/// How fast the pointer glides `held` ms after a direction key went down.
pub fn mouse_speed(held: u64) -> f64 {
    if held < MOUSE_DELAY {
        return 0.0;
    }
    let ramp = (held - MOUSE_DELAY) as f64 / MOUSE_ACCEL as f64;
    (MOUSE_MAX * ramp.min(1.0)).max(MOUSE_MIN)
}

impl MouseKeys {
    /// `code` of the keypad going down or up at `now` (ms); `None`: not a
    /// key mouse keys takes, to be passed on.
    pub fn key(&mut self, code: u32, pressed: bool, now: u64) -> Option<Mouse> {
        let evdev = code.checked_sub(8)?;
        let ours = direction(code).is_some() || matches!(evdev, 76 | 78 | 82 | 83 | 98 | 55 | 74);
        if !pressed {
            if !self.taken.remove(&code) {
                return None;
            }
            self.held.retain(|c| *c != code);
            if self.held.is_empty() {
                self.since = None;
            }
            return Some(Mouse::Nothing);
        }
        if !ours {
            return None;
        }
        self.taken.insert(code);
        if direction(code).is_some() {
            if !self.held.contains(&code) {
                self.held.push(code);
            }
            self.since.get_or_insert(now);
            return Some(Mouse::Move);
        }
        Some(match evdev {
            76 => Mouse::Click,
            78 => Mouse::DoubleClick,
            82 => Mouse::Press,
            83 => Mouse::Release,
            98 => Mouse::Button(BTN_LEFT),
            55 => Mouse::Button(BTN_MIDDLE),
            74 => Mouse::Button(BTN_RIGHT),
            _ => Mouse::Nothing,
        })
    }

    /// Which way the keys held point, each axis -1, 0 or 1.
    pub fn heading(&self) -> (f64, f64) {
        let (mut x, mut y) = (0i32, 0i32);
        for code in &self.held {
            if let Some((dx, dy)) = direction(*code) {
                x += dx as i32;
                y += dy as i32;
            }
        }
        (x.signum() as f64, y.signum() as f64)
    }
}

/// The magnifier's largest zoom and each step in or out.
pub const ZOOM_MAX: f64 = 16.0;
const ZOOM_STEP: f64 = 1.25;
/// What turning the magnifier on gives, the first time.
const ZOOM_ON: f64 = 2.0;

/// A zoom kept sensible: 1 (off) to `ZOOM_MAX`.
pub fn clamp_zoom(zoom: f64) -> f64 {
    if !zoom.is_finite() || zoom < 1.0 {
        1.0
    } else {
        zoom.min(ZOOM_MAX)
    }
}

pub fn zoom_in(zoom: f64) -> f64 {
    clamp_zoom(zoom * ZOOM_STEP)
}

/// A step out; close to 1, off.
pub fn zoom_out(zoom: f64) -> f64 {
    let zoom = zoom / ZOOM_STEP;
    if zoom < 1.05 { 1.0 } else { clamp_zoom(zoom) }
}

/// The point of a monitor at `geo` the zoom grows from: the pointer when
/// it is on it, its middle otherwise; in the monitor's own coordinates.
pub fn zoom_origin(pointer: Option<Point<f64, Logical>>, geo: Rectangle<i32, Logical>) -> Point<f64, Logical> {
    let geo = geo.to_f64();
    match pointer {
        Some(p) if geo.contains(p) => p - geo.loc,
        _ => Point::from((geo.size.w / 2.0, geo.size.h / 2.0)),
    }
}

/// How long the visual bell's flash stays.
const BELL: Duration = Duration::from_millis(150);

/// The state of all of it.
#[derive(Debug)]
pub struct A11y {
    pub sticky: Sticky,
    /// Sticky keys turned off by two modifiers pressed together, until
    /// the config is read again.
    pub sticky_off: bool,
    pub bounce: Bounce,
    pub slow: Slow,
    pub mouse: MouseKeys,
    /// The magnifier now; the zoom it had last when on; and the config's,
    /// so a reload that changes it applies it.
    pub zoom: f64,
    pub zoom_last: f64,
    pub zoom_config: f64,
    /// The monitor flashing, and since when.
    pub bell: Option<(String, Instant)>,
    bell_id: Id,
}

impl A11y {
    pub fn new(config: &Accessibility) -> Self {
        let zoom = clamp_zoom(config.zoom);
        Self {
            sticky: Sticky::default(),
            sticky_off: false,
            bounce: Bounce::default(),
            slow: Slow::default(),
            mouse: MouseKeys::default(),
            zoom,
            zoom_last: if zoom > 1.0 { zoom } else { ZOOM_ON },
            zoom_config: zoom,
            bell: None,
            bell_id: Id::new(),
        }
    }
}

impl Orbit {
    fn a11y_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }

    /// A key from a keyboard, through bounce keys, slow keys, mouse keys
    /// and sticky keys, to the keyboard (`input`'s `feed_key`).
    pub fn a11y_key(&mut self, code: Keycode, pressed: bool, time: InputTime, source: KeyboardSource, remote: bool) {
        let config = self.config.accessibility.clone();
        let raw = code.raw();
        let now = self.a11y_ms();
        if config.bounce_keys > 0 && !self.a11y.bounce.pass(raw, pressed, now, config.bounce_keys) {
            return;
        }
        if config.slow_keys > 0 {
            if pressed {
                let token = self.a11y.slow.press(raw);
                let timer = Timer::from_duration(Duration::from_millis(config.slow_keys));
                let inserted = self.loop_handle.insert_source(timer, move |_, _, state| {
                    if state.a11y.slow.fire(raw, token) {
                        state.a11y_counted(Keycode::new(raw), true, InputTime::now(), source, remote);
                    }
                    TimeoutAction::Drop
                });
                if let Err(err) = inserted {
                    tracing::warn!("slow keys: {err}");
                }
                return;
            }
            if !self.a11y.slow.release(raw) {
                return;
            }
        }
        self.a11y_counted(code, pressed, time, source, remote);
    }

    /// A key that counts: mouse keys, then sticky keys.
    fn a11y_counted(&mut self, code: Keycode, pressed: bool, time: InputTime, source: KeyboardSource, remote: bool) {
        let config = &self.config.accessibility;
        let raw = code.raw();
        let num_lock = self.seat.get_keyboard().is_some_and(|k| k.modifier_state().num_lock);
        // A release whose press mouse keys took is its, Num Lock or not.
        if config.mouse_keys && (!num_lock || !pressed) {
            let now = self.a11y_ms();
            if let Some(action) = self.a11y.mouse.key(raw, pressed, now) {
                self.mouse_key(action);
                return;
            }
        }
        let sticky = config.sticky_keys && !self.a11y.sticky_off;
        if sticky && config.sticky_keys_two_off && pressed && is_modifier(raw) && self.a11y.sticky.other_down(raw) {
            // Two modifiers together: whoever types does not need it.
            tracing::info!("sticky keys off: two modifiers pressed together");
            self.a11y.sticky_off = true;
            for (raw, _) in self.a11y.sticky.release_all() {
                self.feed_key(Keycode::new(raw), KeyState::Released, time, source, remote);
            }
            self.feed_key(code, KeyState::Pressed, time, source, remote);
            self.spawn("notify-send -a Orbit -i preferences-desktop-accessibility 'Sticky keys off' 'Two modifier keys were pressed together. Orbit-Settings, Accessibility, turns them on again.'");
            return;
        }
        let keys = if sticky {
            self.a11y.sticky.key(raw, pressed)
        } else {
            vec![(raw, pressed)]
        };
        for (raw, pressed) in keys {
            let state = if pressed { KeyState::Pressed } else { KeyState::Released };
            self.feed_key(Keycode::new(raw), state, time, source, remote);
        }
    }

    fn mouse_key(&mut self, action: Mouse) {
        let button = self.a11y.mouse.button;
        let now = InputTime::now();
        match action {
            Mouse::Move => {
                self.mouse_glide(1.0);
                if !self.a11y.mouse.ticking {
                    self.a11y.mouse.ticking = true;
                    let tick = Duration::from_millis(16);
                    let inserted = self
                        .loop_handle
                        .insert_source(Timer::from_duration(tick), move |_, _, state| {
                            let Some(since) = state.a11y.mouse.since else {
                                state.a11y.mouse.ticking = false;
                                return TimeoutAction::Drop;
                            };
                            let speed = mouse_speed(state.a11y_ms().saturating_sub(since));
                            state.mouse_glide(speed * tick.as_secs_f64());
                            TimeoutAction::ToDuration(tick)
                        });
                    if let Err(err) = inserted {
                        self.a11y.mouse.ticking = false;
                        tracing::warn!("mouse keys: {err}");
                    }
                }
            }
            Mouse::Click => {
                self.button(button, ButtonState::Pressed, now);
                self.button(button, ButtonState::Released, now);
            }
            Mouse::DoubleClick => {
                for _ in 0..2 {
                    self.button(button, ButtonState::Pressed, now);
                    self.button(button, ButtonState::Released, now);
                }
            }
            Mouse::Press => self.button(button, ButtonState::Pressed, now),
            Mouse::Release => self.button(button, ButtonState::Released, now),
            Mouse::Button(b) => self.a11y.mouse.button = b,
            Mouse::Nothing => {}
        }
    }

    /// The pointer moved `pixels` the way the keypad's keys point.
    fn mouse_glide(&mut self, pixels: f64) {
        let (dx, dy) = self.a11y.mouse.heading();
        if pixels <= 0.0 || (dx == 0.0 && dy == 0.0) {
            return;
        }
        let Some(here) = self.seat.get_pointer().map(|p| p.current_location()) else {
            return;
        };
        let to = self.clamp(here + Point::from((dx * pixels, dy * pixels)));
        self.touched();
        self.pointer_to(to, InputTime::now());
    }

    /// A click went with the modifiers sticky keys holds: on its release,
    /// the latched ones go.
    pub fn sticky_used(&mut self, released: bool) {
        if !released {
            self.a11y.sticky.mark();
            return;
        }
        let time = InputTime::now();
        for (raw, _) in self.a11y.sticky.used() {
            self.feed_key(Keycode::new(raw), KeyState::Released, time, KeyboardSource::MAIN, false);
        }
    }

    /// The config was read again: what changed applies.
    pub fn a11y_reload(&mut self) {
        let config = self.config.accessibility.clone();
        self.a11y.sticky_off = false;
        if !config.sticky_keys {
            let time = InputTime::now();
            for (raw, _) in self.a11y.sticky.release_all() {
                self.feed_key(Keycode::new(raw), KeyState::Released, time, KeyboardSource::MAIN, false);
            }
        }
        let zoom = clamp_zoom(config.zoom);
        if zoom != self.a11y.zoom_config {
            self.a11y.zoom_config = zoom;
            self.set_zoom(zoom);
        }
    }

    /// The magnifier at `zoom` (1: off).
    pub fn set_zoom(&mut self, zoom: f64) {
        let zoom = clamp_zoom(zoom);
        if zoom > 1.0 {
            self.a11y.zoom_last = zoom;
        }
        if zoom != self.a11y.zoom {
            self.a11y.zoom = zoom;
            self.request_render();
        }
    }

    /// `zoom in|out|off|toggle|F`.
    pub fn zoom_command(&mut self, how: &str) -> Result<(), String> {
        let zoom = self.a11y.zoom;
        let to = match how {
            "in" => zoom_in(zoom),
            "out" => zoom_out(zoom),
            "off" => 1.0,
            "toggle" if zoom > 1.0 => 1.0,
            "toggle" => self.a11y.zoom_last,
            value => match value.parse::<f64>() {
                Ok(v) if (1.0..=ZOOM_MAX).contains(&v) => v,
                _ => return Err(format!("zoom: in, out, off, toggle or 1 to {ZOOM_MAX}, not {value}")),
            },
        };
        self.set_zoom(to);
        Ok(())
    }

    /// The visual bell's flash over `output`, while it lasts.
    pub fn bell_element(
        &mut self,
        output: &Output,
        size: smithay::utils::Size<i32, Physical>,
    ) -> Option<SolidColorRenderElement> {
        let (name, since) = self.a11y.bell.as_ref()?;
        if since.elapsed() >= BELL {
            self.a11y.bell = None;
            return None;
        }
        if *name != output.name() {
            return None;
        }
        let alpha = 0.3;
        Some(SolidColorRenderElement::new(
            self.a11y.bell_id.clone(),
            Rectangle::from_size(size),
            CommitCounter::default(),
            [alpha, alpha, alpha, alpha],
            Kind::Unspecified,
        ))
    }
}

impl XdgSystemBellHandler for Orbit {
    fn ring(&mut self, surface: Option<WlSurface>) {
        let window = surface.and_then(|s| self.window_for(&s));
        self.bell(window);
    }
}

impl Orbit {
    /// X programs' bells: XWayland (24.1) has no xdg-system-bell, so a
    /// connection of its own asks XKB for BellNotify and rings the X
    /// window's monitor. It ends with XWayland.
    pub fn watch_x11_bells(&mut self, display: u32) {
        use smithay::reexports::calloop::channel::{Event, channel};
        let (tx, rx) = channel::<u32>();
        let inserted = self.loop_handle.insert_source(rx, |event, _, state| {
            if let Event::Msg(id) = event {
                let window = state.space.elements().find(|w| w.x11_surface().is_some_and(|x| x.window_id() == id)).cloned();
                state.bell(window);
            }
        });
        if let Err(err) = inserted {
            tracing::warn!("X bells: {err}");
            return;
        }
        std::thread::spawn(move || {
            if let Err(err) = x11_bells(display, &tx) {
                tracing::warn!("X bells: {err}");
            }
        });
    }

    /// The visual bell on `window`'s monitor (or the pointer's).
    fn bell(&mut self, window: Option<smithay::desktop::Window>) {
        if !self.config.accessibility.visual_bell {
            return;
        }
        let output = window
            .and_then(|w| self.space.outputs_for_element(&w).into_iter().next())
            .or_else(|| self.output());
        let Some(output) = output else { return };
        tracing::debug!("visual bell on {}", output.name());
        self.a11y.bell = Some((output.name(), Instant::now()));
        self.request_render();
        let inserted = self
            .loop_handle
            .insert_source(Timer::from_duration(BELL), |_, _, state| {
                state.request_render();
                TimeoutAction::Drop
            });
        if let Err(err) = inserted {
            tracing::warn!("visual bell: {err}");
        }
    }
}

/// XKB's BellNotify on XWayland `display`, each one's window sent on.
fn x11_bells(display: u32, tx: &smithay::reexports::calloop::channel::Sender<u32>) -> Result<(), Box<dyn std::error::Error>> {
    use x11rb::{
        connection::Connection,
        protocol::{
            Event,
            xkb::{self, ConnectionExt as _},
        },
    };
    let (conn, _) = x11rb::rust_connection::RustConnection::connect(Some(&format!(":{display}")))?;
    conn.xkb_use_extension(1, 0)?.reply()?;
    conn.xkb_select_events(
        xkb::ID::USE_CORE_KBD.into(),
        xkb::EventType::from(0u16),
        xkb::EventType::BELL_NOTIFY,
        xkb::MapPart::from(0u16),
        xkb::MapPart::from(0u16),
        &xkb::SelectEventsAux::new(),
    )?
    .check()?;
    loop {
        if let Event::XkbBellNotify(bell) = conn.wait_for_event()?
            && tx.send(bell.window).is_err()
        {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIFT: u32 = xkb(42);
    const CTRL: u32 = xkb(29);
    const A: u32 = xkb(30);

    #[test]
    fn sticky_two_down() {
        let mut s = Sticky::default();
        let (ctrl, alt) = (xkb(29), xkb(56));
        s.key(ctrl, true);
        assert!(!s.other_down(ctrl));
        assert!(s.other_down(alt));
        s.key(ctrl, false); // latched, not down
        assert!(!s.other_down(alt));
    }

    #[test]
    fn sticky_latches_for_one_key() {
        let mut s = Sticky::default();
        assert_eq!(s.key(SHIFT, true), vec![(SHIFT, true)]);
        assert_eq!(s.key(SHIFT, false), vec![]);
        assert_eq!(s.key(A, true), vec![(A, true)]);
        assert_eq!(s.key(A, false), vec![(A, false), (SHIFT, false)]);
        // Gone: the next key is plain.
        assert_eq!(s.key(A, true), vec![(A, true)]);
        assert_eq!(s.key(A, false), vec![(A, false)]);
    }

    #[test]
    fn sticky_click_uses_latched() {
        let mut s = Sticky::default();
        s.key(SHIFT, true);
        s.key(SHIFT, false);
        s.mark();
        assert_eq!(s.used(), vec![(SHIFT, false)]);
        assert_eq!(s.used(), vec![]);
    }

    #[test]
    fn sticky_chord_is_plain() {
        let mut s = Sticky::default();
        s.key(CTRL, true);
        s.key(A, true);
        s.key(A, false);
        assert_eq!(s.key(CTRL, false), vec![(CTRL, false)]);
    }

    #[test]
    fn sticky_locks_on_twice() {
        let mut s = Sticky::default();
        s.key(SHIFT, true);
        s.key(SHIFT, false);
        assert_eq!(s.key(SHIFT, true), vec![]);
        assert_eq!(s.key(SHIFT, false), vec![]);
        // Locked: kept over several keys.
        assert_eq!(s.key(A, true), vec![(A, true)]);
        assert_eq!(s.key(A, false), vec![(A, false)]);
        assert_eq!(s.key(A, false).len(), 1);
        // Pressed again: let go.
        assert_eq!(s.key(SHIFT, true), vec![]);
        assert_eq!(s.key(SHIFT, false), vec![(SHIFT, false)]);
    }

    #[test]
    fn sticky_two_latched_and_release_all() {
        let mut s = Sticky::default();
        s.key(CTRL, true);
        s.key(CTRL, false);
        s.key(SHIFT, true);
        s.key(SHIFT, false);
        let mut out = s.key(A, false);
        out.sort();
        assert_eq!(out.len(), 3);
        s.key(CTRL, true);
        s.key(CTRL, false);
        assert_eq!(s.release_all(), vec![(CTRL, false)]);
        assert_eq!(s.release_all(), vec![]);
    }

    #[test]
    fn bounce_drops_a_quick_second_press() {
        let mut b = Bounce::default();
        assert!(b.pass(A, true, 0, 300));
        assert!(b.pass(A, false, 50, 300));
        assert!(!b.pass(A, true, 100, 300));
        assert!(!b.pass(A, false, 120, 300));
        assert!(b.pass(A, true, 400, 300));
        // Another key is not held back.
        assert!(b.pass(SHIFT, true, 60, 300));
    }

    #[test]
    fn slow_counts_only_held_keys() {
        let mut s = Slow::default();
        let t = s.press(A);
        assert!(!s.release(A));
        assert!(!s.fire(A, t));
        let t1 = s.press(A);
        assert!(!s.release(A));
        let t2 = s.press(A);
        // The first press's timer does not count the second.
        assert!(!s.fire(A, t1));
        assert!(s.fire(A, t2));
        assert!(s.release(A));
    }

    #[test]
    fn mouse_keys() {
        let mut m = MouseKeys::default();
        let kp8 = xkb(72);
        let kp6 = xkb(77);
        assert_eq!(m.key(kp8, true, 10), Some(Mouse::Move));
        assert_eq!(m.key(kp6, true, 20), Some(Mouse::Move));
        assert_eq!(m.heading(), (1.0, -1.0));
        assert_eq!(m.since, Some(10));
        assert_eq!(m.key(kp8, false, 30), Some(Mouse::Nothing));
        assert_eq!(m.heading(), (1.0, 0.0));
        assert_eq!(m.key(kp6, false, 40), Some(Mouse::Nothing));
        assert_eq!(m.since, None);
        assert_eq!(m.key(xkb(98), true, 0), Some(Mouse::Button(BTN_LEFT)));
        assert_eq!(m.key(xkb(74), true, 0), Some(Mouse::Button(BTN_RIGHT)));
        assert_eq!(m.key(xkb(76), true, 0), Some(Mouse::Click));
        // Not the keypad's: passed on, its release too.
        assert_eq!(m.key(A, true, 0), None);
        assert_eq!(m.key(A, false, 0), None);
    }

    #[test]
    fn mouse_speed_ramps() {
        assert_eq!(mouse_speed(0), 0.0);
        assert_eq!(mouse_speed(MOUSE_DELAY), MOUSE_MIN);
        assert!(mouse_speed(MOUSE_DELAY + 600) > mouse_speed(MOUSE_DELAY + 300));
        assert_eq!(mouse_speed(MOUSE_DELAY + MOUSE_ACCEL), MOUSE_MAX);
        assert_eq!(mouse_speed(10_000), MOUSE_MAX);
    }

    #[test]
    fn zoom_steps() {
        assert_eq!(zoom_in(1.0), 1.25);
        assert_eq!(zoom_out(1.25), 1.0);
        assert_eq!(zoom_out(1.0), 1.0);
        assert_eq!(zoom_in(15.0), ZOOM_MAX);
        assert_eq!(clamp_zoom(0.5), 1.0);
        assert_eq!(clamp_zoom(f64::NAN), 1.0);
        let geo = Rectangle::new((1920, 0).into(), (1920, 1080).into());
        assert_eq!(
            zoom_origin(Some((2000.0, 100.0).into()), geo),
            Point::from((80.0, 100.0))
        );
        assert_eq!(zoom_origin(Some((10.0, 10.0).into()), geo), Point::from((960.0, 540.0)));
        assert_eq!(zoom_origin(None, geo), Point::from((960.0, 540.0)));
    }
}
