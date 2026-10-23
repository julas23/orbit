//! Orbit-Local-Station: a thin client with no desktop of its own. It takes
//! the screen and the keyboard straight from a text console (no
//! compositor), pairs with an orbit-remote-server and shows that session
//! (pending §9). The greet's Remote opens it, through Orbit-Login, in a
//! session of its own on another console; quitting it brings the locked
//! local session back.
//!
//!   orbit-local-station [SERVER]     SERVER as host[:port]
//!
//! The server comes from the command line, else `$ORBIT_STATION_SERVER`,
//! else `server:` in `~/.config/orbit/station.yaml`, else in
//! `/etc/orbit/station.yaml`; without one it is asked on the screen and
//! kept in the user's file. `output:` there picks the monitor (as
//! `HDMI-A-1`), else the first connected one.
//!
//! F2 types another server; Ctrl+Alt+F1…F12 change the console;
//! Ctrl+Alt+Backspace quits.

mod link;
mod screen;
mod ui;

use std::{collections::HashSet, path::PathBuf, sync::mpsc, time::Duration};

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};
use smithay::reexports::calloop::{EventLoop, channel};

use crate::{
    link::{Command, Event},
    screen::{Key, Screen},
    ui::Ui,
};

/// orbit-remote-server's port.
const PORT: u16 = 7311;
const SYSTEM_CONFIG: &str = "/etc/orbit/station.yaml";
const CODE_LENGTH: usize = 6;
const KEYS: &str = "F2  another server     Ctrl+Alt+Backspace  back to this computer";

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
}

fn user_config() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    config.join("orbit/station.yaml")
}

fn read_config(path: &std::path::Path) -> anyhow::Result<Config> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_yaml_ng::from_str(&text).with_context(|| format!("reading {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(err) => Err(anyhow!("reading {}: {err}", path.display())),
    }
}

/// The user's file over the system's, field by field.
fn config() -> anyhow::Result<Config> {
    let system = read_config(std::path::Path::new(SYSTEM_CONFIG))?;
    let user = read_config(&user_config())?;
    Ok(Config { server: user.server.or(system.server), output: user.output.or(system.output) })
}

/// Keeps `server` in the user's file, with what else is there.
fn save_server(server: &str) -> anyhow::Result<()> {
    let path = user_config();
    let mut config = read_config(&path)?;
    config.server = Some(server.to_string());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, serde_yaml_ng::to_string(&config)?).with_context(|| format!("writing {}", path.display()))
}

/// `host` or `host:port`, as the link wants it.
fn with_port(server: &str) -> String {
    if server.contains(':') { server.to_string() } else { format!("{server}:{PORT}") }
}

/// What the keys type into, if anything.
enum Typing {
    Nothing,
    /// The pairing code.
    Code(String),
    /// The server's address; the one before, if any, for Escape.
    Address { text: String, before: Option<String> },
}

pub struct Station {
    screen: Screen,
    ui: Ui,
    /// Ctrl+Alt+Backspace was pressed.
    quit: bool,
    /// To the link, once there is a server.
    commands: Option<mpsc::Sender<Command>>,
    events: channel::Sender<Event>,
    server: Option<String>,
    typing: Typing,
    /// What the link asked for last, to go back to after F2's Escape.
    asked_code: bool,
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
const KEY_LEFTSHIFT: u32 = 42;
const KEY_RIGHTSHIFT: u32 = 54;
const KEY_F2: u32 = 60;

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

/// What a key types in an address (a host name or IP, and a port), by its
/// place on a US keyboard: letters, digits, `.`, `-` and `:`.
fn address_char(code: u32, shift: bool) -> Option<char> {
    const ROWS: [(u32, &str); 3] = [(16, "qwertyuiop"), (30, "asdfghjkl"), (44, "zxcvbnm")];
    for (first, letters) in ROWS {
        if let Some(c) = code.checked_sub(first).and_then(|i| letters.chars().nth(i as usize)) {
            return Some(c);
        }
    }
    match code {
        12 | 74 => Some('-'),
        52 | 83 => Some('.'),
        39 if shift => Some(':'),
        _ => digit(code),
    }
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
        let held = |a, b| self.held.contains(&a) || self.held.contains(&b);
        let (ctrl, alt, shift) = (held(KEY_LEFTCTRL, KEY_RIGHTCTRL), held(KEY_LEFTALT, KEY_RIGHTALT), held(KEY_LEFTSHIFT, KEY_RIGHTSHIFT));
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
        match &mut self.typing {
            Typing::Address { text, before } => match key.code {
                KEY_BACKSPACE => {
                    text.pop();
                }
                KEY_ESC => {
                    if before.is_some() {
                        self.typing = Typing::Nothing;
                        self.ui.face.set_editing(false);
                        if self.asked_code {
                            self.ask_code(false);
                        }
                    }
                    return;
                }
                KEY_ENTER | KEY_KPENTER => {
                    let server = with_port(text.trim());
                    if !text.trim().is_empty() {
                        self.set_server(server);
                    }
                    return;
                }
                other => match address_char(other, shift) {
                    Some(c) if text.len() < 253 => text.push(c),
                    _ => return,
                },
            },
            Typing::Code(code) => match key.code {
                KEY_F2 => return self.edit_server(),
                KEY_BACKSPACE => {
                    code.pop();
                }
                KEY_ESC => code.clear(),
                other => match digit(other) {
                    Some(d) if code.len() < CODE_LENGTH => code.push(d),
                    _ => return,
                },
            },
            Typing::Nothing => {
                if key.code == KEY_F2 {
                    self.edit_server();
                }
                return;
            }
        }
        self.show_typing();
    }

    /// What is being typed, on the screen; the sixth digit sends the code.
    fn show_typing(&mut self) {
        let face = &self.ui.face;
        match &self.typing {
            Typing::Address { text, .. } => face.set_address(text.as_str().into()),
            Typing::Code(code) if code.len() == CODE_LENGTH => {
                let code = code.clone();
                self.typing = Typing::Nothing;
                self.asked_code = false;
                face.set_asking(false);
                face.set_code("".into());
                if let Some(commands) = &self.commands {
                    let _ = commands.send(Command::Code(code));
                }
            }
            Typing::Code(code) => {
                face.set_code(code.as_str().into());
                face.set_problem("".into());
            }
            Typing::Nothing => {}
        }
    }

    /// F2, or no server yet: its address typed on the screen.
    fn edit_server(&mut self) {
        let before = self.server.clone();
        let face = &self.ui.face;
        face.set_asking(false);
        face.set_editing(true);
        face.set_status("Type the server's address (host or host:port), then Enter".into());
        face.set_problem("".into());
        face.set_address(before.as_deref().unwrap_or_default().into());
        self.typing = Typing::Address { text: before.clone().unwrap_or_default(), before };
    }

    /// A server typed: kept, and the link (re)started with it.
    fn set_server(&mut self, server: String) {
        if let Err(err) = save_server(&server) {
            log::warn!("station: {err:#}");
        }
        self.typing = Typing::Nothing;
        self.asked_code = false;
        let face = &self.ui.face;
        face.set_editing(false);
        face.set_server(server.as_str().into());
        face.set_status("Connecting…".into());
        match &self.commands {
            Some(commands) => {
                let _ = commands.send(Command::Server(server.clone()));
            }
            None => {
                let (commands, commands_in) = mpsc::channel();
                link::start(server.clone(), self.events.clone(), commands_in);
                self.commands = Some(commands);
            }
        }
        self.server = Some(server);
    }

    fn ask_code(&mut self, wrong: bool) {
        self.asked_code = true;
        self.typing = Typing::Code(String::new());
        let face = &self.ui.face;
        face.set_asking(true);
        face.set_code("".into());
        face.set_status("Type the code shown on the server's screen".into());
        face.set_problem(if wrong { "That code was not right".into() } else { "".into() });
    }

    fn link(&mut self, event: Event) {
        // The address being typed stays on the screen; the link's news wait.
        if let Typing::Address { .. } = self.typing {
            if let Event::Code { .. } = event {
                self.asked_code = true;
            }
            return;
        }
        let face = &self.ui.face;
        match event {
            Event::Connecting => face.set_status("Connecting…".into()),
            Event::Unreachable(why) => {
                log::warn!("station: {why}");
                face.set_status("Waiting for the server".into());
                face.set_problem(why.into());
            }
            Event::Code { wrong } => self.ask_code(wrong),
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

/// A session opened behind another (Orbit-Login's, from the greet) has no
/// screen until logind shows it: waited for, as orbit-core does.
fn wait_until_shown() {
    let active = || -> zbus::Result<bool> {
        let connection = zbus::blocking::Connection::system()?;
        let session = zbus::blocking::Proxy::new(&connection, "org.freedesktop.login1", "/org/freedesktop/login1/session/auto", "org.freedesktop.login1.Session")?;
        session.get_property::<bool>("Active")
    };
    loop {
        match active() {
            Ok(false) => std::thread::sleep(Duration::from_millis(100)),
            Ok(true) => break,
            Err(err) => {
                log::debug!("logind: {err}");
                break;
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
    let server = std::env::args().nth(1).or_else(|| std::env::var("ORBIT_STATION_SERVER").ok()).or(config.server).map(|s| with_port(&s));

    wait_until_shown();
    let mut event_loop: EventLoop<'static, Station> = EventLoop::try_new()?;
    let screen = Screen::open(&mut event_loop, config.output.as_deref())?;
    let ui = Ui::new(screen.size)?;
    ui.face.set_keys(KEYS.into());

    let (events, events_in) = channel::channel();
    event_loop
        .handle()
        .insert_source(events_in, |event, _, station: &mut Station| {
            if let channel::Event::Msg(event) = event {
                station.link(event);
            }
        })
        .map_err(|e| anyhow!("the link in the event loop: {e}"))?;

    let mut station =
        Station { screen, ui, quit: false, commands: None, events, server: None, typing: Typing::Nothing, asked_code: false, held: HashSet::new() };
    match server {
        Some(server) => {
            station.ui.face.set_server(server.as_str().into());
            station.ui.face.set_status("Connecting…".into());
            let (commands, commands_in) = mpsc::channel();
            link::start(server.clone(), station.events.clone(), commands_in);
            station.commands = Some(commands);
            station.server = Some(server);
        }
        None => station.edit_server(),
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_typed_by_place() {
        let typed: String = [35, 25, 49, 52, 11, 39].iter().map(|&k| address_char(k, k == 39).unwrap()).collect();
        assert_eq!(typed, "hpn.0:");
        assert_eq!(address_char(39, false), None);
        assert_eq!(digit(82), Some('0'));
        assert_eq!(digit(71), Some('7'));
    }

    #[test]
    fn the_port_is_added_when_missing() {
        assert_eq!(with_port("10.0.0.5"), "10.0.0.5:7311");
        assert_eq!(with_port("julas:9000"), "julas:9000");
    }
}
