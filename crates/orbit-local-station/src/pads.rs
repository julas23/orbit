//! The station's gamepads and joysticks, for the server to make again in
//! the session (orbit-remote-server's `pads`). libinput leaves them out, so
//! they are read here straight from evdev: every `/dev/input/event*` with
//! a joystick's or a gamepad's buttons and some axes, as udev's
//! `ID_INPUT_JOYSTICK` decides. logind gives them to the user at the seat
//! (`uaccess`), as it does the keyboard.
//!
//! Each one is described to the server once (`pad`: its name, its USB
//! ids, its buttons, its axes with their ranges), then its events go a
//! report at a time (`pad-input`); unplugged, `pad-gone`. Plugged in
//! during the session: found by looking again every two seconds.
//!
//! Only while a session is paired: the link makes them and drops them.

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{ErrorKind, Read},
    os::{
        fd::{AsFd, AsRawFd, BorrowedFd},
        unix::fs::OpenOptionsExt,
    },
    path::PathBuf,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const SYN_REPORT: u16 = 0;
const SYN_DROPPED: u16 = 3;
const KEY_MAX: usize = 0x2ff;
const ABS_MAX: usize = 0x3f;
/// BTN_JOYSTICK to BTN_THUMBR: the buttons that make a pad.
const PAD_BUTTONS: std::ops::Range<usize> = 0x120..0x13f;
/// How often unplugged and plugged devices are looked for.
const RESCAN: Duration = Duration::from_secs(2);

/// linux/ioctl.h, as on x86 and ARM.
const fn ioc(dir: u64, nr: u8, size: usize) -> u64 {
    (dir << 30) | ((size as u64) << 16) | ((b'E' as u64) << 8) | nr as u64
}
const READ: u64 = 2;
const EVIOCGID: u64 = ioc(READ, 0x02, 8);
const fn eviocgname(len: usize) -> u64 {
    ioc(READ, 0x06, len)
}
const fn eviocgkey(len: usize) -> u64 {
    ioc(READ, 0x18, len)
}
const fn eviocgbit(kind: u16, len: usize) -> u64 {
    ioc(READ, 0x20 + kind as u8, len)
}
const fn eviocgabs(axis: usize) -> u64 {
    ioc(READ, 0x40 + axis as u8, size_of::<[i32; 6]>())
}

/// An ioctl that fills `out`; false if it failed.
fn get<T>(file: &File, request: u64, out: &mut T) -> bool {
    // Safety: `out` is as large as the request says, alive through the call.
    unsafe { libc::ioctl(file.as_raw_fd(), request as _, out as *mut T) >= 0 }
}

/// The bits of `kind` (or of the event types, for 0) the device has.
fn bits<const N: usize>(file: &File, request: u64) -> [u8; N] {
    let mut bits = [0u8; N];
    get(file, request, &mut bits);
    bits
}

fn has(bits: &[u8], bit: usize) -> bool {
    bits.get(bit / 8).is_some_and(|byte| byte & (1 << (bit % 8)) != 0)
}

/// One pad the server knows of.
struct Pad {
    file: File,
    id: u64,
    keys: Vec<usize>,
    axes: Vec<usize>,
    /// The events since the last report.
    report: Vec<Value>,
    /// Some were dropped: what is read until the next report is skipped,
    /// and the whole state sent instead.
    dropped: bool,
}

impl Pad {
    /// The device at `path`, if it is a pad: its description for the server too.
    fn open(path: &PathBuf, id: u64) -> std::io::Result<Option<(Self, Value)>> {
        let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path)?;
        let types: [u8; 4] = bits(&file, eviocgbit(0, 4));
        let key_bits: [u8; KEY_MAX / 8 + 1] = bits(&file, eviocgbit(EV_KEY, KEY_MAX / 8 + 1));
        let abs_bits: [u8; ABS_MAX / 8 + 1] = bits(&file, eviocgbit(EV_ABS, ABS_MAX / 8 + 1));
        let keys: Vec<usize> = (0..=KEY_MAX).filter(|&k| has(&key_bits, k)).collect();
        let axes: Vec<usize> = (0..=ABS_MAX).filter(|&a| has(&abs_bits, a)).collect();
        if !has(&types, EV_KEY.into()) || !has(&types, EV_ABS.into()) || axes.is_empty() || !keys.iter().any(|k| PAD_BUTTONS.contains(k)) {
            return Ok(None);
        }
        let mut name = [0u8; 128];
        get(&file, eviocgname(name.len()), &mut name);
        let name = String::from_utf8_lossy(name.split(|&b| b == 0).next().unwrap_or_default()).into_owned();
        // bustype, vendor, product, version.
        let mut ids = [0u16; 4];
        get(&file, EVIOCGID, &mut ids);
        let axes_described: Vec<Value> = axes
            .iter()
            .map(|&axis| {
                // value, minimum, maximum, fuzz, flat, resolution.
                let mut info = [0i32; 6];
                get(&file, eviocgabs(axis), &mut info);
                json!({ "code": axis, "value": info[0], "min": info[1], "max": info[2], "fuzz": info[3], "flat": info[4], "resolution": info[5] })
            })
            .collect();
        let description = json!({
            "type": "pad", "id": id, "name": name,
            "bus": ids[0], "vendor": ids[1], "product": ids[2], "version": ids[3],
            "keys": keys, "axes": axes_described,
        });
        log::info!("station: {name:?} ({:04x}:{:04x}) at {} goes to the server", ids[1], ids[2], path.display());
        Ok(Some((Self { file, id, keys, axes, report: Vec::new(), dropped: false }, description)))
    }

    /// Every button and axis as it is now: after events were dropped.
    fn state(&self) -> Vec<Value> {
        let held: [u8; KEY_MAX / 8 + 1] = bits(&self.file, eviocgkey(KEY_MAX / 8 + 1));
        let mut events: Vec<Value> = self.keys.iter().map(|&k| json!([EV_KEY, k, u8::from(has(&held, k))])).collect();
        for &axis in &self.axes {
            let mut info = [0i32; 6];
            if get(&self.file, eviocgabs(axis), &mut info) {
                events.push(json!([EV_ABS, axis, info[0]]));
            }
        }
        events
    }

    /// What is waiting, as whole reports for the server; Err when the
    /// device is gone.
    fn read(&mut self, out: &mut Vec<Value>) -> std::io::Result<()> {
        const SIZE: usize = size_of::<libc::input_event>();
        let mut buffer = [0u8; SIZE * 64];
        loop {
            let n = match self.file.read(&mut buffer) {
                Ok(0) => return Err(ErrorKind::UnexpectedEof.into()),
                Ok(n) => n,
                Err(err) if err.kind() == ErrorKind::WouldBlock => return Ok(()),
                Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            };
            for chunk in buffer[..n].chunks_exact(SIZE) {
                // Safety: the kernel writes whole input_events; read unaligned.
                let event: libc::input_event = unsafe { std::ptr::read_unaligned(chunk.as_ptr().cast()) };
                match (event.type_, event.code) {
                    (EV_SYN, SYN_DROPPED) => {
                        self.dropped = true;
                        self.report.clear();
                    }
                    (EV_SYN, SYN_REPORT) => {
                        let events = if std::mem::take(&mut self.dropped) { self.state() } else { std::mem::take(&mut self.report) };
                        if !events.is_empty() {
                            out.push(json!({ "type": "pad-input", "id": self.id, "events": events }));
                        }
                    }
                    (EV_KEY | EV_ABS, code) if !self.dropped => self.report.push(json!([event.type_, code, event.value])),
                    _ => {}
                }
            }
        }
    }
}

/// The pads of the station, while a session is paired.
pub struct Pads {
    pads: HashMap<PathBuf, Pad>,
    /// Devices that are not pads: not opened again while they stay.
    others: HashSet<PathBuf>,
    next: u64,
    scanned: Option<Instant>,
}

impl Pads {
    pub fn new() -> Self {
        Self { pads: HashMap::new(), others: HashSet::new(), next: 1, scanned: None }
    }

    /// Plugged and unplugged pads, if it is time to look; then what they
    /// sent. All as messages for the server.
    pub fn poll(&mut self) -> Vec<Value> {
        let mut out = Vec::new();
        if self.scanned.is_none_or(|at| at.elapsed() >= RESCAN) {
            self.scanned = Some(Instant::now());
            self.scan(&mut out);
        }
        let mut gone = Vec::new();
        for (path, pad) in &mut self.pads {
            if let Err(err) = pad.read(&mut out) {
                log::info!("station: the pad at {} is gone ({err})", path.display());
                gone.push(path.clone());
            }
        }
        for path in gone {
            if let Some(pad) = self.pads.remove(&path) {
                out.push(json!({ "type": "pad-gone", "id": pad.id }));
            }
        }
        out
    }

    fn scan(&mut self, out: &mut Vec<Value>) {
        let Ok(entries) = std::fs::read_dir("/dev/input") else { return };
        let now: HashSet<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("event"))
            .map(|e| e.path())
            .collect();
        self.others.retain(|path| now.contains(path));
        for path in now {
            if self.pads.contains_key(&path) || self.others.contains(&path) {
                continue;
            }
            match Pad::open(&path, self.next) {
                Ok(Some((pad, description))) => {
                    self.next += 1;
                    out.push(description);
                    self.pads.insert(path, pad);
                }
                Ok(None) => {
                    self.others.insert(path);
                }
                // Not the seat's, or just plugged and its ACL not given
                // yet: tried again next time.
                Err(err) => log::debug!("station: {}: {err}", path.display()),
            }
        }
    }

    /// To wait on, beside the link's socket.
    pub fn fds(&self) -> impl Iterator<Item = BorrowedFd<'_>> {
        self.pads.values().map(|pad| pad.file.as_fd())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_requests_are_the_kernels() {
        // As linux/input.h makes them on x86-64 and arm64.
        assert_eq!(EVIOCGID, 0x8008_4502);
        assert_eq!(eviocgbit(EV_KEY, 96), 0x8060_4521);
        assert_eq!(eviocgabs(0), 0x8018_4540);
        assert_eq!(eviocgkey(96), 0x8060_4518);
    }
}
