//! Opening and closing the eyes (PLANO-CORE §7.1 item 3, §7.3 step 2):
//! while locking or unlocking, the session is drawn into a texture off
//! screen, blurred, and covered by black and the greet, all three on the
//! same curve — the session blurs and fades out as the eyes close, and
//! comes into focus as they open. Only during the transition: otherwise
//! nothing of this is drawn, and locked the session never is.
//!
//! The blur is a dual Kawase (Marius Bjørge, "Bandwidth-Efficient Rendering",
//! SIGGRAPH 2015): each pass down halves the picture, sampling around each
//! pixel; passes up blur it back. Its strength moves smoothly by blending
//! two neighbouring depths: depth 0 is the sharp picture, depth k ends k
//! halvings down, back up at half size.

use std::{collections::HashMap, time::Duration};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Color32F, Frame, Offscreen, Renderer, Texture,
            element::{Id, Kind, solid::SolidColorRenderElement, texture::TextureRenderElement},
            gles::{GlesError, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName, UniformType, UniformValue},
            utils::{CommitCounter, draw_render_elements},
        },
    },
    output::Output,
    utils::{Buffer, Physical, Rectangle, Scale, Size, Transform},
};

use crate::render::OutputElement;

/// How long the eyes take to open or close.
pub const DURATION: Duration = Duration::from_millis(400);
/// The deepest blur, closed: halvings down.
const DEPTH: usize = 4;
/// How far around each pixel the passes sample, in texels.
const OFFSET: f32 = 2.0;

const DOWN: &str = r#"
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

// Half a texel of the source, and how many of them to reach.
uniform vec2 halfpixel;
uniform float offset;

void main() {
    vec2 d = halfpixel * offset;
    vec4 sum = texture2D(tex, v_coords) * 4.0;
    sum += texture2D(tex, v_coords - d);
    sum += texture2D(tex, v_coords + d);
    sum += texture2D(tex, v_coords + vec2(d.x, -d.y));
    sum += texture2D(tex, v_coords - vec2(d.x, -d.y));
    vec4 color = sum / 8.0;
#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif
    gl_FragColor = color * alpha;
}
"#;

const UP: &str = r#"
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

uniform vec2 halfpixel;
uniform float offset;

void main() {
    vec2 d = halfpixel * offset;
    vec4 sum = texture2D(tex, v_coords + vec2(-d.x * 2.0, 0.0));
    sum += texture2D(tex, v_coords + vec2(-d.x, d.y)) * 2.0;
    sum += texture2D(tex, v_coords + vec2(0.0, d.y * 2.0));
    sum += texture2D(tex, v_coords + vec2(d.x, d.y)) * 2.0;
    sum += texture2D(tex, v_coords + vec2(d.x * 2.0, 0.0));
    sum += texture2D(tex, v_coords + vec2(d.x, -d.y)) * 2.0;
    sum += texture2D(tex, v_coords + vec2(0.0, -d.y * 2.0));
    sum += texture2D(tex, v_coords + vec2(-d.x, -d.y)) * 2.0;
    vec4 color = sum / 12.0;
#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif
    gl_FragColor = color * alpha;
}
"#;

/// The passes down and up.
pub fn compile(renderer: &mut GlesRenderer) -> Result<(GlesTexProgram, GlesTexProgram), GlesError> {
    let uniforms = [UniformName::new("halfpixel", UniformType::_2f), UniformName::new("offset", UniformType::_1f)];
    Ok((renderer.compile_custom_texture_shader(DOWN, &uniforms)?, renderer.compile_custom_texture_shader(UP, &uniforms)?))
}

/// The curve the eyes move on: slow at both ends.
pub fn ease(p: f32) -> f32 {
    let p = p.clamp(0.0, 1.0);
    if p < 0.5 { 4.0 * p * p * p } else { 1.0 - (-2.0 * p + 2.0).powi(3) / 2.0 }
}

/// The textures of the transition, kept from frame to frame while it
/// lasts: by monitor, chain (0 the picture and the passes down, 1 and 2
/// the two depths' passes up) and halvings.
#[derive(Default)]
pub struct Cache(HashMap<(String, u8, usize), GlesTexture>);

impl Cache {
    /// The texture for `key`, of `size`; made when missing or resized.
    fn target(&mut self, renderer: &mut GlesRenderer, key: (String, u8, usize), size: Size<i32, Buffer>) -> Result<GlesTexture, GlesError> {
        if let Some(texture) = self.0.get(&key).filter(|t| t.size() == size) {
            return Ok(texture.clone());
        }
        let texture: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, size)?;
        self.0.insert(key, texture.clone());
        Ok(texture)
    }

    /// Frees them, once the eyes stop.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// Draws `elements` (topmost first) into `texture`, over `clear`.
fn draw(renderer: &mut GlesRenderer, mut texture: GlesTexture, scale: Scale<f64>, clear: [f32; 4], elements: &[OutputElement]) -> Result<GlesTexture, GlesError> {
    let size = texture.size();
    let size: Size<i32, Physical> = (size.w, size.h).into();
    let full = Rectangle::from_size(size);
    {
        let mut target = renderer.bind(&mut texture)?;
        let mut frame = renderer.render(&mut target, size, Transform::Normal)?;
        frame.clear(clear.into(), &[full])?;
        draw_render_elements::<GlesRenderer, _, _>(&mut frame, scale, elements, &[full])?;
        // One GL context, in order: the next pass waits for this one.
        let _ = frame.finish()?;
    }
    Ok(texture)
}

/// One pass of `program`: `source` drawn into `texture`, filling it.
fn pass(renderer: &mut GlesRenderer, source: &GlesTexture, mut texture: GlesTexture, program: &GlesTexProgram) -> Result<GlesTexture, GlesError> {
    let from = source.size();
    let size = texture.size();
    let size: Size<i32, Physical> = (size.w, size.h).into();
    let full = Rectangle::from_size(size);
    {
        let mut target = renderer.bind(&mut texture)?;
        let mut frame = renderer.render(&mut target, size, Transform::Normal)?;
        frame.clear(Color32F::TRANSPARENT, &[full])?;
        let uniforms = [
            Uniform::new("halfpixel", UniformValue::_2f(0.5 / from.w as f32, 0.5 / from.h as f32)),
            Uniform::new("offset", OFFSET),
        ];
        frame.render_texture_from_to(source, Rectangle::from_size(from.to_f64()), full, &[full], &[], Transform::Normal, 1.0, Some(program), &uniforms)?;
        let _ = frame.finish()?;
    }
    Ok(texture)
}

/// Half a size, never below a pixel.
fn half(size: Size<i32, Buffer>) -> Size<i32, Buffer> {
    ((size.w / 2).max(1), (size.h / 2).max(1)).into()
}

/// The session on `output`, `session` (its elements, topmost first),
/// blurred by `strength` (0…1): the textures to draw, bottom first, with
/// their alpha.
#[allow(clippy::too_many_arguments)]
fn blurred(
    renderer: &mut GlesRenderer,
    cache: &mut Cache,
    programs: (&GlesTexProgram, &GlesTexProgram),
    output: &str,
    size: Size<i32, Physical>,
    scale: Scale<f64>,
    clear: [f32; 4],
    session: &[OutputElement],
    strength: f32,
) -> Result<Vec<(GlesTexture, f32)>, GlesError> {
    let picture = cache.target(renderer, (output.to_owned(), 0, 0), (size.w, size.h).into())?;
    let picture = draw(renderer, picture, scale, clear, session)?;
    let s = strength.clamp(0.0, 1.0) * DEPTH as f32;
    let low = (s.floor() as usize).min(DEPTH);
    let high = (low + 1).min(DEPTH);
    let blend = s - low as f32;

    let mut down = vec![picture];
    for k in 1..=high {
        let texture = cache.target(renderer, (output.to_owned(), 0, k), half(down[k - 1].size()))?;
        down.push(pass(renderer, &down[k - 1], texture, programs.0)?);
    }
    // Depth k: k halvings down, then back up to half size.
    let depth = |cache: &mut Cache, renderer: &mut GlesRenderer, k: usize, chain: u8| -> Result<GlesTexture, GlesError> {
        let mut texture = down[k].clone();
        for i in (1..k).rev() {
            let target = cache.target(renderer, (output.to_owned(), chain, i), down[i].size())?;
            texture = pass(renderer, &texture, target, programs.1)?;
        }
        Ok(texture)
    };
    let mut layers = vec![(depth(cache, renderer, low, 1)?, 1.0)];
    if high > low && blend > 0.0 {
        layers.push((depth(cache, renderer, high, 2)?, blend));
    }
    Ok(layers)
}

impl crate::state::Orbit {
    /// The frame of `output` while the eyes move, `closed` (0…1) shut:
    /// the greet and black fading in over the session blurring, topmost
    /// first, after whatever `elements` holds (the pointer).
    pub fn eyes_elements(&mut self, output: &Output, renderer: &mut GlesRenderer, scale: Scale<f64>, closed: f32, shaders: &crate::rounded::Shaders, elements: &mut Vec<OutputElement>) {
        let Some(geo) = self.space.output_geometry(output) else { return };
        let size = geo.size.to_physical_precise_round(scale);
        crate::session_lock::face_elements(self, output, renderer, scale, closed, elements);
        elements.push(OutputElement::Solid(SolidColorRenderElement::new(
            Id::new(),
            Rectangle::from_size(size),
            CommitCounter::default(),
            Color32F::new(0.0, 0.0, 0.0, closed),
            Kind::Unspecified,
        )));
        let mut session = Vec::new();
        crate::render::session_elements(self, output, renderer, scale, Some(shaders), &mut session);
        let clear = crate::render::BACKGROUND;
        let mut cache = std::mem::take(&mut self.eyes_cache);
        let layers = blurred(renderer, &mut cache, (&shaders.down, &shaders.up), &output.name(), size, scale, clear, &session, closed);
        self.eyes_cache = cache;
        match layers {
            Ok(layers) => {
                let context = renderer.context_id();
                // Topmost first: the deeper blur over the shallower.
                for (texture, alpha) in layers.into_iter().rev() {
                    // The whole texture (half size when blurred), stretched
                    // over the monitor.
                    let whole = texture.size();
                    let src = Rectangle::from_size((whole.w as f64, whole.h as f64).into());
                    elements.push(OutputElement::Texture(TextureRenderElement::from_static_texture(
                        // A new id each frame: the picture changes each frame.
                        Id::new(),
                        context.clone(),
                        (0.0, 0.0),
                        texture,
                        1,
                        Transform::Normal,
                        Some(alpha),
                        Some(src),
                        Some(geo.size),
                        None,
                        Kind::Unspecified,
                    )));
                }
            }
            // The session stays hidden: black is all that is left.
            Err(err) => tracing::warn!("eyes: {err:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curve_ends_where_it_should() {
        assert_eq!(ease(0.0), 0.0);
        assert_eq!(ease(1.0), 1.0);
        assert_eq!(ease(-1.0), 0.0);
        assert_eq!(ease(2.0), 1.0);
    }

    #[test]
    fn the_curve_is_symmetric() {
        // Turning back halfway (`lock::turn`) keeps the eyes where they are.
        for i in 0..=20 {
            let p = i as f32 / 20.0;
            assert!((ease(1.0 - p) - (1.0 - ease(p))).abs() < 1e-5, "at {p}");
        }
    }
}
