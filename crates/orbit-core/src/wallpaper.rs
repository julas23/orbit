//! The wallpaper, as light as feh: one picture (or a colour) under the
//! layers and the windows of each monitor, and behind the greet while
//! locked — blurred and dimmed there, so the card reads over it.
//!
//! The picture is decoded on a thread, made the monitor's size in pixels
//! as `mode` says, blurred once for the lock, and both kept: nothing about it is done again until the
//! file, the mode or the monitor's size change. Until it is ready, and if
//! it cannot be read, the colour is there.
//!
//! In `core.yaml`:
//!
//! ```yaml
//! wallpaper:
//!   image: ~/Imagens/fundo.jpg   # empty: the colour alone
//!   mode: fill                   # fill, fit, center, stretch, tile
//!   color: "#1b1f27"             # under the picture and around it
//!   effect: aurora               # in place of the picture: aurora, ink,
//!                                # facets, fire, water (`effects`); none
//!   colors: ["#0b1026", "#1d4e89", "#14b8a6"]   # the effect's, 2 to 5
//!   speed: 1                     # how fast it moves (0: still)
//!   fps: 30                      # frames a second while it moves
//! outputs:
//!   HDMI-A-1:
//!     wallpaper: ~/Imagens/outro.png   # this monitor's own
//! ```

use std::{cell::RefCell, collections::HashMap, path::PathBuf};

use serde::Deserialize;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                Id, Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::SolidColorRenderElement,
            },
            gles::{GlesPixelProgram, GlesRenderer, element::PixelShaderElement},
            utils::CommitCounter,
        },
    },
    output::Output,
    reexports::calloop::{
        channel::{self, Sender},
        timer::{TimeoutAction, Timer},
    },
    utils::{Logical, Physical, Rectangle, Scale, Size, Transform},
};

use crate::{render::OutputElement, state::Orbit};

/// How much the locked screen darkens the picture behind the greet.
const LOCKED_VEIL: f32 = 0.4;
/// How much it blurs it, in pixels of the monitor at scale 1.
const LOCKED_BLUR: f32 = 24.0;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Wallpaper {
    /// The picture: a path (`~` for home); empty for the colour alone.
    pub image: String,
    pub mode: Mode,
    /// `#rrggbb`: under the picture, around it when it does not cover
    /// the monitor, and alone without one.
    pub color: String,
    /// Drawn in place of the picture, on the monitors without one of
    /// their own (`outputs.<name>.wallpaper`).
    pub effect: crate::effects::Effect,
    /// The effect's colours, `#rrggbb`; fewer than two: its own.
    pub colors: Vec<String>,
    /// How fast the effect moves; 0 keeps it still.
    pub speed: f32,
    /// Frames a second while it moves.
    pub fps: u32,
}

impl Default for Wallpaper {
    fn default() -> Self {
        Self {
            image: String::new(),
            mode: Mode::Fill,
            color: "#1b1f27".into(),
            effect: crate::effects::Effect::None,
            colors: Vec::new(),
            speed: 1.0,
            fps: 30,
        }
    }
}

/// How the picture meets the monitor (feh's `--bg-*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Covers it, the overflow cut evenly from both sides.
    #[default]
    Fill,
    /// Whole, as large as fits, the colour around.
    Fit,
    /// As it is, in the middle.
    Center,
    /// Stretched to it, proportions lost.
    Stretch,
    /// Repeated from the top left.
    Tile,
}

/// What a monitor's picture is made from.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Key {
    path: PathBuf,
    mode: Mode,
    size: Size<i32, Physical>,
}

/// A decoded picture, the monitor's size, in RGBA.
struct Made {
    output: String,
    key: Key,
    /// The picture, and the picture blurred.
    pixels: Option<(Vec<u8>, Vec<u8>)>,
}

/// One monitor's: what it shows, or is being made.
struct Slot {
    key: Key,
    /// The picture, and the picture blurred (for the lock).
    buffers: Option<(MemoryRenderBuffer, MemoryRenderBuffer)>,
}

#[derive(Default)]
pub struct Wallpapers {
    slots: HashMap<String, Slot>,
    /// The colour's element per monitor, kept so the damage tracker sees
    /// the same one frame after frame.
    colors: HashMap<String, (Id, [u8; 3])>,
    veils: HashMap<String, Id>,
    /// Where the threads send what they made (made the first time).
    sender: Option<Sender<Made>>,
    /// The effect's element per monitor, and the uniforms it was last
    /// given (time, fade, colours): a frame with nothing new is no damage.
    effects: HashMap<String, (crate::effects::Effect, PixelShaderElement, Given)>,
    /// The effect's clock, in seconds: it runs only while the effect is
    /// seen (`tick_effect`).
    time: f32,
    last_tick: Option<std::time::Instant>,
    /// Whether the timer that moves it is there.
    ticking: bool,
}

/// The uniforms an effect's element was last given: time, fade, colours.
type Given = (f32, f32, Vec<[u8; 3]>);

/// The cache, behind the state's back: drawing reads the state only.
pub type Cache = RefCell<Wallpapers>;

impl Orbit {
    /// The wallpaper of `output` (`blurred`, for the lock), topmost
    /// first, at `alpha`.
    pub fn wallpaper_elements(
        &self,
        output: &Output,
        renderer: &mut GlesRenderer,
        scale: Scale<f64>,
        alpha: f32,
        blurred: bool,
        elements: &mut Vec<OutputElement>,
    ) {
        let Some(geo) = self.space.output_geometry(output) else {
            return;
        };
        let name = output.name();
        let size = geo.size.to_physical_precise_round(scale);
        let config = &self.config.wallpaper;
        if let Some(program) = self.effect_program(output) {
            self.effect_element(output, geo.size, program, alpha, elements);
            return;
        }
        let image = self
            .config
            .outputs
            .get(&name)
            .and_then(|o| o.wallpaper.clone())
            .unwrap_or_else(|| config.image.clone());
        let mut cache = self.wallpapers.borrow_mut();
        if !image.trim().is_empty() {
            let key = Key {
                path: expand_home(image.trim()),
                mode: config.mode,
                size,
            };
            let current = cache.slots.get(&name).is_some_and(|s| s.key == key);
            if !current {
                cache.slots.insert(
                    name.clone(),
                    Slot {
                        key: key.clone(),
                        buffers: None,
                    },
                );
                let sender = cache.sender.get_or_insert_with(|| self.wallpaper_channel()).clone();
                make(name.clone(), key, scale.x, sender);
            }
            if let Some((sharp, soft)) = cache.slots.get(&name).and_then(|s| s.buffers.as_ref()) {
                let buffer = if blurred { soft } else { sharp };
                match MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    (0.0, 0.0),
                    buffer,
                    Some(alpha),
                    None,
                    Some(geo.size),
                    Kind::Unspecified,
                ) {
                    Ok(element) => elements.push(OutputElement::Memory(element)),
                    Err(err) => tracing::warn!("wallpaper on {name}: {err:?}"),
                }
            }
        } else {
            cache.slots.remove(&name);
        }
        let rgb = parse_rgb(&config.color).unwrap_or([0x1b, 0x1f, 0x27]);
        let id = match cache.colors.get(&name) {
            Some((id, last)) if *last == rgb => id.clone(),
            _ => {
                let id = Id::new();
                cache.colors.insert(name.clone(), (id.clone(), rgb));
                id
            }
        };
        let [r, g, b] = rgb.map(|c| c as f32 / 255.0);
        elements.push(OutputElement::Solid(SolidColorRenderElement::new(
            id,
            Rectangle::from_size(size),
            CommitCounter::default(),
            [r * alpha, g * alpha, b * alpha, alpha],
            Kind::Unspecified,
        )));
    }

    /// Behind the greet while locked, at `alpha`: the wallpaper under a
    /// dark veil — or black, as before, without a picture.
    pub fn locked_backdrop(
        &self,
        output: &Output,
        renderer: &mut GlesRenderer,
        scale: Scale<f64>,
        alpha: f32,
        elements: &mut Vec<OutputElement>,
    ) {
        let Some(geo) = self.space.output_geometry(output) else {
            return;
        };
        let size = geo.size.to_physical_precise_round(scale);
        let has_picture = self.effect_program(output).is_some()
            || !self.config.wallpaper.image.trim().is_empty()
            || self
                .config
                .outputs
                .get(&*output.name())
                .is_some_and(|o| o.wallpaper.as_deref().is_some_and(|w| !w.trim().is_empty()));
        let veil = if has_picture { LOCKED_VEIL * alpha } else { alpha };
        let id = self
            .wallpapers
            .borrow_mut()
            .veils
            .entry(output.name())
            .or_insert_with(Id::new)
            .clone();
        elements.push(OutputElement::Solid(SolidColorRenderElement::new(
            id,
            Rectangle::from_size(size),
            CommitCounter::default(),
            [0.0, 0.0, 0.0, veil],
            Kind::Unspecified,
        )));
        if has_picture {
            self.wallpaper_elements(output, renderer, scale, alpha, true, elements);
        }
    }

    /// The effect's program for `output`, when it shows one: there is an
    /// effect, the monitor has no picture of its own, and it compiled.
    fn effect_program(&self, output: &Output) -> Option<GlesPixelProgram> {
        let effect = self.config.wallpaper.effect;
        if effect == crate::effects::Effect::None || self.own_picture(output) {
            return None;
        }
        self.shaders.as_ref()?.as_ref()?.effects.get(effect).cloned()
    }

    /// Whether `output` has a picture of its own.
    fn own_picture(&self, output: &Output) -> bool {
        self.config
            .outputs
            .get(&*output.name())
            .and_then(|o| o.wallpaper.as_deref())
            .is_some_and(|w| !w.trim().is_empty())
    }

    /// The effect over `output`'s whole `size`, at `alpha`.
    fn effect_element(
        &self,
        output: &Output,
        size: Size<i32, Logical>,
        program: GlesPixelProgram,
        alpha: f32,
        elements: &mut Vec<OutputElement>,
    ) {
        let config = &self.config.wallpaper;
        let mut colors: Vec<[u8; 3]> = config.colors.iter().filter_map(|c| parse_rgb(c)).collect();
        if colors.len() < 2 {
            colors = config.effect.default_colors().to_vec();
        }
        colors.truncate(crate::effects::MOST);
        let mut cache = self.wallpapers.borrow_mut();
        if !cache.ticking {
            cache.ticking = true;
            self.start_effect_timer();
        }
        let time = cache.time;
        let area = Rectangle::from_size(size);
        let opaque = (alpha >= 1.0).then(|| vec![area]);
        let wanted = (time, alpha, colors);
        let effect = config.effect;
        let entry = cache.effects.entry(output.name());
        // Another effect: another program, so another element.
        let (_, element, last) = match entry {
            std::collections::hash_map::Entry::Occupied(slot) if slot.get().0 == effect => slot.into_mut(),
            entry => {
                let element = PixelShaderElement::new(
                    program,
                    area,
                    opaque.clone(),
                    1.0,
                    crate::effects::uniforms(wanted.0, wanted.1, &wanted.2),
                    Kind::Unspecified,
                );
                entry.insert_entry((effect, element, wanted.clone())).into_mut()
            }
        };
        element.resize(area, opaque);
        if *last != wanted {
            element.update_uniforms(crate::effects::uniforms(wanted.0, wanted.1, &wanted.2));
            *last = wanted;
        }
        elements.push(OutputElement::Border(element.clone()));
    }

    /// Moves the effect on, `fps` times a second while it is seen.
    fn start_effect_timer(&self) {
        let timer = Timer::from_duration(std::time::Duration::from_millis(30));
        let inserted = self
            .loop_handle
            .insert_source(timer, |_, _, state| TimeoutAction::ToDuration(state.tick_effect()));
        if let Err(err) = inserted {
            tracing::error!("wallpaper effect: {err}");
        }
    }

    /// One step of the effect's clock; how long until the next.
    fn tick_effect(&mut self) -> std::time::Duration {
        let config = &self.config.wallpaper;
        let now = std::time::Instant::now();
        let mut cache = self.wallpapers.borrow_mut();
        let since = cache
            .last_tick
            .replace(now)
            .map_or(0.0, |t| (now - t).as_secs_f32().min(0.25));
        if config.effect == crate::effects::Effect::None {
            cache.effects.clear();
            return std::time::Duration::from_millis(500);
        }
        drop(cache);
        // Seen: on a monitor of its own with no fullscreen window over it,
        // unlocked, with the effects on.
        let seen = config.speed > 0.0
            && !self.locked()
            && self.effects()
            && self
                .space
                .outputs()
                .any(|o| !self.own_picture(o) && self.fullscreen_on(o).is_none());
        if !seen {
            return std::time::Duration::from_millis(500);
        }
        let step = since * config.speed;
        let fps = config.fps.clamp(1, 240);
        self.wallpapers.borrow_mut().time += step;
        self.request_render();
        std::time::Duration::from_secs_f64(1.0 / fps as f64)
    }

    /// The channel the threads answer on: what they made goes in its slot
    /// (if still wanted) and is drawn.
    fn wallpaper_channel(&self) -> Sender<Made> {
        let (sender, source) = channel::channel::<Made>();
        let inserted = self.loop_handle.insert_source(source, |event, _, state| {
            let channel::Event::Msg(made) = event else { return };
            let mut cache = state.wallpapers.borrow_mut();
            let Some(slot) = cache.slots.get_mut(&made.output).filter(|s| s.key == made.key) else {
                return;
            };
            let Some((sharp, soft)) = made.pixels else { return };
            let size = made.key.size;
            let buffer = |pixels: &[u8]| {
                MemoryRenderBuffer::from_slice(pixels, Fourcc::Abgr8888, (size.w, size.h), 1, Transform::Normal, None)
            };
            slot.buffers = Some((buffer(&sharp), buffer(&soft)));
            drop(cache);
            state.request_render();
        });
        if let Err(err) = inserted {
            tracing::error!("wallpaper: {err}");
        }
        sender
    }
}

/// Decodes and sizes the picture on a thread of its own.
fn make(output: String, key: Key, scale: f64, sender: Sender<Made>) {
    let spawned = std::thread::Builder::new()
        .name("orbit-wallpaper".into())
        .spawn(move || {
            let started = std::time::Instant::now();
            let pixels = match image::open(&key.path) {
                Ok(picture) => {
                    let sharp = compose(&picture.to_rgba8(), key.mode, key.size);
                    let soft = blur(&sharp, key.size, LOCKED_BLUR * scale as f32);
                    Some((sharp, soft))
                }
                Err(err) => {
                    tracing::warn!("wallpaper {}: {err}", key.path.display());
                    None
                }
            };
            tracing::info!(
                "wallpaper {} for {output}, {}x{}, in {:?}",
                key.path.display(),
                key.size.w,
                key.size.h,
                started.elapsed()
            );
            let _ = sender.send(Made { output, key, pixels });
        });
    if let Err(err) = spawned {
        tracing::error!("wallpaper: {err}");
    }
}

/// `picture` laid on a transparent canvas of `size` as `mode` says; RGBA.
fn compose(picture: &image::RgbaImage, mode: Mode, size: Size<i32, Physical>) -> Vec<u8> {
    use image::imageops::{self, FilterType};
    let (w, h) = (size.w.max(1) as u32, size.h.max(1) as u32);
    let (pw, ph) = picture.dimensions();
    let mut canvas = image::RgbaImage::new(w, h);
    let filter = FilterType::Triangle;
    match mode {
        Mode::Stretch => canvas = imageops::resize(picture, w, h, filter),
        Mode::Fill | Mode::Fit => {
            let sx = w as f64 / pw as f64;
            let sy = h as f64 / ph as f64;
            let s = if mode == Mode::Fill { sx.max(sy) } else { sx.min(sy) };
            let (nw, nh) = (
                ((pw as f64 * s).round() as u32).max(1),
                ((ph as f64 * s).round() as u32).max(1),
            );
            let sized = imageops::resize(picture, nw, nh, filter);
            let x = (w as i64 - nw as i64) / 2;
            let y = (h as i64 - nh as i64) / 2;
            imageops::overlay(&mut canvas, &sized, x, y);
        }
        Mode::Center => {
            let x = (w as i64 - pw as i64) / 2;
            let y = (h as i64 - ph as i64) / 2;
            imageops::overlay(&mut canvas, picture, x, y);
        }
        Mode::Tile => {
            let mut y = 0;
            while y < h {
                let mut x = 0;
                while x < w {
                    imageops::overlay(&mut canvas, picture, x as i64, y as i64);
                    x += pw.max(1);
                }
                y += ph.max(1);
            }
        }
    }
    canvas.into_raw()
}

/// `pixels` (RGBA, `size`) blurred by `sigma` pixels; what the picture
/// leaves uncovered stays so.
fn blur(pixels: &[u8], size: Size<i32, Physical>, sigma: f32) -> Vec<u8> {
    let Some(picture) = image::RgbaImage::from_raw(size.w.max(1) as u32, size.h.max(1) as u32, pixels.to_vec()) else {
        return pixels.to_vec();
    };
    image::imageops::fast_blur(&picture, sigma).into_raw()
}

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix('~') {
        Some(rest) => std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(rest.trim_start_matches('/')),
        None => PathBuf::from(path),
    }
}

/// `#rrggbb` (or `rrggbb`).
fn parse_rgb(text: &str) -> Option<[u8; 3]> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(w: u32, h: u32) -> image::RgbaImage {
        image::RgbaImage::from_pixel(w, h, image::Rgba([255, 0, 0, 255]))
    }

    fn alpha_at(pixels: &[u8], w: i32, x: i32, y: i32) -> u8 {
        pixels[((y * w + x) * 4 + 3) as usize]
    }

    #[test]
    fn fill_covers_and_fit_leaves_bands() {
        let size = Size::from((100, 50));
        let filled = compose(&picture(40, 40), Mode::Fill, size);
        assert_eq!(filled.len(), 100 * 50 * 4);
        assert_eq!(alpha_at(&filled, 100, 0, 0), 255);
        assert_eq!(alpha_at(&filled, 100, 99, 49), 255);
        let fitted = compose(&picture(40, 40), Mode::Fit, size);
        // 50×50 in the middle: the sides are left to the colour.
        assert_eq!(alpha_at(&fitted, 100, 0, 25), 0);
        assert_eq!(alpha_at(&fitted, 100, 50, 25), 255);
    }

    #[test]
    fn center_and_tile() {
        let size = Size::from((10, 10));
        let centered = compose(&picture(4, 4), Mode::Center, size);
        assert_eq!(alpha_at(&centered, 10, 0, 0), 0);
        assert_eq!(alpha_at(&centered, 10, 5, 5), 255);
        let tiled = compose(&picture(4, 4), Mode::Tile, size);
        assert!(tiled.chunks(4).all(|p| p[3] == 255));
    }

    #[test]
    fn reads_colours_and_home() {
        assert_eq!(parse_rgb("#1b1f27"), Some([0x1b, 0x1f, 0x27]));
        assert_eq!(parse_rgb("bad"), None);
        assert!(!expand_home("~/x.png").starts_with("~"));
    }
}
