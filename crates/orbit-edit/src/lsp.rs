//! Language servers (LSP): completion as one types, errors and warnings
//! underlined (their text on hover, the count in the status line), the
//! documentation of what is under the pointer, go to definition (F12, or
//! Ctrl+click), rename (F2) and formatting.
//!
//! A server is started per language and project, the first time a file
//! of that language opens, if its program is installed: rust-analyzer,
//! pylsp (or pyright), clangd, gopls, typescript-language-server,
//! bash-language-server, and others (`SERVERS`); `lsp:` in edit.yaml
//! names other programs, or none (`rust: []`). The project is the
//! nearest folder above the file with a marker (`.git`, `Cargo.toml`,
//! `package.json`, …).
//!
//! JSON-RPC over the server's standard input and output: a thread reads
//! its messages, another writes, so a slow server never holds the
//! window. The text goes whole on each change (a moment after the typing
//! stops, and at once before a question about it).

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    future::Future,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    pin::Pin,
    process::{Child, Command, Stdio},
    rc::{Rc, Weak},
    time::Duration,
};

use gtk::{gio, glib, prelude::*, subclass::prelude::*};
use serde_json::{Value, json};
use sourceview5::{prelude::*, subclass::prelude::*};

use crate::doc::Doc;

/// GtkSourceView's language → the server's program and the LSP's name of
/// the language.
const SERVERS: &[(&str, &[&[&str]], &str)] = &[
    ("rust", &[&["rust-analyzer"]], "rust"),
    (
        "python3",
        &[
            &["pylsp"],
            &["pyright-langserver", "--stdio"],
            &["basedpyright-langserver", "--stdio"],
        ],
        "python",
    ),
    (
        "python",
        &[
            &["pylsp"],
            &["pyright-langserver", "--stdio"],
            &["basedpyright-langserver", "--stdio"],
        ],
        "python",
    ),
    ("c", &[&["clangd"]], "c"),
    ("cpp", &[&["clangd"]], "cpp"),
    ("objc", &[&["clangd"]], "objective-c"),
    ("go", &[&["gopls"]], "go"),
    ("js", &[&["typescript-language-server", "--stdio"]], "javascript"),
    (
        "typescript",
        &[&["typescript-language-server", "--stdio"]],
        "typescript",
    ),
    (
        "typescript-jsx",
        &[&["typescript-language-server", "--stdio"]],
        "typescriptreact",
    ),
    ("jsx", &[&["typescript-language-server", "--stdio"]], "javascriptreact"),
    ("sh", &[&["bash-language-server", "start"]], "shellscript"),
    ("lua", &[&["lua-language-server"]], "lua"),
    ("json", &[&["vscode-json-language-server", "--stdio"]], "json"),
    ("yaml", &[&["yaml-language-server", "--stdio"]], "yaml"),
    ("html", &[&["vscode-html-language-server", "--stdio"]], "html"),
    ("css", &[&["vscode-css-language-server", "--stdio"]], "css"),
    ("java", &[&["jdtls"]], "java"),
    ("php", &[&["intelephense", "--stdio"]], "php"),
    ("ruby", &[&["solargraph", "stdio"]], "ruby"),
    ("zig", &[&["zls"]], "zig"),
    ("haskell", &[&["haskell-language-server-wrapper", "--lsp"]], "haskell"),
    ("toml", &[&["taplo", "lsp", "stdio"]], "toml"),
];

/// What marks a project's top folder.
const MARKERS: &[&str] = &[
    ".git",
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "setup.py",
    "go.mod",
    "compile_commands.json",
    "CMakeLists.txt",
    "meson.build",
    "Makefile",
];

type Reply = Box<dyn FnOnce(Result<Value, String>)>;

// ------------------------------------------------------------- client

/// One server running.
pub struct Client {
    name: String,
    child: RefCell<Child>,
    out: std::sync::mpsc::Sender<Vec<u8>>,
    next: Cell<i64>,
    pending: RefCell<HashMap<i64, Reply>>,
    /// Sent once the server has answered `initialize`.
    ready: Cell<bool>,
    queue: RefCell<Vec<Value>>,
    /// Positions in bytes (utf-8) rather than UTF-16 code units.
    utf8: Cell<bool>,
    capabilities: RefCell<Value>,
    /// Who to tell of the diagnostics for each document, by URI.
    listeners: RefCell<HashMap<String, Weak<Attached>>>,
    /// Still running (a program that ends at once — rustup's stand-in
    /// for a component not installed — is no server).
    alive: Cell<bool>,
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.borrow_mut().kill();
        let _ = self.child.borrow_mut().wait();
    }
}

impl Client {
    fn start(program: &[String], root: &Path) -> Option<Rc<Self>> {
        let mut child = Command::new(&program[0])
            .args(&program[1..])
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;
        let (out, to_write) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            for bytes in to_write {
                if stdin.write_all(&bytes).and_then(|()| stdin.flush()).is_err() {
                    break;
                }
            }
        });
        let (tx, rx) = async_channel::unbounded::<Value>();
        std::thread::spawn(move || read_messages(stdout, &tx));
        let client = Rc::new(Self {
            name: program[0].clone(),
            child: RefCell::new(child),
            out,
            next: Cell::new(1),
            pending: RefCell::default(),
            ready: Cell::new(false),
            queue: RefCell::default(),
            utf8: Cell::new(false),
            capabilities: RefCell::new(Value::Null),
            listeners: RefCell::default(),
            alive: Cell::new(true),
        });
        let weak = Rc::downgrade(&client);
        glib::spawn_future_local(async move {
            while let Ok(message) = rx.recv().await {
                let Some(c) = weak.upgrade() else { break };
                c.dispatch(message);
            }
            if let Some(c) = weak.upgrade() {
                log::info!("{}: the language server ended", c.name);
                c.alive.set(false);
                // Its documents show no count any more.
                let listeners: Vec<Rc<Attached>> = c.listeners.borrow().values().filter_map(Weak::upgrade).collect();
                for a in listeners {
                    (a.on_diagnostics)();
                }
                // Whoever waits for an answer gets none.
                for (_, reply) in c.pending.borrow_mut().drain() {
                    reply(Err("ended".into()));
                }
            }
        });
        let uri = uri_of(root);
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let params = json!({
            "processId": std::process::id(),
            "clientInfo": { "name": "orbit-edit" },
            "rootUri": uri,
            "rootPath": root,
            "workspaceFolders": [{ "uri": uri, "name": name }],
            "capabilities": {
                "general": { "positionEncodings": ["utf-8", "utf-16"] },
                "workspace": { "configuration": true, "workspaceFolders": true, "applyEdit": false },
                "window": { "workDoneProgress": false },
                "textDocument": {
                    "synchronization": { "didSave": true, "dynamicRegistration": false },
                    "completion": {
                        "completionItem": {
                            "snippetSupport": true,
                            "documentationFormat": ["plaintext", "markdown"],
                            "resolveSupport": { "properties": [] },
                        },
                        "contextSupport": true,
                    },
                    "hover": { "contentFormat": ["plaintext", "markdown"] },
                    "definition": { "linkSupport": true },
                    "formatting": {},
                    "rename": { "prepareSupport": false },
                    "publishDiagnostics": { "relatedInformation": false },
                },
            },
        });
        let weak = Rc::downgrade(&client);
        client.request_now("initialize", params, move |result| {
            let Some(c) = weak.upgrade() else { return };
            let Ok(result) = result else {
                log::warn!("{}: initialize failed", c.name);
                return;
            };
            let caps = result.get("capabilities").cloned().unwrap_or(Value::Null);
            c.utf8
                .set(caps.get("positionEncoding").and_then(Value::as_str) == Some("utf-8"));
            *c.capabilities.borrow_mut() = caps;
            c.write(&json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
            c.ready.set(true);
            for message in std::mem::take(&mut *c.queue.borrow_mut()) {
                c.write(&message);
            }
        });
        Some(client)
    }

    fn write(&self, message: &Value) {
        let body = message.to_string();
        let _ = self
            .out
            .send(format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes());
    }

    fn send(&self, message: Value) {
        if self.ready.get() {
            self.write(&message);
        } else {
            self.queue.borrow_mut().push(message);
        }
    }

    fn notify(&self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    fn request(&self, method: &str, params: Value, reply: impl FnOnce(Result<Value, String>) + 'static) {
        let id = self.next.get();
        self.next.set(id + 1);
        self.pending.borrow_mut().insert(id, Box::new(reply));
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
    }

    /// A request before the server is ready (`initialize`).
    fn request_now(&self, method: &str, params: Value, reply: impl FnOnce(Result<Value, String>) + 'static) {
        let id = self.next.get();
        self.next.set(id + 1);
        self.pending.borrow_mut().insert(id, Box::new(reply));
        self.write(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
    }

    /// The same as a future.
    fn ask(&self, method: &str, params: Value) -> impl Future<Output = Result<Value, String>> + use<> {
        let (tx, rx) = async_channel::bounded(1);
        self.request(method, params, move |r| {
            let _ = tx.try_send(r);
        });
        async move { rx.recv().await.unwrap_or_else(|_| Err("no answer".into())) }
    }

    fn dispatch(&self, message: Value) {
        let id = message.get("id").cloned();
        match (message.get("method").and_then(Value::as_str), id) {
            // An answer.
            (None, Some(id)) => {
                let Some(id) = id.as_i64() else { return };
                if let Some(reply) = self.pending.borrow_mut().remove(&id) {
                    match message.get("error") {
                        Some(err) => reply(Err(err
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("error")
                            .to_string())),
                        None => reply(Ok(message.get("result").cloned().unwrap_or(Value::Null))),
                    }
                }
            }
            // The server asks: what is needed is answered, the rest with
            // nothing.
            (Some(method), Some(id)) => {
                let result = match method {
                    "workspace/configuration" => {
                        let n = message
                            .pointer("/params/items")
                            .and_then(Value::as_array)
                            .map_or(0, Vec::len);
                        Value::Array(vec![Value::Null; n])
                    }
                    "workspace/workspaceFolders" => Value::Null,
                    _ => Value::Null,
                };
                self.write(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
            }
            (Some("textDocument/publishDiagnostics"), None) => {
                let Some(params) = message.get("params") else { return };
                let uri = params.get("uri").and_then(Value::as_str).unwrap_or_default();
                let listener = self.listeners.borrow().get(uri).and_then(Weak::upgrade);
                if let Some(a) = listener {
                    let list = params
                        .get("diagnostics")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    a.set_diagnostics(&list);
                }
            }
            _ => {}
        }
    }

    fn trigger_characters(&self) -> Vec<char> {
        self.capabilities
            .borrow()
            .pointer("/completionProvider/triggerCharacters")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .filter_map(|s| s.chars().next())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn can(&self, what: &str) -> bool {
        self.capabilities
            .borrow()
            .get(what)
            .is_some_and(|v| !v.is_null() && v != &Value::Bool(false))
    }
}

/// Reads the server's messages, `Content-Length` framed.
fn read_messages(stdout: impl Read, tx: &async_channel::Sender<Value>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut length = None;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            let line = line.trim();
            if line.is_empty() {
                break;
            }
            if let Some((k, v)) = line.split_once(':')
                && k.eq_ignore_ascii_case("content-length")
            {
                length = v.trim().parse::<usize>().ok();
            }
        }
        let Some(length) = length else { continue };
        let mut body = vec![0; length];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        if let Ok(value) = serde_json::from_slice::<Value>(&body)
            && tx.send_blocking(value).is_err()
        {
            return;
        }
    }
}

// ----------------------------------------------------------- documents

/// A document a server knows of.
pub struct Attached {
    client: Rc<Client>,
    doc: Weak<Doc>,
    uri: String,
    version: Cell<i32>,
    /// A change not yet sent.
    soon: RefCell<Option<glib::SourceId>>,
    handlers: RefCell<Vec<glib::SignalHandlerId>>,
    completion: Provider,
    hover: Hover,
    diagnostics: RefCell<Vec<Diagnostic>>,
    on_diagnostics: Rc<dyn Fn()>,
}

#[derive(Clone)]
pub struct Diagnostic {
    pub start: i32,
    pub end: i32,
    /// 1 error, 2 warning, 3 information, 4 hint.
    pub severity: u8,
    pub message: String,
}

impl Attached {
    /// Sends the text now if a change waits.
    fn flush(&self) {
        if let Some(id) = self.soon.borrow_mut().take() {
            id.remove();
            self.send_change();
        }
    }

    fn send_change(&self) {
        let Some(doc) = self.doc.upgrade() else { return };
        self.version.set(self.version.get() + 1);
        self.client.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": self.uri, "version": self.version.get() },
                "contentChanges": [{ "text": doc.text() }],
            }),
        );
    }

    fn position(&self, iter: &gtk::TextIter) -> Value {
        position_of(iter, self.client.utf8.get())
    }

    fn set_diagnostics(&self, list: &[Value]) {
        let Some(doc) = self.doc.upgrade() else { return };
        let buffer: &gtk::TextBuffer = doc.buffer.upcast_ref();
        ensure_tags(buffer);
        let (start, end) = buffer.bounds();
        for name in ["lsp-error", "lsp-warning", "lsp-info"] {
            buffer.remove_tag_by_name(name, &start, &end);
        }
        let utf8 = self.client.utf8.get();
        let mut kept = Vec::new();
        for d in list {
            let (Some(s), Some(e)) = (d.pointer("/range/start"), d.pointer("/range/end")) else {
                continue;
            };
            let mut from = iter_of(buffer, s, utf8);
            let mut to = iter_of(buffer, e, utf8);
            if from.offset() == to.offset() {
                // Nothing to underline: the word there, or a character.
                if !to.ends_word() && to.inside_word() {
                    to.forward_word_end();
                } else if !to.ends_line() {
                    to.forward_char();
                } else if from.line_offset() > 0 {
                    from.backward_char();
                }
            }
            let severity = d.get("severity").and_then(Value::as_u64).unwrap_or(1) as u8;
            let tag = match severity {
                1 => "lsp-error",
                2 => "lsp-warning",
                _ => "lsp-info",
            };
            ensure_tags(buffer);
            buffer.apply_tag_by_name(tag, &from, &to);
            let source = d.get("source").and_then(Value::as_str).unwrap_or("");
            let message = d.get("message").and_then(Value::as_str).unwrap_or("").to_string();
            kept.push(Diagnostic {
                start: from.offset(),
                end: to.offset(),
                severity,
                message: if source.is_empty() {
                    message
                } else {
                    format!("{source}: {message}")
                },
            });
        }
        *self.diagnostics.borrow_mut() = kept;
        (self.on_diagnostics)();
    }

    fn diagnostics_at(&self, offset: i32) -> Vec<Diagnostic> {
        self.diagnostics
            .borrow()
            .iter()
            .filter(|d| d.start <= offset && offset <= d.end)
            .cloned()
            .collect()
    }
}

fn ensure_tags(buffer: &gtk::TextBuffer) {
    let table = buffer.tag_table();
    if table.lookup("lsp-error").is_some() {
        return;
    }
    let tag = |name: &str, color: &str, kind: gtk::pango::Underline| {
        let rgba = gtk::gdk::RGBA::parse(color).expect("a colour");
        let t = gtk::TextTag::builder()
            .name(name)
            .underline(kind)
            .underline_rgba(&rgba)
            .background_rgba(&gtk::gdk::RGBA::new(rgba.red(), rgba.green(), rgba.blue(), 0.18))
            .build();
        table.add(&t);
    };
    tag("lsp-error", "#bf616a", gtk::pango::Underline::Error);
    tag("lsp-warning", "#ebcb8b", gtk::pango::Underline::Error);
    tag("lsp-info", "#81a1c1", gtk::pango::Underline::Single);
}

// ------------------------------------------------------------- manager

type Open = Rc<dyn Fn(&Path, i32, i32)>;
type Find = Rc<dyn Fn(&Path) -> Option<Rc<Doc>>>;
type DiagnosticsHook = RefCell<Option<Rc<dyn Fn(u64)>>>;

pub struct Lsp {
    overrides: HashMap<String, Vec<String>>,
    clients: RefCell<HashMap<(String, PathBuf), Rc<Client>>>,
    /// Programs looked for and missing, not looked for again.
    missing: RefCell<std::collections::HashSet<String>>,
    attached: RefCell<HashMap<u64, Rc<Attached>>>,
    open: Open,
    find: Find,
    pub on_diagnostics: DiagnosticsHook,
}

impl Lsp {
    pub fn new(
        overrides: HashMap<String, Vec<String>>,
        open: impl Fn(&Path, i32, i32) + 'static,
        find: impl Fn(&Path) -> Option<Rc<Doc>> + 'static,
    ) -> Rc<Self> {
        Rc::new(Self {
            overrides,
            clients: RefCell::default(),
            missing: RefCell::default(),
            attached: RefCell::default(),
            open: Rc::new(open),
            find: Rc::new(find),
            on_diagnostics: RefCell::default(),
        })
    }

    /// The program for a language: edit.yaml's, or the first of ours
    /// installed.
    fn program(&self, language: &str) -> Option<Vec<String>> {
        if let Some(p) = self.overrides.get(language) {
            return (!p.is_empty()).then(|| p.clone());
        }
        let (_, programs, _) = SERVERS.iter().find(|(l, _, _)| *l == language)?;
        programs
            .iter()
            .find(|p| {
                let name = p[0];
                if self.missing.borrow().contains(name) {
                    return false;
                }
                let found = glib::find_program_in_path(name).is_some();
                if !found {
                    self.missing.borrow_mut().insert(name.to_string());
                }
                found
            })
            .map(|p| p.iter().map(|s| s.to_string()).collect())
    }

    /// Tells the server of `doc` (after it is opened, saved somewhere
    /// else, or its language changed); a document of no server's
    /// language, or with no file, is let go.
    pub fn attach(self: &Rc<Self>, doc: &Rc<Doc>) {
        self.detach(doc);
        let (Some(path), Some(language)) = (doc.path(), doc.language_id()) else {
            return;
        };
        let Some(program) = self.program(&language) else { return };
        let lsp_language = SERVERS
            .iter()
            .find(|(l, _, _)| *l == language)
            .map_or(language.as_str(), |(_, _, id)| *id)
            .to_string();
        let root = project_root(&path);
        let key = (program.join(" "), root.clone());
        let existing = self.clients.borrow().get(&key).cloned().filter(|c| c.alive.get());
        let client = match existing {
            Some(c) => c,
            None => {
                let Some(c) = Client::start(&program, &root) else {
                    self.missing.borrow_mut().insert(program[0].clone());
                    return;
                };
                self.clients.borrow_mut().insert(key, c.clone());
                c
            }
        };
        let uri = uri_of(&path);
        client.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": { "uri": uri, "languageId": lsp_language, "version": 1, "text": doc.text() },
            }),
        );
        let completion: Provider = glib::Object::new();
        let hover: Hover = glib::Object::new();
        let id = doc.id;
        let weak_self = Rc::downgrade(self);
        let attached = Rc::new(Attached {
            client: client.clone(),
            doc: Rc::downgrade(doc),
            uri: uri.clone(),
            version: Cell::new(1),
            soon: RefCell::default(),
            handlers: RefCell::default(),
            completion: completion.clone(),
            hover: hover.clone(),
            diagnostics: RefCell::default(),
            on_diagnostics: Rc::new(move || {
                if let Some(l) = weak_self.upgrade()
                    && let Some(f) = l.on_diagnostics.borrow().clone()
                {
                    f(id);
                }
            }),
        });
        *completion.imp().attached.borrow_mut() = Rc::downgrade(&attached);
        *hover.imp().attached.borrow_mut() = Rc::downgrade(&attached);
        for view in doc.views() {
            view.completion().add_provider(&completion);
            view.hover().add_provider(&hover);
        }
        client.listeners.borrow_mut().insert(uri, Rc::downgrade(&attached));
        let weak = Rc::downgrade(&attached);
        let changed = doc.buffer.connect_changed(move |_| {
            let Some(a) = weak.upgrade() else { return };
            if let Some(id) = a.soon.borrow_mut().take() {
                id.remove();
            }
            let weak = Rc::downgrade(&a);
            let id = glib::timeout_add_local_once(Duration::from_millis(200), move || {
                if let Some(a) = weak.upgrade() {
                    a.soon.borrow_mut().take();
                    a.send_change();
                }
            });
            *a.soon.borrow_mut() = Some(id);
        });
        attached.handlers.borrow_mut().push(changed);
        self.attached.borrow_mut().insert(doc.id, attached);
    }

    /// The server forgets `doc` (its tab closed).
    pub fn detach(&self, doc: &Doc) {
        let Some(a) = self.attached.borrow_mut().remove(&doc.id) else {
            return;
        };
        a.flush();
        for h in a.handlers.borrow_mut().drain(..) {
            doc.buffer.disconnect(h);
        }
        for view in doc.views() {
            view.completion().remove_provider(&a.completion);
            view.hover().remove_provider(&a.hover);
        }
        let buffer: &gtk::TextBuffer = doc.buffer.upcast_ref();
        ensure_tags(buffer);
        let (start, end) = buffer.bounds();
        for name in ["lsp-error", "lsp-warning", "lsp-info"] {
            buffer.remove_tag_by_name(name, &start, &end);
        }
        a.client.listeners.borrow_mut().remove(&a.uri);
        a.client
            .notify("textDocument/didClose", json!({ "textDocument": { "uri": a.uri } }));
    }

    /// The view was split: the new view gets completion and hover too.
    pub fn views_changed(&self, doc: &Doc) {
        let Some(a) = self.attached.borrow().get(&doc.id).cloned() else {
            return;
        };
        for view in doc.views() {
            view.completion().remove_provider(&a.completion);
            view.completion().add_provider(&a.completion);
            view.hover().remove_provider(&a.hover);
            view.hover().add_provider(&a.hover);
        }
    }

    pub fn saved(&self, doc: &Doc) {
        if let Some(a) = self.attached.borrow().get(&doc.id) {
            a.flush();
            a.client
                .notify("textDocument/didSave", json!({ "textDocument": { "uri": a.uri } }));
        }
    }

    pub fn server_name(&self, doc: &Doc) -> Option<String> {
        self.attached
            .borrow()
            .get(&doc.id)
            .filter(|a| a.client.alive.get())
            .map(|a| a.client.name.clone())
    }

    /// Errors and warnings of `doc`.
    pub fn counts(&self, doc: &Doc) -> Option<(usize, usize)> {
        let a = self
            .attached
            .borrow()
            .get(&doc.id)
            .cloned()
            .filter(|a| a.client.alive.get())?;
        let d = a.diagnostics.borrow();
        Some((
            d.iter().filter(|d| d.severity == 1).count(),
            d.iter().filter(|d| d.severity == 2).count(),
        ))
    }

    /// The next error or warning after the cursor (F8).
    pub fn next_diagnostic(&self, doc: &Doc) {
        let Some(a) = self.attached.borrow().get(&doc.id).cloned() else {
            return;
        };
        let at = doc.cursor_offset();
        let d = a.diagnostics.borrow();
        let next = d
            .iter()
            .filter(|d| d.start > at)
            .min_by_key(|d| d.start)
            .or_else(|| d.iter().min_by_key(|d| d.start));
        if let Some(d) = next {
            doc.place_cursor(d.start);
        }
    }

    /// F12: where what is under the cursor is defined.
    pub fn definition(self: &Rc<Self>, doc: &Doc) {
        let Some(a) = self.attached.borrow().get(&doc.id).cloned() else {
            return;
        };
        a.flush();
        let at = doc.buffer.iter_at_mark(&doc.buffer.get_insert());
        let params = json!({ "textDocument": { "uri": a.uri }, "position": a.position(&at) });
        let open = self.open.clone();
        let find = self.find.clone();
        let utf8 = a.client.utf8.get();
        a.client.request("textDocument/definition", params, move |result| {
            let Ok(result) = result else { return };
            let first = match &result {
                Value::Array(list) => list.first().cloned(),
                Value::Null => None,
                one => Some(one.clone()),
            };
            let Some(location) = first else { return };
            let uri = location
                .get("targetUri")
                .or_else(|| location.get("uri"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let start = location
                .pointer("/targetSelectionRange/start")
                .or_else(|| location.pointer("/range/start"))
                .cloned()
                .unwrap_or(Value::Null);
            let Some(path) = path_of(uri) else { return };
            let line = start.get("line").and_then(Value::as_i64).unwrap_or(0) as i32;
            let character = start.get("character").and_then(Value::as_i64).unwrap_or(0) as i32;
            // The column in characters, if the file is open (else close
            // enough: most lines are ASCII).
            let column = find(&path).map_or(character, |d| {
                iter_of(d.buffer.upcast_ref(), &start, utf8).line_offset()
            });
            open(&path, line, column);
        });
    }

    /// Formats the whole text with the server; `done` after (also when
    /// there is no server or it cannot).
    pub fn format(&self, doc: &Rc<Doc>, done: impl FnOnce() + 'static) {
        let Some(a) = self
            .attached
            .borrow()
            .get(&doc.id)
            .cloned()
            .filter(|a| a.client.can("documentFormattingProvider"))
        else {
            done();
            return;
        };
        a.flush();
        let params = json!({
            "textDocument": { "uri": a.uri },
            "options": {
                "tabSize": doc.view.tab_width(),
                "insertSpaces": doc.view.is_insert_spaces_instead_of_tabs(),
            },
        });
        let weak = Rc::downgrade(doc);
        let utf8 = a.client.utf8.get();
        a.client.request("textDocument/formatting", params, move |result| {
            if let (Some(doc), Ok(Value::Array(edits))) = (weak.upgrade(), result) {
                apply_edits(doc.buffer.upcast_ref(), &edits, utf8);
            }
            done();
        });
    }

    pub fn can_rename(&self, doc: &Doc) -> bool {
        self.attached
            .borrow()
            .get(&doc.id)
            .is_some_and(|a| a.client.can("renameProvider"))
    }

    /// F2: renames what is under the cursor, everywhere the server says
    /// — in the open documents as edits (undoable), in the other files
    /// on disk.
    pub fn rename(&self, doc: &Doc, name: &str, done: impl FnOnce(Result<usize, String>) + 'static) {
        let Some(a) = self.attached.borrow().get(&doc.id).cloned() else {
            return;
        };
        a.flush();
        let at = doc.buffer.iter_at_mark(&doc.buffer.get_insert());
        let params = json!({
            "textDocument": { "uri": a.uri },
            "position": a.position(&at),
            "newName": name,
        });
        let find = self.find.clone();
        let utf8 = a.client.utf8.get();
        a.client.request("textDocument/rename", params, move |result| {
            let edit = match result {
                Ok(e) if !e.is_null() => e,
                Ok(_) => return done(Ok(0)),
                Err(e) => return done(Err(e)),
            };
            let mut by_file: Vec<(String, Vec<Value>)> = Vec::new();
            if let Some(changes) = edit.get("changes").and_then(Value::as_object) {
                for (uri, edits) in changes {
                    by_file.push((uri.clone(), edits.as_array().cloned().unwrap_or_default()));
                }
            }
            if let Some(list) = edit.get("documentChanges").and_then(Value::as_array) {
                for change in list {
                    if let (Some(uri), Some(edits)) = (
                        change.pointer("/textDocument/uri").and_then(Value::as_str),
                        change.get("edits").and_then(Value::as_array),
                    ) {
                        by_file.push((uri.to_string(), edits.clone()));
                    }
                }
            }
            let mut files = 0;
            for (uri, edits) in by_file {
                let Some(path) = path_of(&uri) else { continue };
                if let Some(d) = find(&path) {
                    apply_edits(d.buffer.upcast_ref(), &edits, utf8);
                } else if let Ok(text) = std::fs::read_to_string(&path) {
                    let buffer = gtk::TextBuffer::new(None);
                    buffer.set_text(&text);
                    apply_edits(&buffer, &edits, utf8);
                    let (s, e) = buffer.bounds();
                    if let Err(err) = std::fs::write(&path, buffer.text(&s, &e, true).as_bytes()) {
                        log::warn!("rename in {}: {err}", path.display());
                        continue;
                    }
                }
                files += 1;
            }
            done(Ok(files));
        });
    }
}

/// Applies a server's text edits, last first, as one undoable edit.
fn apply_edits(buffer: &gtk::TextBuffer, edits: &[Value], utf8: bool) {
    let mut marked: Vec<(gtk::TextMark, gtk::TextMark, String)> = edits
        .iter()
        .filter_map(|e| {
            let start = iter_of(buffer, e.pointer("/range/start")?, utf8);
            let end = iter_of(buffer, e.pointer("/range/end")?, utf8);
            let text = e.get("newText").and_then(Value::as_str)?.to_string();
            Some((
                buffer.create_mark(None, &start, true),
                buffer.create_mark(None, &end, false),
                text,
            ))
        })
        .collect();
    // Marks keep their place as the others change the text.
    marked.sort_by_key(|(s, _, _)| std::cmp::Reverse(buffer.iter_at_mark(s).offset()));
    buffer.begin_user_action();
    for (s, e, text) in &marked {
        let mut start = buffer.iter_at_mark(s);
        let mut end = buffer.iter_at_mark(e);
        buffer.delete(&mut start, &mut end);
        buffer.insert(&mut start, text);
    }
    buffer.end_user_action();
    for (s, e, _) in marked {
        buffer.delete_mark(&s);
        buffer.delete_mark(&e);
    }
}

// ------------------------------------------------------ positions, URIs

fn line_text(iter: &gtk::TextIter) -> String {
    let mut start = *iter;
    start.set_line_offset(0);
    let mut end = start;
    if !end.ends_line() {
        end.forward_to_line_end();
    }
    start.text(&end).to_string()
}

/// An LSP position from a place in the text.
fn position_of(iter: &gtk::TextIter, utf8: bool) -> Value {
    let text = line_text(iter);
    let before: String = text.chars().take(iter.line_offset() as usize).collect();
    let character = if utf8 {
        before.len()
    } else {
        before.encode_utf16().count()
    };
    json!({ "line": iter.line(), "character": character })
}

/// The place in the text of an LSP position.
fn iter_of(buffer: &gtk::TextBuffer, position: &Value, utf8: bool) -> gtk::TextIter {
    let line = position.get("line").and_then(Value::as_i64).unwrap_or(0) as i32;
    let character = position.get("character").and_then(Value::as_i64).unwrap_or(0) as usize;
    let Some(mut iter) = buffer.iter_at_line(line) else {
        return buffer.end_iter();
    };
    let text = line_text(&iter);
    let mut units = 0;
    let mut chars = 0;
    for c in text.chars() {
        if units >= character {
            break;
        }
        units += if utf8 { c.len_utf8() } else { c.len_utf16() };
        chars += 1;
    }
    iter.set_line_offset(chars);
    iter
}

fn uri_of(path: &Path) -> String {
    gio::File::for_path(path).uri().to_string()
}

fn path_of(uri: &str) -> Option<PathBuf> {
    gio::File::for_uri(uri).path()
}

fn project_root(file: &Path) -> PathBuf {
    let mut dir = file.parent();
    while let Some(d) = dir {
        if MARKERS.iter().any(|m| d.join(m).exists()) {
            return d.to_path_buf();
        }
        dir = d.parent();
    }
    file.parent().map_or_else(glib::home_dir, Path::to_path_buf)
}

/// Snippet syntax made plain text, for the list (`${1:x}` → `x`).
fn plain(snippet: &str) -> String {
    let re = regex::Regex::new(r"\$\{\d+:([^}]*)\}|\$\{\d+\}|\$\d+").expect("a regex");
    re.replace_all(snippet, "$1").to_string()
}

fn hover_text(contents: &Value) -> String {
    match contents {
        Value::String(s) => s.clone(),
        Value::Array(list) => list.iter().map(hover_text).collect::<Vec<_>>().join("\n\n"),
        Value::Object(o) => o.get("value").and_then(Value::as_str).unwrap_or("").to_string(),
        _ => String::new(),
    }
}

/// Markdown made readable as plain text: fences and backticks gone.
fn tidy_markdown(text: &str) -> String {
    text.lines()
        .filter(|l| !l.trim_start().starts_with("```"))
        .map(|l| l.replace('`', ""))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

// --------------------------------------------------- completion items

mod proposal_imp {
    use super::*;

    #[derive(Default)]
    pub struct Proposal {
        pub item: RefCell<Value>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Proposal {
        const NAME: &'static str = "OrbitEditLspProposal";
        type Type = super::Proposal;
        type Interfaces = (sourceview5::CompletionProposal,);
    }
    impl ObjectImpl for Proposal {}
    impl CompletionProposalImpl for Proposal {}
}

glib::wrapper! {
    pub struct Proposal(ObjectSubclass<proposal_imp::Proposal>)
        @implements sourceview5::CompletionProposal;
}

impl Proposal {
    fn label(&self) -> String {
        let item = self.imp().item.borrow();
        item.get("label")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    }
    fn filter_text(&self) -> String {
        let item = self.imp().item.borrow();
        item.get("filterText")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| self.label())
    }
}

/// LSP's kinds of completion, shortly.
fn kind_name(kind: u64) -> &'static str {
    match kind {
        2 => "fn",
        3 => "fn",
        4 => "new",
        5 => "field",
        6 => "var",
        7 => "class",
        8 => "trait",
        9 => "mod",
        10 => "prop",
        13 => "enum",
        14 => "kw",
        15 => "snip",
        20 => "variant",
        21 => "const",
        22 => "struct",
        25 => "type",
        _ => "",
    }
}

mod provider_imp {
    use super::*;

    #[derive(Default)]
    pub struct Provider {
        pub attached: RefCell<Weak<Attached>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Provider {
        const NAME: &'static str = "OrbitEditLspProvider";
        type Type = super::Provider;
        type Interfaces = (sourceview5::CompletionProvider,);
    }
    impl ObjectImpl for Provider {}

    impl CompletionProviderImpl for Provider {
        fn title(&self) -> Option<glib::GString> {
            Some("LSP".into())
        }

        fn priority(&self, _context: &sourceview5::CompletionContext) -> i32 {
            // Above the words of the open files.
            100
        }

        fn is_trigger(&self, _iter: &gtk::TextIter, c: char) -> bool {
            self.attached
                .borrow()
                .upgrade()
                .is_some_and(|a| a.client.trigger_characters().contains(&c))
        }

        fn populate_future(
            &self,
            context: &sourceview5::CompletionContext,
        ) -> Pin<Box<dyn Future<Output = Result<gio::ListModel, glib::Error>>>> {
            let attached = self.attached.borrow().upgrade();
            let at = context.bounds().map(|(_, end)| end);
            let word = context.word().to_string();
            Box::pin(async move {
                let nothing = || glib::Error::new(gio::IOErrorEnum::NotSupported, "no server");
                let (Some(a), Some(at)) = (attached, at) else {
                    return Err(nothing());
                };
                a.flush();
                let params = json!({
                    "textDocument": { "uri": a.uri },
                    "position": a.position(&at),
                    "context": { "triggerKind": 1 },
                });
                let result = a
                    .client
                    .ask("textDocument/completion", params)
                    .await
                    .map_err(|_| nothing())?;
                let items = match result {
                    Value::Array(items) => items,
                    Value::Object(o) => o.get("items").and_then(Value::as_array).cloned().unwrap_or_default(),
                    _ => Vec::new(),
                };
                let store = gio::ListStore::new::<Proposal>();
                let mut items = items;
                items.sort_by(|x, y| {
                    let key = |v: &Value| {
                        v.get("sortText")
                            .or_else(|| v.get("label"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string()
                    };
                    key(x).cmp(&key(y))
                });
                for item in items.into_iter().take(500) {
                    let p: Proposal = glib::Object::new();
                    *p.imp().item.borrow_mut() = item;
                    store.append(&p);
                }
                Ok(filtered(store.upcast(), &word))
            })
        }

        fn refilter(&self, context: &sourceview5::CompletionContext, model: &gio::ListModel) {
            if let Some(f) = model.downcast_ref::<gtk::FilterListModel>() {
                f.set_filter(Some(&word_filter(&context.word())));
            }
        }

        fn display(
            &self,
            _context: &sourceview5::CompletionContext,
            proposal: &sourceview5::CompletionProposal,
            cell: &sourceview5::CompletionCell,
        ) {
            let Some(p) = proposal.downcast_ref::<Proposal>() else {
                return;
            };
            let item = p.imp().item.borrow();
            match cell.column() {
                sourceview5::CompletionColumn::TypedText => cell.set_text(Some(&p.label())),
                sourceview5::CompletionColumn::Before => {
                    cell.set_text(Some(kind_name(item.get("kind").and_then(Value::as_u64).unwrap_or(0))))
                }
                sourceview5::CompletionColumn::After => cell.set_text(
                    item.get("detail")
                        .and_then(Value::as_str)
                        .map(|d| d.lines().next().unwrap_or("")),
                ),
                sourceview5::CompletionColumn::Comment => {
                    let doc = item.get("documentation").map(hover_text).unwrap_or_default();
                    cell.set_text(Some(tidy_markdown(&doc).lines().next().unwrap_or("")))
                }
                sourceview5::CompletionColumn::Details => {
                    let doc = item.get("documentation").map(hover_text).unwrap_or_default();
                    cell.set_text(Some(&tidy_markdown(&doc)))
                }
                _ => cell.set_text(None),
            }
        }

        fn activate(&self, context: &sourceview5::CompletionContext, proposal: &sourceview5::CompletionProposal) {
            let (Some(p), Some(buffer), Some(view)) =
                (proposal.downcast_ref::<Proposal>(), context.buffer(), context.view())
            else {
                return;
            };
            let utf8 = self.attached.borrow().upgrade().is_some_and(|a| a.client.utf8.get());
            let item = p.imp().item.borrow().clone();
            let text = item
                .pointer("/textEdit/newText")
                .or_else(|| item.get("insertText"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| p.label());
            let snippet = item.get("insertTextFormat").and_then(Value::as_u64) == Some(2);
            buffer.begin_user_action();
            if let Some((mut start, mut end)) = context.bounds() {
                // The edit's own start, when the server gives one before
                // the word (`self.` completed as a whole, a path).
                if let Some(s) = item
                    .pointer("/textEdit/range/start")
                    .or_else(|| item.pointer("/textEdit/replace/start"))
                {
                    let from = iter_of(buffer.upcast_ref(), s, utf8);
                    if from.offset() < start.offset() && from.line() == end.line() {
                        start = from;
                    }
                }
                buffer.delete(&mut start, &mut end);
                if snippet && let Ok(s) = sourceview5::Snippet::new_parsed(&text) {
                    view.push_snippet(&s, Some(&mut start));
                } else {
                    let text = if snippet { plain(&text) } else { text };
                    buffer.insert(&mut start, &text);
                }
            }
            buffer.end_user_action();
            if let Some(Value::Array(extra)) = item.get("additionalTextEdits") {
                apply_edits(buffer.upcast_ref(), extra, utf8);
            }
        }
    }
}

glib::wrapper! {
    pub struct Provider(ObjectSubclass<provider_imp::Provider>)
        @implements sourceview5::CompletionProvider;
}

fn word_filter(word: &str) -> gtk::CustomFilter {
    let word = word.to_lowercase();
    gtk::CustomFilter::new(move |o| {
        word.is_empty()
            || o.downcast_ref::<Proposal>()
                .is_some_and(|p| crate::sidebar::score(&word, &p.filter_text().to_lowercase()).is_some())
    })
}

fn filtered(model: gio::ListModel, word: &str) -> gio::ListModel {
    gtk::FilterListModel::new(Some(model), Some(word_filter(word))).upcast()
}

// ----------------------------------------------------------------- hover

mod hover_imp {
    use super::*;

    #[derive(Default)]
    pub struct Hover {
        pub attached: RefCell<Weak<Attached>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Hover {
        const NAME: &'static str = "OrbitEditLspHover";
        type Type = super::Hover;
        type Interfaces = (sourceview5::HoverProvider,);
    }
    impl ObjectImpl for Hover {}

    impl HoverProviderImpl for Hover {
        fn populate_future(
            &self,
            context: &sourceview5::HoverContext,
            display: &sourceview5::HoverDisplay,
        ) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>> + 'static>> {
            let attached = self.attached.borrow().upgrade();
            let at = context.iter();
            let display = display.clone();
            Box::pin(async move {
                let nothing = || glib::Error::new(gio::IOErrorEnum::NotFound, "nothing");
                let (Some(a), Some(at)) = (attached, at) else {
                    return Err(nothing());
                };
                let mut shown = false;
                for d in a.diagnostics_at(at.offset()) {
                    display.append(&hover_label(
                        &d.message,
                        match d.severity {
                            1 => "hover-error",
                            2 => "hover-warning",
                            _ => "hover-info",
                        },
                    ));
                    shown = true;
                }
                if a.client.can("hoverProvider") {
                    a.flush();
                    let params = json!({ "textDocument": { "uri": a.uri }, "position": a.position(&at) });
                    if let Ok(result) = a.client.ask("textDocument/hover", params).await {
                        let text = tidy_markdown(&result.get("contents").map(hover_text).unwrap_or_default());
                        if !text.is_empty() {
                            display.append(&hover_label(&text, "hover-doc"));
                            shown = true;
                        }
                    }
                }
                if shown { Ok(()) } else { Err(nothing()) }
            })
        }
    }
}

glib::wrapper! {
    pub struct Hover(ObjectSubclass<hover_imp::Hover>)
        @implements sourceview5::HoverProvider;
}

fn hover_label(text: &str, class: &str) -> gtk::Label {
    let text: String = text.lines().take(30).collect::<Vec<_>>().join("\n");
    gtk::Label::builder()
        .label(&text)
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(90)
        .selectable(true)
        .css_classes(["hover-text", class])
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_made_plain() {
        assert_eq!(plain("println!(${1:x})$0"), "println!(x)");
        assert_eq!(plain("foo($1)"), "foo()");
    }

    #[test]
    fn hover_texts() {
        assert_eq!(
            tidy_markdown(&hover_text(
                &json!({ "kind": "markdown", "value": "```rust\nfn a()\n```\nDoes `a`." })
            )),
            "fn a()\nDoes a."
        );
    }
}
