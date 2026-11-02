#!/bin/sh
# Makes the boot menu's images and GRUB fonts from brand/splash.svg and Inter.
# Run on the build VM (needs librsvg2-bin, fonts-inter, grub-common):
#   sh brand/make-assets.sh   (from distro/iso)
set -e
cd "$(dirname "$0")/.."
G=config/bootloaders/grub-pc
rsvg-convert -w 1024 -h 768 brand/splash.svg -o $G/splash.png
rsvg-convert -w 640 -h 480 brand/splash.svg -o config/bootloaders/syslinux_common/splash.png
INTER=$(fc-match -f '%{file}' 'Inter:style=Regular')
INTER_SEMI=$(fc-match -f '%{file}' 'Inter:style=SemiBold')
grub-mkfont -s 18 -n "Orbit UI" -o $G/live-theme/orbit-18.pf2 "$INTER" 2>/dev/null
grub-mkfont -s 22 -n "Orbit UI Bold" -o $G/live-theme/orbit-22.pf2 "$INTER_SEMI" 2>/dev/null
grub-mkfont -s 14 -n "Orbit UI Small" -o $G/live-theme/orbit-14.pf2 "$INTER" 2>/dev/null
for f in $G/live-theme/*.pf2; do printf '%s: ' "$f"; head -c 64 "$f" | tr -c '[:print:]' ' '; echo; done
