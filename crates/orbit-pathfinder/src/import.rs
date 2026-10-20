//! The passwords other browsers saved — Firefox, Chrome, Chromium, Wavebox —
//! read so they can go to Bitwarden (see `bitwarden`). Nothing is written to
//! the browsers' profiles, and nothing decrypted touches the disk.
//!
//! - **Chromium and its kin** keep them in `Login Data` (SQLite), each
//!   password AES-128-CBC encrypted with a key made by PBKDF2 from a secret:
//!   "peanuts" for the `v10` ones, the secret the browser keeps in the
//!   desktop's keyring (Secret Service, read here over D-Bus) for `v11`.
//!   The file is copied first: the browser may hold it locked.
//! - **Firefox** keeps them in `logins.json`, encrypted with the key in
//!   `key4.db`; NSS, Firefox's own library, opens the profile read-only and
//!   decrypts them, given the primary password when one is set. It must be
//!   the NSS of the Firefox that wrote the profile: Mozilla's builds bring a
//!   newer one than Debian's, which cannot read their profiles. So it runs
//!   in a helper — this program started again with `--firefox-helper`,
//!   Firefox's folder first on the library path — that hands the logins
//!   back through a pipe, keeping two NSS versions out of one process.

use std::{
    collections::HashMap,
    ffi::{CString, c_char, c_void},
    path::{Path, PathBuf},
};

use gtk::{gio, glib, prelude::*};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    Firefox,
    Chrome,
    Chromium,
    Wavebox,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Firefox => "Firefox",
            Source::Chrome => "Chrome",
            Source::Chromium => "Chromium",
            Source::Wavebox => "Wavebox",
        }
    }

    /// The name a Chromium browser files its keyring secret under.
    fn keyring_app(self) -> &'static str {
        match self {
            Source::Chrome => "chrome",
            Source::Chromium => "chromium",
            Source::Wavebox => "wavebox",
            Source::Firefox => "",
        }
    }
}

/// A browser profile with saved passwords, found without decrypting any.
#[derive(Debug, Clone)]
pub struct Found {
    pub source: Source,
    /// `Login Data` for Chromium browsers; the profile folder for Firefox.
    pub path: PathBuf,
    pub count: usize,
}

impl Found {
    /// "Chrome (Default)".
    pub fn label(&self) -> String {
        let profile = match self.source {
            Source::Firefox => self.path.file_name(),
            _ => self.path.parent().and_then(|p| p.file_name()),
        };
        match profile.map(|p| p.to_string_lossy().into_owned()) {
            Some(p) => format!("{} ({p})", self.source.label()),
            None => self.source.label().to_owned(),
        }
    }
}

pub struct Login {
    pub source: Source,
    pub url: String,
    pub username: String,
    pub password: String,
}

/// What failed reading a profile.
#[derive(Debug)]
pub enum Error {
    /// Firefox has a primary password, and it was not given (or is wrong).
    FirefoxPassword,
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::FirefoxPassword => write!(f, "Firefox primary password"),
            Error::Other(e) => write!(f, "{e}"),
        }
    }
}

fn other(e: impl std::fmt::Display) -> Error {
    Error::Other(e.to_string())
}

/// Every profile with saved passwords, and how many each has.
pub fn discover() -> Vec<Found> {
    let config = glib::user_config_dir();
    let mut found = Vec::new();
    for (source, dir) in [
        (Source::Wavebox, config.join("wavebox")),
        (Source::Chrome, config.join("google-chrome")),
        (Source::Chromium, config.join("chromium")),
    ] {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut profiles: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path().join("Login Data")).filter(|p| p.is_file()).collect();
        profiles.sort();
        for path in profiles {
            match chromium_count(&path) {
                Ok(count) if count > 0 => found.push(Found { source, path, count }),
                Ok(_) => {}
                Err(err) => log::warn!("{}: {err}", path.display()),
            }
        }
    }
    let firefox = glib::home_dir().join(".mozilla").join("firefox");
    if let Ok(entries) = std::fs::read_dir(&firefox) {
        let mut profiles: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.join("logins.json").is_file()).collect();
        profiles.sort();
        for path in profiles {
            let count = firefox_logins(&path).map(|l| l.iter().filter(|l| !l.password.is_empty()).count()).unwrap_or(0);
            if count > 0 {
                found.push(Found { source: Source::Firefox, path, count });
            }
        }
    }
    found
}

/// The profile's logins, decrypted.
pub fn read(found: &Found, firefox_password: Option<&str>) -> Result<Vec<Login>, Error> {
    match found.source {
        Source::Firefox => read_firefox(&found.path, firefox_password),
        source => read_chromium(source, &found.path),
    }
}

// ------------------------------------------------------------ Chromium

/// A copy of the database (and its write-ahead log) in a private folder,
/// removed when dropped.
struct Copy {
    dir: PathBuf,
    db: PathBuf,
}

impl Copy {
    fn of(db: &Path) -> Result<Self, Error> {
        let dir = private_dir()?.join(format!("import-{}", glib::uuid_string_random()));
        std::fs::create_dir(&dir).map_err(other)?;
        let copy = Copy { db: dir.join("db"), dir };
        std::fs::copy(db, &copy.db).map_err(other)?;
        let wal = PathBuf::from(format!("{}-wal", db.display()));
        if wal.is_file() {
            let _ = std::fs::copy(&wal, copy.dir.join("db-wal"));
        }
        Ok(copy)
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// `$XDG_RUNTIME_DIR/orbit-pathfinder`: memory, not disk, and the owner's
/// alone.
pub fn private_dir() -> Result<PathBuf, Error> {
    use std::os::unix::fs::PermissionsExt;
    let dir = glib::user_runtime_dir().join("orbit-pathfinder");
    std::fs::create_dir_all(&dir).map_err(other)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(other)?;
    Ok(dir)
}

const SAVED: &str = "SELECT origin_url, username_value, password_value FROM logins \
                     WHERE blacklisted_by_user = 0 AND length(password_value) > 0";

fn chromium_count(path: &Path) -> Result<usize, Error> {
    let copy = Copy::of(path)?;
    let db = rusqlite::Connection::open(&copy.db).map_err(other)?;
    let n: i64 = db.query_row(&format!("SELECT count(*) FROM ({SAVED})"), [], |r| r.get(0)).map_err(other)?;
    Ok(n as usize)
}

fn read_chromium(source: Source, path: &Path) -> Result<Vec<Login>, Error> {
    let copy = Copy::of(path)?;
    let db = rusqlite::Connection::open(&copy.db).map_err(other)?;
    let rows: Vec<(String, String, Vec<u8>)> = db
        .prepare(SAVED)
        .map_err(other)?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(other)?
        .filter_map(Result::ok)
        .collect();
    let v10 = chromium_key(b"peanuts");
    // The keyring is asked only when some password needs it.
    let v11 = if rows.iter().any(|(_, _, p)| p.starts_with(b"v11")) {
        keyring_secret(source.keyring_app()).map(|s| chromium_key(&s))
    } else {
        None
    };
    let mut logins = Vec::new();
    let mut failed = 0;
    for (url, username, blob) in rows {
        match chromium_decrypt(&blob, &v10, v11.as_ref()) {
            Some(password) => logins.push(Login { source, url, username, password }),
            None => failed += 1,
        }
    }
    if failed > 0 {
        log::warn!("{}: {failed} passwords could not be decrypted", path.display());
        if logins.is_empty() {
            return Err(Error::Other(format!("{failed} senhas não decifradas (chaveiro trancado?)")));
        }
    }
    Ok(logins)
}

fn chromium_key(secret: &[u8]) -> [u8; 16] {
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(secret, b"saltysalt", 1, &mut key);
    key
}

fn chromium_decrypt(blob: &[u8], v10: &[u8; 16], v11: Option<&[u8; 16]>) -> Option<String> {
    use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
    let (key, data) = match blob.get(..3) {
        Some(b"v10") => (v10, &blob[3..]),
        Some(b"v11") => (v11?, &blob[3..]),
        // Very old profiles kept them in the clear.
        _ => return String::from_utf8(blob.to_vec()).ok(),
    };
    let plain = cbc::Decryptor::<aes::Aes128>::new(key.into(), &[b' '; 16].into())
        .decrypt_padded_vec_mut::<Pkcs7>(data)
        .ok()?;
    String::from_utf8(plain).ok()
}

/// The secret a Chromium browser keeps in the keyring, over the Secret
/// Service D-Bus API ("plain" session: the bus is the user's own).
fn keyring_secret(app: &str) -> Option<Vec<u8>> {
    use glib::variant::ObjectPath;
    let conn = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok()?;
    let call = |path: &str, iface: &str, method: &str, args: glib::Variant, reply: &str| {
        conn.call_sync(
            Some("org.freedesktop.secrets"),
            path,
            iface,
            method,
            Some(&args),
            Some(glib::VariantTy::new(reply).ok()?),
            gio::DBusCallFlags::NONE,
            5000,
            gio::Cancellable::NONE,
        )
        .map_err(|e| log::warn!("keyring {method}: {e}"))
        .ok()
    };
    let opened = call(
        "/org/freedesktop/secrets",
        "org.freedesktop.Secret.Service",
        "OpenSession",
        ("plain", "".to_variant()).to_variant(),
        "(vo)",
    )?;
    let session: ObjectPath = opened.child_value(1).get()?;
    for schema in ["chrome_libsecret_os_crypt_password_v2", "chrome_libsecret_os_crypt_password"] {
        let attrs: HashMap<&str, &str> = HashMap::from([("xdg:schema", schema), ("application", app)]);
        let found = call("/org/freedesktop/secrets", "org.freedesktop.Secret.Service", "SearchItems", (attrs,).to_variant(), "(aoao)")?;
        let unlocked: Vec<ObjectPath> = found.child_value(0).get()?;
        if let Some(item) = unlocked.first() {
            let secret = call(item.as_str(), "org.freedesktop.Secret.Item", "GetSecret", (session.clone(),).to_variant(), "((oayays))")?;
            let (_, _, value, _): (ObjectPath, Vec<u8>, Vec<u8>, String) = secret.child_value(0).get()?;
            return Some(value);
        }
    }
    log::warn!("keyring: no secret for {app} (locked or absent)");
    None
}

// ------------------------------------------------------------ Firefox

struct FirefoxLogin {
    url: String,
    username: String,
    password: String,
}

fn firefox_logins(profile: &Path) -> Result<Vec<FirefoxLogin>, Error> {
    let text = std::fs::read_to_string(profile.join("logins.json")).map_err(other)?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(other)?;
    let field = |l: &serde_json::Value, k: &str| l.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_owned();
    Ok(json
        .get("logins")
        .and_then(|l| l.as_array())
        .map(|logins| {
            logins
                .iter()
                .map(|l| FirefoxLogin {
                    url: Some(field(l, "hostname")).filter(|h| !h.is_empty()).unwrap_or_else(|| field(l, "formSubmitURL")),
                    username: field(l, "encryptedUsername"),
                    password: field(l, "encryptedPassword"),
                })
                .collect()
        })
        .unwrap_or_default())
}

#[repr(C)]
struct SecItem {
    kind: u32,
    data: *mut u8,
    len: u32,
}

/// NSS, loaded for one profile and shut down when dropped.
struct Nss {
    lib: libloading::Library,
}

impl Nss {
    fn open(profile: &Path, password: Option<&str>) -> Result<Self, Error> {
        // SAFETY: loading the system's NSS, whose functions are called below
        // with the signatures nss/pk11pub.h and secitem.h give them.
        unsafe {
            let lib = libloading::Library::new("libnss3.so").map_err(other)?;
            let init: libloading::Symbol<unsafe extern "C" fn(*const c_char) -> i32> = lib.get(b"NSS_Init").map_err(other)?;
            let dir = CString::new(format!("sql:{}", profile.display())).map_err(other)?;
            if init(dir.as_ptr()) != 0 {
                return Err(Error::Other("NSS_Init falhou".into()));
            }
            let nss = Nss { lib };
            let slot_fn: libloading::Symbol<unsafe extern "C" fn() -> *mut c_void> = nss.lib.get(b"PK11_GetInternalKeySlot").map_err(other)?;
            let need: libloading::Symbol<unsafe extern "C" fn(*mut c_void) -> i32> = nss.lib.get(b"PK11_NeedLogin").map_err(other)?;
            let check: libloading::Symbol<unsafe extern "C" fn(*mut c_void, *const c_char) -> i32> = nss.lib.get(b"PK11_CheckUserPassword").map_err(other)?;
            let free: libloading::Symbol<unsafe extern "C" fn(*mut c_void)> = nss.lib.get(b"PK11_FreeSlot").map_err(other)?;
            let slot = slot_fn();
            if slot.is_null() {
                return Err(Error::Other("NSS sem slot".into()));
            }
            let ok = if need(slot) != 0 {
                let pw = CString::new(password.unwrap_or_default()).map_err(other)?;
                check(slot, pw.as_ptr()) == 0
            } else {
                true
            };
            free(slot);
            if !ok {
                return Err(Error::FirefoxPassword);
            }
            Ok(nss)
        }
    }

    fn decrypt(&self, base64: &str) -> Option<String> {
        if base64.is_empty() {
            return Some(String::new());
        }
        let mut data = glib::base64_decode(base64);
        // SAFETY: see `open`; `out` is filled by NSS and freed by it.
        unsafe {
            let sdr: libloading::Symbol<unsafe extern "C" fn(*mut SecItem, *mut SecItem, *mut c_void) -> i32> = self.lib.get(b"PK11SDR_Decrypt").ok()?;
            let zfree: libloading::Symbol<unsafe extern "C" fn(*mut SecItem, i32)> = self.lib.get(b"SECITEM_ZfreeItem").ok()?;
            let mut input = SecItem { kind: 0, data: data.as_mut_ptr(), len: data.len() as u32 };
            let mut out = SecItem { kind: 0, data: std::ptr::null_mut(), len: 0 };
            if sdr(&mut input, &mut out, std::ptr::null_mut()) != 0 {
                return None;
            }
            let text = String::from_utf8(std::slice::from_raw_parts(out.data, out.len as usize).to_vec()).ok();
            zfree(&mut out, 0);
            text
        }
    }
}

impl Drop for Nss {
    fn drop(&mut self) {
        // SAFETY: see `open`.
        unsafe {
            if let Ok(shutdown) = self.lib.get::<unsafe extern "C" fn() -> i32>(b"NSS_Shutdown") {
                shutdown();
            }
        }
    }
}

/// The argument that starts the helper (see the module's notes).
pub const FIREFOX_HELPER: &str = "--firefox-helper";

/// Where Mozilla's Firefox keeps its own NSS.
const FIREFOX_LIBS: &str = "/usr/lib/firefox";

#[derive(serde::Serialize, serde::Deserialize)]
struct HelperReply {
    #[serde(default)]
    password_needed: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    logins: Vec<(String, String, String)>,
}

/// Runs the helper for `profile`; the primary password goes on its stdin.
fn read_firefox(profile: &Path, password: Option<&str>) -> Result<Vec<Login>, Error> {
    use std::io::Write;
    // Tests run from target/*/deps; the program is one folder up.
    let exe = std::env::var_os("ORBIT_PATHFINDER_EXE").map(PathBuf::from).or_else(|| std::env::current_exe().ok()).ok_or_else(|| other("no executable"))?;
    let mut command = std::process::Command::new(exe);
    command.arg(FIREFOX_HELPER).arg(profile).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped());
    if Path::new(FIREFOX_LIBS).join("libnss3.so").is_file() {
        let path = match std::env::var("LD_LIBRARY_PATH") {
            Ok(old) if !old.is_empty() => format!("{FIREFOX_LIBS}:{old}"),
            _ => FIREFOX_LIBS.to_owned(),
        };
        command.env("LD_LIBRARY_PATH", path);
    }
    let mut child = command.spawn().map_err(other)?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(password.unwrap_or_default().as_bytes());
    }
    let out = child.wait_with_output().map_err(other)?;
    let reply: HelperReply = serde_json::from_slice(&out.stdout).map_err(|e| other(format!("helper: {e}")))?;
    if reply.password_needed {
        return Err(Error::FirefoxPassword);
    }
    if let Some(err) = reply.error {
        return Err(Error::Other(err));
    }
    Ok(reply
        .logins
        .into_iter()
        .map(|(url, username, password)| Login { source: Source::Firefox, url, username, password })
        .collect())
}

/// The helper: decrypts the profile given after `--firefox-helper` with the
/// NSS found first on the library path, the primary password read from
/// stdin, and writes the logins as JSON to stdout.
pub fn firefox_helper() -> glib::ExitCode {
    use std::io::Read;
    let profile = std::env::args_os().nth(2).map(PathBuf::from).unwrap_or_default();
    let mut password = String::new();
    let _ = std::io::stdin().read_to_string(&mut password);
    let password = Some(password.as_str()).filter(|p| !p.is_empty());
    let reply = match decrypt_firefox(&profile, password) {
        Ok(logins) => HelperReply { password_needed: false, error: None, logins },
        Err(Error::FirefoxPassword) => HelperReply { password_needed: true, error: None, logins: Vec::new() },
        Err(Error::Other(e)) => HelperReply { password_needed: false, error: Some(e), logins: Vec::new() },
    };
    match serde_json::to_writer(std::io::stdout(), &reply) {
        Ok(()) => glib::ExitCode::SUCCESS,
        Err(_) => glib::ExitCode::FAILURE,
    }
}

fn decrypt_firefox(profile: &Path, password: Option<&str>) -> Result<Vec<(String, String, String)>, Error> {
    let encrypted = firefox_logins(profile)?;
    let nss = Nss::open(profile, password)?;
    let mut logins = Vec::new();
    let mut failed = 0;
    // Entries without a password (sites marked "never save") are skipped.
    for l in encrypted.into_iter().filter(|l| !l.password.is_empty()) {
        match (nss.decrypt(&l.username), nss.decrypt(&l.password)) {
            (Some(username), Some(password)) if !password.is_empty() => logins.push((l.url, username, password)),
            _ => failed += 1,
        }
    }
    if failed > 0 {
        log::warn!("{}: {failed} logins could not be decrypted", profile.display());
    }
    Ok(logins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decrypts_chromium_passwords() {
        use aes::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
        let key = chromium_key(b"peanuts");
        let enc = cbc::Encryptor::<aes::Aes128>::new((&key).into(), &[b' '; 16].into())
            .encrypt_padded_vec_mut::<Pkcs7>(b"segredo123");
        let mut blob = b"v10".to_vec();
        blob.extend(enc);
        assert_eq!(chromium_decrypt(&blob, &key, None).as_deref(), Some("segredo123"));
        // v11 without the keyring's key: not decrypted.
        blob[2] = b'1';
        assert_eq!(chromium_decrypt(&blob, &key, None), None);
    }

    /// Reads the real profiles on this machine and prints only counts.
    /// `cargo test -p orbit-pathfinder real_profiles -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_profiles() {
        // The program built next to this test binary runs the helper.
        let exe = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("orbit-pathfinder");
        // SAFETY: set before any thread of this test starts.
        unsafe { std::env::set_var("ORBIT_PATHFINDER_EXE", exe) };
        for found in discover() {
            match read(&found, None) {
                Ok(logins) => println!("{}: {} de {} decifradas", found.label(), logins.len(), found.count),
                Err(err) => println!("{}: erro: {err}", found.label()),
            }
        }
    }
}
