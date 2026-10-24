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
- [x] `orbit-remote-server` pairing allowed only while the session is unlocked (2026-10-23, §9).
- [x] orbit-greet chooses between the full local desktop and the remote desktop (§9; done 2026-10-23, fb6deb3).

## 4. Core

- [x] Floating windows: dialogs (X and Wayland) float at their own size, centred over their parent; `floating enable|disable|toggle` (Mod+Shift+Space, Mod+Esc); Mod+drag leaves a floating window where it is dropped. Verified on screen 2026-10-22.
- [x] Arrow shortcuts: Mod+↑ maximize, Mod+↓ restore, Mod+←/→ snap to a half; Alt+arrows focus that way, Mod+Ctrl+arrows move that way (other monitor, then the neighbouring area of the 2×2 grid; nothing at its edge). Verified on screen 2026-10-22.
- [x] Switching: Ctrl+Alt+arrows walk the areas; Alt+Tab (last used first, while Alt is held) on the workspace, Alt+Shift+Tab on every workspace; Mod+Tab the window cards (Looking Glass style, in perspective, over the blurred screen). Verified on screen 2026-10-22.
- [x] Blur: under the panel's confirmation dialog (`blur_layers`, done by the core — no screencopy for grim), and the lock's blur now runs ahead of the black so it shows.
- [x] Mod+right-drag resize: a floating window by the corner nearest the pointer (a snapped one floats first); a tiled one moves the line to the neighbouring column (column weights); the clients' own frame edges too (xdg and X). Mod+Esc toggles floating, as Mod+Shift+Space. Verified on screen 2026-10-22.
- [x] Screen capture for clients (`wlr-screencopy`, `screencopy.rs`): Orbit-Blink and grim work on orbit-core; refused while locked. Nested-tested 2026-10-22; installed, awaiting a session restart.
- [x] Screencopy with damage: `copy_with_damage` waits until the monitor changed since the client's last copy and says what (a tracker per client, monitor and region, looked at after each frame drawn); nested-tested with orbit-remote-server's `capture`: 1.3 fps on a still screen (2026-10-23).
- [x] Rotated monitors: `transform:` per monitor in `core.yaml` (sway's names), applied on connecting and on `reload` (which now lays the monitors out again too), and told over IPC. The corners' shader works in the element's own pixels, before the monitor's turn (the nested window already draws turned, `flipped-180`, with right corners) (2026-10-23). Not tried on a real turned monitor; screencopy of a turned one (grim, the remote server) not checked.
- [x] Phase 2, fractional scale: `scale:` per monitor in `core.yaml` (0.5 to 4, applied on connecting and on `reload`), `wp_fractional_scale_v1`, and each window and layer told its monitor's scale after that monitor's frames (a window on two, the one its middle is on). Nested at 1.5: GTK gets `preferred_scale(180)` and draws sharp (2026-10-23).
- [x] Phase 2, screen sharing through the portal: `orbit-portals.conf` (screencasts and screenshots by xdg-desktop-portal-wlr, secrets by GNOME Keyring, the rest by GTK); without it an Orbit session offered no ScreenCast at all. In `orbit-defaults` and put by `install.sh`; on the hp the portal now offers ScreenCast and Screenshot (2026-10-23). A real share verified on the hp (2026-10-24); with no slurp, xdpw picks a monitor by itself.
- [x] A nested core leaves `startup:` and `login:` out unless `--startup` asks: the default ones handed its display to the outer session's systemd and D-Bus (the outer portals then failed) and `orbit-session` took the outer panel and dock (2026-10-23).
- [x] Phase 2, `wlr-output-management` (ported from niri): wlr-randr, kanshi and nwg-displays see every monitor with its connector's modes, and set mode, place, turn and scale for the session (as `outputs:` would; the file is left alone); switching one off fails, not done yet. Nested with wlr-randr: a virtual monitor's mode and scale and the window's turn applied (2026-10-23). Not tried: a mode change on a real monitor (DRM `use_mode`).
- [ ] Phase 2: hotplug, a monitor switched off, VRR/tearing. Done when the hp gets a full day without going back to sway.
- [x] Phase 3, layouts: each workspace has its own, `columns` (as before), `master` (55% for the first, the rest stacked), `monocle` (one at a time, the focused on top) and `bsp` (bspwm's spiral); `layout NAME` and `layout next` (Mod+Ctrl+Space), `layout:` in core.yaml for new workspaces, the name in get_workspaces (2026-10-23, nested-tested on a virtual monitor; verified on the hp 2026-10-24).
- [x] Phase 3: `master_ratio` and `master_count` in core.yaml; named scratchpads as sway's, `[app_id=…] scratchpad show`: hidden, the window comes floating over the workspace in front; in front and focused, it goes back (2026-10-23, nested-tested).
- [ ] Phase 3: the `scroll` layout (niri's columns that scroll). (Tags left out on 2026-10-22: maybe in a later major version.)
- [x] Phase 4, effects off on battery: `effects: auto` (the default) leaves out the lock's eyes and the blur behind confirmations while on battery (mains looked at every 10 s); `on` and `off` force it (2026-10-23). Not seen yet on battery.
- [ ] Phase 4: animations, shadows, blur, gestures, overview.
- [ ] Phase 5: `orbit-remote-server` (§9, phase 1 working 2026-10-23), Jarvis wallpaper, orbit-aigent integrated with the core, own monitor settings, `orbit-keys` (password app).

## 5. Panel

- [x] Uniform clicks for the old status modules (network, keyboard, battery, notifications): left the quick part, right the full one, the wheel a value (2026-10-22).

## 5a. Dock

- [x] Adding a separator did nothing when the right click was on a gap or a separator: it went to the end of the list, where a separator does not show. Now it goes where the pointer is (launchers and AppImages too); tested nested (2026-10-23).

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

- [x] Phase A: CI building the debs: `.github/workflows/debs.yml` runs `distro/ci/build.sh` in a Debian 13 container at each push to main, the .debs kept as the run's artifact; the same script tried at home in `docker run debian:trixie` (28 packages, 2026-10-23). Publishing stays at home (the repository server is on the LAN).
- [x] Phase A: the hp on the packages instead of `install.sh` (the think is, since 2026-10-23): orbit-desktop, orbit-remote-server, orbit-local-station, orbit-aigent and ArtCraft's 14 from the local repository, LibreOffice purged, `/usr/local/bin/orbit-*` and `~/.local/bin/orbit-*` removed (2026-10-24). To verify after a reboot.
- [x] The keyring prompter's D-Bus service in `orbit-prompter` (`/usr/share/dbus-1/services/`), gcr's file of the same name set aside by `dpkg-divert`; no copy by hand in the home any more (2026-10-24).
- [x] Packaging: `orbit-notify` says `Provides: notification-daemon` and `orbit-polkit` `Provides: polkit-1-auth-agent`, so dunst and the other polkit agents can go without taking blueman and network-manager-applet along (2026-10-23).
- [x] The hp's `~/.local/bin/orbit-*` (its own builds, synced by Syncthing) came before the packages' `/usr/bin` on the think: off the hp, `~/.shell/10-path.sh` puts links to `/usr/bin/orbit-*` in `$XDG_RUNTIME_DIR/orbit-bin` ahead of the PATH (2026-10-23).
- [x] Phase A: `orbit-defaults` (default applications by MIME type, the GSettings override, the examples), pulled by `orbit-desktop`; `orbit-common-data` not needed, its files are compiled in (2026-10-23).
- [x] Phase A: `orbit-fonts`, CaskaydiaCove Nerd Font (the panel's) from the Nerd Fonts release, its own version (3.5.1-1), the same file every build; pulled by `orbit-desktop` (2026-10-23).
- [x] Phase A: `orbit-ruffle`, Ruffle's own nightly (the one the hp runs, 2026-10-20), pinned and checked, the same file every build; `orbit-play` suggests it (2026-10-23).
- [ ] Phase B: `orbit-studio` with ArtCraft mirrored; Orbit egui theme. Done when a fresh Debian 13 VM becomes this desktop with `apt install orbit-desktop orbit-studio`. Its packages resolve: a clean `debian:trixie` container with the local repository plans 1155 packages, every Orbit and ArtCraft one among them (2026-10-23); the VM itself and the egui theme still to do.
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
- [x] `orbit-local-station` started (2026-10-23): the screen and keyboard straight from a text console through Smithay's backend (libseat, DRM, libinput; no compositor), its own screens in Slint's software renderer (Slint's LinuxKMS backend hands over keys only as text, the server wants them by place). Pairs once by the code; the server hands back a token (it keeps only the SHA-256, `~/.local/state/orbit/remote-tokens`; the station keeps it with the server's address). The server's address typed on the screen when none is set (`~/.config/orbit/station.yaml`, over `/etc/orbit/station.yaml`), F2 for another; Ctrl+Alt+Backspace quits. Tested on the hp against its own server: pairing, token, reconnecting (b386573).
- [x] The greet chooses Local or Remote, as a display manager chooses a desktop (2026-10-23): Remote has Orbit-Login check the same password (`station` request) and open a second session of the user on a free console running `orbit-local-station`; the local session stays locked on tty1 and comes back when the station quits. Shown only with Orbit-Login and the station installed.
- [x] `orbit-local-station` shows the session (2026-10-23): webrtcbin, VA-API decoding into a dmabuf put on the screen as it is, the picture fitted whole; the keys (evdev codes) and the mouse go to the server, Ctrl+Alt+Backspace and Ctrl+Alt+F1…F12 kept locally; leaving the session on the server (`ended`) quits the station. Tested on the hp: works.
- [x] Server: keys also by their evdev code (the station's); view only when the compositor has no virtual pointer, instead of dropping the client.
- [x] Julas VM as the server: sway, rofi and Sunshine removed, Orbit and its metapackages from the repo, its own home on its disk (data folders linked to `/mnt/jas`), session open at boot (`/etc/orbit/login.yaml`, backup `.antes-remoto`), packages `orbit-login`, `orbit-prompter`, `orbit-remote-server` added to the build (2026-10-23).

Pending:

- [ ] Phase 1 **done when** the latency, a clock on the server's screen photographed beside the client's, is **≤ 20 ms on the LAN** at 60 fps (for the native client; the browser's noted). Note the page's stats too.
- [x] The server started with the session: on the Julas VM a `startup:` line in `~/.config/orbit/core.yaml` (hostname-guarded, behind the lock, log `~/.local/state/orbit/remote.log`). Packaged since 2026-10-24: `orbit-session` starts it when `~/.config/orbit/remote.yaml` (else `/etc/orbit/remote.yaml`, shipped off) says `start: true`, `virtual: true` for `serve --virtual`; left running on a session restart. The Julas VM's line moved there (2026-10-24, its `remote.yaml` says `start: true`; verified at a boot: the server from /usr/bin listening on 7311, the session unlocked).
- [x] The virtual pointer and keyboard only for orbit-remote-server: the core gives the two globals to a client whose program (`/proc/PID/exe`) is it, other clients do not see them (2026-10-23, nested-tested).
- [x] Pairing only while the session is unlocked: the server follows orbit-core's `lock` event; a code given while locked is answered `locked` (the page and the station say so), and the code is shown only once the session is unlocked. A remembered station still comes in, to the lock screen (2026-10-23, nested-tested).
- [ ] The remote greeter: a station reaching a server with no session open (who logs in, from the station).
- [ ] Input over a WebRTC data channel instead of the WebSocket, if the latency asks for it.
- [x] Audio (Opus) to the client: the default output's monitor (`pulsesrc`, PipeWire's PulseAudio server), 10 ms frames, in the same WebRTC connection; the page plays it with the video (muted until a click if the browser insists), the station on its default output (`pulsesink`). Tested nested with a WebRTC test client: both tracks decoded (2026-10-23). Not tried yet: the station and the browser for real.
- [ ] Sound with the server and the station on the same machine (the hp against itself) comes back through the monitor: an echo. And the server's own speakers keep playing while a client listens (with phase 2, mute them).
- [x] Ctrl+Alt+Backspace and Ctrl+Alt+F1…F12 from a remote keyboard are held back by the core: they neither end the session nor switch the server's console (2026-10-23, nested-tested).
- [x] Phase 2, its base: virtual monitors in orbit-core (`output NAME create 1920x1080@60`, `output NAME remove`; a timer at the mode's rate stands in for the vblank), placed to the right of the local ones and last in the areas, so no local monitor moves. `orbit-remote-server serve --virtual` makes `REMOTE-1` at the size the client gives when it pairs (the station its monitor's, the page the screen's in device pixels) and removes it when the client leaves; plain `serve` mirrors as before (2026-10-23, nested-tested end to end).
- [ ] Phase 2, to decide together: `--virtual` as the default; what the remote sees there (today an empty area: the panel and dock show only on their `monitor:`, windows stay where they are); the local screens locked while a client uses it (a lock of the local monitors only, the virtual one and the remote input left working).
- [ ] Several monitors, only when the station has two or more (the hp has two): the server gives the session as many screens (phase 2's virtual outputs, one stream each), the station shows each on its own monitor, the pointer crossing between them as locally. With one monitor everything stays as it is.
- [ ] Phase 3: gamepads, microphone and webcam.
  - Microphone, begun on branch `wip/station-mic` (2026-10-23): the server side works (a `pw-loopback` whose source is "Orbit Remote microphone", recorded from like any other; `pipewiresink mode=provide` makes a source that gives nothing), the station sends its default microphone. What stops it: the station's `webrtcbin`, as the answerer, leaves the microphone's transceiver unassociated (kind unknown) and answers the sound's line recvonly; a pad asked as `sink_1` before the offer trips an assertion in webrtcbin; a third, recvonly line on the server comes first in the offer. To try: the station asking for its line once the offer is set (`sink_N` of the server's microphone line, with that line kept last), or the station making the offer for its own line.
- [ ] The station's USB and Bluetooth devices in the session. To decide: recreated on the server (read on the station, a virtual device there, as the gamepads, microphone and webcam) or passed through whole (USB/IP). Bluetooth devices stay paired on the station; headphones need only the audio channel.
- [ ] PXE boot: a station machine boots from the network, with nothing installed, straight into the greet pointing at its orbit-remote-server (DHCP/TFTP or iPXE over HTTP, a small kernel and initramfs with the greet and `orbit-local-station`, the server's address given by the boot).
  - Begun 2026-10-24 with LTSP on the Julas VM (`distro/ltsp/build-station-image.sh`): a minimal Debian 13 chroot (`/srv/ltsp/orbit-station`, 516 MB squashfs) with `orbit-local-station`, a `station` user logged in on tty1 straight into it, `server:` the Julas VM; dnsmasq as a proxyDHCP only (the LAN's DHCP untouched), iPXE, NFS. Not booted yet (the think is the test station). Later: the token lives in RAM, a reboot pairs again; the greet in front instead of the station alone.
  - The station decodes on any machine (2026-10-24, 687f8f0, 9320450): `decoder:` in station.yaml, `auto` taking VA-API, then V4L2 (a Raspberry Pi's `v4l2h264dec`, converted by `v4l2convert`, the Pi's ISP), then software (`avdec_h264`); a picture in memory is scaled by the screen as it draws (on the hp: 1080p decoded in 2.7 ms, converted in 3.3; the CPU's scaling would add 4). Not tried on a real station yet.
  - Raspberry Pi 4 (2026-10-24): orbit-local-station for arm64 (`build-debs.sh --arm64`, cross-compiled in Docker; the repository publishes orbit and orbit-public for arm64 too), `distro/ltsp/build-station-image-rpi.sh` on the Julas VM: Raspberry Pi OS Lite arm64 (its kernel has the H.264 decoder) in `/srv/ltsp/orbit-station-rpi`, the root over NFS with no initramfs, LTSP's init, cloud-init off, the Pi's boot files at the TFTP root. Not booted yet: the Pi needs the network in its EEPROM's boot order and a cable.
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
- [x] Mod+A to start listening and to cancel, with the chime (verified on the hp 2026-10-24).

Pending:

- [ ] Try with speakers: no self-interruption, "Jarvis, para" and confirmations through the echo canceller.
- [ ] Try the lock after a login with the new core (log: "the session is locked: deaf").
- [ ] Gemini CLI, OpenCode and Kimi Code (not installed on the hp yet): their headless modes and how each asks for permission.
- [x] A short sound when woken: two rising notes through `pw-play` as its voice (the echo canceller takes them out), `chime: false` in `aigent.yaml` for none (2026-10-23). Not heard yet.
- [x] A shortcut: Mod+A, `exec orbit-aigent toggle` (listen, or cancel), among the core's defaults (2026-10-23).
- [x] Packaging: the PipeWire file, in `/usr/share/pipewire/pipewire.conf.d/`, and pw-play and the echo canceller recommended (2026-10-23).
- [ ] Packaging: the `vulkan` feature, Piper and its voices, the models (about 800 MB).
- [ ] Speaking the answer while it is still being written (stream-json), for a shorter wait.
- [x] The diagnostics taken out: the room level every 30 s and `ORBIT_AIGENT_DUMP` (2026-10-24).
