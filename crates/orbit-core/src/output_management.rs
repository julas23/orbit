//! The monitors for other programs to see and set (`wlr-output-management`):
//! wlr-randr, kanshi, nwg-displays and the like list each monitor's modes,
//! place, turn and scale, and change them. A change applies as `core.yaml`
//! does it (`outputs:`), for this session only: the file is left as it is.
//! A monitor cannot be switched off yet, so a configuration that asks for
//! it fails rather than pretending.
//!
//! Ported from niri `src/protocols/output_management.rs` @ aee25c9
//! (GPL-3.0-or-later, Copyright (c) Ivan Molodetskikh and contributors),
//! with the monitors known by name and without adaptive sync.

use std::{
    collections::{HashMap, hash_map::Entry},
    iter::zip,
    mem,
};

use smithay::{
    output::Output,
    reexports::{
        wayland_protocols_wlr::output_management::v1::server::{
            zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
            zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
            zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
            zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
            zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
        },
        wayland_server::{
            Client, DataInit, DisplayHandle, New, Resource, WEnum, backend::ClientId,
            protocol::wl_output::Transform as WlTransform,
        },
    },
    utils::Transform,
    wayland::{Dispatch2, GlobalDispatch2},
};

use crate::state::Orbit;

const VERSION: u32 = 4;

/// A monitor as the protocol tells it.
#[derive(Debug, Clone, PartialEq)]
pub struct Head {
    pub make: String,
    pub model: String,
    pub serial: String,
    /// In millimetres.
    pub physical: (i32, i32),
    pub modes: Vec<ModeInfo>,
    pub current: Option<usize>,
    pub position: (i32, i32),
    pub transform: Transform,
    pub scale: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModeInfo {
    pub width: i32,
    pub height: i32,
    /// In mHz.
    pub refresh: i32,
    pub preferred: bool,
}

/// What a configuration asks of one monitor.
#[derive(Debug, Clone, Default)]
pub struct Change {
    pub off: bool,
    /// Width, height and rate in mHz.
    pub mode: Option<(i32, i32, i32)>,
    pub position: Option<(i32, i32)>,
    pub transform: Option<Transform>,
    pub scale: Option<f64>,
}

struct ClientData {
    heads: HashMap<String, (ZwlrOutputHeadV1, Vec<ZwlrOutputModeV1>)>,
    confs: HashMap<ZwlrOutputConfigurationV1, Configuring>,
    manager: ZwlrOutputManagerV1,
}

enum Configuring {
    Ongoing(HashMap<String, Change>),
    Finished,
}

pub struct OutputManagement {
    display: DisplayHandle,
    serial: u32,
    clients: HashMap<ClientId, ClientData>,
    current: HashMap<String, Head>,
}

/// The `zwlr_output_manager_v1` global.
pub struct ManagerGlobal;
/// A bound manager.
pub struct ManagerData;
/// A configuration, with the serial it was made against.
pub struct ConfigurationData {
    serial: u32,
}
/// One monitor in a configuration.
pub enum ConfigurationHead {
    Cancelled,
    Ok(String, ZwlrOutputConfigurationV1),
}
/// A mode of a head.
pub struct ModeData;

impl OutputManagement {
    pub fn new(display: &DisplayHandle) -> Self {
        display.create_global::<Orbit, ZwlrOutputManagerV1, ManagerGlobal>(VERSION, ManagerGlobal);
        Self { display: display.clone(), serial: 0, clients: HashMap::new(), current: HashMap::new() }
    }

    /// The monitors as they are now: what changed is told to every client.
    pub fn update(&mut self, heads: HashMap<String, Head>) {
        let mut changed = false;
        for (name, head) in &heads {
            let Some(old) = self.current.get(name) else {
                changed = true;
                for data in self.clients.values_mut() {
                    if let Some(client) = data.manager.client() {
                        send_head(&self.display, &client, data, name, head);
                    }
                }
                continue;
            };
            if old == head {
                continue;
            }
            changed = true;
            for data in self.clients.values_mut() {
                let Some((wl_head, modes)) = data.heads.get_mut(name) else { continue };
                if old.modes != head.modes {
                    // Ends on the shorter list; then the ones added or gone.
                    let common = zip(modes.iter(), &head.modes).len();
                    for (wl_mode, mode) in zip(modes.iter(), &head.modes) {
                        wl_mode.size(mode.width, mode.height);
                        wl_mode.refresh(mode.refresh);
                    }
                    if let Some(client) = data.manager.client() {
                        for mode in &head.modes[common..] {
                            let Ok(wl_mode) = client.create_resource::<ZwlrOutputModeV1, _, Orbit>(&self.display, wl_head.version(), ModeData) else { continue };
                            wl_head.mode(&wl_mode);
                            wl_mode.size(mode.width, mode.height);
                            wl_mode.refresh(mode.refresh);
                            modes.push(wl_mode);
                        }
                    }
                    for gone in modes.drain(head.modes.len().min(modes.len())..) {
                        gone.finished();
                    }
                }
                if old.current != head.current || old.modes != head.modes {
                    match head.current.and_then(|i| modes.get(i)) {
                        Some(mode) => {
                            wl_head.enabled(1);
                            wl_head.current_mode(mode);
                        }
                        None => wl_head.enabled(0),
                    }
                }
                if old.position != head.position {
                    wl_head.position(head.position.0, head.position.1);
                }
                if old.transform != head.transform {
                    wl_head.transform(head.transform.into());
                }
                if old.scale != head.scale {
                    wl_head.scale(head.scale);
                }
            }
        }
        for gone in self.current.keys().filter(|name| !heads.contains_key(*name)) {
            changed = true;
            for data in self.clients.values_mut() {
                if let Some((head, modes)) = data.heads.remove(gone) {
                    modes.iter().for_each(|m| m.finished());
                    head.finished();
                }
            }
        }
        if changed {
            self.current = heads;
            self.serial += 1;
            // Those still being made are outdated now; one just used is
            // answered by its own apply.
            for data in self.clients.values() {
                data.manager.done(self.serial);
                for (conf, _) in data.confs.iter().filter(|(_, c)| matches!(c, Configuring::Ongoing(_))) {
                    conf.cancelled();
                }
            }
        }
    }
}

impl Orbit {
    /// Every monitor, as the protocol tells it.
    pub fn heads(&self) -> HashMap<String, Head> {
        self.space
            .outputs()
            .map(|output| {
                let props = output.physical_properties();
                let current = output.current_mode();
                let preferred = output.preferred_mode();
                let mut modes: Vec<_> = output.modes();
                // A monitor whose modes were not listed still has the one it shows.
                if let Some(current) = current
                    && !modes.contains(&current)
                {
                    modes.push(current);
                }
                let position = self.space.output_geometry(output).map_or((0, 0), |g| (g.loc.x, g.loc.y));
                let head = Head {
                    make: props.make,
                    model: props.model,
                    serial: props.serial_number,
                    physical: (props.size.w, props.size.h),
                    current: current.and_then(|c| modes.iter().position(|m| *m == c)),
                    modes: modes
                        .iter()
                        .map(|m| ModeInfo { width: m.size.w, height: m.size.h, refresh: m.refresh, preferred: Some(*m) == preferred })
                        .collect(),
                    position,
                    transform: output.current_transform(),
                    scale: output.current_scale().fractional_scale(),
                };
                (output.name(), head)
            })
            .collect()
    }

    /// Tells the protocol's clients the monitors as they are now.
    pub fn output_management_changed(&mut self) {
        let heads = self.heads();
        self.output_management.update(heads);
    }

    /// A configuration from a client: true once it took effect. Kept for
    /// this session, as `outputs:` in core.yaml would say it.
    fn apply_heads(&mut self, changes: HashMap<String, Change>) -> bool {
        if changes.values().any(|c| c.off) {
            tracing::info!("output management: switching a monitor off is not done yet");
            return false;
        }
        for (name, change) in &changes {
            let Some(output) = self.output_named(name) else { return false };
            if let Some((w, h, refresh)) = change.mode
                && !self.set_mode(&output, (w, h), refresh)
            {
                return false;
            }
            let config = self.config.outputs.entry(name.clone()).or_default();
            if let Some(at) = change.position {
                config.position = Some(at);
            }
            if let Some(transform) = change.transform {
                config.transform = Some(crate::config::transform_name(transform).to_owned());
            }
            if let Some(scale) = change.scale {
                config.scale = Some(scale);
            }
        }
        self.place_outputs();
        self.renumber();
        self.arrange();
        self.output_event();
        true
    }

    /// Switches `output` to its mode of `size` at `refresh` mHz: the
    /// screen's (DRM), or a virtual monitor's.
    fn set_mode(&mut self, output: &Output, size: (i32, i32), refresh: i32) -> bool {
        if output.current_mode().is_some_and(|m| m.size == size.into() && m.refresh == refresh) {
            return true;
        }
        crate::udev::set_mode(self, output, size, refresh)
    }
}

/// Sends `head` (called `name`) to a client, with its modes.
fn send_head(display: &DisplayHandle, client: &Client, data: &mut ClientData, name: &str, head: &Head) {
    let Ok(wl_head) = client.create_resource::<ZwlrOutputHeadV1, _, Orbit>(display, data.manager.version(), name.to_owned()) else { return };
    data.manager.head(&wl_head);
    wl_head.name(name.to_owned());
    wl_head.description(format!("{} - {} - {name}", head.make, head.model));
    if head.physical.0 > 0 && head.physical.1 > 0 {
        wl_head.physical_size(head.physical.0, head.physical.1);
    }
    let mut modes = Vec::with_capacity(head.modes.len());
    for (i, mode) in head.modes.iter().enumerate() {
        let Ok(wl_mode) = client.create_resource::<ZwlrOutputModeV1, _, Orbit>(display, wl_head.version(), ModeData) else { continue };
        wl_head.mode(&wl_mode);
        wl_mode.size(mode.width, mode.height);
        wl_mode.refresh(mode.refresh);
        if mode.preferred {
            wl_mode.preferred();
        }
        if Some(i) == head.current {
            wl_head.current_mode(&wl_mode);
        }
        modes.push(wl_mode);
    }
    wl_head.position(head.position.0, head.position.1);
    wl_head.transform(head.transform.into());
    wl_head.scale(head.scale);
    wl_head.enabled(i32::from(head.current.is_some()));
    if wl_head.version() >= zwlr_output_head_v1::EVT_MAKE_SINCE {
        wl_head.make(head.make.clone());
    }
    if wl_head.version() >= zwlr_output_head_v1::EVT_MODEL_SINCE {
        wl_head.model(head.model.clone());
    }
    if wl_head.version() >= zwlr_output_head_v1::EVT_SERIAL_NUMBER_SINCE && !head.serial.is_empty() {
        wl_head.serial_number(head.serial.clone());
    }
    data.heads.insert(name.to_owned(), (wl_head, modes));
}

impl GlobalDispatch2<ZwlrOutputManagerV1, Orbit> for ManagerGlobal {
    fn bind(&self, state: &mut Orbit, dh: &DisplayHandle, client: &Client, manager: New<ZwlrOutputManagerV1>, data_init: &mut DataInit<'_, Orbit>) {
        let manager = data_init.init(manager, ManagerData);
        // The monitors as they are, the first client finding the list empty.
        let heads = state.heads();
        let om = &mut state.output_management;
        if om.current.is_empty() {
            om.current = heads;
        }
        let mut data = ClientData { heads: HashMap::new(), confs: HashMap::new(), manager: manager.clone() };
        for (name, head) in &om.current {
            send_head(dh, client, &mut data, name, head);
        }
        om.clients.insert(client.id(), data);
        manager.done(om.serial);
    }
}

impl Dispatch2<ZwlrOutputManagerV1, Orbit> for ManagerData {
    fn request(&self, state: &mut Orbit, client: &Client, _manager: &ZwlrOutputManagerV1, request: zwlr_output_manager_v1::Request, _dh: &DisplayHandle, data_init: &mut DataInit<'_, Orbit>) {
        let om = &mut state.output_management;
        match request {
            zwlr_output_manager_v1::Request::CreateConfiguration { id, serial } => {
                let conf = data_init.init(id, ConfigurationData { serial });
                if let Some(data) = om.clients.get_mut(&client.id()) {
                    if serial != om.serial {
                        conf.cancelled();
                    }
                    data.confs.insert(conf, Configuring::Ongoing(HashMap::new()));
                }
            }
            zwlr_output_manager_v1::Request::Stop => {
                if let Some(data) = om.clients.remove(&client.id()) {
                    data.manager.finished();
                }
            }
            _ => {}
        }
    }

    fn destroyed(&self, state: &mut Orbit, client: ClientId, _manager: &ZwlrOutputManagerV1) {
        state.output_management.clients.remove(&client);
    }
}

impl Dispatch2<ZwlrOutputConfigurationV1, Orbit> for ConfigurationData {
    fn request(&self, state: &mut Orbit, client: &Client, conf: &ZwlrOutputConfigurationV1, request: zwlr_output_configuration_v1::Request, _dh: &DisplayHandle, data_init: &mut DataInit<'_, Orbit>) {
        let om = &mut state.output_management;
        let outdated = self.serial != om.serial;
        let known = |om: &OutputManagement, head: &ZwlrOutputHeadV1| head.data::<String>().filter(|name| om.current.contains_key(*name)).cloned();
        match request {
            zwlr_output_configuration_v1::Request::EnableHead { id, head } => {
                let name = known(om, &head);
                let ongoing = om.clients.get_mut(&client.id()).and_then(|d| d.confs.get_mut(conf));
                let (Some(name), Some(Configuring::Ongoing(changes)), false) = (name, ongoing, outdated) else {
                    data_init.init(id, ConfigurationHead::Cancelled);
                    return;
                };
                match changes.entry(name.clone()) {
                    Entry::Occupied(_) => {
                        data_init.init(id, ConfigurationHead::Cancelled);
                        conf.post_error(zwlr_output_configuration_v1::Error::AlreadyConfiguredHead, "head has been already configured");
                    }
                    Entry::Vacant(entry) => {
                        entry.insert(Change::default());
                        data_init.init(id, ConfigurationHead::Ok(name, conf.clone()));
                    }
                }
            }
            zwlr_output_configuration_v1::Request::DisableHead { head } => {
                let name = known(om, &head);
                let ongoing = om.clients.get_mut(&client.id()).and_then(|d| d.confs.get_mut(conf));
                let (Some(name), Some(Configuring::Ongoing(changes)), false) = (name, ongoing, outdated) else { return };
                match changes.entry(name) {
                    Entry::Occupied(_) => conf.post_error(zwlr_output_configuration_v1::Error::AlreadyConfiguredHead, "head has been already configured"),
                    Entry::Vacant(entry) => {
                        entry.insert(Change { off: true, ..Change::default() });
                    }
                }
            }
            zwlr_output_configuration_v1::Request::Apply | zwlr_output_configuration_v1::Request::Test => {
                let apply = matches!(request, zwlr_output_configuration_v1::Request::Apply);
                if outdated {
                    conf.cancelled();
                    return;
                }
                let Some(slot) = om.clients.get_mut(&client.id()).and_then(|d| d.confs.get_mut(conf)) else { return };
                let Configuring::Ongoing(changes) = mem::replace(slot, Configuring::Finished) else {
                    conf.post_error(zwlr_output_configuration_v1::Error::AlreadyUsed, "configuration had already been used");
                    return;
                };
                // A test only checks; an apply must also take effect.
                let possible = !changes.values().any(|c| c.off) && !changes.is_empty();
                if possible && (!apply || state.apply_heads(changes)) {
                    conf.succeeded();
                } else {
                    conf.failed();
                }
            }
            zwlr_output_configuration_v1::Request::Destroy => {
                if let Some(data) = om.clients.get_mut(&client.id()) {
                    data.confs.remove(conf);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch2<ZwlrOutputConfigurationHeadV1, Orbit> for ConfigurationHead {
    fn request(&self, state: &mut Orbit, client: &Client, conf_head: &ZwlrOutputConfigurationHeadV1, request: zwlr_output_configuration_head_v1::Request, _dh: &DisplayHandle, _data_init: &mut DataInit<'_, Orbit>) {
        let ConfigurationHead::Ok(name, conf) = self else { return };
        let om = &mut state.output_management;
        if conf.data::<ConfigurationData>().is_none_or(|d| d.serial != om.serial) {
            return;
        }
        let current = om.current.get(name).cloned();
        let Some(data) = om.clients.get_mut(&client.id()) else { return };
        let mode_index = |mode: &ZwlrOutputModeV1| data.heads.get(name).and_then(|(_, modes)| modes.iter().position(|m| m.id() == mode.id()));
        let picked = match &request {
            zwlr_output_configuration_head_v1::Request::SetMode { mode } => Some(mode_index(mode)),
            _ => None,
        };
        let Some(Configuring::Ongoing(changes)) = data.confs.get_mut(conf) else {
            conf.post_error(zwlr_output_configuration_v1::Error::AlreadyUsed, "configuration had already been used");
            return;
        };
        let Some(change) = changes.get_mut(name) else { return };
        match request {
            zwlr_output_configuration_head_v1::Request::SetMode { .. } => {
                let mode = picked.flatten().and_then(|i| current.as_ref()?.modes.get(i).copied());
                match mode {
                    Some(mode) => change.mode = Some((mode.width, mode.height, mode.refresh)),
                    None => conf_head.post_error(zwlr_output_configuration_head_v1::Error::InvalidMode, "failed to find requested mode"),
                }
            }
            // A rate of 0: any; the fastest the monitor has at that size.
            zwlr_output_configuration_head_v1::Request::SetCustomMode { width, height, refresh } if width > 0 && height > 0 => {
                change.mode = Some((width, height, refresh));
            }
            zwlr_output_configuration_head_v1::Request::SetPosition { x, y } => change.position = Some((x, y)),
            zwlr_output_configuration_head_v1::Request::SetTransform { transform } => match transform {
                WEnum::Value(t) => change.transform = Some(from_wl(t)),
                WEnum::Unknown(_) => conf_head.post_error(zwlr_output_configuration_head_v1::Error::InvalidTransform, "unknown transform value"),
            },
            zwlr_output_configuration_head_v1::Request::SetScale { scale } => {
                if scale > 0.0 {
                    change.scale = Some(scale);
                } else {
                    conf_head.post_error(zwlr_output_configuration_head_v1::Error::InvalidScale, "scale is negative or zero");
                }
            }
            _ => {}
        }
    }
}

impl Dispatch2<ZwlrOutputHeadV1, Orbit> for String {
    fn request(&self, _state: &mut Orbit, _client: &Client, _head: &ZwlrOutputHeadV1, _request: zwlr_output_head_v1::Request, _dh: &DisplayHandle, _data_init: &mut DataInit<'_, Orbit>) {}

    fn destroyed(&self, state: &mut Orbit, client: ClientId, _head: &ZwlrOutputHeadV1) {
        if let Some(data) = state.output_management.clients.get_mut(&client) {
            data.heads.remove(self);
        }
    }
}

impl Dispatch2<ZwlrOutputModeV1, Orbit> for ModeData {
    fn request(&self, _state: &mut Orbit, _client: &Client, _mode: &ZwlrOutputModeV1, _request: zwlr_output_mode_v1::Request, _dh: &DisplayHandle, _data_init: &mut DataInit<'_, Orbit>) {}
}

fn from_wl(transform: WlTransform) -> Transform {
    match transform {
        WlTransform::_90 => Transform::_90,
        WlTransform::_180 => Transform::_180,
        WlTransform::_270 => Transform::_270,
        WlTransform::Flipped => Transform::Flipped,
        WlTransform::Flipped90 => Transform::Flipped90,
        WlTransform::Flipped180 => Transform::Flipped180,
        WlTransform::Flipped270 => Transform::Flipped270,
        _ => Transform::Normal,
    }
}
