//! What one frame of an output is made of, for either backend: the pointer
//! (when the core draws it), the borders of the windows, then the windows
//! and layer surfaces of the space.
//!
//! The element types follow anvil `src/render.rs` @ 24647fc (Smithay, MIT,
//! Copyright (c) 2017 Victor Berger and Victoria Brekenfeld).

use std::cell::RefCell;

use smithay::utils::IsAlive;

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer, Texture,
        element::{
            AsRenderElements, Kind,
            solid::{SolidColorBuffer, SolidColorRenderElement},
            surface::WaylandSurfaceRenderElement,
        },
    },
    desktop::{
        Window,
        space::{SpaceRenderElements, space_render_elements},
    },
    input::pointer::{CursorImageStatus, CursorImageSurfaceData},
    output::Output,
    render_elements,
    utils::{Physical, Point, Scale},
    wayland::compositor::with_states,
};

use crate::{
    config::color,
    cursor::{PointerElement, PointerRenderElement},
    state::Orbit,
};

/// The screen behind the windows.
pub const BACKGROUND: [f32; 4] = [0.106, 0.122, 0.153, 1.0];

render_elements! {
    pub CustomElement<R> where R: ImportAll + ImportMem;
    Pointer=PointerRenderElement<R>,
    Solid=SolidColorRenderElement,
}

render_elements! {
    pub OutputElement<R, E> where R: ImportAll + ImportMem;
    Space=SpaceRenderElements<R, E>,
    Custom=CustomElement<R>,
}

/// The four sides of a window's border, kept with the window so the
/// damage tracker sees the same elements frame after frame.
#[derive(Default)]
struct Border([SolidColorBuffer; 4]);

fn border_of(window: &Window) -> &RefCell<Border> {
    window.user_data().insert_if_missing(|| RefCell::new(Border::default()));
    window.user_data().get::<RefCell<Border>>().expect("just inserted")
}

/// Everything on `output`, topmost first. `pointer` is drawn when the core
/// owns the screen.
pub fn output_elements<R>(state: &mut Orbit, output: &Output, renderer: &mut R, pointer: bool) -> Vec<OutputElement<R, WaylandSurfaceRenderElement<R>>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    let Some(geo) = state.space.output_geometry(output) else { return Vec::new() };
    let scale = Scale::from(output.current_scale().fractional_scale());
    let mut elements: Vec<OutputElement<R, WaylandSurfaceRenderElement<R>>> = Vec::new();

    if pointer
        && let Some(location) = state.seat.get_pointer().map(|p| p.current_location())
        && geo.to_f64().contains(location)
    {
        {
            if let CursorImageStatus::Surface(surface) = &state.cursor_status
                && !surface.alive()
            {
                state.cursor_status = CursorImageStatus::default_named();
            }
            let hotspot = match &state.cursor_status {
                CursorImageStatus::Surface(surface) => with_states(surface, |states| {
                    states.data_map.get::<CursorImageSurfaceData>().map(|d| d.lock().unwrap().hotspot).unwrap_or_default()
                })
                .to_physical(1),
                _ => Point::default(),
            };
            let (buffer, theme_hotspot) = state.cursor.buffer(state.start_time.elapsed());
            let hotspot = if matches!(state.cursor_status, CursorImageStatus::Named(_)) { theme_hotspot } else { hotspot };
            let at: Point<i32, Physical> = (location - geo.loc.to_f64()).to_physical(scale).to_i32_round() - hotspot;
            let element = PointerElement { status: state.cursor_status.clone(), buffer: Some(buffer) };
            elements.extend(
                AsRenderElements::<R>::render_elements::<PointerRenderElement<R>>(&element, renderer, at, scale, 1.0)
                    .into_iter()
                    .map(|e| OutputElement::Custom(CustomElement::Pointer(e))),
            );
        }
    }

    elements.extend(border_elements(state, geo.loc).into_iter().map(|e| OutputElement::Custom(CustomElement::Solid(e))));
    elements.extend(space_render_elements(renderer, [&state.space], output, 1.0).unwrap_or_default().into_iter().map(OutputElement::Space));
    elements
}

fn border_elements(state: &Orbit, origin: Point<i32, smithay::utils::Logical>) -> Vec<SolidColorRenderElement> {
    let width = state.config.border;
    if width <= 0 {
        return Vec::new();
    }
    let focused = state.focused_window();
    let (on, off) = (color(&state.config.focused_color), color(&state.config.unfocused_color));
    let mut elements = Vec::new();
    for window in &state.tiled() {
        let Some(geo) = state.space.element_geometry(window) else { continue };
        let colour = if Some(window) == focused.as_ref() { on } else { off };
        let border = border_of(window);
        let mut border = border.borrow_mut();
        let (x, y) = (geo.loc.x - origin.x - width, geo.loc.y - origin.y - width);
        let (w, h) = (geo.size.w + 2 * width, geo.size.h + 2 * width);
        let sides = [(x, y, w, width), (x, y + h - width, w, width), (x, y + width, width, h - 2 * width), (x + w - width, y + width, width, h - 2 * width)];
        for (buffer, (sx, sy, sw, sh)) in border.0.iter_mut().zip(sides) {
            buffer.update((sw.max(0), sh.max(0)), colour);
            elements.push(SolidColorRenderElement::from_buffer(buffer, (sx, sy), 1.0, 1.0, Kind::Unspecified));
        }
    }
    elements
}
