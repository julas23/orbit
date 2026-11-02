#!/bin/sh
# Sourced by early.sh, before partman reads the disks: an existing /home is
# looked for and, if the answer is yes, kept (docs/PLANO-DISTRO.md §6.4).
#   - a filesystem (ext2/3/4, xfs, btrfs; a partition or an LVM volume, or
#     inside LUKS, whose password is asked) whose top folders belong to
#     people (uid 1000 to 59999), or a btrfs with a @home subvolume;
#   - kept on another disk: the system goes to the biggest disk without it,
#     whole (ORBIT_TARGET);
#   - kept on the same disk, as a partition of its own: the disk's other
#     partitions are deleted here, and partman puts the system in the
#     biggest free space;
#   - not kept (said why): a /home sharing its filesystem with a system, or
#     an LVM volume on the only disk, or too little room beside it. The
#     orbit-install of the image's second stage does these.
# For late.sh: /tmp/orbit-keep-home (HOME_* variables) and, for LUKS,
# /tmp/orbit-old-home-pw. Sets ORBIT_LAYOUT=keep and ORBIT_TARGET.

PROBE=/tmp/orbit-probe
MIN_MB=39170   # 40 GB of system and the ESP
pt() { case "$(debconf-get debian-installer/locale 2>/dev/null)" in pt*) return 0 ;; esac; return 1; }

# The whole disks a device lives on: through LVM and LUKS (slaves), and
# from a partition to its disk.
disks_of() {
	local n s
	n=$(basename "$(readlink -f "$1")")
	if [ -n "$(ls /sys/class/block/$n/slaves 2>/dev/null)" ]; then
		for s in /sys/class/block/$n/slaves/*; do disks_of "/dev/${s##*/}"; done
	elif [ -f /sys/class/block/$n/partition ]; then
		basename "$(readlink -f /sys/class/block/$n/..)"
	else
		echo "$n"
	fi
}

# "name(uid) ..." of the folders of $1 that belong to people.
home_users() {
	local d n u
	for d in "$1"/*; do
		[ -d "$d" ] || continue
		n=${d##*/}
		[ "$n" = lost+found ] && continue
		u=$(ls -ldn "$d" | awk '{print $3}')
		[ "$u" -ge 1000 ] 2>/dev/null && [ "$u" -lt 60000 ] && printf '%s(%s) ' "$n" "$u"
	done
	return 0
}

# Looks inside filesystem $1 (LUKS source $2, if any); on a /home, sets
# FOUND_* and returns 0.
try_fs() {
	local t opts base sub users shared size
	t=$(blkid -o value -s TYPE "$1" 2>/dev/null)
	case "$t" in
		ext2|ext3|ext4) opts=ro ;;
		xfs) opts=ro,norecovery ;;
		btrfs) opts=ro,subvolid=5 ;;
		*) return 1 ;;
	esac
	mkdir -p $PROBE
	mount -t "$t" -o "$opts" "$1" $PROBE 2>/dev/null || return 1
	base=$PROBE; sub=""; shared=""
	if [ "$t" = btrfs ] && [ -d $PROBE/@home ]; then
		base=$PROBE/@home; sub=@home
		{ [ -d $PROBE/@ ] || [ -d $PROBE/@rootfs ] || [ -d $PROBE/etc ]; } && shared=1
	fi
	users=""
	if [ -d $base/etc ] && [ -d $base/usr ]; then
		# A system's root, its /home a folder: kept by orbit-install only.
		[ -d $base/home ] && users=$(home_users $base/home) && [ -n "$users" ] && shared=1 && base=$base/home
	else
		users=$(home_users $base)
	fi
	size=$(df -h $PROBE 2>/dev/null | awk 'NR==2 {print $2}')
	umount $PROBE
	[ -n "$users" ] || return 1
	log "a /home on $1 ($t${sub:+ $sub}, $size): $users${shared:+ (shared with a system)}"
	FOUND_DEV=$1; FOUND_TYPE=$t; FOUND_SUBVOL=$sub; FOUND_USERS=$users
	FOUND_SIZE=$size; FOUND_SHARED=$shared; FOUND_LUKS=${2:-}
	FOUND_SRC=${2:-$1}
	FOUND_FSUUID=$(blkid -o value -s UUID "$1")
	return 0
}

# A LUKS partition: its password, asked (up to three tries), opens it
# read-only to look inside.
try_luks() {
	local size pw tries=0
	size=$(( $(cat /sys/class/block/$(basename "$1")/size 2>/dev/null || echo 0) / 2097152 ))G
	db_subst orbit/old-home-password DEV "$1"
	db_subst orbit/old-home-password SIZE "$size"
	db_subst orbit/old-home-password ERROR ""
	while [ $tries -lt 3 ]; do
		tries=$((tries + 1))
		if [ -n "${OLD_HOME_PASSWORD:-}" ]; then
			pw=$OLD_HOME_PASSWORD
		else
			db_set orbit/old-home-password ""
			db_fset orbit/old-home-password seen false
			db_input critical orbit/old-home-password || true
			db_go || return 1
			db_get orbit/old-home-password
			pw=$RET
			db_set orbit/old-home-password ""
		fi
		[ -n "$pw" ] || return 1
		if printf '%s' "$pw" | cryptsetup open --readonly --key-file=- "$1" orbit-probe 2>/dev/null; then
			if try_fs /dev/mapper/orbit-probe "$1"; then
				FOUND_LUKS_UUID=$(cryptsetup luksUUID "$1")
				umask 077
				printf '%s' "$pw" > /tmp/orbit-old-home-pw
				umask 022
				cryptsetup close orbit-probe
				return 0
			fi
			cryptsetup close orbit-probe
			log "$1 opened, but holds no /home"
			return 1
		fi
		[ -n "${OLD_HOME_PASSWORD:-}" ] && return 1
		if pt; then db_subst orbit/old-home-password ERROR "Senha incorreta."; else db_subst orbit/old-home-password ERROR "Wrong password."; fi
	done
	return 1
}

# Free space (MB) beside partition $2 of disk $1, the others deleted: the
# bigger of what is before it and after it.
room_beside() {
	local start size last before after
	start=$(cat /sys/class/block/$(basename "$2")/start)
	size=$(cat /sys/class/block/$(basename "$2")/size)
	last=$(cat /sys/class/block/$1/size)
	before=$(( (start - 2048) / 2048 ))
	after=$(( (last - 34 - start - size) / 2048 ))
	[ $before -gt $after ] && echo $before || echo $after
}

find_existing_home() {
	ORBIT_LAYOUT=""; ORBIT_TARGET=""
	FOUND_DEV=""
	[ -f /orbit/test.conf ] && . /orbit/test.conf
	command -v lvm >/dev/null && lvm vgchange -ay >/dev/null 2>&1 || true
	for dev in $(list-devices partition) $(ls /dev/mapper 2>/dev/null | grep -v -e '^control$' -e '^orbit-probe$' | sed 's|^|/dev/mapper/|'); do
		[ -z "$FOUND_DEV" ] || break
		case "$(blkid -o value -s TYPE "$dev" 2>/dev/null)" in
			crypto_LUKS) try_luks "$dev" || true ;;
			*) try_fs "$dev" || true ;;
		esac
	done
	[ -n "$FOUND_DEV" ] || { log "no existing /home"; return 0; }

	# Where the system can go.
	local home_disks d s best=0 reason="" plan=""
	home_disks=" $(disks_of "$FOUND_SRC" | sort -u | tr '\n' ' ')"
	for d in $(list-devices disk); do
		d=${d#/dev/}
		case "$home_disks" in *" $d "*) continue ;; esac
		s=$(( $(cat /sys/class/block/$d/size) / 2048 ))
		[ $s -ge $MIN_MB ] && [ $s -gt $best ] && best=$s && ORBIT_TARGET=/dev/$d
	done
	if [ -n "$ORBIT_TARGET" ]; then
		if pt; then plan="O sistema vai para $ORBIT_TARGET ($((best / 1024)) GB), que será apagado por inteiro."
		else plan="The system goes to $ORBIT_TARGET ($((best / 1024)) GB), which is erased whole."; fi
	elif [ -n "$FOUND_SHARED" ]; then
		if pt; then reason="Ele divide a partição com um sistema instalado: mantê-lo assim ainda não é possível neste instalador."
		else reason="It shares its partition with an installed system: keeping it so is not possible in this installer yet."; fi
	elif [ -f /sys/class/block/$(basename "$FOUND_SRC")/partition ] && [ -z "$(ls /sys/class/block/$(basename "$FOUND_SRC")/slaves 2>/dev/null)" ]; then
		d=$(echo $home_disks)
		if [ "$(room_beside "$d" "$FOUND_SRC")" -ge $MIN_MB ]; then
			ORBIT_TARGET=same
			if pt; then plan="O sistema vai para o mesmo disco ($d): as outras partições dele são apagadas, e o sistema fica no maior espaço livre ao lado do /home."
			else plan="The system goes to the same disk ($d): its other partitions are deleted, and the system takes the biggest free space beside /home."; fi
		else
			if pt; then reason="Não sobra espaço suficiente ao lado dele no disco (o sistema precisa de 40 GB)."
			else reason="There is not room enough beside it on the disk (the system needs 40 GB)."; fi
		fi
	else
		if pt; then reason="Ele está num volume LVM ou criptografado no único disco: mantê-lo assim ainda não é possível neste instalador."
		else reason="It is on an LVM or encrypted volume on the only disk: keeping it so is not possible in this installer yet."; fi
	fi

	if [ -n "$reason" ]; then
		log "the /home cannot be kept: $reason"
		rm -f /tmp/orbit-old-home-pw
		[ -n "${KEEP_HOME:-}" ] && return 0
		db_subst orbit/keep-home-cannot DEV "$FOUND_SRC"
		db_subst orbit/keep-home-cannot SIZE "$FOUND_SIZE"
		db_subst orbit/keep-home-cannot USERS "$FOUND_USERS"
		db_subst orbit/keep-home-cannot REASON "$reason"
		db_input critical orbit/keep-home-cannot || true
		db_go || true
		return 0
	fi

	if [ -n "${KEEP_HOME:-}" ]; then
		keep=$KEEP_HOME
	else
		db_subst orbit/keep-home DEV "$FOUND_SRC"
		db_subst orbit/keep-home SIZE "$FOUND_SIZE"
		db_subst orbit/keep-home FSTYPE "$FOUND_TYPE${FOUND_LUKS:+, LUKS}"
		db_subst orbit/keep-home USERS "$FOUND_USERS"
		db_subst orbit/keep-home PLAN "$plan"
		db_input critical orbit/keep-home || true
		db_go || true
		db_get orbit/keep-home
		keep=$RET
	fi
	log "keep the /home: $keep ($plan)"
	if [ "$keep" != true ]; then
		rm -f /tmp/orbit-old-home-pw
		ORBIT_TARGET=""
		return 0
	fi

	cat > /tmp/orbit-keep-home <<KEEP
HOME_TYPE=$FOUND_TYPE
HOME_SUBVOL=$FOUND_SUBVOL
HOME_FSUUID=$FOUND_FSUUID
HOME_LUKS=${FOUND_LUKS_UUID:-}
KEEP
	ORBIT_LAYOUT=keep
	if [ "$ORBIT_TARGET" = same ]; then
		# Everything else on the disk goes; partman takes the free space.
		d=$(echo $home_disks)
		keep_num=$(cat /sys/class/block/$(basename "$FOUND_SRC")/partition)
		for p in /sys/class/block/$d/$d*; do
			n=$(cat $p/partition 2>/dev/null) || continue
			[ "$n" = "$keep_num" ] && continue
			log "deleting /dev/${p##*/}"
			sfdisk --delete /dev/$d "$n" >/dev/null 2>&1 || log "sfdisk --delete $d $n failed"
		done
		sfdisk -l /dev/$d 2>&1 | while read -r l; do log "$l"; done
	fi
	return 0
}
