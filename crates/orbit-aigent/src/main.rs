//! Orbit-Aigent: talk to a coding agent by voice.
//!
//!   orbit-aigent                 the daemon (started with the session)
//!   orbit-aigent toggle|listen|cancel|yes|no|demo
//!   orbit-aigent agent <name>    use another agent from now on
//!   orbit-aigent approve         the agent's permission prompt (MCP, stdio)
//!
//! The daemon keeps the microphone open, waits for its wake word, hears
//! the request until a pause, hands it to the agent in use and speaks the
//! answer, telling the panel's aigent module what it is doing (`Frame`).
//! Threads: the microphone's, the socket's (one more per client reading
//! its commands), Whisper's, the speaker's, one per request to the agent,
//! and the engine, which owns the state and is the only one that writes to
//! the clients.

mod agent;
mod approve;
mod config;
mod mic;
mod stt;
mod tts;
mod vad;
mod wake;

use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use orbit_aigent::{BANDS, Command, FRAME_RATE, Frame, State};
use orbit_audio::Analyzer;

use crate::config::Config;

/// Listening gives up after this long without speech.
const NO_SPEECH: Duration = Duration::from_secs(6);
/// A question waits this long for its answer.
const NO_ANSWER: Duration = Duration::from_secs(10);
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
    /// The agent wants to use a tool; the answer goes to `reply`.
    Approve {
        tool: String,
        input: serde_json::Value,
        reply: UnixStream,
    },
    /// The agent's answer to request `id`.
    Answered { id: u64, answer: anyhow::Result<agent::Answer> },
    /// The speaker said all of `id`.
    Spoken { id: u64 },
    /// The session locked (or unlocked).
    Locked(bool),
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
        [one] if one == "approve" => return approve::serve(),
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
    eprintln!("usage: orbit-aigent [toggle|listen|cancel|yes|no|demo|agent <name>|approve]");
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
    let (say, orders) = mpsc::channel();
    let now = tts::Now::default();
    let hush = Arc::new(AtomicBool::new(false));
    {
        let events = events.clone();
        let voices = tts::voices(&config.voices);
        let (now, hush) = (now.clone(), hush.clone());
        std::thread::Builder::new().name("speaker".into()).spawn(move || {
            tts::speak(&voices, &orders, &now, &hush, |id| {
                let _ = events.send(Event::Spoken { id });
            });
        })?;
    }
    {
        let events = events.clone();
        std::thread::Builder::new().name("lock".into()).spawn(move || follow_lock(&events))?;
    }
    let engine = Engine::new(&config, jobs, events.clone(), Speaker { say, now, hush });
    std::thread::Builder::new()
        .name("socket".into())
        .spawn(move || serve(&listener, &events))?;
    engine.run(&inbox);
    Ok(())
}

/// Tells the engine when the session locks and unlocks (orbit-core's
/// `lock` event, sent once at once on subscribing). Under sway, or without
/// a compositor, nothing is said, and the engine takes it as unlocked.
fn follow_lock(events: &mpsc::Sender<Event>) {
    let Ok(mut ipc) = orbit_ipc::Connection::open() else { return };
    if ipc.request(orbit_ipc::SUBSCRIBE, r#"["lock"]"#).is_err() {
        return;
    }
    let Some(lock) = orbit_ipc::event_type("lock") else { return };
    while let Ok((kind, body)) = ipc.receive() {
        if kind != lock {
            continue;
        }
        let change: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
        if events.send(Event::Locked(change["change"] == "locked")).is_err() {
            return;
        }
    }
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
            let reply = stream.try_clone();
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Command>(&line) {
                    Ok(Command::Approve { tool, input }) => {
                        let Ok(reply) = reply.as_ref().map(UnixStream::try_clone) else { break };
                        let Ok(reply) = reply else { break };
                        if events.send(Event::Approve { tool, input, reply }).is_err() {
                            break;
                        }
                    }
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
    /// Speech while waiting: is it the wake word? With the faster model,
    /// but for the name alone, which base hears badly ("Jarvis." comes out
    /// "É viz."): short bits go to the larger one when base is it.
    Wake,
    /// The same with the larger model, when the small one heard something
    /// close to the wake word.
    WakeAgain,
    /// The request, with the larger one.
    Request,
    /// Yes or no to a question, with the faster one.
    Reply,
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
            Job::Wake if audio.len() > SHORT_WAKE || !config.wake_model.starts_with("base") => wake.as_mut(),
            Job::Reply => wake.as_mut().or(request.as_mut()),
            Job::Wake | Job::WakeAgain | Job::Request => request.as_mut().or(wake.as_mut()),
        };
        // The wake words for finding the name (in English, so not for a
        // request); requests as they are often said for hearing one.
        let prompt = match job {
            Job::Wake | Job::WakeAgain => &prompt,
            Job::Request => &config.hints,
            // Short answers heard as words ("sim", not "sem"; "pode",
            // not "Podge").
            Job::Reply => "Sim. Não. Pode. Yes. No.",
        };
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

/// Two short rising notes, played as its voice is (the echo canceller
/// takes them out of what it hears). Made once, in the runtime directory.
fn chime() {
    const NOTES: [(f32, f32); 2] = [(659.25, 0.07), (987.77, 0.09)];
    let path = std::env::var_os("XDG_RUNTIME_DIR")
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
        .join("orbit-aigent-chime.wav");
    if !path.exists() {
        let rate = mic::RATE as f32;
        let mut audio = Vec::new();
        for (pitch, length) in NOTES {
            let n = (length * rate) as usize;
            audio.extend((0..n).map(|i| {
                let t = i as f32 / rate;
                // In and out softly, no click at either end.
                let fade = (t / 0.005).min(1.0) * ((length - t) / 0.03).clamp(0.0, 1.0);
                0.18 * fade * (std::f32::consts::TAU * pitch * t).sin()
            }));
        }
        if let Err(err) = write_wav(&path, &audio) {
            return log::warn!("aigent: the chime: {err}");
        }
    }
    match std::process::Command::new("pw-play").arg("--media-role=Assistant").arg(&path).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(err) => log::warn!("aigent: pw-play: {err}"),
    }
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

/// The speaker's end the engine holds: orders to it, what it is playing,
/// and the flag that hushes it.
struct Speaker {
    say: mpsc::Sender<tts::Order>,
    now: tts::Now,
    hush: Arc<AtomicBool>,
}

/// What follows when the speaker is done.
#[derive(Clone, Copy, PartialEq)]
enum Then {
    Wait,
    /// Listen for the answer to the question just asked.
    Confirm,
}

struct Engine {
    config: Config,
    events: mpsc::Sender<Event>,
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
    /// The language of the request under way, `pt` or `en`.
    language: &'static str,
    /// The agent's conversation, resumed by each request; its run under
    /// way, and the request whose answer counts.
    session: Option<String>,
    running: agent::Running,
    request_id: u64,
    speaker: Speaker,
    /// What is being said, and what comes after it; its text, to know
    /// its own voice heard back through the speakers.
    saying: String,
    speech: u64,
    speaking: Option<(u64, Then)>,
    /// Bands of the voice: its analyser, and how far into the sentence
    /// playing it has got (that sentence's start).
    voice: Analyzer,
    voice_rate: u32,
    voice_at: (Option<Instant>, usize),
    /// Where the answer to the question asked goes; what it would allow
    /// (a program, a file), and what was allowed so far in this request,
    /// not asked again.
    approval: Option<UnixStream>,
    asked: Vec<String>,
    allowed: Vec<String>,
    /// Whether the session is locked: nothing is heard or done then. Not
    /// known yet at start, which counts as locked until the compositor
    /// says (or, after 2 s, has nothing to say).
    locked: Option<bool>,
    started: Instant,
    /// Blocks heard, for a word in the log now and then, and the loudest
    /// since the last.
    blocks: u64,
    loudest: f32,
}

/// How long the compositor has to say whether the session is locked.
const LOCK_UNSAID: Duration = Duration::from_secs(2);

/// Piper's rate, until a sentence says otherwise.
const VOICE_RATE: u32 = 22_050;
/// From spawning pw-play to its sound.
const PLAY_LATENCY: Duration = Duration::from_millis(80);

/// Speech while waiting this short may be the name alone.
const SHORT_WAKE: usize = 5 * mic::RATE as usize / 2;
/// Speech while waiting longer than this is not someone calling it.
const LONGEST_WAKE: usize = 10 * mic::RATE as usize;
const BEFORE: usize = 3 * mic::RATE as usize / 10;

impl Engine {
    fn new(config: &Config, jobs: mpsc::Sender<(u64, Job, Vec<f32>)>, events: mpsc::Sender<Event>, speaker: Speaker) -> Self {
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
            config: config.clone(),
            events,
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
            language: stt::LANGUAGES[0],
            session: None,
            running: agent::Running::default(),
            request_id: 0,
            speaker,
            saying: String::new(),
            speech: 0,
            speaking: None,
            voice: Analyzer::new(VOICE_RATE, BANDS, 80.0, 6_000.0, 0.77, FRAME_RATE),
            voice_rate: VOICE_RATE,
            voice_at: (None, 0),
            approval: None,
            asked: Vec::new(),
            allowed: Vec::new(),
            locked: None,
            started: Instant::now(),
            blocks: 0,
            loudest: f32::NEG_INFINITY,
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
                Ok(Event::Locked(locked)) => {
                    log::info!("aigent: the session is {}", if locked { "locked: deaf" } else { "unlocked" });
                    if locked {
                        self.cancel();
                    }
                    self.locked = Some(locked);
                }
                Ok(Event::Approve { tool, input, reply }) => self.approve(&tool, &input, reply),
                Ok(Event::Answered { id, answer }) => {
                    if id == self.request_id {
                        self.answered(answer);
                    }
                }
                Ok(Event::Spoken { id }) => {
                    if let Some((speech, then)) = self.speaking
                        && speech == id
                    {
                        self.speaking = None;
                        match then {
                            Then::Wait => self.enter(State::Idle),
                            Then::Confirm => self.enter(State::Confirm),
                        }
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
        // Woken (by its name or a click): it listens now.
        if state == State::Listening && self.state == State::Idle && self.config.chime {
            chime();
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
        // Locked, only what stops it.
        if self.locked != Some(false) && !matches!(command, Command::Cancel | Command::No | Command::Agent { .. }) {
            log::info!("aigent: locked, {command:?} ignored");
            return;
        }
        match command {
            Command::Toggle if self.state == State::Idle => self.enter(State::Listening),
            Command::Toggle | Command::Cancel => self.cancel(),
            Command::Listen => self.enter(State::Listening),
            Command::Yes | Command::No if self.approval.is_some() => self.decide(command == Command::Yes),
            Command::Yes | Command::No => {}
            // Comes as Event::Approve, with the connection to answer on.
            Command::Approve { .. } => {}
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
        if self.locked != Some(false) {
            if self.locked.is_none() && self.started.elapsed() > LOCK_UNSAID {
                log::info!("aigent: the compositor says nothing of a lock: taken as unlocked");
                self.locked = Some(false);
            }
            return;
        }
        let event = self.vad.push(block);
        if let Some(event) = event {
            log::debug!("aigent: speech {event:?} ({:?}, {:.0} dB)", self.state, vad::level(block));
        }
        self.blocks += 1;
        self.loudest = self.loudest.max(vad::level(block));
        if self.blocks.is_multiple_of(1000) {
            log::debug!(
                "aigent: {:?}, room at {:.0} dB, loudest {:.0} dB, speaking {}, Whisper busy {}",
                self.state,
                self.vad.floor().unwrap_or(0.0),
                self.loudest,
                self.vad.speaking(),
                self.waiting_for.is_some()
            );
            self.loudest = f32::NEG_INFINITY;
        }
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
            // Saying an answer, it listens for its name too, to be cut
            // short ("Jarvis, para", or a new request).
            State::Speaking if self.interruptible() => self.wait(event),
            // Answering while the question is still being said: it stops,
            // and the answer is heard from its start.
            State::Speaking
                if matches!(self.speaking, Some((_, Then::Confirm))) && event == Some(vad::Event::Started) =>
            {
                log::info!("aigent: answered over the question");
                self.speaker.hush.store(true, Ordering::SeqCst);
                self.speaking = None;
                self.after_wake = Some(self.before.iter().copied().collect());
                self.enter(State::Confirm);
            }
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

    /// The request (or the answer to a question) is over: the pause after
    /// it, or too long.
    fn heard_all(&mut self) {
        let audio = self.recording.take().unwrap_or_default();
        log::info!("aigent: heard {:.1} s", audio.len() as f32 / mic::RATE as f32);
        let job = if self.state == State::Confirm { Job::Reply } else { Job::Request };
        self.enter(State::Thinking);
        if self.demo == Demo::Off {
            self.ask(job, audio);
        }
    }

    /// Whisper's answer to the job still wanted.
    fn heard_back(&mut self, job: Job, heard: Option<stt::Heard>, audio: Vec<f32>) {
        let Some(heard) = heard else {
            if job == Job::Reply {
                self.decide(false);
            } else {
                self.enter(State::Idle);
            }
            return;
        };
        match job {
            Job::Wake | Job::WakeAgain => {
                if self.state != State::Idle && !self.interruptible() {
                    return;
                }
                if self.state == State::Speaking && echo(&heard.text, &self.saying) {
                    log::debug!("aigent: its own voice heard back: {:?}", heard.text);
                    self.after_wake = None;
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
                if self.state == State::Speaking {
                    log::info!("aigent: interrupted");
                    self.speaker.hush.store(true, Ordering::SeqCst);
                    self.speaking = None;
                }
                // "Jarvis, para": done, without hearing it again.
                if is_stop(&rest) {
                    log::info!("aigent: told to stop: {rest:?}");
                    self.enter(State::Idle);
                    return;
                }
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
                // The name again and nothing else: still listening.
                if request.is_empty() && !self.retried {
                    log::info!("aigent: the name again, listening on");
                    self.enter(State::Listening);
                    self.retried = true;
                    return;
                }
                self.request(request, heard.language);
            }
            // Its own question heard back (it stopped for it): listen again.
            Job::Reply if echo(&heard.text, &self.saying) => {
                log::debug!("aigent: its own question heard back: {:?}", heard.text);
                self.enter(State::Confirm);
            }
            Job::Reply => match yes_or_no(&heard.text) {
                Some(yes) => self.decide(yes),
                None if !self.retried => {
                    self.retried = true;
                    let again = if self.language == "en" { "Yes or no?" } else { "Sim ou não?" };
                    self.say(again, Then::Confirm);
                }
                None => self.decide(false),
            },
        }
    }

    /// Whether what is being said may be cut short by the name: an answer,
    /// not a question waiting for its own.
    fn interruptible(&self) -> bool {
        self.state == State::Speaking && matches!(self.speaking, Some((_, Then::Wait)))
    }

    /// What to do, in `language`.
    fn request(&mut self, request: String, language: &str) {
        if request.trim().is_empty() {
            log::info!("aigent: nothing to do");
            self.enter(State::Idle);
            return;
        }
        if is_stop(&request) {
            log::info!("aigent: told to stop: {request:?}");
            self.enter(State::Idle);
            return;
        }
        log::info!("aigent: request ({language}) for {}: {request:?}", self.agent);
        self.language = if language == "en" { "en" } else { "pt" };
        self.text = request.clone();
        self.retried = false;
        self.allowed.clear();
        let Some(agent) = self.config.agents.get(&self.agent).cloned() else {
            let none = if self.language == "en" {
                "There is no agent installed to do that."
            } else {
                "Não há nenhum agente instalado para isso."
            };
            self.say(none, Then::Wait);
            return;
        };
        self.enter(State::Thinking);
        self.request_id += 1;
        let (id, name, session) = (self.request_id, self.agent.clone(), self.session.clone());
        let (running, events) = (self.running.clone(), self.events.clone());
        let spawned = std::thread::Builder::new().name("agent".into()).spawn(move || {
            let answer = agent::ask(&name, &agent, &request, session.as_deref(), &running);
            let _ = events.send(Event::Answered { id, answer });
        });
        if let Err(err) = spawned {
            log::error!("aigent: starting the agent: {err}");
            self.enter(State::Idle);
        }
    }

    /// The agent's answer, said.
    fn answered(&mut self, answer: anyhow::Result<agent::Answer>) {
        // An approval left asking (the agent gave up meanwhile).
        if let Some(mut reply) = self.approval.take() {
            let _ = reply.write_all(b"{\"approved\":false}\n");
        }
        match answer {
            Ok(answer) => {
                log::info!("aigent: {} answered: {:?}", self.agent, answer.text);
                self.session = answer.session;
                self.text = answer.text.clone();
                self.say(&answer.text, Then::Wait);
            }
            Err(err) => {
                log::warn!("aigent: {}: {err:#}", self.agent);
                let sorry = if self.language == "en" {
                    "Sorry, the agent could not answer."
                } else {
                    "Desculpe, o agente não conseguiu responder."
                };
                self.say(sorry, Then::Wait);
            }
        }
    }

    /// Says `text` in the request's language, then `then`.
    fn say(&mut self, text: &str, then: Then) {
        self.saying = text.to_owned();
        self.speech += 1;
        self.speaking = Some((self.speech, then));
        self.speaker.hush.store(false, Ordering::SeqCst);
        let order = tts::Order::Say {
            id: self.speech,
            text: text.to_owned(),
            language: self.language.to_owned(),
        };
        if self.speaker.say.send(order).is_err() {
            log::error!("aigent: the speaker is gone");
        }
        self.enter(State::Speaking);
    }

    /// The agent asks to use `tool`: the question said, then listened for.
    fn approve(&mut self, tool: &str, input: &serde_json::Value, mut reply: UnixStream) {
        if self.approval.is_some() || !matches!(self.state, State::Thinking) || self.locked != Some(false) {
            let _ = reply.write_all(b"{\"approved\":false}\n");
            return;
        }
        let wants = wants(tool, input);
        if !wants.is_empty() && wants.iter().all(|w| self.allowed.contains(w)) {
            log::info!("aigent: {tool} allowed already in this request: {wants:?}");
            let _ = reply.write_all(b"{\"approved\":true}\n");
            return;
        }
        let (question, detail) = question(tool, input, self.language);
        log::info!("aigent: asking: {question} ({detail})");
        self.asked = wants;
        self.text = detail;
        self.approval = Some(reply);
        self.retried = false;
        self.say(&question, Then::Confirm);
    }

    /// The answer to the question asked.
    fn decide(&mut self, yes: bool) {
        log::info!("aigent: the answer is {}", if yes { "yes" } else { "no" });
        if yes {
            self.allowed.append(&mut self.asked);
        }
        if let Some(mut reply) = self.approval.take() {
            let line = format!("{{\"approved\":{yes}}}\n");
            let _ = reply.write_all(line.as_bytes());
        }
        self.speaker.hush.store(true, Ordering::SeqCst);
        self.speaking = None;
        self.enter(State::Thinking);
    }

    /// Stops everything: the agent, the voice, a question.
    fn cancel(&mut self) {
        if let Some(mut reply) = self.approval.take() {
            let _ = reply.write_all(b"{\"approved\":false}\n");
        }
        // Its answer, should it still come, no longer counts.
        self.request_id += 1;
        agent::stop(&self.running);
        self.speaker.hush.store(true, Ordering::SeqCst);
        self.speaking = None;
        self.enter(State::Idle);
    }

    /// The voice's bands now: what of the sentence playing has been heard
    /// since the last frame, through the analyser.
    fn voice_bands(&mut self) -> Vec<f32> {
        let playing = self.speaker.now.lock().unwrap().clone();
        let Some(playing) = playing else {
            // Between sentences: the bars fall.
            self.voice.push(&vec![0.0; (self.voice_rate / FRAME_RATE) as usize]);
            return self.voice.frame();
        };
        if self.voice_at.0 != Some(playing.since) {
            self.voice_at = (Some(playing.since), 0);
            if playing.rate != self.voice_rate {
                self.voice_rate = playing.rate;
                self.voice = Analyzer::new(playing.rate, BANDS, 80.0, 6_000.0, 0.77, FRAME_RATE);
            }
        }
        // pw-play starts a little after it is spawned.
        let heard = playing.since.elapsed().saturating_sub(PLAY_LATENCY).as_secs_f32();
        let at = ((heard * playing.rate as f32) as usize).min(playing.samples.len());
        let from = self.voice_at.1.min(at);
        self.voice.push(&playing.samples[from..at]);
        self.voice_at.1 = at;
        self.voice.frame()
    }

    /// Every tick: timeouts, the demo's script, and the frame to the clients.
    fn frame(&mut self) {
        let elapsed = self.since.elapsed();
        match self.state {
            State::Listening | State::Confirm => {
                let patience = if self.state == State::Confirm { NO_ANSWER } else { NO_SPEECH };
                if (!self.heard && elapsed >= patience) || elapsed >= LONGEST {
                    if self.heard {
                        self.heard_all();
                    } else if self.state == State::Confirm {
                        let heard = self.recording.as_deref().unwrap_or_default();
                        let loudest = heard.chunks(mic::BLOCK).map(vad::level).fold(f32::NEG_INFINITY, f32::max);
                        log::info!("aigent: no answer (the loudest 30 ms at {loudest:.0} dB)");
                        if let Some(dir) = std::env::var_os("ORBIT_AIGENT_DUMP") {
                            let path = PathBuf::from(dir).join(format!("{:04}-NoAnswer.wav", self.job));
                            let _ = write_wav(&path, heard);
                        }
                        self.decide(false);
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
            State::Speaking => self.bands = self.voice_bands(),
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
/// it puts every word at the end. Instead, by the sound's loudness in 10 ms
/// frames: the bursts of speech, apart from a breath or a click before
/// them (short, or well below the loudest); where the name should end, by
/// its share of the letters; and the pause between two bursts nearest to
/// that, cut in its middle. Said without a pause, the quietest moment
/// within a quarter of a second of it.
fn name_end(audio: &[f32], text: &str, taken: usize) -> usize {
    const FRAME: usize = mic::RATE as usize / 100;
    let levels: Vec<f32> = audio.chunks(FRAME).map(vad::level).collect();
    let loudest = levels.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let bursts = bursts(&levels, loudest);
    let (Some(first), Some(last)) = (bursts.first(), bursts.last()) else {
        return 0;
    };
    let (start, stop) = (first.0, last.1);
    let letters = |words: &[&str]| words.iter().map(|w| w.chars().filter(|c| c.is_alphanumeric()).count()).sum::<usize>();
    let words: Vec<&str> = text.split_whitespace().collect();
    let share = letters(&words[..taken.min(words.len())]) as f32 / letters(&words).max(1) as f32;
    let guess = start + ((stop - start) as f32 * share) as usize;
    let pause = bursts
        .windows(2)
        .map(|pair| (pair[0].1 + pair[1].0) / 2)
        .min_by_key(|middle| middle.abs_diff(guess));
    let frame = pause.unwrap_or_else(|| {
        let (from, to) = (guess.saturating_sub(25).max(start), (guess + 25).min(stop));
        (from..=to).min_by(|a, b| levels[*a].total_cmp(&levels[*b])).unwrap_or(guess)
    });
    (frame * FRAME).min(audio.len())
}

/// The bursts of speech in `levels` (dB, 10 ms frames), as frame ranges:
/// runs within 25 dB of the loudest, joined across gaps under 80 ms, at
/// least 150 ms long and reaching within 12 dB of it.
fn bursts(levels: &[f32], loudest: f32) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (i, level) in levels.iter().enumerate() {
        if *level <= loudest - 25.0 {
            continue;
        }
        match runs.last_mut() {
            Some(run) if i - run.1 < 8 => run.1 = i + 1,
            _ => runs.push((i, i + 1)),
        }
    }
    runs.retain(|&(a, b)| b - a >= 15 && levels[a..b].iter().any(|l| *l >= loudest - 12.0));
    runs
}

/// The question for the agent's wish to use `tool` with `input`, in
/// `language`, and the whole of it for the tooltip.
fn question(tool: &str, input: &serde_json::Value, language: &str) -> (String, String) {
    let en = language == "en";
    let file = || {
        input["file_path"]
            .as_str()
            .or(input["notebook_path"].as_str())
            .map(|path| path.rsplit('/').next().unwrap_or(path).to_owned())
            .unwrap_or_default()
    };
    match tool {
        "Bash" => {
            // The programs, not the command line, which is no pleasure to
            // hear; nor Claude's description of it, written in English.
            // A command to the desktop, what it does.
            let command = input["command"].as_str().unwrap_or_default();
            let programs = programs(command);
            let and = if en { " and " } else { " e " };
            if let Some(actions) = desktop_actions(command, en) {
                let said = if en { format!("May I {}?", actions.join(and)) } else { format!("Posso {}?", actions.join(and)) };
                return (said, format!("$ {command}"));
            }
            let said = match (programs.is_empty(), en) {
                (true, true) => "May I run a command?".to_owned(),
                (true, false) => "Posso executar um comando?".to_owned(),
                (false, true) => format!("May I run {}?", programs.join(and)),
                (false, false) => format!("Posso executar {}?", programs.join(and)),
            };
            (said, format!("$ {command}"))
        }
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            let name = file();
            let said = if en { format!("May I change the file {name}?") } else { format!("Posso alterar o arquivo {name}?") };
            (said, format!("{tool} {}", input["file_path"].as_str().unwrap_or(&name)))
        }
        _ => {
            let said = if en { format!("May I use {tool}?") } else { format!("Posso usar {tool}?") };
            (said, format!("{tool} {input}"))
        }
    }
}

/// What a command to the desktop (`orbitctl` or `swaymsg` with a command
/// of sway's) does, said: "abrir o kitty", "mover a janela para o monitor
/// HDMI-A-1". `None` for any other command line.
fn desktop_actions(command: &str, en: bool) -> Option<Vec<String>> {
    // Each simple command of the line (`a && b`) must be one to the desktop.
    let mut out = Vec::new();
    for simple in simple_commands(command) {
        for said in desktop_command(&simple, en)? {
            if !out.contains(&said) {
                out.push(said);
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

/// `desktop_actions` for one simple command.
fn desktop_command(command: &str, en: bool) -> Option<Vec<String>> {
    let words = shell_words(command);
    let (program, rest) = words.split_first()?;
    if !matches!(program.rsplit('/').next(), Some("orbitctl" | "swaymsg")) || rest.first().is_some_and(|w| w.starts_with('-')) {
        return None;
    }
    let mut out = Vec::new();
    for part in rest.join(" ").split([';', ',']) {
        // Criteria first: `[app_id="kitty"] move …`.
        let part = part.trim();
        let part = match part.strip_prefix('[') {
            Some(after) => after.split_once(']').map_or("", |(_, rest)| rest.trim()),
            None => part,
        };
        let w: Vec<&str> = part.split_whitespace().collect();
        let said = match (w.as_slice(), en) {
            ([], _) => continue,
            (["exec", program, ..], true) => format!("open {}", program.rsplit('/').next().unwrap_or(program)),
            (["exec", program, ..], false) => format!("abrir o {}", program.rsplit('/').next().unwrap_or(program)),
            ([.., "output", name], true) if w[0] == "move" => format!("move the window to the monitor {name}"),
            ([.., "output", name], false) if w[0] == "move" => format!("mover a janela para o monitor {name}"),
            (["move", .., "workspace", n], true) => format!("move the window to workspace {n}"),
            (["move", .., "workspace", n], false) => format!("mover a janela para a área {n}"),
            (["move", ..], true) => "move the window".to_owned(),
            (["move", ..], false) => "mover a janela".to_owned(),
            (["focus", ..], _) => continue,
            (["workspace", n, ..], true) => format!("go to workspace {n}"),
            (["workspace", n, ..], false) => format!("ir para a área {n}"),
            (["kill", ..], true) => "close the window".to_owned(),
            (["kill", ..], false) => "fechar a janela".to_owned(),
            (_, true) => "change the desktop".to_owned(),
            (_, false) => "mudar a área de trabalho".to_owned(),
        };
        if !out.contains(&said) {
            out.push(said);
        }
    }
    if out.is_empty() {
        out.push(if en { "change the focus" } else { "mudar o foco" }.to_owned());
    }
    Some(out)
}

/// `command` split into words as a shell would, quotes taken off; only
/// for one simple command (it stops at `|`, `;` or `&` outside quotes).
fn shell_words(command: &str) -> Vec<String> {
    let (mut words, mut word, mut quote, mut any) = (Vec::new(), String::new(), None, false);
    for c in command.chars() {
        match (quote, c) {
            (None, '\'' | '"') => (quote, any) = (Some(c), true),
            (Some(q), c) if c == q => quote = None,
            (None, '|' | ';' | '&') => break,
            (None, c) if c.is_whitespace() => {
                if any {
                    words.push(std::mem::take(&mut word));
                    any = false;
                }
            }
            _ => {
                word.push(c);
                any = true;
            }
        }
    }
    if any {
        words.push(word);
    }
    words
}

/// The simple commands of a command line: its parts between pipes, `;`
/// and `&&` that are not quoted.
fn simple_commands(command: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut quote = None;
    for c in command.chars() {
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '|' | ';' | '&' | '\n') => parts.push(String::new()),
            _ => {}
        }
        if let Some(part) = parts.last_mut() {
            part.push(c);
        }
    }
    parts
        .iter()
        .map(|part| part.trim_start_matches(['|', ';', '&', '\n']).trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect()
}

/// The programs a command line runs, at most three, each once: the first
/// word of each simple command, past `sudo` and settings of variables.
fn programs(command: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in &simple_commands(command) {
        let program = part
            .split_whitespace()
            .find(|w| *w != "sudo" && !w.contains('=') && !w.starts_with('('))
            .map(|w| w.rsplit('/').next().unwrap_or(w).trim_matches(['"', '\'', '`', ')']).to_owned());
        if let Some(program) = program
            && !program.is_empty()
            && !out.contains(&program)
            && out.len() < 3
        {
            out.push(program);
        }
    }
    out
}

/// What a yes to `tool` with `input` allows for the rest of the request:
/// its programs (for a command) or its file.
fn wants(tool: &str, input: &serde_json::Value) -> Vec<String> {
    match tool {
        "Bash" => programs(input["command"].as_str().unwrap_or_default()),
        _ => match input["file_path"].as_str().or(input["notebook_path"].as_str()) {
            Some(path) => vec![format!("{tool} {path}")],
            None => vec![tool.to_owned()],
        },
    }
}

/// Whether a request only says to stop ("para", "chega", "stop"): it
/// silences the voice and goes no further.
fn is_stop(request: &str) -> bool {
    // Said around the word that stops it, and left out.
    const AROUND: [&str; 14] = ["por", "favor", "please", "ok", "a", "com", "isso", "boca", "agora", "now", "up", "it", "jarvis", "orbit"];
    const STOP: [&str; 15] = [
        "para", "pare", "parar", "chega", "cancela", "cancelar", "silêncio", "silencio", "quieto", "cala", "stop", "cancel", "enough",
        "quiet", "shut",
    ];
    let words: Vec<String> = request
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .filter(|w| !AROUND.contains(&w.as_str()))
        .collect();
    // Whisper may say it over and over ("Para. Para. Para.").
    let mut words = words;
    words.dedup();
    !words.is_empty() && words.len() <= 2 && words.iter().all(|w| STOP.contains(&w.as_str()))
}

/// Whether `heard` is `saying` coming back through the speakers: most of
/// its words (at least two) are words of what is being said. The echo
/// canceller takes most of it out, not always all.
fn echo(heard: &str, saying: &str) -> bool {
    let words = |text: &str| -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
    };
    let (heard, saying) = (words(heard), words(saying));
    let common = heard.iter().filter(|w| saying.contains(w)).count();
    heard.len() >= 2 && common * 10 >= heard.len() * 7
}

/// Yes or no in what was said, Portuguese or English; `None` when unclear.
fn yes_or_no(text: &str) -> Option<bool> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let has = |list: &[&str]| words.iter().any(|w| list.contains(&w.as_str()));
    // "Não pode" is no: the no words first.
    if has(&["não", "nao", "no", "nope", "cancela", "cancelar", "negativo", "pare", "don't", "stop"]) {
        return Some(false);
    }
    // Also as Whisper sometimes writes them, heard alone.
    if has(&["sim", "sem", "si", "pode", "podge", "pod", "claro", "ok", "okay", "confirmo", "confirma", "manda", "vai", "faz", "faça", "yes", "yeah", "yep", "sure", "go"]) {
        return Some(true);
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 10 ms frames at these levels (dB), as sound.
    fn sound(levels: &[(f32, usize)]) -> Vec<f32> {
        let mut out = Vec::new();
        for &(db, frames) in levels {
            let amplitude = 10f32.powf(db / 20.0) * std::f32::consts::SQRT_2;
            for i in 0..frames * 160 {
                out.push(amplitude * (i as f32 * 0.2).sin());
            }
        }
        out
    }

    #[test]
    fn its_own_voice_is_known() {
        let saying = "São vinte e duas e quarenta e oito. Quer mais alguma coisa?";
        assert!(echo("vinte e duas e quarenta", saying));
        assert!(!echo("Jarvis, para.", saying));
        assert!(!echo("Sim.", "Posso abrir o kitty?"));
        assert!(echo("Posso abrir o kitty", "Posso abrir o kitty?"));
    }

    #[test]
    fn stop_requests_are_told_apart() {
        for stop in ["Para.", "Chega!", "Cancela, por favor.", "Stop.", "Cala a boca.", "Para com isso.", "Para. Para. Para. Para."] {
            assert!(is_stop(stop), "{stop}");
        }
        for request in ["Para onde vai o arquivo?", "Abre o terminal.", "Stop the music player.", "Isso"] {
            assert!(!is_stop(request), "{request}");
        }
    }

    #[test]
    fn yes_and_no_in_both_languages() {
        assert_eq!(yes_or_no("Sim, pode."), Some(true));
        assert_eq!(yes_or_no("Não pode não."), Some(false));
        assert_eq!(yes_or_no("Yes, go ahead."), Some(true));
        assert_eq!(yes_or_no("No."), Some(false));
        assert_eq!(yes_or_no("Hmm, talvez."), None);
        assert_eq!(yes_or_no("sem"), Some(true));
        assert_eq!(yes_or_no("Podge."), Some(true));
    }

    #[test]
    fn the_question_names_the_programs() {
        let input = serde_json::json!({"command": "swaymsg -t get_outputs -r | jq -r '.[] | .name'; LANG=C /usr/bin/kitty &", "description": "x"});
        assert_eq!(question("Bash", &input, "pt").0, "Posso executar swaymsg e jq e kitty?");
    }

    #[test]
    fn a_desktop_command_is_said_as_what_it_does() {
        let ask = |command: &str| question("Bash", &serde_json::json!({ "command": command }), "pt").0;
        assert_eq!(ask("orbitctl 'focus output HDMI-A-1; exec kitty'"), "Posso abrir o kitty?");
        assert_eq!(
            ask("orbitctl '[app_id=\"kitty\"] move container to output HDMI-A-1, focus'"),
            "Posso mover a janela para o monitor HDMI-A-1?"
        );
        assert_eq!(ask("swaymsg -t get_tree"), "Posso executar swaymsg?");
        assert_eq!(ask("orbitctl workspace 11 && orbitctl exec iptvnator"), "Posso ir para a área 11 e abrir o iptvnator?");
    }

    #[test]
    fn the_question_names_the_file() {
        let input = serde_json::json!({"file_path": "/home/u/notas.txt", "content": "x"});
        assert_eq!(question("Write", &input, "pt").0, "Posso alterar o arquivo notas.txt?");
    }

    #[test]
    fn the_name_is_cut_in_the_pause_after_it() {
        // A breath, "Jarvis", a pause, "que horas são", as the K66 gave it.
        let audio = sound(&[(-66.0, 10), (-45.0, 30), (-58.0, 50), (-30.0, 60), (-62.0, 35), (-35.0, 50), (-65.0, 80)]);
        let cut = name_end(&audio, "Jarvis, que a ração.", 1) as f32 / mic::RATE as f32;
        assert!((1.5..1.85).contains(&cut), "{cut}");
    }

    #[test]
    fn without_a_pause_it_goes_by_the_letters() {
        // "Orbit" and the request run together, a dip between the words.
        let audio = sound(&[(-60.0, 20), (-30.0, 30), (-40.0, 3), (-30.0, 120), (-60.0, 20)]);
        let cut = name_end(&audio, "Orbit list the files in my home folder", 1) as f32 / mic::RATE as f32;
        assert!((0.5..0.53).contains(&cut), "{cut}");
    }
}
