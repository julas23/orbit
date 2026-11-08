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
//!                                # facets, fire, water, borealis, stars,
//!                                # rain, nebula, galaxy, logo, scope
//!                                # (`effects`); none
//!   colors: ["#0b1026", "#1d4e89", "#14b8a6"]   # the effect's, 2 to 5
//!   speed: 1                     # how fast it moves (0: still)
//!   fps: 30                      # frames a second while it moves
//!   sound: false                 # the effect follows what the speakers play
//!   music: true                  # with scope: the song and its lyrics over it
//!   screensaver: 300             # seconds idle before the windows fade and
//!                                # the effect shows alone; 0: never
//!   video: ~/Vídeos/mar.mp4      # over all of it: a video or an animated
//!                                # GIF (orbit-wallpaper), silent, looping,
//!                                # laid as `mode` says (not tile)
//! outputs:
//!   HDMI-A-1:
//!     wallpaper: ~/Imagens/outro.png   # this monitor's own
//! ```
//!
//! The screensaver: with an effect on, after `screensaver` seconds with no
//! input (the same that the idle lock counts, and held off by the same:
//! a program's inhibitor, a fullscreen window), the effect is drawn over
//! everything, fading in over a few seconds, and the pointer hides. Any
//! input fades it out at once; a click or a key that wakes it is not
//! passed on (`input`), so nothing hidden is clicked or typed into.

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
    /// A video (or animated GIF) over every monitor, played by
    /// `orbit-wallpaper`; empty: none.
    pub video: String,
    /// The effect follows what the speakers play (`sound`).
    pub sound: bool,
    /// Seconds with no input before the effect shows alone; 0: never.
    pub screensaver: u64,
    /// With the `scope` effect, what is playing over it: the cover, the
    /// song and the lyrics (`orbit-wallpaper --music`).
    pub music: bool,
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
            video: String::new(),
            sound: false,
            screensaver: 300,
            music: true,
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
    /// The video's player, while there is one.
    video: Option<Video>,
    /// A video whose player ended by itself: not started again until the
    /// config is read again.
    failed: Option<VideoKey>,
    /// What the speakers play, while an effect wants it.
    sound: crate::sound::Sound,
    /// `orbit-wallpaper --music`, while the scope shows; whether it ended
    /// by itself (not started again until the config is read again).
    music: Option<std::process::Child>,
    music_failed: bool,
    /// The loudness that hurries the effect's clock, smoothed.
    loud: f32,
    /// How much the screensaver shows, 0..1.
    saver: f32,
}

/// Seconds the screensaver takes to come, and to go.
const SAVER_IN: f32 = 3.0;
const SAVER_OUT: f32 = 0.35;
/// From so much shown, a click or key that wakes it is not passed on.
const SAVER_SWALLOWS: f32 = 0.5;

/// What the video's player was started with: the file and the mode.
type VideoKey = (String, String);

struct Video {
    key: VideoKey,
    child: std::process::Child,
    /// What it was last told: playing, or paused.
    playing: bool,
}

/// The uniforms an effect's element was last given: time, fade, sound,
/// colours.
type Given = (f32, f32, crate::sound::Levels, Vec<[u8; 3]>);

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
            self.effect_element(output, "", geo.size, program, alpha, elements);
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

    /// How much the screensaver shows, 0..1.
    pub fn saver(&self) -> f32 {
        self.wallpapers.borrow().saver
    }

    /// Whether a click or key now would only wake the screensaver.
    pub fn saver_swallows(&self) -> bool {
        self.saver() >= SAVER_SWALLOWS
    }

    /// The screensaver over `output`, at `alpha`: the effect, drawn again
    /// over all (with an element of its own: the one under the windows
    /// stays as it is).
    pub fn saver_elements(&self, output: &Output, alpha: f32, elements: &mut Vec<OutputElement>) {
        let Some(geo) = self.space.output_geometry(output) else {
            return;
        };
        if let Some(program) = self.effect_program(output) {
            self.effect_element(output, "/saver", geo.size, program, alpha, elements);
        }
    }

    /// The effect's program for `output`, when it shows one: there is an
    /// effect, the monitor has no picture of its own, and it compiled.
    fn effect_program(&self, output: &Output) -> Option<GlesPixelProgram> {
        let effect = self.config.wallpaper.effect;
        // A video playing covers it: no frames drawn for nothing.
        let video = self.wallpapers.borrow().video.is_some();
        if effect == crate::effects::Effect::None || video || self.own_picture(output) {
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

    /// The effect over `output`'s whole `size`, at `alpha`; `slot` tells
    /// apart two of the same monitor.
    fn effect_element(
        &self,
        output: &Output,
        slot: &str,
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
        let wanted = (time, alpha, cache.sound.levels(), colors);
        let effect = config.effect;
        let entry = cache.effects.entry(format!("{}{slot}", output.name()));
        // Another effect: another program, so another element.
        let (_, element, last) = match entry {
            std::collections::hash_map::Entry::Occupied(slot) if slot.get().0 == effect => slot.into_mut(),
            entry => {
                let element = PixelShaderElement::new(
                    program,
                    area,
                    opaque.clone(),
                    1.0,
                    crate::effects::uniforms(wanted.0, wanted.1, &wanted.2, &wanted.3),
                    Kind::Unspecified,
                );
                entry.insert_entry((effect, element, wanted.clone())).into_mut()
            }
        };
        element.resize(area, opaque);
        if *last != wanted {
            element.update_uniforms(crate::effects::uniforms(wanted.0, wanted.1, &wanted.2, &wanted.3));
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

    /// The video's player started, changed or ended as the config says,
    /// and the timer that moves effects and pauses the video running.
    /// `read` after the config was read: a player that failed is tried again.
    pub fn sync_wallpaper(&mut self, read: bool) {
        let config = &self.config.wallpaper;
        let wanted = (!config.video.trim().is_empty()).then(|| {
            let mode = match config.mode {
                Mode::Fit => "fit",
                Mode::Center => "center",
                Mode::Stretch => "stretch",
                Mode::Fill | Mode::Tile => "fill",
            };
            (expand_home(config.video.trim()).display().to_string(), mode.to_owned())
        });
        let mut cache = self.wallpapers.borrow_mut();
        if read {
            cache.failed = None;
            cache.music_failed = false;
            cache.sound.retry();
        }
        // Ended by itself (a file it cannot play, a crash).
        let ended = cache.video.as_mut().and_then(|v| v.child.try_wait().ok().flatten());
        if let Some(status) = ended {
            let key = cache.video.take().map(|v| v.key);
            tracing::warn!(
                "wallpaper video {:?}: the player ended ({status})",
                key.as_ref().map(|k| &k.0)
            );
            cache.failed = key;
            self.needs_render = true;
        }
        if cache.video.as_ref().map(|v| &v.key) != wanted.as_ref() {
            if let Some(mut old) = cache.video.take() {
                // Its input closed, it ends; killed too, should it hang.
                drop(old.child.stdin.take());
                let _ = old.child.kill();
                std::thread::spawn(move || old.child.wait());
            }
            if let Some(key) = wanted.filter(|k| cache.failed.as_ref() != Some(k)) {
                let started = self
                    .client("orbit-wallpaper")
                    .arg("--mode")
                    .arg(&key.1)
                    .arg(&key.0)
                    .stdin(std::process::Stdio::piped())
                    .spawn();
                match started {
                    Ok(child) => {
                        tracing::info!("wallpaper video {}", key.0);
                        cache.video = Some(Video {
                            key,
                            child,
                            playing: true,
                        });
                    }
                    Err(err) => {
                        tracing::warn!("orbit-wallpaper: {err}");
                        cache.failed = Some(key);
                    }
                }
            }
        }
        // What is playing, over the spectrum sphere.
        if let Some(child) = cache.music.as_mut()
            && let Ok(Some(status)) = child.try_wait()
        {
            tracing::warn!("orbit-wallpaper --music ended ({status})");
            cache.music = None;
            cache.music_failed = true;
        }
        let music = config.music
            && config.effect == crate::effects::Effect::Scope
            && cache.video.is_none()
            && !cache.music_failed;
        match (music, cache.music.is_some()) {
            (true, false) => {
                let started = self
                    .client("orbit-wallpaper")
                    .arg("--music")
                    .stdin(std::process::Stdio::piped())
                    .spawn();
                match started {
                    Ok(child) => {
                        tracing::info!("wallpaper: what is playing, over the scope");
                        cache.music = Some(child);
                    }
                    Err(err) => {
                        tracing::warn!("orbit-wallpaper --music: {err}");
                        cache.music_failed = true;
                    }
                }
            }
            (false, true) => {
                if let Some(mut old) = cache.music.take() {
                    drop(old.stdin.take());
                    let _ = old.kill();
                    std::thread::spawn(move || old.wait());
                }
            }
            _ => {}
        }
        let timer = (cache.video.is_some() || config.effect != crate::effects::Effect::None) && !cache.ticking;
        if timer {
            cache.ticking = true;
            drop(cache);
            self.start_effect_timer();
        }
    }

    /// One step of the effect's clock, and the video paused or played;
    /// how long until the next.
    fn tick_effect(&mut self) -> std::time::Duration {
        self.sync_wallpaper(false);
        // Seen: some monitor with no fullscreen window over it, unlocked,
        // with the effects on.
        let seen = !self.locked()
            && self.effects()
            && self
                .space
                .outputs()
                .any(|o| !self.own_picture(o) && self.fullscreen_on(o).is_none());
        if let Some(video) = self.wallpapers.borrow_mut().video.as_mut()
            && video.playing != seen
            && let Some(input) = video.child.stdin.as_mut()
        {
            use std::io::Write;
            let word = if seen { "play\n" } else { "pause\n" };
            // A player gone is seen at the next step.
            let _ = input.write_all(word.as_bytes()).and_then(|()| input.flush());
            video.playing = seen;
        }
        let saver_wanted = self.saver_wanted(seen);
        let config = &self.config.wallpaper;
        let now = std::time::Instant::now();
        let fps = config.fps.clamp(1, 240);
        let frame = std::time::Duration::from_secs_f64(1.0 / fps as f64);
        let mut cache = self.wallpapers.borrow_mut();
        let since = cache
            .last_tick
            .replace(now)
            .map_or(0.0, |t| (now - t).as_secs_f32().min(0.25));
        let effect = config.effect != crate::effects::Effect::None && cache.video.is_none();
        cache
            .sound
            .run(effect && seen && (config.sound || config.effect.needs_sound()));
        if !effect {
            cache.effects.clear();
            cache.saver = 0.0;
            return std::time::Duration::from_millis(500);
        }
        // The screensaver comes slowly, and goes at once (gone at once
        // when locked: the greet is drawn instead).
        let saver = if self.locked() {
            0.0
        } else if saver_wanted {
            (cache.saver + since / SAVER_IN).min(1.0)
        } else {
            (cache.saver - since / SAVER_OUT).max(0.0)
        };
        let saver_moved = saver != cache.saver;
        cache.saver = saver;
        if !seen || config.speed <= 0.0 {
            drop(cache);
            if saver_moved {
                self.request_render();
            }
            // Shown, it must go as soon as the user is back.
            return if saver > 0.0 {
                frame
            } else {
                std::time::Duration::from_millis(500)
            };
        }
        // Louder, the clock runs a little faster.
        let levels = cache.sound.levels();
        let target = (levels.bass + levels.treble) * 0.5;
        cache.loud += (target - cache.loud) * (since * 4.0).min(1.0);
        let step = since * config.speed * (1.0 + cache.loud);
        cache.time += step;
        drop(cache);
        self.request_render();
        frame
    }

    /// Whether the screensaver should show: the user away long enough, and
    /// nothing holding it off (as the idle lock).
    fn saver_wanted(&mut self, seen: bool) -> bool {
        let seconds = self.config.wallpaper.screensaver;
        if !seen || seconds == 0 || self.locked() || self.idle.bus_inhibited {
            return false;
        }
        use smithay::reexports::wayland_server::Resource;
        self.idle.inhibitors.retain(|s| s.is_alive());
        if !self.idle.inhibitors.is_empty() || self.space.outputs().any(|o| self.fullscreen_on(o).is_some()) {
            return false;
        }
        self.idle.last_input.elapsed() >= std::time::Duration::from_secs(seconds)
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
