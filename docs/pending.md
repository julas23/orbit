# Pending

What is still open across Orbit, in the order we plan to work on it.
Details live in `docs/PLANO-CORE.md` (§7.3, §8, §10) and `docs/PLANO-PAINEL.md`.

## 1. Done 2026-10-22

- [x] Core: a minimised window goes back to the workspace it left (`minimized_from`).
- [x] Core + `orbit-ipc`: the `lock` IPC event (`{"change": "locked" | "unlocked"}`).
- [x] Panel: `sway::listen` (events with their body), plus the changes in `weather_data.rs` and `windows.rs`.
- [x] Core bug: the pointer looked different over Ghostty (GTK's Adwaita, not the core's Breeze_Light) and clicked beside the arrow (a hotspot left behind when GTK moved its image with `wl_surface.offset`). Fixed with `cursor-shape-v1` (shapes drawn from the core's theme) and the hotspot following the offset; verified on screen 2026-10-22.
- [x] Core: shortcuts in `~/.config/orbit/keys.yaml` (`keys.rs`; combination → IPC command, `none` removes, `$terminal`/`$launcher`/`$run`), plus `kill --force`. More shortcuts to be listed by the user.
- [x] New `orbit-run` (Mod+R): type a command and run it; Tab completes programs then files, Up/Down history, Esc closes.
- [x] Test, then commit.

## 2. Real tests not done yet

- [x] Two-user switch (§7.3 step 10): verified 2026-10-22, including the other user logging out (cc91c07).
- [x] Suspend and resume (`sleep.rs`).
- [x] polkit: wrong password and cancel.
- [x] Keyring prompter (verified 2026-10-22): copy `data/dbus/org.gnome.keyring.SystemPrompter.service` to `~/.local/share/dbus-1/services/` and test with a real app.
- [x] Dock: drop an item off the dock under orbit-core's pointer grab, and type in the app search.

## 3. Login and biometrics (§7.3 step 11)

- [ ] Set up biometrics in Orbit: fingerprint (fprintd) and face, the same way.
- [ ] Review the face recognition script that today runs only in the greeter.
- [ ] `orbit-remote-server` pairing allowed only while the session is unlocked.
- [ ] Later: orbit-greet chooses between the full local desktop and the remote desktop (§9).

## 4. Core

- [x] Floating windows: dialogs (X and Wayland) float at their own size, centred over their parent; `floating enable|disable|toggle` (Mod+Shift+Space, Mod+Esc); Mod+drag leaves a floating window where it is dropped. Verified on screen 2026-10-22.
- [x] Arrow shortcuts: Mod+↑ maximize, Mod+↓ restore, Mod+←/→ snap to a half; Alt+arrows focus that way, Mod+Ctrl+arrows move that way (other monitor, then the neighbouring area of the 2×2 grid; nothing at its edge). Verified on screen 2026-10-22.
- [x] Switching: Ctrl+Alt+arrows walk the areas; Alt+Tab (last used first, while Alt is held) on the workspace, Alt+Shift+Tab on every workspace; Mod+Tab the window cards (Looking Glass style, in perspective, over the blurred screen). Verified on screen 2026-10-22.
- [x] Blur: under the panel's confirmation dialog (`blur_layers`, done by the core — no screencopy for grim), and the lock's blur now runs ahead of the black so it shows.
- [x] Mod+right-drag resize: a floating window by the corner nearest the pointer (a snapped one floats first); a tiled one moves the line to the neighbouring column (column weights); the clients' own frame edges too (xdg and X). Mod+Esc toggles floating, as Mod+Shift+Space. Verified on screen 2026-10-22.
- [x] Screen capture for clients (`wlr-screencopy`, `screencopy.rs`): Orbit-Blink and grim work on orbit-core; refused while locked. Nested-tested 2026-10-22; installed, awaiting a session restart.
- [x] Screencopy with damage: `copy_with_damage` waits until the monitor changed since the client's last copy and says what (a tracker per client, monitor and region, looked at after each frame drawn); nested-tested with orbit-remote-server's `capture`: 1.3 fps on a still screen (2026-10-23).
- [ ] Rounded corners on rotated monitors.
- [ ] Phase 2: hotplug, fractional scale, `wlr-output-management`, portal and screen sharing, VRR/tearing. Done when the hp gets a full day without going back to sway.
- [ ] Phase 3: layouts `bsp`, `master`, `monocle`, `scroll`, per-workspace layouts, named scratchpads. (Tags left out on 2026-10-22: maybe in a later major version.)
- [ ] Phase 4: animations, shadows, blur, gestures, overview, effects off on battery.
- [ ] Phase 5: `orbit-remote-server` (§9, phase 1 working 2026-10-23), Jarvis wallpaper, orbit-aigent integrated with the core, own monitor settings, `orbit-keys` (password app).

## 5. Panel

- [x] Uniform clicks for the old status modules (network, keyboard, battery, notifications): left the quick part, right the full one, the wheel a value (2026-10-22).

## 6. Apps

| App | Effort | Status | Notes |
|---|---|---|---|
| orbit-calc | low–medium | Done | |
| orbit-clip | low–medium | Done (alpha) | Small UI; Wayland clipboard protocol and sync with XWayland |
| orbit-view | medium | Done (alpha) | Image viewer: decode, zoom, browse the folder |
| orbit-play | medium–high | Done (alpha) | GStreamer, many formats |
| orbit-blink | low–medium | Done (alpha) | Screenshots: full screen, current window, rectangle |
| orbit-read | medium | Done (alpha) | TXT and MD are simple; PDF needs a rendering library |
| orbit-files | high | Done (alpha) | A file manager has many edge cases (Yazi as reference) |
| orbit-edit | high | Done (alpha) | A real text editor |
| orbit-aigent | high | Working (alpha) | Voice layer for talking to coding agents (Claude Code, Gemini CLI, OpenCode, Kimi Code): Whisper + Piper, latency; ships together with its panel widget (§13) |

Only the station pair (§9) is still to be started.

## 7. Orbit Linux (`orbit-linux.txt`, plan in `docs/PLANO-DISTRO.md`)

Decided on 2026-10-21: prebuilt `.deb` packages (nothing compiled at install),
own installer (Rust + GTK4, running in orbit-core), EFISTUB as UKI with the
`BOOTX64.EFI` fallback, offline ISO, Plymouth handing over to orbit-greet.
**An existing `/home` is never formatted.**

- [ ] Phase A: CI building and publishing the debs; the hp and the think running on the packages instead of `install.sh` (`apt install orbit-desktop`).
- [ ] Phase A: `orbit-common-data`, `orbit-defaults`, `orbit-ruffle`, `orbit-fonts` (Nerd Font).
- [ ] Phase B: `orbit-studio` with ArtCraft mirrored; Orbit egui theme. Done when a fresh Debian 13 VM becomes this desktop with `apt install orbit-desktop orbit-studio`.
- [ ] Phase C (after core Phase 2): `orbit-install` and the ISO — disk, hostname, user, password; "will format X / will keep Y" confirmation; existing `/home` reused with the owner's UID — a partition holding user data that is clearly the `/home` mount (partition, LVM, LUKS, btrfs `@home`) is kept as it is, never formatted; a single partition with `/` and `/home` together has everything but `/home` deleted and the install goes on in that same partition; "wipe everything and start from scratch" is always offered as an alternative; progress and log on screen; automated install test in a Proxmox VM.
- [ ] Phase D: site, docs, stable/testing channels, security policy, arm64 image, check the "Orbit Linux" name.
- [ ] Packages without an official `.deb` (Popsicle, Ghostty, AniCraft, craft-fonts): AppImage managed by Orbit, or own package.
- [x] Update `docs/PLANO-DISTRO.md` §9: the "orbit-greet doesn't exist yet" risk is gone (2026-10-22).

## 8. Orbit-Safe (`orbit-safe.txt`)

A place to keep passwords safely, reachable after authenticating. It is the
`orbit-keys` app of core Phase 5 (Secret Service, over gnome-keyring today).

- [ ] Fold `orbit-safe` and `orbit-keys` into one app and one name.
- [ ] MFA: TOTP codes (Google Authenticator style) stored and shown by the app, and/or required to open it.
- [ ] Biometrics to open it: fingerprint (fprintd) and face recognition, the same as the login (§3).

## 9. Remote edition: `orbit-local-station` + `orbit-remote-server` (`orbit-station.txt`)

A pair that works only together (LTSP-like), replacing the old Orbit-Station
and `orbit-remote` names:

- `orbit-remote-server`: a sharing layer of orbit-core, not a second Orbit.
  It runs as a separate process, a privileged client of the core (as the
  greet is), over a private protocol: the core hands it the frames (dmabuf,
  no copy) and takes the remote input back (libei). The session is the
  same one, GTK4 apps and all; it can be installed, turned on and off on
  its own, and a crash in it leaves the local session running.
- `orbit-local-station`: no local interface of its own; it pulls the greeter,
  and then the whole session, straight from `orbit-remote-server`.

Decided on 2026-10-22:

- **Rust + Slint** for the server's own small interface (pairing by QR or
  token, who is connected, on/off, quality) and for every client: the web
  page (WASM), `orbit-local-station` (Slint's LinuxKMS backend draws straight
  on the screen, no compositor) and, much later, Android and iOS. The
  desktop apps stay GTK4: what travels is the session, not the interface.
- **What the remote sees:** first a mirror of the screens (it proves the
  whole chain), then a virtual output at the client's own resolution in the
  same session, the local screens locked meanwhile. A separate session per
  station only when there are several.
- **Media:** GStreamer (`webrtcbin`, VA-API encoders, Opus), the control in
  Rust. Video H.264/HEVC/AV1 in low-latency mode over WebRTC (UDP; WebSocket
  only for signalling); decoded on the client's GPU (WebCodecs in the
  browser; VA-API, MediaCodec or VideoToolbox natively). Audio Opus both
  ways; keyboard on a reliable data channel, the mouse on an unreliable one;
  gamepads as a virtual device (uinput); microphone and webcam up to the
  server as PipeWire virtual nodes.

The think and the Julas VM are set aside for now: they are the test machines
for this pair.

**Status (2026-10-23, paused):** phase 1 working end to end on the Julas
VM, the browser on the hp as the client: the VM runs the whole Orbit stack
from the apt repo (b61d73e) on its RTX 3050 at 1920×1080@60; a browser
pairs with the code shown on its screen, gets the screen over WebRTC
(NVENC) and drives it with its mouse and keys. Not measured yet: the
latency. The day by day is in `docs/WORKLOG.md`.

Done:

- [x] Started 2026-10-23. Decided then: the phase-1 web client is a small HTML/JS page served by the server (Slint stays for `orbit-local-station`, the server's own interface and the phones); the ≤ 20 ms mark is for the native client, the browser's latency measured and noted; the first client is a browser (the hp's, in the end). Capture through wlr-screencopy with damage into a dmabuf first (works on sway too), the private protocol later.
- [x] Core: screencopy with damage (`copy_with_damage` waits for a change), `outputs` mode in `core.yaml`, the GPU chosen by its render node in sysfs (not QEMU's VGA), wlr-virtual-pointer and virtual-keyboard taken as real devices (2026-10-23).
- [x] Capture into GBM dmabufs on the render node, four in turn; linear by the flag, else the linear modifier (NVIDIA), else the driver's layout (2026-10-23).
- [x] Encoding on the GPU, the dmabuf as it is: VA-API (`vapostproc → vah264enc`) or NVENC (`glupload → glcolorconvert → nvh264enc`), CBR 20 Mbit/s, no B-frames. Measured: hp VA-API 7 ms (nested), VM NVENC 58 fps at 1080p, 5.0 ms median, 5.6 ms at 95% from the copy to H.264 (2026-10-23).
- [x] Serving: port 7311, the page and its WebSocket (pairing, signalling), a six-digit code by notification (five wrong ones make a new one), one client, `webrtcbin` with zero-latency RTP; the page asks the browser for no buffering and shows fps, buffer and decode times (2026-10-23).
- [x] Remote mouse and keys: the page sends moves as fractions of the picture, buttons, the wheel and keys by place (`KeyboardEvent.code`); the server's virtual pointer is over the shared monitor, its keyboard uses the seat's keymap; full screen holds the keyboard (Chrome's Keyboard Lock). Tested from the hp: works (b61d73e).
- [x] Julas VM as the server: sway, rofi and Sunshine removed, Orbit and its metapackages from the repo, its own home on its disk (data folders linked to `/mnt/jas`), session open at boot (`/etc/orbit/login.yaml`, backup `.antes-remoto`), packages `orbit-login`, `orbit-prompter`, `orbit-remote-server` added to the build (2026-10-23).

Pending:

- [ ] Phase 1 **done when** the latency, a clock on the server's screen photographed beside the client's, is **≤ 20 ms on the LAN** at 60 fps (for the native client; the browser's noted). Note the page's stats too.
- [ ] The server started with the session (now by hand on the VM: `orbit-remote-server serve`, log `~/.local/state/orbit/remote.log`).
- [ ] The virtual pointer and keyboard are open to every client of the session: only orbit-remote-server's (the private protocol, or a client check).
- [ ] Pairing only while the session is unlocked; the remote greeter.
- [ ] Input over a WebRTC data channel instead of the WebSocket, if the latency asks for it.
- [ ] Audio (Opus) to the client.
- [ ] Ctrl+Alt+Backspace from the client ends the server's session: hold it back from remote keyboards.
- [ ] Phase 2: the virtual output at the client's resolution, local screens locked while in use.
- [ ] Phase 3: gamepads, microphone and webcam; `orbit-local-station` on Slint's LinuxKMS.
- [ ] Later: Android and iOS clients (check Slint's iOS support then).
- [ ] External drive on the client (TV, FireStick, Android, iOS) visible to the session; the user's whole environment possibly living on it.
- [ ] Goal: replace Sunshine + Moonlight on the Julas VM.

## 10. Open decisions

- [x] `orbit-remote-server`: Slint, for the clients and its own small interface (2026-10-22, §9).

## 11. Orbit-Edit
- [x] Two look-alike search buttons in the toolbar: quick open (Ctrl+P) now has its own icon (`go-jump-symbolic`).

## 12. Orbit-Files
- [x] Multiple selection: boxes, Shift+click and Shift+arrows for a range, Ctrl+click and Ctrl+Space for one, Ctrl+A, Ctrl+I; and the function keys F1–F12 (2026-10-22).

## 13. Orbit-Aigent

A voice layer for coding agents, after Jarvis (github.com/adewaskar/jarvis):
the daemon `orbit-aigent` and the panel's `aigent` module, Full (4 × 3) and
Mini (2 × 1). A sphere whose edge moves as a spectrum, with a spectrum
across it: white waiting, red listening, yellow speaking, the colour
sweeping in from the edge. Decided on 2026-10-22: a wake word (the
microphone always open), Portuguese and English detected by themselves,
and anything that changes something confirmed by a spoken "yes" or a click.
The day by day is in `docs/WORKLOG.md`.

**Status (2026-10-23):** working end to end on the hp with Claude Code, by
the user's voice: "Jarvis, …" → Whisper on the iGPU → `claude -p` → Piper,
with spoken confirmation of changes and interruption by the name. Started
by `orbit-session`. Not tried yet: speakers (echo cancelling set up, tried
only with the headset), the lock (needs a login with the new core), the
other three agents (not installed).

Done:

- [x] Daemon: microphone, voice detection, states, socket; `orbit-aigent demo` (2026-10-22).
- [x] Panel module: the sphere in both sizes, clicks, the agents' menu, yes/no while it asks (2026-10-22).
- [x] Whisper (whisper.cpp in the daemon): the wake word loosely matched ("Javis", "Já viz"), pt and en with Portuguese on a near tie, the name cut off a request said in one breath, a request said while the name was being heard kept (2026-10-22).
- [x] Whisper on the iGPU (Vulkan, the `vulkan` feature, built by `install.sh` when `glslc` is there): small finds the wake word in 0.4 s, large-v3-turbo hears the request in about 2 s; hints for short requests and for yes/no answers (2026-10-22/23).
- [x] Claude Code headless (`claude -p`, stream-json), the session resumed between requests, told its answer is spoken and that the desktop is Orbit; reading tools and look-only commands free (2026-10-22/23).
- [x] Piper: the answer said sentence by sentence in the request's language (`pw-play`), the sphere moving with the voice (2026-10-22).
- [x] Confirmation: `--permission-prompt-tool` → `orbit-aigent approve` (MCP) → the daemon asks aloud ("Posso abrir o kitty?"), an answer said over the question counts, a yes holds for the rest of the request; spoken yes/no or a click (2026-10-23).
- [x] Interrupting it while it speaks: "Jarvis, para" silences it, anything else after the name is a new request (2026-10-23).
- [x] Started by `orbit-session` (left running on a restart), its log in `~/.local/state/orbit/aigent.log`; deaf while the session is locked (2026-10-23).
- [x] Its own voice through speakers taken out: PipeWire's echo canceller as `orbit-aigent-mic` (`data/pipewire/60-orbit-aigent-aec.conf`), and speech that is mostly its own words ignored (2026-10-23).
- [x] Core, for it: `move … to output`, and the lock state sent on subscribing (2026-10-23).

Pending:

- [ ] Try with speakers: no self-interruption, "Jarvis, para" and confirmations through the echo canceller.
- [ ] Try the lock after a login with the new core (log: "the session is locked: deaf").
- [ ] Gemini CLI, OpenCode and Kimi Code (not installed on the hp yet): their headless modes and how each asks for permission.
- [ ] A short sound when woken.
- [ ] A shortcut in `keys.yaml` (listen/cancel).
- [ ] Packaging: the `vulkan` feature, Piper and its voices, the models (about 800 MB), the PipeWire file.
- [ ] Speaking the answer while it is still being written (stream-json), for a shorter wait.
- [ ] The diagnostics left in the log (room level every 30 s, `ORBIT_AIGENT_DUMP`) once it has run a while.
