//! What the context menu offers beyond moving files about: opening with
//! another application, the user's own actions (as Thunar's and Nemo's
//! custom actions: a command line with the files filled in), and archives
//! — compressing into zip, tar.gz, tar.xz, tar.zst or 7z, and extracting
//! here.
//!
//! The actions are in `~/.config/orbit/files-actions.yaml`; without one,
//! "Open a terminal here" is there.

use std::path::{Path, PathBuf};

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::{text, tr, tr_args};
use serde::Deserialize;

use crate::entry::Entry;

/// One custom action.
#[derive(Debug, Clone, Deserialize)]
pub struct Action {
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    /// %f the first file, %F all of them, %d the folder, %n the names.
    pub command: String,
    /// Offered on files…
    #[serde(default = "yes")]
    pub files: bool,
    /// …on folders, and on the folder's background.
    #[serde(default)]
    pub folders: bool,
    /// Only for these types (prefixes: "image/", "video/mp4"); all if empty.
    #[serde(default)]
    pub types: Vec<String>,
}

/// `child` in a box of CSS class `class` (a dialog's padding).
pub fn padded(class: &str, child: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::builder().css_classes([class]).build();
    b.append(child);
    b
}

fn yes() -> bool {
    true
}

pub fn load(dir: &orbit_common::ConfigDir) -> Vec<Action> {
    let file = dir.path().join("files-actions.yaml");
    match std::fs::read_to_string(&file) {
        Ok(text) => serde_yaml_ng::from_str(&text).unwrap_or_else(|err| {
            log::warn!("{}: {err}", file.display());
            Vec::new()
        }),
        Err(_) => default_actions(),
    }
}

fn default_actions() -> Vec<Action> {
    let terminal = ["ghostty", "alacritty", "kitty", "foot", "x-terminal-emulator"]
        .into_iter()
        .find(|t| glib::find_program_in_path(t).is_some());
    let Some(terminal) = terminal else { return Vec::new() };
    let command = match terminal {
        "ghostty" => "ghostty --working-directory=%d",
        "kitty" => "kitty --directory %d",
        "foot" => "foot --working-directory=%d",
        "alacritty" => "alacritty --working-directory %d",
        _ => "x-terminal-emulator",
    };
    vec![Action {
        name: tr("files-action-terminal"),
        icon: Some("utilities-terminal".into()),
        command: command.into(),
        files: false,
        folders: true,
        types: Vec::new(),
    }]
}

impl Action {
    /// Whether it is offered for `selected` (empty: the background).
    pub fn applies(&self, selected: &[Entry]) -> bool {
        if selected.is_empty() {
            return self.folders;
        }
        selected.iter().all(|e| {
            if e.dir {
                self.folders
            } else {
                self.files && (self.types.is_empty() || self.types.iter().any(|t| e.mime.starts_with(t.as_str())))
            }
        })
    }

    /// Runs it in `dir` for `selected`.
    pub fn run(&self, dir: &Path, selected: &[Entry]) {
        let paths: Vec<String> = selected
            .iter()
            .filter_map(|e| e.location.path().map(|p| p.display().to_string()))
            .collect();
        let quote = |s: &str| glib::shell_quote(s).to_string_lossy().into_owned();
        let mut command = self.command.clone();
        command = command.replace("%F", &paths.iter().map(|p| quote(p)).collect::<Vec<_>>().join(" "));
        command = command.replace("%f", &paths.first().map(|p| quote(p)).unwrap_or_default());
        command = command.replace("%d", &quote(&dir.display().to_string()));
        command = command.replace(
            "%n",
            &selected.iter().map(|e| quote(&e.name)).collect::<Vec<_>>().join(" "),
        );
        let argv = match glib::shell_parse_argv(&command) {
            Ok(a) => a,
            Err(err) => {
                log::warn!("action {}: {err}", self.name);
                return;
            }
        };
        let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::NONE);
        launcher.set_cwd(dir);
        let args: Vec<&std::ffi::OsStr> = argv.iter().map(|a| a.as_os_str()).collect();
        if let Err(err) = launcher.spawn(&args) {
            log::warn!("action {}: {err}", self.name);
        }
    }
}

/// Opens `entry` with `app`, or its default application.
pub fn open(entry: &Entry, app: Option<&gio::AppInfo>, display: &gtk::gdk::Display) {
    let file = entry.location.file();
    let context = display.app_launch_context();
    let result = match app {
        Some(app) => app.launch(&[file], Some(&context)),
        None => gio::AppInfo::launch_default_for_uri(&file.uri(), Some(&context)),
    };
    if let Err(err) = result {
        log::warn!("opening {}: {err}", entry.name);
    }
}

/// The applications that open `mime`, the default first.
pub fn apps_for(mime: &str) -> Vec<gio::AppInfo> {
    let mut apps = gio::AppInfo::recommended_for_type(mime);
    for app in gio::AppInfo::all_for_type(mime) {
        if !apps.iter().any(|a| a.id() == app.id()) {
            apps.push(app);
        }
    }
    apps
}

// ------------------------------------------------------------ archives

pub const FORMATS: &[(&str, &str)] = &[
    ("zip", ".zip"),
    ("tar.gz", ".tar.gz"),
    ("tar.xz", ".tar.xz"),
    ("tar.zst", ".tar.zst"),
    ("7z", ".7z"),
];

/// Whether `name` is an archive this can extract.
pub fn is_archive(name: &str) -> bool {
    let n = name.to_lowercase();
    [".zip", ".tar", ".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.bz2", ".tbz2", ".tar.zst", ".7z", ".rar", ".gz", ".xz", ".bz2", ".zst"]
        .iter()
        .any(|e| n.ends_with(e))
}

/// The command that packs `names` (in the folder) into `archive`.
pub fn compress_command(archive: &str, format: &str, names: &[String]) -> Vec<String> {
    let mut argv: Vec<String> = match format {
        "zip" => vec!["zip".into(), "-r".into(), "-q".into(), archive.into()],
        "7z" => vec!["7z".into(), "a".into(), "-bd".into(), archive.into()],
        // tar picks the compression from the name.
        _ => vec!["tar".into(), "-caf".into(), archive.into()],
    };
    argv.push("--".into());
    if format == "7z" {
        argv.pop();
    }
    argv.extend(names.iter().cloned());
    argv
}

/// The command that unpacks `archive` into `into`.
pub fn extract_command(archive: &Path, into: &Path) -> Vec<String> {
    let a = archive.display().to_string();
    let d = into.display().to_string();
    let n = a.to_lowercase();
    if n.ends_with(".zip") {
        vec!["unzip".into(), "-q".into(), a, "-d".into(), d]
    } else if n.ends_with(".7z") {
        vec!["7z".into(), "x".into(), "-bd".into(), format!("-o{d}"), a]
    } else if n.ends_with(".rar") {
        vec!["unrar".into(), "x".into(), "-idq".into(), a, format!("{d}/")]
    } else if n.contains(".tar") || n.ends_with(".tgz") || n.ends_with(".txz") || n.ends_with(".tbz2") {
        vec!["tar".into(), "-xaf".into(), a, "-C".into(), d]
    } else {
        // A single compressed file: 7z knows them all.
        vec!["7z".into(), "x".into(), "-bd".into(), format!("-o{d}"), a]
    }
}

/// Where `archive` is extracted: a folder of its name beside it.
pub fn extract_folder(archive: &Path) -> PathBuf {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let lower = name.to_lowercase();
    let stem = [".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst", ".tgz", ".txz", ".tbz2", ".zip", ".7z", ".rar", ".tar", ".gz", ".xz", ".bz2", ".zst"]
        .iter()
        .find(|e| lower.ends_with(*e))
        .map_or(name.as_str(), |e| &name[..name.len() - e.len()]);
    let folder = archive.with_file_name(stem);
    if folder.symlink_metadata().is_ok() {
        crate::ops::unique(&folder)
    } else {
        folder
    }
}

/// The compress dialog: a name and a format.
pub fn compress_dialog(parent: &gtk::Window, dir: PathBuf, names: Vec<String>, jobs: std::rc::Rc<crate::ops::Jobs>) {
    let default = if names.len() == 1 {
        names[0].clone()
    } else {
        dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| tr("files-archive"))
    };
    let entry = gtk::Entry::builder().text(&default).width_chars(30).hexpand(true).build();
    let format = gtk::DropDown::from_strings(&FORMATS.iter().map(|(f, _)| *f).collect::<Vec<_>>());
    let go = gtk::Button::builder().label(tr("files-compress")).css_classes(["suggested-action"]).build();
    let row = gtk::Box::builder().spacing(10).build();
    row.append(&entry);
    row.append(&format);
    row.append(&go);
    let window = gtk::Window::builder()
        .transient_for(parent)
        .css_classes(["orbit-files-dialog"])
        .modal(true)
        .title(tr("files-compress"))
        .resizable(false)
        .child(&crate::actions::padded("dialog-box", &row))
        .build();
    let run = {
        let window = window.clone();
        let entry = entry.clone();
        move || {
            let (fmt, ext) = FORMATS[format.selected() as usize];
            let name = format!("{}{ext}", entry.text());
            let what = tr_args("files-job-compress", &[("name", text(name.clone()))]);
            jobs.command(what, compress_command(&name, fmt, &names), dir.clone());
            window.close();
        }
    };
    let run = std::rc::Rc::new(run);
    {
        let r = run.clone();
        go.connect_clicked(move |_| r());
    }
    entry.connect_activate(move |_| run());
    window.present();
    entry.grab_focus();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archives_by_name() {
        assert!(is_archive("a.tar.zst"));
        assert!(is_archive("B.ZIP"));
        assert!(!is_archive("photo.jpg"));
        assert_eq!(compress_command("x.tar.xz", "tar.xz", &["a".into()]), ["tar", "-caf", "x.tar.xz", "--", "a"]);
        assert_eq!(compress_command("x.7z", "7z", &["a".into()]), ["7z", "a", "-bd", "x.7z", "a"]);
    }

    #[test]
    fn extracts_into_a_folder_of_its_name() {
        assert_eq!(extract_folder(Path::new("/nowhere/fotos.tar.gz")), PathBuf::from("/nowhere/fotos"));
        assert_eq!(extract_command(Path::new("/x/a.zip"), Path::new("/x/a")), ["unzip", "-q", "/x/a.zip", "-d", "/x/a"]);
    }
}
