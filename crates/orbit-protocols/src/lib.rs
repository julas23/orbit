//! Orbit's private Wayland protocols, generated from `protocols/*.xml`:
//! `orbit_greet_v1`, the greet's side of the core's lock (PLANO-CORE §7.3
//! step 3). Feature `server` for orbit-core, `client` for the greet.

#[cfg(feature = "server")]
pub mod server {
    use wayland_server;

    pub mod __interfaces {
        use wayland_server::backend as wayland_backend;
        wayland_scanner::generate_interfaces!("protocols/orbit-greet-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("protocols/orbit-greet-v1.xml");
}

#[cfg(feature = "client")]
pub mod client {
    use wayland_client;

    pub mod __interfaces {
        use wayland_client::backend as wayland_backend;
        wayland_scanner::generate_interfaces!("protocols/orbit-greet-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/orbit-greet-v1.xml");
}
