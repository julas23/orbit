//! Sound as bars, the way CAVA draws it: shared by the panel's spectrum
//! module (what the speakers play) and Orbit-Aigent (the microphone while
//! it listens, its own voice while it speaks).
//!
//! An `Analyzer` keeps the last ~46 ms of mono samples, and each `frame`
//! runs a Hann window and an FFT over them, sums the bins into bands spaced
//! evenly on a log scale between `low` and `high`, weights them up with
//! frequency (music is louder low down), applies a sensitivity that adjusts
//! itself so the loudest bar just reaches the top, and lets bars fall with
//! gravity (`noise_reduction`). Unlike CAVA's integral filter, nothing
//! slows a bar on its way up: it follows the sound at once.

use std::f32::consts::PI;

/// Below this peak (−60 dB) a block is silence: a monitor or a microphone
/// carries a faint hiss even when nothing plays, which the sensitivity
/// would raise into dancing bars.
pub const SILENCE: f32 = 1e-3;

/// The window's length: about 46 ms at any rate, a power of two (2048 at
/// 44.1 kHz, 1024 at 16 and 22.05 kHz).
pub fn fft_size(rate: u32) -> usize {
    ((rate as f32 * 0.046) as usize).next_power_of_two()
}

/// CAVA's analysis, for a fixed number of bands.
pub struct Analyzer {
    /// The last `fft_size` samples, oldest first from `head`.
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
    /// Bands for sound at `rate` Hz, handed out `fps` times a second.
    pub fn new(rate: u32, bands: usize, low: f32, high: f32, noise_reduction: f32, fps: u32) -> Self {
        let size = fft_size(rate);
        let high = high.min(rate as f32 / 2.0).max(low * 2.0);
        let bin = |f: f32| ((f * size as f32 / rate as f32).round() as usize).clamp(1, size / 2 - 1);
        let mut ranges = Vec::with_capacity(bands);
        let mut eq = Vec::with_capacity(bands);
        let mut from = bin(low);
        for k in 0..bands {
            let edge = low * (high / low).powf((k + 1) as f32 / bands as f32);
            // Every band at least one bin of its own.
            let to = bin(edge).max(from + 1).min(size / 2);
            let centre = (from + to) as f32 / 2.0 * rate as f32 / size as f32;
            ranges.push((from, to));
            // Music falls about 3 dB an octave; this levels it.
            eq.push((centre / low).sqrt());
            from = to.min(size / 2 - 1);
        }
        let noise_reduction = noise_reduction.clamp(0.0, 0.99);
        Self {
            ring: vec![0.0; size],
            head: 0,
            window: (0..size).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / (size - 1) as f32).cos()).collect(),
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

    pub fn push(&mut self, samples: &[f32]) {
        let size = self.ring.len();
        for &s in samples {
            self.ring[self.head] = s;
            self.head = (self.head + 1) % size;
        }
        self.silent = samples.iter().all(|s| s.abs() < SILENCE);
    }

    /// The bars now, 0 to 1.
    pub fn frame(&mut self) -> Vec<f32> {
        let size = self.ring.len();
        let mut re: Vec<f32> = (0..size).map(|i| self.ring[(self.head + i) % size] * self.window[i]).collect();
        let mut im = vec![0.0; size];
        fft(&mut re, &mut im);
        // A full-scale sine comes out about 1.
        let scale = 4.0 / size as f32;
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
pub fn fft(re: &mut [f32], im: &mut [f32]) {
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

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;
    const BANDS: usize = 12;

    fn sine(rate: u32, freq: f32, amplitude: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| amplitude * (2.0 * PI * freq * i as f32 / rate as f32).sin()).collect()
    }

    #[test]
    fn sizes_are_about_46_ms() {
        assert_eq!(fft_size(44_100), 2048);
        assert_eq!(fft_size(22_050), 1024);
        assert_eq!(fft_size(16_000), 1024);
    }

    #[test]
    fn fft_finds_a_sine() {
        let size = fft_size(RATE);
        let bin = 100;
        let freq = bin as f32 * RATE as f32 / size as f32;
        let mut re = sine(RATE, freq, 1.0, size);
        let mut im = vec![0.0; size];
        fft(&mut re, &mut im);
        let mags: Vec<f32> = (0..size / 2).map(|i| re[i].hypot(im[i])).collect();
        let peak = (0..size / 2).max_by(|&a, &b| mags[a].total_cmp(&mags[b])).unwrap();
        assert_eq!(peak, bin);
        assert!((mags[bin] - size as f32 / 2.0).abs() < 1.0);
    }

    /// How long after a sound starts its bar is past half way, in ms.
    fn rise_ms(fps: u32) -> f32 {
        let mut a = Analyzer::new(RATE, BANDS, 50.0, 10_000.0, 0.77, fps);
        let per = RATE as usize / fps as usize;
        // Teach the sensitivity the tone first, then a pause.
        let tone = sine(RATE, 1000.0, 0.5, per);
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
        for rate in [44_100, 16_000] {
            let a = Analyzer::new(rate, 47, 50.0, 10_000.0, 0.77, 30);
            assert_eq!(a.ranges.len(), 47);
            for pair in a.ranges.windows(2) {
                assert!(pair[0].0 < pair[0].1 && pair[0].1 <= pair[1].0);
            }
        }
    }

    #[test]
    fn a_tone_lights_its_band_and_silence_falls() {
        for rate in [44_100, 16_000] {
            let size = fft_size(rate);
            let mut a = Analyzer::new(rate, BANDS, 50.0, 10_000.0, 0.77, 30);
            let tone = sine(rate, 1000.0, 0.5, rate as usize / 30);
            let mut frame = Vec::new();
            for _ in 0..60 {
                a.push(&tone);
                frame = a.frame();
            }
            let loudest = (0..BANDS).max_by(|&x, &y| frame[x].total_cmp(&frame[y])).unwrap();
            let (from, to) = a.ranges[loudest];
            let bin = 1000.0 * size as f32 / rate as f32;
            assert!((from as f32) <= bin.ceil() && bin.floor() < to as f32, "band {loudest} {from}..{to}");
            // The sensitivity brought the loudest bar to the top.
            assert!(frame[loudest] > 0.9);
            let silence = vec![0.0; rate as usize / 30];
            for _ in 0..90 {
                a.push(&silence);
                frame = a.frame();
            }
            assert!(frame.iter().all(|v| *v == 0.0), "{frame:?}");
        }
    }
}
