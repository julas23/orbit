//! Orbit-Clip: a clipboard history for Wayland (and XWayland, which the
//! compositor bridges).
//!
//! Runs in the background — start it with `--background` when the session
//! starts — keeping what is copied, text and images, in a history that
//! survives restarts. Launching it again (bind it to a key, Super+V) shows
//! or hides the history; choosing an item puts it back in the clipboard.
//! Optionally keeps the clipboard and the primary selection in sync, and
//! leaves out what password managers mark as secret.
//!
//! Options in `~/.config/orbit/clip.yaml`; the history in
//! `~/.local/share/orbit/clip/`.

mod history;
mod ui;
mod wayland;

use gtk::glib;

fn main() -> glib::ExitCode {
    orbit_app::run_app(orbit_app::App {
        name: "orbit-clip",
        app_id: "io.github.julas23.Orbit.Clip",
        css: include_str!("../../../data/clip.css"),
        build: ui::build,
        resident: true,
        open: None,
    })
}
