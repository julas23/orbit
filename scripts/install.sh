#!/usr/bin/env bash
# install.sh — builds Orbit-Panel, Orbit-Dock, Orbit-Notify and the Orbit applications
# (Calc, Clip, Blink, View, Read, Play, Edit, Files, Pathfinder, Term) and installs them, with
# orbit-session, into ~/.local/bin (or PREFIX/bin); the applications' menu
# entries go to ~/.local/share/applications (or PREFIX/share/...).
#
#   scripts/install.sh            # ~/.local/bin
#   PREFIX=/usr/local scripts/install.sh
#
# Configuration is not touched: copy examples/orbit/ to ~/.config/orbit/ to
# start from the examples.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
DEST=${PREFIX:-$HOME/.local}/bin
APPS=${PREFIX:-$HOME/.local}/share/applications

cargo build --release --manifest-path "$ROOT/Cargo.toml" -p orbit-panel -p orbit-dock -p orbit-notify -p orbit-calc -p orbit-clip -p orbit-blink -p orbit-view -p orbit-read -p orbit-play -p orbit-edit -p orbit-files -p orbit-pathfinder -p orbit-term -p orbitctl

mkdir -p "$DEST"
# install(1) replaces the file instead of writing into it, so a running
# panel keeps its old binary until restarted.
install -m 755 "$ROOT/target/release/orbit-panel" "$DEST/orbit-panel"
install -m 755 "$ROOT/target/release/orbit-dock" "$DEST/orbit-dock"
install -m 755 "$ROOT/target/release/orbit-notify" "$DEST/orbit-notify"
install -m 755 "$ROOT/target/release/orbit-calc" "$DEST/orbit-calc"
install -m 755 "$ROOT/target/release/orbit-clip" "$DEST/orbit-clip"
install -m 755 "$ROOT/target/release/orbit-blink" "$DEST/orbit-blink"
install -m 755 "$ROOT/target/release/orbit-view" "$DEST/orbit-view"
install -m 755 "$ROOT/target/release/orbit-read" "$DEST/orbit-read"
install -m 755 "$ROOT/target/release/orbit-play" "$DEST/orbit-play"
install -m 755 "$ROOT/target/release/orbit-edit" "$DEST/orbit-edit"
install -m 755 "$ROOT/target/release/orbit-files" "$DEST/orbit-files"
install -m 755 "$ROOT/target/release/orbit-pathfinder" "$DEST/orbit-pathfinder"
install -m 755 "$ROOT/target/release/orbit-term" "$DEST/orbit-term"
# orbitctl talks to sway as well as to Orbit-Core.
install -m 755 "$ROOT/target/release/orbitctl" "$DEST/orbitctl"
install -m 755 "$ROOT/scripts/orbit-session" "$DEST/orbit-session"
mkdir -p "$APPS"
install -m 644 "$ROOT/data/io.github.julas23.Orbit.Calc.desktop" "$APPS/"
install -m 644 "$ROOT/data/io.github.julas23.Orbit.Clip.desktop" "$APPS/"
install -m 644 "$ROOT/data/io.github.julas23.Orbit.Blink.desktop" "$APPS/"
install -m 644 "$ROOT/data/io.github.julas23.Orbit.View.desktop" "$APPS/"
install -m 644 "$ROOT/data/io.github.julas23.Orbit.Read.desktop" "$APPS/"
install -m 644 "$ROOT/data/io.github.julas23.Orbit.Play.desktop" "$APPS/"
install -m 644 "$ROOT/data/io.github.julas23.Orbit.Edit.desktop" "$APPS/"
install -m 644 "$ROOT/data/io.github.julas23.Orbit.Files.desktop" "$APPS/"
install -m 644 "$ROOT/data/io.github.julas23.Orbit.Pathfinder.desktop" "$APPS/"
install -m 644 "$ROOT/data/io.github.julas23.Orbit.Term.desktop" "$APPS/"
# The Ferdium recipes Pathfinder reads unread counts with (MIT, see
# data/pathfinder/recipes/LICENSE.md); your own go in
# ~/.local/share/orbit/pathfinder/recipes and win over these.
RECIPES=${PREFIX:-$HOME/.local}/share/orbit-pathfinder/recipes
rm -rf "$RECIPES"
mkdir -p "$(dirname "$RECIPES")"
cp -r "$ROOT/data/pathfinder/recipes" "$RECIPES"

echo "installed orbit-panel, orbit-dock, orbit-notify, orbit-calc, orbit-clip, orbit-blink, orbit-view, orbit-read, orbit-play, orbit-edit, orbit-files, orbit-pathfinder, orbit-term, orbitctl and orbit-session in $DEST"
echo "and the Orbit-Calc, Orbit-Clip, Orbit-Blink, Orbit-View, Orbit-Read, Orbit-Play, Orbit-Edit and Orbit-Files menu entries in $APPS"
command -v update-desktop-database >/dev/null && update-desktop-database -q "$APPS" || true
echo "start them with: orbit-session"
