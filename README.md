# Orbit Rack

![Orbit: the core, panel, dock and applications of the shell](orbit.png)

**Orbit Rack** is a desktop for Linux written in **Rust**: the full set of
Orbit components — a Wayland compositor, a panel and a dock, and the
everyday applications around them — designed to look and behave as one.
It is part of the [JUX](https://github.com/julas23/jux) desktop suite.

> **Status:** early development. **Orbit-Panel** and **Orbit-Dock** work and
> are being prepared to replace the [EWW](https://github.com/elkowar/eww)
> panel they were rewritten from; the other components are planned.
> Configuration and APIs may still change.

## The rack

| Component | What it is | Status |
| --- | --- | --- |
| **orbit-core** | The compositor, built on [Smithay](https://github.com/Smithay/smithay) | planned |
| **orbit-panel** | Panel with independent modules (clock, calendar, weather, tray, windows…) | **working** |
| **orbit-dock** | Application launcher dock | **working** |
| orbit-view | Universal image viewer | planned |
| orbit-read | PDF, Markdown and plain-text reader | planned |
| orbit-files | Complete file manager | planned |
| orbit-play | Video player for common formats | planned |
| orbit-talk | Speech: Whisper (speech-to-text) and Piper (text-to-speech) | planned |
| orbit-clip | Copy and paste across Wayland and XWayland | planned |
| orbit-calc | Calculator: standard, scientific, HP-12C and IP subnet profiles | planned |
| orbit-edit | Text editor: column selection and indentation by level | planned |

Orbit is a clean reimplementation: existing desktops and compositors are
studied as references, but no code is copied from them.

## Principles

- **Native and event-driven.** Data arrives by events — compositor IPC,
  D-Bus, PipeWire, kernel notifications — or internal timers. Nothing polls by
  spawning processes. The whole panel idles at about 0.1% of a CPU core.
- **Independent, optional modules.** Every part of the panel can be turned on,
  off and reordered in its configuration file.
- **Content always upright.** Text and icons are never rotated, whatever edge
  the panel is on: moving the panel changes how its content is arranged, not
  which way it faces.
- **Themeable with CSS.** The look is GTK CSS, on top of a built-in theme,
  reloaded live when saved.
- **One process per surface.** The panel and the dock run separately, so a
  crash in one leaves the other running.

## Orbit-Panel

A column (or row) of independent modules, anchored to a screen edge.

| Module | Shows | Source |
| --- | --- | --- |
| `menu` | Application menu button | — |
| `windows` | Open windows: app name and icon, area colour, focus, minimized; menu to minimize, maximize, send to an area, close. Ordered by area, then monitor and window position, top to bottom and left to right | sway IPC |
| `clock` | Date, local time with zone, a minute "hourglass" that empties, up to four world clocks | internal timer |
| `calendar` | Month as a 7 × 6 grid, weekends and today highlighted | internal timer |
| `weather` | Current conditions, min/max, sun and moon, humidity and wind | wttr.in + Open-Meteo |
| `forecast` | The next days | same as `weather` |
| `tray` | System tray (StatusNotifierItem), with application menus | D-Bus |
| `brightness` | Backlight, or an external monitor over DDC/CI | kernel + logind / ddcutil |
| `volume` | Default output volume and mute | PulseAudio protocol (PipeWire) |
| `workspaces` | Global areas, shared by all monitors | sway IPC |
| `session` | Power off, reboot, log out — confirmed over a blurred screen | logind / sway |

Still to come from the EWW panel: **network**, **keyboard layout** and
**battery**.

**Layout.** Every size, gap and padding is a multiple of 5 px. A vertical
panel is three columns wide (150 px by default) and each module has a *block*
form; a horizontal panel is one line (50 px) and each module has a *line* form,
with the details in tooltips and popovers. Small modules share a line in a
vertical panel. `maintenance: true` draws the 5 px grid behind the panel to
check alignment.

## Orbit-Dock

A row of launchers on a screen edge, built from desktop files, in groups with
separators between them. Icons come from the icon theme or from the files the
desktop entries point to, loaded at display size.

## Building and running

Requirements: Rust 1.88+, GTK 4.12+ and `gtk4-layer-shell` development files.
On Debian/Ubuntu:

```sh
sudo apt install libgtk-4-dev libgtk4-layer-shell-dev
cargo build --release
./target/release/orbit-panel &
./target/release/orbit-dock &
```

Start the panel first. Each surface reserves its edge (layer-shell exclusive
zone) and the compositor lays them out in the order they appear: a left
panel started first keeps the full height, and a top dock started after it
takes the top edge to the right of the panel.

Runtime: a Wayland compositor with `wlr-layer-shell` (developed on sway; the
`windows` and `workspaces` modules talk to sway's IPC). Optional: `grim` for
the blurred confirmation screen, `ddcutil` for external-monitor brightness.
The panel and dock also have an X11 backend (EWMH dock hints and struts),
tested on bspwm in earlier versions.

## Configuration

Everything lives in `~/.config/orbit/` (or the directory given with
`--config DIR`), one file per part:

```text
~/.config/orbit/
├── panel.yaml        edge, size, monitor, maintenance, and the module lists
├── dock.yaml         the dock and its launchers
├── style.css         user stylesheet, on top of the built-in theme
└── modules/
    ├── clock.yaml    one optional file per panel module
    ├── weather.yaml
    └── …
```

Every file and every key is optional. A module is turned on by listing it in
`panel.yaml` under `start`, `center` or `end`, in display order. A module that
is unknown or whose file has an error is left out with a message in the log;
the rest of the panel still starts. [`examples/orbit/`](examples/orbit) has
every key, and [`examples/maintenance/`](examples/maintenance) runs the panel
and dock on a second monitor with the layout grid shown:

```sh
./target/release/orbit-panel --config examples/maintenance &
./target/release/orbit-dock --config examples/maintenance &
```

The weather pictures (from the brweather themes) are not distributed here;
point `icon_dir` in `modules/weather.yaml` at them, or Nerd Font glyphs are
used instead.

## Project layout

| Crate | Purpose |
| --- | --- |
| `orbit-common` | Configuration, CSS loading, display-backend abstraction |
| `orbit-backend-wayland` | Wayland backend: layer-shell |
| `orbit-backend-x11` | X11 backend: EWMH dock type, struts and positioning |
| `orbit-app` | Startup shared by the panel and the dock, icon loading |
| `orbit-panel` | The panel and its modules |
| `orbit-dock` | The dock |

The plan for the panel (in Portuguese) is in
[`docs/PLANO-PAINEL.md`](docs/PLANO-PAINEL.md).

## Contributing

Ideas and feedback are welcome. The project is young: please open an issue to
discuss larger changes before a pull request.

## License

Orbit Rack is licensed under the
[GNU General Public License, version 3 or any later version](LICENSE).
