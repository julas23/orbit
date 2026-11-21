#!/bin/sh
# Builds the Orbit Linux ISO, as root, on a Debian 13 system with
# live-build: the GitHub `iso` workflow runs it in a privileged debian:trixie
# container (.github/workflows/iso.yml). live-build needs root and a chroot,
# so it never runs on a workstation.
#   distro/iso/build.sh [OUTDIR]          OUTDIR: ./out by default
# The work happens in a copy of distro/iso, in ORBIT_ISO_WORK
# (/srv/orbit-iso by default), where live-build's cache of packages stays
# between builds on the same machine.
# ORBIT_ISO_SSH_KEY=path/to/key.pub makes a debugging image: openssh-server
# and that key for the live user. Never for an image handed to anyone.
# ORBIT_ISO_TEST=1 makes an image whose installer asks nothing: user teste,
# password orbit-teste, the whole first disk. For a test VM only.
# ORBIT_ISO_TEST=luks likewise, and /home encrypted (on a disk above 120 GB).
# ORBIT_ISO_TEST=keep likewise, keeping an existing /home (LUKS: its password
# in ORBIT_ISO_OLD_HOME_PASSWORD).
set -e
[ "$(id -u)" = 0 ] || { echo "build.sh: run as root (live-build)" >&2; exit 1; }
HERE="$(cd "$(dirname "$0")" && pwd)"
OUTDIR="$(mkdir -p "${1:-out}" && cd "${1:-out}" && pwd)"
WORK="${ORBIT_ISO_WORK:-/srv/orbit-iso}"
VERSION="$(git -C "$HERE" describe --always --dirty 2>/dev/null || echo unknown)"

mkdir -p "$WORK"
rsync -a --delete --exclude /cache --exclude /chroot --exclude /binary \
	--exclude /.build --exclude build.log "$HERE/" "$WORK/"
cd "$WORK"
# Orbit's install steps (distro/install/), one copy for each installer:
# debian-installer's (/orbit) and the Live's orbit-install, with the
# archive keyring both give the new system.
INSTALL="$HERE/../install"
cp "$INSTALL/steps.sh" config/includes.installer/orbit/steps.sh
LIB=config/includes.chroot_after_packages/usr/lib/orbit-install
mkdir -p "$LIB"
install -m 644 "$INSTALL/steps.sh" config/archives/orbit.key.chroot "$LIB/"
mv "$LIB/orbit.key.chroot" "$LIB/orbit-archive-keyring.gpg"
install -m 755 "$INSTALL/orbit-install-backend" "$LIB/"

if [ -n "${ORBIT_ISO_SSH_KEY:-}" ]; then
	mkdir -p config/includes.chroot_after_packages/etc/skel/.ssh
	install -m 600 "$ORBIT_ISO_SSH_KEY" config/includes.chroot_after_packages/etc/skel/.ssh/authorized_keys
	echo openssh-server > config/package-lists/debug.list.chroot
	# orbit-firewall drops what no service file opens: SSH to the LAN.
	mkdir -p config/includes.chroot_after_packages/etc/orbit/firewall.d
	echo 'add rule inet orbit lan_in tcp dport 22 accept comment "SSH, debugging image"' \
		> config/includes.chroot_after_packages/etc/orbit/firewall.d/90-debug-ssh.nft
	VERSION="$VERSION-debug"
fi
if [ -n "${ORBIT_ISO_TEST:-}" ]; then
	cat >> config/includes.installer/preseed.cfg <<'PRESEED'
# --- test image: nothing asked ---
d-i keyboard-configuration/xkb-keymap select br
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
PRESEED
	{
		echo "ENCRYPT_HOME=$([ "$ORBIT_ISO_TEST" = luks ] && echo true || echo false)"
		if [ "$ORBIT_ISO_TEST" = keep ]; then
			echo "KEEP_HOME=true"
			echo "OLD_HOME_PASSWORD='${ORBIT_ISO_OLD_HOME_PASSWORD:-}'"
		fi
	} > config/includes.installer/orbit/test.conf
	VERSION="$VERSION-test"
	[ "$ORBIT_ISO_TEST" != 1 ] && VERSION="$VERSION-$ORBIT_ISO_TEST"
fi

lb clean >/dev/null 2>&1 || true
lb config >/dev/null
lb build
OUT="$OUTDIR/orbit-linux-$(date +%Y%m%d)-$VERSION-amd64.iso"
mv live-image-amd64.hybrid.iso "$OUT"
(cd "$OUTDIR" && sha256sum "${OUT##*/}" > "${OUT##*/}.sha256")
ls -la "$OUT"
