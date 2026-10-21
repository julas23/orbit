//! The sway-compatible IPC (PLANO-CORE §5): a Unix socket at
//! `$XDG_RUNTIME_DIR/orbit-ipc.<uid>.<pid>.sock`, exported to clients as
//! `ORBIT_SOCK`, `SWAYSOCK` and `I3SOCK`, so the panel, the blink, `swaymsg`
//! and the rest work unchanged. A compatibility layer: the replies carry the
//! fields clients read, not every field sway has.
//!
//! Requests are read without blocking from the event loop; replies and
//! events are queued per client and written as the socket takes them.

use std::{
    collections::HashMap,
    io::{ErrorKind, Read, Write},
    os::unix::{fs::MetadataExt, net::{UnixListener, UnixStream}},
    path::PathBuf,
};

use orbit_ipc as proto;
use serde_json::{Value, json};
use smithay::{
    output::Output,
    desktop::Window,
    reexports::{
        calloop::{EventLoop, Interest, Mode, PostAction, generic::Generic},
        wayland_server::Resource,
    },
    wayland::{compositor::with_states, shell::xdg::XdgToplevelSurfaceData},
};

use crate::{state::Orbit, workspace};

/// A client with more than this waiting to be written is not reading:
/// dropped.
const BACKLOG: usize = 4 * 1024 * 1024;

#[derive(Default)]
pub struct Ipc {
    /// The socket's path, once listening.
    pub path: Option<PathBuf>,
    clients: HashMap<u64, Client>,
    next: u64,
}

struct Client {
    stream: UnixStream,
    input: Vec<u8>,
    output: Vec<u8>,
    /// The event types it subscribed to.
    events: Vec<u32>,
}

impl Drop for Ipc {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Opens the socket and puts it in the event loop.
pub fn listen(event_loop: &mut EventLoop<'static, Orbit>, state: &mut Orbit) -> anyhow::Result<()> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).ok_or_else(|| anyhow::anyhow!("XDG_RUNTIME_DIR is not set"))?;
    let uid = std::fs::metadata(&runtime)?.uid();
    remove_stale(&runtime, uid);
    let path = runtime.join(format!("orbit-ipc.{uid}.{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    listener.set_nonblocking(true)?;
    state.ipc.path = Some(path.clone());
    tracing::info!("IPC on {}", path.display());
    event_loop
        .handle()
        .insert_source(Generic::new(listener, Interest::READ, Mode::Level), |_, listener, state| {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => accept(state, stream),
                    Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                    Err(err) => {
                        tracing::warn!("IPC accept: {err}");
                        break;
                    }
                }
            }
            Ok(PostAction::Continue)
        })
        .map_err(|e| anyhow::anyhow!("IPC socket in the event loop: {e}"))?;
    Ok(())
}

/// Sockets left by cores that were killed (a signal skips the cleanup).
fn remove_stale(runtime: &std::path::Path, uid: u32) {
    let prefix = format!("orbit-ipc.{uid}.");
    let Ok(entries) = std::fs::read_dir(runtime) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(pid) = name.strip_prefix(&prefix).and_then(|r| r.strip_suffix(".sock")) else { continue };
        if !std::path::Path::new("/proc").join(pid).exists() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn accept(state: &mut Orbit, stream: UnixStream) {
    let (Ok(reader), Ok(())) = (stream.try_clone(), stream.set_nonblocking(true)) else { return };
    let id = state.ipc.next;
    state.ipc.next += 1;
    state.ipc.clients.insert(id, Client { stream, input: Vec::new(), output: Vec::new(), events: Vec::new() });
    let handle = state.loop_handle.clone();
    let inserted = handle.insert_source(Generic::new(reader, Interest::READ, Mode::Level), move |_, reader, state| {
        let mut chunk = [0u8; 8192];
        // Safety: the stream stays open for as long as the source exists.
        let read = unsafe { reader.get_mut().read(&mut chunk) };
        match read {
            Ok(0) => {
                state.ipc.clients.remove(&id);
                return Ok(PostAction::Remove);
            }
            Ok(n) => {
                if let Some(client) = state.ipc.clients.get_mut(&id) {
                    client.input.extend_from_slice(&chunk[..n]);
                }
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock || err.kind() == ErrorKind::Interrupted => return Ok(PostAction::Continue),
            Err(_) => {
                state.ipc.clients.remove(&id);
                return Ok(PostAction::Remove);
            }
        }
        loop {
            let message = match state.ipc.clients.get_mut(&id).map(|c| proto::decode(&mut c.input)) {
                Some(Ok(Some(m))) => m,
                Some(Ok(None)) => break,
                Some(Err(_)) | None => {
                    state.ipc.clients.remove(&id);
                    return Ok(PostAction::Remove);
                }
            };
            let (kind, payload) = message;
            let reply = state.ipc_request(id, kind, &payload);
            state.ipc_send(id, kind, &reply);
        }
        Ok(PostAction::Continue)
    });
    if inserted.is_err() {
        state.ipc.clients.remove(&id);
    }
}

impl Ipc {
    /// Writes what the sockets take now; called at every turn of the loop.
    pub fn flush(&mut self) {
        self.clients.retain(|_, client| write_out(client));
    }
}

/// Writes a client's queue; whether it is still there.
fn write_out(client: &mut Client) -> bool {
    while !client.output.is_empty() {
        match client.stream.write(&client.output) {
            Ok(0) => return false,
            Ok(n) => {
                client.output.drain(..n);
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => break,
            Err(err) if err.kind() == ErrorKind::Interrupted => {}
            Err(_) => return false,
        }
    }
    client.output.len() <= BACKLOG
}

impl Orbit {
    fn ipc_send(&mut self, id: u64, kind: u32, body: &Value) {
        let bytes = proto::encode(kind, body.to_string().as_bytes());
        let alive = match self.ipc.clients.get_mut(&id) {
            Some(client) => {
                client.output.extend(bytes);
                write_out(client)
            }
            None => return,
        };
        if !alive {
            self.ipc.clients.remove(&id);
        }
    }

    /// Sends event `name` to the clients that subscribed to it. The body is
    /// built only when someone listens.
    fn emit(&mut self, name: &str, body: impl FnOnce(&Orbit) -> Value) {
        let Some(kind) = proto::event_type(name) else { return };
        let ids: Vec<u64> = self.ipc.clients.iter().filter(|(_, c)| c.events.contains(&kind)).map(|(id, _)| *id).collect();
        if ids.is_empty() {
            return;
        }
        let body = body(self);
        for id in ids {
            self.ipc_send(id, kind, &body);
        }
    }

    /// A `workspace` event: "init", "focus", "empty"…
    pub fn workspace_event(&mut self, change: &str, num: i32, old: Option<i32>) {
        self.emit("workspace", |s| json!({ "change": change, "current": s.workspace_json(num), "old": old.map(|o| s.workspace_json(o)) }));
    }

    /// A `window` event: "new", "close", "focus", "title", "move".
    pub fn window_event(&mut self, change: &str, window: &Window) {
        let window = window.clone();
        self.emit("window", |s| json!({ "change": change, "container": s.window_json(&window) }));
    }

    /// An `output` event: a monitor came, went or moved. sway says no more.
    pub fn output_event(&mut self) {
        self.emit("output", |_| json!({ "change": "unspecified" }));
    }

    pub fn input_event(&mut self) {
        self.emit("input", |s| json!({ "change": "xkb_layout", "input": s.inputs_json()[0].clone() }));
    }

    /// The reply to a request.
    fn ipc_request(&mut self, client: u64, kind: u32, payload: &[u8]) -> Value {
        let text = String::from_utf8_lossy(payload);
        match kind {
            proto::RUN_COMMAND => Value::Array(crate::command::run(self, &text)),
            proto::GET_WORKSPACES => Value::Array(self.workspaces.iter().map(|w| w.num).collect::<Vec<_>>().into_iter().map(|n| self.workspace_json(n)).collect()),
            proto::SUBSCRIBE => match serde_json::from_str::<Vec<String>>(&text) {
                Ok(names) => {
                    let types: Option<Vec<u32>> = names.iter().map(|n| proto::event_type(n)).collect();
                    match (types, self.ipc.clients.get_mut(&client)) {
                        (Some(types), Some(c)) => {
                            c.events.extend(types);
                            json!({ "success": true })
                        }
                        _ => json!({ "success": false }),
                    }
                }
                Err(_) => json!({ "success": false }),
            },
            proto::GET_OUTPUTS => Value::Array(self.outputs_json(false)),
            proto::GET_TREE => self.tree_json(),
            proto::GET_MARKS => json!([]),
            proto::GET_BAR_CONFIG => json!([]),
            proto::GET_VERSION => json!({
                "human_readable": concat!("orbit-core ", env!("CARGO_PKG_VERSION")),
                "variant": "orbit",
                // What sway clients check; the commands are sway 1.10's.
                "major": 1, "minor": 10, "patch": 0,
                "loaded_config_file_name": crate::config::Config::path().to_string_lossy(),
            }),
            proto::GET_BINDING_MODES => json!(["default"]),
            proto::GET_BINDING_STATE => json!({ "name": "default" }),
            proto::GET_CONFIG => json!({ "config": "" }),
            proto::SEND_TICK => json!({ "success": true }),
            proto::GET_INPUTS => Value::Array(self.inputs_json()),
            proto::GET_SEATS => json!([{ "name": "seat0", "capabilities": 3, "focus": self.focused_window().map(|w| workspace::id(&w)).unwrap_or(0), "devices": self.inputs_json() }]),
            _ => json!({ "success": false, "error": "unknown message type" }),
        }
    }

    /// The rectangle around every monitor.
    fn layout_rect(&self) -> Value {
        let geos: Vec<_> = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).collect();
        let x = geos.iter().map(|g| g.loc.x).min().unwrap_or(0);
        let y = geos.iter().map(|g| g.loc.y).min().unwrap_or(0);
        let right = geos.iter().map(|g| g.loc.x + g.size.w).max().unwrap_or(0);
        let bottom = geos.iter().map(|g| g.loc.y + g.size.h).max().unwrap_or(0);
        rect((x, y, right - x, bottom - y))
    }

    fn outputs_json(&self, with_nodes: bool) -> Vec<Value> {
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        outputs.iter().enumerate().map(|(i, o)| self.output_json(i, o, with_nodes)).collect()
    }

    fn output_json(&self, index: usize, output: &Output, with_nodes: bool) -> Value {
        let name = output.name();
        let geo = self.space.output_geometry(output).unwrap_or_default();
        let mode = output.current_mode().map(|m| json!({ "width": m.size.w, "height": m.size.h, "refresh": m.refresh })).unwrap_or(Value::Null);
        let props = output.physical_properties();
        let mut value = json!({
            "id": 10 + index as i64,
            "type": "output",
            "name": name,
            "make": props.make,
            "model": props.model,
            "serial": props.serial_number,
            "active": true,
            "dpms": true,
            "power": true,
            "primary": false,
            "scale": output.current_scale().fractional_scale(),
            "subpixel_hinting": "unknown",
            "transform": "normal",
            "current_workspace": self.visible.get(&name).map(|n| n.to_string()),
            "modes": [mode.clone()],
            "current_mode": mode,
            "rect": rect((geo.loc.x, geo.loc.y, geo.size.w, geo.size.h)),
            "focused": self.focused_output_name() == name,
            "visible": true,
        });
        if with_nodes {
            value["nodes"] = Value::Array(self.workspaces.iter().filter(|w| w.output == name).map(|w| self.workspace_json(w.num)).collect());
            value["floating_nodes"] = json!([]);
        }
        value
    }

    pub fn workspace_json(&self, num: i32) -> Value {
        let windows: Vec<Value> = self.workspaces.iter().find(|w| w.num == num).map(|w| w.windows.iter().map(|win| self.window_json(win)).collect()).unwrap_or_default();
        let output = self.ws_output(num).unwrap_or_else(|| self.focused_output_name());
        let area = self.output_named(&output).and_then(|o| self.usable_area(&o)).unwrap_or_default();
        json!({
            "id": 1000 + num as i64,
            "type": "workspace",
            "name": num.to_string(),
            "num": num,
            "output": output,
            "focused": num == self.current && self.focused_window().is_none_or(|w| self.workspace_of(&w) == Some(num)),
            "visible": self.is_visible(num),
            "urgent": false,
            "layout": "splith",
            "orientation": "horizontal",
            "rect": rect(area),
            "nodes": windows,
            "floating_nodes": [],
            "focus": [],
            "representation": "H[]",
        })
    }

    pub fn window_json(&self, window: &Window) -> Value {
        let info = workspace::info(window).borrow();
        let (title, app_id, pid) = match window.toplevel() {
            Some(toplevel) => {
                let surface = toplevel.wl_surface();
                let (title, app_id) = with_states(surface, |states| {
                    states
                        .data_map
                        .get::<XdgToplevelSurfaceData>()
                        .map(|d| {
                            let d = d.lock().unwrap();
                            (d.title.clone().unwrap_or_default(), d.app_id.clone().unwrap_or_default())
                        })
                        .unwrap_or_default()
                });
                let pid = self.display_handle.get_client(surface.id()).ok().and_then(|c| c.get_credentials(&self.display_handle).ok()).map(|c| c.pid);
                (title, app_id, pid)
            }
            None => (String::new(), String::new(), None),
        };
        let on = self.workspace_of(window);
        json!({
            "id": info.id,
            "type": "con",
            "name": title,
            "app_id": app_id,
            "pid": pid,
            "shell": "xdg_shell",
            "rect": rect(info.rect),
            "window_rect": rect((0, 0, info.rect.2, info.rect.3)),
            "deco_rect": rect((0, 0, 0, 0)),
            "geometry": rect((0, 0, info.rect.2, info.rect.3)),
            "focused": self.focused_window().as_ref() == Some(window),
            "visible": on.is_some_and(|n| self.is_visible(n)),
            "urgent": false,
            "layout": "none",
            "orientation": "none",
            "border": "pixel",
            "current_border_width": self.config.border,
            "fullscreen_mode": 0,
            "sticky": false,
            "marks": [],
            "nodes": [],
            "floating_nodes": [],
            "focus": [],
        })
    }

    /// The whole tree: root, the scratchpad's hidden output, then the
    /// output with its workspaces and windows.
    fn tree_json(&self) -> Value {
        let scratch: Vec<Value> = self.scratchpad.iter().map(|w| {
            let mut con = self.window_json(w);
            con["type"] = json!("floating_con");
            con
        }).collect();
        let hidden = json!({
            "id": 3, "type": "output", "name": "__i3", "rect": rect((0, 0, 0, 0)), "floating_nodes": [],
            "nodes": [{ "id": 4, "type": "workspace", "name": "__i3_scratch", "num": -1, "rect": rect((0, 0, 0, 0)), "nodes": [], "floating_nodes": scratch }],
        });
        let mut nodes = vec![hidden];
        nodes.extend(self.outputs_json(true));
        json!({ "id": 1, "type": "root", "name": "root", "rect": self.layout_rect(), "focused": false, "nodes": nodes, "floating_nodes": [] })
    }

    fn inputs_json(&self) -> Vec<Value> {
        let (names, active) = self.layouts.clone();
        vec![
            json!({
                "identifier": "0:0:orbit-keyboard",
                "name": "Orbit keyboard",
                "type": "keyboard",
                "xkb_layout_names": names,
                "xkb_active_layout_index": active,
                "xkb_active_layout_name": names.get(active),
                "libinput": { "send_events": "enabled" },
            }),
            json!({ "identifier": "0:0:orbit-pointer", "name": "Orbit pointer", "type": "pointer", "libinput": { "send_events": "enabled" } }),
        ]
    }
}

fn rect((x, y, width, height): (i32, i32, i32, i32)) -> Value {
    json!({ "x": x, "y": y, "width": width, "height": height })
}
