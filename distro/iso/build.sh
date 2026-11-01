#!/bin/sh
# Builds the Orbit Linux ISO on the build VM (Proxmox VM 910, orbit-build)
# and leaves it in the VM's ~/iso-out. live-build needs root and a chroot,
# so it never runs on a workstation.
#   distro/iso/build.sh [orbit@10.0.0.56]
set -e
HOST="${1:-orbit@10.0.0.56}"
HERE="$(cd "$(dirname "$0")" && pwd)"
VERSION="$(git -C "$HERE" describe --always --dirty)"

# The sources go to ~/iso-src as the user; live-build works as root in
# /srv/orbit-iso, where its cache of packages stays between builds.
rsync -a --delete --exclude build.log "$HERE/" "$HOST:iso-src/"
ssh "$HOST" "set -e
	sudo mkdir -p /srv/orbit-iso
	sudo rsync -a --delete --exclude /cache --exclude /chroot --exclude /binary --exclude /.build ~/iso-src/ /srv/orbit-iso/
	cd /srv/orbit-iso
	sudo lb clean >/dev/null 2>&1 || true
	sudo lb config >/dev/null
	sudo lb build
	mkdir -p ~/iso-out
	OUT=~/iso-out/orbit-linux-\$(date +%Y%m%d)-$VERSION-amd64.iso
	sudo mv live-image-amd64.hybrid.iso \$OUT
	sudo chown \$(id -u):\$(id -g) \$OUT
	sha256sum \$OUT > \$OUT.sha256
	ls -la \$OUT"
