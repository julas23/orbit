# Worklog

What was done, tried and learnt, by component, newest first. What is still
open lives in `docs/pending.md`.

## Orbit-Aigent — 2026-10-22/23

The voice layer for coding agents (pending §13), from nothing to working
end to end with Claude Code, by the user's voice, in one session.

### What was built, in order

| Commit | What |
|---|---|
| e88760f | Daemon (microphone, voice detection, states, socket, demo) and the panel's sphere, Full 4 × 3 and Mini 2 × 1; CAVA's analysis moved to the new `orbit-audio` crate. |
| 6e1a3f3 | Whisper (whisper-rs, bundled bindings: no libclang needed): wake word and request, pt/en. |
| 79f3879 | The name cut off at the pause after it, found by loudness. |
| e7969ea | Whisper on the iGPU (Vulkan): small for the wake word, large-v3-turbo for the request. |
| 73a91c0 | Claude Code answers, Piper says it, changes asked aloud (MCP permission prompt). |
| 092df4a | Hints for short requests ("que horas são", not "que oração"). |
| ac349b9 | The answer heard over the question; shorter questions; more look-only commands. |
| 262dd30 | Core: `move … to output` (the agent needed it). |
| ebc5db9 | Questions say what a desktop command does; a yes holds for the request. |
| 763d644 | Short yes/no heard right; chained commands asked about whole. |
| 4914f03 | Core: the lock state sent on subscribing. |
| 5a720df | Started by `orbit-session`; deaf while locked. |
| e606e80 | An answer cut short by the wake word ("Jarvis, para"). |
| 4b46e7d | Echo cancelling for speakers (PipeWire, WebRTC) and its own voice ignored. |

### Measurements (hp: Ryzen 7 5825U, Radeon Barcelo iGPU)

- Whisper on the CPU, 8 threads: small 1.4 s with a 10 s encoder context
  (5–7 s with the default 30 s); medium 4–11 s and no better.
- Whisper on the iGPU: small 0.4 s, large-v3-turbo about 1.7–2 s.
- From the end of speech to the answer spoken: about 5–8 s, most of it
  Claude Code (a request with one command ~6 s).
- Piper, kept running: a sentence in 0.2–0.3 s.
- Idle: 0.5% CPU, ~130 MB RAM (the models sit in the iGPU's memory).

### Lessons

- The default microphone was the Bluetooth headset's: it records as a
  phone call (Whisper hears it badly) and, open, drags the headset into
  call mode for everything. Use the USB one (K66), now through the echo
  canceller (`orbit-aigent-mic`).
- The K66 is quiet (speech peaks at −20 to −30 dB): the audio is raised
  before Whisper, and the voice detector asks for 18 dB over the room.
- Whisper's prompt is double-edged: "Ok Jarvis. Ok Orbit." gets the name
  spelt right but leans to English and, written bare, makes it drop the
  name; so it is used only to find the name. Requests get common requests
  as hints; answers get "Sim. Não. Pode. Yes. No.".
- Language detection on a few words is shaky, and the English name pulls
  it to English: the name is cut off before hearing the request, and
  Portuguese wins a near tie, unless both hearings are the same words.
- Whisper's token timestamps are useless with noise (every word at the
  end): the name is cut by loudness instead.
- whisper-rs's own Vulkan feature needs libclang for its bindings; turning
  on whisper-rs-sys's feature alone builds whisper.cpp with Vulkan and the
  bundled bindings do.
- Two "nothing heard" reports were the voice not reaching the K66; the log
  now says the room's level and the loudest sound every 30 s at debug.

### Not tried yet

Speakers (echo cancelling), the lock with the new core, Gemini CLI,
OpenCode and Kimi Code.

## Orbit-Remote-Server — 2026-10-23

The Julas VM became the server: sway, rofi and Sunshine removed, the whole
Orbit stack from the local apt repo, its own home on its disk (the shared
one was the hp's, mirrored, and brought the hp's binaries and SSH keys
along), the session open at boot for remote tests, the TV at 1920×1080@60
(`outputs` mode in core.yaml). The browser on the hp is the client.

- The core took QEMU's VGA card: Smithay gives a card without a render
  node one of another card; the GPU is now one whose render node sysfs
  shows.
- NVIDIA's GBM refuses the LINEAR flag; the linear modifier works.
- nvh264enc was missing from a stale GStreamer registry cache.
- NVENC's first frame takes over a second (its CUDA context).
- Measured: 58 fps at 1080p, 5.0 ms median and 5.6 ms at 95% from the copy
  to H.264 (VA-API on the hp, nested: 7 ms).
- Remote mouse and keys: the core speaks wlr-virtual-pointer and
  virtual-keyboard (Smithay's), the server makes a pointer over the shared
  monitor and a keyboard with the seat's keymap; the page sends moves as
  fractions of the picture and keys by their place. Tested from the hp:
  works.
- The deb build's metapackages name no versions: upgrading means
  `apt-get install --only-upgrade` on every installed `orbit-*`.

### Not tried yet

The latency (the clock photo), the server started with the session, a
client other than Chrome on the hp. Paused here to go back to orbit-core.

## The night of 2026-10-23: the pending list, alone

The user went to sleep and asked for the list to be worked through on the
hp, nested, asking nothing; what needs the user, the hardware or a decision
was left for the morning. The think and the Julas VM were switched off.

- Remote: the core gives the virtual pointer and keyboard only to
  orbit-remote-server (the program at the socket's other end) and keeps a
  remote keyboard's Ctrl+Alt+Backspace and Ctrl+Alt+Fn; no pairing by code
  while the session is locked, the code shown once it is unlocked; the
  session's sound in Opus beside the video; the first frame sent again as a
  key frame once WebRTC connects (a still screen showed nothing).
- A regression found on the way: since NVENC's fix the capture passed GBM's
  implicit modifier on, and VA-API refused it; on AMD and Intel the video
  never negotiated. A buffer made linear by the flag says it is linear.
- Phase 2: virtual monitors in the core (`output NAME create`), placed to
  the right of the local ones; `serve --virtual` gives the client one of
  its size. Monitors turned (`transform:`), scaled (`scale:`, fractional
  scale), set by wlr-randr (wlr-output-management, from niri), shared by
  the portal (`orbit-portals.conf`: the Orbit session had no ScreenCast).
- Phase 3: layouts per workspace (columns, master, monocle, bsp), master's
  ratio and count, sway's named scratchpads.
- Aigent: a chime when woken, Mod+A, the echo canceller in its package.
- Distro: orbit-defaults, orbit-fonts, orbit-ruffle; CI building the debs
  in a Debian 13 container (tried the same way in docker); a clean Debian 13
  resolves `orbit-desktop orbit-studio`.
- The microphone from the station (branch `wip/station-mic`): the server's
  half works (a PipeWire loopback whose source is the session's
  microphone), the station's webrtcbin does not associate its sending
  transceiver with the server's line.

### Lessons

- A nested core ran the default `startup:`: `dbus-update-activation-environment`
  handed the nested display to the user's systemd (the user's portals then
  failed) and `orbit-session` killed the user's panel and dock by name. A
  nested core now runs neither unless `--startup` asks.
- When the user's session locks, a nested window gets no frames and stops
  drawing: screenshots of it are empty. Virtual monitors have their own
  clock; tests draw there.
- `pkill -f` with a pattern in the command itself kills the shell running it.

### Not tried yet (for the morning)

The station for real (sound, the virtual monitor at its size), a browser
with sound, a turned or scaled real monitor, a DRM mode change from
wlr-randr, a real screen share (Meet, OBS), the chime heard.

## The morning of 2026-10-24: the night's work on the real session

The user booted the hp on the night's packages and tried them:

- The boot: everything in place and working.
- The layouts and their shortcuts: right.
- Mod+A: listening and cancelling work, the chime heard.
- A real screen share through the portal: works.

## The hp on the packages — 2026-10-24

Phase A's last part: the hp now runs Orbit from the local repository, as
the think does. Installed orbit-desktop, orbit-remote-server,
orbit-local-station, orbit-aigent and ArtCraft's 14 packages (one by one,
not orbit-studio, which pulls LibreOffice back); LibreOffice purged. What
`install.sh` had put is gone: `/usr/local/bin/orbit-*`, the
`/etc/systemd/system/orbit-login.service` copy (the package's, in
`/usr/lib`, is the one enabled now), `/usr/local/share/orbit-pathfinder`,
and in the home `~/.local/bin/orbit-*`, the menu entries, the portal and
echo canceller configs and the recipes. `/etc/pam.d/orbit-*` and
`/etc/orbit/login.yaml` were the same as the packages'. The PATH trick in
`~/.shell/10-path.sh` now holds on the hp too, so a later `install.sh`
cannot shadow the packages. Not packaged yet: the keyring prompter's D-Bus
service, left in `~/.local/share/dbus-1/services/`.
