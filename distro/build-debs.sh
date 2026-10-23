#!/usr/bin/env bash
# build-debs.sh — builds Orbit's packages: one .deb per program (cargo-deb,
# from each crate's [package.metadata.deb]) and the metapackages in
# distro/meta/, into target/debian/. With --upload HOST, puts them in the
# repository server's incoming folder (distro/repo/sync.sh publishes them).
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
ls -1 "$OUT"

if [ -n "$UPLOAD" ]; then
    rsync -a --delete "$OUT/" "$UPLOAD:/srv/orbit-incoming/orbit/"
    echo "uploaded to $UPLOAD; run distro/repo/sync.sh there to publish"
fi
