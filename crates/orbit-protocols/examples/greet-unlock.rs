//! A stand-in greet, for testing the core's side of `orbit_greet_v1`: it
//! unlocks SECONDS after each lock, with no password and no window.
//!
//!   core.yaml: greet: /path/to/greet-unlock 3
//!
//! Run from an ordinary client it finds no global and says so: only the
//! core's own greet sees it.

use std::time::Duration;

use orbit_protocols::client::orbit_greet_v1::{self, OrbitGreetV1};
use wayland_client::{Connection, Dispatch, QueueHandle, globals::{GlobalListContents, registry_queue_init}, protocol::wl_registry};

struct State {
    locked: bool,
}

fn main() {
    let seconds: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let conn = Connection::connect_to_env().expect("a Wayland connection");
    let (globals, mut queue) = registry_queue_init::<State>(&conn).expect("the registry");
    let qh = queue.handle();
    let greet: OrbitGreetV1 = match globals.bind(&qh, 1..=1, ()) {
        Ok(g) => g,
        Err(err) => {
            eprintln!("greet-unlock: no orbit_greet_v1 ({err}): not the core's greet");
            std::process::exit(1);
        }
    };
    let mut state = State { locked: false };
    loop {
        queue.blocking_dispatch(&mut state).expect("the connection");
        if state.locked {
            eprintln!("greet-unlock: locked; unlocking in {seconds} s");
            std::thread::sleep(Duration::from_secs(seconds));
            greet.unlock();
            state.locked = false;
            conn.flush().expect("the connection");
        }
    }
}

impl Dispatch<OrbitGreetV1, ()> for State {
    fn event(state: &mut Self, _: &OrbitGreetV1, event: orbit_greet_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            orbit_greet_v1::Event::Locked => state.locked = true,
            orbit_greet_v1::Event::Unlocked => eprintln!("greet-unlock: unlocked"),
            _ => {}
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}
