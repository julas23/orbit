//! Orbit-Remote-Server: the session, shared over the network.
//!
//!   orbit-remote-server capture [OUTPUT]   measure the capture for 10 s
//!   orbit-remote-server encode [OUTPUT]    and the encoding
//!
//! A sharing layer of orbit-core, not a second desktop (pending §9): it
//! takes the monitor's frames as they change (`capture`), encodes them on
//! the GPU (VA-API) and sends them over WebRTC to a browser or to
//! orbit-local-station, and takes the remote keyboard and mouse back.

mod capture;
mod encode;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["capture"] => capture::measure(None),
        ["capture", output] => capture::measure(Some(output)),
        ["encode"] => encode::measure(None),
        ["encode", output] => encode::measure(Some(output)),
        _ => {
            eprintln!("usage: orbit-remote-server capture|encode [OUTPUT]");
            std::process::exit(2);
        }
    };
    if let Err(err) = result {
        log::error!("remote: {err:#}");
        std::process::exit(1);
    }
}
