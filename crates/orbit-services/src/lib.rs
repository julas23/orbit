//! orbit-services: the services Orbit turns on and off — the rows of
//! Orbit-Settings' "Services" page and of `orbitctl services`
//! (docs/PLANO-DISTRO.md §12).
//!
//! Each service is a package and a way of being on: a system unit enabled
//! and running ([`Kind::System`]), or a program the user's session starts
//! when its YAML says `start: true` ([`Kind::Session`], as `orbit-session`
//! does for the remote server). The ports a service opens are read from the
//! firewall file its package brings, so they are said in one place.
//!
//! What needs root — installing, removing, a unit on or off — is a list of
//! [`Privileged`] steps run by `orbit-services-helper` under `pkexec`
//! (polkit asks for the password, once for the whole list). The helper takes
//! only these typed steps, for the packages and units of [`SERVICES`]. A
//! step that could cut this machine off is applied on trial: undone after a
//! while unless the caller says to keep it.

use std::{
    fs,
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

use serde::Serialize;

pub mod accounts;
pub mod drives;
pub mod updates;

/// The boot menu's times Orbit offers, in seconds (0: hidden behind a
/// key, as Orbit Linux installs it).
pub const BOOT_TIMEOUTS: [(u32, &str); 5] = [(0, "0"), (3, "3"), (5, "5"), (10, "10"), (30, "30")];

/// Packages Orbit-Settings' pages install when asked besides the
/// services': what a page needs and Debian does not always bring.
pub const EXTRA_PACKAGES: [&str; 11] = [
    "geoclue-2.0",
    "orca",
    "wvkbd",
    "fwupd",
    "bolt",
    "colord",
    "switcheroo-control",
    "iio-sensor-proxy",
    "plymouth",
    "cifs-utils",
    "nfs-common",
];

/// The helper, run as root by `pkexec`.
pub const HELPER: &str = "/usr/libexec/orbit-services-helper";
/// The firewall's service files: the packages', then this machine's own.
pub const FIREWALL_DIRS: [&str; 2] = ["/usr/share/orbit/firewall.d", "/etc/orbit/firewall.d"];
/// Seconds a step on trial waits to be kept before it is undone.
pub const TRIAL_SECONDS: u32 = 30;

/// The page's groups, in its order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Group {
    Security,
    Network,
    RemoteAccess,
    Media,
    Storage,
}

impl Group {
    pub const ALL: [Group; 5] = [Group::Security, Group::Network, Group::RemoteAccess, Group::Media, Group::Storage];

    pub fn id(self) -> &'static str {
        match self {
            Group::Security => "security",
            Group::Network => "network",
            Group::RemoteAccess => "remote-access",
            Group::Media => "media",
            Group::Storage => "storage",
        }
    }

    /// The English title (`orbitctl`); Orbit-Settings translates the id.
    pub fn title(self) -> &'static str {
        match self {
            Group::Security => "Security",
            Group::Network => "Network",
            Group::RemoteAccess => "Remote access",
            Group::Media => "Media",
            Group::Storage => "Storage",
        }
    }
}

/// How a service is on.
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    /// A system service: on is its unit enabled; running, active.
    System { unit: &'static str },
    /// Started by the user's session when `~/.config/orbit/<config>.yaml`
    /// (else `/etc/orbit/<config>.yaml`) says `start: true`; running is a
    /// process of the user's whose command line matches `pattern`.
    Session {
        config: &'static str,
        program: &'static str,
        pattern: &'static str,
    },
}

/// A row of the page.
#[derive(Debug, Clone, Copy)]
pub struct Service {
    pub id: &'static str,
    pub group: Group,
    /// English name and summary (`orbitctl`); Orbit-Settings translates
    /// `services-<id>` and `services-<id>-summary`.
    pub name: &'static str,
    pub summary: &'static str,
    pub package: &'static str,
    pub kind: Kind,
    /// Its file in /usr/share/orbit/firewall.d, if it opens ports.
    pub firewall: Option<&'static str>,
    /// Whether the page offers to uninstall it (not when the desktop
    /// itself depends on it).
    pub removable: bool,
    /// Turning it on could cut this machine off: kept only when confirmed.
    pub trial: bool,
    /// Turning it off lets something in, or out, that the user should know
    /// of first (the page and `orbitctl` ask).
    pub warn_off: Option<&'static str>,
}

/// Every service, in the page's order.
pub const SERVICES: &[Service] = &[
    Service {
        id: "firewall",
        group: Group::Security,
        name: "Firewall",
        summary: "Drops what comes in, but for what each service opens",
        package: "orbit-firewall",
        kind: Kind::System {
            unit: "orbit-firewall.service",
        },
        firewall: None,
        removable: false,
        trial: true,
        warn_off: Some("Everything that reaches this machine will be let in."),
    },
    Service {
        id: "remote",
        group: Group::RemoteAccess,
        name: "Orbit Remote Server",
        summary: "This session, shared with a browser or a station",
        package: "orbit-remote-server",
        kind: Kind::Session {
            config: "remote",
            program: "orbit-remote-server",
            pattern: "^[^ ]*orbit-remote-server serve",
        },
        firewall: Some("10-remote.nft"),
        removable: true,
        trial: false,
        warn_off: Some("A station or browser connected now is let go."),
    },
];

/// The service `id`.
pub fn find(id: &str) -> Option<&'static Service> {
    SERVICES.iter().find(|s| s.id == id)
}

/// What a service is now.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct State {
    /// The installed version; `None`: not installed.
    pub installed: Option<String>,
    /// Set to be on (its unit enabled; its session's `start: true`).
    pub on: bool,
    /// Running now.
    pub running: bool,
}

/// The state of `service`, read now.
pub fn state(service: &Service) -> State {
    let installed = installed_version(service.package);
    let (on, running) = match service.kind {
        Kind::System { unit } => (systemctl_quiet("is-enabled", unit), systemctl_quiet("is-active", unit)),
        Kind::Session { config, pattern, .. } => (session_flag(config, "start"), user_process(pattern)),
    };
    // Not installed is not on, whatever a file left behind says.
    State {
        on: on && installed.is_some(),
        installed,
        running,
    }
}

/// The version of `package` dpkg has installed.
pub fn installed_version(package: &str) -> Option<String> {
    let out = Command::new("dpkg-query")
        .args(["-W", "-f", "${db:Status-Status}\t${Version}", package])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (status, version) = text.split_once('\t')?;
    (status == "installed").then(|| version.trim().to_owned())
}

fn systemctl_quiet(verb: &str, unit: &str) -> bool {
    Command::new("systemctl")
        .args([verb, "--quiet", unit])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

fn user_process(pattern: &str) -> bool {
    Command::new("pgrep")
        .args(["-u", &uid().to_string(), "-f", pattern])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

// --- The session's YAML ---------------------------------------------------

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".config"))
}

fn state_home() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local/state"))
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// The user's `<name>.yaml` in ~/.config/orbit.
pub fn user_config(name: &str) -> PathBuf {
    config_home().join("orbit").join(format!("{name}.yaml"))
}

/// The machine's `<name>.yaml` in /etc/orbit.
pub fn system_config(name: &str) -> PathBuf {
    PathBuf::from("/etc/orbit").join(format!("{name}.yaml"))
}

/// The file the session reads: the user's, else the machine's.
fn session_config(name: &str) -> PathBuf {
    let user = user_config(name);
    if user.exists() { user } else { system_config(name) }
}

/// `key: true` in the file the session reads, as `orbit-session` reads it.
pub fn session_flag(name: &str, key: &str) -> bool {
    fs::read_to_string(session_config(name)).is_ok_and(|text| flag(&text, key) == Some(true))
}

/// The value of a top-level `key: true|false` line.
pub fn flag(text: &str, key: &str) -> Option<bool> {
    text.lines().find_map(|line| {
        let rest = line.strip_prefix(key)?.strip_prefix(':')?;
        let value = rest.split('#').next()?.trim();
        match value {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    })
}

/// `text` with `key:` set to `value`: its line rewritten (a comment after
/// it kept), or one added at the end. Every other line stays as it was.
pub fn with_flag(text: &str, key: &str, value: bool) -> String {
    let mut out = Vec::new();
    let mut done = false;
    for line in text.lines() {
        if !done && let Some(rest) = line.strip_prefix(key).and_then(|r| r.strip_prefix(':')) {
            let comment = rest.find('#').map(|i| format!("  {}", &rest[i..])).unwrap_or_default();
            out.push(format!("{key}: {value}{comment}"));
            done = true;
        } else {
            out.push(line.to_owned());
        }
    }
    if !done {
        out.push(format!("{key}: {value}"));
    }
    let mut joined = out.join("\n");
    joined.push('\n');
    joined
}

/// Sets `key: value` in the user's `<name>.yaml`; a file the user did not
/// have yet starts as a copy of the machine's, comments and all.
pub fn set_session_flag(name: &str, key: &str, value: bool) -> io::Result<()> {
    let path = user_config(name);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => fs::read_to_string(system_config(name)).unwrap_or_default(),
        Err(e) => return Err(e),
    };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("yaml.tmp");
    fs::write(&tmp, with_flag(&text, key, value))?;
    fs::rename(&tmp, &path)
}

// --- The ports ------------------------------------------------------------

/// Who a port is open to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// The local network and the tunnels.
    Lan,
    /// Anyone.
    Anyone,
}

/// A port a firewall file opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Port {
    /// `tcp` or `udp`.
    pub proto: String,
    pub port: String,
    /// The answers of a discovery: the port is where they come from.
    pub source: bool,
    pub scope: Scope,
    /// The rule's comment.
    pub note: String,
}

impl Port {
    /// `7311/tcp`, `1900/udp (answers)`.
    pub fn label(&self) -> String {
        if self.source {
            format!("{}/{} (answers)", self.port, self.proto)
        } else {
            format!("{}/{}", self.port, self.proto)
        }
    }
}

/// The ports a firewall file's `add rule inet orbit … accept` lines open.
pub fn parse_rules(text: &str) -> Vec<Port> {
    text.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("add rule inet orbit ")?;
            let (chain, rest) = rest.split_once(' ')?;
            let scope = match chain {
                "lan_in" => Scope::Lan,
                "any_in" => Scope::Anyone,
                _ => return None,
            };
            let (rule, note) = match rest.split_once(" comment ") {
                Some((rule, note)) => (rule, note.trim().trim_matches('"').to_owned()),
                None => (rest, String::new()),
            };
            let words: Vec<&str> = rule.split_whitespace().collect();
            let at = words.iter().position(|w| *w == "tcp" || *w == "udp")?;
            let proto = words[at].to_owned();
            let dir = *words.get(at + 1)?;
            let port = (*words.get(at + 2)?).to_owned();
            if dir != "dport" && dir != "sport" {
                return None;
            }
            Some(Port {
                proto,
                port,
                source: dir == "sport",
                scope,
                note,
            })
        })
        .collect()
}

/// The ports `service` opens, from its package's firewall file.
pub fn ports(service: &Service) -> Vec<Port> {
    service
        .firewall
        .and_then(|file| fs::read_to_string(Path::new(FIREWALL_DIRS[0]).join(file)).ok())
        .map(|text| parse_rules(&text))
        .unwrap_or_default()
}

/// Every firewall file, the packages' then this machine's, with what each
/// opens: what the firewall's own row shows.
pub fn firewall_files() -> Vec<(PathBuf, Vec<Port>)> {
    let mut files = Vec::new();
    for dir in FIREWALL_DIRS {
        let Ok(entries) = fs::read_dir(dir) else { continue };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "nft"))
            .collect();
        paths.sort();
        for path in paths {
            let rules = fs::read_to_string(&path).map(|t| parse_rules(&t)).unwrap_or_default();
            files.push((path, rules));
        }
    }
    files
}

// --- Turning on and off ---------------------------------------------------

/// What the user asks of a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    On,
    Off,
    Remove,
}

impl Action {
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "on" => Some(Action::On),
            "off" => Some(Action::Off),
            "remove" => Some(Action::Remove),
            _ => None,
        }
    }
}

/// The logind settings the helper may set (`logind KEY VALUE`): what the
/// lid does on battery and on mains, and what the power key does.
pub const LOGIND_KEYS: [&str; 3] = ["HandleLidSwitch", "HandleLidSwitchExternalPower", "HandlePowerKey"];
/// The values they may take.
pub const LOGIND_VALUES: [&str; 6] = ["suspend", "hibernate", "poweroff", "lock", "ignore", "suspend-then-hibernate"];

/// A step that needs root, as the helper takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Privileged {
    Install(String),
    Remove(String),
    UnitOn(String),
    UnitOff(String),
    /// One of [`LOGIND_KEYS`] set to one of [`LOGIND_VALUES`].
    Logind(String, String),
    /// apt's lists fetched again.
    Refresh,
    /// Every update installed (`apt-get dist-upgrade`).
    Upgrade,
    /// What unattended-upgrades installs alone: one of
    /// [`updates::AUTO_MODES`].
    AutoUpdates(String),
    /// The boot loader's entries and the snapshots, read (an `info` line).
    BootInfo,
    /// Seconds the boot menu waits (0: hidden behind a key).
    BootTimeout(u32),
    /// The boot loader's default entry (an id `bootctl list` gives).
    BootDefault(String),
    /// A snapshot of the system now, with this description.
    Snapshot(String),
    /// Plymouth's theme for the boot (a theme installed), the initramfs
    /// made again.
    BootTheme(String),
    /// A network drive for every user, mounted at boot (an address
    /// [`drives::SystemDrive`] takes); an SMB share's password comes on
    /// the helper's standard input, a line.
    SystemDrive(String),
    /// That drive taken away: unmounted, its units and password deleted.
    SystemDriveRemove(String),
}

impl Privileged {
    /// Its words on the helper's command line.
    pub fn args(&self) -> Vec<&str> {
        match self {
            Privileged::Install(p) => vec!["install", p],
            Privileged::Remove(p) => vec!["remove", p],
            Privileged::UnitOn(u) => vec!["unit-on", u],
            Privileged::UnitOff(u) => vec!["unit-off", u],
            Privileged::Logind(k, v) => vec!["logind", k, v],
            Privileged::Refresh => vec!["refresh"],
            Privileged::Upgrade => vec!["upgrade"],
            Privileged::AutoUpdates(m) => vec!["auto-updates", m],
            Privileged::BootInfo => vec!["boot-info"],
            Privileged::BootTimeout(_) => vec!["boot-timeout", self.number()],
            Privileged::BootDefault(id) => vec!["boot-default", id],
            Privileged::Snapshot(d) => vec!["snapshot", d],
            Privileged::BootTheme(t) => vec!["boot-theme", t],
            Privileged::SystemDrive(a) => vec!["system-drive", a],
            Privileged::SystemDriveRemove(a) => vec!["system-drive-remove", a],
        }
    }

    fn number(&self) -> &str {
        match self {
            Privileged::BootTimeout(n) => BOOT_TIMEOUTS.iter().find(|(v, _)| v == n).map_or("0", |(_, s)| s),
            _ => "",
        }
    }

    /// The steps of one word: `refresh`, `upgrade`.
    pub fn word(verb: &str) -> Result<Self, String> {
        match verb {
            "refresh" => Ok(Privileged::Refresh),
            "upgrade" => Ok(Privileged::Upgrade),
            "boot-info" => Ok(Privileged::BootInfo),
            _ => Err(format!("unknown step: {verb}")),
        }
    }

    /// `logind KEY VALUE`, refused unless both are known.
    pub fn logind(key: &str, value: &str) -> Result<Self, String> {
        if !LOGIND_KEYS.contains(&key) {
            return Err(format!("not a logind setting Orbit sets: {key}"));
        }
        if !LOGIND_VALUES.contains(&value) {
            return Err(format!("not a value for {key}: {value}"));
        }
        Ok(Privileged::Logind(key.to_owned(), value.to_owned()))
    }

    /// The step from its words, refused unless its package or unit is one
    /// of the catalogue's.
    pub fn parse(verb: &str, arg: &str) -> Result<Self, String> {
        let package = || SERVICES.iter().any(|s| s.package == arg) || EXTRA_PACKAGES.contains(&arg);
        let unit = || {
            SERVICES
                .iter()
                .any(|s| matches!(s.kind, Kind::System { unit } if unit == arg))
        };
        // A loader entry: letters, digits and - _ . + (`bootctl list`).
        let entry = || !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_.+@".contains(c));
        match verb {
            "boot-timeout" => BOOT_TIMEOUTS
                .iter()
                .find(|(_, s)| *s == arg)
                .map(|(v, _)| Privileged::BootTimeout(*v))
                .ok_or_else(|| format!("not a boot menu time: {arg}")),
            "boot-default" if entry() => Ok(Privileged::BootDefault(arg.to_owned())),
            "boot-default" => Err(format!("not a boot entry: {arg}")),
            "snapshot" if arg.chars().count() <= 80 && !arg.chars().any(char::is_control) => Ok(Privileged::Snapshot(arg.to_owned())),
            "snapshot" => Err("a snapshot's description: one line, at most 80 characters".into()),
            "boot-theme" if arg.len() <= 64 && entry() && !arg.starts_with('.') => Ok(Privileged::BootTheme(arg.to_owned())),
            "boot-theme" => Err(format!("not a boot theme: {arg}")),
            "system-drive" => drives::SystemDrive::parse(arg).map(|d| Privileged::SystemDrive(d.address())),
            "system-drive-remove" => drives::SystemDrive::parse(arg).map(|d| Privileged::SystemDriveRemove(d.address())),
            "auto-updates" if updates::AUTO_MODES.contains(&arg) => Ok(Privileged::AutoUpdates(arg.to_owned())),
            "auto-updates" => Err(format!("not a mode of the automatic updates: {arg}")),
            "install" if package() => Ok(Privileged::Install(arg.to_owned())),
            "remove" if package() => Ok(Privileged::Remove(arg.to_owned())),
            "unit-on" if unit() => Ok(Privileged::UnitOn(arg.to_owned())),
            "unit-off" if unit() => Ok(Privileged::UnitOff(arg.to_owned())),
            "install" | "remove" => Err(format!("not a package of Orbit's services: {arg}")),
            "unit-on" | "unit-off" => Err(format!("not a unit of Orbit's services: {arg}")),
            _ => Err(format!("unknown step: {verb}")),
        }
    }

    /// The step that undoes this one, for a trial (installing and removing
    /// are not undone: they cut nothing off).
    pub fn undo(&self) -> Option<Privileged> {
        match self {
            Privileged::UnitOn(u) => Some(Privileged::UnitOff(u.clone())),
            Privileged::UnitOff(u) => Some(Privileged::UnitOn(u.clone())),
            _ => None,
        }
    }
}

/// What turning a service on, off or away takes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Steps for the helper, in order.
    pub privileged: Vec<Privileged>,
    /// The session's `start:` set to this, afterwards.
    pub session: Option<bool>,
    /// Applied on trial.
    pub trial: bool,
}

/// What `action` on `service`, in `state`, takes; an error says why it
/// cannot be done.
pub fn plan(service: &Service, state: &State, action: Action) -> Result<Plan, String> {
    let mut plan = Plan::default();
    match action {
        Action::On => {
            if state.installed.is_none() {
                plan.privileged.push(Privileged::Install(service.package.into()));
            }
            match service.kind {
                Kind::System { unit } => plan.privileged.push(Privileged::UnitOn(unit.into())),
                Kind::Session { .. } => plan.session = Some(true),
            }
            plan.trial = service.trial;
        }
        Action::Off => {
            if state.installed.is_none() {
                return Err(format!("{} is not installed", service.name));
            }
            match service.kind {
                Kind::System { unit } => plan.privileged.push(Privileged::UnitOff(unit.into())),
                Kind::Session { .. } => plan.session = Some(false),
            }
        }
        Action::Remove => {
            if !service.removable {
                return Err(format!("{} cannot be removed: the desktop depends on it", service.name));
            }
            if state.installed.is_none() {
                return Err(format!("{} is not installed", service.name));
            }
            match service.kind {
                Kind::System { unit } => plan.privileged.push(Privileged::UnitOff(unit.into())),
                Kind::Session { .. } => plan.session = Some(false),
            }
            plan.privileged.push(Privileged::Remove(service.package.into()));
        }
    }
    Ok(plan)
}

/// A line the helper prints.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Starting a step: its description.
    Step(String),
    /// apt's progress, 0–100, and what it is doing.
    Progress(u8, String),
    /// Applied on trial: undone after this many seconds unless kept.
    Trial(u32),
    Kept,
    Undone,
    Error(String),
    Done,
    /// What a reading step found: one JSON line (`boot-info`).
    Info(String),
}

impl Event {
    pub fn parse(line: &str) -> Option<Self> {
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        Some(match word {
            "step" => Event::Step(rest.into()),
            "progress" => {
                let (n, text) = rest.split_once(' ').unwrap_or((rest, ""));
                Event::Progress(n.parse().ok()?, text.into())
            }
            "trial" => Event::Trial(rest.parse().ok()?),
            "kept" => Event::Kept,
            "undone" => Event::Undone,
            "error" => Event::Error(rest.into()),
            "done" => Event::Done,
            "info" => Event::Info(rest.into()),
            _ => return None,
        })
    }
}

/// The helper at work.
pub struct Run {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: std::io::Lines<BufReader<ChildStdout>>,
}

impl Run {
    /// Starts the helper on `steps` — as root already, directly; else by
    /// `pkexec`, which asks for the password.
    pub fn spawn(steps: &[Privileged], trial: bool) -> io::Result<Run> {
        let mut command = if uid() == 0 {
            Command::new(HELPER)
        } else {
            let mut c = Command::new("pkexec");
            c.arg(HELPER);
            c
        };
        if trial {
            command.args(["--trial", &TRIAL_SECONDS.to_string()]);
        }
        for (i, step) in steps.iter().enumerate() {
            if i > 0 {
                command.arg("--");
            }
            command.args(step.args());
        }
        let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()?;
        let stdout = child.stdout.take().expect("piped");
        Ok(Run {
            stdin: child.stdin.take(),
            child,
            lines: BufReader::new(stdout).lines(),
        })
    }

    /// The helper's next event; `None` when it has ended.
    pub fn next_event(&mut self) -> Option<Event> {
        loop {
            let line = self.lines.next()?.ok()?;
            if let Some(event) = Event::parse(&line) {
                return Some(event);
            }
        }
    }

    /// Keeps what was applied on trial.
    pub fn keep(&mut self) {
        self.say("keep");
    }

    /// Undoes what was applied on trial, without waiting for its time.
    pub fn undo(&mut self) {
        self.say("undo");
    }

    /// A line on the helper's standard input: the password a
    /// [`Privileged::SystemDrive`] step reads.
    pub fn send(&mut self, line: &str) {
        self.say(line);
    }

    fn say(&mut self, word: &str) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = writeln!(stdin, "{word}");
        }
    }

    /// Waits for the helper; whether it succeeded (pkexec says 126 when the
    /// password was refused or the dialog dismissed, 127 when not
    /// authorised).
    pub fn wait(mut self) -> io::Result<std::process::ExitStatus> {
        drop(self.stdin.take());
        self.child.wait()
    }
}

/// Starts or stops a session service now, in this session. Starting
/// needs a session to start it in (a Wayland display); outside one (over
/// SSH), it starts with the next session.
pub fn session_now(service: &Service, on: bool) -> io::Result<bool> {
    let Kind::Session { config, program, pattern } = service.kind else {
        return Ok(false);
    };
    if !on {
        let _ = Command::new("pkill")
            .args(["-u", &uid().to_string(), "-f", pattern])
            .status();
        return Ok(true);
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_none() || user_process(pattern) {
        return Ok(false);
    }
    let mut args = vec!["serve"];
    if session_flag(config, "virtual") {
        args.push("--virtual");
    }
    let dir = state_home().join("orbit");
    fs::create_dir_all(&dir)?;
    let log = fs::File::create(dir.join(format!("{config}.log")))?;
    Command::new("setsid")
        .arg(program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()?;
    Ok(true)
}

/// A row as `orbitctl services --json` gives it.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub id: &'static str,
    pub group: Group,
    pub name: &'static str,
    pub summary: &'static str,
    pub package: &'static str,
    #[serde(flatten)]
    pub state: State,
    pub ports: Vec<Port>,
    pub removable: bool,
}

/// Every row, read now.
pub fn rows() -> Vec<Row> {
    SERVICES
        .iter()
        .map(|s| Row {
            id: s.id,
            group: s.group,
            name: s.name,
            summary: s.summary,
            package: s.package,
            state: state(s),
            ports: ports(s),
            removable: s.removable,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_are_read() {
        let text = "# a comment\n\
            add rule inet orbit lan_in tcp dport 7311 accept comment \"Orbit Remote Server\"\n\
            add rule inet orbit lan_in udp sport 1900 accept comment \"SSDP answers\"\n\
            add rule inet orbit any_in ip saddr 0.0.0.0 udp dport 67 accept\n\
            add rule inet orbit input tcp dport 1 accept\n";
        let ports = parse_rules(text);
        assert_eq!(ports.len(), 3);
        assert_eq!(ports[0].label(), "7311/tcp");
        assert_eq!(ports[0].note, "Orbit Remote Server");
        assert_eq!(ports[1].label(), "1900/udp (answers)");
        assert_eq!(ports[2].scope, Scope::Anyone);
        assert_eq!(ports[2].port, "67");
    }

    #[test]
    fn flags_are_read_and_set_keeping_the_rest() {
        let text = "# The remote server.\nstart: false   # off by default\nvirtual: true\n";
        assert_eq!(flag(text, "start"), Some(false));
        assert_eq!(flag(text, "virtual"), Some(true));
        assert_eq!(flag(text, "missing"), None);
        let on = with_flag(text, "start", true);
        assert_eq!(on, "# The remote server.\nstart: true  # off by default\nvirtual: true\n");
        assert_eq!(with_flag("", "start", true), "start: true\n");
        // A key that only starts the same is another key.
        assert_eq!(flag("started: true\n", "start"), None);
    }

    #[test]
    fn the_helper_takes_only_the_catalogue() {
        assert_eq!(
            Privileged::parse("install", "orbit-remote-server"),
            Ok(Privileged::Install("orbit-remote-server".into()))
        );
        assert!(Privileged::parse("install", "openssh-server").is_err());
        assert!(Privileged::parse("unit-on", "ssh.service").is_err());
        assert!(Privileged::parse("unit-on", "orbit-firewall.service").is_ok());
        assert!(Privileged::parse("rm", "-rf").is_err());
        assert!(Privileged::logind("HandleLidSwitch", "suspend").is_ok());
        assert!(Privileged::logind("HandleLidSwitch", "rm -rf /").is_err());
        assert!(Privileged::logind("KillUserProcesses", "yes").is_err());
    }

    #[test]
    fn plans() {
        let firewall = find("firewall").unwrap();
        let remote = find("remote").unwrap();
        let none = State::default();
        let installed = State {
            installed: Some("1".into()),
            ..State::default()
        };
        let p = plan(remote, &none, Action::On).unwrap();
        assert_eq!(p.privileged, vec![Privileged::Install("orbit-remote-server".into())]);
        assert_eq!(p.session, Some(true));
        let p = plan(firewall, &installed, Action::On).unwrap();
        assert_eq!(p.privileged, vec![Privileged::UnitOn("orbit-firewall.service".into())]);
        assert!(p.trial);
        assert!(plan(firewall, &installed, Action::Remove).is_err());
        assert!(plan(remote, &none, Action::Off).is_err());
        let p = plan(remote, &installed, Action::Remove).unwrap();
        assert_eq!(p.privileged, vec![Privileged::Remove("orbit-remote-server".into())]);
        assert_eq!(p.session, Some(false));
    }

    #[test]
    fn events() {
        assert_eq!(Event::parse("progress 42 Installing x"), Some(Event::Progress(42, "Installing x".into())));
        assert_eq!(Event::parse("trial 30"), Some(Event::Trial(30)));
        assert_eq!(Event::parse("Reading package lists..."), None);
    }
}
