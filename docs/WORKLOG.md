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
