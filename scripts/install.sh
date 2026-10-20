#!/usr/bin/env bash
# install.sh — builds Orbit-Panel, Orbit-Dock, Orbit-Calc, Orbit-Clip,
# Orbit-Blink and Orbit-View and installs them, with orbit-session, into
# ~/.local/bin (or PREFIX/bin); the applications' menu entries go to
# ~/.local/share/applications (or PREFIX/share/...).
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

cargo build --release --manifest-path "$ROOT/Cargo.toml" -p orbit-panel -p orbit-dock -p orbit-calc -p orbit-clip -p orbit-blink -p orbit-view

mkdir -p "$DEST"
# install(1) replaces the file instead of writing into it, so a running
# panel keeps its old binary until restarted.
install -m 755 "$ROOT/target/release/orbit-panel" "$DEST/orbit-panel"
install -m 755 "$ROOT/target/release/orbit-dock" "$DEST/orbit-dock"
install -m 755 "$ROOT/target/release/orbit-calc" "$DEST/orbit-calc"
install -m 755 "$ROOT/target/release/orbit-clip" "$DEST/orbit-clip"
install -m 755 "$ROOT/target/release/orbit-blink" "$DEST/orbit-blink"
install -m 755 "$ROOT/target/release/orbit-view" "$DEST/orbit-view"
install -m 755 "$ROOT/scripts/orbit-session" "$DEST/orbit-session"
mkdir -p "$APPS"
install -m 644 "$ROOT/data/io.github.julas23.Orbit.Calc.desktop" "$APPS/"
install -m 644 "$ROOT/data/io.github.julas23.Orbit.Clip.desktop" "$APPS/"
install -m 644 "$ROOT/data/io.github.julas23.Orbit.Blink.desktop" "$APPS/"
install -m 644 "$ROOT/data/io.github.julas23.Orbit.View.desktop" "$APPS/"

echo "installed orbit-panel, orbit-dock, orbit-calc, orbit-clip, orbit-blink, orbit-view and orbit-session in $DEST"
echo "and the Orbit-Calc, Orbit-Clip, Orbit-Blink and Orbit-View menu entries in $APPS"
command -v update-desktop-database >/dev/null && update-desktop-database -q "$APPS" || true
echo "start them with: orbit-session"
