//! Spectrum module: what the speakers play, as CAVA draws it.
//!
//!   Mini    (2 × 1)  one line across the bands
//!   Normal  (4 × 2)  12 bars
//!   Full    (4 × 2)  thin bars in a red–orange–yellow gradient, on a grid
//!
//! The sound is the default sink's monitor, recorded through PulseAudio
//! (`pulse::record_monitor`, served by pipewire-pulse) on a thread of its
//! own, mono at 44.1 kHz, in blocks of one frame. The thread analyses it
//! as CAVA does (`orbit_audio::Analyzer`): nothing slows a bar on its way
//! up, which is what keeps it in step with what is heard. It hands the GTK thread the newest
//! bars `framerate` times a second (a frame not drawn yet is replaced, never
//! queued), and nothing while all of them rest at zero.
//!
//! The line and the 12 bars take the module's CSS `color` (the accent);
//! the Full bars' gradient is fixed.

use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use gtk::{cairo, prelude::*};
use orbit_audio::Analyzer;
use serde::Deserialize;

use orbit_app::i18n::{text, tr, tr_args};

use crate::{board::Form, pulse};

/// `modules/spectrum.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpectrumConfig {
    /// Frames a second.
    pub framerate: u32,
    /// The lowest and highest frequencies shown, in Hz.
    pub low: f32,
    pub high: f32,
    /// As CAVA's `noise_reduction`, but for the fall only: 0 drops at
    /// once, 1 floats down slowly.
    pub noise_reduction: f32,
    /// The colours: one of [`THEMES`]; `custom` takes `colours`.
    pub theme: String,
    /// `custom`'s colours, top to bottom, `#rrggbb`.
    pub colours: Vec<String>,
    /// Small media buttons under the bars (Normal and Full).
    pub media: bool,
}

impl Default for SpectrumConfig {
    fn default() -> Self {
        Self {
            framerate: 60,
            low: 50.0,
            high: 10_000.0,
            noise_reduction: 0.77,
            theme: "classic".into(),
            colours: vec!["#eb3330".into(), "#fa8c1f".into(), "#fad933".into()],
            media: true,
        }
    }
}

/// The looks a left click goes through, in order. `classic`: the theme's
/// accent, and the fire gradient in Full; `accent` and `mono`: one colour
/// (the accent, the text's); the others a gradient in every size.
pub const THEMES: &[&str] = &["classic", "fire", "ocean", "aurora", "neon", "sunset", "accent", "mono", "custom"];

type Rgb = (f64, f64, f64);

/// The CSS colours at hand: the area's (the accent) and the card's text.
struct Ink {
    accent: gtk::gdk::RGBA,
    text: gtk::gdk::RGBA,
}

/// What the bars are painted with.
#[derive(Clone)]
enum Paint {
    /// The CSS colour of the area (the accent).
    Accent,
    /// The text's colour.
    Mono,
    /// Top to bottom.
    Gradient([Rgb; 3]),
}

fn hex(colour: &str) -> Option<Rgb> {
    let c = gtk::gdk::RGBA::parse(colour).ok()?;
    Some((f64::from(c.red()), f64::from(c.green()), f64::from(c.blue())))
}

/// The paint of `theme` in `form`.
fn paint(config: &SpectrumConfig, theme: &str, form: Form) -> Paint {
    let g = |a: Rgb, b: Rgb, c: Rgb| Paint::Gradient([a, b, c]);
    match theme {
        "classic" if form == Form::Full => g(RED, ORANGE, YELLOW),
        "classic" | "accent" => Paint::Accent,
        "mono" => Paint::Mono,
        "fire" => g(RED, ORANGE, YELLOW),
        "ocean" => g((0.30, 0.85, 0.95), (0.13, 0.55, 0.90), (0.10, 0.25, 0.65)),
        "aurora" => g((0.75, 0.40, 0.95), (0.25, 0.80, 0.85), (0.20, 0.90, 0.45)),
        "neon" => g((1.0, 0.20, 0.75), (0.60, 0.25, 1.0), (0.15, 0.85, 1.0)),
        "sunset" => g((0.98, 0.45, 0.35), (0.85, 0.30, 0.55), (0.40, 0.20, 0.60)),
        "custom" => {
            let pick = |i: usize, fallback: Rgb| config.colours.get(i).and_then(|c| hex(c)).unwrap_or(fallback);
            g(pick(0, RED), pick(1, ORANGE), pick(2, YELLOW))
        }
        _ => Paint::Accent,
    }
}

/// Sets `cr`'s source to `paint` over the height from `top` to `bottom`.
fn set_paint(cr: &cairo::Context, paint: &Paint, ink: &Ink, top: f64, bottom: f64) -> Result<(), cairo::Error> {
    match paint {
        Paint::Accent => set_colour(cr, &ink.accent, 1.0),
        Paint::Mono => set_colour(cr, &ink.text, 1.0),
        Paint::Gradient(stops) => {
            let gradient = cairo::LinearGradient::new(0.0, top, 0.0, bottom);
            for (offset, (r, g, b)) in [0.0, 0.5, 1.0].into_iter().zip(stops) {
                gradient.add_color_stop_rgb(offset, *r, *g, *b);
            }
            cr.set_source(&gradient)?;
        }
    }
    Ok(())
}

const RATE: u32 = 44_100;
/// The line's points, and the Full bars.
const LINE_POINTS: usize = 24;
const NORMAL_BARS: usize = 12;
const FULL_BARS: usize = 47;
/// Inside the card.
const PAD: f64 = 4.0;
const NORMAL_GAP: f64 = 3.0;
const FULL_GAP: f64 = 1.0;
/// The Full gradient, top to bottom.
const RED: (f64, f64, f64) = (0.92, 0.20, 0.18);
const ORANGE: (f64, f64, f64) = (0.98, 0.55, 0.12);
const YELLOW: (f64, f64, f64) = (0.98, 0.85, 0.20);

pub fn new(config: &SpectrumConfig, form: Form) -> gtk::Widget {
    let bands = match form {
        Form::Mini | Form::Line => LINE_POINTS,
        Form::Normal => NORMAL_BARS,
        Form::Full => FULL_BARS,
    };
    let area = gtk::DrawingArea::builder()
        .css_classes(["spectrum", form.class()])
        .hexpand(true)
        .vexpand(true)
        .build();
    if form == Form::Line {
        area.set_size_request(70, 24);
    }
    let bars = Rc::new(RefCell::new(vec![0.0f32; bands]));
    let theme = Rc::new(RefCell::new(config.theme.clone()));
    {
        let bars = bars.clone();
        let (theme, options) = (theme.clone(), config.clone());
        area.set_draw_func(move |area, cr, w, h| {
            let bars = bars.borrow();
            let colour = area.color();
            let ink = Ink {
                accent: colour,
                text: area.parent().map_or(colour, |p| p.color()),
            };
            let painted = paint(&options, &theme.borrow(), form);
            let (w, h) = (f64::from(w), f64::from(h));
            let drawn = match form {
                Form::Mini | Form::Line => draw_line(cr, &bars, w, h, &painted, &ink),
                Form::Normal => draw_bars(cr, &bars, w, h, &painted, &ink),
                Form::Full => draw_full(cr, &bars, w, h, &painted, &ink),
            };
            if let Err(err) = drawn {
                log::warn!("spectrum: drawing: {err}");
            }
        });
    }

    // The newest bars wait in `latest`; the channel only wakes the GTK
    // thread, once however many frames came meanwhile.
    let latest: Latest = Arc::default();
    let (wake, woken) = async_channel::bounded::<()>(1);
    clicks(&area, config, &theme);
    let media = config.media;
    let config = config.clone();
    {
        let latest = latest.clone();
        std::thread::Builder::new()
            .name("spectrum".into())
            .spawn(move || listen(&config, bands, &latest, &wake))
            .expect("a thread");
    }
    let weak = area.downgrade();
    gtk::glib::spawn_future_local(async move {
        while woken.recv().await.is_ok() {
            let Some(area) = weak.upgrade() else { break };
            if let Some(frame) = latest.lock().unwrap().take() {
                bars.replace(frame);
                area.queue_draw();
            }
        }
    });
    if media && matches!(form, Form::Normal | Form::Full) {
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["spectrum-media"])
            .build();
        column.append(&area);
        column.append(&crate::media::controls());
        return column.upcast();
    }
    area.upcast()
}

/// A left click goes to the next look (kept in the file); a right click
/// opens the module's menu.
fn clicks(area: &gtk::DrawingArea, config: &SpectrumConfig, theme: &Rc<RefCell<String>>) {
    {
        let theme = theme.clone();
        crate::settings::on_click(area, 1, move |widget, _, _| {
            let next = {
                let now = theme.borrow();
                let at = THEMES.iter().position(|t| *t == now.as_str()).unwrap_or(0);
                THEMES[(at + 1) % THEMES.len()].to_string()
            };
            widget.set_tooltip_text(Some(&tr(&format!("spectrum-theme-{next}"))));
            *theme.borrow_mut() = next.clone();
            widget.queue_draw();
            crate::settings::save("spectrum", &[("theme", next.into())], &[]);
        });
    }
    let (config, theme) = (config.clone(), theme.clone());
    crate::settings::on_click(area, 3, move |widget, x, y| menu(widget, x, y, &config, &theme));
}

/// The looks, the custom colours, the frame rate and how the bars fall.
fn menu(anchor: &gtk::Widget, x: f64, y: f64, config: &SpectrumConfig, theme: &Rc<RefCell<String>>) {
    use crate::settings::{Menu, save, section, submenu};
    let menu = Menu::new();
    let first = section(&menu, Some(&tr("panel-module-spectrum")));
    let looks = submenu(&first, &tr("spectrum-theme"));
    let choices: Vec<(String, String)> = THEMES
        .iter()
        .map(|t| ((*t).to_string(), tr(&format!("spectrum-theme-{t}"))))
        .collect();
    {
        let current = theme.borrow().clone();
        let (theme, anchor) = (theme.clone(), anchor.clone());
        menu.radio(&looks, &choices, &current, move |value| {
            *theme.borrow_mut() = value.clone();
            anchor.queue_draw();
            save("spectrum", &[("theme", value.into())], &[]);
        });
    }
    {
        let anchor = anchor.clone();
        let config = config.clone();
        menu.item(&first, &tr("spectrum-colours"), move || colours(&anchor, x, y, &config));
    }

    let more = section(&menu, None);
    let rate = submenu(&more, &tr("spectrum-framerate"));
    let choices: Vec<(String, String)> = [30u32, 60, 120]
        .iter()
        .map(|f| (f.to_string(), tr_args("spectrum-fps", &[("fps", text(f.to_string()))])))
        .collect();
    menu.radio(&rate, &choices, &config.framerate.to_string(), |value| {
        if let Ok(fps) = value.parse::<u32>() {
            save("spectrum", &[("framerate", fps.into())], &["spectrum"]);
        }
    });
    let fall = submenu(&more, &tr("spectrum-fall"));
    let current = match config.noise_reduction {
        n if n < 0.65 => "0.5",
        n if n > 0.85 => "0.9",
        _ => "0.77",
    };
    menu.radio(
        &fall,
        &[
            ("0.5".into(), tr("spectrum-fall-fast")),
            ("0.77".into(), tr("spectrum-fall-normal")),
            ("0.9".into(), tr("spectrum-fall-slow")),
        ],
        current,
        |value| {
            if let Ok(n) = value.parse::<f64>() {
                save("spectrum", &[("noise_reduction", n.into())], &["spectrum"]);
            }
        },
    );
    let media = config.media;
    menu.item(
        &more,
        &tr(if media { "spectrum-media-hide" } else { "spectrum-media-show" }),
        move || save("spectrum", &[("media", (!media).into())], &["spectrum"]),
    );
    menu.item(&more, &tr("spectrum-refresh"), || crate::settings::reload(&["spectrum"]));
    menu.open(anchor, x, y);
}

/// The custom look's three colours, top to bottom.
fn colours(anchor: &gtk::Widget, x: f64, y: f64, config: &SpectrumConfig) {
    let content = crate::settings::form_box(&tr("spectrum-colours"));
    let buttons: Vec<gtk::ColorDialogButton> = (0..3)
        .map(|i| {
            let button = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::builder().with_alpha(false).build()));
            let colour = config.colours.get(i).and_then(|c| gtk::gdk::RGBA::parse(c).ok());
            if let Some(colour) = colour {
                button.set_rgba(&colour);
            }
            let row = gtk::Box::builder().spacing(8).build();
            row.append(
                &gtk::Label::builder()
                    .label(tr(["spectrum-colour-top", "spectrum-colour-middle", "spectrum-colour-bottom"][i]))
                    .hexpand(true)
                    .xalign(0.0)
                    .build(),
            );
            row.append(&button);
            content.append(&row);
            button
        })
        .collect();
    let apply = gtk::Button::builder().label(tr("spectrum-colours-apply")).css_classes(["suggested-action"]).build();
    content.append(&apply);
    let popover = crate::settings::popover(anchor, x, y, &content);
    let weak = popover.downgrade();
    apply.connect_clicked(move |_| {
        let hexes: Vec<serde_json::Value> = buttons
            .iter()
            .map(|b| {
                let c = b.rgba();
                let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                format!("#{:02x}{:02x}{:02x}", byte(c.red()), byte(c.green()), byte(c.blue())).into()
            })
            .collect();
        crate::settings::save(
            "spectrum",
            &[("theme", "custom".into()), ("colours", serde_json::Value::Array(hexes))],
            &["spectrum"],
        );
        if let Some(p) = weak.upgrade() {
            p.popdown();
        }
    });
}

/// The bars not drawn yet.
type Latest = Arc<Mutex<Option<Vec<f32>>>>;

/// Records and analyses until the module is gone; records again when the
/// sound server or the default sink changes.
fn listen(config: &SpectrumConfig, bands: usize, latest: &Latest, wake: &async_channel::Sender<()>) {
    let fps = config.framerate.clamp(1, 144);
    let mut analyzer = Analyzer::new(RATE, bands, config.low, config.high, config.noise_reduction, fps);
    let per_frame = (RATE / fps) as usize;
    loop {
        let mut since = 0;
        let mut resting = false;
        let recorded = pulse::record_monitor(RATE, per_frame as u32, |samples| {
            analyzer.push(samples);
            since += samples.len();
            if since < per_frame {
                return true;
            }
            since = 0;
            let frame = analyzer.frame();
            let rest = frame.iter().all(|v| *v <= 0.0);
            if rest && resting {
                return !wake.is_closed();
            }
            resting = rest;
            latest.lock().unwrap().replace(frame);
            match wake.try_send(()) {
                Ok(()) | Err(async_channel::TrySendError::Full(())) => true,
                Err(async_channel::TrySendError::Closed(())) => false,
            }
        });
        match recorded {
            Ok(pulse::Stop::Done) => return,
            Ok(pulse::Stop::Again) => {}
            Err(err) => {
                log::warn!("spectrum: recording: {err}; again in 2 s");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
        if wake.is_closed() {
            return;
        }
    }
}

fn set_colour(cr: &cairo::Context, colour: &gtk::gdk::RGBA, alpha: f64) {
    cr.set_source_rgba(
        f64::from(colour.red()),
        f64::from(colour.green()),
        f64::from(colour.blue()),
        f64::from(colour.alpha()) * alpha,
    );
}

/// Mini: one smooth line through the bands, along the bottom when quiet.
fn draw_line(
    cr: &cairo::Context,
    bars: &[f32],
    w: f64,
    h: f64,
    paint: &Paint,
    ink: &Ink,
) -> Result<(), cairo::Error> {
    let (left, width) = (PAD, w - 2.0 * PAD);
    let (bottom, height) = (h - PAD, h - 2.0 * PAD);
    let points: Vec<(f64, f64)> = bars
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = left + width * i as f64 / (bars.len().max(2) - 1) as f64;
            (x, bottom - f64::from(*v) * height)
        })
        .collect();
    let Some(&(x0, y0)) = points.first() else { return Ok(()) };
    cr.move_to(x0, y0);
    // Catmull-Rom through the points, as Bézier curves.
    for i in 0..points.len() - 1 {
        let p0 = points[i.saturating_sub(1)];
        let (p1, p2) = (points[i], points[i + 1]);
        let p3 = points[(i + 2).min(points.len() - 1)];
        cr.curve_to(
            p1.0 + (p2.0 - p0.0) / 6.0,
            p1.1 + (p2.1 - p0.1) / 6.0,
            p2.0 - (p3.0 - p1.0) / 6.0,
            p2.1 - (p3.1 - p1.1) / 6.0,
            p2.0,
            p2.1,
        );
    }
    set_paint(cr, paint, ink, PAD, h - PAD)?;
    cr.set_line_width(1.5);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    cr.stroke()
}

/// Where bar `i` of `n` goes across `w`, with `gap` between bars.
fn bar_x(i: usize, n: usize, w: f64, gap: f64) -> (f64, f64) {
    let width = (w - 2.0 * PAD - gap * (n - 1) as f64) / n as f64;
    (PAD + i as f64 * (width + gap), width)
}

/// Normal: 12 bars with rounded tops; a 1 px stub when quiet.
fn draw_bars(
    cr: &cairo::Context,
    bars: &[f32],
    w: f64,
    h: f64,
    paint: &Paint,
    ink: &Ink,
) -> Result<(), cairo::Error> {
    let (bottom, height) = (h - PAD, h - 2.0 * PAD);
    set_paint(cr, paint, ink, PAD, bottom)?;
    for (i, v) in bars.iter().enumerate() {
        let (x, width) = bar_x(i, bars.len(), w, NORMAL_GAP);
        let tall = (f64::from(*v) * height).max(1.0);
        let r = (width / 2.0).min(2.0).min(tall / 2.0);
        let top = bottom - tall;
        cr.new_sub_path();
        cr.arc(x + width - r, top + r, r, -std::f64::consts::PI / 2.0, 0.0);
        cr.line_to(x + width, bottom);
        cr.line_to(x, bottom);
        cr.arc(
            x + r,
            top + r,
            r,
            std::f64::consts::PI,
            3.0 * std::f64::consts::PI / 2.0,
        );
        cr.close_path();
    }
    cr.fill()
}

/// Full: a grid, then thin bars cut from one gradient — yellow at the
/// bottom, orange in the middle, red at the top.
fn draw_full(
    cr: &cairo::Context,
    bars: &[f32],
    w: f64,
    h: f64,
    paint: &Paint,
    ink: &Ink,
) -> Result<(), cairo::Error> {
    let colour = &ink.accent;
    let (left, right) = (PAD, w - PAD);
    let (top, bottom) = (PAD, h - PAD);
    // The grid: 8 columns, and rows as many as keep its cells square, on
    // whole pixels so its lines are sharp.
    set_colour(cr, colour, 0.12);
    cr.set_line_width(1.0);
    let rows = (8.0 * (bottom - top) / (right - left)).round().max(1.0);
    for k in 0..=8 {
        let x = (left + (right - left) * f64::from(k) / 8.0).round() - 0.5;
        cr.move_to(x, top);
        cr.line_to(x, bottom);
    }
    for k in 0..=rows as i32 {
        let y = (top + (bottom - top) * f64::from(k) / rows).round() - 0.5;
        cr.move_to(left, y);
        cr.line_to(right, y);
    }
    cr.stroke()?;

    set_paint(cr, paint, ink, top, bottom)?;
    let height = bottom - top;
    for (i, v) in bars.iter().enumerate() {
        let (x, width) = bar_x(i, bars.len(), w, FULL_GAP);
        let tall = (f64::from(*v) * height).max(1.0);
        cr.rectangle(x, bottom - tall, width, tall);
    }
    cr.fill()
}

#[cfg(test)]
mod tests {
    use std::f32::consts::PI;

    use super::*;

    /// What the speakers play for a second, as Normal bars:
    /// `cargo test -p orbit-panel records -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn records_the_monitor() {
        let mut a = Analyzer::new(RATE, NORMAL_BARS, 50.0, 10_000.0, 0.77, 30);
        let (mut n, mut peak, mut frames) = (0usize, 0.0f32, 0);
        let start = std::time::Instant::now();
        let stop = pulse::record_monitor(RATE, 1024, |samples| {
            if n < 20 * 1024 {
                eprintln!(
                    "{:>6.1} ms: {} samples",
                    start.elapsed().as_secs_f64() * 1000.0,
                    samples.len()
                );
            }
            n += samples.len();
            peak = samples.iter().fold(peak, |p, s| p.max(s.abs()));
            a.push(samples);
            if n / (RATE as usize / 30) > frames {
                frames += 1;
                let bars: String = a
                    .frame()
                    .iter()
                    .map(|v| [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'][(v * 8.0) as usize])
                    .collect();
                eprintln!("|{bars}|");
            }
            n < RATE as usize
        });
        eprintln!(
            "{n} samples, peak {peak}, stopped: {}",
            matches!(stop, Ok(pulse::Stop::Done))
        );
        assert!(n >= RATE as usize);
    }

    /// The three sizes drawn to `$ORBIT_SPECTRUM_PPM`, 4× (a look without
    /// a panel): `ORBIT_SPECTRUM_PPM=x.ppm cargo test -p orbit-panel draws -- --ignored`.
    #[test]
    #[ignore]
    fn draws_the_three_sizes() {
        let Ok(path) = std::env::var("ORBIT_SPECTRUM_PPM") else {
            return;
        };
        // Something like music: a kick, a bass line, chords, a hi-hat.
        let signal = |t: usize| {
            let x = t as f32 / RATE as f32;
            let tones = [
                (55.0, 0.5),
                (110.0, 0.3),
                (330.0, 0.15),
                (440.0, 0.12),
                (660.0, 0.08),
                (1320.0, 0.04),
                (2640.0, 0.02),
                (5280.0, 0.01),
            ];
            let mut v: f32 = tones.iter().map(|(f, a)| a * (2.0 * PI * f * x).sin()).sum();
            // Cheap noise for the cymbals.
            v += 0.01 * (((t as u32).wrapping_mul(2654435761) >> 16) as f32 / 32768.0 - 1.0);
            v * 0.5
        };
        // Or real sound: mono f32 at 44.1 kHz in `$ORBIT_SPECTRUM_RAW`
        // (`ffmpeg -i song.mp3 -f f32le -ac 1 -ar 44100 song.raw`).
        let raw: Option<Vec<f32>> = std::env::var("ORBIT_SPECTRUM_RAW").ok().map(|p| {
            std::fs::read(p)
                .unwrap()
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect()
        });
        let frames = |bands| {
            let mut a = Analyzer::new(RATE, bands, 50.0, 10_000.0, 0.77, 30);
            let per = RATE as usize / 30;
            let mut frame = Vec::new();
            for f in 0..90 {
                let block: Vec<f32> = match &raw {
                    Some(raw) => raw[f * per..(f + 1) * per].to_vec(),
                    None => (f * per..(f + 1) * per).map(signal).collect(),
                };
                a.push(&block);
                frame = a.frame();
            }
            frame
        };
        let (scale, gap) = (4.0, 10.0);
        let sizes = [
            (Form::Mini, 70.0, 30.0, LINE_POINTS),
            (Form::Normal, 150.0, 70.0, NORMAL_BARS),
            (Form::Full, 150.0, 70.0, FULL_BARS),
        ];
        let width = sizes.iter().map(|s| s.1).sum::<f64>() + gap * 4.0;
        let mut surface = cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            (width * scale) as i32,
            ((70.0 + 2.0 * gap) * scale) as i32,
        )
        .unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        cr.scale(scale, scale);
        cr.set_source_rgb(0.08, 0.09, 0.11);
        cr.paint().unwrap();
        let accent = gtk::gdk::RGBA::new(0.27, 0.51, 0.71, 1.0);
        let ink = Ink { accent, text: gtk::gdk::RGBA::WHITE };
        let config = SpectrumConfig::default();
        let mut x = gap;
        for (form, w, h, bands) in sizes {
            let bars = frames(bands);
            cr.save().unwrap();
            cr.translate(x, gap);
            // The card.
            cr.set_source_rgb(0.15, 0.17, 0.21);
            cr.rectangle(0.0, 0.0, w, h);
            cr.fill().unwrap();
            match form {
                Form::Mini => draw_line(&cr, &bars, w, h, &paint(&config, "classic", form), &ink),
                Form::Normal => draw_bars(&cr, &bars, w, h, &paint(&config, "classic", form), &ink),
                _ => draw_full(&cr, &bars, w, h, &paint(&config, "classic", form), &ink),
            }
            .unwrap();
            cr.restore().unwrap();
            x += w + gap;
        }
        // A binary PPM (cairo here is built without PNG).
        drop(cr);
        surface.flush();
        let (w, h, stride) = (
            surface.width() as usize,
            surface.height() as usize,
            surface.stride() as usize,
        );
        let data = surface.data().unwrap();
        let mut ppm = format!("P6 {w} {h} 255\n").into_bytes();
        for y in 0..h {
            for x in 0..w {
                let p = &data[y * stride + x * 4..][..4];
                ppm.extend([p[2], p[1], p[0]]);
            }
        }
        std::fs::write(path, ppm).unwrap();
    }
}
