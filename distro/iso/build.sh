#!/bin/sh
# Builds the Orbit Linux ISO on the build VM (Proxmox VM 910, orbit-build)
# and leaves it in the VM's ~/iso-out. live-build needs root and a chroot,
# so it never runs on a workstation.
#   distro/iso/build.sh [orbit@10.0.0.56]
# ORBIT_ISO_SSH_KEY=~/.ssh/id_rsa.pub makes a debugging image: openssh-server
# and that key for the live user. Never for an image handed to anyone.
# ORBIT_ISO_TEST=1 makes an image whose installer asks nothing: user teste,
# password orbit-teste, the whole first disk. For a test VM only.
set -e
HOST="${1:-orbit@10.0.0.56}"
HERE="$(cd "$(dirname "$0")" && pwd)"
VERSION="$(git -C "$HERE" describe --always --dirty)"

# The sources go to ~/iso-src as the user; live-build works as root in
# /srv/orbit-iso, where its cache of packages stays between builds.
rsync -a --delete --exclude build.log "$HERE/" "$HOST:iso-src/"
DEBUG_STEP=":"
if [ -n "${ORBIT_ISO_SSH_KEY:-}" ]; then
	scp -q "$ORBIT_ISO_SSH_KEY" "$HOST:/tmp/iso-debug-$$.pub"
	DEBUG_STEP="sudo mkdir -p config/includes.chroot_after_packages/etc/skel/.ssh
	sudo install -m 600 /tmp/iso-debug-$$.pub config/includes.chroot_after_packages/etc/skel/.ssh/authorized_keys
	echo openssh-server | sudo tee config/package-lists/debug.list.chroot >/dev/null"
	VERSION="$VERSION-debug"
fi
TEST_STEP=":"
if [ -n "${ORBIT_ISO_TEST:-}" ]; then
	TEST_STEP="sudo tee -a config/includes.installer/preseed.cfg >/dev/null <<'PRESEED'
# --- test image: nothing asked ---
d-i netcfg/get_hostname string orbit-teste
d-i netcfg/get_hostname seen true
d-i passwd/user-fullname string Teste
d-i passwd/username string teste
d-i passwd/user-password password orbit-teste
d-i passwd/user-password-again password orbit-teste
d-i user-setup/allow-password-weak boolean true
d-i partman-auto/disk string /dev/sda
d-i partman-auto/method string regular
d-i partman-partitioning/confirm_write_new_label boolean true
d-i partman/choose_partition select finish
d-i partman/confirm boolean true
d-i partman/confirm_nooverwrite boolean true
PRESEED"
	VERSION="$VERSION-test"
fi
ssh "$HOST" "set -e
	sudo mkdir -p /srv/orbit-iso
	sudo rsync -a --delete --exclude /cache --exclude /chroot --exclude /binary --exclude /.build ~/iso-src/ /srv/orbit-iso/
	cd /srv/orbit-iso
	$DEBUG_STEP
	$TEST_STEP
	sudo lb clean >/dev/null 2>&1 || true
	sudo lb config >/dev/null
	sudo lb build
	mkdir -p ~/iso-out
	OUT=~/iso-out/orbit-linux-\$(date +%Y%m%d)-$VERSION-amd64.iso
	sudo mv live-image-amd64.hybrid.iso \$OUT
	sudo chown \$(id -u):\$(id -g) \$OUT
	sha256sum \$OUT > \$OUT.sha256
	ls -la \$OUT"
