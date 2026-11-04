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
//! - `borealis`: curtains of light over a night sky and a dark ridge, the
//!   northern lights: green at their foot, the later colours higher up.
//! - `stars`: a slow flight through the stars, the Milky Way behind.
//! - `rain`: rain on a window at night, the lights out of focus behind
//!   it; each drop a lens, the trails clearing the fogged glass.
//! - `nebula`: clouds of gas turning slowly round a bright heart, dust
//!   lanes across them, stars behind.
//! - `galaxy`: a spiral galaxy turning, seen a little from the side: a
//!   bright heart, blue arms of young stars with dust along them.
//! - `logo`: Orbit's mark alive: the open orbit turning round the core,
//!   the moon going round the closed one, a glow that breathes with the
//!   bass, stars behind.
//! - `scope`: a spectrum analyser as a sphere, after Winamp's: the
//!   spectrum bends its outline and the two lines crossing inside it; the
//!   louder, the more the lines smoke and blur (sharp in silence), and the
//!   spectra just gone echo outwards, fainter and softer. It listens even
//!   without `wallpaper.sound`.
//!
//! With `wallpaper.sound` the effects follow what the speakers play
//! (`sound.rs`): the bass swells them, the treble makes them glint.
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
    Borealis,
    Stars,
    Rain,
    Nebula,
    Galaxy,
    Logo,
    Scope,
}

impl Effect {
    /// Whether it needs the sound to be itself (it listens without
    /// `wallpaper.sound`).
    pub fn needs_sound(self) -> bool {
        self == Effect::Scope
    }

    /// Every effect, `None` aside.
    pub const ALL: [Effect; 12] = [
        Effect::Aurora,
        Effect::Ink,
        Effect::Facets,
        Effect::Fire,
        Effect::Water,
        Effect::Borealis,
        Effect::Stars,
        Effect::Rain,
        Effect::Nebula,
        Effect::Galaxy,
        Effect::Logo,
        Effect::Scope,
    ];
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
            Effect::Borealis => &[
                [0x03, 0x07, 0x12],
                [0x4a, 0xde, 0x80],
                [0x2d, 0xd4, 0xbf],
                [0x81, 0x8c, 0xf8],
                [0xc0, 0x84, 0xfc],
            ],
            Effect::Stars => &[
                [0x02, 0x03, 0x0a],
                [0x1e, 0x29, 0x4f],
                [0x93, 0xc5, 0xfd],
                [0xfd, 0xe6, 0x8a],
                [0xfc, 0xa5, 0xa5],
            ],
            Effect::Rain => &[
                [0x05, 0x08, 0x12],
                [0x1e, 0x29, 0x3b],
                [0xf5, 0x9e, 0x0b],
                [0xef, 0x44, 0x44],
                [0x38, 0xbd, 0xf8],
            ],
            Effect::Nebula => &[
                [0x03, 0x02, 0x0c],
                [0x3b, 0x07, 0x64],
                [0xbe, 0x18, 0x5d],
                [0xf9, 0x73, 0x16],
                [0xfe, 0xf3, 0xc7],
            ],
            Effect::Galaxy => &[
                [0x01, 0x02, 0x08],
                [0x0b, 0x2a, 0x6f],
                [0x25, 0x63, 0xeb],
                [0x7d, 0xd3, 0xfc],
                [0xff, 0xf4, 0xd6],
            ],
            // The mark's own: the night, the core, the open orbit, the
            // closed one, the moon.
            Effect::Logo => &[
                [0x05, 0x07, 0x0d],
                [0x46, 0x82, 0xb4],
                [0x6e, 0xa8, 0xe0],
                [0xd8, 0xde, 0xe9],
                [0xff, 0x95, 0x00],
            ],
            // The night, then the lines from the low end to the high.
            Effect::Scope => &[
                [0x02, 0x03, 0x08],
                [0x22, 0xd3, 0xee],
                [0x4a, 0xde, 0x80],
                [0xfa, 0xcc, 0x15],
                [0xf4, 0x3f, 0x5e],
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
// What the speakers play, 0..1: the low end, and the high.
uniform float bass;
uniform float treble;
uniform vec3 c0;
uniform vec3 c1;
uniform vec3 c2;
uniform vec3 c3;
uniform vec3 c4;
// The spectrum now (16 bands, low to high, four in each), and six past
// ones, newest first, 8 bands each in two halves (e0, e1 the newest).
uniform vec4 s0;
uniform vec4 s1;
uniform vec4 s2;
uniform vec4 s3;
uniform vec4 e0;
uniform vec4 e1;
uniform vec4 e2;
uniform vec4 e3;
uniform vec4 e4;
uniform vec4 e5;
uniform vec4 e6;
uniform vec4 e7;
uniform vec4 e8;
uniform vec4 e9;
uniform vec4 e10;
uniform vec4 e11;

float pick(vec4 v, float i) {
    if (i < 0.5) return v.x;
    if (i < 1.5) return v.y;
    if (i < 2.5) return v.z;
    return v.w;
}

float band_now(float i) {
    if (i < 4.0) return pick(s0, i);
    if (i < 8.0) return pick(s1, i - 4.0);
    if (i < 12.0) return pick(s2, i - 8.0);
    return pick(s3, i - 12.0);
}

// The spectrum at `x` (0 low … 1 high), smooth between the bands.
float spectrum(float x) {
    float f = clamp(x, 0.0, 1.0) * 15.0;
    float i = floor(f);
    return mix(band_now(i), band_now(min(i + 1.0, 15.0)), smoothstep(0.0, 1.0, f - i));
}

vec4 echo_half(float j) {
    if (j < 0.5) return e0;
    if (j < 1.5) return e1;
    if (j < 2.5) return e2;
    if (j < 3.5) return e3;
    if (j < 4.5) return e4;
    if (j < 5.5) return e5;
    if (j < 6.5) return e6;
    if (j < 7.5) return e7;
    if (j < 8.5) return e8;
    if (j < 9.5) return e9;
    if (j < 10.5) return e10;
    return e11;
}

float echo_band(float k, float i) {
    vec4 v = i < 4.0 ? echo_half(k * 2.0) : echo_half(k * 2.0 + 1.0);
    return pick(v, i < 4.0 ? i : i - 4.0);
}

// Echo `k` (0 the newest) at `x`, smooth between its bands.
float echo(float k, float x) {
    float f = clamp(x, 0.0, 1.0) * 7.0;
    float i = floor(f);
    return mix(echo_band(k, i), echo_band(k, min(i + 1.0, 7.0)), smoothstep(0.0, 1.0, f - i));
}

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

// The colours past the first (the background's), 0..1.
vec3 lit(float t) {
    return band(mix(1.0 / max(count - 1.0, 1.0), 1.0, clamp(t, 0.0, 1.0)));
}

// Fixed stars, one in some cells of `scale` per unit, twinkling.
float stars(vec2 uv, float scale, float rare) {
    vec2 p = uv * scale;
    vec2 id = floor(p);
    float h = hash(id);
    if (h < rare) return 0.0;
    vec2 at = (hash2(id + 1.3) - 0.5) * 0.6;
    float d = length(fract(p) - 0.5 - at);
    float bright = fract(h * 47.3);
    float twinkle = 0.65 + 0.35 * sin(time * (1.5 + 3.0 * fract(h * 91.7)) + h * 60.0);
    return smoothstep(0.1 + 0.14 * bright, 0.0, d) * twinkle * (0.35 + 0.65 * bright) * (1.0 + treble);
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
            float r = 0.35 + 0.12 * sin(t * 1.3 + fk) + 0.1 * bass;
            float d = length(p - centre);
            float w = exp(-(d * d) / (r * r)) + 0.02;
            sum += color_at(fk) * w;
            weight += w;
        }
    }
    vec3 c = sum / weight;
    // A faint veil of light along the blend, as an aurora has.
    float veil = fbm(vec2(p.x * 1.5 + t * 2.0, p.y * 4.0 - t));
    c *= 0.92 + (0.16 + 0.3 * treble) * veil;
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
    c *= 0.75 + (0.45 + 0.4 * bass) * clamp(r.x * r.y * 2.0, 0.0, 1.0);
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
    c *= 0.9 + 0.12 * light * first + (0.08 + 0.3 * treble) * sin(t * 1.7 + h * 6.2831) + 0.15 * bass;
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
    float tongues = (0.55 + 0.45 * fbm(vec2(p.x * 2.2, t * 0.3))) * (1.0 + 0.6 * bass);
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
    vec3 col = mix(water, color_at(count - 1.0), clamp(light * (0.75 + 0.6 * bass), 0.0, 1.0));
    finish(col);
}
"#;

const BOREALIS: &str = r#"
void main() {
    float aspect = size.x / max(size.y, 1.0);
    // Up from the bottom edge.
    vec2 uv = vec2(v_coords.x * aspect, 1.0 - v_coords.y);
    float t = time * 0.07;
    // The night: a little light low down, darkest above; stars.
    vec3 c = mix(c0 * 1.8 + 0.012, c0 * 0.6, smoothstep(0.0, 0.9, uv.y));
    c += vec3(stars(uv, 160.0, 0.93) + stars(uv + 3.1, 70.0, 0.985)) * smoothstep(0.15, 0.5, uv.y) * 0.9;
    vec3 light = vec3(0.0);
    for (int k = 0; k < 3; k++) {
        float fk = float(k);
        // The curtain's foot, folding slowly.
        float foot = 0.4 + 0.09 * fk
            + 0.22 * (fbm(vec2(uv.x * 0.8 + t * (1.0 + 0.35 * fk) + fk * 7.3, t * 0.4 + fk)) - 0.5);
        float above = uv.y - foot;
        // Rays along it, sliding sideways.
        float rays = 0.3 + 0.7 * noise(vec2(uv.x * 26.0 + 5.0 * fbm(uv * 1.8 + t + fk) - t * 4.0, fk * 3.7));
        float glow = smoothstep(-0.015, 0.01, above) * exp(-max(above, 0.0) * (4.2 - fk * 0.9));
        glow += 0.7 * exp(-abs(above) * 70.0);
        glow *= rays * (0.7 + 0.9 * bass);
        // Curtains come and go across the sky.
        glow *= smoothstep(0.22, 0.55, fbm(vec2(uv.x * 0.6 - t * 0.7, fk * 5.0 + t * 0.25)));
        // Their foot the first colour, higher up the later ones.
        light += lit(clamp(max(above, 0.0) * 2.4 + fk * 0.12, 0.0, 1.0)) * glow * (1.0 - fk * 0.22);
    }
    c += light;
    // A dark ridge along the bottom, the sky's glow on its edge.
    float ridge = 0.1 + 0.07 * fbm(vec2(uv.x * 2.5, 3.0)) + 0.015 * noise(vec2(uv.x * 40.0, 1.0));
    float land = smoothstep(ridge + 1.5 / size.y, ridge, uv.y);
    c = mix(c, c0 * 0.35 + light * 0.06, land);
    finish(clamp(c, 0.0, 1.0));
}
"#;

const STARS: &str = r#"
// One depth of the flight: a star in each cell, a flare across the big.
vec3 layer(vec2 p) {
    vec3 c = vec3(0.0);
    vec2 id = floor(p);
    vec2 f = fract(p) - 0.5;
    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec2 o = vec2(float(x), float(y));
            float n = hash(id + o);
            vec2 at = f - o - (hash2(id + o + 0.7) - 0.5);
            float d = length(at);
            float big = fract(n * 345.32);
            float m = 0.01 * big / max(d, 1e-3);
            m += smoothstep(0.88, 1.0, big) * max(0.0, 1.0 - abs(at.x * at.y * 1200.0)) * 0.5;
            m *= smoothstep(0.9, 0.15, d);
            vec3 col = mix(vec3(1.0), lit(fract(n * 2345.2)), 0.55);
            float twinkle = 0.75 + (0.25 + 0.5 * treble) * sin(time * 3.0 + n * 6.2831);
            c += m * col * twinkle;
        }
    }
    return c;
}

void main() {
    float aspect = size.x / max(size.y, 1.0);
    vec2 uv = vec2(v_coords.x * aspect, v_coords.y) - vec2(aspect * 0.5, 0.5);
    float t = time * 0.02;
    vec3 c = c0;
    // The Milky Way, slanting across.
    float off = dot(uv, vec2(0.6, 0.8)) + 0.08 * (fbm(uv * 2.0 + 4.0) - 0.5);
    float milky = exp(-off * off * 14.0) * (0.35 + 0.65 * fbm(uv * 3.5 + vec2(t, -t)));
    c += lit(0.15 + 0.35 * fbm(uv * 2.0 - t)) * milky * 0.3;
    c += vec3(stars(uv, 260.0, 0.9)) * (0.3 + 0.7 * milky) * 0.6;
    // The flight: layers coming nearer, appearing far and fading near.
    for (int i = 0; i < 4; i++) {
        float fi = float(i);
        float depth = fract(fi * 0.25 + time * 0.012);
        float scale = mix(16.0, 1.2, depth);
        float near = smoothstep(0.0, 0.5, depth) * smoothstep(1.0, 0.85, depth);
        c += layer(uv * scale + fi * 37.1) * near * (1.0 + 0.8 * bass);
    }
    finish(clamp(c, 0.0, 1.0));
}
"#;

const RAIN: &str = r#"
// The lights behind the window, out of focus.
vec3 behind(vec2 uv) {
    vec3 c = mix(c0, mix(c0, color_at(1.0), 0.7), smoothstep(0.0, 1.0, uv.y));
    for (int k = 0; k < 2; k++) {
        float fk = float(k);
        float s = 5.0 + 4.0 * fk;
        vec2 p = uv * s + vec2(fk * 13.1, fk * 7.7);
        vec2 id = floor(p);
        float h = hash(id + fk * 7.0);
        vec2 at = (hash2(id) - 0.5) * 0.36;
        float d = length(fract(p) - 0.5 - at);
        float pulse = 0.75 + 0.25 * sin(time * 0.4 + h * 30.0);
        // Most in the lower half, as a street's are.
        float on = step(0.38, h) * (0.4 + 0.6 * smoothstep(0.1, 0.8, uv.y));
        c += lit(fract(h * 13.7)) * smoothstep(0.3, 0.06, d) * on * (0.6 - 0.2 * fk) * pulse * (1.0 + 0.8 * bass);
    }
    return c;
}

// How much of a drop at `p` (its centre the origin, `r` its radii), and
// where on it: -1..1 across.
vec3 drop(vec2 p, vec2 r) {
    vec2 d = p / r;
    float m = smoothstep(1.0, 0.75, length(d));
    return vec3(m, d * m);
}

void main() {
    float aspect = size.x / max(size.y, 1.0);
    // y down, as the drops run.
    vec2 uv = vec2(v_coords.x * aspect, v_coords.y);
    float t = time;
    vec3 acc = vec3(0.0);
    float trail = 0.0;
    // Drops sitting on the glass, coming and drying.
    for (int k = 0; k < 2; k++) {
        float fk = float(k);
        vec2 p = uv * (14.0 + 12.0 * fk) + fk * 5.3;
        vec2 id = floor(p);
        vec2 h = hash2(id + fk);
        float life = fract(t * 0.04 + h.x * 7.0);
        float r = (0.1 + 0.22 * h.y) * smoothstep(0.0, 0.08, life) * smoothstep(1.0, 0.6, life);
        if (hash(id + 3.7 + fk) > 0.55)
            acc += drop(fract(p) - 0.5 - (h - 0.5) * 0.3, vec2(max(r, 1e-3)));
    }
    // Drops running down, each in a column of its own, a wet trail above.
    for (int k = 0; k < 2; k++) {
        float fk = float(k);
        float cols = 6.0 + 5.0 * fk;
        float col = floor(uv.x * cols);
        float n = hash(vec2(col, fk * 17.0));
        if (hash(vec2(col + 0.5, fk * 5.0)) < 0.45) continue;
        float fall = 0.12 + 0.22 * n;
        float y = fract(t * fall / 1.8 + n) * 1.8 - 0.4;
        float x = (col + 0.5 + 0.18 * sin(uv.y * 9.0 + n * 20.0) + 0.1 * sin(uv.y * 23.0 + n * 9.0)) / cols;
        vec2 local = vec2(uv.x - x, uv.y - y) * cols;
        acc += drop(local, vec2(0.16, 0.2));
        float up = -local.y;
        float wet = smoothstep(0.1, 0.03, abs(local.x)) * step(0.0, up) * smoothstep(0.45 * cols, 0.0, up);
        trail = max(trail, wet);
        // Beads left along the trail.
        float bead = floor(up * 1.3);
        float size_ = 0.05 + 0.06 * hash(vec2(col, bead + fk * 31.0));
        acc += drop(vec2(local.x, fract(up * 1.3) - 0.5), vec2(size_)) * wet * step(0.5, hash(vec2(bead, col)));
    }
    float m = clamp(acc.x, 0.0, 1.0);
    vec2 lens = acc.yz;
    // Each drop a lens: what is behind it, upside down.
    vec3 through = behind(uv - lens * 0.08) * 1.25 + 0.015;
    // The glass fogged, but where the drops are and have run.
    vec3 fog = mix(behind(uv), lit(0.0) * 0.25 + c0 + 0.03, 0.3);
    vec3 c = mix(fog, through, clamp(m + trail * 0.85, 0.0, 1.0));
    // A rim, and a glint at the top of each drop.
    c *= 1.0 - 0.15 * m * smoothstep(0.6, 1.0, length(lens));
    c += m * smoothstep(0.4, 0.95, -lens.y) * (0.3 + 0.4 * treble);
    finish(clamp(c, 0.0, 1.0));
}
"#;

const NEBULA: &str = r#"
void main() {
    float aspect = size.x / max(size.y, 1.0);
    vec2 uv = vec2(v_coords.x * aspect, v_coords.y);
    vec2 p = (uv - vec2(aspect * 0.5, 0.5)) * 2.0;
    float t = time * 0.025;
    // The whole cloud turning, slowly.
    float a = t * 0.4;
    vec2 q = mat2(cos(a), -sin(a), sin(a), cos(a)) * p;
    vec2 w = vec2(fbm(q * 1.2 + t), fbm(q * 1.2 - t + 3.1));
    float gas = fbm(q * 1.7 + w * 1.7 + vec2(t * 0.5, 0.0));
    float dust = fbm(q * 3.4 - w * 2.0 + 7.0);
    // Denser towards the heart.
    float heart = exp(-dot(p, p) * 0.4);
    float dense = smoothstep(0.25, 0.85, gas) * (0.35 + 0.85 * heart);
    vec3 c = c0;
    c += vec3(stars(uv, 240.0, 0.9) + stars(uv + 7.7, 90.0, 0.97)) * (1.0 - dense * 0.8) * 0.8;
    c += lit(clamp(gas * 1.3 - 0.25 + w.x * 0.35, 0.0, 1.0)) * dense * (1.0 + 0.4 * bass);
    // Dark lanes of dust across it.
    c *= 1.0 - 0.6 * smoothstep(0.5, 0.78, dust) * dense;
    // The heart's glow.
    c += color_at(count - 1.0) * pow(heart, 3.0) * (0.22 + 0.3 * bass);
    c += color_at(count - 1.0) * pow(max(gas - 0.55, 0.0) * 2.2, 3.0) * (0.25 + 0.6 * treble);
    finish(clamp(c, 0.0, 1.0));
}
"#;

const GALAXY: &str = r#"
void main() {
    float aspect = size.x / max(size.y, 1.0);
    vec2 uv = vec2(v_coords.x * aspect, v_coords.y);
    vec2 p = (uv - vec2(aspect * 0.5, 0.5)) * 2.6;
    float t = time * 0.035;
    // Seen a little from the side: the disc tilted and squashed.
    float tilt = -0.35;
    p = mat2(cos(tilt), -sin(tilt), sin(tilt), cos(tilt)) * p;
    p.y *= 1.75;
    float r = length(p);
    float a = atan(p.y, p.x);
    // The stars' own frame, turning with the arms.
    vec2 q = mat2(cos(t), -sin(t), sin(t), cos(t)) * p;
    // Arms on a logarithmic spiral, bent here and there, the pattern turning.
    float bend = (fbm(q * 1.4 + 11.0) - 0.5) * 1.3;
    float phase = a + log(r + 0.05) * 3.4 - t + bend;
    // Two strong arms, two fainter between them, none the same all along.
    float major = pow(0.5 + 0.5 * cos(2.0 * phase), 3.0);
    float minor = pow(0.5 + 0.5 * cos(2.0 * phase + 3.1416 + bend), 5.0) * 0.6;
    float arms = max(major, minor) * (0.45 + 0.8 * fbm(q * 2.2 - 5.0));
    float clumps = fbm(q * 5.0 + 3.0);
    float fine = fbm(q * 14.0 - 7.0);
    // Most of it gathered round the heart.
    float disc = exp(-r * 2.6);
    float glow = disc * (0.18 + 1.1 * arms * (0.5 + 0.9 * clumps)) * (1.0 + 0.5 * bass);
    vec3 c = c0;
    c += vec3(stars(uv, 250.0, 0.9) + stars(uv + 4.4, 80.0, 0.975)) * (1.0 - clamp(glow * 2.0, 0.0, 1.0)) * 0.8;
    // The arms blue, bluer and brighter where they are thick.
    c += lit(clamp(0.15 + arms * 0.6 * clumps + fine * 0.15, 0.0, 0.85)) * glow * 1.5;
    // Dust along the inner edge of each arm.
    float dust = pow(0.5 + 0.5 * cos(2.0 * (phase + 0.45)), 6.0) * smoothstep(0.45, 0.7, clumps);
    c *= 1.0 - 0.6 * dust * smoothstep(1.4, 0.15, r);
    // Bright knots: clusters of new stars on the arms.
    float knots = smoothstep(0.7, 0.85, fine) * arms * disc;
    c += lit(0.75) * knots * (1.4 + 1.5 * treble);
    // Suns along the arms: bright points with a glow, and their grain.
    float reach = smoothstep(2.2, 0.25, r);
    float suns = stars(q * 0.5 + 2.0, 70.0, 0.55) * 2.2 + stars(q * 0.5 + 6.0, 140.0, 0.5) * 1.2;
    c += mix(lit(0.85), vec3(1.0), 0.5) * suns * smoothstep(0.08, 0.5, arms) * reach * (1.0 + treble);
    c += lit(0.8) * stars(q * 0.5 + 9.0, 300.0, 0.5) * arms * reach;
    // The heart: warm, white at the centre, a halo round it.
    vec3 heart = color_at(count - 1.0);
    c += heart * exp(-r * r * 40.0) * 1.0;
    c += mix(lit(0.6), heart, 0.6) * exp(-r * 4.0) * 0.4 * (1.0 + 0.4 * bass);
    finish(clamp(c, 0.0, 1.0));
}
"#;

const LOGO: &str = r#"
// The mark is drawn in its own units (svg/brand/orbit-mark.svg: 100 × 100,
// centred on 50, 50), 100 of them across 55% of the screen's height.
float stroke(float d, float half_width, float aa) {
    return smoothstep(half_width + aa, half_width - aa, d);
}

void main() {
    float scale = 100.0 / (0.55 * min(size.x, size.y));
    vec2 p = (v_coords * size - size * 0.5) * scale;
    float aa = scale * 1.2;
    float t = time;
    float r = length(p);
    float a = atan(p.y, p.x);

    // The night: darker at the edges, a few stars, a halo round the mark.
    vec2 uv = v_coords * size / min(size.x, size.y);
    vec3 c = c0 * (1.0 - 0.35 * smoothstep(40.0, 140.0, r));
    c += vec3(stars(uv, 120.0, 0.93) + stars(uv + 3.7, 45.0, 0.985)) * 0.6 * smoothstep(42.0, 70.0, r);
    float breath = 0.5 + 0.5 * sin(t * 0.7);
    c += c1 * exp(-r * r / 2200.0) * (0.10 + 0.06 * breath + 0.35 * bass);

    // The closed orbit.
    float ring = stroke(abs(r - 38.0), 3.0, aa);
    c = mix(c, c3, ring);

    // The open orbit: two thirds of a circle (100 of 150.8 in the mark),
    // with round ends, turning.
    float span = 100.0 / 150.8 * 6.2832;
    float start = radians(200.0) + t * 0.25;
    float rel = mod(a - start, 6.2832);
    float d_arc = rel < span ? abs(r - 24.0) : 1e3;
    vec2 end0 = 24.0 * vec2(cos(start), sin(start));
    vec2 end1 = 24.0 * vec2(cos(start + span), sin(start + span));
    d_arc = min(d_arc, min(length(p - end0), length(p - end1)));
    float arc = stroke(d_arc, 3.0, aa);
    c = mix(c, c2, arc);
    // A light running along it.
    float run = pow(0.5 + 0.5 * cos(rel * 2.0 - t * 1.5), 8.0) * arc;
    c += mix(c2, vec3(1.0), 0.5) * run * (0.25 + 0.5 * treble);

    // The core, its glow breathing.
    float core = stroke(r, 10.0, aa);
    c += c1 * exp(-r * r / 300.0) * (0.25 + 0.2 * breath + 0.6 * bass);
    c = mix(c, c1 * (1.0 + 0.25 * bass), core);

    // The moon on the closed orbit (where the mark has it, then round).
    float m_angle = atan(34.4, 16.1) - t * 0.18;
    vec2 moon = 38.0 * vec2(cos(m_angle), sin(m_angle));
    float dm = length(p - moon);
    c += c4 * exp(-dm * dm / 90.0) * (0.35 + 0.5 * treble);
    c = mix(c, c4, stroke(dm, 6.5, aa));
    finish(clamp(c, 0.0, 1.0));
}
"#;

const SCOPE: &str = r#"
// A line `d` away, `w` wide: a sharp core when thin, a soft smoke when wide.
float glow(float d, float w) {
    return exp(-(d * d) / (w * w)) * (0.004 / max(w, 0.004) + 0.15);
}

// How much of echo `k` there is: none for a silent one.
float echo_energy(float k) {
    return clamp((echo(k, 0.0) + echo(k, 0.3) + echo(k, 0.6) + echo(k, 1.0)) * 0.6, 0.0, 1.0);
}

void main() {
    float aspect = size.x / max(size.y, 1.0);
    vec2 p = (v_coords - 0.5) * vec2(aspect, 1.0);
    float t = time;
    // How loud, 0..1: silence keeps every line sharp.
    float loud = clamp((s0.x + s0.y + s0.z + s0.w + s1.x + s1.y + s1.z + s1.w
                       + s2.x + s2.y + s2.z + s2.w + s3.x + s3.y + s3.z + s3.w) / 9.0, 0.0, 1.0);
    // The smoke: the place each line is looked for, stirred.
    vec2 stir = vec2(fbm(p * 4.0 + vec2(t * 0.35, 0.0)), fbm(p * 4.0 + vec2(3.1, -t * 0.3))) - 0.5;
    vec2 q = p + stir * 0.05 * loud;
    float r = length(q);
    float a = atan(q.y, q.x);
    // The low end on the left, the high on the right, the same above and
    // below.
    float x = 1.0 - abs(a) / 3.1416;
    float base = 0.26;
    float width = 0.0018 + 0.02 * loud;

    vec3 c = c0;
    // The echoes: the spectra just gone, outwards, fainter and softer,
    // each turned a little, as the trail of a beam.
    for (int k = 5; k >= 0; k--) {
        float fk = float(k);
        float grow = 1.0 + 0.13 * (fk + 1.0);
        float turn = 0.06 * (fk + 1.0);
        vec2 e = mat2(cos(turn), -sin(turn), sin(turn), cos(turn)) * q;
        float ex = 1.0 - abs(atan(e.y, e.x)) / 3.1416;
        float er = base * grow + 0.12 * echo(fk, ex) * grow;
        float fadek = 1.0 - fk / 6.5;
        float ew = 0.002 + 0.012 * loud + 0.0025 * fk;
        c += lit(ex * 0.8 + fk * 0.04) * glow(length(e) - er, ew) * 0.9 * fadek * fadek * echo_energy(fk);
    }

    // The sphere's outline, bent by the spectrum now.
    float level = spectrum(x);
    float outline = base + 0.11 * level;
    c += lit(x) * glow(r - outline, width) * 0.9;
    // A faint shading inside, as a sphere has.
    float inside = smoothstep(outline, outline - 0.02, r);
    c += lit(0.2) * inside * (0.035 + 0.05 * loud) * (1.0 - r / max(outline, 0.01));

    // Two lines crossing inside, each an oscilloscope of the spectrum,
    // fading towards the outline.
    float within = smoothstep(outline * 0.97, outline * 0.7, r);
    float along_h = clamp(q.x / base * 0.5 + 0.5, 0.0, 1.0);
    float along_v = clamp(q.y / base * 0.5 + 0.5, 0.0, 1.0);
    float wave_h = 0.07 * spectrum(abs(along_h - 0.5) * 2.0) * sin(along_h * 31.4 + t * 5.0);
    float wave_v = 0.07 * spectrum(abs(along_v - 0.5) * 2.0) * sin(along_v * 31.4 - t * 4.3);
    c += lit(0.35 + 0.5 * along_h) * glow(q.y - wave_h * within, width) * within * 0.8;
    c += lit(0.85 - 0.5 * along_v) * glow(q.x - wave_v * within, width) * within * 0.8;
    // Their echoes, turned a little more each time.
    for (int k = 0; k < 3; k++) {
        float fk = float(k) + 1.0;
        float turn = 0.07 * fk * (0.3 + loud);
        vec2 e = mat2(cos(turn), -sin(turn), sin(turn), cos(turn)) * q;
        float ew = width * (1.5 + fk);
        float strength = echo_energy(fk - 1.0) * 0.3 / fk;
        float eh = glow(e.y - 0.07 * echo(fk - 1.0, abs(e.x / base)) * sin(e.x / base * 15.7 + t * 5.0 - fk), ew);
        float ev = glow(e.x - 0.07 * echo(fk - 1.0, abs(e.y / base)) * sin(e.y / base * 15.7 - t * 4.3 - fk), ew);
        c += lit(0.6) * (eh + ev) * within * strength;
    }
    // The heart where they cross.
    c += lit(1.0) * exp(-r * r / 0.0009) * (0.2 + 0.8 * bass);
    finish(clamp(c, 0.0, 1.0));
}
"#;

/// The programs; one that did not compile is missing.
#[derive(Clone, Default)]
pub struct Programs(std::collections::HashMap<Effect, GlesPixelProgram>);

impl Programs {
    pub fn compile(renderer: &mut GlesRenderer) -> Self {
        let mut programs = std::collections::HashMap::new();
        for effect in Effect::ALL {
            match compile(renderer, effect.source()) {
                Ok(program) => {
                    programs.insert(effect, program);
                }
                Err(err) => tracing::error!("wallpaper effect {effect:?}: {err:?}"),
            }
        }
        Self(programs)
    }

    pub fn get(&self, effect: Effect) -> Option<&GlesPixelProgram> {
        self.0.get(&effect)
    }
}

impl Effect {
    /// Its pixel shader, after `HEADER`.
    fn source(self) -> &'static str {
        match self {
            Effect::None => "",
            Effect::Aurora => AURORA,
            Effect::Ink => INK,
            Effect::Facets => FACETS,
            Effect::Fire => FIRE,
            Effect::Water => WATER,
            Effect::Borealis => BOREALIS,
            Effect::Stars => STARS,
            Effect::Rain => RAIN,
            Effect::Nebula => NEBULA,
            Effect::Galaxy => GALAXY,
            Effect::Logo => LOGO,
            Effect::Scope => SCOPE,
        }
    }
}

fn compile(renderer: &mut GlesRenderer, body: &str) -> Result<GlesPixelProgram, GlesError> {
    let mut uniforms = vec![
        UniformName::new("time", UniformType::_1f),
        UniformName::new("fade", UniformType::_1f),
        UniformName::new("count", UniformType::_1f),
        UniformName::new("bass", UniformType::_1f),
        UniformName::new("treble", UniformType::_1f),
    ];
    for name in ["c0", "c1", "c2", "c3", "c4"] {
        uniforms.push(UniformName::new(name, UniformType::_3f));
    }
    for name in SPECTRUM.iter().chain(&ECHOES) {
        uniforms.push(UniformName::new(*name, UniformType::_4f));
    }
    renderer.compile_custom_pixel_shader(format!("{HEADER}{body}"), &uniforms)
}

const SPECTRUM: [&str; 4] = ["s0", "s1", "s2", "s3"];
const ECHOES: [&str; 12] = ["e0", "e1", "e2", "e3", "e4", "e5", "e6", "e7", "e8", "e9", "e10", "e11"];

/// The uniforms of a frame: the time in seconds, how much it shows, the
/// sound, and the colours (the last repeated up to `MOST`).
pub fn uniforms(time: f32, fade: f32, sound: &crate::sound::Levels, colors: &[[u8; 3]]) -> Vec<Uniform<'static>> {
    let quad = |v: &[f32]| UniformValue::_4f(v[0], v[1], v[2], v[3]);
    let mut list = vec![
        Uniform::new("time", time),
        Uniform::new("fade", fade),
        Uniform::new("bass", sound.bass),
        Uniform::new("treble", sound.treble),
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
    for (name, four) in SPECTRUM.into_iter().zip(sound.bands.chunks_exact(4)) {
        list.push(Uniform::new(name, quad(four)));
    }
    let halves = sound.echoes.iter().flat_map(|e| e.chunks_exact(4));
    for (name, four) in ECHOES.into_iter().zip(halves) {
        list.push(Uniform::new(name, quad(four)));
    }
    list
}
