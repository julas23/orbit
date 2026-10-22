//! The window cards of Mod+Tab (`switcher`), after Sun's Project Looking
//! Glass: every window a card standing in space, turned a little to the
//! side, one behind the other going back and to the right, over the
//! blurred, dimmed screen. Each card is the window's picture under a title
//! bar — the program's name, then what it shows (its title: the file, the
//! page, the picture). Tab sends the front card to the back and brings the
//! next forward; the one in front gets the keyboard when Mod is let go.
//!
//! Each card is drawn off screen (title bar by Pango, the window's
//! surfaces scaled down), then put on screen through a perspective: the
//! card's corners are projected from 3D, and a shader maps each pixel of
//! their bounding box back onto the card (a homography), rounding the
//! corners and shading the cards further back.

use std::collections::HashMap;

use glam::{Mat3, Vec3};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Color32F, Frame, Offscreen, Renderer, Texture,
            element::{
                Element, Id, Kind, RenderElement, UnderlyingStorage,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::SolidColorRenderElement,
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            },
            gles::{GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName, UniformType, UniformValue},
            utils::{CommitCounter, DamageSet, OpaqueRegions, draw_render_elements},
        },
    },
    desktop::Window,
    output::Output,
    utils::{Buffer, Physical, Point, Rectangle, Scale, Size, Transform},
    wayland::{compositor::with_states, seat::WaylandFocus, shell::xdg::XdgToplevelSurfaceData},
};

use crate::{render::OutputElement, state::Orbit};

/// The title bar's height and the cards' corner radius, in logical pixels.
const TITLE: f64 = 34.0;
const RADIUS: f32 = 12.0;
/// How far each card turns (radians): its right edge further away.
const TURN: f32 = 0.62;
/// How many cards show; those further back fade out.
const SHOWN: f32 = 6.0;

const SHADER: &str = r#"
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

// The bounding box (0…1) back onto the card (0…1).
uniform mat3 inv;
// The card's size in its own pixels, its corners' radius, and how many
// of its pixels one pixel of the screen spans (for smooth edges).
uniform vec2 size;
uniform float radius;
uniform float aa;
uniform float shade;

void main() {
    vec3 p = inv * vec3(v_coords, 1.0);
    vec2 uv = p.xy / p.z;
    vec2 half_size = size * 0.5;
    vec2 e = abs(uv * size - half_size) - (half_size - vec2(radius));
    float d = length(max(e, 0.0)) + min(max(e.x, e.y), 0.0) - radius;
    float cover = clamp(0.5 - d / aa, 0.0, 1.0);
    if (cover <= 0.0) {
        discard;
    }
    vec4 color = texture2D(tex, clamp(uv, 0.0, 1.0));
#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif
    color.rgb *= shade;
    gl_FragColor = color * alpha * cover;
}
"#;

pub fn compile(renderer: &mut GlesRenderer) -> Result<GlesTexProgram, GlesError> {
    renderer.compile_custom_texture_shader(
        SHADER,
        &[
            UniformName::new("inv", UniformType::Matrix3x3),
            UniformName::new("size", UniformType::_2f),
            UniformName::new("radius", UniformType::_1f),
            UniformName::new("aa", UniformType::_1f),
            UniformName::new("shade", UniformType::_1f),
        ],
    )
}

/// What the cards keep while they are shown: the title bars by window,
/// title and width, the cards' textures by window, the blur's textures,
/// and the programs' names by app id.
#[derive(Default)]
pub struct Cache {
    titles: HashMap<(u64, String, i32), MemoryRenderBuffer>,
    cards: HashMap<u64, GlesTexture>,
    pub blur: crate::eyes::Cache,
    names: HashMap<String, String>,
}

/// The projective map taking the unit square's corners (0,0), (1,0),
/// (1,1), (0,1) to `quad` (Heckbert's square-to-quad).
pub fn square_to_quad(quad: [(f32, f32); 4]) -> Mat3 {
    let [(x0, y0), (x1, y1), (x2, y2), (x3, y3)] = quad;
    let (dx1, dx2, dx3) = (x1 - x2, x3 - x2, x0 - x1 + x2 - x3);
    let (dy1, dy2, dy3) = (y1 - y2, y3 - y2, y0 - y1 + y2 - y3);
    let (g, h) = if dx3.abs() < 1e-6 && dy3.abs() < 1e-6 {
        (0.0, 0.0)
    } else {
        let den = dx1 * dy2 - dx2 * dy1;
        ((dx3 * dy2 - dx2 * dy3) / den, (dx1 * dy3 - dx3 * dy1) / den)
    };
    Mat3::from_cols(
        Vec3::new(x1 - x0 + g * x1, y1 - y0 + g * y1, g),
        Vec3::new(x3 - x0 + h * x3, y3 - y0 + h * y3, h),
        Vec3::new(x0, y0, 1.0),
    )
}

/// Where the corners of a card `depth` cards back (fractional while they
/// move) fall on a screen `w`×`h`, for a card `cw`×`ch`: top left, top
/// right, bottom right, bottom left.
pub fn card_corners(depth: f32, w: f32, h: f32, cw: f32, ch: f32) -> [(f32, f32); 4] {
    let focal = 1.6 * w;
    // Back and to the right, a little higher, each card.
    let centre = (0.36 * w + depth * 0.07 * w, 0.60 * h - depth * 0.075 * h, depth * 0.32 * cw);
    let (sin, cos) = TURN.sin_cos();
    let project = |lx: f32, ly: f32| {
        let x = centre.0 + lx * cos;
        let z = centre.2 + lx * sin;
        let y = centre.1 + ly;
        let k = focal / (focal + z).max(1.0);
        (w / 2.0 + (x - w / 2.0) * k, h / 2.0 + (y - h / 2.0) * k)
    };
    let (a, b) = (cw / 2.0, ch / 2.0);
    [project(-a, -b), project(a, -b), project(a, b), project(-a, b)]
}

/// A card on screen: its texture, mapped through a perspective onto its
/// bounding box.
pub struct CardElement {
    id: Id,
    texture: GlesTexture,
    program: GlesTexProgram,
    geometry: Rectangle<i32, Physical>,
    inv: Mat3,
    aa: f32,
    shade: f32,
    alpha: f32,
}

impl Element for CardElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        CommitCounter::default()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size(self.texture.size().to_f64())
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry
    }

    fn damage_since(&self, _scale: Scale<f64>, _commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        DamageSet::from_slice(&[Rectangle::from_size(self.geometry.size)])
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for CardElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&smithay::utils::user_data::UserDataMap>,
    ) -> Result<(), GlesError> {
        let size = self.texture.size();
        let uniforms = [
            Uniform::new("inv", UniformValue::Matrix3x3 { matrices: vec![self.inv.to_cols_array()], transpose: false }),
            Uniform::new("size", (size.w as f32, size.h as f32)),
            Uniform::new("radius", RADIUS * size.w as f32 / self.geometry.size.w.max(1) as f32),
            Uniform::new("aa", self.aa),
            Uniform::new("shade", self.shade),
        ];
        frame.render_texture_from_to(&self.texture, src, dst, damage, &[], Transform::Normal, self.alpha, Some(&self.program), &uniforms)
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

/// The name a program goes by: its desktop entry's `Name`, else its app
/// id (or X class) made readable.
fn app_name(app_id: &str) -> String {
    if app_id.is_empty() {
        return String::new();
    }
    let home = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|d| !d.is_empty())
        .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.local/share")));
    let dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let dirs: Vec<String> = home.into_iter().chain(dirs.split(':').map(str::to_owned)).map(|d| format!("{d}/applications")).collect();
    let name_in = |text: &str| {
        let mut in_entry = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_entry = line == "[Desktop Entry]";
            } else if in_entry && let Some(name) = line.strip_prefix("Name=") {
                return Some(name.to_owned());
            }
        }
        None
    };
    for file in [app_id.to_owned(), app_id.to_lowercase()] {
        for dir in &dirs {
            if let Some(name) = std::fs::read_to_string(format!("{dir}/{file}.desktop")).ok().and_then(|t| name_in(&t)) {
                return name;
            }
        }
    }
    // An X class or an app id not named after its file.
    let wm_class = format!("StartupWMClass={app_id}");
    for dir in &dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for entry in entries.flatten() {
            let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
            if text.lines().any(|l| l.trim().eq_ignore_ascii_case(&wm_class))
                && let Some(name) = name_in(&text)
            {
                return name;
            }
        }
    }
    let last = app_id.rsplit('.').next().unwrap_or(app_id);
    let mut chars = last.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// What a window shows, without the program's name the title often ends
/// with ("notes.md — Orbit Edit").
fn detail(title: &str, name: &str, app_id: &str) -> String {
    for sep in [" — ", " – ", " - ", " · "] {
        if let Some((head, tail)) = title.rsplit_once(sep)
            && [name, app_id].iter().any(|n| !n.is_empty() && tail.trim().eq_ignore_ascii_case(n))
        {
            return head.trim().to_owned();
        }
    }
    if title.trim().eq_ignore_ascii_case(name) { String::new() } else { title.trim().to_owned() }
}

/// A window's title and app id (an X window's class).
fn title_and_app(window: &Window) -> (String, String) {
    if let Some(x) = window.x11_surface() {
        return (x.title(), x.class());
    }
    let Some(toplevel) = window.toplevel() else { return Default::default() };
    with_states(toplevel.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .map(|d| {
                let d = d.lock().unwrap();
                (d.title.clone().unwrap_or_default(), d.app_id.clone().unwrap_or_default())
            })
            .unwrap_or_default()
    })
}

/// The title bar, `w`×`h` pixels: the program's name in bold, then what
/// the window shows, cut short with an ellipsis.
fn title_bar(name: &str, detail: &str, w: i32, h: i32, scale: f64) -> Option<MemoryRenderBuffer> {
    use pangocairo::{cairo, pango};
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).ok()?;
    {
        let cr = cairo::Context::new(&surface).ok()?;
        cr.set_source_rgb(0.231, 0.259, 0.322);
        cr.paint().ok()?;
        let layout = pangocairo::functions::create_layout(&cr);
        let mut font = pango::FontDescription::from_string("Sans");
        font.set_absolute_size(13.0 * scale * pango::SCALE as f64);
        layout.set_font_description(Some(&font));
        let pad = (14.0 * scale) as i32;
        layout.set_width((w - 2 * pad).max(1) * pango::SCALE);
        layout.set_ellipsize(pango::EllipsizeMode::End);
        layout.set_single_paragraph_mode(true);
        let escape = |s: &str| pango::glib::markup_escape_text(s).to_string();
        let markup = if detail.is_empty() {
            format!("<b>{}</b>", escape(name))
        } else {
            format!("<b>{}</b>   <span foreground=\"#aeb7c6\">{}</span>", escape(name), escape(detail))
        };
        layout.set_markup(&markup);
        let (_, text_h) = layout.pixel_size();
        cr.set_source_rgb(0.925, 0.937, 0.957);
        cr.move_to(pad as f64, ((h - text_h) / 2) as f64);
        pangocairo::functions::show_layout(&cr, &layout);
    }
    surface.flush();
    let stride = surface.stride();
    let data = surface.data().ok()?;
    // Cairo's rows may be longer than the picture.
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for row in data.chunks(stride as usize).take(h as usize) {
        pixels.extend_from_slice(&row[..(w * 4) as usize]);
    }
    Some(MemoryRenderBuffer::from_slice(&pixels, Fourcc::Argb8888, (w, h), 1, Transform::Normal, None))
}

impl Orbit {
    /// The monitor the cards are shown on: the first (the laptop's).
    pub fn cards_output(&self) -> Option<Output> {
        self.monitors().first().and_then(|o| self.output_named(o))
    }

    /// The window's card, `size` pixels: its title bar above its picture.
    fn draw_card(&mut self, renderer: &mut GlesRenderer, window: &Window, size: Size<i32, Physical>, scale: f64) -> Option<GlesTexture> {
        let id = crate::workspace::id(window);
        let (title, app_id) = title_and_app(window);
        let cache = &mut self.switcher.as_mut()?.cache;
        let name = cache.names.entry(app_id.clone()).or_insert_with(|| app_name(&app_id)).clone();
        let detail = detail(&title, &name, &app_id);
        let bar_h = (TITLE * scale).round() as i32;
        let bar = match cache.titles.get(&(id, title.clone(), size.w)) {
            Some(bar) => bar.clone(),
            None => {
                let bar = title_bar(&name, &detail, size.w, bar_h, scale)?;
                cache.titles.insert((id, title, size.w), bar.clone());
                bar
            }
        };
        let mut texture = match cache.cards.get(&id).filter(|t| t.size() == (size.w, size.h).into()) {
            Some(t) => t.clone(),
            None => {
                let t: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, (size.w, size.h).into()).ok()?;
                cache.cards.insert(id, t.clone());
                t
            }
        };

        // The picture, as large as fits under the bar, centred.
        let geo = window.geometry();
        let room = (size.w as f64, (size.h - bar_h) as f64);
        let k = (room.0 / geo.size.w.max(1) as f64).min(room.1 / geo.size.h.max(1) as f64);
        let at: Point<i32, Physical> = (
            ((room.0 - geo.size.w as f64 * k) / 2.0 - geo.loc.x as f64 * k).round() as i32,
            (bar_h as f64 + (room.1 - geo.size.h as f64 * k) / 2.0 - geo.loc.y as f64 * k).round() as i32,
        )
            .into();
        let picture: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = match window.wl_surface() {
            Some(surface) => render_elements_from_surface_tree(renderer, &surface, at, Scale::from(k), 1.0, Kind::Unspecified),
            None => Vec::new(),
        };
        let bar = MemoryRenderBufferRenderElement::from_buffer(renderer, (0.0, 0.0), &bar, None, None, None, Kind::Unspecified).ok()?;
        let full = Rectangle::from_size(size);
        {
            let mut target = renderer.bind(&mut texture).ok()?;
            let mut frame = renderer.render(&mut target, size, Transform::Normal).ok()?;
            frame.clear(Color32F::new(0.153, 0.169, 0.208, 1.0), &[full]).ok()?;
            draw_render_elements::<GlesRenderer, _, _>(&mut frame, Scale::from(k), &picture, &[full]).ok()?;
            draw_render_elements::<GlesRenderer, _, _>(&mut frame, Scale::from(1.0), &[bar], &[full]).ok()?;
            let _ = frame.finish().ok()?;
        }
        Some(texture)
    }

    /// The cards on `output`, topmost first, over the blurred screen.
    pub fn card_elements(&mut self, output: &Output, renderer: &mut GlesRenderer, scale: Scale<f64>, shaders: &crate::rounded::Shaders, elements: &mut Vec<OutputElement>) {
        let Some(geo) = self.space.output_geometry(output) else { return };
        let Some(switcher) = self.switcher.as_ref() else { return };
        let (list, index, progress, back) = (switcher.list.clone(), switcher.index, switcher.progress(), switcher.stepped.is_some_and(|(_, b)| b));
        let stepped = switcher.stepped.is_some();
        let n = list.len();
        let (w, h) = (geo.size.w as f32, geo.size.h as f32);
        let (cw, ch) = (0.42 * w, 0.42 * w * 0.66);
        let s = scale.x;
        let size: Size<i32, Physical> = ((cw as f64 * s).round() as i32, (ch as f64 * s).round() as i32).into();

        // Each card's depth: how many are before it, moving there from
        // one step behind (or before, going back). The card that went
        // round flies out of the front, or into it, fading.
        let mut cards: Vec<(f32, f32, &Window)> = Vec::new();
        for (i, window) in list.iter().enumerate() {
            let d = ((i + n - index) % n) as f32;
            let still = 1.0 - progress;
            let (depth, alpha) = match (stepped && n > 1, back) {
                (false, _) => (d, 1.0),
                (true, false) if d as usize == n - 1 => {
                    if progress < 0.5 { (-progress * 1.2, 1.0 - progress * 2.0) } else { (d + still, progress * 2.0 - 1.0) }
                }
                (true, true) if d == 0.0 => {
                    if progress < 0.5 { (n as f32 - 1.0 + progress, 1.0 - progress * 2.0) } else { (-still * 1.2, progress * 2.0 - 1.0) }
                }
                (true, false) => (d + still, 1.0),
                (true, true) => (d - still, 1.0),
            };
            let fade = (SHOWN + 1.0 - depth).clamp(0.0, 1.0);
            if alpha * fade > 0.0 {
                cards.push((depth, alpha * fade, window));
            }
        }
        // Front first: the nearest is drawn on top.
        cards.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (depth, alpha, window) in cards {
            let window = window.clone();
            let Some(texture) = self.draw_card(renderer, &window, size, s) else { continue };
            let corners = card_corners(depth, w, h, cw, ch).map(|(x, y)| (x * s as f32, y * s as f32));
            let (min_x, max_x) = corners.iter().fold((f32::MAX, f32::MIN), |(a, b), c| (a.min(c.0), b.max(c.0)));
            let (min_y, max_y) = corners.iter().fold((f32::MAX, f32::MIN), |(a, b), c| (a.min(c.1), b.max(c.1)));
            let bbox = Rectangle::<i32, Physical>::new(
                (min_x.floor() as i32, min_y.floor() as i32).into(),
                ((max_x.ceil() - min_x.floor()) as i32, (max_y.ceil() - min_y.floor()) as i32).into(),
            );
            if bbox.size.w <= 0 || bbox.size.h <= 0 {
                continue;
            }
            let normal = corners.map(|(x, y)| ((x - bbox.loc.x as f32) / bbox.size.w as f32, (y - bbox.loc.y as f32) / bbox.size.h as f32));
            let projected_w = ((corners[1].0 - corners[0].0).powi(2) + (corners[1].1 - corners[0].1).powi(2)).sqrt();
            elements.push(OutputElement::Card(CardElement {
                id: Id::new(),
                texture,
                program: shaders.card.clone(),
                geometry: bbox,
                inv: square_to_quad(normal).inverse(),
                aa: (size.w as f32 / projected_w.max(1.0)).max(1.0),
                shade: (1.0 - 0.11 * depth.max(0.0)).max(0.35),
                alpha,
            }));
        }

        // Behind them, the screen blurred and dimmed.
        let full = geo.size.to_physical_precise_round(scale);
        elements.push(OutputElement::Solid(SolidColorRenderElement::new(
            Id::new(),
            Rectangle::from_size(full),
            CommitCounter::default(),
            Color32F::new(0.0, 0.0, 0.0, 0.35),
            Kind::Unspecified,
        )));
        let mut session = Vec::new();
        crate::render::session_elements(self, output, renderer, scale, Some(shaders), true, &mut session);
        let mut blur = self.switcher.as_mut().map(|s| std::mem::take(&mut s.cache.blur)).unwrap_or_default();
        crate::eyes::backdrop(renderer, &mut blur, shaders, output, geo.size, scale, &session, 0.75, elements);
        if let Some(switcher) = self.switcher.as_mut() {
            switcher.cache.blur = blur;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_square_lands_on_the_quad() {
        let quad = [(10.0, 20.0), (110.0, 30.0), (100.0, 90.0), (5.0, 80.0)];
        let m = square_to_quad(quad);
        for ((u, v), (x, y)) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].into_iter().zip(quad) {
            let p = m * Vec3::new(u, v, 1.0);
            assert!((p.x / p.z - x).abs() < 1e-3 && (p.y / p.z - y).abs() < 1e-3, "({u},{v})");
        }
        // And back.
        let p = m.inverse() * Vec3::new(100.0, 90.0, 1.0);
        assert!((p.x / p.z - 1.0).abs() < 1e-4 && (p.y / p.z - 1.0).abs() < 1e-4);
    }

    #[test]
    fn cards_further_back_are_smaller_and_to_the_right() {
        let front = card_corners(0.0, 1920.0, 1080.0, 800.0, 528.0);
        let back = card_corners(3.0, 1920.0, 1080.0, 800.0, 528.0);
        let height = |c: &[(f32, f32); 4]| c[3].1 - c[0].1;
        assert!(height(&back) < height(&front));
        assert!(back[0].0 > front[0].0);
        // Turned: the right edge is further away, so shorter.
        assert!(front[2].1 - front[1].1 < front[3].1 - front[0].1);
    }

    #[test]
    fn titles_lose_the_program_name() {
        assert_eq!(detail("notes.md — Orbit Edit", "Orbit Edit", "orbit-edit"), "notes.md");
        assert_eq!(detail("Rust - Mozilla Firefox", "Firefox", "firefox"), "Rust - Mozilla Firefox");
        assert_eq!(detail("Firefox", "Firefox", "firefox"), "");
        assert_eq!(detail("~/src", "foot", "foot"), "~/src");
    }
}
