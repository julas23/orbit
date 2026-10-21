//! Spectrum module: what the speakers play, as CAVA draws it.
//!
//!   Mini    (2 × 1)  one line across the bands
//!   Normal  (4 × 2)  12 bars
//!   Full    (4 × 2)  thin bars in a red–orange–yellow gradient, on a grid
//!
//! The sound is the default sink's monitor, recorded through PulseAudio
//! (`pulse::record_monitor`, served by pipewire-pulse) on a thread of its
//! own, mono at 44.1 kHz, in blocks of one frame. The thread analyses it
//! as CAVA does: a Hann window over the last 2048 samples (46 ms), an FFT,
//! bands spaced evenly on a log scale between `low` and `high`, weighted up
//! with frequency (music is louder low down), a sensitivity that adjusts
//! itself so the loudest bar just reaches the top, and bars that fall with
//! gravity (`noise_reduction`). Unlike CAVA's integral filter, nothing
//! slows a bar on its way up: it follows the sound at once, which is what
//! keeps it in step with what is heard. It hands the GTK thread the newest
//! bars `framerate` times a second (a frame not drawn yet is replaced, never
//! queued), and nothing while all of them rest at zero.
//!
//! The line and the 12 bars take the module's CSS `color` (the accent);
//! the Full bars' gradient is fixed.

use std::{
    cell::RefCell,
    f32::consts::PI,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use gtk::{cairo, prelude::*};
use serde::Deserialize;

use crate::{board::Form, pulse};

/// `modules/spectrum.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpectrumConfig {
    /// Frames a second.
    pub framerate: u32,
    /// The lowest and highest frequencies shown, in Hz.
    pub low: f32,
    pub high: f32,
    /// As CAVA's `noise_reduction`, but for the fall only: 0 drops at
    /// once, 1 floats down slowly.
    pub noise_reduction: f32,
}

impl Default for SpectrumConfig {
    fn default() -> Self {
        Self {
            framerate: 60,
            low: 50.0,
            high: 10_000.0,
            noise_reduction: 0.77,
        }
    }
}

const RATE: u32 = 44_100;
/// Samples per FFT: 46 ms, bins 21.5 Hz apart.
const FFT_SIZE: usize = 2048;
/// The line's points, and the Full bars.
const LINE_POINTS: usize = 24;
const NORMAL_BARS: usize = 12;
const FULL_BARS: usize = 47;
/// Below this peak (−60 dB) a block is silence: the monitor carries a
/// faint hiss even when nothing plays, which the sensitivity would raise
/// into dancing bars.
const SILENCE: f32 = 1e-3;
/// Inside the card.
const PAD: f64 = 4.0;
const NORMAL_GAP: f64 = 3.0;
const FULL_GAP: f64 = 1.0;
/// The Full gradient, top to bottom.
const RED: (f64, f64, f64) = (0.92, 0.20, 0.18);
const ORANGE: (f64, f64, f64) = (0.98, 0.55, 0.12);
const YELLOW: (f64, f64, f64) = (0.98, 0.85, 0.20);

pub fn new(config: &SpectrumConfig, form: Form) -> gtk::Widget {
    let bands = match form {
        Form::Mini | Form::Line => LINE_POINTS,
        Form::Normal => NORMAL_BARS,
        Form::Full => FULL_BARS,
    };
    let area = gtk::DrawingArea::builder()
        .css_classes(["spectrum", form.class()])
        .hexpand(true)
        .vexpand(true)
        .build();
    if form == Form::Line {
        area.set_size_request(70, 24);
    }
    let bars = Rc::new(RefCell::new(vec![0.0f32; bands]));
    {
        let bars = bars.clone();
        area.set_draw_func(move |area, cr, w, h| {
            let bars = bars.borrow();
            let colour = area.color();
            let (w, h) = (f64::from(w), f64::from(h));
            let drawn = match form {
                Form::Mini | Form::Line => draw_line(cr, &bars, w, h, &colour),
                Form::Normal => draw_bars(cr, &bars, w, h, &colour),
                Form::Full => draw_full(cr, &bars, w, h, &colour),
            };
            if let Err(err) = drawn {
                log::warn!("spectrum: drawing: {err}");
            }
        });
    }

    // The newest bars wait in `latest`; the channel only wakes the GTK
    // thread, once however many frames came meanwhile.
    let latest: Latest = Arc::default();
    let (wake, woken) = async_channel::bounded::<()>(1);
    let config = config.clone();
    {
        let latest = latest.clone();
        std::thread::Builder::new()
            .name("spectrum".into())
            .spawn(move || listen(&config, bands, &latest, &wake))
            .expect("a thread");
    }
    let weak = area.downgrade();
    gtk::glib::spawn_future_local(async move {
        while woken.recv().await.is_ok() {
            let Some(area) = weak.upgrade() else { break };
            if let Some(frame) = latest.lock().unwrap().take() {
                bars.replace(frame);
                area.queue_draw();
            }
        }
    });
    area.upcast()
}

/// The bars not drawn yet.
type Latest = Arc<Mutex<Option<Vec<f32>>>>;

/// Records and analyses until the module is gone; records again when the
/// sound server or the default sink changes.
fn listen(config: &SpectrumConfig, bands: usize, latest: &Latest, wake: &async_channel::Sender<()>) {
    let fps = config.framerate.clamp(1, 144);
    let mut analyzer = Analyzer::new(bands, config.low, config.high, config.noise_reduction, fps);
    let per_frame = (RATE / fps) as usize;
    loop {
        let mut since = 0;
        let mut resting = false;
        let recorded = pulse::record_monitor(RATE, per_frame as u32, |samples| {
            analyzer.push(samples);
            since += samples.len();
            if since < per_frame {
                return true;
            }
            since = 0;
            let frame = analyzer.frame();
            let rest = frame.iter().all(|v| *v <= 0.0);
            if rest && resting {
                return !wake.is_closed();
            }
            resting = rest;
            latest.lock().unwrap().replace(frame);
            match wake.try_send(()) {
                Ok(()) | Err(async_channel::TrySendError::Full(())) => true,
                Err(async_channel::TrySendError::Closed(())) => false,
            }
        });
        match recorded {
            Ok(pulse::Stop::Done) => return,
            Ok(pulse::Stop::Again) => {}
            Err(err) => {
                log::warn!("spectrum: recording: {err}; again in 2 s");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
        if wake.is_closed() {
            return;
        }
    }
}

/// CAVA's analysis, for a fixed number of bands.
struct Analyzer {
    /// The last `FFT_SIZE` samples, oldest first from `head`.
    ring: Vec<f32>,
    head: usize,
    window: Vec<f32>,
    /// Each band's FFT bins, and its weight.
    ranges: Vec<(usize, usize)>,
    eq: Vec<f32>,
    sens: f32,
    /// Raising the sensitivity fast, until the bars first reach the top.
    sens_init: bool,
    noise_reduction: f32,
    gravity: f32,
    /// Per band: the peak it falls from, how far it has fallen, and the
    /// last output.
    peak: Vec<f32>,
    fall: Vec<f32>,
    prev: Vec<f32>,
    /// Whether the last block was silence (`SILENCE`).
    silent: bool,
}

impl Analyzer {
    fn new(bands: usize, low: f32, high: f32, noise_reduction: f32, fps: u32) -> Self {
        let high = high.min(RATE as f32 / 2.0).max(low * 2.0);
        let bin = |f: f32| ((f * FFT_SIZE as f32 / RATE as f32).round() as usize).clamp(1, FFT_SIZE / 2 - 1);
        let mut ranges = Vec::with_capacity(bands);
        let mut eq = Vec::with_capacity(bands);
        let mut from = bin(low);
        for k in 0..bands {
            let edge = low * (high / low).powf((k + 1) as f32 / bands as f32);
            // Every band at least one bin of its own.
            let to = bin(edge).max(from + 1).min(FFT_SIZE / 2);
            let centre = (from + to) as f32 / 2.0 * RATE as f32 / FFT_SIZE as f32;
            ranges.push((from, to));
            // Music falls about 3 dB an octave; this levels it.
            eq.push((centre / low).sqrt());
            from = to.min(FFT_SIZE / 2 - 1);
        }
        let noise_reduction = noise_reduction.clamp(0.0, 0.99);
        Self {
            ring: vec![0.0; FFT_SIZE],
            head: 0,
            window: (0..FFT_SIZE).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / (FFT_SIZE - 1) as f32).cos()).collect(),
            ranges,
            eq,
            sens: 1.0,
            sens_init: true,
            noise_reduction,
            // CAVA's `gravity_mod`, so the fall looks the same at any rate.
            gravity: (60.0 / fps as f32).powf(2.5) * 1.54 / noise_reduction.max(0.1),
            peak: vec![0.0; bands],
            fall: vec![0.0; bands],
            prev: vec![0.0; bands],
            silent: true,
        }
    }

    fn push(&mut self, samples: &[f32]) {
        for &s in samples {
            self.ring[self.head] = s;
            self.head = (self.head + 1) % FFT_SIZE;
        }
        self.silent = samples.iter().all(|s| s.abs() < SILENCE);
    }

    /// The bars now, 0 to 1.
    fn frame(&mut self) -> Vec<f32> {
        let mut re: Vec<f32> = (0..FFT_SIZE).map(|i| self.ring[(self.head + i) % FFT_SIZE] * self.window[i]).collect();
        let mut im = vec![0.0; FFT_SIZE];
        fft(&mut re, &mut im);
        // A full-scale sine comes out about 1.
        let scale = 4.0 / FFT_SIZE as f32;
        let mut out: Vec<f32> = self
            .ranges
            .iter()
            .zip(&self.eq)
            .map(|(&(from, to), eq)| {
                if self.silent {
                    return 0.0;
                }
                let sum: f32 = (from..to).map(|i| re[i].hypot(im[i])).sum();
                sum / (to - from) as f32 * scale * eq * self.sens
            })
            .collect();

        let mut overshoot = false;
        for (k, v) in out.iter_mut().enumerate() {
            // Up at once; down with gravity, from the peak, faster and
            // faster, but never below the sound itself.
            if *v < self.prev[k] && self.noise_reduction > 0.0 {
                let falling = self.peak[k] * (1.0 - self.fall[k] * self.fall[k] * self.gravity);
                *v = v.max(falling);
                self.fall[k] += 0.028;
            } else {
                self.peak[k] = *v;
                self.fall[k] = 0.0;
            }
            self.prev[k] = *v;
            if *v > 1.0 {
                overshoot = true;
                *v = 1.0;
            }
            if *v < 1e-3 {
                *v = 0.0;
            }
        }
        if overshoot {
            self.sens *= 0.98;
            self.sens_init = false;
        } else if !self.silent {
            self.sens *= if self.sens_init { 1.1 } else { 1.001 };
        }
        self.sens = self.sens.clamp(1e-4, 1e4);
        out
    }
}

/// In-place radix-2 FFT; `re.len()` is a power of two.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * PI / len as f32;
        let (w_im, w_re) = angle.sin_cos();
        for start in (0..n).step_by(len) {
            let (mut c_re, mut c_im) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let t_re = re[b] * c_re - im[b] * c_im;
                let t_im = re[b] * c_im + im[b] * c_re;
                re[b] = re[a] - t_re;
                im[b] = im[a] - t_im;
                re[a] += t_re;
                im[a] += t_im;
                (c_re, c_im) = (c_re * w_re - c_im * w_im, c_re * w_im + c_im * w_re);
            }
        }
        len <<= 1;
    }
}

fn set_colour(cr: &cairo::Context, colour: &gtk::gdk::RGBA, alpha: f64) {
    cr.set_source_rgba(
        f64::from(colour.red()),
        f64::from(colour.green()),
        f64::from(colour.blue()),
        f64::from(colour.alpha()) * alpha,
    );
}

/// Mini: one smooth line through the bands, along the bottom when quiet.
fn draw_line(cr: &cairo::Context, bars: &[f32], w: f64, h: f64, colour: &gtk::gdk::RGBA) -> Result<(), cairo::Error> {
    let (left, width) = (PAD, w - 2.0 * PAD);
    let (bottom, height) = (h - PAD, h - 2.0 * PAD);
    let points: Vec<(f64, f64)> = bars
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = left + width * i as f64 / (bars.len().max(2) - 1) as f64;
            (x, bottom - f64::from(*v) * height)
        })
        .collect();
    let Some(&(x0, y0)) = points.first() else { return Ok(()) };
    cr.move_to(x0, y0);
    // Catmull-Rom through the points, as Bézier curves.
    for i in 0..points.len() - 1 {
        let p0 = points[i.saturating_sub(1)];
        let (p1, p2) = (points[i], points[i + 1]);
        let p3 = points[(i + 2).min(points.len() - 1)];
        cr.curve_to(
            p1.0 + (p2.0 - p0.0) / 6.0,
            p1.1 + (p2.1 - p0.1) / 6.0,
            p2.0 - (p3.0 - p1.0) / 6.0,
            p2.1 - (p3.1 - p1.1) / 6.0,
            p2.0,
            p2.1,
        );
    }
    set_colour(cr, colour, 1.0);
    cr.set_line_width(1.5);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.stroke()
}

/// Where bar `i` of `n` goes across `w`, with `gap` between bars.
fn bar_x(i: usize, n: usize, w: f64, gap: f64) -> (f64, f64) {
    let width = (w - 2.0 * PAD - gap * (n - 1) as f64) / n as f64;
    (PAD + i as f64 * (width + gap), width)
}

/// Normal: 12 bars with rounded tops; a 1 px stub when quiet.
fn draw_bars(cr: &cairo::Context, bars: &[f32], w: f64, h: f64, colour: &gtk::gdk::RGBA) -> Result<(), cairo::Error> {
    let (bottom, height) = (h - PAD, h - 2.0 * PAD);
    set_colour(cr, colour, 1.0);
    for (i, v) in bars.iter().enumerate() {
        let (x, width) = bar_x(i, bars.len(), w, NORMAL_GAP);
        let tall = (f64::from(*v) * height).max(1.0);
        let r = (width / 2.0).min(2.0).min(tall / 2.0);
        let top = bottom - tall;
        cr.new_sub_path();
        cr.arc(x + width - r, top + r, r, -std::f64::consts::PI / 2.0, 0.0);
        cr.line_to(x + width, bottom);
        cr.line_to(x, bottom);
        cr.arc(x + r, top + r, r, std::f64::consts::PI, 3.0 * std::f64::consts::PI / 2.0);
        cr.close_path();
    }
    cr.fill()
}

/// Full: a grid, then thin bars cut from one gradient — yellow at the
/// bottom, orange in the middle, red at the top.
fn draw_full(cr: &cairo::Context, bars: &[f32], w: f64, h: f64, colour: &gtk::gdk::RGBA) -> Result<(), cairo::Error> {
    let (left, right) = (PAD, w - PAD);
    let (top, bottom) = (PAD, h - PAD);
    // The grid: 8 columns, and rows as many as keep its cells square, on
    // whole pixels so its lines are sharp.
    set_colour(cr, colour, 0.12);
    cr.set_line_width(1.0);
    let rows = (8.0 * (bottom - top) / (right - left)).round().max(1.0);
    for k in 0..=8 {
        let x = (left + (right - left) * f64::from(k) / 8.0).round() - 0.5;
        cr.move_to(x, top);
        cr.line_to(x, bottom);
    }
    for k in 0..=rows as i32 {
        let y = (top + (bottom - top) * f64::from(k) / rows).round() - 0.5;
        cr.move_to(left, y);
        cr.line_to(right, y);
    }
    cr.stroke()?;

    let gradient = cairo::LinearGradient::new(0.0, top, 0.0, bottom);
    for (offset, (r, g, b)) in [(0.0, RED), (0.5, ORANGE), (1.0, YELLOW)] {
        gradient.add_color_stop_rgb(offset, r, g, b);
    }
    cr.set_source(&gradient)?;
    let height = bottom - top;
    for (i, v) in bars.iter().enumerate() {
        let (x, width) = bar_x(i, bars.len(), w, FULL_GAP);
        let tall = (f64::from(*v) * height).max(1.0);
        cr.rectangle(x, bottom - tall, width, tall);
    }
    cr.fill()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, amplitude: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| amplitude * (2.0 * PI * freq * i as f32 / RATE as f32).sin()).collect()
    }

    #[test]
    fn fft_finds_a_sine() {
        let bin = 100;
        let freq = bin as f32 * RATE as f32 / FFT_SIZE as f32;
        let mut re = sine(freq, 1.0, FFT_SIZE);
        let mut im = vec![0.0; FFT_SIZE];
        fft(&mut re, &mut im);
        let mags: Vec<f32> = (0..FFT_SIZE / 2).map(|i| re[i].hypot(im[i])).collect();
        let peak = (0..FFT_SIZE / 2).max_by(|&a, &b| mags[a].total_cmp(&mags[b])).unwrap();
        assert_eq!(peak, bin);
        assert!((mags[bin] - FFT_SIZE as f32 / 2.0).abs() < 1.0);
    }

    /// How long after a sound starts its bar is past half way, in ms.
    fn rise_ms(fps: u32) -> f32 {
        let mut a = Analyzer::new(NORMAL_BARS, 50.0, 10_000.0, 0.77, fps);
        let per = RATE as usize / fps as usize;
        // Teach the sensitivity the tone first, then a pause.
        let tone = sine(1000.0, 0.5, per);
        let silence = vec![0.0; per];
        for _ in 0..3 * fps {
            a.push(&tone);
            a.frame();
        }
        for _ in 0..2 * fps {
            a.push(&silence);
            a.frame();
        }
        for n in 1..=fps {
            a.push(&tone);
            if a.frame().iter().any(|v| *v > 0.5) {
                return n as f32 * 1000.0 / fps as f32;
            }
        }
        f32::INFINITY
    }

    #[test]
    fn bars_rise_quickly() {
        for fps in [30, 60] {
            let ms = rise_ms(fps);
            eprintln!("{fps} fps: {ms} ms");
            assert!(ms <= 70.0, "{fps} fps: {ms} ms");
        }
    }

    #[test]
    fn bands_cover_the_range_in_order() {
        let a = Analyzer::new(FULL_BARS, 50.0, 10_000.0, 0.77, 30);
        assert_eq!(a.ranges.len(), FULL_BARS);
        for pair in a.ranges.windows(2) {
            assert!(pair[0].0 < pair[0].1 && pair[0].1 <= pair[1].0);
        }
    }

    #[test]
    fn a_tone_lights_its_band_and_silence_falls() {
        let mut a = Analyzer::new(NORMAL_BARS, 50.0, 10_000.0, 0.77, 30);
        let tone = sine(1000.0, 0.5, RATE as usize / 30);
        let mut frame = Vec::new();
        for _ in 0..60 {
            a.push(&tone);
            frame = a.frame();
        }
        let loudest = (0..NORMAL_BARS).max_by(|&x, &y| frame[x].total_cmp(&frame[y])).unwrap();
        let (from, to) = a.ranges[loudest];
        let bin = 1000.0 * FFT_SIZE as f32 / RATE as f32;
        assert!((from as f32) <= bin.ceil() && bin.floor() < to as f32, "band {loudest} {from}..{to}");
        // The sensitivity brought the loudest bar to the top.
        assert!(frame[loudest] > 0.9);
        let silence = vec![0.0; RATE as usize / 30];
        for _ in 0..90 {
            a.push(&silence);
            frame = a.frame();
        }
        assert!(frame.iter().all(|v| *v == 0.0), "{frame:?}");
    }

    /// What the speakers play for a second, as Normal bars:
    /// `cargo test -p orbit-panel records -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn records_the_monitor() {
        let mut a = Analyzer::new(NORMAL_BARS, 50.0, 10_000.0, 0.77, 30);
        let (mut n, mut peak, mut frames) = (0usize, 0.0f32, 0);
        let start = std::time::Instant::now();
        let stop = pulse::record_monitor(RATE, 1024, |samples| {
            if n < 20 * 1024 {
                eprintln!("{:>6.1} ms: {} samples", start.elapsed().as_secs_f64() * 1000.0, samples.len());
            }
            n += samples.len();
            peak = samples.iter().fold(peak, |p, s| p.max(s.abs()));
            a.push(samples);
            if n / (RATE as usize / 30) > frames {
                frames += 1;
                let bars: String = a.frame().iter().map(|v| [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'][(v * 8.0) as usize]).collect();
                eprintln!("|{bars}|");
            }
            n < RATE as usize
        });
        eprintln!("{n} samples, peak {peak}, stopped: {}", matches!(stop, Ok(pulse::Stop::Done)));
        assert!(n >= RATE as usize);
    }

    /// The three sizes drawn to `$ORBIT_SPECTRUM_PPM`, 4× (a look without
    /// a panel): `ORBIT_SPECTRUM_PPM=x.ppm cargo test -p orbit-panel draws -- --ignored`.
    #[test]
    #[ignore]
    fn draws_the_three_sizes() {
        let Ok(path) = std::env::var("ORBIT_SPECTRUM_PPM") else { return };
        // Something like music: a kick, a bass line, chords, a hi-hat.
        let signal = |t: usize| {
            let x = t as f32 / RATE as f32;
            let tones = [(55.0, 0.5), (110.0, 0.3), (330.0, 0.15), (440.0, 0.12), (660.0, 0.08), (1320.0, 0.04), (2640.0, 0.02), (5280.0, 0.01)];
            let mut v: f32 = tones.iter().map(|(f, a)| a * (2.0 * PI * f * x).sin()).sum();
            // Cheap noise for the cymbals.
            v += 0.01 * (((t as u32).wrapping_mul(2654435761) >> 16) as f32 / 32768.0 - 1.0);
            v * 0.5
        };
        // Or real sound: mono f32 at 44.1 kHz in `$ORBIT_SPECTRUM_RAW`
        // (`ffmpeg -i song.mp3 -f f32le -ac 1 -ar 44100 song.raw`).
        let raw: Option<Vec<f32>> = std::env::var("ORBIT_SPECTRUM_RAW")
            .ok()
            .map(|p| std::fs::read(p).unwrap().chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect());
        let frames = |bands| {
            let mut a = Analyzer::new(bands, 50.0, 10_000.0, 0.77, 30);
            let per = RATE as usize / 30;
            let mut frame = Vec::new();
            for f in 0..90 {
                let block: Vec<f32> = match &raw {
                    Some(raw) => raw[f * per..(f + 1) * per].to_vec(),
                    None => (f * per..(f + 1) * per).map(signal).collect(),
                };
                a.push(&block);
                frame = a.frame();
            }
            frame
        };
        let (scale, gap) = (4.0, 10.0);
        let sizes = [(Form::Mini, 70.0, 30.0, LINE_POINTS), (Form::Normal, 150.0, 70.0, NORMAL_BARS), (Form::Full, 150.0, 70.0, FULL_BARS)];
        let width = sizes.iter().map(|s| s.1).sum::<f64>() + gap * 4.0;
        let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, (width * scale) as i32, ((70.0 + 2.0 * gap) * scale) as i32).unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        cr.scale(scale, scale);
        cr.set_source_rgb(0.08, 0.09, 0.11);
        cr.paint().unwrap();
        let accent = gtk::gdk::RGBA::new(0.27, 0.51, 0.71, 1.0);
        let mut x = gap;
        for (form, w, h, bands) in sizes {
            let bars = frames(bands);
            cr.save().unwrap();
            cr.translate(x, gap);
            // The card.
            cr.set_source_rgb(0.15, 0.17, 0.21);
            cr.rectangle(0.0, 0.0, w, h);
            cr.fill().unwrap();
            match form {
                Form::Mini => draw_line(&cr, &bars, w, h, &accent),
                Form::Normal => draw_bars(&cr, &bars, w, h, &accent),
                _ => draw_full(&cr, &bars, w, h, &accent),
            }
            .unwrap();
            cr.restore().unwrap();
            x += w + gap;
        }
        // A binary PPM (cairo here is built without PNG).
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
