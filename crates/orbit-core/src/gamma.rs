//! wlr-gamma-control: a monitor's gamma ramp set by a client — night light
//! (gammastep, wlsunset), colour calibration. One client a monitor at a
//! time; when it goes, the ramp goes back to neutral. On the screen only
//! (the nested window has no ramp of its own), and never for a sandbox.

use std::{collections::HashMap, os::unix::fs::FileExt};

use smithay::{
    output::Output,
    reexports::{
        wayland_protocols_wlr::gamma_control::v1::server::{
            zwlr_gamma_control_manager_v1::{self, ZwlrGammaControlManagerV1},
            zwlr_gamma_control_v1::{self, ZwlrGammaControlV1},
        },
        wayland_server::{Client, DataInit, DisplayHandle, New, Resource, backend::ClientId},
    },
    wayland::{Dispatch2, GlobalDispatch2},
};

use crate::state::Orbit;

/// The control of each monitor, by its name.
#[derive(Default)]
pub struct Gamma {
    owners: HashMap<String, ZwlrGammaControlV1>,
}

pub struct ManagerGlobal;
pub struct ManagerData;
/// The monitor a control is for (None: it failed from the start).
pub struct ControlData(Option<Output>);

impl Gamma {
    pub fn new(dh: &DisplayHandle) -> Self {
        dh.create_global::<Orbit, ZwlrGammaControlManagerV1, ManagerGlobal>(1, ManagerGlobal);
        Self::default()
    }
}

impl GlobalDispatch2<ZwlrGammaControlManagerV1, Orbit> for ManagerGlobal {
    fn bind(
        &self,
        _state: &mut Orbit,
        _dh: &DisplayHandle,
        _client: &Client,
        manager: New<ZwlrGammaControlManagerV1>,
        data_init: &mut DataInit<'_, Orbit>,
    ) {
        data_init.init(manager, ManagerData);
    }

    fn can_view(&self, client: &Client) -> bool {
        !crate::protocols::sandboxed(client)
    }
}

impl Dispatch2<ZwlrGammaControlManagerV1, Orbit> for ManagerData {
    fn request(
        &self,
        state: &mut Orbit,
        _client: &Client,
        _manager: &ZwlrGammaControlManagerV1,
        request: zwlr_gamma_control_manager_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Orbit>,
    ) {
        if let zwlr_gamma_control_manager_v1::Request::GetGammaControl { id, output } = request {
            let output = Output::from_resource(&output);
            let taken = output
                .as_ref()
                .is_some_and(|o| state.gamma.owners.get(&o.name()).is_some_and(|c| c.is_alive()));
            let size = output.as_ref().map_or(0, |o| crate::udev::gamma_size(state, o));
            if taken || size == 0 {
                let control = data_init.init(id, ControlData(None));
                control.failed();
                return;
            }
            let output = output.expect("checked above");
            let control = data_init.init(id, ControlData(Some(output.clone())));
            control.gamma_size(size);
            state.gamma.owners.insert(output.name(), control);
        }
    }
}

impl Dispatch2<ZwlrGammaControlV1, Orbit> for ControlData {
    fn request(
        &self,
        state: &mut Orbit,
        _client: &Client,
        control: &ZwlrGammaControlV1,
        request: zwlr_gamma_control_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Orbit>,
    ) {
        let zwlr_gamma_control_v1::Request::SetGamma { fd } = request else {
            return;
        };
        let Some(output) = &self.0 else { return };
        let size = crate::udev::gamma_size(state, output) as usize;
        let mut bytes = vec![0u8; size * 3 * 2];
        let file = std::fs::File::from(fd);
        if size == 0 || file.read_exact_at(&mut bytes, 0).is_err() {
            control.failed();
            state.gamma.owners.remove(&output.name());
            return;
        }
        let ramp: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|b| u16::from_ne_bytes([b[0], b[1]]))
            .collect();
        let (red, rest) = ramp.split_at(size);
        let (green, blue) = rest.split_at(size);
        if !crate::udev::set_gamma(state, output, red, green, blue) {
            control.failed();
        }
    }

    fn destroyed(&self, state: &mut Orbit, _client: ClientId, control: &ZwlrGammaControlV1) {
        let Some(output) = &self.0 else { return };
        if state.gamma.owners.get(&output.name()) == Some(control) {
            state.gamma.owners.remove(&output.name());
            crate::udev::reset_gamma(state, output);
        }
    }
}
