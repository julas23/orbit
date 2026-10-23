//! Orbit-Aigent: talk to a coding agent by voice.
//!
//!   orbit-aigent                 the daemon (started with the session)
//!   orbit-aigent toggle|listen|cancel|yes|no|demo
//!   orbit-aigent agent <name>    use another agent from now on
//!
//! The daemon keeps the microphone open, waits for its wake word, hears
//! the request until a pause, hands it to the agent in use and speaks the
//! answer, telling the panel's aigent module what it is doing (`Frame`).
//! Four threads: the microphone's, the socket's (one more per client
//! reading its commands), Whisper's, and the engine, which owns the state
//! and is the only one that writes to the clients.

mod config;
mod mic;
mod stt;
mod vad;
mod wake;

use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

use orbit_aigent::{BANDS, Command, FRAME_RATE, Frame, State};
use orbit_audio::Analyzer;

use crate::config::Config;

/// Listening gives up after this long without speech.
const NO_SPEECH: Duration = Duration::from_secs(6);
/// And stops after this long of it.
const LONGEST: Duration = Duration::from_secs(30);

enum Event {
    Block(Vec<f32>),
    Command(Command),
    Client(UnixStream),
    /// Whisper's answer to job `id`, with the audio it was given.
    Heard {
        id: u64,
        job: Job,
        heard: Option<stt::Heard>,
        audio: Vec<f32>,
    },
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        std::process::exit(client(&args));
    }
    if let Err(err) = daemon() {
        log::error!("aigent: {err:#}");
        std::process::exit(1);
    }
}

/// `orbit-aigent <command>`.
fn client(args: &[String]) -> i32 {
    let command = match args {
        [one] => match one.as_str() {
            "toggle" => Command::Toggle,
            "listen" => Command::Listen,
            "cancel" => Command::Cancel,
            "yes" => Command::Yes,
            "no" => Command::No,
            "demo" => Command::Demo,
            _ => return usage(),
        },
        [agent, name] if agent == "agent" => Command::Agent { name: name.clone() },
        _ => return usage(),
    };
    match orbit_aigent::send(&command) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("orbit-aigent: {}: {err} (is the daemon running?)", orbit_aigent::socket_path().display());
            1
        }
    }
}

fn usage() -> i32 {
    eprintln!("usage: orbit-aigent [toggle|listen|cancel|yes|no|demo|agent <name>]");
    2
}

fn daemon() -> anyhow::Result<()> {
    let path = orbit_aigent::socket_path();
    // A socket left by a daemon that died; a live one answers.
    if UnixStream::connect(&path).is_ok() {
        anyhow::bail!("another orbit-aigent is running ({})", path.display());
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    log::info!("aigent: listening on {}", path.display());

    let config = Config::load();
    let (events, inbox) = mpsc::channel();
    {
        let events = events.clone();
        let microphone = config.microphone.clone();
        std::thread::Builder::new()
            .name("mic".into())
            .spawn(move || {
                let send = |block: &[f32]| events.send(Event::Block(block.to_vec())).is_ok();
                match std::env::var_os("ORBIT_AIGENT_INPUT") {
                    Some(files) => mic::play_files(&files, send),
                    None => mic::record(microphone.as_deref(), send),
                }
            })?;
    }
    let (jobs, queue) = mpsc::channel();
    {
        let events = events.clone();
        let config = config.clone();
        std::thread::Builder::new()
            .name("whisper".into())
            .spawn(move || whisper(&config, &queue, &events))?;
    }
    std::thread::Builder::new()
        .name("socket".into())
        .spawn(move || serve(&listener, &events))?;
    Engine::new(&config, jobs).run(&inbox);
    Ok(())
}

/// Accepts clients: the engine gets a copy to write to, and a thread reads
/// its commands.
fn serve(listener: &UnixListener, events: &mpsc::Sender<Event>) {
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(err) => {
                log::warn!("aigent: accepting: {err}");
                continue;
            }
        };
        let Ok(writer) = stream.try_clone() else { continue };
        // A client that stops reading must not stall the engine.
        let _ = writer.set_write_timeout(Some(Duration::from_millis(100)));
        if events.send(Event::Client(writer)).is_err() {
            return;
        }
        let events = events.clone();
        let _ = std::thread::Builder::new().name("client".into()).spawn(move || {
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Command>(&line) {
                    Ok(command) => {
                        if events.send(Event::Command(command)).is_err() {
                            break;
                        }
                    }
                    Err(err) => log::warn!("aigent: a client sent {line:?}: {err}"),
                }
            }
        });
    }
}


/// What Whisper is asked to hear.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Job {
    /// Speech while waiting: is it the wake word? With the small model,
    /// but for the name alone, which it hears badly ("Jarvis." comes out
    /// "É viz."): short bits go to the larger one.
    Wake,
    /// The same with the larger model, when the small one heard something
    /// close to the wake word.
    WakeAgain,
    /// The request, with the larger one.
    Request,
}

/// Runs Whisper off the engine's thread: it takes a second or more, and
/// the sphere must keep moving meanwhile. The models load here too, so the
/// daemon answers the panel at once on start.
fn whisper(config: &Config, jobs: &mpsc::Receiver<(u64, Job, Vec<f32>)>, events: &mpsc::Sender<Event>) {
    let load = |name: &str| {
        stt::Stt::load(name)
            .inspect_err(|err| log::error!("aigent: Whisper: {err:#}"))
            .ok()
    };
    let mut wake = load(&config.wake_model);
    let mut request = if config.model == config.wake_model { None } else { load(&config.model) };
    let prompt = stt::prompt(&config.wake);
    // Each bit heard, kept as a WAV file for a look at what the microphone
    // gives: `ORBIT_AIGENT_DUMP=dir`.
    let dump = std::env::var_os("ORBIT_AIGENT_DUMP").map(PathBuf::from);
    for (id, job, audio) in jobs {
        if let Some(dir) = &dump {
            let path = dir.join(format!("{id:04}-{job:?}.wav"));
            if let Err(err) = write_wav(&path, &audio) {
                log::warn!("aigent: {}: {err}", path.display());
            }
        }
        let stt = match job {
            Job::Wake if audio.len() > SHORT_WAKE => wake.as_mut(),
            Job::Wake | Job::WakeAgain | Job::Request => request.as_mut().or(wake.as_mut()),
        };
        let prompt = if job == Job::Request { "" } else { &prompt };
        let heard = stt.and_then(|stt| {
            stt.hear(&audio, prompt, job == Job::Request)
                .inspect_err(|err| log::warn!("aigent: Whisper: {err:#}"))
                .ok()
        });
        if events.send(Event::Heard { id, job, heard, audio }).is_err() {
            return;
        }
    }
}

/// 16 kHz mono, 16-bit.
fn write_wav(path: &std::path::Path, audio: &[f32]) -> std::io::Result<()> {
    let data = (audio.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&mic::RATE.to_le_bytes());
    out.extend_from_slice(&(mic::RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in audio {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, out)
}

/// Speech after the name, while Whisper hears it.
#[derive(Clone, Copy, PartialEq)]
enum Said {
    Nothing,
    Started,
    /// Started and ended: a whole request.
    All,
}

/// The made-up run of `Command::Demo`: it listens for real, then thinks
/// and speaks with sound of its own.
#[derive(Clone, Copy, PartialEq)]
enum Demo {
    Off,
    On,
}

struct Engine {
    agents: Vec<String>,
    agent: String,
    wake_words: Vec<String>,
    state: State,
    /// When the state was entered.
    since: Instant,
    text: String,
    vad: vad::Vad,
    /// Whether speech started since listening began.
    heard: bool,
    /// Whether listening began again after hearing only noise.
    retried: bool,
    /// The last 300 ms, so speech is recorded from just before the voice
    /// detector noticed it.
    before: VecDeque<f32>,
    /// The speech being recorded, while waiting or listening.
    recording: Option<Vec<f32>>,
    /// What came after speech went to Whisper for the wake word, until it
    /// answers: the request said straight after the name, which listening
    /// then starts with.
    after_wake: Option<Vec<f32>>,
    /// How much of something was said in it.
    after_wake_said: Said,
    /// To Whisper, and the job whose answer counts: a newer one, or a
    /// cancel, makes an older answer moot.
    jobs: mpsc::Sender<(u64, Job, Vec<f32>)>,
    job: u64,
    waiting_for: Option<u64>,
    analyzer: Analyzer,
    bands: Vec<f32>,
    demo: Demo,
    clients: Vec<UnixStream>,
    /// Something changed since the last frame went out.
    dirty: bool,
}

/// Speech while waiting this short may be the name alone.
const SHORT_WAKE: usize = 5 * mic::RATE as usize / 2;
/// Speech while waiting longer than this is not someone calling it.
const LONGEST_WAKE: usize = 10 * mic::RATE as usize;
const BEFORE: usize = 3 * mic::RATE as usize / 10;

impl Engine {
    fn new(config: &Config, jobs: mpsc::Sender<(u64, Job, Vec<f32>)>) -> Self {
        let agents = config.usable();
        let agent = if agents.contains(&config.agent) {
            config.agent.clone()
        } else {
            if !config.agent.is_empty() {
                log::warn!("aigent: agent {} is not set up or not installed", config.agent);
            }
            agents.first().cloned().unwrap_or_default()
        };
        log::info!("aigent: agents {agents:?}, using {agent:?}, woken by {:?}", config.wake);
        Self {
            agents,
            agent,
            wake_words: config.wake.clone(),
            state: State::Idle,
            since: Instant::now(),
            text: String::new(),
            vad: vad::Vad::default(),
            heard: false,
            retried: false,
            before: VecDeque::with_capacity(BEFORE + mic::BLOCK),
            recording: None,
            after_wake: None,
            after_wake_said: Said::Nothing,
            jobs,
            job: 0,
            waiting_for: None,
            analyzer: Analyzer::new(mic::RATE, BANDS, 80.0, 6_000.0, 0.77, FRAME_RATE),
            bands: Vec::new(),
            demo: Demo::Off,
            clients: Vec::new(),
            dirty: true,
        }
    }

    fn run(mut self, inbox: &mpsc::Receiver<Event>) {
        let tick = Duration::from_secs(1) / FRAME_RATE;
        let mut next = Instant::now() + tick;
        loop {
            let wait = next.saturating_duration_since(Instant::now());
            match inbox.recv_timeout(wait) {
                Ok(Event::Block(block)) => self.block(&block),
                Ok(Event::Command(command)) => self.command(command),
                Ok(Event::Client(client)) => {
                    self.clients.push(client);
                    self.dirty = true;
                }
                Ok(Event::Heard { id, job, heard, audio }) => {
                    if self.waiting_for == Some(id) {
                        self.waiting_for = None;
                        self.heard_back(job, heard, audio);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
            if Instant::now() >= next {
                next += tick;
                // Behind by more than a frame (a suspend): start afresh.
                if next < Instant::now() {
                    next = Instant::now() + tick;
                }
                self.frame();
            }
        }
    }

    fn enter(&mut self, state: State) {
        if state != self.state {
            log::debug!("aigent: {:?} → {state:?}", self.state);
        }
        self.state = state;
        self.since = Instant::now();
        self.dirty = true;
        match state {
            State::Listening | State::Confirm => {
                // Woken: from just after the name, perhaps already talking.
                if let Some(after) = self.after_wake.take() {
                    self.heard = self.vad.speaking();
                    self.recording = Some(after);
                } else {
                    self.vad.reset();
                    self.heard = false;
                    self.recording = Some(self.before.iter().copied().collect());
                }
            }
            State::Idle => {
                self.demo = Demo::Off;
                self.retried = false;
                self.recording = None;
                // Whatever Whisper is still hearing no longer counts.
                self.waiting_for = None;
            }
            _ => self.recording = None,
        }
        self.after_wake = None;
        if !moves(state) {
            self.bands.clear();
        }
    }

    /// Hands `audio` to Whisper; only the answer to the newest job counts.
    fn ask(&mut self, job: Job, audio: Vec<f32>) {
        self.job += 1;
        self.waiting_for = Some(self.job);
        if self.jobs.send((self.job, job, audio)).is_err() {
            log::error!("aigent: Whisper is gone");
            self.waiting_for = None;
        }
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Toggle if self.state == State::Idle => self.enter(State::Listening),
            Command::Toggle | Command::Cancel => self.enter(State::Idle),
            Command::Listen => self.enter(State::Listening),
            // Nothing asks yet: the agents come next.
            Command::Yes | Command::No => {}
            Command::Agent { name } => {
                if self.agents.contains(&name) {
                    log::info!("aigent: using {name}");
                    self.agent = name;
                    self.dirty = true;
                } else {
                    log::warn!("aigent: no agent {name} (set up: {:?})", self.agents);
                }
            }
            Command::Demo => {
                self.demo = Demo::On;
                self.enter(State::Listening);
            }
        }
    }

    /// A block from the microphone. The voice detector hears them all, to
    /// know the room; while waiting, speech is recorded for the wake word,
    /// and while listening, for the request.
    fn block(&mut self, block: &[f32]) {
        let event = self.vad.push(block);
        if let Some(recording) = &mut self.recording {
            recording.extend_from_slice(block);
        }
        if let Some(after) = &mut self.after_wake {
            after.extend_from_slice(block);
        }
        self.before.extend(block);
        while self.before.len() > BEFORE {
            self.before.pop_front();
        }
        match self.state {
            State::Idle => self.wait(event),
            State::Listening | State::Confirm => {
                self.analyzer.push(block);
                match event {
                    Some(vad::Event::Started) => self.heard = true,
                    Some(vad::Event::Ended) => self.heard_all(),
                    None => {}
                }
            }
            _ => {}
        }
    }

    /// Waiting: each bit of speech goes to Whisper for the wake word, one
    /// at a time (speech while it is busy is not called for it).
    fn wait(&mut self, event: Option<vad::Event>) {
        match event {
            Some(vad::Event::Started) if self.waiting_for.is_none() => {
                self.recording = Some(self.before.iter().copied().collect());
            }
            Some(vad::Event::Started) => {
                if self.after_wake.is_some() {
                    self.after_wake_said = Said::Started;
                }
            }
            Some(vad::Event::Ended) => {
                if let Some(recording) = self.recording.take() {
                    self.ask(Job::Wake, recording);
                    // From 300 ms back: the next words may have begun,
                    // softly, before the pause was long enough to end it.
                    self.after_wake = Some(self.before.iter().copied().collect());
                    self.after_wake_said = Said::Nothing;
                } else if self.after_wake_said == Said::Started {
                    self.after_wake_said = Said::All;
                }
            }
            _ => {
                if self.recording.as_ref().is_some_and(|r| r.len() > LONGEST_WAKE) {
                    self.recording = None;
                }
            }
        }
    }

    /// The request is over: the pause after it, or too long.
    fn heard_all(&mut self) {
        let audio = self.recording.take().unwrap_or_default();
        log::info!("aigent: heard {:.1} s", audio.len() as f32 / mic::RATE as f32);
        self.enter(State::Thinking);
        if self.demo == Demo::Off {
            self.ask(Job::Request, audio);
        }
    }

    /// Whisper's answer to the job still wanted.
    fn heard_back(&mut self, job: Job, heard: Option<stt::Heard>, audio: Vec<f32>) {
        let Some(heard) = heard else {
            self.enter(State::Idle);
            return;
        };
        match job {
            Job::Wake | Job::WakeAgain => {
                if self.state != State::Idle {
                    return;
                }
                let Some((taken, rest)) = wake::split_at(&heard.text, &self.wake_words) else {
                    if job == Job::Wake && wake::near(&heard.text, &self.wake_words) {
                        log::debug!("aigent: {:?} is close to a wake word, hearing it again", heard.text);
                        let after = self.after_wake.take();
                        self.ask(Job::WakeAgain, audio);
                        self.after_wake = after;
                    } else {
                        self.after_wake = None;
                    }
                    return;
                };
                if !rest.is_empty() {
                    // Called and asked in one breath: the request heard
                    // again, from after the name. With the name, an English
                    // word, Whisper takes a short Portuguese request for
                    // English ("Jarvis, que horas são?" as "Jarvis
                    // Kerasom.").
                    log::info!("aigent: woken, with a request");
                    self.enter(State::Thinking);
                    let cut = name_end(&audio, &heard.text, taken);
                    log::debug!(
                        "aigent: the name ends at {:.2} s of {:.2} s",
                        cut as f32 / mic::RATE as f32,
                        audio.len() as f32 / mic::RATE as f32
                    );
                    self.ask(Job::Request, audio[cut..].to_vec());
                } else if self.after_wake_said == Said::All {
                    // The name, a pause, and the request, all said before
                    // Whisper had the name.
                    log::info!("aigent: woken, the request already said");
                    let audio = self.after_wake.take().unwrap_or_default();
                    self.enter(State::Thinking);
                    self.ask(Job::Request, audio);
                } else {
                    log::info!("aigent: woken");
                    self.enter(State::Listening);
                }
            }
            Job::Request => {
                // Nothing said after all (a noise the voice detector took
                // for speech): listen once more.
                if heard.text.is_empty() && !self.retried {
                    log::info!("aigent: nothing said, listening again");
                    self.enter(State::Listening);
                    self.retried = true;
                    return;
                }
                // The wake word, when the request came with it.
                let request = wake::split(&heard.text, &self.wake_words).unwrap_or(heard.text);
                self.request(request, heard.language);
            }
        }
    }

    /// What to do, in `language`.
    fn request(&mut self, request: String, language: &str) {
        if request.trim().is_empty() {
            log::info!("aigent: nothing to do");
            self.enter(State::Idle);
            return;
        }
        log::info!("aigent: request ({language}) for {}: {request:?}", self.agent);
        self.text = request;
        // The agents come next: for now the request is shown.
        self.enter(State::Idle);
    }

    /// Every tick: timeouts, the demo's script, and the frame to the clients.
    fn frame(&mut self) {
        let elapsed = self.since.elapsed();
        match self.state {
            State::Listening | State::Confirm => {
                if (!self.heard && elapsed >= NO_SPEECH) || elapsed >= LONGEST {
                    if self.heard {
                        self.heard_all();
                    } else {
                        log::info!("aigent: nothing heard");
                        if self.demo == Demo::On {
                            self.enter(State::Thinking);
                        } else {
                            self.enter(State::Idle);
                        }
                    }
                } else {
                    self.bands = self.analyzer.frame();
                }
            }
            State::Thinking if self.demo == Demo::On && elapsed >= Duration::from_millis(1500) => {
                self.enter(State::Speaking);
            }
            State::Speaking if self.demo == Demo::On => {
                if elapsed >= Duration::from_secs(5) {
                    self.text = "demo".into();
                    self.enter(State::Idle);
                } else {
                    self.bands = voice(elapsed.as_secs_f32());
                }
            }
            _ => {}
        }
        if self.dirty || moves(self.state) {
            self.dirty = false;
            self.broadcast();
        }
    }

    fn broadcast(&mut self) {
        let frame = Frame {
            state: self.state,
            agent: self.agent.clone(),
            agents: self.agents.clone(),
            bands: self.bands.clone(),
            text: self.text.clone(),
        };
        let mut line = serde_json::to_vec(&frame).expect("a frame serialises");
        line.push(b'\n');
        self.clients.retain_mut(|client| client.write_all(&line).is_ok());
    }
}

/// Where in `audio` the first `taken` words of `text` (the wake word) end,
/// in samples. Whisper's own word times are no help: with a little noise
/// it puts every word at the end. Instead: the speech's span, from the
/// first loud 10 ms to the last; the name's share of it by its share of
/// the letters; and from there, the quietest moment within a quarter of a
/// second, the gap between two words.
fn name_end(audio: &[f32], text: &str, taken: usize) -> usize {
    const FRAME: usize = mic::RATE as usize / 100;
    let levels: Vec<f32> = audio.chunks(FRAME).map(vad::level).collect();
    let loudest = levels.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let loud = |l: &f32| *l > loudest - 30.0;
    let (Some(first), Some(last)) = (levels.iter().position(loud), levels.iter().rposition(loud)) else {
        return 0;
    };
    let letters = |words: &[&str]| words.iter().map(|w| w.chars().filter(|c| c.is_alphanumeric()).count()).sum::<usize>();
    let words: Vec<&str> = text.split_whitespace().collect();
    let share = letters(&words[..taken.min(words.len())]) as f32 / letters(&words).max(1) as f32;
    let guess = first + ((last - first) as f32 * share) as usize;
    let (from, to) = (guess.saturating_sub(25).max(first), (guess + 25).min(last));
    let quietest = (from..=to).min_by(|a, b| levels[*a].total_cmp(&levels[*b])).unwrap_or(guess);
    (quietest * FRAME).min(audio.len())
}

/// Whether the sphere moves in `state`, so frames go out all the time.
fn moves(state: State) -> bool {
    matches!(state, State::Listening | State::Speaking | State::Confirm)
}

/// Something like a voice's bands at `t` s: syllables four or five a
/// second, in phrases, loudest in the formants.
fn voice(t: f32) -> Vec<f32> {
    let syllable = (t * std::f32::consts::TAU * 4.3).sin().max(0.0).sqrt();
    let phrase = if (t % 1.7) < 1.35 { 1.0 } else { 0.1 };
    (0..BANDS)
        .map(|k| {
            let k = k as f32;
            let formants = (-((k - 5.0) / 3.5).powi(2)).exp() + 0.7 * (-((k - 13.0) / 4.0).powi(2)).exp();
            let shimmer = 0.75 + 0.25 * (t * 23.0 + k * 1.3).sin();
            (syllable * phrase * formants * shimmer).clamp(0.0, 1.0)
        })
        .collect()
}
