#!/usr/bin/env bash
# build.sh — builds Orbit's packages on a bare Debian 13: what the CI runs
# (.github/workflows/debs.yml), and what can be tried the same way at home:
#
#   docker run --rm -v "$PWD:/src:ro" -v /tmp/orbit-debs:/out debian:trixie /src/distro/ci/build.sh
#
# The sources are copied first (the mount may be read-only); the packages
# land in /out (or $OUT). Rust comes from rustup, at the version Cargo.toml
# asks for at least: Debian's is older.
set -euo pipefail

SRC=${SRC:-/src}
OUT=${OUT:-/out}
export DEBIAN_FRONTEND=noninteractive

apt-get update -qq
apt-get install -y -qq --no-install-recommends \
    ca-certificates curl git build-essential pkg-config clang cmake mold rsync \
    libgtk-4-dev libgtk4-layer-shell-dev libseccomp-dev libpoppler-glib-dev \
    libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev libgstreamer-plugins-bad1.0-dev \
    libgtksourceview-5-dev libvte-2.91-gtk4-dev libwebkitgtk-6.0-dev \
    libseat-dev libinput-dev libudev-dev libgbm-dev libxkbcommon-dev libdrm-dev libsystemd-dev \
    libdisplay-info-dev libpixman-1-dev libssl-dev libsqlite3-dev libpam0g-dev \
    libfontconfig-dev libfreetype-dev libasound2-dev libpipewire-0.3-dev libdbus-1-dev \
    libxcb1-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libegl-dev libgles-dev \
    >/dev/null

if ! command -v cargo >/dev/null; then
    curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable >/dev/null
fi
# shellcheck disable=SC1091
. "$HOME/.cargo/env"
command -v cargo-deb >/dev/null || cargo install --locked cargo-deb >/dev/null

# A clean copy, its history kept: the versions come from the last commit.
WORK=$(mktemp -d)
rsync -a --exclude target "$SRC/" "$WORK/"
git config --global --add safe.directory "$WORK"
cd "$WORK"
distro/build-debs.sh
mkdir -p "$OUT"
cp target/debian/*.deb "$OUT/"
ls -1 "$OUT"
