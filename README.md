# Orbit Rack

![Orbit: the core, panel, dock and applications of the shell](orbit.png)

**Orbit Rack** is a desktop for Linux written in **Rust**: the full set of
Orbit components — a Wayland compositor, a panel and a dock, and the
everyday applications around them — designed to look and behave as one.
It is part of the [JUX](https://github.com/julas23/jux) desktop suite.

> **Status:** early development. **Orbit-Panel** and **Orbit-Dock** work and
> have replaced the [EWW](https://github.com/elkowar/eww) panel they were
> rewritten from; **Orbit-Calc**, **Orbit-Clip**, **Orbit-Blink**,
> **Orbit-View**, **Orbit-Read**, **Orbit-Play**, **Orbit-Edit** and
> **Orbit-Files** work; the other components are planned.
> Configuration and APIs may still change.

## The rack

| Component | What it is | Status |
| --- | --- | --- |
| **orbit-core** | The compositor, built on [Smithay](https://github.com/Smithay/smithay): runs nested for now, tiles windows, hosts the panel and dock | in progress |
| **orbit-panel** | Panel with independent modules (clock, calendar, weather, tray, windows…) | **working** |
| **orbit-dock** | Application launcher dock | **working** |
| **orbit-notify** | Notification server in place of dunst: bubbles, a centre grouped by app, do not disturb, the panel's bell | **working** |
| **orbit-view** | Universal image viewer: every format in a sandbox, zoom, thumbnails, EXIF, slideshow | **working** |
| **orbit-read** | PDF, Markdown and plain-text reader: outline, thumbnails, search, night mode | **working** |
| **orbit-files** | File manager: Yazi's speed, three views, two panes, preview, bulk rename, actions, archives, network | **working** |
| **orbit-play** | Video and audio player for practically every format, and Flash movies through Ruffle | **working** |
| **orbit-pathfinder** | Private web browser on WebKitGTK: a sidebar of services in groups (Wavebox's layout), each in storage of its own, unread counts (Ferdium recipes) and notifications, EasyList/EasyPrivacy blocking, idle services hibernated, tracking parameters stripped, no telemetry | **working** |
| **orbitctl** | Commands and queries to orbit-core (and sway), as `swaymsg`: `orbitctl workspace number 2`, `orbitctl -t get_tree` | **working** |
| orbit-talk | Speech: Whisper (speech-to-text) and Piper (text-to-speech) | planned |
| **orbit-clip** | Clipboard history for Wayland and XWayland: text and images, search | **working** |
| **orbit-blink** | Screen capture: the whole screen, the current window or a selected area | **working** |
| **orbit-calc** | Calculator: standard, scientific, financial (HP-12C) and IPv4 subnet profiles | **working** |
| **orbit-edit** | Text and code editor: several cursors, column selection, indent guides and folding, folder sidebar, session | **working** |

Orbit is its own code. Existing desktops and compositors are studied as
references; where orbit-core takes code from one (cosmic-comp, niri, anvil,
sway), the file says where it came from and keeps that project's licence —
code from cosmic-comp is GPL-3.0-only, so the orbit-core binary is too, while
the rest of Orbit stays GPL-3.0-or-later (see `docs/PLANO-CORE.md`, section 9).

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
- **Follows the system.** Light or dark and the accent colour come from the
  desktop (XDG portal) and switch live; the language comes from the locale.

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
| `network` | Connection name and IP/prefix; the icon opens the connection editor (left) or the network menu (right) | NetworkManager |
| `keyboard` | Layout language and variant; click for the next layout | sway IPC |
| `battery` | Charge and uptime | UPower |
| `session` | Power off, reboot, log out — confirmed over a blurred screen | logind / sway |

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

An AppImage — picked with "Add AppImage…" while editing the dock, or dropped
on it — needs nothing else: its icon is taken out of it and kept next to it,
and a desktop file that runs it is written to `~/.local/share/applications`.

## Orbit-Calc

A calculator with four profiles, one tab each; it always opens on the
standard one. The window has a fixed size, each profile's own, so tiling
compositors float it.

- **Standard** and **scientific**: type an expression — with the keypad or
  the keyboard — and see its value as you type; usual precedence,
  parentheses, desk-calculator percent (`200 + 10%` is 220), trigonometry in
  degrees or radians, logarithms, powers, roots, factorial.
- **Financial**: after the HP-12C — the 12c's keyboard and RPN stack, with its gold (f) and blue
  (g) functions: time value of money (n, i, PV, PMT, FV, BEGIN/END, 12×,
  12÷), amortization, simple interest, cash flows with NPV and IRR,
  depreciation (SL, SOYD, DB), bond price and yield, statistics and linear
  estimates, dates (DATE, ΔDYS, D.MY/M.DY) and the STO/RCL registers.
  Programming (P/R, R/S, GTO…) is not implemented.
- **IPv4 subnet**: an address with a prefix or mask in, its mask, wildcard,
  network, broadcast, host range and count, class, kind (private, CGNAT,
  loopback…) and binary form out.

Numbers follow the language: `1.234,5` in Portuguese, `1,234.5` in English.

## Orbit-Clip

A clipboard history. It runs in the background and keeps what is copied —
text and images, from Wayland and X11 (XWayland) applications alike — in a
history that survives restarts (`~/.local/share/orbit/clip/`). Launching it
again shows or hides the history: typing searches it, Enter or a click puts
an item back in the clipboard, ready for Ctrl+V; Delete removes one.

It can keep the clipboard and the primary selection (mouse selection,
middle-button paste) in sync, and leaves out what password managers mark as
secret. Options in `~/.config/orbit/clip.yaml` (see
[`examples/orbit/clip.yaml`](examples/orbit/clip.yaml)). It needs a
compositor with `wlr-data-control` (sway, Hyprland, river, labwc…).

In sway:

```
exec orbit-clip --background
bindsym $mod+v exec orbit-clip
```

## Orbit-Blink

Screen capture: the whole screen, the focused window, or an area chosen on a
frozen picture of the screens. Each capture is saved in `$HOME`, copied to the
clipboard, and shown on a card with a thumbnail and buttons to copy it again
(if something else was copied since) or save a copy elsewhere. Without a
mode it opens an options window (mode, delay, whether to include the
cursor). It needs sway (for the layout of the screens and windows) and
`wlr-screencopy`.

```
bindsym Print       exec orbit-blink screen
bindsym Shift+Print exec orbit-blink area
bindsym $mod+Print  exec orbit-blink
```

## Orbit-View

An image viewer. The image fills the window and the controls show while the
pointer moves. Opening an image lists its folder — arrow keys, the buttons or
a strip of thumbnails go through it, and the list follows files added or
removed there. The wheel or a pinch zooms around the pointer, dragging pans,
a double-click switches between fitted and actual size. It turns and mirrors
images and saves that into JPEG files (by changing only the orientation tag
when there is one) and PNG files; shows the file's and the camera's details
(EXIF: date, camera, exposure, place with a map link); plays animations and
slideshows (F5); copies the image; and sends files to the trash (Delete).

Every format is decoded by [glycin](https://gitlab.gnome.org/GNOME/glycin) —
the engine of GNOME's Loupe — each in a sandbox, so a malformed file can only
crash its loader: PNG, JPEG, GIF, WebP, AVIF, HEIC, JPEG XL, SVG, TIFF, QOI,
OpenEXR and more, as installed (`glycin-loaders`). Thumbnails share the
freedesktop.org cache with file managers. Options in
`~/.config/orbit/view.yaml` (see
[`examples/orbit/view.yaml`](examples/orbit/view.yaml)).

| Key | | Key | |
| --- | --- | --- | --- |
| ← → Space | previous, next | + − | zoom in, out |
| Home End | first, last | 0 1 | fit, actual size |
| R, Shift+R | turn right, left | H V | mirror |
| Ctrl+S | save the turn | Ctrl+C | copy |
| I | information | F5 | slideshow |
| F11 or F | full screen | Delete | to the trash |

## Orbit-Read

A reader for PDF, Markdown and plain text; documents open at the top.

- **PDF**, drawn by [poppler](https://poppler.freedesktop.org/): pages one
  under another, fitted to the width or zoomed (Ctrl + wheel); a sidebar
  with the outline, when the document has one, and the pages; search with
  every match marked (Ctrl+F, Enter / Shift+Enter); text selected by
  dragging and copied with Ctrl+C; links followed, inside the document or
  out; night mode, which turns the pages' colours over and keeps their hues
  (Ctrl+I); printing (Ctrl+P); and a password asked for when the file has
  one. Only the pages on screen are rendered, so long documents open at
  once.
- **Markdown**, laid out: headings, emphasis, lists and task lists, quotes,
  code, tables, local images, links (to other documents, which open in the
  reader, and to headings in the page) and footnotes; Ctrl+U shows the
  source.
- **Plain text** in a monospaced font, UTF-8 or Latin-1.

The text keeps to a comfortable width, centred; Ctrl + / − / 0 changes its
size.

## Orbit-Play

A player for video and audio, on [GStreamer](https://gstreamer.freedesktop.org/)
with FFmpeg's decoders behind it (`gstreamer1.0-libav`), so practically
every format plays — MKV, MP4, WebM, AVI, FLV, WMV, RMVB, MP3, FLAC, Opus…
— decoded on the GPU where it can (VA-API). The picture is drawn into the
window by GTK; audio shows the album's cover, title and artist.

The folder is a playlist: previous and next, and on to the next when one
ends. Audio tracks and subtitles — embedded, the file of the same name
beside the video (`filme.srt`, `filme.pt-BR.srt`), or one chosen — speed
from 0.25× to 2×, frame by frame. The controls go a moment after the
pointer stops while a video plays.

Old **Flash movies** (`.swf`) are not video: they are handed to
[Ruffle](https://ruffle.rs/), the Flash Player emulator, in a window of its
own (`ruffle` in `~/.local/bin` or on the PATH).

| Key | | Key | |
| --- | --- | --- | --- |
| Space | play, pause | ← → | 5 s back, on (Shift: 60 s) |
| ↑ ↓ | volume | M | mute |
| , . | frame back, on | [ ] Backspace | slower, faster, normal |
| PgUp PgDn | previous, next | F or F11 | full screen |

## Orbit-Edit

A text and code editor on [GtkSourceView](https://gitlab.gnome.org/GNOME/gtksourceview):
highlighting for hundreds of languages, line numbers, auto-indentation,
search and replace with regular expressions, the minimap. Of its own:

- **Several cursors.** Ctrl+D selects the word, then adds the next
  occurrence; Ctrl+click adds a cursor; Alt+drag selects a column, a cursor
  per line. What is typed at one goes in at all — dead keys and input
  methods too — and so do Backspace, Delete, Enter, the arrows, copy and
  paste (a line per cursor when the counts match). Escape: back to one.
- **Indentation.** Guides at every level, through blank lines; blocks fold
  by indentation from the arrow in the margin or Ctrl+Shift+[ / ]. A
  file's own indentation — tabs, or how many spaces — is read from it.
- **The folder** in a sidebar (Ctrl+B), and Ctrl+P to open a file of it by
  a few letters of its path.
- **The session.** Closing never asks: the tabs, their cursors and any
  unsaved text come back next time (`~/.local/state/orbit/edit/`).
- Ctrl+/ comments lines in or out; Ctrl+G goes to a line; the status line
  shows and changes the indentation and the language; UTF-8 or Latin-1
  and LF or CRLF are kept as found. Options in `~/.config/orbit/edit.yaml`.

## Orbit-Files

A file manager with [Yazi](https://yazi-rs.github.io/)'s speed in a window,
and the best of Thunar, Caja, Nautilus, Konqueror and Nemo.

- **Fast.** A folder is read on a thread — one `readdir` and one `lstat`
  per entry, the type guessed from the name — and fills in as it is read:
  fifty thousand files list and sort in about 0.2 s, the window never
  waits. A monitor keeps it current entry by entry.
- **Three views of one folder**, switched with Ctrl+1/2/3: a list with
  sortable columns; icons with thumbnails (the shared thumbnail cache, or
  the system's thumbnailers: images, videos, PDFs); and Yazi's columns —
  the parent folder, the folder, and a preview of the selection (images,
  text highlighted, a PDF's first page, video and audio to play, a
  folder's contents).
- **Tabs and two panes** (F3), F5 / F6 copy or move to the other pane.
- **Places**: home and the user's folders, GTK bookmarks, the trash, disks
  (mount, eject) and *Connect to server* — SMB, SFTP, FTP, WebDAV through
  GVfs, with the password asked for. Under "Devices", volumes only:
  folders a server mounts into home (NFS, bind mounts) are left out unless
  `devices: all` is set in `~/.config/orbit/files.yaml`. Below the places,
  the selection's thumbnail, name and size.
- **The mouse's back and forward buttons** go through the history.
- **Copy, move, trash, delete** on threads, with progress and a stop
  button; a file already there asks: skip, keep both, replace (or all).
  Cut, copy and paste go to and from Nautilus, Thunar, Nemo and PCManFM;
  drag and drop moves within a disk and copies across (Ctrl copies, Shift
  moves).
- **Rename** in place (F2), or many at once as Thunar does: find and
  replace with regular expressions, numbering from a pattern, text before
  or after, case — every new name shown before anything changes.
- **Actions** of your own in the context menu, as Thunar's and Nemo's
  ([`examples/orbit/files-actions.yaml`](examples/orbit/files-actions.yaml)),
  and **archives**: compress to zip, tar.gz, tar.xz, tar.zst or 7z; extract
  here.

| Key | | Key | |
| --- | --- | --- | --- |
| j k, ↓ ↑ | down, up | h l, ⌫ ⏎ | up a folder, open |
| gg G | first, last | Space | mark and go on |
| yy dd pp | copy, cut, paste | / or Ctrl+F | filter |
| Ctrl+C X V | copy, cut, paste | F2 | rename (many: bulk) |
| Delete | to the trash | Shift+Delete | delete for good |
| Alt+← → ↑ | back, forward, up | Ctrl+L | type a path or `smb://…` |
| Ctrl+H | hidden files | Ctrl+T, Ctrl+W | new tab, close tab |
| F3 | two panes | F5, F6 | copy, move to the other pane |

## Orbit-Term

A terminal whose prompt is part of the window: it stays at the top, and
outputs never mix with it. Each command's output fills the box below,
never wrapped — scrolled both ways — in its colours; the last 50 that
worked (exit code 0 and nothing on stderr) are listed on the right, a
click showing one again (`~/.local/state/orbit/term/`).

Commands run in one shell session that loads `~/.shell/*.sh` (or
`~/.bash_aliases`): `cd`, `export` and aliases hold from one command to the
next. Interactive programs — editors, `htop`, `less`, `ssh`, `sudo`… (the
built-in list plus `interactive:` in `term.yaml`), or any command run with
Ctrl+Enter — run in a real terminal (VTE) that takes the output's place and
the keyboard until they end. ↑ ↓ walk the commands run before, Ctrl+C
interrupts (twice: kills), Ctrl+L clears the output; `font:` in `term.yaml`
sets the font.

## Building and running

Requirements: Rust 1.92+, GTK 4.12+ and `gtk4-layer-shell` development files;
for Orbit-View also the `libseccomp` development files, glycin's loaders and
bubblewrap (its sandbox); for Orbit-Read the poppler-glib development
files; for Orbit-Play GStreamer's (core, base and bad plugins) and
`gstreamer1.0-libav`; for Orbit-Edit GtkSourceView 5's; for Orbit-Term VTE's
(GTK 4). Orbit-Files uses
GVfs for network shares and zip, unzip, tar and 7z for archives.
On Debian/Ubuntu:

```sh
sudo apt install libgtk-4-dev libgtk4-layer-shell-dev libseccomp-dev glycin-loaders bubblewrap libpoppler-glib-dev \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev libgstreamer-plugins-bad1.0-dev gstreamer1.0-libav \
  libgtksourceview-5-dev libvte-2.91-gtk4-dev gvfs-backends p7zip-full unzip zip
cargo build --release
./target/release/orbit-panel &
./target/release/orbit-dock &
```

Start the panel first. Each surface reserves its edge (layer-shell exclusive
zone) and the compositor lays them out in the order they appear: a left
panel started first keeps the full height, and a top dock started after it
takes the top edge to the right of the panel.

To install into `~/.local/bin` and start both in the right order from the
compositor's configuration:

```sh
scripts/install.sh                 # builds; installs orbit-panel, orbit-dock, orbit-calc, orbit-clip, orbit-blink, orbit-view, orbit-read, orbit-play, orbit-edit, orbit-files, orbit-session
orbit-session                      # (re)starts the panel, then the dock
orbit-session --fallback 'eww …'   # runs another panel if Orbit cannot start
```

In sway, e.g. `exec ~/.local/bin/orbit-session` — and set `gaps top`/`gaps
left` to 0 if they were reserving room for another panel: Orbit reserves its
own.

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

## Colours and languages

**Colours.** The panel and the dock use a dark or a light palette
([`data/palette-dark.css`](data/palette-dark.css),
[`data/palette-light.css`](data/palette-light.css)) following the system's
`color-scheme`, and the system's `accent-color` when it has one — both read
from the XDG desktop portal, and changed live. With no preference, the GTK
theme decides (`GTK_THEME`, else the configured theme; "…-dark" means dark).
Every colour is a named `@orbit_*` colour that `style.css` can redefine.

**Languages.** Text is translated with [Fluent](https://projectfluent.org/):
one file per language in [`data/i18n/`](data/i18n), English
([`en-US.ftl`](data/i18n/en-US.ftl)) being the source and the fallback. The
language comes from `LC_ALL`, `LC_MESSAGES` or `LANG`. To translate Orbit,
copy `en-US.ftl` to your locale (e.g. `de-DE.ftl`), translate the text after
each `=`, and add the file to `CATALOGS` in
[`crates/orbit-app/src/i18n.rs`](crates/orbit-app/src/i18n.rs); a test checks
that every catalog has every message. Dates, weekdays and months already
follow the locale.

## Project layout

| Crate | Purpose |
| --- | --- |
| `orbit-common` | Configuration, CSS loading, display-backend abstraction |
| `orbit-backend-wayland` | Wayland backend: layer-shell |
| `orbit-backend-x11` | X11 backend: EWMH dock type, struts and positioning |
| `orbit-app` | Startup shared by the panel, the dock and the applications; icons, theme, translations |
| `orbit-panel` | The panel and its modules |
| `orbit-dock` | The dock |
| `orbit-calc` | The calculator |
| `orbit-clip` | The clipboard history |
| `orbit-blink` | The screen capture |
| `orbit-view` | The image viewer |
| `orbit-read` | The document reader |
| `orbit-play` | The media player |
| `orbit-edit` | The text editor |
| `orbit-files` | The file manager |

The plan for the panel (in Portuguese) is in
[`docs/PLANO-PAINEL.md`](docs/PLANO-PAINEL.md).

## Contributing

Ideas and feedback are welcome. The project is young: please open an issue to
discuss larger changes before a pull request.

## License

Orbit Rack is licensed under the
[GNU General Public License, version 3 or any later version](LICENSE).

`data/pathfinder/recipes` holds the recipes of
[ferdium-recipes](https://github.com/ferdium/ferdium-recipes), under the MIT
license ([its text](data/pathfinder/recipes/LICENSE.md)). The block lists
(EasyList, EasyPrivacy) are not shipped: Orbit-Pathfinder downloads them.
