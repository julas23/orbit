//! Offering the capture on the clipboard, with wlr-data-control.
//!
//! The clipboard on Wayland is served by whoever copied: the capture stays
//! pasteable only while this process offers it. So it does, on a thread of
//! its own, until something else is copied; then `done` is signalled and the
//! program may end. Data-control needs no window or keyboard focus, so this
//! works for captures taken straight from a key binding.

use std::io::Write;

use anyhow::Context;
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_registry, wl_seat},
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self as device, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
    zwlr_data_control_source_v1::{self as source, ZwlrDataControlSourceV1},
};

/// Offers `png` as the clipboard; `done` gets a message once it is
/// replaced by another copy.
pub fn offer(png: Vec<u8>, done: async_channel::Sender<()>) -> anyhow::Result<()> {
    let conn = Connection::connect_to_env().context("connecting to the Wayland display")?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
    let qh = queue.handle();
    let manager: ZwlrDataControlManagerV1 = globals
        .bind(&qh, 1..=2, ())
        .context("the compositor has no wlr-data-control")?;
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).context("no seat")?;
    let device = manager.get_data_device(&seat, &qh, ());
    let source = manager.create_data_source(&qh, ());
    source.offer("image/png".into());
    device.set_selection(Some(&source));
    conn.flush()?;

    std::thread::Builder::new()
        .name("orbit-blink-clipboard".into())
        .spawn(move || {
            let mut state = State {
                png,
                cancelled: false,
            };
            while !state.cancelled {
                if let Err(err) = queue.blocking_dispatch(&mut state) {
                    log::warn!("clipboard: {err}");
                    break;
                }
            }
            let _ = done.send_blocking(());
        })?;
    Ok(())
}

struct State {
    png: Vec<u8>,
    cancelled: bool,
}

impl Dispatch<ZwlrDataControlSourceV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlSourceV1,
        event: source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            source::Event::Send { fd, .. } => {
                let png = state.png.clone();
                // A thread per paste: a slow reader must not stop us
                // answering the compositor.
                std::thread::spawn(move || {
                    let mut file = std::fs::File::from(fd);
                    let _ = file.write_all(&png);
                });
            }
            source::Event::Cancelled => state.cancelled = true,
            _ => {}
        }
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        event: device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Offers of other copies: not wanted, let them go.
        if let device::Event::DataOffer { id } = event {
            id.destroy();
        }
    }

    wayland_client::event_created_child!(State, ZwlrDataControlDeviceV1, [
        device::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ()),
    ]);
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
    wl_seat::WlSeat,
    ZwlrDataControlManagerV1,
    ZwlrDataControlOfferV1
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
