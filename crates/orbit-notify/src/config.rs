//! `~/.config/orbit/notify.yaml`. Every option has a default; the file may
//! be missing.

use orbit_common::config::MonitorSpec;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// The monitor for the bubbles and the centre; without it, the panel's.
    pub monitor: Option<MonitorSpec>,
    /// Width of a bubble, px.
    pub width: i32,
    /// Width of the centre, px.
    pub center_width: i32,
    /// Distance from the screen's edges, px.
    pub margin: i32,
    /// Bubbles on screen at once; the rest wait their turn.
    pub max_popups: usize,
    /// How long a bubble stays, by urgency, in seconds (0: until closed),
    /// when the program that sent it does not say.
    pub timeout: Timeouts,
    /// Notifications kept in the centre.
    pub history_limit: usize,
    /// Programs whose notifications are only bubbles, never kept (volume
    /// and brightness changes, say).
    pub no_history: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            monitor: None,
            width: 360,
            center_width: 400,
            margin: 10,
            max_popups: 5,
            timeout: Timeouts::default(),
            history_limit: 200,
            no_history: vec!["volume".into(), "microfone".into()],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Timeouts {
    pub low: u32,
    pub normal: u32,
    pub critical: u32,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self { low: 5, normal: 8, critical: 0 }
    }
}

impl Config {
    /// How long a bubble of `urgency` stays, in ms; 0 is "until closed".
    pub fn timeout_ms(&self, urgency: u8) -> u32 {
        let secs = match urgency {
            0 => self.timeout.low,
            2 => self.timeout.critical,
            _ => self.timeout.normal,
        };
        secs.saturating_mul(1000)
    }
}
