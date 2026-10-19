//! Shared building blocks for Orbit-Panel and Orbit-Dock: configuration, CSS styling and the
//! display-server abstraction implemented by the X11 and Wayland backends.

pub mod backend;
pub mod config;
pub mod style;

pub use backend::{Backend, Placement};
pub use config::{ConfigDir, Edge};
