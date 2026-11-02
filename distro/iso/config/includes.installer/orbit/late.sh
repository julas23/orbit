#!/bin/sh
# Runs in debian-installer at the end (preseed/late_command), with the new
# system in /target (docs/PLANO-DISTRO.md §6.2–6.4):
#   1. btrfs: @rootfs becomes @, the default subvolume; @log, @cache, @tmp,
#      @snapshots (and @home on a one-partition disk) take their folders
#   2. UEFI: systemd-boot and one UKI per kernel, boot counting (3 tries)
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

ROOTDEV=$(awk '$2=="/target" {print $1}' /proc/mounts)
UUID=$(awk '$2=="/" && $1 ~ /^UUID=/ {sub(/^UUID=/, "", $1); print $1}' $T/etc/fstab)
LAYOUT=$(cat /tmp/orbit-layout 2>/dev/null || echo single)
DISK_MB=$(cat /tmp/orbit-disk-mb 2>/dev/null || echo 100000)
RAM_MB=$(cat /tmp/orbit-ram-mb 2>/dev/null || echo 8192)
USERNAME=$(awk -F: '$3>=1000 && $3<60000 {print $1; exit}' $T/etc/passwd)
OPTS="compress=zstd:1,noatime"

# --- 1. btrfs subvolumes ------------------------------------------------------
TOP=/tmp/orbit-top
mkdir -p $TOP
mount -t btrfs -o subvolid=5 "$ROOTDEV" $TOP
[ -d $TOP/@rootfs ] && [ ! -e $TOP/@ ] && mv $TOP/@rootfs $TOP/@

move_to_subvol() {   # move_to_subvol <subvolume> <folder in the system>
	btrfs subvolume create "$TOP/$1"
	if [ -d "$T$2" ]; then
		cp -a "$T$2/." "$TOP/$1/"
		find "$T$2" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
	else
		mkdir -p "$T$2"
	fi
	echo "UUID=$UUID $2 btrfs subvol=$1,$OPTS 0 0" >> $T/etc/fstab
}
# The root line: no subvol= (the default subvolume is mounted), our options;
# a separate /home (btrfs, no snapshots) gets the same options. Both before
# the lines below are added, which carry their own subvol=.
sed -i -E "s|^(UUID=$UUID[[:space:]]+/[[:space:]]+btrfs[[:space:]]+)[^[:space:]]+|\1$OPTS|" $T/etc/fstab
[ "$LAYOUT" = separate ] && sed -i -E "s|^(UUID=[^[:space:]]+[[:space:]]+/home[[:space:]]+btrfs[[:space:]]+)[^[:space:]]+|\1$OPTS|" $T/etc/fstab
[ "$LAYOUT" = single ] && move_to_subvol @home /home
move_to_subvol @log /var/log
move_to_subvol @cache /var/cache
move_to_subvol @tmp /var/tmp
chmod 1777 $TOP/@tmp
ID=$(btrfs subvolume list $TOP | awk '$NF=="@" {print $2}')
btrfs subvolume set-default "$ID" $TOP

# --- 2. systemd-boot and UKIs (UEFI) -----------------------------------------
if [ -e /tmp/orbit-uefi ]; then
	# bootctl and efibootmgr in the new system need /sys (with the EFI
	# variables) and /dev; in-target gives it /proc only.
	grep -q " /sys/firmware/efi/efivars " /proc/mounts || mount -t efivarfs efivarfs /sys/firmware/efi/efivars
	grep -q " $T/sys " /proc/mounts || mount -o bind /sys $T/sys
	grep -q " $T/sys/firmware/efi/efivars " /proc/mounts || mount -o bind /sys/firmware/efi/efivars $T/sys/firmware/efi/efivars
	grep -q " $T/dev " /proc/mounts || mount -o bind /dev $T/dev
	mkdir -p $T/etc/kernel
	echo "layout=uki" > $T/etc/kernel/install.conf
	echo "root=UUID=$UUID rw rootflags=$OPTS quiet splash" > $T/etc/kernel/cmdline
	echo 3 > $T/etc/kernel/tries
	# The UKIs are named orbit-<version>.efi (the Live's machine-id is gone,
	# and kernel-install would take its name from elsewhere).
	echo orbit > $T/etc/kernel/entry-token
	[ -s $T/etc/machine-id ] || in-target systemd-machine-id-setup
	# systemd-boot's postinst installs the loader in the ESP and makes a UKI
	# of each kernel (install.conf above); bootctl install again is harmless.
	in-target sh -c 'dpkg -i /usr/share/orbit/debs/systemd-boot_*.deb'
	in-target bootctl install --esp-path=/boot/efi
	cat > $T/boot/efi/loader/loader.conf <<LOADER
# The menu stays hidden: hold Space while starting to see it
# (docs/PLANO-DISTRO.md §6.3).
timeout 0
editor no
LOADER
	in-target update-initramfs -u -k all
	for v in $(ls $T/lib/modules); do
		in-target kernel-install add "$v" "/boot/vmlinuz-$v"
	done
	ls -la $T/boot/efi/EFI/Linux/
fi

rm -rf $T/usr/share/orbit/debs

# --- 3. what only the Live has ------------------------------------------------
# The language chosen in the installer (the Live's image says C.UTF-8).
LOCALE=$(debconf-get debian-installer/locale 2>/dev/null || true)
[ -n "$LOCALE" ] || LOCALE=pt_BR.UTF-8
grep -q "^$LOCALE UTF-8" $T/etc/locale.gen || echo "$LOCALE UTF-8" >> $T/etc/locale.gen
grep -q "^en_US.UTF-8 UTF-8" $T/etc/locale.gen || echo "en_US.UTF-8 UTF-8" >> $T/etc/locale.gen
in-target locale-gen
# /etc/default/locale is written by finish-install.d/95orbit-locale: a later
# installer step would overwrite it here.

cat > $T/etc/orbit/login.yaml <<LOGIN
# /etc/orbit/login.yaml — Orbit-Login (PLANO-CORE §7.4): the user whose
# session opens at boot, locked; the greet asks for the password.
user: $USERNAME
command: orbit-core --locked
vt: 1
LOGIN
rm -f $T/etc/orbit/live
for f in $T/etc/skel/.config/orbit/core.yaml $T/home/*/.config/orbit/core.yaml; do
	[ -f "$f" ] && sed -i -e '/^# The Live session never locks/d' -e '/^idle_lock: 0$/d' "$f"
done

# --- 4. snapper, zram, swap ----------------------------------------------------
in-target snapper --no-dbus -c root create-config /
btrfs subvolume delete $T/.snapshots
btrfs subvolume create $TOP/@snapshots
mkdir -p $T/.snapshots
echo "UUID=$UUID /.snapshots btrfs subvol=@snapshots,$OPTS 0 0" >> $T/etc/fstab
mount -t btrfs -o subvol=@snapshots "$ROOTDEV" $T/.snapshots
chmod 750 $T/.snapshots
LIMIT=10; [ "$DISK_MB" -le 61035 ] && LIMIT=5   # 64 GB
sed -i -E -e 's/^TIMELINE_CREATE=.*/TIMELINE_CREATE="no"/' \
	-e "s/^NUMBER_LIMIT=.*/NUMBER_LIMIT=\"$LIMIT\"/" \
	-e 's/^NUMBER_LIMIT_IMPORTANT=.*/NUMBER_LIMIT_IMPORTANT="5"/' $T/etc/snapper/configs/root

cat > $T/etc/systemd/zram-generator.conf <<ZRAM
# zram on every machine (docs/PLANO-DISTRO.md §6.4).
[zram0]
zram-size = min(ram / 2, 8192)
compression-algorithm = zstd
ZRAM
if [ "$RAM_MB" -le 4096 ]; then
	btrfs subvolume create $TOP/@swap
	btrfs filesystem mkswapfile --size "${RAM_MB}m" $TOP/@swap/swapfile
	mkdir -p $T/swap
	echo "UUID=$UUID /swap btrfs subvol=@swap,noatime 0 0" >> $T/etc/fstab
	echo "/swap/swapfile none swap defaults,pri=10 0 0" >> $T/etc/fstab
fi

# --- 5. apt ------------------------------------------------------------------
sed -i '/^deb cdrom:/d' $T/etc/apt/sources.list 2>/dev/null || true
[ -s $T/etc/apt/sources.list ] || rm -f $T/etc/apt/sources.list
cp /orbit/orbit-archive-keyring.gpg $T/usr/share/keyrings/orbit-archive-keyring.gpg
cat > $T/etc/apt/sources.list.d/debian.sources <<SOURCES
Types: deb
URIs: http://deb.debian.org/debian
Suites: trixie trixie-updates
Components: main contrib non-free non-free-firmware
Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg

Types: deb
URIs: http://security.debian.org/debian-security
Suites: trixie-security
Components: main contrib non-free non-free-firmware
Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg
SOURCES
cat > $T/etc/apt/sources.list.d/orbit.sources <<SOURCES
Types: deb
URIs: https://rationalazimuth.com/orbit
Suites: trixie
Components: main
Signed-By: /usr/share/keyrings/orbit-archive-keyring.gpg
SOURCES

cat $T/etc/fstab
echo "orbit late.sh done $(date)"
cp "$LOG" $TOP/@log/orbit-install.log
umount $T/.snapshots || true
umount $TOP || true
for m in $T/sys/firmware/efi/efivars $T/sys $T/dev; do
	grep -q " $m " /proc/mounts && umount "$m" || true
done
