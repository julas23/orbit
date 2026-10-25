//! Orbit-Safe: the passwords in the keyring, and one-time codes, opened
//! only after authenticating.
//!
//! The keyring is reached over the Secret Service API (gnome-keyring
//! today), so the safe shows what the apps keep there and is the Orbit's
//! place of Seahorse (it is also what PLANO-CORE called orbit-keys). Codes
//! (TOTP, as Google Authenticator makes them) are items of the safe's own
//! schema, the key as their secret.
//!
//! Opening asks what `/etc/pam.d/orbit-safe` asks: the password by default;
//! a fingerprint, the face or a code from the phone when their modules are
//! added there.
//!
//! Options in `~/.config/orbit/safe.yaml`.

mod pam;
mod secrets;
mod totp;
mod ui;

use gtk::glib;

fn main() -> glib::ExitCode {
    orbit_app::run_app(orbit_app::App {
        name: "orbit-safe",
        app_id: "io.github.julas23.Orbit.Safe",
        css: include_str!("../../../data/safe.css"),
        build: ui::build,
        resident: false,
        open: None,
    })
}
