//! Orbit-Login: the sessions of Orbit on seat0 (PLANO-CORE §7.4), a
//! small root service in the place of a display manager, as greetd is.
//!
//! At boot it opens the default user's session on its console, without a
//! password — the session starts locked (`orbit-core --locked`) and the
//! greet asks for it — as the getty's autologin did. When it ends after a
//! while (logging out, a crash) it opens again, locked; when it dies at
//! once, and nothing else is open, the service fails and the unit's
//! `OnFailure=` gives the console back to the getty.
//!
//! The greet asks it, on `orbit_login::SOCKET`, for the switch to another
//! user: `prepare` opens that user's session on a free console, behind
//! the one on screen (its services start, its core waits to be shown);
//! `switch` checks their password, shows their session (logind's
//! `Activate`) and tells its core to unlock — the core takes that only
//! from root. Only programs of the sessions it opened may ask.
//!
//!   orbit-login                   the service, as root
//!
//! `/etc/orbit/login.yaml`:
//!
//!   user: juliano                 opened at boot
//!   command: orbit-core --locked  run through the user's login shell
//!   vt: 1                         the default user's console

mod pam;
mod worker;

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::PermissionsExt,
            net::{UnixListener, UnixStream},
        },
    },
    path::PathBuf,
    process::{Command, ExitStatus, Stdio},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use orbit_login::{Reply, Request, SOCKET, User};
use serde::Deserialize;

const CONFIG: &str = "/etc/orbit/login.yaml";
/// A default session that lasted less than this died at start: it is not
/// opened again, or it would loop.
const RESTART_AFTER: Duration = Duration::from_secs(10);
/// How long a session may take to open.
const OPEN_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a core may take to listen once its session is shown.
const UNLOCK_TIMEOUT: Duration = Duration::from_secs(30);
/// Wrong passwords for a user that cost no wait; after them, 2, 4, 8… up
/// to 30 s, as the greet does for its own.
const FREE_TRIES: u32 = 3;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    user: String,
    command: String,
    vt: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self { user: String::new(), command: "orbit-core --locked".into(), vt: 1 }
    }
}

/// A session this service opened.
struct Session {
    uid: u32,
    vt: u32,
    /// logind's id.
    id: Option<String>,
    /// The command's pid: the core, which the login shell execs.
    core: Option<u32>,
}

struct Service {
    config: Config,
    exe: PathBuf,
    sessions: Mutex<HashMap<String, Session>>,
    /// Sessions open one at a time: each takes the next free console.
    opening: Mutex<()>,
    /// Wrong passwords per user, and the last one's time.
    failures: Mutex<HashMap<String, (u32, Instant)>>,
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => match serve() {
            Ok(()) => 0,
            Err(err) => {
                log::error!("{err:#}");
                1
            }
        },
        ["--session", user, vt, command] => worker::session(user, vt.parse().unwrap_or(0), command),
        ["--check", user] => worker::check(user),
        ["-h" | "--help"] => {
            println!("Usage: orbit-login  (the service, as root; configured in {CONFIG})");
            0
        }
        _ => {
            eprintln!("orbit-login: unknown arguments: {args:?}");
            2
        }
    };
    std::process::exit(code);
}

fn serve() -> anyhow::Result<()> {
    // Safety: no arguments, no memory.
    if unsafe { libc::geteuid() } != 0 {
        anyhow::bail!("orbit-login runs as root (the service, orbit-login.service)");
    }
    let text = std::fs::read_to_string(CONFIG).map_err(|e| anyhow::anyhow!("{CONFIG}: {e}"))?;
    let config: Config = serde_yaml_ng::from_str(&text).map_err(|e| anyhow::anyhow!("{CONFIG}: {e}"))?;
    let default = orbit_login::user(&config.user).ok_or_else(|| anyhow::anyhow!("{CONFIG}: user {:?} cannot log in", config.user))?;
    let service = Arc::new(Service {
        exe: std::env::current_exe()?,
        config,
        sessions: Mutex::default(),
        opening: Mutex::default(),
        failures: Mutex::default(),
    });

    let _ = std::fs::create_dir_all("/run/orbit");
    let _ = std::fs::remove_file(SOCKET);
    let listener = UnixListener::bind(SOCKET)?;
    // Anyone may connect; who asks is checked per request.
    std::fs::set_permissions(SOCKET, std::fs::Permissions::from_mode(0o666))?;

    service.open(&default, Some(service.config.vt)).map_err(|e| anyhow::anyhow!(e))?;
    log::info!("listening on {SOCKET}");
    // One request at a time: a check waits for PAM anyway.
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => service.handle(stream),
            Err(err) => log::warn!("accept: {err}"),
        }
    }
    Ok(())
}

impl Service {
    /// Opens `user`'s session on `vt`, or on the next free console.
    fn open(self: &Arc<Self>, user: &User, vt: Option<u32>) -> Result<(), String> {
        let _turn = self.opening.lock().unwrap_or_else(|e| e.into_inner());
        if self.sessions.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&user.name) {
            return Ok(());
        }
        let vt = match vt {
            Some(vt) => vt,
            None => self.free_vt()?,
        };
        let mut child = Command::new(&self.exe)
            .args(["--session", &user.name, &vt.to_string(), &self.config.command])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("{}: {e}", self.exe.display()))?;
        let started = Instant::now();
        let stdout = child.stdout.take().expect("piped");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(stdout).read_line(&mut line);
            let _ = tx.send(line);
        });
        let line = rx.recv_timeout(OPEN_TIMEOUT).unwrap_or_default();
        let mut words = line.split_whitespace();
        if words.next() != Some("ready") {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{}'s session did not open (the journal says why)", user.name));
        }
        let id = words.next().filter(|w| *w != "-").map(str::to_owned);
        let core = words.next().and_then(|w| w.parse().ok());
        log::info!("{}: session {} on tty{vt}", user.name, id.as_deref().unwrap_or("?"));
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).insert(user.name.clone(), Session { uid: user.uid, vt, id, core });
        let service = self.clone();
        let name = user.name.clone();
        std::thread::spawn(move || {
            let status = child.wait();
            service.ended(&name, started.elapsed(), status);
        });
        Ok(())
    }

    /// The lowest console no one holds and no session of this service
    /// was given (a core holds its console only once shown).
    fn free_vt(&self) -> Result<u32, String> {
        #[repr(C)]
        #[derive(Default)]
        struct VtStat {
            active: u16,
            signal: u16,
            state: u16,
        }
        const VT_GETSTATE: libc::c_ulong = 0x5603;
        let tty = std::fs::File::open("/dev/tty0").map_err(|e| format!("/dev/tty0: {e}"))?;
        let mut stat = VtStat::default();
        // Safety: VT_GETSTATE fills a struct vt_stat.
        if unsafe { libc::ioctl(tty.as_raw_fd(), VT_GETSTATE, &mut stat) } != 0 {
            return Err(format!("VT_GETSTATE: {}", std::io::Error::last_os_error()));
        }
        let given: Vec<u32> = self.sessions.lock().unwrap_or_else(|e| e.into_inner()).values().map(|s| s.vt).collect();
        (2..16)
            .find(|&vt| stat.state & (1 << vt) == 0 && !given.contains(&vt) && vt != self.config.vt)
            .ok_or_else(|| "no free console".to_owned())
    }

    fn ended(self: &Arc<Self>, name: &str, ran: Duration, status: std::io::Result<ExitStatus>) {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).remove(name);
        log::info!("{name}: the session ended after {} s ({status:?})", ran.as_secs());
        if name == self.config.user {
            if ran < RESTART_AFTER {
                log::error!("{name}'s session died at start: not opened again");
                if self.sessions.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
                    // The unit's OnFailure= gives tty1 to the getty.
                    std::process::exit(1);
                }
                return;
            }
            if let Some(user) = orbit_login::user(name)
                && let Err(err) = self.open(&user, Some(self.config.vt))
            {
                log::error!("{err}");
            }
        }
        self.show_something();
    }

    /// After a session ended: if the screen shows none of this service's,
    /// the default user's comes back.
    fn show_something(self: &Arc<Self>) {
        let active = Command::new("loginctl")
            .args(["show-seat", "seat0", "--property=ActiveSession", "--value"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default();
        if self.sessions.lock().unwrap_or_else(|e| e.into_inner()).values().any(|s| s.id.as_deref() == Some(&active)) {
            return;
        }
        let Some(user) = orbit_login::user(&self.config.user) else { return };
        if let Err(err) = self.open(&user, Some(self.config.vt)) {
            log::error!("{err}");
            return;
        }
        if let Some(id) = self.session_id(&user.name) {
            activate(&id);
        }
    }

    fn session_id(&self, name: &str) -> Option<String> {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).get(name).and_then(|s| s.id.clone())
    }

    fn handle(self: &Arc<Self>, stream: UnixStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let reply = self.answer(&stream);
        let mut line = serde_json::to_vec(&reply).unwrap_or_default();
        line.push(b'\n');
        let mut stream = stream;
        let _ = stream.write_all(&line);
    }

    fn answer(self: &Arc<Self>, stream: &UnixStream) -> Reply {
        let Some(peer) = peer_uid(stream) else { return Reply::Error("who is asking?".into()) };
        let mut line = String::new();
        if BufReader::new(stream.take(64 * 1024)).read_line(&mut line).is_err() {
            return Reply::Error("bad request".into());
        }
        let Ok(request) = serde_json::from_str::<Request>(&line) else { return Reply::Error("bad request".into()) };
        if !self.sessions.lock().unwrap_or_else(|e| e.into_inner()).values().any(|s| s.uid == peer) {
            log::warn!("uid {peer} asked, with no session of orbit-login");
            return Reply::Error("only the sessions of orbit-login may ask".into());
        }
        let name = match &request {
            Request::Prepare { user } | Request::Switch { user, .. } => user,
        };
        let Some(target) = orbit_login::user(name) else { return Reply::Error(format!("{name}: no such user")) };
        match request {
            Request::Prepare { .. } => {
                if target.uid == peer {
                    return Reply::Ok;
                }
                log::info!("uid {peer} prepares {}", target.name);
                match self.open(&target, None) {
                    Ok(()) => Reply::Ok,
                    Err(err) => Reply::Error(err),
                }
            }
            Request::Switch { password, .. } => {
                if target.uid == peer {
                    return Reply::Error("that is the session asking".into());
                }
                log::info!("uid {peer} switches to {}", target.name);
                if let Some(wait) = self.wait_left(&target.name) {
                    return Reply::Refused(format!("Too many tries: wait {wait} s"));
                }
                // Open first: the check unlocks the keyring of the session.
                if let Err(err) = self.open(&target, None) {
                    return Reply::Error(err);
                }
                if let Err(why) = self.check(&target, password) {
                    log::warn!("{}: password refused: {why}", target.name);
                    let mut failures = self.failures.lock().unwrap_or_else(|e| e.into_inner());
                    let entry = failures.entry(target.name.clone()).or_insert((0, Instant::now()));
                    *entry = (entry.0 + 1, Instant::now());
                    return Reply::Refused(why);
                }
                self.failures.lock().unwrap_or_else(|e| e.into_inner()).remove(&target.name);
                let (id, core) = {
                    let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
                    let Some(session) = sessions.get(&target.name) else { return Reply::Error("the session went away".into()) };
                    (session.id.clone(), session.core)
                };
                let Some(id) = id else { return Reply::Error("the session has no logind id".into()) };
                if !activate(&id) {
                    return Reply::Error("logind did not show the session".into());
                }
                let uid = target.uid;
                std::thread::spawn(move || unlock(uid, core));
                Reply::Ok
            }
        }
    }

    /// The seconds still to wait before `user`'s password is tried again.
    fn wait_left(&self, user: &str) -> Option<u64> {
        let failures = self.failures.lock().unwrap_or_else(|e| e.into_inner());
        let (count, at) = *failures.get(user)?;
        let wait = count.checked_sub(FREE_TRIES).map(|n| 30u64.min(2 << n.min(4)))?;
        let left = wait.saturating_sub(at.elapsed().as_secs());
        (left > 0).then_some(left)
    }

    /// `user`'s password, checked by a program of its own (`--check`).
    fn check(&self, user: &User, password: String) -> Result<(), String> {
        let mut child = Command::new(&self.exe)
            .args(["--check", &user.name])
            .env("XDG_RUNTIME_DIR", format!("/run/user/{}", user.uid))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut password = password.into_bytes();
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(&password);
        }
        password.iter_mut().for_each(|b| *b = 0);
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            let why = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            Err(if why.is_empty() { "Authentication failure".into() } else { why })
        }
    }
}

/// Shows session `id` (logind switches to its console).
fn activate(id: &str) -> bool {
    let ok = Command::new("loginctl").args(["activate", id]).status().is_ok_and(|s| s.success());
    if !ok {
        log::error!("loginctl activate {id} failed");
    }
    ok
}

/// Tells the core of `uid`'s session to unlock, once it listens: shown
/// only now, it is still taking the screen.
fn unlock(uid: u32, core: Option<u32>) {
    let runtime = PathBuf::from(format!("/run/user/{uid}"));
    let start = Instant::now();
    while start.elapsed() < UNLOCK_TIMEOUT {
        let path = match core {
            Some(pid) => Some(runtime.join(format!("orbit-ipc.{uid}.{pid}.sock"))),
            None => std::fs::read_dir(&runtime).ok().and_then(|entries| {
                entries.flatten().map(|e| e.path()).find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&format!("orbit-ipc.{uid}."))))
            }),
        };
        if let Some(path) = path
            && let Ok(mut stream) = UnixStream::connect(&path)
        {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            if stream.write_all(&orbit_ipc::encode(orbit_ipc::RUN_COMMAND, b"unlock")).is_ok() {
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 1024];
                while let Ok(n) = stream.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    buffer.extend_from_slice(&chunk[..n]);
                    if let Ok(Some((_, reply))) = orbit_ipc::decode(&mut buffer) {
                        log::info!("uid {uid}: the core says {}", String::from_utf8_lossy(&reply));
                        return;
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    log::error!("uid {uid}: the core never listened; its greet asks for the password");
}

/// The uid of the process at the other end.
fn peer_uid(stream: &UnixStream) -> Option<u32> {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // Safety: SO_PEERCRED fills a struct ucred of that length.
    let rc = unsafe { libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, &mut cred as *mut _ as *mut libc::c_void, &mut len) };
    (rc == 0).then_some(cred.uid)
}
