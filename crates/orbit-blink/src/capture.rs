//! Capturing the screens, with wlr-screencopy.
//!
//! Each output is copied into a shared-memory buffer by the compositor;
//! sway's IPC gives where each output sits in the layout (its logical
//! rectangle) and the focused window, so a capture can be cropped to a
//! window or to an area chosen on screen.

use std::{
    io::Read,
    os::fd::{AsFd, OwnedFd},
};

use anyhow::{Context, bail};
use orbit_common::sway;
use serde::Deserialize;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool},
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self as frame, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

use crate::image::{Image, Rect};

/// One output, captured.
pub struct Screen {
    /// The output's name (`eDP-1`).
    pub name: String,
    /// Where it is in the layout, in logical pixels.
    pub logical: Rect,
    pub focused: bool,
    pub image: Image,
}

impl Screen {
    /// Physical pixels per logical pixel.
    pub fn scale(&self) -> f64 {
        f64::from(self.image.width) / f64::from(self.logical.width.max(1))
    }

    /// `rect`, in global logical pixels, cut out of this screen.
    pub fn crop(&self, rect: Rect) -> Option<Image> {
        let local = rect.intersect(self.logical)?;
        let s = self.scale();
        self.image.crop(Rect {
            x: (f64::from(local.x - self.logical.x) * s).round() as i32,
            y: (f64::from(local.y - self.logical.y) * s).round() as i32,
            width: (f64::from(local.width) * s).round() as i32,
            height: (f64::from(local.height) * s).round() as i32,
        })
    }
}

#[derive(Deserialize)]
struct SwayOutput {
    name: String,
    active: bool,
    focused: bool,
    rect: SwayRect,
}

#[derive(Deserialize, Clone, Copy)]
struct SwayRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl From<SwayRect> for Rect {
    fn from(r: SwayRect) -> Rect {
        Rect {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        }
    }
}

#[derive(Deserialize)]
struct Node {
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    visible: Option<bool>,
    #[serde(default)]
    pid: Option<i64>,
    rect: SwayRect,
    #[serde(default)]
    nodes: Vec<Node>,
    #[serde(default)]
    floating_nodes: Vec<Node>,
}

/// The windows on screen, in global logical pixels, topmost last.
pub fn windows() -> Vec<Rect> {
    fn walk(node: &Node, out: &mut Vec<Rect>) {
        if node.pid.is_some() && node.visible == Some(true) {
            out.push(node.rect.into());
        }
        for child in node.nodes.iter().chain(&node.floating_nodes) {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    if let Ok(tree) = sway::Connection::open().and_then(|mut c| c.query::<Node>(sway::GET_TREE)) {
        walk(&tree, &mut out);
    }
    out
}

/// The focused window, in global logical pixels.
pub fn focused_window() -> Option<Rect> {
    fn walk(node: &Node) -> Option<Rect> {
        if node.focused && node.pid.is_some() {
            return Some(node.rect.into());
        }
        node.nodes.iter().chain(&node.floating_nodes).find_map(walk)
    }
    let tree: Node = sway::Connection::open()
        .and_then(|mut c| c.query(sway::GET_TREE))
        .ok()?;
    walk(&tree)
}

/// Every active output, captured; with the cursor drawn in when `cursor`.
pub fn screens(cursor: bool) -> anyhow::Result<Vec<Screen>> {
    let layout: Vec<SwayOutput> = sway::Connection::open()
        .and_then(|mut c| c.query(sway::GET_OUTPUTS))
        .context("asking sway for the outputs")?;

    let conn = Connection::connect_to_env().context("connecting to the Wayland display")?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
    let qh = queue.handle();
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).context("no wl_shm")?;
    let manager: ZwlrScreencopyManagerV1 = globals
        .bind(&qh, 1..=3, ())
        .context("the compositor has no wlr-screencopy")?;
    let mut state = State::default();
    let outputs: Vec<wl_output::WlOutput> = globals.contents().with_list(|list| {
        list.iter()
            .filter(|g| g.interface == "wl_output")
            .enumerate()
            .map(|(index, g)| {
                state.names.push(String::new());
                globals
                    .registry()
                    .bind(g.name, g.version.min(4), &qh, index)
            })
            .collect()
    });
    // The outputs' names (wl_output version 4).
    queue.roundtrip(&mut state)?;

    let mut screens = Vec::new();
    for (index, output) in outputs.iter().enumerate() {
        let name = state.names[index].clone();
        let Some(place) = layout.iter().find(|o| o.active && o.name == name) else {
            continue;
        };
        let image = capture(
            &conn, &mut queue, &mut state, &shm, &manager, output, cursor,
        )
        .with_context(|| format!("capturing {name}"))?;
        screens.push(Screen {
            name,
            logical: place.rect.into(),
            focused: place.focused,
            image,
        });
    }
    if screens.is_empty() {
        bail!("no output to capture");
    }
    Ok(screens)
}

fn capture(
    conn: &Connection,
    queue: &mut wayland_client::EventQueue<State>,
    state: &mut State,
    shm: &wl_shm::WlShm,
    manager: &ZwlrScreencopyManagerV1,
    output: &wl_output::WlOutput,
    cursor: bool,
) -> anyhow::Result<Image> {
    let qh = queue.handle();
    state.frame = FrameState::default();
    let frame = manager.capture_output(i32::from(cursor), output, &qh, ());
    // The buffer it wants; version 3 announces the end of the list.
    let version = manager.version();
    let announced = |f: &FrameState| f.buffer_done || (version < 3 && f.format.is_some());
    while !announced(&state.frame) && !state.frame.failed {
        queue.blocking_dispatch(state)?;
    }
    let Some((format, width, height, stride)) = state.frame.format else {
        bail!("no buffer format we can read");
    };
    let size = (stride * height) as usize;
    let fd: OwnedFd = rustix::fs::memfd_create("orbit-blink", rustix::fs::MemfdFlags::CLOEXEC)?;
    rustix::fs::ftruncate(&fd, size as u64)?;
    let pool = shm.create_pool(fd.as_fd(), size as i32, &qh, ());
    let buffer = pool.create_buffer(
        0,
        width as i32,
        height as i32,
        stride as i32,
        format,
        &qh,
        (),
    );
    frame.copy(&buffer);
    conn.flush()?;
    while !state.frame.ready && !state.frame.failed {
        queue.blocking_dispatch(state)?;
    }
    frame.destroy();
    buffer.destroy();
    pool.destroy();
    if state.frame.failed {
        bail!("the compositor could not copy the output");
    }

    let mut raw = vec![0u8; size];
    std::fs::File::from(fd).read_exact(&mut raw)?;
    let (w, h) = (width as usize, height as usize);
    let mut bgrx = vec![0u8; w * h * 4];
    for y in 0..h {
        let source_row = if state.frame.y_invert { h - 1 - y } else { y };
        let src = &raw[source_row * stride as usize..][..w * 4];
        let dst = &mut bgrx[y * w * 4..][..w * 4];
        match format {
            // Little-endian XRGB: B, G, R, X in memory — as we keep it.
            wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888 => dst.copy_from_slice(src),
            // XBGR: R, G, B, X — swap red and blue.
            _ => {
                for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    d.copy_from_slice(&[s[2], s[1], s[0], 255]);
                }
            }
        }
    }
    // Opaque, whatever the X byte held.
    for px in bgrx.chunks_exact_mut(4) {
        px[3] = 255;
    }
    Ok(Image {
        width: width as i32,
        height: height as i32,
        bgrx,
    })
}

#[derive(Default)]
struct FrameState {
    /// (format, width, height, stride) of a buffer we can read.
    format: Option<(wl_shm::Format, u32, u32, u32)>,
    buffer_done: bool,
    y_invert: bool,
    ready: bool,
    failed: bool,
}

#[derive(Default)]
struct State {
    names: Vec<String>,
    frame: FrameState,
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrScreencopyFrameV1,
        event: frame::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let f = &mut state.frame;
        match event {
            frame::Event::Buffer {
                format: WEnum::Value(format),
                width,
                height,
                stride,
            } if f.format.is_none()
                && matches!(
                    format,
                    wl_shm::Format::Xrgb8888
                        | wl_shm::Format::Argb8888
                        | wl_shm::Format::Xbgr8888
                        | wl_shm::Format::Abgr8888
                ) =>
            {
                f.format = Some((format, width, height, stride));
            }
            frame::Event::Flags { flags } => {
                f.y_invert = matches!(flags, WEnum::Value(v) if v.contains(frame::Flags::YInvert));
            }
            frame::Event::BufferDone => f.buffer_done = true,
            frame::Event::Ready { .. } => f.ready = true,
            frame::Event::Failed => f.failed = true,
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, usize> for State {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            state.names[*index] = name;
        }
    }
}

macro_rules! ignore {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for State {
            fn event(_: &mut Self, _: &$t, _: <$t as wayland_client::Proxy>::Event, _: &(),
                     _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
ignore!(
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_buffer::WlBuffer,
    ZwlrScreencopyManagerV1
);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
