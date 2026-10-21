//! CPU, RAM, LAN and GPU modules: how busy each is, as a graph of the last
//! minute on a 4 × 8 grid, with the part's name at the top left and the
//! value now at the top right.
//!
//!   Mini    (2 × 1)  name and value on the graph
//!   Normal  (4 × 1)  the same, the value with more to it
//!   Full    (4 × 2)  and a line of details at the bottom
//!
//! Everything is read from the kernel every `interval` seconds on the GTK
//! thread — a few small files under /proc and /sys — except an NVIDIA
//! card's numbers, which only `nvidia-smi` gives and which are read off the
//! GTK thread.
//!
//!   cpu  /proc/stat; the clock from cpufreq, the temperature from the
//!        k10temp, zenpower or coretemp sensor
//!   ram  /proc/meminfo: used is total less available; swap in the details
//!   lan  /proc/net/dev for the interface of the default route (or every
//!        one but lo): down and up, each scaled to the busiest second shown
//!   gpu  amdgpu's gpu_busy_percent, VRAM and sensor; or nvidia-smi. With
//!        neither, the module stays hidden.

use std::{
    cell::RefCell,
    collections::VecDeque,
    path::{Path, PathBuf},
    rc::Rc,
    time::Instant,
};

use gtk::{cairo, glib, prelude::*};
use serde::Deserialize;

use crate::board::Form;

/// `modules/{cpu,ram,lan,gpu}.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MeterConfig {
    /// Seconds between readings.
    pub interval: u32,
    /// lan: the interface to watch, instead of the default route's.
    pub interface: Option<String>,
}

impl Default for MeterConfig {
    fn default() -> Self {
        Self {
            interval: 1,
            interface: None,
        }
    }
}

/// Readings kept: a minute at the default interval.
const HISTORY: usize = 60;
/// The second line (LAN up), beside the accent.
const SECOND: (f64, f64, f64) = (0.98, 0.55, 0.12);

/// How a series is scaled to the graph's height.
#[derive(Clone, Copy)]
enum Scale {
    /// 0 to 100.
    Percent,
    /// 0 to the largest value shown, never less than this.
    Auto(f64),
}

/// A graph with its labels.
struct Meter {
    root: gtk::Overlay,
    area: gtk::DrawingArea,
    value: gtk::Label,
    detail: Option<gtk::Label>,
    history: Rc<RefCell<Vec<VecDeque<f64>>>>,
}

impl Meter {
    fn new(class: &str, name: &str, form: Form, series: usize, scale: Scale) -> Self {
        let history = Rc::new(RefCell::new(vec![VecDeque::with_capacity(HISTORY); series]));
        let area = gtk::DrawingArea::builder().hexpand(true).vexpand(true).build();
        {
            let history = history.clone();
            area.set_draw_func(move |area, cr, w, h| {
                if let Err(err) = draw(cr, &history.borrow(), scale, f64::from(w), f64::from(h), &area.color()) {
                    log::warn!("meter: drawing: {err}");
                }
            });
        }
        let root = gtk::Overlay::builder()
            .css_classes(["meter", class, form.class()])
            .child(&area)
            .build();
        if form == Form::Line {
            root.set_size_request(70, 24);
        }
        let label = |text: &str, class: &str, halign| {
            gtk::Label::builder()
                .label(text)
                .css_classes([class])
                .halign(halign)
                .valign(gtk::Align::Start)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build()
        };
        let top = gtk::Box::builder().valign(gtk::Align::Start).build();
        let name = label(name, "name", gtk::Align::Start);
        let value = label("", "value", gtk::Align::End);
        value.set_hexpand(true);
        top.append(&name);
        top.append(&value);
        root.add_overlay(&top);
        let detail = (form == Form::Full).then(|| {
            let detail = label("", "detail", gtk::Align::Start);
            detail.set_valign(gtk::Align::End);
            root.add_overlay(&detail);
            detail
        });
        Self {
            root,
            area,
            value,
            detail,
            history,
        }
    }

    /// Adds a reading of each series.
    fn push(&self, values: &[f64]) {
        for (series, value) in self.history.borrow_mut().iter_mut().zip(values) {
            if series.len() == HISTORY {
                series.pop_front();
            }
            series.push_back(*value);
        }
        self.area.queue_draw();
    }

    /// The value on the graph, the details under it (Full), and all of
    /// it, with `more`, in the tooltip.
    fn show(&self, value: &str, detail: &str, more: &str) {
        self.value.set_label(value);
        if let Some(label) = &self.detail {
            label.set_label(detail);
        }
        let tip = [value, detail, more].into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join("\n");
        self.root.set_tooltip_text(Some(&tip));
    }
}

/// The grid, then each series as a line over a faint fill, newest at the
/// right.
fn draw(cr: &cairo::Context, history: &[VecDeque<f64>], scale: Scale, w: f64, h: f64, colour: &gtk::gdk::RGBA) -> Result<(), cairo::Error> {
    let (r, g, b, a) = (f64::from(colour.red()), f64::from(colour.green()), f64::from(colour.blue()), f64::from(colour.alpha()));
    cr.set_source_rgba(r, g, b, a * 0.12);
    cr.set_line_width(1.0);
    for k in 0..=8 {
        let x = (w * f64::from(k) / 8.0).round().clamp(0.5, w - 0.5);
        cr.move_to(x, 0.0);
        cr.line_to(x, h);
    }
    for k in 0..=4 {
        let y = (h * f64::from(k) / 4.0).round().clamp(0.5, h - 0.5);
        cr.move_to(0.0, y);
        cr.line_to(w, y);
    }
    cr.stroke()?;

    let top = match scale {
        Scale::Percent => 100.0,
        Scale::Auto(floor) => history.iter().flatten().copied().fold(floor, f64::max),
    };
    let step = w / (HISTORY - 1) as f64;
    for (i, series) in history.iter().enumerate().rev() {
        if series.len() < 2 {
            continue;
        }
        let (r, g, b) = if i == 0 { (r, g, b) } else { SECOND };
        let x0 = w - (series.len() - 1) as f64 * step;
        let y = |v: f64| h - 1.0 - (v / top).clamp(0.0, 1.0) * (h - 2.0);
        cr.move_to(x0, y(series[0]));
        for (k, v) in series.iter().enumerate().skip(1) {
            cr.line_to(x0 + k as f64 * step, y(*v));
        }
        let line = cr.copy_path()?;
        cr.line_to(w, h);
        cr.line_to(x0, h);
        cr.close_path();
        cr.set_source_rgba(r, g, b, 0.25);
        cr.fill()?;
        cr.append_path(&line);
        cr.set_source_rgba(r, g, b, 1.0);
        cr.set_line_width(1.5);
        cr.set_line_join(cairo::LineJoin::Round);
        cr.stroke()?;
    }
    Ok(())
}

/// Reads with `read` now and every `interval` seconds while the meter
/// lives.
fn every(interval: u32, meter: Meter, mut read: impl FnMut(&Meter) + 'static) -> gtk::Widget {
    let root = meter.root.clone();
    read(&meter);
    let weak = root.downgrade();
    glib::timeout_add_seconds_local(interval.max(1), move || {
        if weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        read(&meter);
        glib::ControlFlow::Continue
    });
    root.upcast()
}

/// A short size: `1.2G`, `340M`, `12K`, `512` (bytes, powers of 1024).
pub fn short(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["", "K", "M", "G", "T"];
    let mut v = bytes.max(0.0);
    let mut unit = 0;
    while v >= 1000.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 || v >= 10.0 {
        format!("{v:.0}{}", UNITS[unit])
    } else {
        format!("{v:.1}{}", UNITS[unit])
    }
}

fn read(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn read_num(path: impl AsRef<Path>) -> Option<f64> {
    read(path)?.trim().parse().ok()
}

/// The first hwmon sensor called one of `names`: its first temperature, °C.
fn hwmon_temp(names: &[&str]) -> Option<f64> {
    let dirs = std::fs::read_dir("/sys/class/hwmon").ok()?;
    let mut dirs: Vec<PathBuf> = dirs.flatten().map(|e| e.path()).collect();
    dirs.sort();
    dirs.into_iter()
        .find(|d| read(d.join("name")).is_some_and(|n| names.contains(&n.trim())))
        .and_then(|d| read_num(d.join("temp1_input")))
        .map(|milli| milli / 1000.0)
}

// ------------------------------------------------------------------ CPU

pub fn cpu(config: &MeterConfig, form: Form) -> gtk::Widget {
    let meter = Meter::new("cpu", "CPU", form, 1, Scale::Percent);
    let mut last: Option<(u64, u64)> = None;
    let cores = std::thread::available_parallelism().map_or(0, |n| n.get());
    every(config.interval, meter, move |meter| {
        let Some(now) = cpu_times() else { return };
        let busy = match last.replace(now) {
            Some((total, idle)) if now.0 > total => 100.0 * (1.0 - (now.1 - idle) as f64 / (now.0 - total) as f64),
            _ => return,
        };
        meter.push(&[busy]);
        let mut detail = Vec::new();
        if let Some(ghz) = cpu_ghz() {
            detail.push(format!("{ghz:.2} GHz"));
        }
        if let Some(t) = hwmon_temp(&["k10temp", "zenpower", "coretemp"]) {
            detail.push(format!("{t:.0} °C"));
        }
        let value = match form {
            Form::Normal => detail.iter().fold(format!("{busy:.0}%"), |s, d| format!("{s} · {d}")),
            _ => format!("{busy:.0}%"),
        };
        meter.show(&value, &detail.join(" · "), &format!("{cores} threads"));
    })
}

/// All CPUs' (total, idle + iowait) jiffies.
fn cpu_times() -> Option<(u64, u64)> {
    let stat = read("/proc/stat")?;
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let fields: Vec<u64> = line.split_whitespace().skip(1).filter_map(|f| f.parse().ok()).collect();
    // user nice system idle iowait irq softirq steal (guest is in user).
    let total = fields.iter().take(8).sum();
    let idle = fields.get(3)? + fields.get(4).unwrap_or(&0);
    Some((total, idle))
}

/// The cores' average clock.
fn cpu_ghz() -> Option<f64> {
    let khz: Vec<f64> = std::fs::read_dir("/sys/devices/system/cpu")
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().strip_prefix("cpu").is_some_and(|n| n.parse::<u32>().is_ok()))
        .filter_map(|e| read_num(e.path().join("cpufreq/scaling_cur_freq")))
        .collect();
    (!khz.is_empty()).then(|| khz.iter().sum::<f64>() / khz.len() as f64 / 1e6)
}

// ------------------------------------------------------------------ RAM

pub fn ram(config: &MeterConfig, form: Form) -> gtk::Widget {
    let meter = Meter::new("ram", "RAM", form, 1, Scale::Percent);
    every(config.interval, meter, move |meter| {
        let Some(mem) = meminfo() else { return };
        let used = mem.total - mem.available;
        let percent = 100.0 * used / mem.total.max(1.0);
        meter.push(&[percent]);
        let value = match form {
            Form::Normal | Form::Full => format!("{percent:.0}% · {} / {}", short(used), short(mem.total)),
            _ => format!("{percent:.0}%"),
        };
        let mut detail = format!("cache {}", short(mem.cached));
        if mem.swap_total > 0.0 {
            detail = format!("swap {} / {} · {detail}", short(mem.swap_total - mem.swap_free), short(mem.swap_total));
        }
        meter.show(&value, &detail, "");
    })
}

struct Mem {
    total: f64,
    available: f64,
    cached: f64,
    swap_total: f64,
    swap_free: f64,
}

/// /proc/meminfo, in bytes.
fn meminfo() -> Option<Mem> {
    let text = read("/proc/meminfo")?;
    let get = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
            .and_then(|v| v.split_whitespace().next()?.parse::<f64>().ok())
            .map(|kib| kib * 1024.0)
    };
    Some(Mem {
        total: get("MemTotal")?,
        available: get("MemAvailable")?,
        cached: get("Cached").unwrap_or(0.0),
        swap_total: get("SwapTotal").unwrap_or(0.0),
        swap_free: get("SwapFree").unwrap_or(0.0),
    })
}

// ------------------------------------------------------------------ LAN

pub fn lan(config: &MeterConfig, form: Form) -> gtk::Widget {
    // Down in the accent, up beside it; scaled to at least 100 KiB/s so a
    // quiet line stays low.
    let meter = Meter::new("lan", "LAN", form, 2, Scale::Auto(100.0 * 1024.0));
    let wanted = config.interface.clone();
    let mut last: Option<(Instant, u64, u64)> = None;
    every(config.interval, meter, move |meter| {
        let iface = wanted.clone().or_else(default_route);
        let Some((rx, tx)) = net_bytes(iface.as_deref()) else { return };
        let now = Instant::now();
        let Some((then, last_rx, last_tx)) = last.replace((now, rx, tx)) else { return };
        let secs = now.duration_since(then).as_secs_f64().max(0.001);
        let down = rx.saturating_sub(last_rx) as f64 / secs;
        let up = tx.saturating_sub(last_tx) as f64 / secs;
        meter.push(&[down, up]);
        let value = match form {
            Form::Mini | Form::Line => format!("↓{}", short(down)),
            _ => format!("↓ {}/s ↑ {}/s", short(down), short(up)),
        };
        let detail = format!("total ↓ {} ↑ {}", short(rx as f64), short(tx as f64));
        meter.show(&value, &detail, iface.as_deref().unwrap_or("all interfaces"));
    })
}

/// The interface of the default route.
fn default_route() -> Option<String> {
    let routes = read("/proc/net/route")?;
    routes.lines().skip(1).find_map(|l| {
        let mut f = l.split_whitespace();
        let iface = f.next()?;
        (f.next()? == "00000000").then(|| iface.to_string())
    })
}

/// Bytes received and sent by `iface`, or by every interface but lo.
fn net_bytes(iface: Option<&str>) -> Option<(u64, u64)> {
    let text = read("/proc/net/dev")?;
    let mut sums = None;
    for line in text.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else { continue };
        let name = name.trim();
        if iface.map_or(name == "lo", |want| name != want) {
            continue;
        }
        let f: Vec<u64> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
        let (rx, tx) = (*f.first()?, *f.get(8)?);
        let (r, t) = sums.unwrap_or((0, 0));
        sums = Some((r + rx, t + tx));
    }
    sums
}

// ------------------------------------------------------------------ GPU

pub fn gpu(config: &MeterConfig, form: Form) -> gtk::Widget {
    let Some(source) = GpuSource::find() else {
        log::info!("gpu: no amdgpu card and no nvidia-smi; module hidden");
        let empty = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        empty.set_visible(false);
        return empty.upcast();
    };
    let meter = Meter::new("gpu", "GPU", form, 1, Scale::Percent);
    let shown = Rc::new(RefCell::new(None::<GpuReading>));
    every(config.interval, meter, move |meter| match &source {
        GpuSource::Amd(dir) => {
            if let Some(reading) = amd_reading(dir) {
                show_gpu(meter, form, &reading);
            }
        }
        GpuSource::Nvidia => {
            // The last reading shows; the next comes off the GTK thread.
            if let Some(reading) = shown.borrow_mut().take() {
                show_gpu(meter, form, &reading);
            }
            let shown = shown.clone();
            glib::spawn_future_local(async move {
                if let Ok(Some(reading)) = gtk::gio::spawn_blocking(nvidia_reading).await {
                    shown.replace(Some(reading));
                }
            });
        }
    })
}

enum GpuSource {
    /// The card's `device` directory.
    Amd(PathBuf),
    Nvidia,
}

impl GpuSource {
    fn find() -> Option<Self> {
        let mut cards: Vec<PathBuf> = std::fs::read_dir("/sys/class/drm")
            .ok()?
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().strip_prefix("card").is_some_and(|n| n.parse::<u32>().is_ok()))
            .map(|e| e.path().join("device"))
            .collect();
        cards.sort();
        if let Some(dir) = cards.into_iter().find(|d| d.join("gpu_busy_percent").exists()) {
            return Some(Self::Amd(dir));
        }
        let nvidia = std::process::Command::new("nvidia-smi")
            .arg("-L")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        nvidia.is_ok_and(|s| s.success()).then_some(Self::Nvidia)
    }
}

#[derive(Default)]
struct GpuReading {
    busy: f64,
    vram: Option<(f64, f64)>,
    temp: Option<f64>,
    ghz: Option<f64>,
}

fn amd_reading(dir: &Path) -> Option<GpuReading> {
    let busy = read_num(dir.join("gpu_busy_percent"))?;
    let vram = read_num(dir.join("mem_info_vram_used")).zip(read_num(dir.join("mem_info_vram_total")));
    let hwmon = std::fs::read_dir(dir.join("hwmon")).ok().and_then(|mut d| d.next()?.ok()).map(|e| e.path());
    let temp = hwmon.as_ref().and_then(|h| read_num(h.join("temp1_input"))).map(|m| m / 1000.0);
    let ghz = hwmon.as_ref().and_then(|h| read_num(h.join("freq1_input"))).map(|hz| hz / 1e9);
    Some(GpuReading { busy, vram, temp, ghz })
}

fn nvidia_reading() -> Option<GpuReading> {
    let out = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=utilization.gpu,memory.used,memory.total,temperature.gpu,clocks.gr",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let f: Vec<Option<f64>> = text.lines().next()?.split(',').map(|v| v.trim().parse().ok()).collect();
    let get = |i: usize| f.get(i).copied().flatten();
    let mib = 1024.0 * 1024.0;
    Some(GpuReading {
        busy: get(0)?,
        vram: get(1).zip(get(2)).map(|(u, t)| (u * mib, t * mib)),
        temp: get(3),
        ghz: get(4).map(|mhz| mhz / 1000.0),
    })
}

fn show_gpu(meter: &Meter, form: Form, r: &GpuReading) {
    meter.push(&[r.busy]);
    let temp = r.temp.map(|t| format!("{t:.0} °C"));
    let value = match (form, &temp) {
        (Form::Normal | Form::Full, Some(t)) => format!("{:.0}% · {t}", r.busy),
        _ => format!("{:.0}%", r.busy),
    };
    let mut detail = Vec::new();
    if let Some((used, total)) = r.vram {
        detail.push(format!("VRAM {}/{}", short(used), short(total)));
    }
    if let Some(ghz) = r.ghz {
        detail.push(format!("{ghz:.2} GHz"));
    }
    meter.show(&value, &detail.join(" · "), "");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_short() {
        assert_eq!(short(512.0), "512");
        assert_eq!(short(1536.0), "1.5K");
        assert_eq!(short(15.4 * 1024.0 * 1024.0 * 1024.0), "15G");
        assert_eq!(short(2.0 * 1024.0 * 1024.0), "2.0M");
    }

    #[test]
    fn this_machine_reads() {
        assert!(cpu_times().is_some());
        let mem = meminfo().unwrap();
        assert!(mem.total > mem.available);
        assert!(net_bytes(None).is_some());
    }
}
