//! A stand-in locker of others, for testing the core's side of
//! `ext-session-lock-v1` (PLANO-CORE §7.3 step 4): it locks, covers every
//! monitor with a dark red surface, and unlocks SECONDS after `locked`.
//! With `--die` it quits without unlocking instead, as a crashed locker
//! would: the session must stay locked and the greet take over.
//!
//!   session-lock [SECONDS] [--die]
//!
//! It says `finished` when the compositor refused the lock (already
//! locked).

use std::{os::fd::AsFd, time::Duration};

use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface},
};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1::ExtSessionLockManagerV1,
    ext_session_lock_surface_v1::{self, ExtSessionLockSurfaceV1},
    ext_session_lock_v1::{self, ExtSessionLockV1},
};

#[derive(Default)]
struct State {
    locked: bool,
    finished: bool,
    /// Surfaces configured but not yet drawn: (surface, width, height).
    to_draw: Vec<(wl_surface::WlSurface, u32, u32)>,
}

/// Dark red, ARGB.
const COLOUR: u32 = 0xff40_0010;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seconds: u64 = args.iter().find_map(|a| a.parse().ok()).unwrap_or(3);
    let die = args.iter().any(|a| a == "--die");
    let conn = Connection::connect_to_env().expect("a Wayland connection");
    let (globals, mut queue) = registry_queue_init::<State>(&conn).expect("the registry");
    let qh = queue.handle();
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=4, ()).expect("wl_compositor");
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("wl_shm");
    let manager: ExtSessionLockManagerV1 = globals.bind(&qh, 1..=1, ()).expect("no ext_session_lock_manager_v1");
    let outputs: Vec<wl_output::WlOutput> = globals
        .contents()
        .clone_list()
        .into_iter()
        .filter(|g| g.interface == "wl_output")
        .map(|g| globals.registry().bind(g.name, g.version.min(4), &qh, ()))
        .collect();

    let lock = manager.lock(&qh, ());
    let mut surfaces = Vec::new();
    for output in &outputs {
        let surface = compositor.create_surface(&qh, ());
        let lock_surface = lock.get_lock_surface(&surface, output, &qh, surface.clone());
        surfaces.push((surface, lock_surface));
    }
    eprintln!("session-lock: asked to lock {} monitor(s)", outputs.len());

    let mut state = State::default();
    loop {
        queue.blocking_dispatch(&mut state).expect("the connection");
        for (surface, w, h) in std::mem::take(&mut state.to_draw) {
            let buffer = fill(&shm, &qh, w, h);
            surface.attach(Some(&buffer), 0, 0);
            surface.damage(0, 0, w as i32, h as i32);
            surface.commit();
        }
        conn.flush().expect("the connection");
        if state.finished {
            eprintln!("session-lock: finished (refused)");
            return;
        }
        if state.locked {
            eprintln!("session-lock: locked");
            if die {
                eprintln!("session-lock: quitting without unlocking");
                std::process::exit(0);
            }
            std::thread::sleep(Duration::from_secs(seconds));
            lock.unlock_and_destroy();
            conn.roundtrip().expect("the connection");
            eprintln!("session-lock: unlocked");
            return;
        }
    }
}

/// A buffer of `w` × `h` in `COLOUR`.
fn fill(shm: &wl_shm::WlShm, qh: &QueueHandle<State>, w: u32, h: u32) -> wl_buffer::WlBuffer {
    let size = (w * h * 4) as usize;
    let fd = rustix::fs::memfd_create("session-lock", rustix::fs::MemfdFlags::CLOEXEC).expect("memfd");
    let pixels: Vec<u8> = std::iter::repeat_n(COLOUR.to_le_bytes(), (w * h) as usize).flatten().collect();
    rustix::io::write(&fd, &pixels).expect("writing the buffer");
    let pool = shm.create_pool(fd.as_fd(), size as i32, qh, ());
    let buffer = pool.create_buffer(0, w as i32, h as i32, (w * 4) as i32, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    buffer
}

impl Dispatch<ExtSessionLockV1, ()> for State {
    fn event(state: &mut Self, _: &ExtSessionLockV1, event: ext_session_lock_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            ext_session_lock_v1::Event::Locked => state.locked = true,
            ext_session_lock_v1::Event::Finished => state.finished = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtSessionLockSurfaceV1, wl_surface::WlSurface> for State {
    fn event(state: &mut Self, lock_surface: &ExtSessionLockSurfaceV1, event: ext_session_lock_surface_v1::Event, surface: &wl_surface::WlSurface, _: &Connection, _: &QueueHandle<Self>) {
        if let ext_session_lock_surface_v1::Event::Configure { serial, width, height } = event {
            lock_surface.ack_configure(serial);
            state.to_draw.push((surface.clone(), width, height));
        }
    }
}

macro_rules! ignore {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for State {
            fn event(_: &mut Self, _: &$t, _: <$t as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
ignore!(wl_compositor::WlCompositor, wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_buffer::WlBuffer, wl_output::WlOutput, wl_surface::WlSurface, ExtSessionLockManagerV1);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}
