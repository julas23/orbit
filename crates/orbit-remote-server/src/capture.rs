//! Capturing a monitor as it changes: wlr-screencopy's `copy_with_damage`,
//! into dmabufs. The compositor draws each frame straight into a buffer of
//! ours on the GPU, and answers only when something changed on that
//! monitor since our last frame (orbit-core keeps the copy waiting until
//! then, PLANO-CORE §8), saying what. Nothing goes through the CPU: the
//! encoder takes the same dmabuf.
//!
//! The buffers are made by GBM on the render node, linear where the driver
//! allows (what VA-API imports everywhere; NVIDIA's GBM refuses the LINEAR
//! flag but takes the linear modifier), otherwise in the layout it picks,
//! its modifier passed on; a few of them in turn: the one just captured is
//! the encoder's while the next is being drawn.

use std::{
    os::fd::{AsFd, OwnedFd},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_buffer, wl_output, wl_registry},
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1::{self, ZwpLinuxBufferParamsV1},
    zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self as frame, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

/// Buffers taken in turn (see `encode`).
pub const BUFFERS: usize = 4;
/// DRM's XRGB8888 (`XR24`), what orbit-core copies into.
const XRGB8888: u32 = u32::from_le_bytes(*b"XR24");

/// One of our dmabufs, as the encoder needs it.
pub struct Plane {
    pub fd: OwnedFd,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub offset: u32,
    /// Its layout (DRM format modifier); 0 is linear.
    pub modifier: u64,
}

/// A buffer the compositor can draw into: linear by the flag, by the
/// modifier, or as the driver likes; with its modifier. One made by the
/// flag says none (it is implicit), but it is linear: said so, as VA-API
/// will not take an implicit one.
fn allocate(device: &gbm::Device<std::fs::File>, width: u32, height: u32) -> std::io::Result<(gbm::BufferObject<()>, u64)> {
    use gbm::{BufferObjectFlags as Flags, Format, Modifier};
    if let Ok(object) = device.create_buffer_object::<()>(width, height, Format::Xrgb8888, Flags::RENDERING | Flags::LINEAR) {
        let modifier = match object.modifier() {
            Modifier::Invalid => Modifier::Linear,
            modifier => modifier,
        };
        return Ok((object, modifier.into()));
    }
    let object = device
        .create_buffer_object_with_modifiers2(width, height, Format::Xrgb8888, [Modifier::Linear].into_iter(), Flags::RENDERING)
        .or_else(|_| device.create_buffer_object(width, height, Format::Xrgb8888, Flags::RENDERING))?;
    let modifier = object.modifier().into();
    Ok((object, modifier))
}

/// A frame captured into buffer `index`.
pub struct Captured {
    pub index: usize,
    /// What changed since the last frame, in pixels (x, y, w, h).
    pub damage: Vec<(u32, u32, u32, u32)>,
}

pub struct Capture {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    manager: ZwlrScreencopyManagerV1,
    output: wl_output::WlOutput,
    pub name: String,
    pub planes: Vec<Plane>,
    buffers: Vec<wl_buffer::WlBuffer>,
    next: usize,
    cursor: bool,
    /// The GBM objects, kept alive as long as their dmabufs.
    _objects: Vec<gbm::BufferObject<()>>,
}

impl Capture {
    /// Opens the capture of the monitor named `output` (the first one when
    /// `None`), the pointer drawn in when `cursor`.
    pub fn open(output: Option<&str>, cursor: bool) -> anyhow::Result<Self> {
        let conn = Connection::connect_to_env().context("connecting to the Wayland display")?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
        let qh = queue.handle();
        let manager: ZwlrScreencopyManagerV1 =
            globals.bind(&qh, 3..=3, ()).context("the compositor has no wlr-screencopy version 3")?;
        let dmabuf: ZwpLinuxDmabufV1 = globals.bind(&qh, 3..=3, ()).context("the compositor has no linux-dmabuf")?;
        let mut state = State::default();
        let outputs: Vec<wl_output::WlOutput> = globals.contents().with_list(|list| {
            list.iter()
                .filter(|g| g.interface == "wl_output")
                .enumerate()
                .map(|(index, g)| {
                    state.names.push(String::new());
                    globals.registry().bind(g.name, g.version.min(4), &qh, index)
                })
                .collect()
        });
        queue.roundtrip(&mut state)?;
        let index = match output {
            Some(wanted) => state
                .names
                .iter()
                .position(|n| n == wanted)
                .with_context(|| format!("no monitor {wanted} (there are {:?})", state.names))?,
            None => 0,
        };
        let output = outputs.get(index).cloned().context("no monitor")?;
        let name = state.names[index].clone();

        // What the compositor wants to copy into: asked with a first frame.
        let probe = manager.capture_output(i32::from(cursor), &output, &qh, ());
        while !state.frame.buffer_done && !state.frame.failed {
            queue.blocking_dispatch(&mut state)?;
        }
        probe.destroy();
        let Some((fourcc, width, height)) = state.frame.dmabuf else {
            bail!("the compositor offers no dmabuf for {name}");
        };
        if fourcc != XRGB8888 {
            bail!("the compositor wants format {fourcc:#x}, not XRGB8888");
        }

        let device = gbm::Device::new(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/dri/renderD128")
                .context("opening the render node")?,
        )?;
        let (mut planes, mut buffers, mut objects) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..BUFFERS {
            let (object, modifier) = allocate(&device, width, height).context("making a dmabuf")?;
            let plane = Plane {
                fd: object.fd_for_plane(0).context("its file")?,
                width,
                height,
                stride: object.stride_for_plane(0),
                offset: object.offset(0),
                modifier,
            };
            let params = dmabuf.create_params(&qh, ());
            params.add(plane.fd.as_fd(), 0, plane.offset, plane.stride, (modifier >> 32) as u32, modifier as u32);
            let buffer = params.create_immed(width as i32, height as i32, fourcc, zwp_linux_buffer_params_v1::Flags::empty(), &qh, ());
            params.destroy();
            planes.push(plane);
            buffers.push(buffer);
            objects.push(object);
        }
        conn.flush()?;
        log::info!("remote: capturing {name}, {width}×{height}, into {BUFFERS} dmabufs");
        Ok(Self {
            conn,
            queue,
            state,
            manager,
            output,
            name,
            planes,
            buffers,
            next: 0,
            cursor,
            _objects: objects,
        })
    }

    /// The next frame: blocks until something has changed on the monitor.
    pub fn next(&mut self) -> anyhow::Result<Captured> {
        let qh = self.queue.handle();
        let index = self.next;
        self.next = (self.next + 1) % BUFFERS;
        self.state.frame = FrameState::default();
        let frame = self.manager.capture_output(i32::from(self.cursor), &self.output, &qh, ());
        while !self.state.frame.buffer_done && !self.state.frame.failed {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        frame.copy_with_damage(&self.buffers[index]);
        self.conn.flush()?;
        while !self.state.frame.ready && !self.state.frame.failed {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        frame.destroy();
        if self.state.frame.failed {
            bail!("the compositor could not copy {} (locked?)", self.name);
        }
        Ok(Captured {
            index,
            damage: std::mem::take(&mut self.state.frame.damage),
        })
    }
}

/// `orbit-remote-server capture [OUTPUT]`: captures for 10 s and says how
/// fast frames came and how much of the screen changed.
pub fn measure(output: Option<&str>) -> anyhow::Result<()> {
    let mut capture = Capture::open(output, true)?;
    let area = f64::from(capture.planes[0].width) * f64::from(capture.planes[0].height);
    let start = Instant::now();
    let (mut frames, mut changed, mut longest) = (0u32, 0.0f64, Duration::ZERO);
    let mut last = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        let frame = capture.next()?;
        let took = last.elapsed();
        last = Instant::now();
        longest = longest.max(took);
        frames += 1;
        let pixels: f64 = frame.damage.iter().map(|&(_, _, w, h)| f64::from(w) * f64::from(h)).sum();
        changed += pixels / area;
        log::debug!("remote: frame {frames} in buffer {} after {took:?}: {} rects, {:.1}% changed", frame.index, frame.damage.len(), 100.0 * pixels / area);
    }
    let seconds = start.elapsed().as_secs_f64();
    println!(
        "{}: {frames} frames in {seconds:.1} s ({:.1} fps), {:.1}% of the screen changed a frame on average, longest wait {longest:?}",
        capture.name,
        f64::from(frames) / seconds,
        100.0 * changed / f64::from(frames.max(1)),
    );
    Ok(())
}

#[derive(Default)]
struct FrameState {
    dmabuf: Option<(u32, u32, u32)>,
    buffer_done: bool,
    damage: Vec<(u32, u32, u32, u32)>,
    ready: bool,
    failed: bool,
}

#[derive(Default)]
struct State {
    names: Vec<String>,
    frame: FrameState,
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for State {
    fn event(state: &mut Self, _: &ZwlrScreencopyFrameV1, event: frame::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let f = &mut state.frame;
        match event {
            frame::Event::LinuxDmabuf { format, width, height } => f.dmabuf = Some((format, width, height)),
            frame::Event::BufferDone => f.buffer_done = true,
            frame::Event::Damage { x, y, width, height } => f.damage.push((x, y, width, height)),
            frame::Event::Ready { .. } => f.ready = true,
            frame::Event::Failed => f.failed = true,
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, usize> for State {
    fn event(state: &mut Self, _: &wl_output::WlOutput, event: wl_output::Event, index: &usize, _: &Connection, _: &QueueHandle<Self>) {
        if let wl_output::Event::Name { name } = event {
            state.names[*index] = name;
        }
    }
}

impl Dispatch<ZwpLinuxBufferParamsV1, ()> for State {
    fn event(_: &mut Self, _: &ZwpLinuxBufferParamsV1, event: zwp_linux_buffer_params_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let zwp_linux_buffer_params_v1::Event::Failed = event {
            log::error!("remote: the compositor would not take a dmabuf");
        }
    }
}

macro_rules! ignore {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for State {
            fn event(_: &mut Self, _: &$t, _: <$t as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
ignore!(wl_buffer::WlBuffer, ZwlrScreencopyManagerV1, ZwpLinuxDmabufV1);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}
