//! Rounded corners: each surface of a window is drawn through a texture
//! shader that fades out what lies outside the window's rounded rectangle
//! (with a pixel of anti-aliasing), and the border is a ring drawn by a
//! pixel shader along the same curve.
//!
//! The shader learns where a pixel is in the window by undoing the texture
//! matrix the renderer builds (Smithay's `build_texture_mat`, gles/mod.rs @
//! 24647fc, MIT, Copyright (c) 2017 Victor Berger and Victoria
//! Brekenfeld) — the method of niri's `ClippedSurfaceRenderElement`.

use glam::{Affine2, Mat3, Vec2};
use smithay::{
    backend::renderer::{
        Texture,
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage, surface::WaylandSurfaceRenderElement, surface::WaylandSurfaceTexture},
        gles::{GlesError, GlesFrame, GlesPixelProgram, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType, UniformValue},
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    utils::{Buffer, Physical, Point, Rectangle, Scale, Size, Transform, user_data::UserDataMap},
};

const CLIP: &str = r#"
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

// Texture coordinates to pixels from the window's top-left corner.
uniform mat3 input_to_geo;
uniform vec2 geo_size;
uniform float radius;

// How much of the pixel at p is inside the rounded rectangle.
float inside(vec2 p, vec2 size, float r) {
    vec2 hs = size * 0.5;
    vec2 q = abs(p - hs) - (hs - vec2(r));
    float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
    return clamp(0.5 - d, 0.0, 1.0);
}

void main() {
    vec4 color = texture2D(tex, v_coords);
#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif
    vec3 p = input_to_geo * vec3(v_coords, 1.0);
    color = color * alpha * inside(p.xy, geo_size, radius);

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
"#;

const BORDER: &str = r#"
precision highp float;
uniform vec2 size;
uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

uniform vec4 color;
// The window's corner radius and the border's width.
uniform float radius;
uniform float width;

float inside(vec2 p, vec2 box, float r) {
    vec2 hs = box * 0.5;
    vec2 q = abs(p - hs) - (hs - vec2(r));
    float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
    return clamp(0.5 - d, 0.0, 1.0);
}

void main() {
    vec2 p = v_coords * size;
    float outer = inside(p, size, radius + width);
    float inner = inside(p - vec2(width), size - vec2(2.0 * width), radius);
    vec4 c = color * alpha * outer * (1.0 - inner);

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        c = vec4(0.0, 0.2, 0.0, 0.2) + c * 0.8;
#endif

    gl_FragColor = c;
}
"#;

/// The two programs, compiled once per renderer.
#[derive(Clone)]
pub struct Shaders {
    pub clip: GlesTexProgram,
    pub border: GlesPixelProgram,
}

impl Shaders {
    pub fn compile(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let clip = renderer.compile_custom_texture_shader(
            CLIP,
            &[
                UniformName::new("input_to_geo", UniformType::Matrix3x3),
                UniformName::new("geo_size", UniformType::_2f),
                UniformName::new("radius", UniformType::_1f),
            ],
        )?;
        let border = renderer.compile_custom_pixel_shader(
            BORDER,
            &[
                UniformName::new("color", UniformType::_4f),
                UniformName::new("radius", UniformType::_1f),
                UniformName::new("width", UniformType::_1f),
            ],
        )?;
        Ok(Self { clip, border })
    }
}

/// The uniforms of a border: its colour, the window's radius and the
/// border's width, in logical pixels.
pub fn border_uniforms(color: [f32; 4], radius: f32, width: f32) -> Vec<Uniform<'static>> {
    vec![
        Uniform::new("color", UniformValue::_4f(color[0], color[1], color[2], color[3])),
        Uniform::new("radius", radius),
        Uniform::new("width", width),
    ]
}

/// A surface of a window, cut to the window's rounded rectangle.
pub struct RoundedElement {
    inner: WaylandSurfaceRenderElement<GlesRenderer>,
    program: GlesTexProgram,
    /// The window's rectangle on the output, and the radius, in physical
    /// pixels.
    window: Rectangle<i32, Physical>,
    radius: f32,
}

impl RoundedElement {
    pub fn new(inner: WaylandSurfaceRenderElement<GlesRenderer>, program: GlesTexProgram, window: Rectangle<i32, Physical>, radius: f32) -> Self {
        Self { inner, program, window, radius }
    }

    /// Whether the surface reaches into a corner at all; when it does not,
    /// it is drawn as it is.
    pub fn needs_clip(inner: &WaylandSurfaceRenderElement<GlesRenderer>, window: Rectangle<i32, Physical>, radius: i32, scale: Scale<f64>) -> bool {
        let geo = inner.geometry(scale);
        corners(window, radius).iter().any(|c| c.overlaps(geo))
    }
}

/// The four squares of side `radius` in the corners of `window`.
fn corners(window: Rectangle<i32, Physical>, radius: i32) -> [Rectangle<i32, Physical>; 4] {
    let (x, y, w, h, r) = (window.loc.x, window.loc.y, window.size.w, window.size.h, radius);
    let size: Size<i32, Physical> = (r, r).into();
    [
        Rectangle::new((x, y).into(), size),
        Rectangle::new((x + w - r, y).into(), size),
        Rectangle::new((x, y + h - r).into(), size),
        Rectangle::new((x + w - r, y + h - r).into(), size),
    ]
}

impl Element for RoundedElement {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale)
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }

    fn damage_since(&self, scale: Scale<f64>, commit: Option<CommitCounter>) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }

    /// The surface's, less the corners: what shows through them must be
    /// drawn.
    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        let origin = self.inner.geometry(scale).loc;
        let cut = corners(self.window, self.radius.ceil() as i32).map(|mut c| {
            c.loc -= origin;
            c
        });
        self.inner.opaque_regions(scale).into_iter().flat_map(|r| r.subtract_rects(cut)).collect()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl RenderElement<GlesRenderer> for RoundedElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let WaylandSurfaceTexture::Texture(texture) = self.inner.texture() else {
            return self.inner.draw(frame, src, dst, damage, opaque_regions, cache);
        };
        let Some(to_tex) = texture_mat(src, dst, texture.size(), self.inner.transform(), texture.is_y_inverted()) else {
            return self.inner.draw(frame, src, dst, damage, opaque_regions, cache);
        };
        // Texture coordinates → pixels in `dst` → pixels in the window.
        let offset = Vec2::new((dst.loc.x - self.window.loc.x) as f32, (dst.loc.y - self.window.loc.y) as f32);
        let input_to_geo = Mat3::from_translation(offset) * to_tex.inverse();
        let uniforms = vec![
            Uniform::new("input_to_geo", UniformValue::Matrix3x3 { matrices: vec![input_to_geo.to_cols_array()], transpose: false }),
            Uniform::new("geo_size", (self.window.size.w as f32, self.window.size.h as f32)),
            Uniform::new("radius", self.radius),
        ];
        frame.override_default_tex_program(self.program.clone(), uniforms);
        let result = self.inner.draw(frame, src, dst, damage, opaque_regions, cache);
        frame.clear_tex_program_override();
        result
    }

    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        self.inner.underlying_storage(renderer)
    }
}

/// What the renderer turns a pixel of `dst` (from its corner) into: a
/// texture coordinate. As Smithay's `build_texture_mat`, plus the flip of
/// a texture stored upside down.
fn texture_mat(src: Rectangle<f64, Buffer>, dst: Rectangle<i32, Physical>, texture: Size<i32, Buffer>, transform: Transform, y_inverted: bool) -> Option<Mat3> {
    if dst.size.is_empty() || src.size.is_empty() || texture.is_empty() {
        return None;
    }
    let dst_src_size = transform.transform_size(src.size);
    let scale = dst_src_size.to_f64() / dst.size.to_f64();
    let (w, h) = (dst_src_size.w as f32, dst_src_size.h as f32);
    let translation = match transform {
        Transform::Normal | Transform::Flipped90 => Vec2::ZERO,
        Transform::_90 => Vec2::new(0.0, w),
        Transform::_180 => Vec2::new(w, h),
        Transform::_270 => Vec2::new(h, 0.0),
        Transform::Flipped => Vec2::new(w, 0.0),
        Transform::Flipped180 => Vec2::new(0.0, h),
        Transform::Flipped270 => Vec2::new(h, w),
    };
    let mut m = Affine2::from_scale(Vec2::new(scale.x as f32, scale.y as f32));
    m = transform.matrix() * m;
    m = Affine2::from_translation(translation) * m;
    m = Affine2::from_translation(Vec2::new(src.loc.x as f32, src.loc.y as f32)) * m;
    m = Affine2::from_scale(Vec2::new(1.0 / texture.w as f32, 1.0 / texture.h as f32)) * m;
    let mut m = Mat3::from(m);
    if y_inverted {
        m = Mat3::from_cols_array(&[1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0]) * m;
    }
    Some(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pixel_comes_back_to_where_it_was() {
        // A 200×100 buffer drawn at 2× into 100×50 pixels at (10, 20).
        let src = Rectangle::new((0.0, 0.0).into(), (200.0, 100.0).into());
        let dst = Rectangle::new((10, 20).into(), (100, 50).into());
        let m = texture_mat(src, dst, (200, 100).into(), Transform::Normal, false).unwrap();
        let uv = m.transform_point2(Vec2::new(50.0, 25.0));
        assert!((uv - Vec2::new(0.5, 0.5)).length() < 1e-5);
        let back = m.inverse().transform_point2(Vec2::new(1.0, 1.0));
        assert!((back - Vec2::new(100.0, 50.0)).length() < 1e-3);
    }
}
