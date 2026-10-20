//! Watching and setting the clipboard, with wlr-data-control.
//!
//! The protocol lets a client that has no window of its own see every new
//! selection — the clipboard (Ctrl+C) and the primary selection (selected
//! with the mouse, pasted with the middle button) — read it, and offer its
//! own. XWayland's selections are bridged to Wayland's by the compositor, so
//! this sees X11 applications' copies too.
//!
//! It runs on its own thread with its own Wayland connection, separate from
//! GTK's: copies go to the application as [`Copied`] on a channel, and the
//! application sends back [`Command`]s.

use std::{
    collections::HashMap,
    io::{Read, Write},
    os::fd::{AsFd, OwnedFd},
    sync::mpsc,
    time::{Duration, Instant},
};

use anyhow::Context;
use rustix::event::{PollFd, PollFlags, poll};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    backend::ObjectId,
    event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_registry, wl_seat},
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self as device, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::{self as offer, ZwlrDataControlOfferV1},
    zwlr_data_control_source_v1::{self as source, ZwlrDataControlSourceV1},
};

use crate::history::Content;

/// Something copied by another application.
#[derive(Debug)]
pub struct Copied {
    pub content: Content,
    /// From the primary selection.
    pub primary: bool,
}

#[derive(Debug)]
pub enum Command {
    /// Puts `content` in the clipboard (and in the primary selection too,
    /// when they are kept in sync).
    Set(Content),
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Keep the clipboard and the primary selection the same.
    pub sync_primary: bool,
    /// Skip copies marked as secret by password managers.
    pub ignore_secrets: bool,
    /// Largest copy kept, in bytes.
    pub max_bytes: usize,
}

/// Talks to the watcher thread.
pub struct Handle {
    commands: mpsc::Sender<Command>,
    wake: std::os::unix::net::UnixStream,
}

impl Handle {
    pub fn send(&self, command: Command) {
        if self.commands.send(command).is_ok() {
            let _ = (&self.wake).write_all(&[1]);
        }
    }
}

/// Text types, best first: what is read from others and offered by us.
const TEXT: &[&str] = &[
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
    "TEXT",
];

/// KeePassXC (and KDE's tools) mark a copied password with this type.
const SECRET: &str = "x-kde-passwordManagerHint";

/// Starts watching; copies arrive on `copied`.
pub fn spawn(copied: async_channel::Sender<Copied>, options: Options) -> anyhow::Result<Handle> {
    let conn = Connection::connect_to_env().context("connecting to the Wayland display")?;
    let (globals, queue) = registry_queue_init::<State>(&conn).context("Wayland registry")?;
    let qh = queue.handle();
    let manager: ZwlrDataControlManagerV1 = globals
        .bind(&qh, 2..=2, ())
        .context("the compositor has no wlr-data-control (version 2)")?;
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).context("no seat")?;
    let device = manager.get_data_device(&seat, &qh, ());

    let (commands, receiver) = mpsc::channel();
    let (wake, wake_reader) = std::os::unix::net::UnixStream::pair()?;
    wake_reader.set_nonblocking(true)?;

    let state = State {
        manager,
        device,
        offers: HashMap::new(),
        pending: Vec::new(),
        sources: HashMap::new(),
        own: [None, None],
        finished: false,
    };
    std::thread::Builder::new()
        .name("orbit-clip-wayland".into())
        .spawn(move || {
            if let Err(err) = run(conn, queue, state, receiver, wake_reader, copied, options) {
                log::error!("clipboard watcher stopped: {err:#}");
            }
        })?;
    Ok(Handle { commands, wake })
}

struct State {
    manager: ZwlrDataControlManagerV1,
    device: ZwlrDataControlDeviceV1,
    /// Offers announced, with their MIME types.
    offers: HashMap<ObjectId, (ZwlrDataControlOfferV1, Vec<String>)>,
    /// New selections to read: (offer, primary); None when cleared.
    pending: Vec<(Option<ZwlrDataControlOfferV1>, bool)>,
    /// Our sources still offered, with what they hold.
    sources: HashMap<ObjectId, (Content, bool)>,
    /// Our current source for the clipboard [0] and the primary [1]:
    /// selections that are ours are not read back.
    own: [Option<ObjectId>; 2],
    finished: bool,
}

impl State {
    /// Offers `content` as the clipboard or the primary selection.
    fn set(&mut self, content: Content, primary: bool, qh: &QueueHandle<State>) {
        let source = self.manager.create_data_source(qh, ());
        match &content {
            Content::Text(_) => TEXT.iter().for_each(|m| source.offer((*m).into())),
            Content::Image { mime, .. } => source.offer(mime.clone()),
        }
        if primary {
            self.device.set_primary_selection(Some(&source));
        } else {
            self.device.set_selection(Some(&source));
        }
        self.own[usize::from(primary)] = Some(source.id());
        self.sources.insert(source.id(), (content, primary));
    }
}

fn run(
    conn: Connection,
    mut queue: wayland_client::EventQueue<State>,
    mut state: State,
    commands: mpsc::Receiver<Command>,
    mut wake: std::os::unix::net::UnixStream,
    copied: async_channel::Sender<Copied>,
    options: Options,
) -> anyhow::Result<()> {
    let qh = queue.handle();
    loop {
        queue.dispatch_pending(&mut state)?;
        if state.finished {
            anyhow::bail!("the compositor ended the data-control device");
        }

        for (offer, primary) in std::mem::take(&mut state.pending) {
            let Some(offer) = offer else { continue };
            let mimes = state
                .offers
                .remove(&offer.id())
                .map(|(_, m)| m)
                .unwrap_or_default();
            let ours = state.own[usize::from(primary)]
                .as_ref()
                .is_some_and(|id| state.sources.contains_key(id));
            let content = if ours {
                None
            } else {
                read(&conn, &offer, &mimes, options)
            };
            offer.destroy();
            let Some(content) = content else { continue };
            if options.sync_primary && matches!(content, Content::Text(_)) {
                // The other selection gets the same text.
                state.set(content.clone(), !primary, &qh);
            }
            if copied.send_blocking(Copied { content, primary }).is_err() {
                return Ok(()); // the application has quit
            }
        }

        while let Ok(Command::Set(content)) = commands.try_recv() {
            if options.sync_primary && matches!(content, Content::Text(_)) {
                state.set(content.clone(), true, &qh);
            }
            state.set(content, false, &qh);
        }

        conn.flush()?;
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let (wayland_ready, wake_ready) = {
            let display_fd = guard.connection_fd();
            let mut fds = [
                PollFd::new(&display_fd, PollFlags::IN),
                PollFd::new(&wake, PollFlags::IN),
            ];
            poll(&mut fds, None)?;
            (!fds[0].revents().is_empty(), !fds[1].revents().is_empty())
        };
        if wayland_ready {
            guard.read()?;
        } else {
            drop(guard);
        }
        if wake_ready {
            let mut buf = [0u8; 64];
            while matches!(wake.read(&mut buf), Ok(n) if n > 0) {}
        }
    }
}

/// Reads a selection: text when offered, else an image; None for secrets,
/// empty or oversized copies, and other types.
fn read(
    conn: &Connection,
    offer: &ZwlrDataControlOfferV1,
    mimes: &[String],
    options: Options,
) -> Option<Content> {
    if options.ignore_secrets && mimes.iter().any(|m| m == SECRET) {
        log::debug!("skipping a copy marked as secret");
        return None;
    }
    let text = TEXT.iter().find(|t| mimes.iter().any(|m| m == *t));
    let image = [
        "image/png",
        "image/jpeg",
        "image/webp",
        "image/gif",
        "image/bmp",
    ]
    .iter()
    .find(|t| mimes.iter().any(|m| m == *t));
    let mime = text.or(image)?;
    let bytes = receive(conn, offer, mime, options.max_bytes)?;
    if bytes.is_empty() {
        return None;
    }
    if text.is_some() {
        Some(Content::Text(String::from_utf8_lossy(&bytes).into_owned()))
    } else {
        Some(Content::Image {
            mime: (*mime).into(),
            bytes,
        })
    }
}

/// The offer's data in `mime`, through a pipe; gives up after two seconds
/// of silence or past `max` bytes.
fn receive(
    conn: &Connection,
    offer: &ZwlrDataControlOfferV1,
    mime: &str,
    max: usize,
) -> Option<Vec<u8>> {
    let (reader, writer): (OwnedFd, OwnedFd) = rustix::pipe::pipe().ok()?;
    offer.receive(mime.into(), writer.as_fd());
    drop(writer);
    conn.flush().ok()?;
    let mut file = std::fs::File::from(reader);
    let mut data = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut last = Instant::now();
    loop {
        let ready = {
            let mut fds = [PollFd::new(&file, PollFlags::IN)];
            let timeout = rustix::time::Timespec {
                tv_sec: 0,
                tv_nsec: 200_000_000,
            };
            poll(&mut fds, Some(&timeout)).ok()?
        };
        if ready == 0 {
            if last.elapsed() > Duration::from_secs(2) {
                log::warn!("a copy took too long to read; skipped");
                return None;
            }
            continue;
        }
        match file.read(&mut buf) {
            Ok(0) => return Some(data),
            Ok(n) => {
                data.extend_from_slice(&buf[..n]);
                last = Instant::now();
                if data.len() > max {
                    log::info!("a copy larger than {max} bytes; not kept");
                    return None;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
}

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

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlManagerV1,
        _: wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        event: device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            device::Event::DataOffer { id } => {
                state.offers.insert(id.id(), (id, Vec::new()));
            }
            device::Event::Selection { id } => state.pending.push((id, false)),
            device::Event::PrimarySelection { id } => state.pending.push((id, true)),
            device::Event::Finished => state.finished = true,
            _ => {}
        }
    }

    event_created_child!(State, ZwlrDataControlDeviceV1, [
        device::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ZwlrDataControlOfferV1, ()> for State {
    fn event(
        state: &mut Self,
        offer: &ZwlrDataControlOfferV1,
        event: offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let offer::Event::Offer { mime_type } = event
            && let Some((_, mimes)) = state.offers.get_mut(&offer.id())
        {
            mimes.push(mime_type);
        }
    }
}

impl Dispatch<ZwlrDataControlSourceV1, ()> for State {
    fn event(
        state: &mut Self,
        source: &ZwlrDataControlSourceV1,
        event: source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            source::Event::Send { mime_type, fd } => {
                let Some((content, _)) = state.sources.get(&source.id()) else {
                    return;
                };
                let bytes = match content {
                    Content::Text(text) => text.clone().into_bytes(),
                    Content::Image { mime, bytes } if *mime == mime_type => bytes.clone(),
                    Content::Image { .. } => return,
                };
                // Written on a thread of its own: the reader may be slow,
                // and the watcher must keep answering the compositor.
                std::thread::spawn(move || {
                    let mut file = std::fs::File::from(fd);
                    let _ = file.write_all(&bytes);
                });
            }
            source::Event::Cancelled => {
                state.sources.remove(&source.id());
                source.destroy();
            }
            _ => {}
        }
    }
}
