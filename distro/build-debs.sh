#!/usr/bin/env bash
# build-debs.sh — builds Orbit's packages: one .deb per program (cargo-deb,
# from each crate's [package.metadata.deb]), the metapackages in
# distro/meta/ and orbit-defaults (distro/defaults/), into target/debian/.
# With --arm64, also the arm64 ones (distro/ci/build-arm64.sh, in Docker,
# its toolchain and build kept in ~/.cache/orbit-arm64). With --upload
# HOST, puts them in the repository server's incoming folder
# (distro/repo/sync.sh publishes them).
#
#   distro/build-debs.sh [--arm64] [--upload ubuntu@rationalazimuth.com] [--all]
#
# With --upload, only the packages that changed since the ones in the
# incoming folder are built and sent (distro/changed.py says which, and
# why); the others stay as they are, so an update brings only what
# changed. --all builds and sends every one, the folder made exactly this
# build (packages no longer built leave it).
#
# Versions: the workspace's, then +git<commit date>.<commit>, so every build
# of a newer commit is an upgrade (and a build with uncommitted changes says
# so: .dirty).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
UPLOAD=""
ARM64=""
ALL=""
INCOMING=/srv/orbit-incoming/orbit
while [ $# -gt 0 ]; do
    case "$1" in
        --arm64) ARM64=1 ;;
        --upload) UPLOAD=${2:?--upload needs a host}; shift ;;
        --all) ALL=1 ;;
        *) echo "unknown option $1" >&2; exit 1 ;;
    esac
    shift
done

CRATES=(orbit-core orbit-login orbit-greet orbit-polkit orbit-prompter orbit-run orbit-settings orbit-wallpaper orbitctl orbit-panel orbit-dock orbit-notify
        orbit-calc orbit-clip orbit-blink orbit-view orbit-read orbit-play orbit-edit orbit-files orbit-pathfinder orbit-term orbit-safe
        orbit-remote-server orbit-local-station orbit-aigent)
BASE=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
VERSION="$BASE+git$(git log -1 --format=%cd --date=format:%Y%m%d%H%M).$(git rev-parse --short HEAD)"
git diff --quiet HEAD -- crates data scripts Cargo.toml Cargo.lock || VERSION="$VERSION.dirty"
OUT="$ROOT/target/debian"
rm -rf "$OUT"
mkdir -p "$OUT"
echo "Orbit $VERSION"

# Only what changed, when sending to a repository (not with --all).
PLAN=""
if [ -n "$UPLOAD" ] && [ -z "$ALL" ]; then
    PUBLISHED=$(mktemp)
    trap 'rm -f "$PUBLISHED"' EXIT
    ssh -o BatchMode=yes "${UPLOAD}" ls "$INCOMING" > "$PUBLISHED"
    PLAN=$(python3 "$ROOT/distro/changed.py" "$PUBLISHED" "${CRATES[@]}")
    if [ -z "$PLAN" ]; then
        echo "Nothing changed since the packages published."
        exit 0
    fi
    echo "To build (the rest stays as published):"
    sed 's/^/  /; s/\t/: /' <<< "$PLAN"
    kept=()
    for crate in "${CRATES[@]}"; do
        cut -f1 <<< "$PLAN" | grep -qx "$crate" && kept+=("$crate")
    done
    CRATES=("${kept[@]}")
fi
# Whether `name` is to be built; whether the package `file` (always the
# same, as orbit-fonts) is not published yet.
want() { [ -z "$PLAN" ] || cut -f1 <<< "$PLAN" | grep -qx "$1"; }
missing() { [ -z "$PLAN" ] || ! grep -qx "$1" "$PUBLISHED"; }

if [ ${#CRATES[@]} -gt 0 ]; then
    cargo build --release --locked $(printf -- '-p %s ' "${CRATES[@]}")
fi
for crate in "${CRATES[@]}"; do
    cargo deb -p "$crate" --no-build --deb-version "$VERSION" -o "$OUT/" -q
done
# Every program in them starts as a program does (ELF, or #! for a
# script): orbit-calc 4089bd8 went out with its first 64 bytes zeroed, and
# `exec` refused it ("Exec format error").
check_debs() {
    local deb dir bin head bad=0
    for deb in "$OUT"/*.deb; do
        [ -e "$deb" ] || continue
        dir=$(mktemp -d)
        dpkg-deb -x "$deb" "$dir"
        for bin in "$dir"/usr/bin/* "$dir"/usr/libexec/*; do
            [ -f "$bin" ] && [ ! -L "$bin" ] || continue
            head=$(head -c4 "$bin" | od -An -tx1 | tr -d ' \n')
            case "$head" in
                7f454c46|2321*) ;;
                *) echo "$(basename "$deb"): ${bin#"$dir"} is not a program (starts $head)" >&2; bad=1 ;;
            esac
        done
        rm -rf "$dir"
    done
    return $bad
}

# Metapackages: distro/meta/<name> holds the control fields that differ.
for meta in "$ROOT"/distro/meta/*; do
    name=$(basename "$meta")
    want "$name" || continue
    dir=$(mktemp -d)
    mkdir -p "$dir/DEBIAN"
    {
        echo "Package: $name"
        echo "Version: $VERSION"
        echo "Architecture: all"
        echo "Maintainer: Juliano <julianoas@gmail.com>"
        echo "Section: metapackages"
        echo "Priority: optional"
        echo "Homepage: https://github.com/julas23/orbit"
        cat "$meta"
    } > "$dir/DEBIAN/control"
    dpkg-deb --root-owner-group -Zxz --build "$dir" "$OUT/${name}_${VERSION}_all.deb" >/dev/null
    rm -rf "$dir"
done
# orbit-defaults: files, not code (distro/defaults/).
if want orbit-defaults; then
dir=$(mktemp -d)
mkdir -p "$dir/DEBIAN" "$dir/usr/share/applications" "$dir/usr/share/glib-2.0/schemas" "$dir/usr/share/doc/orbit" \
    "$dir/usr/share/xdg-desktop-portal"
{
    echo "Package: orbit-defaults"
    echo "Version: $VERSION"
    echo "Architecture: all"
    echo "Maintainer: Juliano <julianoas@gmail.com>"
    echo "Priority: optional"
    echo "Homepage: https://github.com/julas23/orbit"
    cat "$ROOT/distro/defaults/control"
} > "$dir/DEBIAN/control"
"$ROOT/distro/defaults/mimeapps.sh" "$ROOT/data" > "$dir/usr/share/applications/orbit-mimeapps.list"
cp "$ROOT/data/gsettings/90_orbit.gschema.override" "$dir/usr/share/glib-2.0/schemas/"
cp "$ROOT/data/portals/orbit-portals.conf" "$dir/usr/share/xdg-desktop-portal/"
cp -r "$ROOT/examples" "$dir/usr/share/doc/orbit/examples"
chmod -R u=rwX,go=rX "$dir/usr"
dpkg-deb --root-owner-group -Zxz --build "$dir" "$OUT/orbit-defaults_${VERSION}_all.deb" >/dev/null
rm -rf "$dir"
fi
# orbit-release: Orbit Linux's name, version and logo (distro/release/),
# for the image only. Its version is the system's: the workspace's
# major.minor; BUILD_ID is this build's.
REL="$ROOT/distro/release"
OS_VERSION=$(echo "$BASE" | cut -d. -f1,2)
BUILD_ID="$(git log -1 --format=%cd --date=format:%Y%m%d).$(git rev-parse --short HEAD)"
if want orbit-release; then
dir=$(mktemp -d)
mkdir -p "$dir/DEBIAN" "$dir/usr/lib" "$dir/etc/xdg/fastfetch" "$dir/etc/default/grub.d" "$dir/etc/skel/.config/neofetch" \
    "$dir/usr/share/orbit/ascii" "$dir/usr/share/icons/hicolor/scalable/apps" "$dir/usr/share/doc/orbit-release" \
    "$dir/usr/share/plymouth/themes"
{
    echo "Package: orbit-release"
    echo "Version: $VERSION"
    echo "Architecture: all"
    echo "Maintainer: Juliano <julianoas@gmail.com>"
    echo "Priority: optional"
    echo "Homepage: https://github.com/julas23/orbit"
    cat "$REL/control"
} > "$dir/DEBIAN/control"
cp "$REL/preinst" "$REL/postinst" "$REL/postrm" "$REL/triggers" "$dir/DEBIAN/"
printf '%s\n' /etc/xdg/fastfetch/config.jsonc /etc/default/grub.d/orbit-release.cfg /etc/skel/.config/neofetch/config.conf > "$dir/DEBIAN/conffiles"
fill() { sed -e "s/@VERSION@/$OS_VERSION/g" -e "s/@BUILD@/$BUILD_ID/g" "$1"; }
fill "$REL/os-release.in" > "$dir/usr/lib/os-release"
fill "$REL/issue.in" > "$dir/etc/issue"
fill "$REL/issue.net.in" > "$dir/etc/issue.net"
cp "$REL/motd" "$dir/etc/motd"
cp "$REL/fastfetch.jsonc" "$dir/etc/xdg/fastfetch/config.jsonc"
cp "$REL/grub.cfg" "$dir/etc/default/grub.d/orbit-release.cfg"
cp "$REL/neofetch.conf" "$dir/etc/skel/.config/neofetch/config.conf"
cp "$REL"/orbit-linux.fastfetch "$REL"/orbit-linux.neofetch "$REL"/orbit-linux.txt "$dir/usr/share/orbit/ascii/"
cp "$ROOT/data/brand/orbit-mark.svg" "$dir/usr/share/icons/hicolor/scalable/apps/orbit-linux.svg"
cp -r "$REL/plymouth/orbit" "$dir/usr/share/plymouth/themes/"
cp "$ROOT/LICENSE" "$dir/usr/share/doc/orbit-release/copyright" 2>/dev/null || true
chmod -R u=rwX,go=rX "$dir/usr" "$dir/etc"
chmod 755 "$dir" "$dir/DEBIAN/preinst" "$dir/DEBIAN/postinst" "$dir/DEBIAN/postrm"
dpkg-deb --root-owner-group -Zxz --build "$dir" "$OUT/orbit-release_${VERSION}_all.deb" >/dev/null
rm -rf "$dir"
fi
# orbit-fonts: the Nerd Font the panel and dock draw with, from its
# release; its own version, the same file every build (fixed dates).
FONTS_VERSION=3.5.1
FONTS_SHA256=ae598e9401e2846aa3ee364513715de0490b844a4b54f0768991f45f23aa8369
fonts="$ROOT/target/fonts/CascadiaCode-$FONTS_VERSION.tar.xz"
if missing "orbit-fonts_${FONTS_VERSION}-1_all.deb"; then
if ! echo "$FONTS_SHA256  $fonts" | sha256sum -c --status 2>/dev/null; then
    mkdir -p "$(dirname "$fonts")"
    curl -fsSL -o "$fonts" "https://github.com/ryanoasis/nerd-fonts/releases/download/v$FONTS_VERSION/CascadiaCode.tar.xz"
    echo "$FONTS_SHA256  $fonts" | sha256sum -c --status || { echo "the Nerd Font's download is not the one expected" >&2; exit 1; }
fi
dir=$(mktemp -d)
mkdir -p "$dir/DEBIAN" "$dir/usr/share/fonts/truetype/orbit" "$dir/usr/share/doc/orbit-fonts"
tar -xJf "$fonts" -C "$dir/usr/share/fonts/truetype/orbit" --wildcards 'CaskaydiaCoveNerdFont-*.ttf'
tar -xJOf "$fonts" LICENSE > "$dir/usr/share/doc/orbit-fonts/copyright"
# Its README lists each glyph set's licence.
tar -xJOf "$fonts" README.md > "$dir/usr/share/doc/orbit-fonts/README.md"
{
    echo "Package: orbit-fonts"
    echo "Version: $FONTS_VERSION-1"
    echo "Architecture: all"
    echo "Maintainer: Juliano <julianoas@gmail.com>"
    echo "Priority: optional"
    echo "Homepage: https://www.nerdfonts.com"
    cat "$ROOT/distro/fonts/control"
} > "$dir/DEBIAN/control"
chmod -R u=rwX,go=rX "$dir"
find "$dir" -exec touch -h -d @1755734400 {} +
SOURCE_DATE_EPOCH=1755734400 dpkg-deb --root-owner-group -Zxz --build "$dir" "$OUT/orbit-fonts_${FONTS_VERSION}-1_all.deb" >/dev/null
rm -rf "$dir"
fi
# orbit-ruffle: Ruffle's own build, as orbit-fonts is made.
RUFFLE_NIGHTLY=2026-10-20
RUFFLE_SHA256=4b9031f38d029dea0fdab69c5ad8886507a4bce753eb7d18fa1bdc4ba9a4bc2c
ruffle="$ROOT/target/ruffle/ruffle-$RUFFLE_NIGHTLY.tar.gz"
if missing "orbit-ruffle_0.8.0~nightly${RUFFLE_NIGHTLY//-/}-1_amd64.deb"; then
if ! echo "$RUFFLE_SHA256  $ruffle" | sha256sum -c --status 2>/dev/null; then
    mkdir -p "$(dirname "$ruffle")"
    curl -fsSL -o "$ruffle" "https://github.com/ruffle-rs/ruffle/releases/download/nightly-$RUFFLE_NIGHTLY/ruffle-nightly-${RUFFLE_NIGHTLY//-/_}-linux-x86_64.tar.gz"
    echo "$RUFFLE_SHA256  $ruffle" | sha256sum -c --status || { echo "Ruffle's download is not the one expected" >&2; exit 1; }
fi
dir=$(mktemp -d)
mkdir -p "$dir/DEBIAN" "$dir/usr/bin" "$dir/usr/share/applications" "$dir/usr/share/metainfo" \
    "$dir/usr/share/icons/hicolor/scalable/apps" "$dir/usr/share/doc/orbit-ruffle"
tar -xzOf "$ruffle" ruffle > "$dir/usr/bin/ruffle"
tar -xzOf "$ruffle" extras/rs.ruffle.Ruffle.desktop > "$dir/usr/share/applications/rs.ruffle.Ruffle.desktop"
tar -xzOf "$ruffle" extras/rs.ruffle.Ruffle.metainfo.xml > "$dir/usr/share/metainfo/rs.ruffle.Ruffle.metainfo.xml"
tar -xzOf "$ruffle" extras/rs.ruffle.Ruffle.svg > "$dir/usr/share/icons/hicolor/scalable/apps/rs.ruffle.Ruffle.svg"
tar -xzOf "$ruffle" LICENSE.md > "$dir/usr/share/doc/orbit-ruffle/copyright"
{
    echo "Package: orbit-ruffle"
    echo "Version: 0.8.0~nightly${RUFFLE_NIGHTLY//-/}-1"
    echo "Architecture: amd64"
    echo "Maintainer: Juliano <julianoas@gmail.com>"
    echo "Priority: optional"
    echo "Homepage: https://ruffle.rs"
    echo "Provides: ruffle"
    cat "$ROOT/distro/ruffle/control"
} > "$dir/DEBIAN/control"
chmod -R u=rwX,go=rX "$dir"
chmod 755 "$dir/usr/bin/ruffle"
find "$dir" -exec touch -h -d @1760918400 {} +
SOURCE_DATE_EPOCH=1760918400 dpkg-deb --root-owner-group -Zxz --build "$dir" "$OUT/orbit-ruffle_0.8.0~nightly${RUFFLE_NIGHTLY//-/}-1_amd64.deb" >/dev/null
rm -rf "$dir"
fi

# arm64: cross-compiled in a bare Debian 13, from this same tree (so with
# the same version).
if [ -n "$ARM64" ] && want orbit-local-station:arm64; then
    # ORBIT_ARM64_CACHE: elsewhere, e.g. on a machine whose container root
    # cannot write the one another machine's root made (an LXC).
    CACHE=${ORBIT_ARM64_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/orbit-arm64}
    mkdir -p "$CACHE"
    docker run --rm -v "$ROOT:/src:ro" -v "$OUT:/out" -v "$CACHE:/cache" \
        -e CARGO_HOME=/cache/cargo -e RUSTUP_HOME=/cache/rustup -e CARGO_TARGET_DIR=/cache/target \
        debian:trixie /src/distro/ci/build-arm64.sh
fi
check_debs || { echo "broken packages; nothing sent" >&2; exit 1; }
ls -1 "$OUT"

if [ -n "$UPLOAD" ]; then
    if [ -z "$PLAN" ]; then
        rsync -a --delete "$OUT/" "$UPLOAD:$INCOMING/"
    else
        # Added beside the others; of these, the versions they replace go.
        rsync -a "$OUT/" "$UPLOAD:$INCOMING/"
        ssh -o BatchMode=yes "$UPLOAD" "cd $INCOMING && for new in $(cd "$OUT" && ls -1 *.deb | tr '\n' ' '); do
            name=\${new%%_*}; arch=\${new##*_}
            for old in \${name}_*_\$arch; do [ \"\$old\" = \"\$new\" ] || rm -f -- \"\$old\"; done
        done"
    fi
    echo "uploaded to $UPLOAD; run distro/repo/sync.sh there to publish"
    [ -z "$PLAN" ] || { echo "Updated:"; ls -1 "$OUT" | sed 's/^/  /'; }
fi
