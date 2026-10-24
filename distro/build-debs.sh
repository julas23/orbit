#!/usr/bin/env bash
# build-debs.sh — builds Orbit's packages: one .deb per program (cargo-deb,
# from each crate's [package.metadata.deb]), the metapackages in
# distro/meta/ and orbit-defaults (distro/defaults/), into target/debian/.
# With --upload HOST, puts them in the repository server's incoming folder
# (distro/repo/sync.sh publishes them).
#
#   distro/build-debs.sh [--upload root@10.0.0.111]
#
# Versions: the workspace's, then +git<commit date>.<commit>, so every build
# of a newer commit is an upgrade (and a build with uncommitted changes says
# so: .dirty).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
UPLOAD=""
[ "${1:-}" = --upload ] && UPLOAD=${2:?--upload needs a host}

CRATES=(orbit-core orbit-login orbit-greet orbit-polkit orbit-prompter orbit-run orbitctl orbit-panel orbit-dock orbit-notify
        orbit-calc orbit-clip orbit-blink orbit-view orbit-read orbit-play orbit-edit orbit-files orbit-pathfinder orbit-term
        orbit-remote-server orbit-local-station orbit-aigent)
BASE=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
VERSION="$BASE+git$(git log -1 --format=%cd --date=format:%Y%m%d%H%M).$(git rev-parse --short HEAD)"
git diff --quiet HEAD -- crates data scripts Cargo.toml Cargo.lock || VERSION="$VERSION.dirty"
OUT="$ROOT/target/debian"
rm -rf "$OUT"
mkdir -p "$OUT"
echo "Orbit $VERSION"

cargo build --release --locked $(printf -- '-p %s ' "${CRATES[@]}")
for crate in "${CRATES[@]}"; do
    cargo deb -p "$crate" --no-build --deb-version "$VERSION" -o "$OUT/" -q
done

# Metapackages: distro/meta/<name> holds the control fields that differ.
for meta in "$ROOT"/distro/meta/*; do
    name=$(basename "$meta")
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
dir=$(mktemp -d)
mkdir -p "$dir/DEBIAN" "$dir/usr/share/applications" "$dir/usr/share/glib-2.0/schemas" "$dir/usr/share/doc/orbit"
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
cp -r "$ROOT/examples" "$dir/usr/share/doc/orbit/examples"
chmod -R u=rwX,go=rX "$dir/usr"
dpkg-deb --root-owner-group -Zxz --build "$dir" "$OUT/orbit-defaults_${VERSION}_all.deb" >/dev/null
rm -rf "$dir"
# orbit-fonts: the Nerd Font the panel and dock draw with, from its
# release; its own version, the same file every build (fixed dates).
FONTS_VERSION=3.5.1
FONTS_SHA256=ae598e9401e2846aa3ee364513715de0490b844a4b54f0768991f45f23aa8369
fonts="$ROOT/target/fonts/CascadiaCode-$FONTS_VERSION.tar.xz"
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
ls -1 "$OUT"

if [ -n "$UPLOAD" ]; then
    rsync -a --delete "$OUT/" "$UPLOAD:/srv/orbit-incoming/orbit/"
    echo "uploaded to $UPLOAD; run distro/repo/sync.sh there to publish"
fi
