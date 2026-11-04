//! Wallpapers drawn from colours, in motion, on the GPU: `wallpaper.effect`
//! in core.yaml.
//!
//! - `aurora`: a few soft blots of colour that blend into one another and
//!   drift slowly (a mesh gradient).
//! - `ink`: noise folded on itself (domain warping), the colours flowing
//!   like ink in water, or lava.
//! - `facets`: a mosaic of cells (Voronoi) shaded like cut stone, the
//!   colours running across the screen, the cells shifting and glinting.
//! - `fire`: flames rising from the bottom edge, the colours from the
//!   coolest (the dark above them) to the hottest (at their root): red and
//!   yellow by default, blue and white with those colours.
//! - `water`: light through moving water (caustics), seen from below.
//!
//! Each is one pixel shader over the monitor; the colours are
//! `wallpaper.colors` (two to five), else the effect's own. A timer draws
//! a new frame `fps` times a second while the effect can be seen; with
//! the effects off (`effects: off`, or on battery with `auto`), under a
//! fullscreen window, or locked behind the greet, time stops and so do
//! the frames.

use smithay::backend::renderer::gles::{
    GlesError, GlesPixelProgram, GlesRenderer, Uniform, UniformName, UniformType, UniformValue,
};

use serde::Deserialize;

/// Which effect, or none (the picture, or the colour).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    #[default]
    None,
    Aurora,
    Ink,
    Facets,
    Fire,
    Water,
}

impl Effect {
    /// The colours it has without `wallpaper.colors`.
    pub fn default_colors(self) -> &'static [[u8; 3]] {
        match self {
            Effect::None => &[],
            Effect::Aurora => &[
                [0x0b, 0x10, 0x26],
                [0x1d, 0x4e, 0x89],
                [0x14, 0xb8, 0xa6],
                [0x7c, 0x3a, 0xed],
                [0xdb, 0x27, 0x77],
            ],
            Effect::Ink => &[
                [0x07, 0x0a, 0x14],
                [0x1e, 0x3a, 0x8a],
                [0xf9, 0x73, 0x16],
                [0xfd, 0xe6, 0x8a],
            ],
            Effect::Facets => &[
                [0x0f, 0x17, 0x2a],
                [0x33, 0x41, 0x55],
                [0x46, 0x82, 0xb4],
                [0x8b, 0x5c, 0xf6],
            ],
            Effect::Fire => &[
                [0x05, 0x02, 0x02],
                [0x7f, 0x1d, 0x1d],
                [0xdc, 0x26, 0x26],
                [0xf9, 0x73, 0x16],
                [0xfd, 0xe0, 0x47],
            ],
            Effect::Water => &[
                [0x02, 0x15, 0x26],
                [0x03, 0x34, 0x6e],
                [0x0e, 0x74, 0x90],
                [0x67, 0xe8, 0xf9],
                [0xec, 0xfe, 0xff],
            ],
        }
    }
}

/// The most colours an effect takes.
pub const MOST: usize = 5;

/// What every effect starts with: smithay's uniforms, the time, the
/// colours, and a dither against banding in the soft gradients.
const HEADER: &str = r#"
precision highp float;
uniform vec2 size;
uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

uniform float time;
uniform float fade;
uniform float count;
uniform vec3 c0;
uniform vec3 c1;
uniform vec3 c2;
uniform vec3 c3;
uniform vec3 c4;

vec3 color_at(float i) {
    if (i < 0.5) return c0;
    if (i < 1.5) return c1;
    if (i < 2.5) return c2;
    if (i < 3.5) return c3;
    return c4;
}

// The colours as one smooth band, 0..1 from the first to the last.
vec3 band(float t) {
    float f = clamp(t, 0.0, 1.0) * (count - 1.0);
    float i = floor(f);
    return mix(color_at(i), color_at(min(i + 1.0, count - 1.0)), smoothstep(0.0, 1.0, f - i));
}

float hash(vec2 p) {
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
}

vec2 hash2(vec2 p) {
    float h = hash(p);
    return vec2(h, hash(p + h + 17.17));
}

float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x),
               mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x), u.y);
}

float fbm(vec2 p) {
    float v = 0.0;
    float a = 0.5;
    for (int i = 0; i < 4; i++) {
        v += a * noise(p);
        p = mat2(1.6, 1.2, -1.2, 1.6) * p;
        a *= 0.5;
    }
    return v;
}

void finish(vec3 c) {
    // A grain finer than a step of 8 bits: no bands in the gradients.
    c += (hash(v_coords * size + fract(time)) - 0.5) / 255.0;
    float a = alpha * fade;
    vec4 o = vec4(c * a, a);
#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        o = vec4(0.0, 0.2, 0.0, 0.2) + o * 0.8;
#endif
    gl_FragColor = o;
}
"#;

const AURORA: &str = r#"
void main() {
    float aspect = size.x / max(size.y, 1.0);
    vec2 p = vec2(v_coords.x * aspect, v_coords.y);
    float t = time * 0.06;
    vec3 sum = vec3(0.0);
    float weight = 0.0;
    for (int k = 0; k < 5; k++) {
        float fk = float(k);
        if (fk < count) {
            // Each blot on its own slow Lissajous path.
            vec2 centre = vec2(
                aspect * (0.5 + 0.42 * sin(t * (0.9 + 0.23 * fk) + fk * 2.1)),
                0.5 + 0.42 * cos(t * (0.7 + 0.31 * fk) + fk * 1.3));
            float r = 0.35 + 0.12 * sin(t * 1.3 + fk);
            float d = length(p - centre);
            float w = exp(-(d * d) / (r * r)) + 0.02;
            sum += color_at(fk) * w;
            weight += w;
        }
    }
    vec3 c = sum / weight;
    // A faint veil of light along the blend, as an aurora has.
    float veil = fbm(vec2(p.x * 1.5 + t * 2.0, p.y * 4.0 - t));
    c *= 0.92 + 0.16 * veil;
    finish(c);
}
"#;

const INK: &str = r#"
void main() {
    float aspect = size.x / max(size.y, 1.0);
    vec2 p = vec2(v_coords.x * aspect, v_coords.y) * 2.2;
    float t = time * 0.05;
    vec2 q = vec2(fbm(p + vec2(0.0, t)), fbm(p + vec2(5.2, 1.3) - t));
    vec2 r = vec2(fbm(p + 4.0 * q + vec2(1.7, 9.2) + 1.5 * t),
                  fbm(p + 4.0 * q + vec2(8.3, 2.8) - 1.2 * t));
    float f = fbm(p + 3.5 * r);
    vec3 c = band(smoothstep(0.15, 0.95, f * 1.1 + 0.25 * length(q) - 0.1));
    // Darker in the folds, brighter where the ink runs thin.
    c *= 0.75 + 0.45 * clamp(r.x * r.y * 2.0, 0.0, 1.0);
    finish(c);
}
"#;

const FACETS: &str = r#"
void main() {
    float aspect = size.x / max(size.y, 1.0);
    vec2 p = vec2(v_coords.x * aspect, v_coords.y) * 7.0;
    float t = time * 0.15;
    vec2 cell = floor(p);
    float first = 8.0;
    float second = 8.0;
    vec2 near = vec2(0.0);
    vec2 near_cell = vec2(0.0);
    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec2 c = cell + vec2(float(x), float(y));
            vec2 h = hash2(c);
            vec2 point = c + 0.5 + 0.4 * sin(t + 6.2831 * h);
            float d = length(p - point);
            if (d < first) {
                second = first;
                first = d;
                near = point;
                near_cell = c;
            } else if (d < second) {
                second = d;
            }
        }
    }
    float h = hash(near_cell);
    // The colours run across the screen, each cell a little off.
    float along = (near.x / (7.0 * aspect)) * 0.6 + (1.0 - near.y / 7.0) * 0.4;
    vec3 c = band(clamp(along + (h - 0.5) * 0.18 + 0.06 * sin(t * 0.4 + along * 3.0), 0.0, 1.0));
    // Cut like stone: lit from the top left, each cell glinting in turn.
    vec2 from = normalize(p - near + 1e-4);
    float light = dot(from, normalize(vec2(-0.6, -0.8)));
    c *= 0.9 + 0.12 * light * first + 0.08 * sin(t * 1.7 + h * 6.2831);
    // A fine dark seam between cells.
    c *= mix(0.82, 1.0, smoothstep(0.0, 0.06, second - first));
    finish(c);
}
"#;

const FIRE: &str = r#"
void main() {
    float aspect = size.x / max(size.y, 1.0);
    // Up from the bottom edge.
    float y = 1.0 - v_coords.y;
    vec2 p = vec2(v_coords.x * aspect, y);
    float t = time * 0.45;
    // Turbulence rising, bent by a slower turbulence.
    vec2 q = vec2(p.x * 3.0, p.y * 2.0 - t * 1.6);
    float bend = fbm(q * 0.8 + vec2(0.0, -t * 0.5));
    float n = fbm(q * 1.6 + vec2(bend * 1.8, 0.0));
    // Tongues of flame: tall where the slow noise says, low between.
    float tongues = 0.55 + 0.45 * fbm(vec2(p.x * 2.2, t * 0.3));
    // At the root from half heat to white heat, as the noise says; cooler
    // as it rises.
    float heat = clamp(0.2 + n * 1.0 - y * 1.05 / tongues, 0.0, 1.0);
    vec3 c = band(heat);
    // Embers' glow at the root.
    c += color_at(count - 1.0) * 0.12 * smoothstep(0.25, 0.0, y);
    finish(c);
}
"#;

const WATER: &str = r#"
void main() {
    float aspect = size.x / max(size.y, 1.0);
    vec2 uv = vec2(v_coords.x * aspect, v_coords.y);
    float t = time * 0.25 + 23.0;
    // Light through moving water, the classic tileable caustic.
    vec2 p = mod(uv * 6.28318 * 0.8, 6.28318) - 250.0;
    vec2 i = p;
    float c = 1.0;
    float inten = 0.005;
    for (int n = 0; n < 5; n++) {
        float tt = t * (1.0 - (3.5 / float(n + 1)));
        i = p + vec2(cos(tt - i.x) + sin(tt + i.y), sin(tt - i.y) + cos(tt + i.x));
        c += 1.0 / length(vec2(p.x / (sin(i.x + tt) / inten), p.y / (cos(i.y + tt) / inten)));
    }
    c /= 5.0;
    c = 1.17 - pow(c, 1.4);
    float light = clamp(pow(abs(c), 8.0), 0.0, 1.0);
    // Deeper, darker, towards the bottom; the light on top.
    float depth = 1.0 - v_coords.y;
    vec3 water = band(0.15 + 0.45 * (1.0 - depth) * (0.8 + 0.2 * fbm(uv * 3.0 + t * 0.2)));
    vec3 col = mix(water, color_at(count - 1.0), light * 0.75);
    finish(col);
}
"#;

/// The programs; one that did not compile is missing.
#[derive(Clone, Default)]
pub struct Programs {
    aurora: Option<GlesPixelProgram>,
    ink: Option<GlesPixelProgram>,
    facets: Option<GlesPixelProgram>,
    fire: Option<GlesPixelProgram>,
    water: Option<GlesPixelProgram>,
}

impl Programs {
    pub fn compile(renderer: &mut GlesRenderer) -> Self {
        let mut one = |name: &str, body: &str| -> Option<GlesPixelProgram> {
            compile(renderer, body)
                .inspect_err(|err| tracing::error!("wallpaper effect {name}: {err:?}"))
                .ok()
        };
        Self {
            aurora: one("aurora", AURORA),
            ink: one("ink", INK),
            facets: one("facets", FACETS),
            fire: one("fire", FIRE),
            water: one("water", WATER),
        }
    }

    pub fn get(&self, effect: Effect) -> Option<&GlesPixelProgram> {
        match effect {
            Effect::None => None,
            Effect::Aurora => self.aurora.as_ref(),
            Effect::Ink => self.ink.as_ref(),
            Effect::Facets => self.facets.as_ref(),
            Effect::Fire => self.fire.as_ref(),
            Effect::Water => self.water.as_ref(),
        }
    }
}

fn compile(renderer: &mut GlesRenderer, body: &str) -> Result<GlesPixelProgram, GlesError> {
    let mut uniforms = vec![
        UniformName::new("time", UniformType::_1f),
        UniformName::new("fade", UniformType::_1f),
        UniformName::new("count", UniformType::_1f),
    ];
    for name in ["c0", "c1", "c2", "c3", "c4"] {
        uniforms.push(UniformName::new(name, UniformType::_3f));
    }
    renderer.compile_custom_pixel_shader(format!("{HEADER}{body}"), &uniforms)
}

/// The uniforms of a frame: the time in seconds, how much it shows, and
/// the colours (the last repeated up to `MOST`).
pub fn uniforms(time: f32, fade: f32, colors: &[[u8; 3]]) -> Vec<Uniform<'static>> {
    let mut list = vec![
        Uniform::new("time", time),
        Uniform::new("fade", fade),
        Uniform::new("count", colors.len().clamp(1, MOST) as f32),
    ];
    for (i, name) in ["c0", "c1", "c2", "c3", "c4"].into_iter().enumerate() {
        let [r, g, b] = colors
            .get(i)
            .or(colors.last())
            .copied()
            .unwrap_or([0, 0, 0])
            .map(|c| c as f32 / 255.0);
        list.push(Uniform::new(name, UniformValue::_3f(r, g, b)));
    }
    list
}
