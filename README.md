# Orbit Panel

![Orbit: the core, panel, dock and applications of the shell](orbit.png)

**Orbit-Panel** and **Orbit-Dock**: a desktop panel and an application dock for Linux, written in **Rust** with **GTK4** and fully themeable through **CSS**. They are the panel and dock of the Orbit shell, part of the [JUX](https://github.com/julas23/jux) desktop suite.

Orbit Panel is a ground-up rewrite of an existing panel/dock setup built on [EWW](https://github.com/elkowar/eww). The goal is to keep the flexibility of CSS-based styling while gaining the performance, type safety, and maintainability of a native Rust application.

> **Status:** early development. The panel and dock run on X11 and Wayland with a clock, an app-menu button and launchers; most modules are still to come. APIs, configuration format, and features are all subject to change.

## Goals

- **Native and lightweight** — two small Rust binaries, no scripting runtime or polling shell scripts.
- **One process per surface** — `orbit-panel` and `orbit-dock` run separately, so a crash in one does not take the other down.
- **CSS-first theming** — the entire look and feel is defined in a user stylesheet, with live reload.
- **Panel + dock** — a top/bottom panel and an application dock, both anchored to screen edges.
- **X11 and Wayland** — a backend abstraction: EWMH dock hints and struts on X11 (tested on bspwm), the layer-shell protocol via `gtk4-layer-shell` on Wayland.
- **Easy migration from EWW** — familiar concepts and styling for users coming from an EWW-based setup.

## Planned features

### Panel
- Workspaces indicator
- Focused window title
- Clock and calendar
- System tray
- Audio, network, battery, and other status modules

### Dock
- Pinned and running applications
- Running/focused indicators
- Click to launch or focus

## Tech stack

| Component      | Choice                                                                 |
| -------------- | ---------------------------------------------------------------------- |
| Language       | [Rust](https://www.rust-lang.org/)                                     |
| UI toolkit     | [GTK4](https://gtk.org/) via [gtk4-rs](https://gtk-rs.org/)            |
| Layer shell    | [gtk4-layer-shell](https://github.com/wmww/gtk4-layer-shell)           |
| Styling        | GTK CSS                                                                |

## Building

Requirements:

- Rust (stable, 1.88+) and Cargo
- GTK 4.12+ development files
- `gtk4-layer-shell` development files

On Debian/Ubuntu:

```sh
sudo apt install libgtk-4-dev libgtk4-layer-shell-dev
```

Then:

```sh
cargo build --release
./target/release/orbit-dock &
./target/release/orbit-panel &
```

Start the dock first: where a left panel meets a top dock, the surface created
last is drawn on top. On Wayland each surface uses its own layer-shell
namespace (`orbit-panel`, `orbit-dock`), so compositor rules can target them
separately.

## Configuration

Everything lives in `~/.config/orbit/` (or the directory given with
`--config DIR`), one file per part:

```text
~/.config/orbit/
├── panel.yaml        position, size, monitor, and the list of modules
├── dock.yaml         the dock
├── style.css         user stylesheet
└── modules/
    ├── menu.yaml     options of each panel module
    └── clock.yaml
```

Every file and every key is optional. A panel module is turned on by listing
it in `panel.yaml` (under `start`, `center` or `end`, in display order); its
file under `modules/` only holds its options. A module that is unknown or
whose file has an error is left out with a message in the log; the rest of
the panel still starts. See [`examples/orbit/`](examples/orbit) for all keys.

The old single `config.yaml` is no longer read; the binaries warn if they find
one without a `panel.yaml` next to it.

`style.css` is applied on top of the [built-in theme](data/style.css) and
reloaded automatically when saved.

### Maintenance mode

`maintenance: true` in `panel.yaml` draws the 5 px layout grid behind the
panel, to check that every size lands on it. To try it on a second monitor
without touching your own config:

```sh
./target/release/orbit-panel --config examples/maintenance
```

On X11 the panel and dock reserve their space through `_NET_WM_STRUT_PARTIAL`,
so window-manager padding such as bspwm's `left_padding`/`top_padding` is not
needed. Set `reserve: false` to disable this.

## Project layout

| Crate                     | Purpose                                                    |
| ------------------------- | ---------------------------------------------------------- |
| `orbit-common`            | Configuration, CSS loading, display-backend abstraction    |
| `orbit-backend-x11`       | X11 backend: EWMH dock type, struts and positioning        |
| `orbit-backend-wayland`   | Wayland backend: layer-shell                               |
| `orbit-app`               | Startup shared by both binaries: arguments, config, CSS, backend selection |
| `orbit-panel`             | The `orbit-panel` binary: menu button and status widgets  |
| `orbit-dock`              | The `orbit-dock` binary: application launchers            |

## Contributing

Contributions, ideas, and feedback are welcome. As the project is in its early stages, please open an issue to discuss larger changes before submitting a pull request.

## License

Orbit Panel is licensed under the [GNU General Public License v3.0](LICENSE).
