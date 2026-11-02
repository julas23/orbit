#!/bin/sh
# Runs in debian-installer before partitioning (partman/early_command).
# Writes the partman recipe "orbit" for the size of the biggest disk and the
# RAM (docs/PLANO-DISTRO.md §6.4), and on UEFI leaves GRUB out: late.sh
# installs systemd-boot.
#   up to 120 GB: ESP + one btrfs (/ and /home as subvolumes)
#   above:        ESP + btrfs / (25% of the disk, 60 to 100 GiB) + btrfs /home
#   below 40 GB:  the recipe does not fit
set -e
. /usr/share/debconf/confmodule
log() { logger -t orbit-early "$*"; }

disk_mb=0
for d in $(list-devices disk); do
	s=$(( $(cat /sys/block/${d#/dev/}/size 2>/dev/null || echo 0) / 2048 ))
	[ "$s" -gt "$disk_mb" ] && disk_mb=$s && disk=$d
done
ram_mb=$(( $(sed -n 's/^MemTotal: *\([0-9]*\) kB/\1/p' /proc/meminfo) / 1024 ))
log "biggest disk $disk: $disk_mb MB; RAM: $ram_mb MB"
echo "$disk_mb" > /tmp/orbit-disk-mb
echo "$ram_mb" > /tmp/orbit-ram-mb

# Below 40 GB the recipe below does not fit and partman says so; a clear
# message of our own is still to do.
[ "$disk_mb" -lt 38146 ] && log "disk smaller than 40 GB"

if [ -d /sys/firmware/efi ]; then
	boot='1024 1024 1024 fat32 $iflabel{ gpt } method{ efi } format{ } .'
	db_set grub-installer/skip true
	db_fset grub-installer/skip seen true
	db_set nobootloader/confirmation_common ""
	db_fset nobootloader/confirmation_common seen true
	touch /tmp/orbit-uefi
	# The installer's own systemd-boot step (in its initrd) fetches
	# systemd-boot from a mirror and makes no UKI; late.sh does it instead.
	for f in /var/lib/dpkg/info/systemd-boot-installer.postinst; do
		[ -e "$f" ] && printf '#!/bin/sh\nexit 0\n' > "$f"
	done
else
	boot='1 1 1 free $iflabel{ gpt } $reusemethod{ } method{ biosgrub } .'
fi

if [ "$disk_mb" -le 114441 ]; then   # 120 GB
	cat > /tmp/orbit-recipe <<RECIPE
orbit ::
	$boot
	20480 10000 -1 btrfs method{ format } format{ } use_filesystem{ } filesystem{ btrfs } mountpoint{ / } .
RECIPE
	echo single > /tmp/orbit-layout
else
	# partman shares what is above the minimums by priority minus minimum:
	# 1 to the system, 3 to /home, so the system gets about 25%.
	cat > /tmp/orbit-recipe <<RECIPE
orbit ::
	$boot
	61440 1061440 102400 btrfs method{ format } format{ } use_filesystem{ } filesystem{ btrfs } mountpoint{ / } .
	20480 3020480 -1 btrfs method{ format } format{ } use_filesystem{ } filesystem{ btrfs } mountpoint{ /home } .
RECIPE
	echo separate > /tmp/orbit-layout
fi
db_set partman-auto/expert_recipe_file /tmp/orbit-recipe
log "layout $(cat /tmp/orbit-layout)"
