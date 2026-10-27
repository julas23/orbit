//! Text to speech: Piper, one process per voice kept running (`--output_dir`:
//! a line of text in, the path of its WAV file out, in a fifth of a second),
//! and `pw-play` to play each sentence. Piper and its voices live in
//! `$XDG_DATA_HOME/orbit/aigent/{piper,voices}`.
//!
//! The speaker runs on a thread of its own, sentence by sentence, and
//! shows the engine what is playing (`Playing`), so the sphere can move
//! with the voice.

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    time::Instant,
};

use crate::stt::data_dir;

/// What is playing: the sentence's samples, their rate, and when it began.
#[derive(Clone)]
pub struct Playing {
    pub samples: Arc<Vec<f32>>,
    pub rate: u32,
    pub since: Instant,
}

pub type Now = Arc<Mutex<Option<Playing>>>;

pub enum Order {
    /// Say `text` in `language` (`pt`, `en`), then tell the engine with
    /// `id`; unless `more` of the same comes (an answer said as it is
    /// written), told after the last.
    Say { id: u64, text: String, language: String, more: bool },
}

/// One Piper, one voice.
struct Voice {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Voice {
    fn start(model: &Path, out: &Path) -> anyhow::Result<Self> {
        let dir = data_dir().join("piper");
        let mut child = Command::new(dir.join("piper"))
            .arg("--model")
            .arg(model)
            .arg("--output_dir")
            .arg(out)
            .args(["--sentence_silence", "0.1"])
            // Its own libraries (onnxruntime, espeak-ng) sit beside it.
            .env("LD_LIBRARY_PATH", &dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = BufReader::new(child.stdout.take().expect("piped"));
        Ok(Self { child, stdin, stdout })
    }

    /// The WAV file of `line`.
    fn say(&mut self, line: &str) -> anyhow::Result<PathBuf> {
        writeln!(self.stdin, "{line}")?;
        self.stdin.flush()?;
        let mut path = String::new();
        if self.stdout.read_line(&mut path)? == 0 {
            anyhow::bail!("piper ended");
        }
        Ok(PathBuf::from(path.trim()))
    }
}

impl Drop for Voice {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The voice of each language, `voices/<name>.onnx`.
pub fn voices(configured: &HashMap<String, String>) -> HashMap<String, PathBuf> {
    let dir = data_dir().join("voices");
    let mut names: HashMap<String, String> = [("pt", "pt_BR-faber-medium"), ("en", "en_US-ryan-medium")]
        .into_iter()
        .map(|(l, v)| (l.to_owned(), v.to_owned()))
        .collect();
    names.extend(configured.clone());
    names
        .into_iter()
        .map(|(language, name)| (language, dir.join(format!("{name}.onnx"))))
        .collect()
}

/// The speaker's thread: says what it is told, one sentence at a time,
/// until `stop` is raised (then drops the rest) or the orders end.
pub fn speak(
    voices: &HashMap<String, PathBuf>,
    orders: &mpsc::Receiver<Order>,
    now: &Now,
    stop: &Arc<std::sync::atomic::AtomicBool>,
    done: impl Fn(u64),
) {
    use std::sync::atomic::Ordering;
    let out = std::env::temp_dir().join(format!("orbit-aigent-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&out);
    let mut running: HashMap<String, Voice> = HashMap::new();
    for Order::Say { id, text, language, more } in orders {
        stop.store(false, Ordering::SeqCst);
        let Some(model) = voices.get(&language).or_else(|| voices.get("pt")) else {
            log::error!("aigent: no voice for {language}");
            if !more {
                done(id);
            }
            continue;
        };
        for sentence in sentences(&spoken(&text)) {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let voice = match running.entry(language.clone()) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => match Voice::start(model, &out) {
                    Ok(voice) => entry.insert(voice),
                    Err(err) => {
                        log::error!("aigent: piper {}: {err}", model.display());
                        break;
                    }
                },
            };
            let wav = match voice.say(&sentence) {
                Ok(wav) => wav,
                Err(err) => {
                    log::error!("aigent: piper: {err}");
                    running.remove(&language);
                    break;
                }
            };
            play(&wav, now, stop);
            let _ = std::fs::remove_file(&wav);
        }
        if !more {
            done(id);
        }
    }
}

/// Plays `wav` with `pw-play`, showing it in `now`, until it ends or
/// `stop` is raised.
fn play(wav: &Path, now: &Now, stop: &std::sync::atomic::AtomicBool) {
    use std::sync::atomic::Ordering;
    let (samples, rate) = match read_wav(wav) {
        Ok(read) => read,
        Err(err) => {
            log::warn!("aigent: {}: {err}", wav.display());
            return;
        }
    };
    let mut child = match Command::new("pw-play").arg("--media-role=Assistant").arg(wav).spawn() {
        Ok(child) => child,
        Err(err) => {
            log::error!("aigent: pw-play: {err}");
            return;
        }
    };
    now.lock().unwrap().replace(Playing {
        samples: Arc::new(samples),
        rate,
        since: Instant::now(),
    });
    loop {
        if stop.load(Ordering::SeqCst) {
            let _ = child.kill();
        }
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => break,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
    now.lock().unwrap().take();
}

/// A 16-bit PCM WAV file's samples (the first channel) and rate.
fn read_wav(path: &Path) -> anyhow::Result<(Vec<f32>, u32)> {
    let bytes = std::fs::read(path)?;
    anyhow::ensure!(bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE", "not a WAV file");
    let (mut rate, mut channels, mut at) = (0u32, 1usize, 12);
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into()?) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        match id {
            b"fmt " if body.len() >= 16 => {
                channels = usize::from(u16::from_le_bytes([body[2], body[3]])).max(1);
                rate = u32::from_le_bytes(body[4..8].try_into()?);
            }
            b"data" => {
                let samples = body
                    .chunks_exact(2 * channels)
                    .map(|frame| f32::from(i16::from_le_bytes([frame[0], frame[1]])) / 32768.0)
                    .collect();
                return Ok((samples, rate));
            }
            _ => {}
        }
        at += 8 + size + size % 2;
    }
    anyhow::bail!("no audio in it")
}

/// `text` as it should be said: Markdown marks and links out.
fn spoken(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_inclusive(char::is_whitespace) {
        if word.trim_start_matches(['(', '<']).starts_with("http") {
            out.push_str("link ");
        } else {
            out.push_str(word);
        }
    }
    out.chars().filter(|c| !matches!(c, '*' | '#' | '`' | '_' | '|' | '>')).collect()
}

/// `text` cut into sentences, so the first is heard while the next is
/// made.
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '\n' {
            push(&mut out, &mut current);
            continue;
        }
        current.push(c);
        let next = chars.get(i + 1).copied();
        if matches!(c, '.' | '!' | '?' | ';' | ':') && next.is_none_or(char::is_whitespace) {
            push(&mut out, &mut current);
        }
    }
    push(&mut out, &mut current);
    out
}

fn push(out: &mut Vec<String>, current: &mut String) {
    let sentence = current.trim();
    if sentence.chars().any(char::is_alphanumeric) {
        out.push(sentence.to_owned());
    }
    current.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentences_are_cut_at_their_ends() {
        assert_eq!(
            sentences("São 22h15. Tem 120 GB livres!\nMais algo? Versão 1.2 ok"),
            ["São 22h15.", "Tem 120 GB livres!", "Mais algo?", "Versão 1.2 ok"]
        );
    }

    #[test]
    fn marks_and_links_are_not_said() {
        assert_eq!(spoken("**Pronto**: veja `ls` em https://x.y/z."), "Pronto: veja ls em link ");
    }
}
