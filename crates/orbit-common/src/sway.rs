//! The compositor's IPC (the i3/sway protocol): commands and queries, for
//! the panel and the applications that need to know about outputs and
//! windows. Talks to Orbit-Core (`$ORBIT_SOCK`) or sway (`$SWAYSOCK`); the
//! protocol itself is in `orbit-ipc`.

pub use orbit_ipc::{Connection, GET_INPUTS, GET_OUTPUTS, GET_TREE, GET_WORKSPACES, RUN_COMMAND, SUBSCRIBE, available};
