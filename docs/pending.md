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
- [ ] Rounded corners on rotated monitors.
- [ ] Phase 2: hotplug, fractional scale, `wlr-output-management`, portal and screen sharing, VRR/tearing. Done when the hp gets a full day without going back to sway.
- [ ] Phase 3: layouts `bsp`, `master`, `monocle`, `scroll`, per-workspace layouts, tags, named scratchpads.
- [ ] Phase 4: animations, shadows, blur, gestures, overview, effects off on battery.
- [ ] Phase 5: `orbit-remote-server` (§9), Jarvis wallpaper, orbit-aigent integrated with the core, own monitor settings, `orbit-keys` (password app).

## 5. Panel

- [ ] Uniform clicks for the old status modules (network, keyboard, battery, notifications).
- [ ] Free per-cell positions on the grid (editing only reorders the flow today).

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
| orbit-aigent | high | Not started | Voice layer for talking to coding agents (Claude Code, Gemini CLI, OpenCode, Kimi Code): Whisper + Piper, latency; ships together with its panel widget |

Only orbit-aigent and the station pair (§9) are still to be started.

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
- [ ] Update `docs/PLANO-DISTRO.md` §9: the "orbit-greet doesn't exist yet" risk is gone.

## 8. Orbit-Safe (`orbit-safe.txt`)

A place to keep passwords safely, reachable after authenticating. It is the
`orbit-keys` app of core Phase 5 (Secret Service, over gnome-keyring today).

- [ ] Fold `orbit-safe` and `orbit-keys` into one app and one name.
- [ ] MFA: TOTP codes (Google Authenticator style) stored and shown by the app, and/or required to open it.

## 9. Remote edition: `orbit-local-station` + `orbit-remote-server` (`orbit-station.txt`)

A pair that works only together (LTSP-like), replacing the old Orbit-Station
and `orbit-remote` names:

- `orbit-remote-server`: orbit-core with a remote streaming layer.
- `orbit-local-station`: no local interface of its own; it pulls the greeter,
  and then the whole session, straight from `orbit-remote-server`.

The think and the Julas VM are set aside for now: they are the test machines
for this pair.

- [ ] Not started yet.
- [ ] The remote version's interface uses Iced or Slint instead of GTK4, for their WebSocket/WebRTC (§10).
- [ ] External drive on the client (TV, FireStick, Android, iOS) visible to the session; the user's whole environment possibly living on it.
- [ ] Goal: replace Sunshine + Moonlight on the Julas VM.

## 10. Open decisions

- [ ] Tags (dwm) and workspaces in the same panel bar, or only one of them? Decide before Phase 3.
- [ ] `orbit-remote-server`: Iced or Slint?

## 11. Orbit-Edit
- [x] Two look-alike search buttons in the toolbar: quick open (Ctrl+P) now has its own icon (`go-jump-symbolic`).
