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
//! (`input`) and the client's gamepads (`pads`).

mod audio;
mod capture;
mod encode;
mod input;
mod lanes;
mod pads;
mod serve;

/// The port of the page and its WebSocket.
const PORT: u16 = 7311;

fn main() {
    // NVENC's GL steps want a GL context, and GStreamer makes its window
    // on the session's Wayland: an empty one the core tiles. A context
    // with no surface does the same work and shows nothing.
    if std::env::var_os("GST_GL_WINDOW").is_none() {
        // Safety: nothing else runs yet, so nothing reads the environment.
        unsafe { std::env::set_var("GST_GL_WINDOW", "surfaceless") };
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["capture"] => capture::measure(None),
        ["capture", output] => capture::measure(Some(output)),
        ["encode"] => encode::measure(None, None),
        ["encode", output] => encode::measure(Some(output), None),
        ["encode", output, size] => match size.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))) {
            Some(size) => encode::measure(Some(output), Some(size)),
            None => Err(anyhow::anyhow!("a size as 800x600, not {size}")),
        },
        [] | ["serve"] => serve::serve(PORT, serve::Screen::Mirror(None)),
        ["serve", "--virtual"] => serve::serve(PORT, serve::Screen::Virtual),
        ["serve", output] => serve::serve(PORT, serve::Screen::Mirror(Some(output.to_string()))),
        _ => {
            eprintln!("usage: orbit-remote-server [serve [--virtual]|capture|encode] [OUTPUT] [WxH: encode, the client's screen]");
            std::process::exit(2);
        }
    };
    if let Err(err) = result {
        log::error!("remote: {err:#}");
        std::process::exit(1);
    }
}
