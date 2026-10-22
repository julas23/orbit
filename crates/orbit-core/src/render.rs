//! What one frame of an output is made of, for either backend, topmost
//! first: the pointer (when the core draws it), the overlay and top layers
//! (menus, notifications), the windows — each with its menus, its surfaces
//! cut to rounded corners (`rounded`) and its border — then the bottom and
//! background layers. A fullscreen window hides most of that
//! (`fullscreen`); locking and unlocking blur it under black (`eyes`).
//!
//! The element types follow anvil `src/render.rs` @ 24647fc (Smithay, MIT,
//! Copyright (c) 2017 Victor Berger and Victoria Brekenfeld); a window's
//! menus and surfaces are gathered as Smithay's `Window::render_elements`
//! does (`desktop/space/wayland/window.rs`, same commit and licence).

use std::cell::RefCell;

use smithay::utils::IsAlive;

use smithay::{
    backend::renderer::{
        element::{
            AsRenderElements, Kind,
            solid::SolidColorRenderElement,
            surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            texture::TextureRenderElement,
        },
        gles::{GlesRenderer, GlesTexture, element::PixelShaderElement},
    },
    desktop::{PopupManager, Window, layer_map_for_output},
    input::pointer::{CursorIcon, CursorImageStatus, CursorImageSurfaceData},
    output::Output,
    render_elements,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Physical, Point, Rectangle, Scale},
    wayland::{compositor::with_states, seat::WaylandFocus, shell::wlr_layer::Layer as WlrLayer},
};

use crate::{
    config::color,
    cursor::{PointerElement, PointerRenderElement},
    rounded::{RoundedElement, Shaders, border_uniforms},
    state::Orbit,
};

/// The screen behind the windows.
pub const BACKGROUND: [f32; 4] = [0.106, 0.122, 0.153, 1.0];
/// The screen while locked (§7.1: eyes closed).
const LOCKED: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// What the screen is cleared to.
pub fn background(state: &Orbit) -> [f32; 4] {
    if state.locked() { LOCKED } else { BACKGROUND }
}

render_elements! {
    pub OutputElement<=GlesRenderer>;
    Pointer=PointerRenderElement<GlesRenderer>,
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Rounded=RoundedElement,
    Border=PixelShaderElement,
    Texture=TextureRenderElement<GlesTexture>,
    Solid=SolidColorRenderElement,
}

/// A window's border, kept with the window so the damage tracker sees the
/// same element frame after frame; and the colour it was last given.
#[derive(Default)]
struct Border(Option<(PixelShaderElement, BorderLook)>);

/// Colour, corner radius and width.
type BorderLook = ([f32; 4], i32, i32);

fn border_of(window: &Window) -> &RefCell<Border> {
    window.user_data().insert_if_missing(|| RefCell::new(Border::default()));
    window.user_data().get::<RefCell<Border>>().expect("just inserted")
}

/// The shaders, compiled the first time; `None` when they do not compile
/// (then corners are square and there are no borders).
fn shaders(state: &mut Orbit, renderer: &mut GlesRenderer) -> Option<Shaders> {
    state
        .shaders
        .get_or_insert_with(|| {
            Shaders::compile(renderer)
                .inspect_err(|err| tracing::error!("shaders: {err:?}; square corners, no borders"))
                .ok()
        })
        .clone()
}

/// Everything on `output`, topmost first. `pointer` is drawn when the core
/// owns the screen. Locked, the pointer and the greet are all there is.
pub fn output_elements(state: &mut Orbit, output: &Output, renderer: &mut GlesRenderer, pointer: bool) -> Vec<OutputElement> {
    let Some(geo) = state.space.output_geometry(output) else { return Vec::new() };
    let scale = Scale::from(output.current_scale().fractional_scale());
    let mut elements: Vec<OutputElement> = Vec::new();

    if pointer
        && let Some(location) = state.seat.get_pointer().map(|p| p.current_location())
        && geo.to_f64().contains(location)
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
        let shape = match &state.cursor_status {
            CursorImageStatus::Named(shape) => *shape,
            _ => CursorIcon::Default,
        };
        let (buffer, theme_hotspot) = state.cursor.buffer(shape, state.start_time.elapsed());
        let hotspot = if matches!(state.cursor_status, CursorImageStatus::Named(_)) { theme_hotspot } else { hotspot };
        let at: Point<i32, Physical> = (location - geo.loc.to_f64()).to_physical(scale).to_i32_round() - hotspot;
        let element = PointerElement { status: state.cursor_status.clone(), buffer: Some(buffer) };
        elements.extend(
            AsRenderElements::<GlesRenderer>::render_elements::<PointerRenderElement<GlesRenderer>>(&element, renderer, at, scale, 1.0)
                .into_iter()
                .map(OutputElement::Pointer),
        );
    }
    state.step_eyes();
    let shaders = shaders(state, renderer);
    // Closing or opening: the session blurred under black and the greet
    // (`eyes`).
    if let (Some(closed), Some(shaders)) = (state.eyes(), shaders.as_ref()) {
        state.eyes_elements(output, renderer, scale, closed, shaders, &mut elements);
        return elements;
    }
    if state.locked() {
        crate::session_lock::face_elements(state, output, renderer, scale, 1.0, &mut elements);
        return elements;
    }
    session_elements(state, output, renderer, scale, shaders.as_ref(), &mut elements);
    elements
}

/// The session on `output` — layers and windows — topmost first.
pub fn session_elements(state: &Orbit, output: &Output, renderer: &mut GlesRenderer, scale: Scale<f64>, shaders: Option<&Shaders>, elements: &mut Vec<OutputElement>) {
    let Some(geo) = state.space.output_geometry(output) else { return };
    // A fullscreen window hides the rest, but for the overlay, the top
    // layer's menus and X menus (`fullscreen`).
    if let Some(full) = state.fullscreen_on(output) {
        layer_elements(output, renderer, scale, &[WlrLayer::Overlay], elements);
        // Before the map is held: `top_above` takes it too, and taking it
        // twice hangs the core.
        let above = crate::fullscreen::top_above(output);
        let map = layer_map_for_output(output);
        for surface in &above {
            let Some(at) = map.layer_geometry(surface).map(|g| g.loc.to_physical_precise_round(scale)) else { continue };
            elements.extend(
                AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(surface, renderer, at, scale, 1.0)
                    .into_iter()
                    .map(OutputElement::Surface),
            );
        }
        drop(map);
        let menus: Vec<Window> = state
            .space
            .elements_for_output(output)
            .filter(|w| w.x11_surface().is_some_and(|x| x.is_override_redirect()))
            .cloned()
            .collect();
        for window in menus.iter().rev().chain([&full]) {
            window_elements(state, window, geo, renderer, scale, shaders, elements);
        }
        return;
    }
    layer_elements(output, renderer, scale, &[WlrLayer::Overlay, WlrLayer::Top], elements);
    // Only this monitor's: a window of the next one that reaches over the
    // edge (its shadow, a frame of the old size) is not drawn here. X menus
    // and the window being dragged go where they are.
    let shown = state.visible.get(&output.name()).copied();
    let windows: Vec<Window> = state
        .space
        .elements_for_output(output)
        .filter(|w| !state.all_windows().any(|m| m == *w) || state.dragging.as_ref() == Some(*w) || state.workspace_of(w) == shown)
        .cloned()
        .collect();
    for window in windows.iter().rev() {
        window_elements(state, window, geo, renderer, scale, shaders, elements);
    }
    layer_elements(output, renderer, scale, &[WlrLayer::Bottom, WlrLayer::Background], elements);
}

/// The layer surfaces of `layers` on `output`, topmost first.
fn layer_elements(output: &Output, renderer: &mut GlesRenderer, scale: Scale<f64>, layers: &[WlrLayer], elements: &mut Vec<OutputElement>) {
    let map = layer_map_for_output(output);
    for layer in layers {
        for surface in map.layers_on(*layer).rev() {
            let Some(at) = map.layer_geometry(surface).map(|g| g.loc.to_physical_precise_round(scale)) else { continue };
            elements.extend(
                AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(surface, renderer, at, scale, 1.0)
                    .into_iter()
                    .map(OutputElement::Surface),
            );
        }
    }
}

/// A window on the output at `out`: its menus, its surfaces (rounded) and
/// its border.
fn window_elements(state: &Orbit, window: &Window, out: Rectangle<i32, Logical>, renderer: &mut GlesRenderer, scale: Scale<f64>, shaders: Option<&Shaders>, elements: &mut Vec<OutputElement>) {
    // An X window has its surface once XWayland gives it one.
    let Some(surface) = window.wl_surface() else { return };
    let surface: &WlSurface = &surface;
    let Some(mut rect) = state.space.element_geometry(window) else { return };
    rect.loc -= out.loc;
    let geometry = window.geometry();
    let at = (rect.loc - geometry.loc).to_physical_precise_round(scale);

    // An X menu or tooltip is drawn as it is: no corners, no border.
    if window.x11_surface().is_some_and(|x| x.is_override_redirect()) {
        elements.extend(
            render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<GlesRenderer>>(renderer, surface, at, scale, 1.0, Kind::Unspecified)
                .into_iter()
                .map(OutputElement::Surface),
        );
        return;
    }

    for (popup, offset) in PopupManager::popups_for_surface(surface) {
        let offset = (geometry.loc + offset - popup.geometry().loc).to_physical_precise_round(scale);
        elements.extend(
            render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<GlesRenderer>>(renderer, popup.wl_surface(), at + offset, scale, 1.0, Kind::Unspecified)
                .into_iter()
                .map(OutputElement::Surface),
        );
    }

    // Fullscreen: square corners, no border.
    let fullscreen = crate::fullscreen::is_fullscreen(window);
    let radius = if fullscreen { 0 } else { state.config.corner_radius.max(0) };
    let physical = rect.to_physical_precise_round(scale);
    let physical_radius = (radius as f64 * scale.x).round() as i32;
    for element in render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<GlesRenderer>>(renderer, surface, at, scale, 1.0, Kind::Unspecified) {
        match shaders {
            Some(s) if physical_radius > 0 && RoundedElement::needs_clip(&element, physical, physical_radius, scale) => {
                elements.push(OutputElement::Rounded(RoundedElement::new(element, s.clip.clone(), physical, physical_radius as f32)));
            }
            _ => elements.push(OutputElement::Surface(element)),
        }
    }

    let width = state.config.border;
    let Some(shaders) = shaders.filter(|_| width > 0 && !fullscreen) else { return };
    let colour = if state.focused_window().as_ref() == Some(window) { color(&state.config.focused_color) } else { color(&state.config.unfocused_color) };
    let area = Rectangle::new(rect.loc - Point::from((width, width)), (rect.size.w + 2 * width, rect.size.h + 2 * width).into());
    let border = border_of(window);
    let mut border = border.borrow_mut();
    match &mut border.0 {
        Some((element, last)) => {
            element.resize(area, None);
            if *last != (colour, radius, width) {
                element.update_uniforms(border_uniforms(colour, radius as f32, width as f32));
                *last = (colour, radius, width);
            }
        }
        None => {
            let element = PixelShaderElement::new(shaders.border.clone(), area, None, 1.0, border_uniforms(colour, radius as f32, width as f32), Kind::Unspecified);
            border.0 = Some((element, (colour, radius, width)));
        }
    }
    if let Some((element, ..)) = &border.0 {
        elements.push(OutputElement::Border(element.clone()));
    }
}
