//! The client's gamepads and joysticks, made again in the session: each
//! one the client describes (`pad`) becomes a uinput device here, with its
//! name, its USB ids (SDL and Steam know a controller by them), its
//! buttons and its axes; its events (`pad-input`) are written to it a
//! report at a time, and it goes with `pad-gone` or with the client.
//!
//! The station reads its devices as they are (evdev); the browser's page
//! sends the Gamepad API's standard pad as an Xbox 360 controller.
//!
//! /dev/uinput is root's alone by default: the package's udev rule gives
//! it to the user at the seat (`uaccess`), as Steam's does.

use std::{
    collections::HashMap,
    fs::File,
    io::Write,
    os::fd::AsRawFd,
};

use anyhow::{Context, bail};
use serde_json::Value;

/// More would be a client gone wrong.
const MOST: usize = 8;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const SYN_REPORT: u16 = 0;
/// The last key code and the last axis (linux/input-event-codes.h).
const KEY_MAX: u64 = 0x2ff;
const ABS_MAX: u64 = 0x3f;

/// linux/ioctl.h, as on x86 and ARM.
const fn ioc(dir: u64, kind: u8, nr: u8, size: usize) -> u64 {
    (dir << 30) | ((size as u64) << 16) | ((kind as u64) << 8) | nr as u64
}
const WRITE: u64 = 1;
const UI_DEV_CREATE: u64 = ioc(0, b'U', 1, 0);
const UI_DEV_DESTROY: u64 = ioc(0, b'U', 2, 0);
const UI_DEV_SETUP: u64 = ioc(WRITE, b'U', 3, size_of::<Setup>());
const UI_ABS_SETUP: u64 = ioc(WRITE, b'U', 4, size_of::<AbsSetup>());
const UI_SET_EVBIT: u64 = ioc(WRITE, b'U', 100, size_of::<i32>());
const UI_SET_KEYBIT: u64 = ioc(WRITE, b'U', 101, size_of::<i32>());
const UI_SET_ABSBIT: u64 = ioc(WRITE, b'U', 103, size_of::<i32>());

/// struct uinput_setup.
#[repr(C)]
struct Setup {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
    name: [u8; 80],
    ff_effects_max: u32,
}

/// struct uinput_abs_setup: an axis and its struct input_absinfo.
#[repr(C)]
struct AbsSetup {
    code: u16,
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

/// One device made for the client.
struct Pad {
    file: File,
}

impl Pad {
    /// A device as `pad` describes it.
    fn create(pad: &Value) -> anyhow::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/uinput")
            .context("/dev/uinput (the uinput module loaded, and orbit-remote-server's udev rule giving it to the seat's user)")?;
        let fd = file.as_raw_fd();
        let control = |request: u64, arg: libc::c_ulong| -> anyhow::Result<()> {
            // Safety: requests that take an int, on a file of ours.
            if unsafe { libc::ioctl(fd, request as _, arg) } < 0 {
                bail!("uinput: {}", std::io::Error::last_os_error());
            }
            Ok(())
        };
        let codes = |name: &str, max: u64| -> Vec<u16> {
            pad[name].as_array().into_iter().flatten().filter_map(Value::as_u64).filter(|&c| c <= max).map(|c| c as u16).collect()
        };
        let keys = codes("keys", KEY_MAX);
        if !keys.is_empty() {
            control(UI_SET_EVBIT, EV_KEY.into())?;
            for key in keys {
                control(UI_SET_KEYBIT, key.into())?;
            }
        }
        let axes: Vec<&Value> = pad["axes"].as_array().into_iter().flatten().filter(|a| a["code"].as_u64().is_some_and(|c| c <= ABS_MAX)).collect();
        if !axes.is_empty() {
            control(UI_SET_EVBIT, EV_ABS.into())?;
        }
        for axis in axes {
            let number = |name: &str| axis[name].as_i64().and_then(|v| i32::try_from(v).ok()).unwrap_or(0);
            let setup = AbsSetup {
                code: number("code") as u16,
                value: number("value"),
                minimum: number("min"),
                maximum: number("max"),
                fuzz: number("fuzz"),
                flat: number("flat"),
                resolution: number("resolution"),
            };
            control(UI_SET_ABSBIT, setup.code.into())?;
            // Safety: the struct the request reads, alive through the call.
            if unsafe { libc::ioctl(fd, UI_ABS_SETUP as _, &setup) } < 0 {
                bail!("uinput, axis {}: {}", setup.code, std::io::Error::last_os_error());
            }
        }
        let id = |name: &str| pad[name].as_u64().and_then(|v| u16::try_from(v).ok()).unwrap_or(0);
        let mut setup = Setup { bustype: id("bus"), vendor: id("vendor"), product: id("product"), version: id("version"), name: [0; 80], ff_effects_max: 0 };
        let name = pad["name"].as_str().unwrap_or("Orbit Remote pad");
        // Up to the last whole character that fits, the NUL after it.
        let end = (0..=name.len().min(79)).rev().find(|&i| name.is_char_boundary(i)).unwrap_or(0);
        setup.name[..end].copy_from_slice(&name.as_bytes()[..end]);
        // Safety: as above.
        if unsafe { libc::ioctl(fd, UI_DEV_SETUP as _, &setup) } < 0 {
            bail!("uinput, the device: {}", std::io::Error::last_os_error());
        }
        control(UI_DEV_CREATE, 0)?;
        log::info!("remote: the client's {name:?} is a device here ({:04x}:{:04x})", setup.vendor, setup.product);
        Ok(Self { file })
    }

    /// One report: its events, then SYN_REPORT.
    fn write(&mut self, events: &[Value]) -> std::io::Result<()> {
        let mut bytes = Vec::with_capacity((events.len() + 1) * size_of::<libc::input_event>());
        let mut event = |kind: u16, code: u16, value: i32| {
            let event = libc::input_event { time: libc::timeval { tv_sec: 0, tv_usec: 0 }, type_: kind, code, value };
            // Safety: a plain struct, read as its bytes.
            bytes.extend_from_slice(unsafe {
                std::slice::from_raw_parts((&event as *const libc::input_event).cast::<u8>(), size_of::<libc::input_event>())
            });
        };
        for triple in events {
            let part = |i: usize| triple[i].as_i64();
            let (Some(kind), Some(code), Some(value)) = (part(0), part(1), part(2)) else { continue };
            // Only what a pad has: no keyboard keys or relative moves smuggled in.
            let fits = match kind as u16 {
                EV_KEY => (0..=KEY_MAX as i64).contains(&code),
                EV_ABS => (0..=ABS_MAX as i64).contains(&code),
                _ => false,
            };
            if fits && let Ok(value) = i32::try_from(value) {
                event(kind as u16, code as u16, value);
            }
        }
        event(EV_SYN, SYN_REPORT, 0);
        self.file.write_all(&bytes)
    }
}

impl Drop for Pad {
    fn drop(&mut self) {
        // Safety: a request with no argument, on a file of ours.
        unsafe { libc::ioctl(self.file.as_raw_fd(), UI_DEV_DESTROY as _) };
    }
}

/// The client's pads, by the client's numbers for them.
#[derive(Default)]
pub struct Pads {
    pads: HashMap<u64, Pad>,
    /// uinput failed once: said once, not for every report.
    failed: bool,
}

impl Pads {
    /// One message from the client (`pad`, `pad-input`, `pad-gone`);
    /// false if it was not one of those. A pad that cannot be made is
    /// left out: the rest of the session goes on.
    pub fn handle(&mut self, message: &Value) -> bool {
        let id = message["id"].as_u64().unwrap_or(0);
        match message["type"].as_str() {
            Some("pad") => {
                self.pads.remove(&id);
                if self.pads.len() >= MOST {
                    log::warn!("remote: more than {MOST} pads from the client; this one left out");
                    return true;
                }
                match Pad::create(message) {
                    Ok(pad) => {
                        self.pads.insert(id, pad);
                    }
                    Err(err) if !self.failed => {
                        self.failed = true;
                        log::warn!("remote: the client's pads are left out: {err:#}");
                    }
                    Err(_) => {}
                }
            }
            Some("pad-input") => {
                let events = message["events"].as_array().map(Vec::as_slice).unwrap_or_default();
                if let Some(pad) = self.pads.get_mut(&id)
                    && let Err(err) = pad.write(events)
                {
                    log::warn!("remote: pad {id}: {err}");
                    self.pads.remove(&id);
                }
            }
            Some("pad-gone") => {
                if self.pads.remove(&id).is_some() {
                    log::info!("remote: the client's pad {id} is gone");
                }
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_requests_are_the_kernels() {
        // As linux/uinput.h makes them on x86-64 and arm64.
        assert_eq!(UI_DEV_CREATE, 0x5501);
        assert_eq!(UI_DEV_SETUP, 0x405c_5503);
        assert_eq!(UI_ABS_SETUP, 0x401c_5504);
        assert_eq!(UI_SET_KEYBIT, 0x4004_5565);
    }
}
