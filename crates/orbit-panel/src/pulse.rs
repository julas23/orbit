//! Minimal PulseAudio client for the default sink's volume and mute, over
//! the native protocol in pure Rust (the `pulseaudio` crate) — PipeWire
//! serves it through pipewire-pulse. No libpulse, no pactl/pamixer processes.
//!
//! Queries and commands run on the GTK thread on one connection; the event
//! subscription runs on its own thread and wakes the GTK thread through a
//! channel, like the sway client.
//!
//! Percentages follow pamixer and pavucontrol: 100% is the server's "norm"
//! volume and the scale is linear in the raw value, not in loudness.

use std::{
    ffi::CString,
    io::{self, BufReader},
    os::unix::net::UnixStream,
    time::Duration,
};

use pulseaudio::protocol::{self, ChannelVolume, Command, Volume};

/// The raw value of 100% (`PA_VOLUME_NORM`).
const NORM: u32 = 0x1_0000;

/// The default sink, as the panel needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sink {
    pub index: u32,
    pub percent: u32,
    pub muted: bool,
    channels: u8,
}

pub struct Connection {
    sock: BufReader<UnixStream>,
    version: u16,
    seq: u32,
}

fn other(err: impl std::fmt::Display) -> io::Error {
    io::Error::other(err.to_string())
}

impl Connection {
    pub fn open() -> io::Result<Self> {
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
        protocol::write_command_message(
            sock.get_mut(),
            0,
            &Command::Auth(auth),
            protocol::MAX_VERSION,
        )
        .map_err(other)?;
        let (_, reply) =
            protocol::read_reply_message::<protocol::AuthReply>(&mut sock, protocol::MAX_VERSION)
                .map_err(other)?;
        let version = protocol::MAX_VERSION.min(reply.version);

        let mut props = protocol::Props::new();
        props.set(
            protocol::Prop::ApplicationName,
            CString::new("orbit-panel").expect("no NUL"),
        );
        protocol::write_command_message(sock.get_mut(), 1, &Command::SetClientName(props), version)
            .map_err(other)?;
        protocol::read_reply_message::<protocol::SetClientNameReply>(&mut sock, version)
            .map_err(other)?;
        Ok(Self {
            sock,
            version,
            seq: 2,
        })
    }

    fn send(&mut self, command: &Command) -> io::Result<u32> {
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        protocol::write_command_message(self.sock.get_mut(), seq, command, self.version)
            .map_err(other)?;
        Ok(seq)
    }

    /// The current default sink.
    pub fn default_sink(&mut self) -> io::Result<Sink> {
        self.send(&Command::GetServerInfo)?;
        let (_, server) =
            protocol::read_reply_message::<protocol::ServerInfo>(&mut self.sock, self.version)
                .map_err(other)?;
        let name = server
            .default_sink_name
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no default sink"))?;

        self.send(&Command::GetSinkInfo(protocol::GetSinkInfo {
            index: None,
            name: Some(name),
        }))?;
        let (_, info) =
            protocol::read_reply_message::<protocol::SinkInfo>(&mut self.sock, self.version)
                .map_err(other)?;
        let volumes = info.cvolume.channels();
        let average = volumes.iter().map(|v| u64::from(v.as_u32())).sum::<u64>()
            / volumes.len().max(1) as u64;
        Ok(Sink {
            index: info.index,
            percent: ((average * 100 + u64::from(NORM) / 2) / u64::from(NORM)) as u32,
            muted: info.muted,
            channels: volumes.len().max(1) as u8,
        })
    }

    /// Sets every channel of `sink` to `percent`.
    pub fn set_volume(&mut self, sink: &Sink, percent: u32) -> io::Result<()> {
        let raw = (u64::from(percent) * u64::from(NORM) / 100) as u32;
        let mut volume = ChannelVolume::empty();
        for _ in 0..sink.channels {
            volume.push(Volume::from_u32_clamped(raw));
        }
        self.send(&Command::SetSinkVolume(protocol::SetDeviceVolumeParams {
            device_index: Some(sink.index),
            device_name: None,
            volume,
        }))?;
        protocol::read_ack_message(&mut self.sock).map_err(other)?;
        Ok(())
    }

    pub fn set_mute(&mut self, sink: &Sink, mute: bool) -> io::Result<()> {
        self.send(&Command::SetSinkMute(protocol::SetDeviceMuteParams {
            device_index: Some(sink.index),
            device_name: None,
            mute,
        }))?;
        protocol::read_ack_message(&mut self.sock).map_err(other)?;
        Ok(())
    }
}

/// Calls `notify` on the GTK thread after any sink or server change (volume,
/// mute, default sink). A burst of events results in one call. Reconnects if
/// the sound server restarts.
pub fn subscribe(notify: impl Fn() + 'static) {
    let (tx, rx) = async_channel::unbounded::<()>();
    std::thread::spawn(move || {
        loop {
            let run = || -> io::Result<()> {
                let mut conn = Connection::open()?;
                let mask = protocol::SubscriptionMask::SINK | protocol::SubscriptionMask::SERVER;
                conn.send(&Command::Subscribe(mask))?;
                protocol::read_ack_message(&mut conn.sock).map_err(other)?;
                // The server came (back): let the panel re-read the state.
                if tx.send_blocking(()).is_err() {
                    return Ok(());
                }
                loop {
                    protocol::read_command_message(&mut conn.sock, conn.version).map_err(other)?;
                    if tx.send_blocking(()).is_err() {
                        return Ok(()); // the GTK side is gone
                    }
                }
            };
            match run() {
                Ok(()) => return,
                Err(err) => {
                    log::warn!("pulse: subscription lost ({err}), retrying");
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        }
    });
    gtk::glib::spawn_future_local(async move {
        while rx.recv().await.is_ok() {
            while rx.try_recv().is_ok() {}
            notify();
        }
    });
}

/// Why `record_monitor` returned.
pub enum Stop {
    /// `on_samples` asked to stop: nobody wants the sound any more.
    Done,
    /// The default sink changed or the stream was taken away: record again
    /// to follow it.
    Again,
}

/// Records what the default sink plays (its monitor source), mono, 32-bit
/// float at `rate` Hz, in blocks of about `block` samples, and hands each
/// block to `on_samples` until it returns `false`. Blocks the calling
/// thread.
pub fn record_monitor(rate: u32, block: u32, mut on_samples: impl FnMut(&[f32]) -> bool) -> io::Result<Stop> {
    use std::io::Read;

    let mut conn = Connection::open()?;
    // Told when the default sink changes, to follow it.
    conn.send(&Command::Subscribe(protocol::SubscriptionMask::SERVER))?;
    protocol::read_ack_message(&mut conn.sock).map_err(other)?;

    conn.send(&Command::CreateRecordStream(protocol::RecordStreamParams {
        source_name: Some(CString::new("@DEFAULT_MONITOR@").expect("no NUL")),
        sample_spec: protocol::SampleSpec {
            format: protocol::SampleFormat::Float32Le,
            channels: 1,
            sample_rate: rate,
        },
        channel_map: protocol::ChannelMap::mono(),
        buffer_attr: protocol::stream::BufferAttr {
            fragment_size: block * 4,
            ..Default::default()
        },
        flags: protocol::stream::StreamFlags {
            adjust_latency: true,
            // Listening must not keep the sound card awake.
            no_inhibit_auto_suspend: true,
            ..Default::default()
        },
        props: {
            let mut props = protocol::Props::new();
            props.set(protocol::Prop::MediaName, CString::new("spectrum").expect("no NUL"));
            props
        },
        ..Default::default()
    }))?;
    let (_, stream) =
        protocol::read_reply_message::<protocol::CreateRecordStreamReply>(&mut conn.sock, conn.version)
            .map_err(other)?;
    log::debug!(
        "pulse: recording in blocks of {} bytes, {} µs behind",
        stream.buffer_attr.fragment_size,
        stream.stream_latency
    );

    let mut bytes = Vec::new();
    let mut samples = Vec::new();
    loop {
        let desc = protocol::read_descriptor(&mut conn.sock).map_err(other)?;
        if desc.channel == u32::MAX {
            let (_, command) =
                Command::read_tag_prefixed(&mut conn.sock, conn.version).map_err(other)?;
            match command {
                Command::SubscribeEvent(_) => return Ok(Stop::Again),
                Command::RecordStreamKilled(index) if index == stream.channel => return Ok(Stop::Again),
                _ => {}
            }
            continue;
        }
        bytes.resize(desc.length as usize, 0);
        conn.sock.read_exact(&mut bytes)?;
        if desc.channel != stream.channel {
            continue;
        }
        samples.clear();
        samples.extend(bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
        if !on_samples(&samples) {
            return Ok(Stop::Done);
        }
    }
}
