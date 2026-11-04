#!/bin/sh
# Makes the Plymouth theme's images from the mark (data/brand/orbit-mark.svg),
# in parts the script moves: the core with the closed outer orbit (still),
# the open inner orbit (turned), the moon (travelling the outer orbit) and
# the password bullet. Needs rsvg-convert or inkscape.
#   sh distro/release/plymouth/make-images.sh
set -e
cd "$(dirname "$0")/orbit"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
svg() { # name size viewbox body
    printf '<svg xmlns="http://www.w3.org/2000/svg" viewBox="%s" width="%s" height="%s">%s</svg>\n' "$3" "$2" "$2" "$4" > "$TMP/$1.svg"
    if command -v rsvg-convert >/dev/null; then
        rsvg-convert -w "$2" -h "$2" "$TMP/$1.svg" -o "$1.png"
    else
        inkscape "$TMP/$1.svg" -w "$2" -h "$2" -o "$1.png" >/dev/null 2>&1
    fi
}
# 4 px for each unit of the mark's 100-unit box.
svg base 400 "0 0 100 100" '<circle cx="50" cy="50" r="10" fill="#4682b4"/><circle cx="50" cy="50" r="38" fill="none" stroke="#d8dee9" stroke-width="6"/>'
svg inner 400 "0 0 100 100" '<circle cx="50" cy="50" r="24" fill="none" stroke="#6ea8e0" stroke-width="6" stroke-linecap="round" stroke-dasharray="100 50.8" transform="rotate(200 50 50)"/>'
svg moon 56 "0 0 14 14" '<circle cx="7" cy="7" r="6.5" fill="#ff9500"/>'
svg bullet 32 "0 0 10 10" '<circle cx="5" cy="5" r="4" fill="#d8dee9"/>'
