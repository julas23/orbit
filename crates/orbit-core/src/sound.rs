//! What the speakers play, for the wallpaper's effects (`wallpaper.sound`):
//! the default sink's monitor, recorded by `pw-record` (PipeWire's own,
//! no library in the core) on a thread of its own, mono at 44.1 kHz, and
//! analysed as the panel's spectrum is (`orbit_audio::Analyzer`) into two
//! numbers, 0 to 1: the bass and the treble.
//!
//! Recording runs only while an effect is seen and wants the sound: the
//! wallpaper's timer starts and stops it (`Sound::run`). A recorder that
//! ends by itself (no PipeWire, no `pw-record`) is not started again
//! until the config is read again; the effects then move as without it.

use std::{
    io::Read,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
};

use orbit_audio::Analyzer;

const RATE: u32 = 44_100;
/// Blocks a second: one analysis each.
const BLOCKS: u32 = 60;
const BANDS: usize = 16;
/// The past spectra kept for the echoes, and their bands.
pub const ECHOES: usize = 6;
pub const ECHO_BANDS: usize = 8;
/// Blocks between two echoes (a tenth of a second).
const ECHO_EVERY: u32 = 6;

/// What the effects get of the sound, 0..1 each.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Levels {
    pub bass: f32,
    pub treble: f32,
    /// The spectrum now, low to high.
    pub bands: [f32; BANDS],
    /// The spectrum as it was, newest first, every tenth of a second, each
    /// in fewer bands.
    pub echoes: [[f32; ECHO_BANDS]; ECHOES],
}

impl Levels {
    fn from_bands(frame: &[f32], before: &Levels, shift: bool) -> Self {
        let [bass, treble] = split(frame);
        let mut bands = [0.0; BANDS];
        for (b, f) in bands.iter_mut().zip(frame) {
            *b = *f;
        }
        let mut echoes = before.echoes;
        if shift {
            echoes.rotate_right(1);
            for (i, e) in echoes[0].iter_mut().enumerate() {
                let per = BANDS / ECHO_BANDS;
                *e = bands[i * per..(i + 1) * per].iter().sum::<f32>() / per as f32;
            }
        }
        Self { bass, treble, bands, echoes }
    }
}

#[derive(Default)]
pub struct Sound {
    recorder: Option<Child>,
    /// Bass and treble, the newest; zero when silent or not recording.
    levels: Arc<Mutex<Levels>>,
    /// The recorder ended by itself: not tried again until `retry`.
    failed: bool,
}

impl Sound {
    /// Records while `wanted`, stops when not.
    pub fn run(&mut self, wanted: bool) {
        if let Some(child) = self.recorder.as_mut()
            && let Ok(Some(status)) = child.try_wait()
        {
            tracing::warn!("wallpaper sound: pw-record ended ({status})");
            self.recorder = None;
            self.failed = true;
        }
        match (wanted && !self.failed, self.recorder.is_some()) {
            (true, false) => self.start(),
            (false, true) => self.stop(),
            _ => {}
        }
    }

    /// After the config is read: a recorder that failed may be tried again.
    pub fn retry(&mut self) {
        self.failed = false;
    }

    /// The sound now.
    pub fn levels(&self) -> Levels {
        if self.recorder.is_none() {
            return Levels::default();
        }
        *self.levels.lock().unwrap()
    }

    fn start(&mut self) {
        let spawned = Command::new("pw-record")
            .args(["-P", "{ stream.capture.sink = true }"])
            .args(["--format", "s16", "--rate", &RATE.to_string(), "--channels", "1"])
            .arg("-")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(err) => {
                tracing::warn!("wallpaper sound: pw-record: {err}");
                self.failed = true;
                return;
            }
        };
        let Some(out) = child.stdout.take() else { return };
        let levels = self.levels.clone();
        let analysed = std::thread::Builder::new()
            .name("orbit-sound".into())
            .spawn(move || analyse(out, &levels));
        if let Err(err) = analysed {
            tracing::error!("wallpaper sound: {err}");
        }
        tracing::info!("wallpaper sound: recording");
        self.recorder = Some(child);
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.recorder.take() {
            let _ = child.kill();
            std::thread::spawn(move || child.wait());
            tracing::info!("wallpaper sound: stopped");
        }
        *self.levels.lock().unwrap() = Levels::default();
    }
}

impl Drop for Sound {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Reads the recorder until it ends, one block at a time.
fn analyse(mut out: impl Read, levels: &Mutex<Levels>) {
    let per_block = (RATE / BLOCKS) as usize;
    let mut analyzer = Analyzer::new(RATE, BANDS, 40.0, 12_000.0, 0.7, BLOCKS);
    let mut bytes = vec![0u8; per_block * 2];
    let mut samples = vec![0f32; per_block];
    let mut block = 0u32;
    while out.read_exact(&mut bytes).is_ok() {
        for (s, b) in samples.iter_mut().zip(bytes.chunks_exact(2)) {
            *s = f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0;
        }
        analyzer.push(&samples);
        block = block.wrapping_add(1);
        let mut now = levels.lock().unwrap();
        *now = Levels::from_bands(&analyzer.frame(), &now, block.is_multiple_of(ECHO_EVERY));
    }
    *levels.lock().unwrap() = Levels::default();
}

/// The bands as two: the lowest quarter, and the highest third.
fn split(bands: &[f32]) -> [f32; 2] {
    let mean = |b: &[f32]| if b.is_empty() { 0.0 } else { b.iter().sum::<f32>() / b.len() as f32 };
    let low = bands.len().div_ceil(4);
    let high = bands.len() - bands.len() / 3;
    [mean(&bands[..low]), mean(&bands[high..])]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_low_and_high() {
        let bands = [1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 0.5];
        assert_eq!(split(&bands), [1.0, 0.5]);
        assert_eq!(split(&[]), [0.0, 0.0]);
    }

    #[test]
    fn echoes_shift_newest_first() {
        let loud = [1.0; BANDS];
        let a = Levels::from_bands(&loud, &Levels::default(), true);
        assert_eq!(a.echoes[0], [1.0; ECHO_BANDS]);
        let b = Levels::from_bands(&[0.0; BANDS], &a, true);
        assert_eq!(b.echoes[0], [0.0; ECHO_BANDS]);
        assert_eq!(b.echoes[1], [1.0; ECHO_BANDS]);
        let c = Levels::from_bands(&loud, &b, false);
        assert_eq!(c.echoes, b.echoes);
    }
}
