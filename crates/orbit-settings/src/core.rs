//! What orbit-settings reads from and writes to `core.yaml`, and the core
//! told over its IPC.
//!
//! Only the keys a page changes are rewritten (`ConfigDir::save_app_keys`);
//! the rest of the file, comments included, stays. A monitor's settings go
//! under `HOST/CONNECTOR`, as the file is synced between machines.

use serde::Deserialize;
use serde_yaml_ng::{Mapping, Value};

use orbit_common::ConfigDir;

/// The `wallpaper:` section.
#[derive(Debug, Clone, PartialEq)]
pub struct Wallpaper {
    pub image: String,
    pub mode: String,
    pub color: String,
    /// `none`, `aurora`, `ink`, … (the core's `effects.rs`).
    pub effect: String,
    /// The effect's colours; empty: its own.
    pub colors: Vec<String>,
    pub speed: f64,
    /// A video (or animated GIF) over every monitor; empty: none.
    pub video: String,
    /// The effect follows what the speakers play.
    pub sound: bool,
    /// Seconds idle before the effect shows alone; 0: never.
    pub screensaver: u64,
}

/// The core's own defaults (`wallpaper.rs`).
impl Default for Wallpaper {
    fn default() -> Self {
        Self {
            image: String::new(),
            mode: "fill".into(),
            color: "#1b1f27".into(),
            effect: "none".into(),
            colors: Vec::new(),
            speed: 1.0,
            video: String::new(),
            sound: false,
            screensaver: 300,
        }
    }
}

/// This machine's name, as the core matches `HOST/CONNECTOR`.
pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

/// The whole file, empty when missing or unreadable.
fn read(dir: &ConfigDir) -> Mapping {
    std::fs::read_to_string(dir.path().join("core.yaml"))
        .ok()
        .and_then(|text| serde_yaml_ng::from_str::<Value>(&text).ok())
        .and_then(|v| v.as_mapping().cloned())
        .unwrap_or_default()
}

fn text(map: &Mapping, key: &str) -> Option<String> {
    map.get(key).and_then(Value::as_str).map(str::to_owned)
}

pub fn wallpaper(dir: &ConfigDir) -> Wallpaper {
    let file = read(dir);
    let default = Wallpaper::default();
    let Some(section) = file.get("wallpaper").and_then(Value::as_mapping) else {
        return default;
    };
    Wallpaper {
        image: text(section, "image").unwrap_or(default.image),
        mode: text(section, "mode").unwrap_or(default.mode),
        color: text(section, "color").unwrap_or(default.color),
        effect: text(section, "effect").unwrap_or(default.effect),
        colors: section
            .get("colors")
            .and_then(Value::as_sequence)
            .map(|list| list.iter().filter_map(|c| c.as_str().map(str::to_owned)).collect())
            .unwrap_or_default(),
        speed: section.get("speed").and_then(Value::as_f64).unwrap_or(default.speed),
        video: text(section, "video").unwrap_or(default.video),
        sound: section.get("sound").and_then(Value::as_bool).unwrap_or(default.sound),
        screensaver: section
            .get("screensaver")
            .and_then(Value::as_u64)
            .unwrap_or(default.screensaver),
    }
}

/// The core's `idle_off` when the file does not say (`config.rs`).
const IDLE_OFF_DEFAULT: u64 = 900;

/// Seconds idle before the monitors switch off on this machine; 0: never.
/// `idle_off` is one number for every machine, or `{ default: …, HOST: … }`,
/// read as the core reads it.
pub fn idle_off(dir: &ConfigDir) -> u64 {
    match read(dir).get("idle_off") {
        None => IDLE_OFF_DEFAULT,
        Some(Value::Mapping(map)) => map
            .get(hostname().as_str())
            .or_else(|| map.get("default"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        Some(value) => value.as_u64().unwrap_or(IDLE_OFF_DEFAULT),
    }
}

/// Writes this machine's `idle_off` as `HOST: seconds` (the file is synced):
/// a single number becomes the other machines' `default`.
pub fn save_idle_off(dir: &ConfigDir, seconds: u64) -> anyhow::Result<()> {
    let mut map = match read(dir).get("idle_off") {
        Some(Value::Mapping(map)) => map.clone(),
        other => {
            let mut map = Mapping::new();
            let all = other.and_then(Value::as_u64).unwrap_or(IDLE_OFF_DEFAULT);
            map.insert("default".into(), all.into());
            map
        }
    };
    map.insert(hostname().into(), seconds.into());
    let mut block = Mapping::new();
    block.insert("idle_off".into(), Value::Mapping(map));
    dir.save_app_keys("core", &["idle_off"], &serde_yaml_ng::to_string(&block)?)
}

/// The picture of monitor `name` alone, when it has one of its own.
pub fn output_wallpaper(dir: &ConfigDir, name: &str) -> Option<String> {
    output_entry(&read(dir), name)
        .get("wallpaper")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Writes `wallpaper:`. With `everywhere`, the monitors' own pictures go,
/// so the one picture shows on all of them.
pub fn save_wallpaper(dir: &ConfigDir, wallpaper: &Wallpaper, everywhere: bool) -> anyhow::Result<()> {
    // What the file has besides (`fps`) stays.
    let mut section = read(dir)
        .get("wallpaper")
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default();
    section.insert("image".into(), wallpaper.image.clone().into());
    section.insert("mode".into(), wallpaper.mode.clone().into());
    section.insert("color".into(), wallpaper.color.clone().into());
    section.insert("effect".into(), wallpaper.effect.clone().into());
    if wallpaper.colors.is_empty() {
        section.remove("colors");
    } else {
        let list = wallpaper.colors.iter().cloned().map(Value::from).collect();
        section.insert("colors".into(), Value::Sequence(list));
    }
    section.insert("speed".into(), wallpaper.speed.into());
    section.insert("sound".into(), wallpaper.sound.into());
    section.insert("screensaver".into(), wallpaper.screensaver.into());
    if wallpaper.video.is_empty() {
        section.remove("video");
    } else {
        section.insert("video".into(), wallpaper.video.clone().into());
    }
    let mut block = Mapping::new();
    block.insert("wallpaper".into(), Value::Mapping(section));
    dir.save_app_keys("core", &["wallpaper"], &serde_yaml_ng::to_string(&block)?)?;
    if everywhere {
        let file = read(dir);
        let has_own = file
            .get("outputs")
            .and_then(Value::as_mapping)
            .is_some_and(|outputs| outputs.values().any(|o| o.get("wallpaper").is_some()));
        if has_own {
            let mut outputs = file
                .get("outputs")
                .and_then(Value::as_mapping)
                .cloned()
                .unwrap_or_default();
            for (_, entry) in outputs.iter_mut() {
                if let Some(entry) = entry.as_mapping_mut() {
                    entry.remove("wallpaper");
                }
            }
            outputs.retain(|_, entry| entry.as_mapping().is_none_or(|e| !e.is_empty()));
            save_outputs(dir, outputs)?;
        }
    }
    Ok(())
}

/// Monitor `name`'s entry on this machine: its `HOST/NAME`, else the plain
/// `NAME` (which the host's would replace whole, so it starts from it).
fn output_entry(file: &Mapping, name: &str) -> Mapping {
    let outputs = file.get("outputs").and_then(Value::as_mapping);
    let entry = |key: &str| outputs.and_then(|o| o.get(key)).and_then(Value::as_mapping).cloned();
    entry(&format!("{}/{name}", hostname()))
        .or_else(|| entry(name))
        .unwrap_or_default()
}

/// Takes monitor `name`'s own picture away, if it has one.
pub fn drop_output_wallpaper(dir: &ConfigDir, name: &str) -> anyhow::Result<()> {
    if output_wallpaper(dir, name).is_none() {
        return Ok(());
    }
    save_output(dir, name, |e| {
        e.remove("wallpaper");
    })
}

/// Changes monitor `name`'s entry for this machine with `change`.
pub fn save_output(dir: &ConfigDir, name: &str, change: impl FnOnce(&mut Mapping)) -> anyhow::Result<()> {
    let file = read(dir);
    let mut entry = output_entry(&file, name);
    change(&mut entry);
    let mut outputs = file
        .get("outputs")
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default();
    let key = format!("{}/{name}", hostname());
    if entry.is_empty() {
        outputs.remove(key.as_str());
    } else {
        outputs.insert(key.into(), Value::Mapping(entry));
    }
    save_outputs(dir, outputs)
}

fn save_outputs(dir: &ConfigDir, outputs: Mapping) -> anyhow::Result<()> {
    let block = if outputs.is_empty() {
        "outputs: {}\n".to_owned()
    } else {
        let mut block = Mapping::new();
        block.insert("outputs".into(), Value::Mapping(outputs));
        serde_yaml_ng::to_string(&block)?
    };
    dir.save_app_keys("core", &["outputs"], &block)
}

/// A monitor, as the core's `get_outputs` tells it.
#[derive(Debug, Clone, Deserialize)]
pub struct Output {
    pub name: String,
    #[serde(default)]
    pub make: String,
    #[serde(default)]
    pub model: String,
    #[serde(default = "one")]
    pub scale: f64,
    #[serde(default)]
    pub transform: String,
    #[serde(default)]
    pub modes: Vec<Mode>,
    pub current_mode: Option<Mode>,
}

fn one() -> f64 {
    1.0
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Mode {
    pub width: i32,
    pub height: i32,
    /// mHz.
    pub refresh: i32,
    #[serde(default)]
    pub preferred: bool,
}

impl Mode {
    /// `1920x1080@59.94`, as `output … mode` and core.yaml take it.
    pub fn spec(&self) -> String {
        format!("{}x{}@{}", self.width, self.height, hz(self.refresh))
    }
}

/// mHz as Hz, without needless decimals: 60000 → "60", 59940 → "59.94".
pub fn hz(refresh: i32) -> String {
    let text = format!("{:.2}", refresh as f64 / 1000.0);
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

pub fn outputs() -> anyhow::Result<Vec<Output>> {
    Ok(orbit_ipc::Connection::open()?.query(orbit_ipc::GET_OUTPUTS)?)
}

/// Runs a core command; its error when it failed.
pub fn command(command: &str) -> anyhow::Result<()> {
    let body = orbit_ipc::Connection::open()?.request(orbit_ipc::RUN_COMMAND, command)?;
    let results: Vec<serde_json::Value> = serde_json::from_slice(&body)?;
    match results.iter().find(|r| r["success"] == false) {
        Some(failed) => anyhow::bail!("{}", failed["error"].as_str().unwrap_or("failed")),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_read_short() {
        assert_eq!(hz(60000), "60");
        assert_eq!(hz(59940), "59.94");
        assert_eq!(hz(143_980), "143.98");
        let mode = Mode {
            width: 1920,
            height: 1080,
            refresh: 60000,
            preferred: false,
        };
        assert_eq!(mode.spec(), "1920x1080@60");
    }

    #[test]
    fn writes_only_its_keys() {
        let dir = std::env::temp_dir().join(format!("orbit-settings-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("core.yaml"),
            "# mine\nkeyboard: { layout: us }\noutputs:\n  DP-1: { scale: 2, wallpaper: ~/a.png }\nwallpaper:\n  image: ~/x.jpg\n",
        )
        .unwrap();
        let config = ConfigDir::new(&dir);
        assert_eq!(wallpaper(&config).image, "~/x.jpg");
        save_output(&config, "DP-1", |e| {
            e.insert("mode".into(), "1920x1080@60".into());
        })
        .unwrap();
        let file = read(&config);
        let host = &file["outputs"][format!("{}/DP-1", hostname()).as_str()];
        // Started from the plain entry: the scale and picture are kept.
        assert_eq!(host["scale"].as_f64(), Some(2.0));
        assert_eq!(host["mode"].as_str(), Some("1920x1080@60"));
        let mut paper = wallpaper(&config);
        paper.image = "~/y.jpg".into();
        save_wallpaper(&config, &paper, true).unwrap();
        let text = std::fs::read_to_string(dir.join("core.yaml")).unwrap();
        assert!(text.starts_with("# mine\nkeyboard: { layout: us }\n"));
        assert!(!text.contains("a.png"));
        assert_eq!(wallpaper(&config).image, "~/y.jpg");
        assert!(output_wallpaper(&config, "DP-1").is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn idle_off_per_machine() {
        let dir = std::env::temp_dir().join(format!("orbit-settings-idle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = ConfigDir::new(&dir);
        let host = hostname();
        // Not in the file: the core's default.
        std::fs::write(dir.join("core.yaml"), "# mine\nidle_lock: 600\n").unwrap();
        assert_eq!(idle_off(&config), 900);
        // One number for all: it stays for the others, this machine gets its own.
        std::fs::write(dir.join("core.yaml"), "# mine\nidle_off: 300\n").unwrap();
        assert_eq!(idle_off(&config), 300);
        save_idle_off(&config, 0).unwrap();
        assert_eq!(idle_off(&config), 0);
        let file = read(&config);
        assert_eq!(file["idle_off"]["default"].as_u64(), Some(300));
        assert_eq!(file["idle_off"][host.as_str()].as_u64(), Some(0));
        // Per machine already: the others' entries stay.
        std::fs::write(dir.join("core.yaml"), "idle_off: { default: 900, otherhost: 60 }\n").unwrap();
        assert_eq!(idle_off(&config), 900);
        save_idle_off(&config, 1800).unwrap();
        assert_eq!(idle_off(&config), 1800);
        assert_eq!(read(&config)["idle_off"]["otherhost"].as_u64(), Some(60));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
