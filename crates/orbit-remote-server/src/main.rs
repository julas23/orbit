//! Orbit-Remote-Server: the session, shared over the network.
//!
//!   orbit-remote-server capture [OUTPUT]   measure the capture for 10 s
//!   orbit-remote-server encode [OUTPUT]    and the encoding
//!   orbit-remote-server [serve] [OUTPUT]   share it with a browser (port 7311)
//!
//! A sharing layer of orbit-core, not a second desktop (pending §9): it
//! takes the monitor's frames as they change (`capture`), encodes them on
//! the GPU (VA-API or NVENC) and sends them over WebRTC to a browser or to
//! orbit-local-station, and takes the remote keyboard and mouse back
//! (`input`).

mod audio;
mod capture;
mod encode;
mod input;
mod serve;

/// The port of the page and its WebSocket.
const PORT: u16 = 7311;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["capture"] => capture::measure(None),
        ["capture", output] => capture::measure(Some(output)),
        ["encode"] => encode::measure(None),
        ["encode", output] => encode::measure(Some(output)),
        [] | ["serve"] => serve::serve(PORT, None),
        ["serve", output] => serve::serve(PORT, Some(output.to_string())),
        _ => {
            eprintln!("usage: orbit-remote-server [serve|capture|encode] [OUTPUT]");
            std::process::exit(2);
        }
    };
    if let Err(err) = result {
        log::error!("remote: {err:#}");
        std::process::exit(1);
    }
}
