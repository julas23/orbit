#!/usr/bin/env bash
# build-arm64.sh — the arm64 packages, cross-compiled on a bare Debian 13
# (amd64): today only orbit-local-station, for stations such as the
# Raspberry Pi 4. Run the same way as build.sh:
#
#   docker run --rm -v "$PWD:/src:ro" -v /tmp/orbit-debs:/out debian:trixie /src/distro/ci/build-arm64.sh
#
# The arm64 libraries come from Debian's multiarch; the version is
# build-debs.sh's, so both architectures of a commit go out together.
# CARGO_HOME, RUSTUP_HOME and CARGO_TARGET_DIR on a kept volume spare the
# toolchain and the dependencies' build the next time (build-debs.sh
# --arm64 does so).
set -euo pipefail

SRC=${SRC:-/src}
OUT=${OUT:-/out}
CRATES=(orbit-local-station)
TARGET=aarch64-unknown-linux-gnu
export DEBIAN_FRONTEND=noninteractive

dpkg --add-architecture arm64
apt-get update -qq
apt-get install -y -qq --no-install-recommends \
    ca-certificates curl git rsync pkg-config build-essential crossbuild-essential-arm64 \
    libgstreamer1.0-dev:arm64 libgstreamer-plugins-base1.0-dev:arm64 libgstreamer-plugins-bad1.0-dev:arm64 \
    libseat-dev:arm64 libinput-dev:arm64 libudev-dev:arm64 libgbm-dev:arm64 libdrm-dev:arm64 \
    libxkbcommon-dev:arm64 libwayland-dev:arm64 libfontconfig-dev:arm64 libfreetype-dev:arm64 \
    libegl-dev:arm64 libgles-dev:arm64 \
    >/dev/null

if ! command -v cargo >/dev/null; then
    curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable >/dev/null
fi
# shellcheck disable=SC1091
. "${CARGO_HOME:-$HOME/.cargo}/env"
rustup target add "$TARGET" >/dev/null
command -v cargo-deb >/dev/null || env -u CARGO_TARGET_DIR cargo install --locked cargo-deb >/dev/null

export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
export PKG_CONFIG_ALLOW_CROSS=1
export PKG_CONFIG_LIBDIR=/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig
export PKG_CONFIG_SYSROOT_DIR=/

WORK=$(mktemp -d)
rsync -a --exclude target "$SRC/" "$WORK/"
git config --global --add safe.directory "$WORK"
cd "$WORK"
BASE=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
VERSION="$BASE+git$(git log -1 --format=%cd --date=format:%Y%m%d%H%M).$(git rev-parse --short HEAD)"
git diff --quiet HEAD -- crates data scripts Cargo.toml Cargo.lock || VERSION="$VERSION.dirty"
echo "Orbit $VERSION (arm64)"

cargo build --release --locked --target "$TARGET" $(printf -- '-p %s ' "${CRATES[@]}")
mkdir -p "$OUT"
for crate in "${CRATES[@]}"; do
    # cargo-deb finds the assets (target/release/) under target/$TARGET/.
    cargo deb -p "$crate" --no-build --target "$TARGET" --deb-version "$VERSION" -o "$OUT/" -q
done
# Owned as the folder they land in, not by the container's root.
chown "$(stat -c %u:%g "$OUT")" "$OUT"/*_arm64.deb
ls -1 "$OUT"/*_arm64.deb
