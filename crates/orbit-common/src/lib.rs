//! Shared building blocks for the Orbit programs: configuration, CSS styling,
//! the display-server abstraction implemented by the X11 and Wayland
//! backends, and a sway IPC client.

pub mod backend;
pub mod config;
pub mod natural;
pub mod style;
pub mod sway;

pub use backend::{Backend, Placement};
pub use config::{ConfigDir, Edge};
