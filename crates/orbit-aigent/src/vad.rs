//! Voice activity, by energy, as Jarvis's `vad.ts` does: instant, and it
//! cannot quietly fail. Each 30 ms block's level is compared with the
//! room's noise floor, which follows the quietest blocks quickly and the
//! louder ones slowly; speech is a run of blocks well above it, and it ends
//! after a pause long enough not to be a breath between words.

/// A block this far over the floor is voice, in dB.
const OVER_FLOOR: f32 = 12.0;
/// And at least this loud, so a silent room's hiss is never voice.
const MIN_LEVEL: f32 = -50.0;
/// Blocks of voice in a row before speech starts: 90 ms.
const START_BLOCKS: u32 = 3;
/// Blocks of quiet that end it: 810 ms.
const END_BLOCKS: u32 = 27;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Started,
    Ended,
}

#[derive(Default)]
pub struct Vad {
    /// The noise floor, in dB; the first block's level to begin with.
    floor: Option<f32>,
    speaking: bool,
    /// Blocks of voice in a row (before speech) or of quiet (during it).
    run: u32,
}

/// A block's level, in dB from full scale.
pub fn level(block: &[f32]) -> f32 {
    let power = block.iter().map(|s| s * s).sum::<f32>() / block.len().max(1) as f32;
    10.0 * power.max(1e-10).log10()
}

impl Vad {
    /// Takes the next block; says when speech starts or ends.
    pub fn push(&mut self, block: &[f32]) -> Option<Event> {
        let db = level(block);
        let floor = self.floor.get_or_insert(db);
        let voice = db > *floor + OVER_FLOOR && db > MIN_LEVEL;
        // Down fast, up slowly: voice barely moves it, but a room that
        // stays louder is the floor within a few seconds.
        *floor += (db - *floor)
            * if db < *floor {
                0.3
            } else if !voice {
                0.02
            } else {
                0.002
            };
        if self.speaking {
            self.run = if voice { 0 } else { self.run + 1 };
            if self.run >= END_BLOCKS {
                self.speaking = false;
                self.run = 0;
                return Some(Event::Ended);
            }
        } else {
            self.run = if voice { self.run + 1 } else { 0 };
            if self.run >= START_BLOCKS {
                self.speaking = true;
                self.run = 0;
                return Some(Event::Started);
            }
        }
        None
    }

    /// Forgets a speech under way (the floor is kept).
    pub fn reset(&mut self) {
        self.speaking = false;
        self.run = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mic::{BLOCK, RATE};

    fn noise(amplitude: f32, seed: &mut u32) -> Vec<f32> {
        (0..BLOCK)
            .map(|_| {
                *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                amplitude * ((*seed >> 8) as f32 / (1 << 23) as f32 - 1.0)
            })
            .collect()
    }

    fn voice(amplitude: f32, n: usize) -> Vec<f32> {
        (0..BLOCK)
            .map(|i| amplitude * (2.0 * std::f32::consts::PI * 220.0 * (n * BLOCK + i) as f32 / RATE as f32).sin())
            .collect()
    }

    #[test]
    fn speech_over_a_noisy_room_starts_and_ends() {
        let mut vad = Vad::default();
        let mut seed = 1;
        let mut events = Vec::new();
        // Two seconds of room, one of voice, one of room again.
        for n in 0..133 {
            let block = match n {
                0..66 => noise(0.003, &mut seed),
                66..99 => voice(0.2, n),
                _ => noise(0.003, &mut seed),
            };
            if let Some(event) = vad.push(&block) {
                events.push((n, event));
            }
        }
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(events[0], (66 + START_BLOCKS as usize - 1, Event::Started));
        assert_eq!(events[1].1, Event::Ended);
        assert!((99..99 + END_BLOCKS as usize + 1).contains(&events[1].0), "{events:?}");
    }

    #[test]
    fn the_room_alone_is_not_speech() {
        let mut vad = Vad::default();
        let mut seed = 7;
        for _ in 0..300 {
            assert_eq!(vad.push(&noise(0.01, &mut seed)), None);
        }
    }
}
