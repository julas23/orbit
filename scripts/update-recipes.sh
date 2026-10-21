#!/usr/bin/env bash
# update-recipes.sh — refreshes the Ferdium recipes Orbit-Pathfinder ships
# (data/pathfinder/recipes) from github.com/ferdium/ferdium-recipes (MIT).
# Only what runs in the page is kept: package.json, webview.js, the CSS and
# the page scripts; not the icons nor index.js (Electron's side).
#
#   scripts/update-recipes.sh
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
DEST=$ROOT/data/pathfinder/recipes
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

git clone -q --depth 1 https://github.com/ferdium/ferdium-recipes.git "$TMP/repo"
rm -rf "$DEST"
mkdir -p "$DEST"
cp "$TMP/repo/LICENSE.md" "$DEST/LICENSE.md"
for dir in "$TMP"/repo/recipes/*/; do
  id=$(basename "$dir")
  [ -f "$dir/package.json" ] && [ -f "$dir/webview.js" ] || continue
  mkdir -p "$DEST/$id"
  find "$dir" -maxdepth 1 -type f \( -name package.json -o -name webview.js -o -name '*.css' -o -name '*-unsafe.js' \) \
    -exec cp {} "$DEST/$id/" \;
done
echo "$(git -C "$TMP/repo" log -1 --format='%H %cs')" > "$DEST/VERSION"
echo "recipes: $(ls -d "$DEST"/*/ | wc -l) from ferdium-recipes $(cut -c1-8 "$DEST/VERSION")"
