//! The microphone: the default source, recorded over PulseAudio's native
//! protocol (served by pipewire-pulse), mono 32-bit float at 16 kHz — what
//! Whisper takes — in blocks of 30 ms.

use std::{
    ffi::CString,
    io::{self, BufReader, Read},
    os::unix::net::UnixStream,
    time::Duration,
};

use pulseaudio::protocol::{self, Command};

/// Samples a second.
pub const RATE: u32 = 16_000;
/// Samples a block: 30 ms.
pub const BLOCK: usize = 480;

fn other(err: impl std::fmt::Display) -> io::Error {
    io::Error::other(err.to_string())
}

/// Records for good on the calling thread, handing each block to
/// `on_block` until it returns `false`; records again when the default
/// source changes or the sound server goes away.
pub fn record(mut on_block: impl FnMut(&[f32]) -> bool) {
    loop {
        match record_once(&mut on_block) {
            Ok(true) => return,
            Ok(false) => log::info!("aigent: the default source changed, recording again"),
            Err(err) => {
                log::warn!("aigent: recording: {err}; again in 2 s");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

/// `Ok(true)` when `on_block` asked to stop, `Ok(false)` to record again.
fn record_once(on_block: &mut impl FnMut(&[f32]) -> bool) -> io::Result<bool> {
    let path = pulseaudio::socket_path_from_env()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no PulseAudio socket"))?;
    let mut sock = BufReader::new(UnixStream::connect(path)?);
    let cookie = pulseaudio::cookie_path_from_env()
        .and_then(|path| std::fs::read(path).ok())
        .unwrap_or_default();
    let auth = protocol::AuthParams {
        version: protocol::MAX_VERSION,
        supports_shm: false,
        supports_memfd: false,
        cookie,
    };
    protocol::write_command_message(sock.get_mut(), 0, &Command::Auth(auth), protocol::MAX_VERSION)
        .map_err(other)?;
    let (_, reply) =
        protocol::read_reply_message::<protocol::AuthReply>(&mut sock, protocol::MAX_VERSION).map_err(other)?;
    let version = protocol::MAX_VERSION.min(reply.version);

    let mut props = protocol::Props::new();
    props.set(protocol::Prop::ApplicationName, CString::new("orbit-aigent").expect("no NUL"));
    protocol::write_command_message(sock.get_mut(), 1, &Command::SetClientName(props), version).map_err(other)?;
    protocol::read_reply_message::<protocol::SetClientNameReply>(&mut sock, version).map_err(other)?;

    // Told when the default source changes, to follow it.
    protocol::write_command_message(
        sock.get_mut(),
        2,
        &Command::Subscribe(protocol::SubscriptionMask::SERVER),
        version,
    )
    .map_err(other)?;
    protocol::read_ack_message(&mut sock).map_err(other)?;

    let params = protocol::RecordStreamParams {
        source_name: Some(CString::new("@DEFAULT_SOURCE@").expect("no NUL")),
        sample_spec: protocol::SampleSpec {
            format: protocol::SampleFormat::Float32Le,
            channels: 1,
            sample_rate: RATE,
        },
        channel_map: protocol::ChannelMap::mono(),
        buffer_attr: protocol::stream::BufferAttr {
            fragment_size: BLOCK as u32 * 4,
            ..Default::default()
        },
        flags: protocol::stream::StreamFlags {
            adjust_latency: true,
            ..Default::default()
        },
        props: {
            let mut props = protocol::Props::new();
            props.set(protocol::Prop::MediaName, CString::new("wake word").expect("no NUL"));
            props
        },
        ..Default::default()
    };
    protocol::write_command_message(sock.get_mut(), 3, &Command::CreateRecordStream(params), version)
        .map_err(other)?;
    let (_, stream) =
        protocol::read_reply_message::<protocol::CreateRecordStreamReply>(&mut sock, version).map_err(other)?;
    log::info!("aigent: recording the microphone, {} µs behind", stream.stream_latency);

    // The server's fragments are cut into blocks of exactly `BLOCK`.
    let mut bytes = Vec::new();
    let mut pending: Vec<f32> = Vec::with_capacity(2 * BLOCK);
    loop {
        let desc = protocol::read_descriptor(&mut sock).map_err(other)?;
        if desc.channel == u32::MAX {
            let (_, command) = Command::read_tag_prefixed(&mut sock, version).map_err(other)?;
            match command {
                Command::SubscribeEvent(_) => return Ok(false),
                Command::RecordStreamKilled(index) if index == stream.channel => return Ok(false),
                _ => {}
            }
            continue;
        }
        bytes.resize(desc.length as usize, 0);
        sock.read_exact(&mut bytes)?;
        if desc.channel != stream.channel {
            continue;
        }
        pending.extend(bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
        let mut start = 0;
        while pending.len() - start >= BLOCK {
            if !on_block(&pending[start..start + BLOCK]) {
                return Ok(true);
            }
            start += BLOCK;
        }
        pending.drain(..start);
    }
}
