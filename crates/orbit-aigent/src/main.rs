//! Orbit-Aigent: talk to a coding agent by voice.
//!
//!   orbit-aigent                 the daemon (started with the session)
//!   orbit-aigent toggle|listen|cancel|yes|no|demo
//!   orbit-aigent agent <name>    use another agent from now on
//!
//! The daemon keeps the microphone open, waits for its wake word, hears
//! the request until a pause, hands it to the agent in use and speaks the
//! answer, telling the panel's aigent module what it is doing (`Frame`).
//! Three threads: the microphone's, the socket's (one more per client
//! reading its commands) and the engine, which owns the state and is the
//! only one that writes to the clients.

mod config;
mod mic;
mod vad;

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
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

    let (events, inbox) = mpsc::channel();
    {
        let events = events.clone();
        std::thread::Builder::new()
            .name("mic".into())
            .spawn(move || mic::record(|block| events.send(Event::Block(block.to_vec())).is_ok()))?;
    }
    std::thread::Builder::new()
        .name("socket".into())
        .spawn(move || serve(&listener, &events))?;
    Engine::new(Config::load()).run(&inbox);
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
    state: State,
    /// When the state was entered.
    since: Instant,
    text: String,
    vad: vad::Vad,
    /// Whether speech started since listening began.
    heard: bool,
    analyzer: Analyzer,
    bands: Vec<f32>,
    demo: Demo,
    clients: Vec<UnixStream>,
    /// Something changed since the last frame went out.
    dirty: bool,
}

impl Engine {
    fn new(config: Config) -> Self {
        let agents = config.usable();
        let agent = if agents.contains(&config.agent) {
            config.agent.clone()
        } else {
            if !config.agent.is_empty() {
                log::warn!("aigent: agent {} is not set up or not installed", config.agent);
            }
            agents.first().cloned().unwrap_or_default()
        };
        log::info!("aigent: agents {agents:?}, using {agent:?}");
        Self {
            agents,
            agent,
            state: State::Idle,
            since: Instant::now(),
            text: String::new(),
            vad: vad::Vad::default(),
            heard: false,
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
                self.vad.reset();
                self.heard = false;
            }
            State::Idle => self.demo = Demo::Off,
            _ => {}
        }
        if !moves(state) {
            self.bands.clear();
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
    /// know the room; the rest only while listening.
    fn block(&mut self, block: &[f32]) {
        let event = self.vad.push(block);
        if !matches!(self.state, State::Listening | State::Confirm) {
            return;
        }
        self.analyzer.push(block);
        match event {
            Some(vad::Event::Started) => self.heard = true,
            Some(vad::Event::Ended) => self.heard_all(),
            None => {}
        }
    }

    /// The request is over: the pause after it, or too long.
    fn heard_all(&mut self) {
        let took = self.since.elapsed().as_secs_f32();
        self.text = format!("{took:.1} s heard");
        log::info!("aigent: heard {took:.1} s");
        if self.demo == Demo::On {
            self.enter(State::Thinking);
        } else {
            self.enter(State::Idle);
        }
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
