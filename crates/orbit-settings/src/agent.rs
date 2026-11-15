//! `orbit-settings agent`: what the session needs running in the
//! background for two pages, started from core.yaml's `login:` (after the
//! first unlock: nothing is mounted for a locked screen).
//!
//! - Removable media (Storage): a pen drive, a disk, a camera or a phone
//!   plugged in is mounted and opened in Files, only mounted, or left
//!   alone, as `media.yaml` says (`disks:` and `cameras:`, each `open`,
//!   `mount` or `nothing`; `open` by default). The drives there before the
//!   agent started are left as they are. An encrypted one asks for its
//!   passphrase (GTK's mount dialog).
//! - The location service (Privacy): geoclue's agent for this user. Apps
//!   get a location only while `org.gnome.system.location enabled` is on,
//!   at most as precise as its `max-accuracy-level`; the portal still asks
//!   per app. geoclue lets only the agents its configuration lists in
//!   (`/etc/geoclue/conf.d/90-orbit.conf`, from the package).

use std::{path::PathBuf, rc::Rc};

use gtk::{gio, glib, prelude::*};
use orbit_common::ConfigDir;
use serde_yaml_ng::Value;

/// The kinds of removable media `media.yaml` tells apart.
pub const MEDIA_KINDS: [&str; 2] = ["disks", "cameras"];
/// What can be done with one, in the page's order.
pub const MEDIA_ACTIONS: [&str; 3] = ["open", "mount", "nothing"];

/// What `media.yaml` says to do with `kind`.
pub fn media_action(dir: &ConfigDir, kind: &str) -> &'static str {
    let said = std::fs::read_to_string(dir.path().join("media.yaml"))
        .ok()
        .and_then(|t| serde_yaml_ng::from_str::<Value>(&t).ok())
        .and_then(|v| v.get(kind).and_then(Value::as_str).map(str::to_owned));
    said.and_then(|s| MEDIA_ACTIONS.iter().find(|a| **a == s).copied()).unwrap_or("open")
}

pub fn save_media_action(dir: &ConfigDir, kind: &str, action: &str) -> anyhow::Result<()> {
    dir.save_app_keys("media", &[kind], &format!("{kind}: {action}\n"))
}

/// A camera or a phone: GVfs's gphoto2, MTP or Apple volumes.
fn is_camera(volume: &gio::Volume) -> bool {
    volume
        .activation_root()
        .and_then(|r| r.uri_scheme())
        .is_some_and(|s| ["gphoto2", "mtp", "afc"].contains(&s.as_str()))
}

/// Where Files opens: a camera's photos when it has the folder.
fn opened(mount: &gio::Mount) -> gio::File {
    let root = mount.root();
    let photos = root.child("DCIM");
    if photos.query_exists(gio::Cancellable::NONE) { photos } else { root }
}

fn open(mount: &gio::Mount) {
    let uri = opened(mount).uri();
    if let Err(e) = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE) {
        log::warn!("{uri}: {e}");
    }
}

fn volume_added(dir: &ConfigDir, volume: &gio::Volume) {
    let camera = is_camera(volume);
    let action = media_action(dir, if camera { "cameras" } else { "disks" });
    // A disk the system does not hint as removable (udisks' auto hint)
    // is not ours to mount.
    if action == "nothing" || !volume.can_mount() || (!camera && !volume.should_automount()) {
        return;
    }
    let name = volume.name();
    log::info!("{name}: plugged in, {action}");
    let operation = gtk::MountOperation::new(None::<&gtk::Window>);
    let v = volume.clone();
    volume.mount(gio::MountMountFlags::NONE, Some(&operation), gio::Cancellable::NONE, move |result| {
        match result {
            Err(e) if !e.matches(gio::IOErrorEnum::AlreadyMounted) => {
                if !e.matches(gio::IOErrorEnum::FailedHandled) {
                    log::warn!("{name}: {e}");
                }
                return;
            }
            _ => {}
        }
        if action == "open"
            && let Some(mount) = v.get_mount()
        {
            open(&mount);
        }
    });
}

/// The `MaxAccuracyLevel`s geoclue knows, by GSettings' names.
fn accuracy(settings: &gio::Settings) -> u32 {
    if !settings.boolean("enabled") {
        return 0;
    }
    match settings.string("max-accuracy-level").as_str() {
        "country" => 1,
        "city" => 4,
        "neighborhood" => 5,
        "street" => 6,
        _ => 8,
    }
}

const GEOCLUE: &str = "org.freedesktop.GeoClue2";
const AGENT_PATH: &str = "/org/freedesktop/GeoClue2/Agent";
const AGENT: &str = "org.freedesktop.GeoClue2.Agent";
/// The id geoclue's configuration lists.
const AGENT_ID: &str = "orbit-settings";
const AGENT_XML: &str = r#"<node>
  <interface name="org.freedesktop.GeoClue2.Agent">
    <method name="AuthorizeApp">
      <arg name="desktop_id" type="s" direction="in"/>
      <arg name="req_accuracy_level" type="u" direction="in"/>
      <arg name="authorized" type="b" direction="out"/>
      <arg name="allowed_accuracy_level" type="u" direction="out"/>
    </method>
    <property name="MaxAccuracyLevel" type="u" access="read"/>
  </interface>
</node>"#;

/// The schema the location switch is kept in, when it is installed.
fn location_settings() -> Option<gio::Settings> {
    let source = gio::SettingsSchemaSource::default()?;
    source.lookup("org.gnome.system.location", true)?;
    Some(gio::Settings::new("org.gnome.system.location"))
}

/// Serves geoclue's agent, and tells geoclue each time it starts.
fn location(keep: &mut Vec<Box<dyn std::any::Any>>) -> anyhow::Result<()> {
    let Some(settings) = location_settings() else { return Ok(()) };
    let conn = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)?;
    let node = gio::DBusNodeInfo::for_xml(AGENT_XML)?;
    let info = node.lookup_interface(AGENT).ok_or_else(|| anyhow::anyhow!("no {AGENT}"))?;
    let (calls, props) = (settings.clone(), settings.clone());
    let id = conn
        .register_object(AGENT_PATH, &info)
        .method_call(move |_, _, _, _, method, params, invocation| {
            if method != "AuthorizeApp" {
                invocation.return_value(None);
                return;
            }
            let app = params.child_value(0).get::<String>().unwrap_or_default();
            let asked = params.child_value(1).get::<u32>().unwrap_or(0);
            let most = accuracy(&calls);
            log::info!("location for {app}: {}", if most > 0 { "allowed" } else { "refused (off)" });
            invocation.return_value(Some(&(most > 0, asked.min(most)).to_variant()));
        })
        .property(move |_, _, _, _, _| accuracy(&props).to_variant())
        .build()?;
    keep.push(Box::new(id));
    // The level changed: geoclue follows the property.
    let told = conn.clone();
    let handler = settings.connect_changed(None, move |s, _| {
        let changed = glib::VariantDict::new(None);
        changed.insert("MaxAccuracyLevel", accuracy(s));
        let body = (AGENT, changed.end(), Vec::<String>::new()).to_variant();
        if let Err(e) = told.emit_signal(None, AGENT_PATH, "org.freedesktop.DBus.Properties", "PropertiesChanged", Some(&body)) {
            log::warn!("location: {e}");
        }
    });
    keep.push(Box::new((settings, handler)));
    let watch = gio::bus_watch_name_on_connection(
        &conn,
        GEOCLUE,
        gio::BusNameWatcherFlags::NONE,
        |conn, _, _| {
            conn.call(
                Some(GEOCLUE),
                "/org/freedesktop/GeoClue2/Manager",
                "org.freedesktop.GeoClue2.Manager",
                "AddAgent",
                Some(&(AGENT_ID,).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                5000,
                gio::Cancellable::NONE,
                |r| match r {
                    Ok(_) => log::info!("location: geoclue's agent"),
                    Err(e) => log::warn!("location: geoclue refused the agent: {e}"),
                },
            );
        },
        |_, _| {},
    );
    keep.push(Box::new(watch));
    Ok(())
}

/// core.yaml's line that starts the agent.
pub const AT_LOGIN: &str = "orbit-settings agent";

/// Whether an agent of this user runs.
fn running() -> bool {
    // SAFETY: getuid has no preconditions and cannot fail.
    let me = unsafe { libc::getuid() };
    std::fs::read_dir("/proc").into_iter().flatten().flatten().any(|e| {
        let path = e.path();
        let mine = std::fs::metadata(&path).is_ok_and(|m| std::os::unix::fs::MetadataExt::uid(&m) == me);
        mine && std::fs::read(path.join("cmdline")).is_ok_and(|c| {
            let words: Vec<&[u8]> = c.split(|b| *b == 0).collect();
            words.first().is_some_and(|w| w.ends_with(b"orbit-settings")) && words.contains(&&b"agent"[..])
        })
    })
}

/// The agent kept in `login:`, and started now if it is not running.
pub fn ensure(dir: &ConfigDir) -> anyhow::Result<()> {
    crate::apps_page::ensure_login(dir, AT_LOGIN, true)?;
    if !running() {
        use std::os::unix::process::CommandExt;
        let exe = std::env::current_exe().unwrap_or_else(|_| "orbit-settings".into());
        let mut command = std::process::Command::new(exe);
        if dir.path() != ConfigDir::default_path() {
            command.arg("-c").arg(dir.path());
        }
        command
            .arg("agent")
            .process_group(0)
            .stdin(std::process::Stdio::null())
            .spawn()?;
    }
    Ok(())
}

pub fn run(config: PathBuf) -> glib::ExitCode {
    if let Err(e) = gtk::init() {
        log::error!("{e}");
        return glib::ExitCode::FAILURE;
    }
    let dir = ConfigDir::new(&config);
    let monitor = gio::VolumeMonitor::get();
    let dir = Rc::new(dir);
    monitor.connect_volume_added(move |_, volume| volume_added(&dir, volume));
    let mut keep: Vec<Box<dyn std::any::Any>> = Vec::new();
    if let Err(e) = location(&mut keep) {
        log::warn!("location: {e}");
    }
    glib::MainLoop::new(None, false).run();
    drop(keep);
    glib::ExitCode::SUCCESS
}
