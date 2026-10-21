//! The pointer, drawn by the core when it owns the screen (the DRM
//! backend; nested, the host draws its own): the image a client asked for
//! (`wl_pointer.set_cursor`), or the theme's default arrow — the theme and
//! size of the config, as sway's `seat * xcursor_theme`.
//!
//! Ported from anvil `src/cursor.rs` and `src/drawing.rs` @ 24647fc
//! (Smithay, MIT, Copyright (c) 2017 Victor Berger and Victoria Brekenfeld),
//! with its fallback image `resources/cursor.rgba`.

use std::{io::Read, time::Duration};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            ImportAll, ImportMem, Renderer, Texture,
            element::{
                AsRenderElements, Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            },
        },
    },
    input::pointer::CursorImageStatus,
    render_elements,
    utils::{Physical, Point, Scale, Transform},
};
use xcursor::{
    CursorTheme,
    parser::{Image, parse_xcursor},
};

static FALLBACK: &[u8] = include_bytes!("../resources/cursor.rgba");

/// The theme's default cursor, in every size the theme has.
pub struct Cursor {
    icons: Vec<Image>,
    size: u32,
    /// The image last drawn, uploaded once.
    buffer: Option<(Image, MemoryRenderBuffer)>,
}

impl Cursor {
    pub fn load(theme: &str, size: u32) -> Self {
        let icons = load_icon(&CursorTheme::load(theme)).unwrap_or_else(|err| {
            tracing::warn!("cursor theme {theme}: {err}; using the built-in arrow");
            vec![Image { size: 32, width: 64, height: 64, xhot: 1, yhot: 1, delay: 1, pixels_rgba: FALLBACK.to_vec(), pixels_argb: Vec::new() }]
        });
        Self { icons, size, buffer: None }
    }

    /// The image at its start, for XWayland's root window.
    pub fn image(&self) -> Image {
        frame(0, self.size, &self.icons)
    }

    /// The image to show at `time` (themes may animate it) and its hotspot.
    pub fn buffer(&mut self, time: Duration) -> (MemoryRenderBuffer, Point<i32, Physical>) {
        let image = frame(time.as_millis() as u32, self.size, &self.icons);
        let hotspot = Point::from((image.xhot as i32, image.yhot as i32));
        if let Some((cached, buffer)) = &self.buffer
            && *cached == image
        {
            return (buffer.clone(), hotspot);
        }
        let buffer = MemoryRenderBuffer::from_slice(&image.pixels_rgba, Fourcc::Argb8888, (image.width as i32, image.height as i32), 1, Transform::Normal, None);
        self.buffer = Some((image, buffer.clone()));
        (buffer, hotspot)
    }
}

fn nearest_images(size: u32, images: &[Image]) -> impl Iterator<Item = &Image> {
    let nearest = images.iter().min_by_key(|image| (size as i32 - image.size as i32).abs()).expect("at least one image");
    images.iter().filter(move |image| image.width == nearest.width && image.height == nearest.height)
}

fn frame(mut millis: u32, size: u32, images: &[Image]) -> Image {
    let total = nearest_images(size, images).fold(0, |acc, image| acc + image.delay);
    if total == 0 {
        return nearest_images(size, images).next().expect("an image").clone();
    }
    millis %= total;
    for image in nearest_images(size, images) {
        if millis < image.delay {
            return image.clone();
        }
        millis -= image.delay;
    }
    unreachable!("the delays add up to the total")
}

fn load_icon(theme: &CursorTheme) -> Result<Vec<Image>, String> {
    let path = theme.load_icon("default").ok_or("no default cursor")?;
    let mut data = Vec::new();
    std::fs::File::open(path).and_then(|mut f| f.read_to_end(&mut data)).map_err(|e| e.to_string())?;
    parse_xcursor(&data).ok_or_else(|| "not an xcursor file".into())
}

/// What the pointer looks like now, ready to draw.
pub struct PointerElement {
    pub status: CursorImageStatus,
    pub buffer: Option<MemoryRenderBuffer>,
}

render_elements! {
    pub PointerRenderElement<R> where R: ImportAll + ImportMem;
    Surface=WaylandSurfaceRenderElement<R>,
    Memory=MemoryRenderBufferRenderElement<R>,
}

impl<T: Texture + Clone + Send + 'static, R> AsRenderElements<R> for PointerElement
where
    R: Renderer<TextureId = T> + ImportAll + ImportMem,
{
    type RenderElement = PointerRenderElement<R>;

    fn render_elements<E>(&self, renderer: &mut R, location: Point<i32, Physical>, scale: Scale<f64>, alpha: f32) -> Vec<E>
    where
        E: From<PointerRenderElement<R>>,
    {
        match &self.status {
            CursorImageStatus::Hidden => Vec::new(),
            // Any named shape is the default arrow, for now.
            CursorImageStatus::Named(_) => self
                .buffer
                .as_ref()
                .and_then(|buffer| MemoryRenderBufferRenderElement::from_buffer(renderer, location.to_f64(), buffer, None, None, None, Kind::Cursor).ok())
                .map(|e| vec![E::from(PointerRenderElement::from(e))])
                .unwrap_or_default(),
            CursorImageStatus::Surface(surface) => {
                let elements: Vec<PointerRenderElement<R>> = render_elements_from_surface_tree(renderer, surface, location, scale, alpha, Kind::Cursor);
                elements.into_iter().map(E::from).collect()
            }
        }
    }
}
