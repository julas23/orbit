//! Speech to text: whisper.cpp (`whisper-rs`) on the CPU, the models in
//! `$XDG_DATA_HOME/orbit/aigent/models` as `ggml-<name>.bin`.
//!
//! Two models: a small, fast one hears every bit of speech while the
//! daemon waits, only to find the wake word; a larger one hears the
//! request. Both take the language from the speech, Portuguese or
//! English: Whisper picks one on its own, and when it picks neither the
//! more likely of the two is used.

use std::{path::PathBuf, time::Instant};

use anyhow::Context;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};

/// Above this, Whisper's own guess that nothing was said wins.
const NO_SPEECH: f32 = 0.6;

/// What the first language wins a near tie by, in mean log probability
/// of the tokens heard.
const FIRST_BONUS: f32 = 0.1;

/// The languages heard; the first when it is unclear.
pub const LANGUAGES: [&str; 2] = ["pt", "en"];

pub struct Stt {
    /// Made once: a state holds a few hundred MB of buffers.
    state: WhisperState,
    name: String,
    threads: i32,
}

/// What was said, and in which language.
#[derive(Debug, Clone, PartialEq)]
pub struct Heard {
    pub text: String,
    pub language: &'static str,
}

/// `$XDG_DATA_HOME/orbit/aigent` (`~/.local/share/orbit/aigent`).
pub fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"))
        .join("orbit/aigent")
}

impl Stt {
    /// Loads model `name` (`small-q5_1` is `models/ggml-small-q5_1.bin`).
    pub fn load(name: &str) -> anyhow::Result<Self> {
        whisper_rs::install_logging_hooks();
        let path = data_dir().join("models").join(format!("ggml-{name}.bin"));
        let mut params = WhisperContextParameters::default();
        // whisper-rs turns the GPU on only with its own GPU features.
        params.use_gpu(cfg!(feature = "vulkan"));
        let context = WhisperContext::new_with_params(&path, params)
            .with_context(|| format!("loading {}", path.display()))?;
        let state = context.create_state()?;
        // Half the threads: the desktop stays responsive while it works.
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get() / 2).clamp(1, 8) as i32;
        let mut stt = Self {
            state,
            name: name.to_owned(),
            threads,
        };
        // The first run is twice as slow as the rest: done now, on a second
        // of silence, not on the first thing said.
        stt.state.full(full_params(threads, Some(LANGUAGES[0]), "", audio_ctx(0)), &[0.0; 16_000])?;
        log::info!("aigent: Whisper {name} loaded, {threads} threads");
        Ok(stt)
    }

    /// Transcribes 16 kHz mono `audio`. `prompt` steers it towards the wake
    /// words (`prompt`), but towards English too, which it is written in:
    /// only for finding the wake word, not for a request. With `languages`,
    /// the language is made sure of (see below), which a request needs and
    /// finding the wake word does not.
    pub fn hear(&mut self, audio: &[f32], prompt: &str, languages: bool) -> anyhow::Result<Heard> {
        let start = Instant::now();
        let mut audio = audio.to_vec();
        // A quiet microphone, up to a peak of −3 dB (by 30 dB at most).
        let peak = audio.iter().fold(0.0f32, |p, s| p.max(s.abs()));
        let gain = (0.7 / peak.max(1e-6)).clamp(1.0, 31.6);
        audio.iter_mut().for_each(|s| *s *= gain);
        // Whisper wants at least a second; pad short bits with silence.
        if audio.len() < 16_000 + 1_600 {
            audio.resize(16_000 + 1_600, 0.0);
        }
        let audio = &audio[..];
        let ctx = audio_ctx(audio.len());
        let params = full_params(self.threads, None, prompt, ctx);
        self.state.full(params, audio)?;
        let detected = whisper_rs::get_lang_str(self.state.full_lang_id_from_state()).unwrap_or("");
        let (mut text, score) = self.read();
        let mut language = LANGUAGES.into_iter().find(|l| *l == detected);
        if languages && language != Some(LANGUAGES[0]) {
            // Whisper's guess on a few words is shaky, and the wake word is
            // an English one: heard again as the first language, the run
            // Whisper is surer of wins, the first language on a near tie.
            let mut best = (language, text, score);
            for other in LANGUAGES {
                if Some(other) == language {
                    continue;
                }
                let params = full_params(self.threads, Some(other), prompt, ctx);
                self.state.full(params, audio)?;
                let (heard, sure) = self.read();
                log::debug!("aigent: as {other}: {heard:?} ({sure:.2}), against {:?} ({:.2})", best.1, best.2);
                let bonus = if other == LANGUAGES[0] { FIRST_BONUS } else { 0.0 };
                // The same words as another language ("List the files…"
                // heard as Portuguese): it was that language after all.
                let same = best.0.is_some() && letters(&heard) == letters(&best.1);
                if !same && (best.0.is_none() || sure + bonus > best.2) {
                    best = (Some(other), heard, sure);
                }
            }
            (language, text) = (best.0, best.1);
        }
        let language = language.unwrap_or(LANGUAGES[0]);
        let no_speech = self.state.get_segment(0).map_or(1.0, |s| s.no_speech_probability());
        // Noise, or a breath: Whisper writes something anyway, often the
        // prompt's "Ok.".
        if no_speech > NO_SPEECH {
            text.clear();
        }
        log::info!(
            "aigent: {} heard {:.1} s (peak {:.0} dB, raised {:.0} dB, no speech {no_speech:.2}) in {:.2} s ({language}): {text:?}",
            self.name,
            audio.len() as f32 / 16_000.0,
            20.0 * peak.max(1e-6).log10(),
            20.0 * gain.log10(),
            start.elapsed().as_secs_f32()
        );
        Ok(Heard { text, language })
    }

    /// The text of the last run, and how sure Whisper was of it: the mean
    /// log probability of its tokens (special ones left out).
    fn read(&self) -> (String, f32) {
        let mut text = String::new();
        let (mut sum, mut n) = (0.0f32, 0);
        for segment in self.state.as_iter() {
            if let Ok(s) = segment.to_str_lossy() {
                text.push_str(&s);
            }
            for i in 0..segment.n_tokens() {
                let Some(token) = segment.get_token(i) else { continue };
                let Ok(piece) = token.to_str_lossy() else { continue };
                if piece.starts_with("[_") || piece.starts_with("<|") {
                    continue;
                }
                sum += token.token_probability().max(1e-6).ln();
                n += 1;
            }
        }
        (unrepeated(text.trim()), if n == 0 { f32::NEG_INFINITY } else { sum / n as f32 })
    }
}

/// The prompt that gets the wake words heard: "Ok Jarvis. Ok Orbit." Said
/// alone, a name Whisper does not know comes out as anything ("Jarvis." as
/// "Servis!" or "J.J."); with it written as one says it before, it is
/// spelt right, alone or before a request. Written bare ("Orbit, Jarvis.")
/// it is worse: Whisper takes the name as already said and leaves it out.
pub fn prompt(wake: &[String]) -> String {
    wake.iter()
        .map(|word| {
            let mut chars = word.chars();
            let word: String = chars.next().into_iter().flat_map(char::to_uppercase).chain(chars).collect();
            format!("Ok {word}.")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `text` with a sentence said again straight after itself said once:
/// Whisper, on a short bit, may repeat it ("Para. Para. Para.").
fn unrepeated(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for sentence in text.split_inclusive(['.', '!', '?']) {
        if out.last().is_some_and(|last| letters(last) == letters(sentence)) {
            continue;
        }
        out.push(sentence);
    }
    out.concat().trim().to_owned()
}

/// `text` in lower case, letters only, to compare two hearings.
fn letters(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Greedy, one segment, nothing printed, no context from the last run.
fn full_params<'a>(threads: i32, language: Option<&'a str>, prompt: &str, audio_ctx: i32) -> FullParams<'a, 'a> {
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_audio_ctx(audio_ctx);
    params.set_n_threads(threads);
    params.set_language(Some(language.unwrap_or("auto")));
    params.set_no_context(true);
    params.set_single_segment(true);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_suppress_blank(true);
    params.set_suppress_nst(true);
    params.set_initial_prompt(prompt);
    params
}

/// The encoder's context for `samples`: Whisper always encodes 30 s (1500
/// frames of 20 ms), which for a few seconds of speech is mostly silence
/// and most of the time spent. A context cut to the speech itself makes it
/// drop words and repeat itself; about 10 s (512) at least keeps what it
/// hears as good as the whole 30 s, in a third of the time (small, 4 s of
/// Portuguese: 1.4 s instead of 5 to 7 s on the hp's Ryzen).
fn audio_ctx(samples: usize) -> i32 {
    let frames = samples / 320 + 128;
    frames.clamp(512, 1500) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Transcribes `$ORBIT_AIGENT_RAW` (16 kHz mono f32, from `ffmpeg -i x
    /// -ar 16000 -ac 1 -f f32le x.raw`; several separated by `:`) with
    /// each model (or those in `$ORBIT_AIGENT_MODELS`, separated by `,`) as
    /// a request: `ORBIT_AIGENT_RAW=a.raw
    /// cargo test --release -p orbit-aigent hears -- --ignored --nocapture`.
    #[test]
    fn repeated_sentences_are_said_once() {
        assert_eq!(unrepeated("Para. Para. Para."), "Para.");
        assert_eq!(unrepeated("Sim. Não. Sim."), "Sim. Não. Sim.");
        assert_eq!(unrepeated("Abra o terminal"), "Abra o terminal");
    }

    #[test]
    fn the_prompt_names_the_words() {
        assert_eq!(prompt(&["jarvis".into(), "orbit".into()]), "Ok Jarvis. Ok Orbit.");
    }

    #[test]
    #[ignore]
    fn hears_the_files() {
        let Ok(files) = std::env::var("ORBIT_AIGENT_RAW") else { return };
        let _ = env_logger::builder().is_test(true).parse_default_env().try_init();
        for model in std::env::var("ORBIT_AIGENT_MODELS").unwrap_or("base-q5_1,small-q5_1".into()).split(',') {
            let mut stt = Stt::load(model).unwrap();
            for file in files.split(':') {
                let audio: Vec<f32> = std::fs::read(file)
                    .unwrap()
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                let heard = stt.hear(&audio, &std::env::var("ORBIT_AIGENT_PROMPT").unwrap_or_default(), true).unwrap();
                eprintln!("{model} {}: {heard:?}", file.rsplit('/').next().unwrap_or(file));
            }
        }
    }
}
