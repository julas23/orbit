#!/bin/sh
# Runs in debian-installer at the end (preseed/late_command), with the new
# system in /target (docs/PLANO-DISTRO.md §6.2–6.4). The steps are Orbit's
# install steps (steps.sh, from distro/install/, shared with the Live's
# orbit-install); this script gathers what debian-installer knows:
#   1. btrfs: @rootfs becomes @, the default subvolume; @log, @cache, @tmp,
#      @snapshots (and @home on a one-partition disk) take their folders;
#      a /home of its own encrypted, if 05orbit-home was told so; an
#      existing /home kept (home.sh) mounted as it is
#   2. UEFI: systemd-boot and one UKI per kernel, boot counting (3 tries);
#      BIOS: GRUB installed again on the new subvolume
#   3. what only the Live has goes: its login, its marker, no idle lock
#   4. snapper on / only; zram; swap on disk only at 4 GB of RAM or less
#   5. apt: Debian and Orbit's repository
set -e
T=/target
# The log stays in /tmp (debian-installer's) until the end, when it goes to
# the system's /var/log (the @log subvolume).
LOG=/tmp/orbit-install.log
exec >>"$LOG" 2>&1
set -x
echo "orbit late.sh $(date)"

TOP=/tmp/orbit-top
ROOTDEV=$(awk '$2=="/target" {print $1}' /proc/mounts)
UUID=$(awk '$2=="/" && $1 ~ /^UUID=/ {sub(/^UUID=/, "", $1); print $1}' $T/etc/fstab)
LAYOUT=$(cat /tmp/orbit-layout 2>/dev/null || echo single)
DISK_MB=$(cat /tmp/orbit-disk-mb 2>/dev/null || echo 100000)
RAM_MB=$(cat /tmp/orbit-ram-mb 2>/dev/null || echo 8192)
USERNAME=$(awk -F: '$3>=1000 && $3<60000 {print $1; exit}' $T/etc/passwd)
LOCALE=$(debconf-get debian-installer/locale 2>/dev/null || true)
[ -n "$LOCALE" ] || LOCALE=pt_BR.UTF-8
UEFI=""; [ -e /tmp/orbit-uefi ] && UEFI=1
KEYRING=/orbit/orbit-archive-keyring.gpg
HOME_PW=/tmp/orbit-home-pw                  # 05orbit-home
RECOVERY_KEY=/tmp/orbit-recovery-key        # shown by 96orbit-recovery
target() { in-target "$@"; }

. /orbit/steps.sh
orbit_subvolumes
orbit_bind_system
orbit_encrypt_home
orbit_keep_home
rm -f "$HOME_PW"
orbit_boot
orbit_system
orbit_snapper_swap
orbit_apt
orbit_unmount
