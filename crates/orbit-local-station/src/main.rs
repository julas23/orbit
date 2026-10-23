//! Orbit-Local-Station: a thin client with no desktop of its own. It takes
//! the screen and the keyboard straight from a text console (no
//! compositor), pairs with an orbit-remote-server and shows that session
//! (pending §9).
//!
//!   orbit-local-station [SERVER]     SERVER as host[:port]
//!
//! The server comes from the command line, else `$ORBIT_STATION_SERVER`,
//! else `server:` in `/etc/orbit/station.yaml`; `output:` there picks the
//! monitor (as `HDMI-A-1`), else the first connected one.
//!
//! Ctrl+Alt+F1…F12 change the console; Ctrl+Alt+Backspace quits.

mod link;
mod screen;
mod ui;

use std::{collections::HashSet, sync::mpsc};

use anyhow::{Context, anyhow};
use serde::Deserialize;
use smithay::reexports::calloop::{EventLoop, channel};

use crate::{
    link::Event,
    screen::{Key, Screen},
    ui::Ui,
};

/// orbit-remote-server's port.
const PORT: u16 = 7311;
const CONFIG: &str = "/etc/orbit/station.yaml";
const CODE_LENGTH: usize = 6;

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    server: Option<String>,
    output: Option<String>,
}

fn config() -> anyhow::Result<Config> {
    match std::fs::read_to_string(CONFIG) {
        Ok(text) => serde_yaml_ng::from_str(&text).with_context(|| format!("reading {CONFIG}")),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(err) => Err(anyhow!("reading {CONFIG}: {err}")),
    }
}

pub struct Station {
    screen: Screen,
    ui: Ui,
    /// Ctrl+Alt+Backspace was pressed.
    quit: bool,
    codes: mpsc::Sender<String>,
    /// The code being typed, while one is asked for.
    code: Option<String>,
    /// The keys held down, by their evdev code.
    held: HashSet<u32>,
}

// evdev's codes (linux/input-event-codes.h).
const KEY_ESC: u32 = 1;
const KEY_BACKSPACE: u32 = 14;
const KEY_ENTER: u32 = 28;
const KEY_KPENTER: u32 = 96;
const KEY_LEFTCTRL: u32 = 29;
const KEY_RIGHTCTRL: u32 = 97;
const KEY_LEFTALT: u32 = 56;
const KEY_RIGHTALT: u32 = 100;

/// The digit a key types: the row above the letters, or the keypad.
fn digit(code: u32) -> Option<char> {
    let n = match code {
        2..=10 => code - 1,
        11 => 0,
        71..=73 => code - 64,
        75..=77 => code - 71,
        79..=81 => code - 78,
        82 => 0,
        _ => return None,
    };
    char::from_digit(n, 10)
}

/// The console a function key goes to with Ctrl+Alt.
fn console(code: u32) -> Option<i32> {
    match code {
        59..=68 => Some(code as i32 - 58),
        87 => Some(11),
        88 => Some(12),
        _ => None,
    }
}

impl Station {
    pub fn key(&mut self, key: Key) {
        if !key.down {
            self.held.remove(&key.code);
            return;
        }
        self.held.insert(key.code);
        let ctrl = self.held.contains(&KEY_LEFTCTRL) || self.held.contains(&KEY_RIGHTCTRL);
        let alt = self.held.contains(&KEY_LEFTALT) || self.held.contains(&KEY_RIGHTALT);
        if ctrl && alt {
            if let Some(vt) = console(key.code) {
                if let Err(err) = smithay::backend::session::Session::change_vt(&mut self.screen.session, vt) {
                    log::warn!("VT {vt}: {err}");
                }
                return;
            }
            if key.code == KEY_BACKSPACE {
                log::info!("station: Ctrl+Alt+Backspace, quitting");
                self.quit = true;
                return;
            }
        }
        let Some(code) = &mut self.code else { return };
        match key.code {
            KEY_BACKSPACE => {
                code.pop();
            }
            KEY_ESC => code.clear(),
            KEY_ENTER | KEY_KPENTER if code.len() == CODE_LENGTH => {}
            other => match digit(other) {
                Some(d) if code.len() < CODE_LENGTH => code.push(d),
                _ => return,
            },
        }
        self.ui.face.set_code(code.clone().into());
        self.ui.face.set_problem("".into());
        // The sixth digit sends it.
        if code.len() == CODE_LENGTH {
            let code = self.code.take().unwrap_or_default();
            let _ = self.codes.send(code);
            self.ui.face.set_asking(false);
            self.ui.face.set_code("".into());
        }
    }

    fn link(&mut self, event: Event) {
        let face = &self.ui.face;
        match event {
            Event::Connecting => face.set_status("Connecting…".into()),
            Event::Unreachable(why) => {
                log::warn!("station: {why}");
                face.set_status("Waiting for the server".into());
                face.set_problem(why.into());
            }
            Event::Code { wrong } => {
                self.code = Some(String::new());
                face.set_asking(true);
                face.set_code("".into());
                face.set_status("Type the code shown on the server's screen".into());
                face.set_problem(if wrong { "That code was not right".into() } else { "".into() });
            }
            Event::Busy => {
                face.set_status("The server is in use by another client".into());
                face.set_problem("".into());
            }
            Event::Paired => {
                face.set_status("Paired".into());
                face.set_problem("".into());
            }
            Event::Lost(why) => {
                face.set_status("The session ended".into());
                face.set_problem(why.into());
            }
        }
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    if let Err(err) = run() {
        log::error!("station: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let config = config()?;
    let server = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("ORBIT_STATION_SERVER").ok())
        .or(config.server)
        .ok_or_else(|| anyhow!("no server: give it as an argument, or as `server:` in {CONFIG}"))?;
    let server = if server.contains(':') { server } else { format!("{server}:{PORT}") };

    let mut event_loop: EventLoop<'static, Station> = EventLoop::try_new()?;
    let screen = Screen::open(&mut event_loop, config.output.as_deref())?;
    let ui = Ui::new(screen.size)?;
    ui.face.set_server(server.clone().into());
    ui.face.set_status("Connecting…".into());

    let (events, events_in) = channel::channel();
    let (codes, codes_in) = mpsc::channel();
    event_loop
        .handle()
        .insert_source(events_in, |event, _, station: &mut Station| {
            if let channel::Event::Msg(event) = event {
                station.link(event);
            }
        })
        .map_err(|e| anyhow!("the link in the event loop: {e}"))?;
    link::start(server, events, codes_in);

    let mut station = Station { screen, ui, quit: false, codes, code: None, held: HashSet::new() };
    while !station.quit {
        if station.ui.update() {
            station.screen.changed();
        }
        station.screen.draw(&station.ui.buffer);
        // Asleep until an event, or until Slint has something to do.
        let wake = station.ui.next_wake();
        event_loop.dispatch(wake, &mut station)?;
    }
    Ok(())
}
