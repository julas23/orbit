//! wlr-foreign-toplevel-management: the windows, each with its title,
//! application, monitors and state (maximized, minimised, focused,
//! fullscreen), for taskbars and docks — Waybar's taskbar, nwg-dock,
//! sfwbar — which can also bring one forward, close, minimise, maximize
//! or make it fullscreen. Orbit's own panel and dock speak the sway IPC;
//! this is for the others. A sandboxed client (`protocols::sandboxed`)
//! does not see it.

use std::collections::HashMap;

use smithay::{
    desktop::Window,
    output::Output,
    reexports::{
        wayland_protocols_wlr::foreign_toplevel::v1::server::{
            zwlr_foreign_toplevel_handle_v1::{self, State as WlrState, ZwlrForeignToplevelHandleV1},
            zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
        },
        wayland_server::{Client, DataInit, DisplayHandle, New, Resource, backend::ClientId},
    },
    wayland::{Dispatch2, GlobalDispatch2},
};

use crate::state::Orbit;

const VERSION: u32 = 3;

/// What a client was last told of a window.
#[derive(Clone, PartialEq, Default)]
struct Told {
    title: String,
    app_id: String,
    states: Vec<u32>,
    outputs: Vec<String>,
}

#[derive(Default)]
pub struct Toplevels {
    managers: Vec<ZwlrForeignToplevelManagerV1>,
    /// Each window's handles, one a manager, and what they were told.
    handles: HashMap<Window, (Vec<ZwlrForeignToplevelHandleV1>, Told)>,
}

pub struct ManagerGlobal;
pub struct ManagerData;

impl Toplevels {
    pub fn new(dh: &DisplayHandle) -> Self {
        dh.create_global::<Orbit, ZwlrForeignToplevelManagerV1, ManagerGlobal>(VERSION, ManagerGlobal);
        Self::default()
    }
}

impl Orbit {
    /// What the clients should know of `window` now.
    fn told(&self, window: &Window) -> Told {
        let (title, app_id) = crate::protocols::names(window);
        let mut states = Vec::new();
        if crate::fullscreen::is_maximized(window) {
            states.push(WlrState::Maximized as u32);
        }
        if self.scratchpad.contains(window) {
            states.push(WlrState::Minimized as u32);
        }
        if self.focused_window().as_ref() == Some(window) {
            states.push(WlrState::Activated as u32);
        }
        if crate::fullscreen::is_fullscreen(window) {
            states.push(WlrState::Fullscreen as u32);
        }
        let outputs = self
            .workspace_of(window)
            .filter(|n| self.is_visible(*n))
            .and_then(|n| self.ws_output(n))
            .into_iter()
            .collect();
        Told {
            title,
            app_id,
            states,
            outputs,
        }
    }

    fn outputs_of(
        &self,
        client: &Client,
        names: &[String],
    ) -> Vec<smithay::reexports::wayland_server::protocol::wl_output::WlOutput> {
        names
            .iter()
            .filter_map(|n| self.output_named(n))
            .flat_map(|o: Output| o.client_outputs(client).collect::<Vec<_>>())
            .collect()
    }

    /// A new handle for `window` on `manager`, told everything.
    fn announce(
        &self,
        manager: &ZwlrForeignToplevelManagerV1,
        window: &Window,
        told: &Told,
    ) -> Option<ZwlrForeignToplevelHandleV1> {
        let client = manager.client()?;
        let handle = client
            .create_resource::<ZwlrForeignToplevelHandleV1, _, Orbit>(
                &self.display_handle,
                manager.version(),
                window.clone(),
            )
            .ok()?;
        manager.toplevel(&handle);
        handle.title(told.title.clone());
        handle.app_id(told.app_id.clone());
        for output in self.outputs_of(&client, &told.outputs) {
            handle.output_enter(&output);
        }
        handle.state(states_bytes(&told.states));
        handle.done();
        Some(handle)
    }

    /// `window` came or changed: every manager hears what is new.
    pub fn toplevel_updated(&mut self, window: &Window) {
        if !self.all_windows().any(|w| w == window) && !self.scratchpad.contains(window) {
            return;
        }
        let told = self.told(window);
        let managers = self.toplevels.managers.clone();
        let Some((handles, before)) = self.toplevels.handles.remove(window) else {
            let handles = managers
                .iter()
                .filter_map(|m| self.announce(m, window, &told))
                .collect();
            self.toplevels.handles.insert(window.clone(), (handles, told));
            return;
        };
        if before == told {
            self.toplevels.handles.insert(window.clone(), (handles, before));
            return;
        }
        for handle in &handles {
            let Some(client) = handle.client() else { continue };
            if before.title != told.title {
                handle.title(told.title.clone());
            }
            if before.app_id != told.app_id {
                handle.app_id(told.app_id.clone());
            }
            if before.outputs != told.outputs {
                for output in self.outputs_of(&client, &before.outputs) {
                    handle.output_leave(&output);
                }
                for output in self.outputs_of(&client, &told.outputs) {
                    handle.output_enter(&output);
                }
            }
            handle.state(states_bytes(&told.states));
            handle.done();
        }
        self.toplevels.handles.insert(window.clone(), (handles, told));
    }

    /// Every window looked at again (the focus or the workspaces changed).
    pub fn toplevels_refresh(&mut self) {
        if self.toplevels.managers.is_empty() {
            return;
        }
        let windows: Vec<Window> = self.toplevels.handles.keys().cloned().collect();
        for w in windows {
            self.toplevel_updated(&w);
        }
    }

    pub fn toplevel_closed(&mut self, window: &Window) {
        if let Some((handles, _)) = self.toplevels.handles.remove(window) {
            for handle in handles {
                handle.closed();
            }
        }
    }
}

fn states_bytes(states: &[u32]) -> Vec<u8> {
    states.iter().flat_map(|s| s.to_ne_bytes()).collect()
}

impl GlobalDispatch2<ZwlrForeignToplevelManagerV1, Orbit> for ManagerGlobal {
    fn bind(
        &self,
        state: &mut Orbit,
        _dh: &DisplayHandle,
        _client: &Client,
        manager: New<ZwlrForeignToplevelManagerV1>,
        data_init: &mut DataInit<'_, Orbit>,
    ) {
        let manager = data_init.init(manager, ManagerData);
        let windows: Vec<Window> = state.all_windows().chain(state.scratchpad.iter()).cloned().collect();
        for window in windows {
            let told = state.told(&window);
            if let Some(handle) = state.announce(&manager, &window, &told) {
                state
                    .toplevels
                    .handles
                    .entry(window)
                    .or_insert_with(|| (Vec::new(), told))
                    .0
                    .push(handle);
            }
        }
        state.toplevels.managers.push(manager);
    }

    fn can_view(&self, client: &Client) -> bool {
        !crate::protocols::sandboxed(client)
    }
}

impl Dispatch2<ZwlrForeignToplevelManagerV1, Orbit> for ManagerData {
    fn request(
        &self,
        state: &mut Orbit,
        _client: &Client,
        manager: &ZwlrForeignToplevelManagerV1,
        request: zwlr_foreign_toplevel_manager_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Orbit>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Request::Stop = request {
            state.toplevels.managers.retain(|m| m != manager);
            manager.finished();
        }
    }

    fn destroyed(&self, state: &mut Orbit, _client: ClientId, manager: &ZwlrForeignToplevelManagerV1) {
        state.toplevels.managers.retain(|m| m != manager);
    }
}

impl Dispatch2<ZwlrForeignToplevelHandleV1, Orbit> for Window {
    fn request(
        &self,
        state: &mut Orbit,
        _client: &Client,
        _handle: &ZwlrForeignToplevelHandleV1,
        request: zwlr_foreign_toplevel_handle_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Orbit>,
    ) {
        use zwlr_foreign_toplevel_handle_v1::Request;
        let window = self.clone();
        let known = state.all_windows().any(|w| w == &window) || state.scratchpad.contains(&window);
        if !known || state.locked() {
            return;
        }
        match request {
            Request::Activate { .. } | Request::UnsetMinimized => state.focus_anywhere(&window),
            Request::Close => state.close_window(&window),
            Request::SetMinimized => state.move_to_scratchpad(&window),
            Request::SetMaximized => state.set_maximized(&window, true),
            Request::UnsetMaximized => state.set_maximized(&window, false),
            Request::SetFullscreen { .. } => state.set_fullscreen(&window, true),
            Request::UnsetFullscreen => state.set_fullscreen(&window, false),
            Request::SetRectangle { .. } | Request::Destroy => {}
            _ => {}
        }
        state.toplevel_updated(&window);
    }

    fn destroyed(&self, state: &mut Orbit, _client: ClientId, handle: &ZwlrForeignToplevelHandleV1) {
        if let Some((handles, _)) = state.toplevels.handles.get_mut(self) {
            handles.retain(|h| h != handle);
        }
    }
}
