//! Brightness module, after the brightness card of the eww panel.
//!
//! Laptop: the first device in /sys/class/backlight. Its `actual_brightness`
//! is watched with poll(): the kernel wakes it on every change (sysfs_notify),
//! whoever made it — keys, brightnessctl, this slider — so nothing is polled
//! on a timer. Brightness is set through logind (`SetBrightness`), which any
//! active session may do without being in the `video` group.
//!
//! Desktop without a backlight: the external monitor over DDC/CI, through
//! `ddcutil`. DDC is slow (~0.4 s per call) and has no change events, so the
//! level is read once at start and every set runs off the GTK thread; while
//! one call runs, further slider moves only update the wanted value, and the
//! next call applies the latest one — no queue of stale calls.
//!
//! Without either, the module stays hidden.

use std::{
    cell::{Cell, RefCell},
    fs::File,
    io::{Read, Seek},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};
use serde::Deserialize;

use crate::control::{Actions, Control};

/// `modules/brightness.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrightnessConfig {
    /// Lowest level the slider and the wheel can set, so the screen never
    /// goes fully dark by accident.
    pub min: u32,
    /// Percentage per wheel step.
    pub step: u32,
    /// `ddcutil --display` number, for monitors over DDC/CI.
    pub ddc_display: u32,
}

impl Default for BrightnessConfig {
    fn default() -> Self {
        Self {
            min: 5,
            step: 5,
            ddc_display: 1,
        }
    }
}

const GLYPH: &str = "\u{f00df}"; // 󰃟

enum Device {
    Backlight {
        name: String,
        dir: PathBuf,
        max: u32,
    },
    Ddc,
}

struct Module {
    config: BrightnessConfig,
    device: Device,
    control: RefCell<Option<Control>>,
    percent: Cell<u32>,
    /// DDC: the level wanted, and whether a ddcutil call is running.
    wanted: Cell<Option<u32>>,
    busy: Cell<bool>,
}

pub fn new(config: &BrightnessConfig, vertical: bool) -> gtk::Widget {
    let Some(device) = detect() else {
        log::info!("brightness: no backlight and no ddcutil; module hidden");
        let empty = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        empty.set_visible(false);
        return empty.upcast();
    };
    let module = Rc::new(Module {
        config: config.clone(),
        device,
        control: RefCell::new(None),
        percent: Cell::new(0),
        wanted: Cell::new(None),
        busy: Cell::new(false),
    });

    let w = Rc::downgrade(&module);
    let set = {
        let w = w.clone();
        move |p: u32| {
            if let Some(m) = w.upgrade() {
                m.set(p);
            }
        }
    };
    let step = move |s: i32| {
        if let Some(m) = w.upgrade() {
            let target = m.percent.get() as i32 + s * m.config.step as i32;
            m.set(target.clamp(0, 100) as u32);
        }
    };
    let control = Control::new(
        "brightness",
        vertical,
        Actions {
            set: Box::new(set),
            step: Box::new(step),
            click: None,
            right_click: None,
        },
    );
    let root = control.root.clone();
    *module.control.borrow_mut() = Some(control);

    match &module.device {
        Device::Backlight { dir, .. } => watch_backlight(&module, dir),
        Device::Ddc => module.read_ddc(),
    }
    root.connect_destroy(move |_| {
        let _ = &module;
    });
    root
}

fn detect() -> Option<Device> {
    let backlight = std::fs::read_dir("/sys/class/backlight")
        .ok()
        .and_then(|mut entries| entries.next())
        .and_then(Result::ok);
    if let Some(entry) = backlight {
        let dir = entry.path();
        let max = read_number(&dir.join("max_brightness"))?.max(1);
        return Some(Device::Backlight {
            name: entry.file_name().to_string_lossy().into_owned(),
            dir,
            max,
        });
    }
    glib::find_program_in_path("ddcutil").map(|_| Device::Ddc)
}

fn read_number(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Shows the backlight level now and after every change the kernel reports.
fn watch_backlight(module: &Rc<Module>, dir: &Path) {
    let Ok(mut file) = File::open(dir.join("actual_brightness")) else {
        return;
    };
    let read = move |file: &mut File| -> Option<u32> {
        let mut text = String::new();
        file.rewind().ok()?;
        file.read_to_string(&mut text).ok()?; // reading re-arms the notification
        text.trim().parse().ok()
    };
    if let Some(value) = read(&mut file) {
        module.show_raw(value);
    }
    let fd = file.as_raw_fd();
    let weak = Rc::downgrade(module);
    glib_unix::unix_fd_add_local(
        fd,
        glib::IOCondition::PRI | glib::IOCondition::ERR,
        move |_, _| {
            let Some(module) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if let Some(value) = read(&mut file) {
                module.show_raw(value);
            }
            glib::ControlFlow::Continue
        },
    );
}

impl Module {
    fn show(&self, percent: u32) {
        self.percent.set(percent);
        if let Some(control) = &*self.control.borrow() {
            control.show(percent, GLYPH, &format!("brilho: {percent}%"));
        }
    }

    fn show_raw(&self, value: u32) {
        if let Device::Backlight { max, .. } = &self.device {
            self.show((value * 100 + max / 2) / max);
        }
    }

    fn set(self: &Rc<Self>, percent: u32) {
        let percent = percent.clamp(self.config.min.min(100), 100);
        match &self.device {
            Device::Backlight { name, max, .. } => {
                let value = (percent * max + 50) / 100;
                set_backlight(name, value.max(1));
            }
            Device::Ddc => {
                self.show(percent);
                self.wanted.set(Some(percent));
                self.apply_ddc();
            }
        }
    }

    fn read_ddc(self: &Rc<Self>) {
        let display = self.config.ddc_display.to_string();
        let argv = ["ddcutil", "--display", &display, "getvcp", "10", "--brief"];
        let Some(process) = spawn(&argv, gio::SubprocessFlags::STDOUT_PIPE) else {
            return;
        };
        let weak = Rc::downgrade(self);
        process.communicate_utf8_async(None, gio::Cancellable::NONE, move |result| {
            // "VCP 10 C 60 100": the fourth field is the current value.
            let value = result
                .ok()
                .and_then(|(out, _)| out)
                .and_then(|out| out.split_whitespace().nth(3).and_then(|v| v.parse().ok()));
            if let (Some(module), Some(value)) = (weak.upgrade(), value) {
                module.show(value);
            }
        });
    }

    /// Starts a ddcutil call for the latest wanted value, unless one runs.
    fn apply_ddc(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        let Some(percent) = self.wanted.take() else {
            return;
        };
        let display = self.config.ddc_display.to_string();
        let value = percent.to_string();
        let argv = [
            "ddcutil",
            "--display",
            &display,
            "--noverify",
            "setvcp",
            "10",
            &value,
        ];
        let Some(process) = spawn(&argv, gio::SubprocessFlags::NONE) else {
            return;
        };
        self.busy.set(true);
        let weak = Rc::downgrade(self);
        process.wait_async(gio::Cancellable::NONE, move |_| {
            if let Some(module) = weak.upgrade() {
                module.busy.set(false);
                module.apply_ddc();
            }
        });
    }
}

fn spawn(argv: &[&str], flags: gio::SubprocessFlags) -> Option<gio::Subprocess> {
    let argv: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
    gio::Subprocess::newv(&argv, flags | gio::SubprocessFlags::STDERR_SILENCE)
        .inspect_err(|err| log::error!("brightness: {err}"))
        .ok()
}

/// Sets the backlight through logind; falls back to writing the sysfs file
/// (which needs the `video` group) if logind refuses.
fn set_backlight(name: &str, value: u32) {
    let bus = match gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE) {
        Ok(bus) => bus,
        Err(err) => {
            log::warn!("brightness: no system bus ({err})");
            write_backlight(name, value);
            return;
        }
    };
    let name = name.to_owned();
    bus.call(
        Some("org.freedesktop.login1"),
        "/org/freedesktop/login1/session/auto",
        "org.freedesktop.login1.Session",
        "SetBrightness",
        Some(&("backlight", name.as_str(), value).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        -1,
        gio::Cancellable::NONE,
        move |result| {
            if let Err(err) = result {
                log::warn!("brightness: logind refused ({err}); writing sysfs");
                write_backlight(&name, value);
            }
        },
    );
}

fn write_backlight(name: &str, value: u32) {
    let path = Path::new("/sys/class/backlight")
        .join(name)
        .join("brightness");
    if let Err(err) = std::fs::write(&path, value.to_string()) {
        log::error!("brightness: writing {}: {err}", path.display());
    }
}
