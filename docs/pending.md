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
- [ ] `orbit-remote` pairing allowed only while the session is unlocked.

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
- [ ] Phase 5: `orbit-remote`, Jarvis wallpaper, orbit-talk integrated with the core, own monitor settings, `orbit-keys` (password app).
- [ ] In parallel: run the core on the think (AMD RX 6400).

## 5. Panel

- [ ] Uniform clicks for the old status modules (network, keyboard, battery, notifications).
- [ ] Free per-cell positions on the grid (editing only reorders the flow today).

## 6. Apps

| App | Effort | Notes |
|---|---|---|
| orbit-clip | low–medium | Small UI; Wayland clipboard protocol and sync with XWayland |
| orbit-blink | low–medium | Screenshots: full screen, current window, rectangle |
| orbit-view | medium | Image viewer: decode, zoom, browse the folder |
| orbit-read | medium | TXT and MD are simple; PDF needs a rendering library |
| orbit-play | medium–high | GStreamer, many formats |
| orbit-talk | high | Whisper + Piper, latency; ships together with its panel widget |
| orbit-files | high | A file manager has many edge cases (Yazi as reference) |
| orbit-edit | high | A real text editor |
| orbit-calc | — | Done |

## 7. Orbit Linux (`orbit-linux.txt`, plan in `docs/PLANO-DISTRO.md`)

Decided on 2026-10-21: prebuilt `.deb` packages (nothing compiled at install),
own installer (Rust + GTK4, running in orbit-core), EFISTUB as UKI with the
`BOOTX64.EFI` fallback, offline ISO, Plymouth handing over to orbit-greet.
**An existing `/home` is never formatted.**

- [ ] Phase A: CI building and publishing the debs; the hp and the think running on the packages instead of `install.sh` (`apt install orbit-desktop`).
- [ ] Phase A: `orbit-common-data`, `orbit-defaults`, `orbit-ruffle`, `orbit-fonts` (Nerd Font).
- [ ] Phase B: `orbit-studio` with ArtCraft mirrored; Orbit egui theme. Done when a fresh Debian 13 VM becomes this desktop with `apt install orbit-desktop orbit-studio`.
- [ ] Phase C (after core Phase 2): `orbit-install` and the ISO — disk, hostname, user, password; "will format X / will keep Y" confirmation; existing `/home` reused (partition, LVM, LUKS, btrfs `@home`) with the owner's UID; progress and log on screen; automated install test in a Proxmox VM.
- [ ] Phase D: site, docs, stable/testing channels, security policy, arm64 image, check the "Orbit Linux" name.
- [ ] Packages without an official `.deb` (Popsicle, Ghostty, AniCraft, craft-fonts): AppImage managed by Orbit, or own package.
- [ ] Update `docs/PLANO-DISTRO.md` §9: the "orbit-greet doesn't exist yet" risk is gone.

## 8. Orbit-Safe (`orbit-safe.txt`)

A place to keep passwords safely, reachable after authenticating. It is the
`orbit-keys` app of core Phase 5 (Secret Service, over gnome-keyring today).

- [ ] Fold `orbit-safe` and `orbit-keys` into one app and one name.
- [ ] MFA: TOTP codes (Google Authenticator style) stored and shown by the app, and/or required to open it.

## 9. Orbit-Station (`orbit-station.txt`)

A remote-only edition of Orbit Linux (LTSP-like): type the machine's IP in any
browser (phone, tablet, TV, another PC) and get the full session, with nothing
installed. Never used as a local desktop.

- [ ] Build on `orbit-remote` (core Phase 5: WebRTC, VA-API, web client) rather than compiling the UI to WASM.
- [ ] External drive on the client (TV, FireStick, Android, iOS) visible to the session; the user's whole environment possibly living on it.
- [ ] Goal: replace Sunshine + Moonlight on the Julas VM.

## 10. Open decisions

- [ ] Does wayfire leave the think too, under the same two-week rule as sway?
- [ ] Tags (dwm) and workspaces in the same panel bar, or only one of them? Decide before Phase 3.
- [ ] Installer: `/home` as a folder inside `/` — refuse, or install wiping everything but `/home`?
- [ ] Orbit-Station: WASM UI (as in the note) or streaming the real session (`orbit-remote`)? My recommendation is streaming: one code base, and the apps stay native.

## 11. Orbit-Edit
- [x] Two look-alike search buttons in the toolbar: quick open (Ctrl+P) now has its own icon (`go-jump-symbolic`).
