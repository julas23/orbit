#!/bin/bash
# distro/ltsp/build-station-image.sh — the station's network boot image.
#
# Run as root on the machine that serves the stations (the one running
# orbit-remote-server): it builds a minimal Debian 13 chroot with
# orbit-local-station, turns it into an LTSP image and sets LTSP up to
# serve it (dnsmasq as a proxyDHCP beside the LAN's DHCP, iPXE, NFS).
# A machine booted from the network then lands on tty1 straight in the
# station, pointed at this server.
#
#   distro/ltsp/build-station-image.sh [SERVER_ADDRESS]
#
# SERVER_ADDRESS defaults to this machine's first IPv4 address. Run it
# again to rebuild the image (the chroot is upgraded, not made anew).
set -euo pipefail

NAME=orbit-station
CHROOT=/srv/ltsp/$NAME
SERVER=${1:-$(hostname -I | awk '{print $1}')}
MIRROR=http://deb.debian.org/debian
HERE=$(cd "$(dirname "$0")" && pwd)

[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }

export DEBIAN_FRONTEND=noninteractive
apt-get install -y -q ltsp ipxe dnsmasq nfs-kernel-server squashfs-tools debootstrap

if [ ! -x "$CHROOT/usr/bin/apt-get" ]; then
    debootstrap --variant=minbase --components=main,contrib,non-free,non-free-firmware \
        trixie "$CHROOT" "$MIRROR"
fi

cat >"$CHROOT/etc/apt/sources.list" <<EOF
deb $MIRROR trixie main contrib non-free non-free-firmware
deb $MIRROR trixie-updates main contrib non-free non-free-firmware
deb http://security.debian.org/debian-security trixie-security main contrib non-free non-free-firmware
EOF
# Orbit's own packages, with this machine's source and keyring.
cp /usr/share/keyrings/orbit-archive-keyring.gpg "$CHROOT/usr/share/keyrings/"
cp /etc/apt/sources.list.d/orbit.sources "$CHROOT/etc/apt/sources.list.d/"
echo "$NAME" >"$CHROOT/etc/hostname"

mount --bind /proc "$CHROOT/proc"
mount --bind /sys "$CHROOT/sys"
mount --bind /dev "$CHROOT/dev"
mount --bind /dev/pts "$CHROOT/dev/pts"
trap 'umount -l "$CHROOT/dev/pts" "$CHROOT/dev" "$CHROOT/sys" "$CHROOT/proc"' EXIT

# The kernel and its initramfs; the firmware and VA-API drivers of the GPUs
# a station may have, and the software decoder for one without VA-API;
# logind, which libseat goes through; PipeWire for the session's sound.
# LTSP's own code comes with its initrd (ltsp.img), the image needs no ltsp
# package.
chroot "$CHROOT" sh -ec '
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -q
    apt-get install -y -q --no-install-recommends \
        linux-image-amd64 initramfs-tools zstd \
        systemd-sysv systemd-timesyncd udev dbus libpam-systemd kbd locales ca-certificates \
        firmware-amd-graphics firmware-intel-graphics firmware-misc-nonfree firmware-realtek \
        mesa-va-drivers intel-media-va-driver-non-free vainfo gstreamer1.0-libav \
        pipewire pipewire-pulse wireplumber \
        orbit-local-station
    apt-get -y -q dist-upgrade
    apt-get clean
'

# The station's user, logged in on tty1 by the getty, and its login shell
# running the station there. Its token lives in RAM: a reboot pairs again.
chroot "$CHROOT" sh -ec '
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
    orbit-local-station >>"$HOME/station.log" 2>&1
    # Back to the station after a quit (Ctrl+Alt+Backspace) or a crash.
    exit
fi
EOF
chroot "$CHROOT" chown station:station /home/station/.profile

mkdir -p "$CHROOT/etc/orbit"
cat >"$CHROOT/etc/orbit/station.yaml" <<EOF
# The server this network-booted station shows (build-station-image.sh).
server: $SERVER
EOF

# LTSP: the image (and its kernel), iPXE's menu, NFS, the proxyDHCP only
# (the LAN keeps its own DHCP) and the initrd that carries ltsp.conf.
install -D -m 0644 "$HERE/ltsp.conf" /etc/ltsp/ltsp.conf
ltsp image "$NAME"
ltsp ipxe
ltsp nfs
ltsp dnsmasq --proxy-dhcp=1 --real-dhcp=0
ltsp initrd
echo "station image $NAME ready, pointed at $SERVER"
