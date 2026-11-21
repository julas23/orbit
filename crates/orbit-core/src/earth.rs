//! The `earth` wallpaper effect: the world map, day where the Sun is and
//! night with the cities' lights where it is not, the line between them
//! moving with the hour; over it the clouds as the weather satellites saw
//! them, fetched every 15 minutes, each new picture fading in over the last.
//!
//! The maps (`/usr/share/orbit/wallpaper/earth-{day,night}.jpg`) are NASA's
//! Blue Marble and Black Marble 2016, in the public domain. The clouds come
//! from earth-live (https://kckbytes.github.io/earth-live/, stitched from
//! five geostationary satellites every 10 minutes: modified EUMETSAT data,
//! NASA GIBS, NOAA, JMA), else Live Cloud Maps
//! (https://clouds.matteason.co.uk/, EUMETSAT data, every 3 hours); the
//! last one is kept in `~/.cache/orbit/clouds.jpg`, so the map starts with
//! clouds, offline too.
//!
//! The screen shows all the latitudes, the longitudes as many as its shape
//! allows (320° on 16:9), the user's own in the middle (`world`), marked
//! with a dot that pulses.
//!
//! One texture holds it all: the day map in its colours with the night map
//! in its alpha, and under it, at half the size, the clouds before (red)
//! and after (green) the last fetch.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use smithay::{
    backend::renderer::{
        Texture,
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        gles::{GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName, UniformType},
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    reexports::calloop::channel::Sender,
    utils::{Buffer, Physical, Rectangle, Scale, Transform},
};

const MAP_W: usize = 4096;
const MAP_H: usize = 2048;
const CLOUD_W: usize = 2048;
const CLOUD_H: usize = 1024;
pub const ATLAS_W: usize = MAP_W;
pub const ATLAS_H: usize = MAP_H + CLOUD_H;

const CLOUDS: [&str; 2] = [
    "https://kckbytes.github.io/earth-live/clouds_2048.jpg",
    "https://clouds.matteason.co.uk/images/2048x1024/clouds.jpg",
];
/// How often the clouds are fetched, and again after a failure.
const EVERY: Duration = Duration::from_secs(15 * 60);
const RETRY: Duration = Duration::from_secs(3 * 60);
/// Seconds a new picture of the clouds takes to fade in.
pub const FADE: f32 = 120.0;

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

// Where the Sun is overhead (latitude, longitude, radians); the longitude
// in the middle of the screen; the screen's width over its height; the
// clouds' fade (0 the older picture, 1 the newer) and whether there are
// any; the user's place (latitude, longitude, known); the clock (seconds).
uniform vec2 sun;
uniform float centre;
uniform float aspect;
uniform float fade_in;
uniform float clouds;
uniform vec3 home;
uniform float time;

const float PI = 3.14159265;
const float ATLAS_MAP = 2.0 / 3.0;

vec3 unit(float lat, float lon) {
    return vec3(cos(lat) * cos(lon), cos(lat) * sin(lon), sin(lat));
}

vec4 map_at(vec2 m) {
    return texture2D(tex, vec2(m.x, clamp(m.y, 0.0005, 0.9995) * ATLAS_MAP));
}

float clouds_at(vec2 m) {
    vec2 at = vec2(clamp(m.x, 0.0003, 0.9997) * 0.5, ATLAS_MAP + clamp(m.y, 0.001, 0.999) / 3.0);
    vec2 two = texture2D(tex, at).rg;
    return mix(two.r, two.g, fade_in) * clouds;
}

void main() {
    float lat = (0.5 - v_coords.y) * PI;
    float lon = centre + (v_coords.x - 0.5) * PI * aspect;
    vec2 m = vec2(fract(lon / (2.0 * PI) + 0.5), v_coords.y);
    // A few taps across a pixel: the lights stay steady as they shrink.
    vec2 px = vec2(0.25 / (aspect * 1024.0), 0.25 / 2048.0);
    vec4 base = 0.25 * (map_at(m + px) + map_at(m - px) + map_at(m + vec2(px.x, -px.y)) + map_at(m + vec2(-px.x, px.y)));
    float cloud = clouds_at(m);

    float sunward = dot(unit(lat, lon), unit(sun.x, sun.y));
    float day = smoothstep(-0.1, 0.1, sunward);
    vec3 lit = base.rgb * (0.55 + 0.45 * smoothstep(-0.1, 0.6, sunward));
    lit = mix(lit, vec3(0.96, 0.97, 1.0) * (0.6 + 0.4 * smoothstep(-0.1, 0.5, sunward)), cloud * 0.92);
    // The night: the land barely there, the cities' lights warm, dimmed
    // under the clouds, the clouds themselves a faint grey.
    float lights = smoothstep(0.16, 0.7, base.a);
    vec3 night = base.rgb * 0.035 + vec3(1.0, 0.72, 0.38) * lights * (1.0 - 0.75 * cloud);
    night += vec3(0.05, 0.06, 0.08) * cloud;
    vec3 c = mix(night, lit, day);
    // Dawn and dusk along the line between them.
    c += vec3(1.0, 0.42, 0.15) * 0.18 * exp(-pow(sunward / 0.05, 2.0)) * (0.4 + 0.6 * (1.0 - cloud));

    // The user's place.
    if (home.z > 0.5) {
        // Its distance in pixels of a 1080-line screen.
        float dlon = mod(home.y - centre + PI, 2.0 * PI) - PI;
        vec2 at = vec2(0.5 + dlon / (PI * aspect), 0.5 - home.x / PI);
        float r = length((v_coords - at) * vec2(aspect, 1.0)) * 1080.0;
        float pulse = fract(time * 0.4);
        c = mix(c, vec3(1.0, 0.35, 0.3), smoothstep(5.0, 3.5, r));
        c += vec3(1.0, 0.4, 0.3) * smoothstep(2.0, 0.0, abs(r - 6.0 - 26.0 * pulse)) * (1.0 - pulse) * 0.8;
    }

    vec4 o = vec4(c * alpha, alpha);
#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        o = vec4(0.0, 0.2, 0.0, 0.2) + o * 0.8;
#endif
    gl_FragColor = o;
}
"#;

pub fn compile(renderer: &mut GlesRenderer) -> Result<GlesTexProgram, GlesError> {
    renderer.compile_custom_texture_shader(
        SHADER,
        &[
            UniformName::new("sun", UniformType::_2f),
            UniformName::new("centre", UniformType::_1f),
            UniformName::new("aspect", UniformType::_1f),
            UniformName::new("fade_in", UniformType::_1f),
            UniformName::new("clouds", UniformType::_1f),
            UniformName::new("home", UniformType::_3f),
            UniformName::new("time", UniformType::_1f),
        ],
    )
}

/// What the map is drawn with, frame to frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub sun: (f32, f32),
    pub centre: f32,
    pub aspect: f32,
    pub fade_in: f32,
    pub clouds: bool,
    pub home: Option<(f32, f32)>,
    pub time: f32,
}

/// The map over one monitor.
#[derive(Clone)]
pub struct EarthElement {
    id: Id,
    commit: CommitCounter,
    texture: GlesTexture,
    program: GlesTexProgram,
    geometry: Rectangle<i32, Physical>,
    look: Look,
    alpha: f32,
}

impl EarthElement {
    pub fn new(
        id: Id,
        commit: CommitCounter,
        texture: GlesTexture,
        program: GlesTexProgram,
        geometry: Rectangle<i32, Physical>,
        look: Look,
        alpha: f32,
    ) -> Self {
        Self {
            id,
            commit,
            texture,
            program,
            geometry,
            look,
            alpha,
        }
    }
}

impl Element for EarthElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size(self.texture.size().to_f64())
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry
    }

    fn damage_since(&self, _scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        if commit == Some(self.commit) {
            DamageSet::default()
        } else {
            DamageSet::from_slice(&[Rectangle::from_size(self.geometry.size)])
        }
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        if self.alpha >= 1.0 {
            OpaqueRegions::from_slice(&[Rectangle::from_size(self.geometry.size)])
        } else {
            OpaqueRegions::default()
        }
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for EarthElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&smithay::utils::user_data::UserDataMap>,
    ) -> Result<(), GlesError> {
        let look = &self.look;
        let (home, known) = look.home.map_or(((0.0, 0.0), 0.0), |h| (h, 1.0));
        let uniforms = [
            Uniform::new("sun", look.sun),
            Uniform::new("centre", look.centre),
            Uniform::new("aspect", look.aspect),
            Uniform::new("fade_in", look.fade_in),
            Uniform::new("clouds", if look.clouds { 1.0f32 } else { 0.0 }),
            Uniform::new("home", (home.0, home.1, known)),
            Uniform::new("time", look.time),
        ];
        frame.render_texture_from_to(
            &self.texture,
            src,
            dst,
            damage,
            &[],
            Transform::Normal,
            self.alpha,
            Some(&self.program),
            &uniforms,
        )
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

/// A new texture's pixels (RGBA, `ATLAS_W`×`ATLAS_H`), from the thread:
/// whether it has clouds, and whether they just changed (to fade in).
pub struct Atlas {
    pub pixels: Vec<u8>,
    pub clouds: bool,
    pub fresh: bool,
}

/// The thread that makes the texture and fetches the clouds; it ends when
/// `alive` goes false (the effect changed).
pub fn start(sender: Sender<Atlas>, alive: Arc<AtomicBool>) {
    let spawned = std::thread::Builder::new()
        .name("orbit-earth".into())
        .spawn(move || run(&sender, &alive));
    if let Err(err) = spawned {
        tracing::error!("earth: {err}");
    }
}

fn run(sender: &Sender<Atlas>, alive: &AtomicBool) {
    let cache = cache_path();
    let mut last = std::fs::read(&cache).ok().and_then(|b| clouds_from(&b));
    let Some(pixels) = atlas(last.as_deref(), last.as_deref()) else {
        return;
    };
    if sender
        .send(Atlas {
            pixels,
            clouds: last.is_some(),
            fresh: false,
        })
        .is_err()
    {
        return;
    }
    // Fetched less than `EVERY` ago (the file's age): not again yet.
    let age = std::fs::metadata(&cache)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok());
    let mut due = Instant::now() + age.map_or(Duration::ZERO, |a| EVERY.saturating_sub(a));
    while alive.load(Ordering::Relaxed) {
        if Instant::now() < due {
            std::thread::sleep(Duration::from_secs(2));
            continue;
        }
        match fetch() {
            Ok((bytes, clouds)) => {
                due = Instant::now() + EVERY;
                if last.as_deref() == Some(&clouds[..]) {
                    continue;
                }
                if let Some(dir) = cache.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&cache, bytes);
                let before = last.take().unwrap_or_else(|| clouds.clone());
                let Some(pixels) = atlas(Some(&before), Some(&clouds)) else {
                    return;
                };
                last = Some(clouds);
                if sender
                    .send(Atlas {
                        pixels,
                        clouds: true,
                        fresh: true,
                    })
                    .is_err()
                {
                    return;
                }
            }
            Err(err) => {
                tracing::warn!("earth: the clouds: {err}");
                due = Instant::now() + RETRY;
            }
        }
    }
}

/// The newest clouds: the file, and the picture as `CLOUD_W`×`CLOUD_H`
/// greys.
fn fetch() -> Result<(Vec<u8>, Vec<u8>), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into();
    let mut errors = Vec::new();
    for url in CLOUDS {
        let got = agent
            .get(url)
            .header("User-Agent", "orbit-core (Orbit desktop wallpaper)")
            .call()
            .and_then(|mut r| r.body_mut().with_config().limit(16 * 1024 * 1024).read_to_vec());
        match got {
            Ok(bytes) => match clouds_from(&bytes) {
                Some(clouds) => return Ok((bytes, clouds)),
                None => errors.push(format!("{url}: not a picture")),
            },
            Err(err) => errors.push(format!("{url}: {err}")),
        }
    }
    Err(errors.join("; "))
}

/// A picture of the clouds (any size, equirectangular) as `CLOUD_W`×`CLOUD_H`
/// greys.
fn clouds_from(bytes: &[u8]) -> Option<Vec<u8>> {
    let picture = image::load_from_memory(bytes).ok()?.into_luma8();
    let sized = image::imageops::resize(
        &picture,
        CLOUD_W as u32,
        CLOUD_H as u32,
        image::imageops::FilterType::Triangle,
    );
    Some(sized.into_raw())
}

/// The whole texture: the maps from their files, the clouds `before` and
/// `after` (none: clear).
fn atlas(before: Option<&[u8]>, after: Option<&[u8]>) -> Option<Vec<u8>> {
    let day = load("earth-day.jpg").map(|p| p.into_rgb8());
    let night = load("earth-night.jpg").map(|p| p.into_luma8());
    let (Some(day), Some(night)) = (day, night) else {
        tracing::error!("earth: the maps (earth-day.jpg, earth-night.jpg) are missing");
        return None;
    };
    let fit = |w: u32, h: u32| (w as usize, h as usize) == (MAP_W, MAP_H);
    let day = if fit(day.width(), day.height()) {
        day
    } else {
        image::imageops::resize(&day, MAP_W as u32, MAP_H as u32, image::imageops::FilterType::Triangle)
    };
    let night = if fit(night.width(), night.height()) {
        night
    } else {
        image::imageops::resize(
            &night,
            MAP_W as u32,
            MAP_H as u32,
            image::imageops::FilterType::Triangle,
        )
    };
    let mut pixels = vec![0u8; ATLAS_W * ATLAS_H * 4];
    for (i, (rgb, l)) in day.pixels().zip(night.pixels()).enumerate() {
        pixels[i * 4..i * 4 + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], l[0]]);
    }
    for y in 0..CLOUD_H {
        for x in 0..CLOUD_W {
            let i = y * CLOUD_W + x;
            let at = ((MAP_H + y) * ATLAS_W + x) * 4;
            let a = before.map_or(0, |c| c[i]);
            let b = after.map_or(a, |c| c[i]);
            pixels[at..at + 4].copy_from_slice(&[a, b, 0, 255]);
        }
    }
    Some(pixels)
}

/// One of the maps, from the first data directory that has it.
fn load(name: &str) -> Option<image::DynamicImage> {
    let home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    let dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    home.into_iter()
        .chain(dirs.split(':').map(PathBuf::from))
        .map(|d| d.join("orbit/wallpaper").join(name))
        .find_map(|p| image::open(p).ok())
}

fn cache_path() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache"))
        .join("orbit/clouds.jpg")
}
