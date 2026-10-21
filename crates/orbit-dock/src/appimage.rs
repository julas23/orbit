//! Adding an AppImage to the dock.
//!
//! The AppImage's own runtime takes out only what the launcher needs
//! (`--appimage-extract PATTERN`, into a scratch directory): its icon
//! (`.DirIcon`, or the `Icon=` of its desktop file) and its desktop file.
//! The icon is kept next to the AppImage, under the AppImage's name; a
//! desktop file is written to the user's applications — the AppImage's own,
//! made to run the AppImage and show that icon, or a minimal one — and the
//! dock points at it.

use std::{
    fs,
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use anyhow::Context;
use gtk::glib;

/// How long one extraction may take.
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(30);

/// Whether `path` is named like an AppImage.
pub fn is_appimage(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("appimage"))
}

/// Takes the icon and desktop file out of `appimage` and writes its
/// launcher; returns the desktop file's path, as `dock.yaml` keeps it.
pub fn install(appimage: &Path) -> anyhow::Result<String> {
    let appimage = appimage
        .canonicalize()
        .with_context(|| format!("{}", appimage.display()))?;
    make_executable(&appimage)?;

    let scratch = Scratch::new()?;
    let extract = Extract { appimage: &appimage, dir: &scratch.0 };
    let desktop = extract.desktop();
    let icon_name = desktop.as_deref().and_then(|text| key(text, "Icon"));
    let icon = extract
        .icon(icon_name.as_deref())
        .and_then(|icon| keep_icon(&appimage, &icon).map_err(|err| log::warn!("dock: {err:#}")).ok());
    if icon.is_none() {
        log::warn!("dock: no icon found in {}", appimage.display());
    }

    let text = match &desktop {
        Some(text) => rewrite(text, &appimage, icon.as_deref()),
        None => minimal(&appimage, icon.as_deref()),
    };
    let name = key(&text, "Name").unwrap_or_else(|| pretty_name(&appimage));
    let dir = glib::user_data_dir().join("applications");
    fs::create_dir_all(&dir).with_context(|| format!("{}", dir.display()))?;
    let path = free_path(&dir, &slug(&name));
    fs::write(&path, text).with_context(|| format!("{}", path.display()))?;
    log::info!("dock: {} → {}", appimage.display(), path.display());
    Ok(tilde(&path))
}

fn make_executable(path: &Path) -> anyhow::Result<()> {
    let mut perms = fs::metadata(path)?.permissions();
    if perms.mode() & 0o100 == 0 {
        // Execute for whoever may read it.
        perms.set_mode(perms.mode() | (perms.mode() & 0o444) >> 2);
        fs::set_permissions(path, perms).with_context(|| format!("making {} executable", path.display()))?;
    }
    Ok(())
}

/// A directory removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> anyhow::Result<Self> {
        let base = glib::user_runtime_dir();
        let base = if base.is_dir() { base } else { std::env::temp_dir() };
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = base.join(format!("orbit-dock-appimage-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).with_context(|| format!("{}", dir.display()))?;
        Ok(Self(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Extract<'a> {
    appimage: &'a Path,
    dir: &'a Path,
}

impl Extract<'_> {
    /// Where the runtime puts what it takes out (some runtimes make it a
    /// link to `AppDir`).
    fn root(&self) -> PathBuf {
        self.dir.join("squashfs-root")
    }

    /// Runs the runtime to take out the files matching `pattern`.
    fn run(&self, pattern: &str) -> bool {
        let child = Command::new(self.appimage)
            .arg("--appimage-extract")
            .arg(pattern)
            .current_dir(self.dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(err) => {
                log::warn!("dock: running {}: {err}", self.appimage.display());
                return false;
            }
        };
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) if start.elapsed() < EXTRACT_TIMEOUT => std::thread::sleep(Duration::from_millis(50)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    log::warn!("dock: {} took too long to extract {pattern}", self.appimage.display());
                    return false;
                }
            }
        }
    }

    /// Takes out `rel` (a path inside the AppImage) and, while it is a
    /// link, what it points at; returns the file.
    fn fetch(&self, rel: &str) -> Option<PathBuf> {
        let root = self.root();
        let mut rel = PathBuf::from(rel.trim_start_matches('/'));
        for _ in 0..8 {
            let path = root.join(&rel);
            if fs::symlink_metadata(&path).is_err() {
                self.run(&rel.to_string_lossy());
            }
            let meta = fs::symlink_metadata(&path).ok()?;
            if !meta.file_type().is_symlink() {
                return meta.is_file().then_some(path);
            }
            let target = fs::read_link(&path).ok()?;
            rel = if target.is_absolute() {
                // Absolute links inside an AppImage name its own root.
                target.strip_prefix("/").ok()?.to_path_buf()
            } else {
                normalize(&rel.parent().unwrap_or(Path::new("")).join(target))?
            };
        }
        None
    }

    /// The AppImage's desktop file (the one at its root).
    fn desktop(&self) -> Option<String> {
        self.run("*.desktop");
        let mut names: Vec<String> = fs::read_dir(self.root())
            .ok()?
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".desktop"))
            .collect();
        names.sort();
        let path = self.fetch(names.first()?)?;
        fs::read_to_string(path).ok()
    }

    /// The AppImage's icon: `.DirIcon`, or the one its desktop file names.
    fn icon(&self, name: Option<&str>) -> Option<PathBuf> {
        let usable = |path: &PathBuf| fs::metadata(path).is_ok_and(|m| m.len() > 0);
        if let Some(path) = self.fetch(".DirIcon").filter(usable) {
            return Some(path);
        }
        let name = name?;
        if name.contains('/') {
            return self.fetch(name).filter(usable);
        }
        ["png", "svg", "xpm"]
            .iter()
            .find_map(|ext| self.fetch(&format!("{name}.{ext}")).filter(usable))
    }
}

/// `a/./b/../c` → `a/c`; `None` if it climbs out of the root.
fn normalize(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::Normal(p) => out.push(p),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(out)
}

/// Copies the icon next to the AppImage, under its name, with the
/// extension its contents call for (unless one is there); returns where.
fn keep_icon(appimage: &Path, icon: &Path) -> anyhow::Result<PathBuf> {
    let ext = icon_kind(icon);
    let stem = appimage.file_stem().unwrap_or_default().to_string_lossy();
    let beside = appimage.with_file_name(format!("{stem}.{ext}"));
    // One already there was put there by hand, or by an earlier add.
    if beside.is_file() {
        return Ok(beside);
    }
    match fs::copy(icon, &beside) {
        Ok(_) => Ok(beside),
        Err(err) => {
            // The AppImage's folder may not be ours to write.
            let dir = glib::user_data_dir().join("orbit").join("dock").join("icons");
            fs::create_dir_all(&dir)?;
            let elsewhere = dir.join(format!("{stem}.{ext}"));
            fs::copy(icon, &elsewhere)
                .with_context(|| format!("copying the icon to {} ({err})", elsewhere.display()))?;
            Ok(elsewhere)
        }
    }
}

/// "png", "svg" or "xpm", by the file's first bytes.
fn icon_kind(path: &Path) -> &'static str {
    let mut head = [0u8; 512];
    let n = fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    let head = &head[..n];
    if head.starts_with(b"\x89PNG") {
        "png"
    } else if head.windows(4).any(|w| w == b"<svg") || head.starts_with(b"<?xml") {
        "svg"
    } else if head.windows(7).any(|w| w == b"XPM */") || head.starts_with(b"/* XPM") {
        "xpm"
    } else {
        "png"
    }
}

/// The value of `key` in the `[Desktop Entry]` group.
fn key(text: &str, key: &str) -> Option<String> {
    let mut main = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            main = line == "[Desktop Entry]";
        } else if main
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            return Some(v.trim().to_string()).filter(|v| !v.is_empty());
        }
    }
    None
}

/// The AppImage's desktop file, made to run the AppImage and show `icon`.
fn rewrite(text: &str, appimage: &Path, icon: Option<&Path>) -> String {
    let program = quote(&appimage.to_string_lossy());
    let mut out = String::new();
    let mut has_icon = false;
    let mut main = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if main && !has_icon {
                push_icon(&mut out, icon);
            }
            main = trimmed == "[Desktop Entry]";
            has_icon = false;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let k = trimmed.split_once('=').map(|(k, _)| k.trim());
        match k {
            Some("Exec") => {
                let args = trimmed.split_once('=').map_or("", |(_, v)| v.trim());
                let args = rest_of_command(args);
                out.push_str(&format!("Exec={program}{args}\n"));
            }
            Some("Icon") if main => {
                has_icon = true;
                match icon {
                    Some(icon) => out.push_str(&format!("Icon={}\n", icon.display())),
                    None => {
                        out.push_str(line);
                        out.push('\n');
                    }
                }
            }
            // Gone from what the AppImage runs as here: a check for a
            // program in PATH, D-Bus activation, AppImage tool hints.
            Some("TryExec" | "DBusActivatable") => {}
            Some(k) if k.starts_with("X-AppImage") => {}
            _ => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    if main && !has_icon {
        push_icon(&mut out, icon);
    }
    out
}

fn push_icon(out: &mut String, icon: Option<&Path>) {
    if let Some(icon) = icon {
        out.push_str(&format!("Icon={}\n", icon.display()));
    }
}

/// `prog --flag %U` → ` --flag %U`: what follows the program, quoted or not.
fn rest_of_command(exec: &str) -> &str {
    let exec = exec.trim_start();
    let end = if let Some(quoted) = exec.strip_prefix('"') {
        let mut escaped = false;
        quoted
            .char_indices()
            .find(|&(_, c)| {
                let close = c == '"' && !escaped;
                escaped = c == '\\' && !escaped;
                close
            })
            .map_or(exec.len(), |(i, _)| i + 2)
    } else {
        exec.find(char::is_whitespace).unwrap_or(exec.len())
    };
    &exec[end..]
}

/// A desktop file for an AppImage that has none.
fn minimal(appimage: &Path, icon: Option<&Path>) -> String {
    let mut out = format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec={}\n",
        pretty_name(appimage),
        quote(&appimage.to_string_lossy()),
    );
    push_icon(&mut out, icon);
    out.push_str("Terminal=false\n");
    out
}

/// A program in an `Exec=` line, quoted as the desktop entry spec says
/// if it has a reserved character (and its backslashes doubled again for
/// the key file).
fn quote(program: &str) -> String {
    if !program.contains(|c: char| c.is_whitespace() || "\"'\\><~|&;$*?#()`".contains(c)) {
        return program.to_string();
    }
    let mut out = String::from("\"");
    for c in program.chars() {
        match c {
            '"' | '`' | '$' => {
                out.push_str("\\\\");
                out.push(c);
            }
            '\\' => out.push_str("\\\\\\\\"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// "Scribus-1.6.4-linux-x86_64.AppImage" → "Scribus": the name's words up
/// to the first that is a version or a platform.
fn pretty_name(appimage: &Path) -> String {
    let stem = appimage.file_stem().unwrap_or_default().to_string_lossy();
    let noise = ["linux", "x86", "x64", "amd64", "x86-64", "aarch64", "arm64", "anylinux", "ubuntu", "appimage"];
    let words: Vec<&str> = stem
        .split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .take_while(|w| {
            let lower = w.to_lowercase();
            let version = lower.starts_with(|c: char| c.is_ascii_digit())
                || (lower.starts_with('v') && lower[1..].starts_with(|c: char| c.is_ascii_digit()));
            !version && !noise.iter().any(|n| lower.starts_with(n))
        })
        .collect();
    if words.is_empty() { stem.into_owned() } else { words.join(" ") }
}

/// "Bambu Studio" → "bambu-studio".
fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-');
    if out.is_empty() { "appimage".into() } else { out.into() }
}

/// `dir/appimage-<slug>.desktop`, numbered if taken.
fn free_path(dir: &Path, slug: &str) -> PathBuf {
    let mut path = dir.join(format!("appimage-{slug}.desktop"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("appimage-{slug}-{n}.desktop"));
        n += 1;
    }
    path
}

/// The path with the home folder as `~`.
fn tilde(path: &Path) -> String {
    match path.strip_prefix(glib::home_dir()) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_from_file_names() {
        assert_eq!(pretty_name(Path::new("Scribus-1.6.4-linux-x86_64.AppImage")), "Scribus");
        assert_eq!(pretty_name(Path::new("Zen_Browser.AppImage")), "Zen Browser");
        assert_eq!(pretty_name(Path::new("BambuStudio_ubuntu22.04-v02.08.AppImage")), "BambuStudio");
        assert_eq!(pretty_name(Path::new("Raspberry_Pi_Imager-v2.0.11.1-desktop-x86_64.AppImage")), "Raspberry Pi Imager");
        assert_eq!(slug("Bambu Studio"), "bambu-studio");
        assert_eq!(quote("/a/b.AppImage"), "/a/b.AppImage");
        assert_eq!(quote("/my apps/b$.AppImage"), "\"/my apps/b\\\\$.AppImage\"");
    }

    #[test]
    fn exec_runs_the_appimage() {
        let text = "[Desktop Entry]\nName=Obsidian\nExec=obsidian %U\nTryExec=obsidian\nIcon=obsidian\nX-AppImage-Version=1\n\
                    [Desktop Action new]\nExec=\"/usr/bin/my app\" --new\n";
        let out = rewrite(text, Path::new("/a/Obsidian.AppImage"), Some(Path::new("/a/Obsidian.png")));
        assert_eq!(
            out,
            "[Desktop Entry]\nName=Obsidian\nExec=/a/Obsidian.AppImage %U\nIcon=/a/Obsidian.png\n\
             [Desktop Action new]\nExec=/a/Obsidian.AppImage --new\n"
        );
        // No Icon key: one is added to the main group.
        let out = rewrite("[Desktop Entry]\nName=X\n", Path::new("/x"), Some(Path::new("/x.png")));
        assert_eq!(out, "[Desktop Entry]\nName=X\nIcon=/x.png\n");
    }

    #[test]
    fn links_stay_inside() {
        assert_eq!(normalize(Path::new("usr/./share/../lib/x.png")), Some(PathBuf::from("usr/lib/x.png")));
        assert_eq!(normalize(Path::new("../etc/passwd")), None);
    }

    /// Adds every AppImage in `$ORBIT_APPIMAGES` and prints what it wrote:
    /// `ORBIT_APPIMAGES=dir cargo test -p orbit-dock -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn install_these() {
        let Ok(dir) = std::env::var("ORBIT_APPIMAGES") else { return };
        for entry in fs::read_dir(dir).unwrap().flatten() {
            if is_appimage(&entry.path()) {
                match install(&entry.path()) {
                    Ok(desktop) => {
                        let text = fs::read_to_string(expand(&desktop)).unwrap();
                        eprintln!("== {desktop}\n{text}");
                    }
                    Err(err) => eprintln!("== {}: {err:#}", entry.path().display()),
                }
            }
        }
    }

    fn expand(path: &str) -> PathBuf {
        orbit_common::config::expand_home(Path::new(path))
    }
}
