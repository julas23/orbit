#!/usr/bin/env bash
# install.sh — builds Orbit-Panel and Orbit-Dock and installs them, with
# orbit-session, into ~/.local/bin (or PREFIX/bin).
#
#   scripts/install.sh            # ~/.local/bin
#   PREFIX=/usr/local scripts/install.sh
#
# Configuration is not touched: copy examples/orbit/ to ~/.config/orbit/ to
# start from the examples.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
DEST=${PREFIX:-$HOME/.local}/bin

cargo build --release --manifest-path "$ROOT/Cargo.toml" -p orbit-panel -p orbit-dock

mkdir -p "$DEST"
# install(1) replaces the file instead of writing into it, so a running
# panel keeps its old binary until restarted.
install -m 755 "$ROOT/target/release/orbit-panel" "$DEST/orbit-panel"
install -m 755 "$ROOT/target/release/orbit-dock" "$DEST/orbit-dock"
install -m 755 "$ROOT/scripts/orbit-session" "$DEST/orbit-session"

echo "installed orbit-panel, orbit-dock and orbit-session in $DEST"
echo "start them with: orbit-session"
