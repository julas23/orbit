#!/bin/bash
# distro/ltsp/ipxe-menu.sh — LTSP's iPXE menu, with Orbit's stations only.
#
# Run as root on the station server (the build-station-image*.sh scripts
# call it). `ltsp ipxe` lists every image and chroot in /srv/ltsp; the
# menu keeps the PC's image, first and the default, and names the
# Raspberry Pi's second, as a line to read: a Pi boots by its own
# bootloader, never from this menu (an arm64 kernel on a PC would fail).
# The chroot entries go: booting the PC's raw chroot left it read-only.
set -euo pipefail

MENU=/srv/tftp/ltsp/ltsp.ipxe

ltsp ipxe
items="item --key 1 orbit-station        Orbit station (PC, amd64)"
if [ -d /srv/ltsp/orbit-station-rpi ]; then
    items="$items
item --gap                        Orbit station (Raspberry Pi 4, arm64): boots by itself on the Pi"
fi
# The image items, after their heading, replaced by ours.
awk -v items="$items" '
    /^item --key [0-9]+ / { next }
    { print }
    /^item --gap +Boot an image from the network/ { print items }
' "$MENU" >"$MENU.new"
mv "$MENU.new" "$MENU"
grep -n "^item" "$MENU" | head -4
