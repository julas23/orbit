#!/bin/bash
# distro/ltsp/build-station-image-rpi.sh — the network boot image of a
# Raspberry Pi 4 station.
#
# Run as root on the station server, after build-station-image.sh (which
# set LTSP up). It takes Raspberry Pi OS Lite (arm64, Debian 13 based: its
# kernel has the Pi's H.264 decoder, Debian's does not), unpacks it into a
# directory exported by NFS, adds orbit-local-station through qemu-user
# and puts the Pi's boot files at the TFTP root, where its bootloader
# looks. The Pi boots with no initramfs: its kernel mounts the NFS root and
# runs LTSP's init, which lays a tmpfs over it.
#
#   distro/ltsp/build-station-image-rpi.sh [SERVER_ADDRESS]
#
# The Pi's bootloader must try the network (rpi-eeprom-config, as
# BOOT_ORDER=0xf21: SD card, then network); the boot needs a cable.
# STATION_SSH_KEY=FILE.pub turns on the image's SSH server, logging in as
# `station` with that key only (to look into a station; off otherwise).
set -euo pipefail

NAME=orbit-station-rpi
CHROOT=/srv/ltsp/$NAME
SERVER=${1:-$(hostname -I | awk '{print $1}')}
IMAGE_URL=https://downloads.raspberrypi.com/raspios_lite_arm64_latest
TFTP=/srv/tftp
HERE=$(cd "$(dirname "$0")" && pwd)

[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }

export DEBIAN_FRONTEND=noninteractive
apt-get install -y -q qemu-user-binfmt xz-utils rsync curl

if [ ! -x "$CHROOT/usr/bin/apt-get" ]; then
    work=$(mktemp -d)
    trap 'umount "$work"/boot "$work"/root 2>/dev/null; [ -n "${loop:-}" ] && losetup -d "$loop"; rm -rf "$work"' EXIT
    echo "downloading Raspberry Pi OS Lite (arm64)"
    curl -fL# "$IMAGE_URL" | xz -d >"$work/raspios.img"
    loop=$(losetup -fP --show "$work/raspios.img")
    mkdir -p "$work/boot" "$work/root" "$CHROOT"
    mount -o ro "${loop}p1" "$work/boot"
    mount -o ro "${loop}p2" "$work/root"
    rsync -aHAX "$work/root/" "$CHROOT/"
    mkdir -p "$CHROOT/boot/firmware"
    rsync -rt "$work/boot/" "$CHROOT/boot/firmware/"
    umount "$work/boot" "$work/root"
    losetup -d "$loop"
    loop=""
    rm -rf "$work"
    trap - EXIT
fi

# Orbit's own packages, with this machine's source and keyring (the Debian
# subset it names is amd64 only: the Pi takes Debian from its own sources).
cp /usr/share/keyrings/orbit-archive-keyring.gpg "$CHROOT/usr/share/keyrings/"
cat >"$CHROOT/etc/apt/sources.list.d/orbit.sources" <<'EOF'
# Orbit Linux: its packages (the station server's repository).
Types: deb
URIs: http://10.0.0.111/orbit
Suites: trixie
Components: main
Signed-By: /usr/share/keyrings/orbit-archive-keyring.gpg
EOF
echo "$NAME" >"$CHROOT/etc/hostname"

mount --bind /proc "$CHROOT/proc"
mount --bind /sys "$CHROOT/sys"
mount --bind /dev "$CHROOT/dev"
mount --bind /dev/pts "$CHROOT/dev/pts"
trap 'umount -l "$CHROOT/dev/pts" "$CHROOT/dev" "$CHROOT/sys" "$CHROOT/proc"' EXIT

# initramfs-tools cannot tell the root's device in a chroot and fails; the
# Pi boots without one anyway (auto_initramfs below), but apt must finish.
mkdir -p "$CHROOT/etc/initramfs-tools/conf.d"
echo MODULES=most >"$CHROOT/etc/initramfs-tools/conf.d/orbit-station"

# LTSP's init, orbit-local-station, the Pi's decoder and converter
# (v4l2h264dec, v4l2convert: gstreamer1.0-plugins-good), the software one
# as the fallback, PipeWire for the session's sound, and mount.nfs
# (nfs-common, only recommended by ltsp): without an initramfs, LTSP's
# init mounts the TFTP folder over NFS itself to fetch ltsp.img, and
# aborts the boot on "mount failed" without it. Raspberry Pi OS's
# first-boot work (cloud-init, a user to make) is switched off: the
# station's user is made here.
chroot "$CHROOT" sh -ec '
    export DEBIAN_FRONTEND=noninteractive
    dpkg --configure -a
    apt-get update -q
    apt-get install -y -q --no-install-recommends \
        ltsp nfs-common orbit-local-station \
        gstreamer1.0-plugins-good gstreamer1.0-libav \
        pipewire pipewire-pulse wireplumber
    apt-get -y -q dist-upgrade
    apt-get clean
    mkdir -p /etc/cloud
    touch /etc/cloud/cloud-init.disabled
    systemctl disable userconfig.service 2>/dev/null || true
    systemctl mask userconfig.service rpi-resize.service resize2fs_once.service 2>/dev/null || true
    id station >/dev/null 2>&1 || useradd -m -s /bin/bash -G video,render,input,audio station
    passwd -d station >/dev/null
'
mkdir -p "$CHROOT/etc/systemd/system/getty@tty1.service.d"
cat >"$CHROOT/etc/systemd/system/getty@tty1.service.d/autologin.conf" <<'EOF'
[Service]
ExecStart=
ExecStart=-/sbin/agetty --autologin station --noclear %I $TERM
EOF
cat >"$CHROOT/home/station/.profile" <<'EOF'
# The network-booted station: tty1 is the station, other consoles a shell.
if [ "$(tty)" = /dev/tty1 ]; then
    orbit-local-station 2>&1 | tee -a "$HOME/station.log"
    # A crash stays on the screen a while; then back to the station (as
    # after a quit, Ctrl+Alt+Backspace).
    [ "${PIPESTATUS[0]}" = 0 ] || sleep 30
    exit
fi
EOF
chroot "$CHROOT" chown station:station /home/station/.profile

# Raspberry Pi OS has the SSH server, switched off, and makes its host
# keys at the first boot, which a read-only root never keeps: they are
# made here.
if [ -n "${STATION_SSH_KEY:-}" ]; then
    chroot "$CHROOT" ssh-keygen -A
    install -d -m 0700 "$CHROOT/home/station/.ssh"
    install -m 0600 "$STATION_SSH_KEY" "$CHROOT/home/station/.ssh/authorized_keys"
    chroot "$CHROOT" chown -R station:station /home/station/.ssh
    printf 'PasswordAuthentication no\nKbdInteractiveAuthentication no\n' >"$CHROOT/etc/ssh/sshd_config.d/orbit-station.conf"
    chroot "$CHROOT" systemctl enable ssh.service
fi

mkdir -p "$CHROOT/etc/orbit"
cat >"$CHROOT/etc/orbit/station.yaml" <<EOF
# The server this network-booted station shows (build-station-image-rpi.sh).
server: $SERVER
EOF

# The kernel's command line: the root over NFS, LTSP's init. The serial
# console and the first-boot resize go. No initramfs: the Pi's kernel has
# its network and NFS built in and mounts the root itself.
sed -i 's/^auto_initramfs=.*/auto_initramfs=0/' "$CHROOT/boot/firmware/config.txt"
cat >"$CHROOT/boot/firmware/cmdline.txt" <<EOF
console=tty1 ip=dhcp root=/dev/nfs nfsroot=$SERVER:$CHROOT,vers=3,tcp,nolock ro rootwait init=/usr/share/ltsp/client/init/init
EOF

# The Pi's bootloader asks the TFTP root for start4.elf, config.txt and the
# rest: links to the image's /boot/firmware.
for file in "$CHROOT/boot/firmware/"*; do
    ln -rsfn "$file" "$TFTP/${file##*/}"
done
# The proxyDHCP answers Pis by their MAC's maker part; LTSP's list predates
# the newer Pi 4 boards.
cat >/etc/dnsmasq.d/orbit-rpi.conf <<'EOF'
# Raspberry Pi 4 boards newer than LTSP's list (build-station-image-rpi.sh).
dhcp-mac=set:rpi,28:cd:c1:*:*:*
dhcp-mac=set:rpi,d8:3a:dd:*:*:*
dhcp-mac=set:rpi,2c:cf:67:*:*:*
EOF
systemctl restart dnsmasq
ltsp nfs
# The Pi's line in the PCs' iPXE menu.
"$HERE/ipxe-menu.sh"
echo "station image $NAME ready, pointed at $SERVER"
