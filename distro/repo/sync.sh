#!/usr/bin/env bash
# sync.sh — builds and publishes the Orbit apt repository with aptly, on the
# repository server (root). Run it again at any time: every step picks up
# where things are.
#
#   distro/repo/sync.sh [DIR]     DIR: this directory's copy on the server
#
# Publishes two signed repositories (the key: orbit-archive-keyring.gpg at
# the top of the site):
#
#   http://SERVER/debian trixie main   the part of Debian 13 Orbit uses —
#       debian.list, every package of priority required, important and
#       standard, and what the packages below depend on — with all their
#       dependencies, merged with trixie-updates and trixie-security
#       (amd64; and i386.list for Steam and Proton, in i386)
#   http://SERVER/orbit  trixie main   Orbit's own packages (dropped in
#       /srv/orbit-incoming/orbit by the build), the GitHub releases in
#       github.list (ArtCraft, third parties), the newest packages of the
#       vendors' repositories in vendors.list (VS Code, Firefox, Docker,
#       Chrome, Steam...) — only the newest: those repositories keep every
#       version ever made — and the .deb of direct.list (Discord)
#   http://SERVER/orbit-public trixie main   Orbit's own packages only, but
#       those in public.exclude (metapackages that need the third parties):
#       what may go to the internet — served as
#       https://rationalazimuth.com/orbit
#
# GitHub's API allows 60 requests an hour without a token: set GITHUB_TOKEN
# (e.g. in /etc/orbit-repo.env, read below) for more.
#
# Each run makes dated snapshots and switches the publications to them, so
# a bad update can be switched back (`aptly publish switch`).
#
# Before the first run: aptly checks the Debian mirrors against root's
# trustedkeys.gpg, which must hold Debian's current archive keys (trixie's
# too; on a server not running trixie, take debian-archive-keyring from
# trixie itself, its hash checked through a Release its older key signs):
#   gpg --no-default-keyring --keyring trustedkeys.gpg \
#       --import /usr/share/keyrings/debian-archive-keyring.gpg
# and /etc/aptly.conf with "rootDir": "/srv/aptly".
set -euo pipefail

DIR=$(cd "${1:-$(dirname "$0")}" && pwd)
[[ -r /etc/orbit-repo.env ]] && . /etc/orbit-repo.env
DIST=trixie
KEY="Orbit Linux Archive"
INCOMING=/srv/orbit-incoming
STAMP=$(date +%Y%m%d-%H%M%S)
DEBIAN=http://deb.debian.org/debian
SECURITY=http://security.debian.org/debian-security
COMPONENTS="main contrib non-free non-free-firmware"

log() { printf '\n=== %s\n' "$*"; }
names() { sed 's/#.*//' "$1" | tr -s ' \t' '\n' | sed '/^$/d'; }
mirror_exists() { aptly mirror show "$1" >/dev/null 2>&1; }
publish() { # prefix snapshot [architectures]
    local archs=${3:-amd64,all} have
    # A publication keeps the architectures it was made with: other ones
    # mean making it again.
    # Not published yet (the first run): `publish show` fails, and with
    # pipefail that would end the script.
    have=$(aptly publish show "$DIST" "$1" 2>/dev/null | sed -n 's/^Architectures: //p' | tr ' ' '\n' | sort | paste -sd,) || have=""
    if [[ -n "$have" && "$have" != "$(tr ',' '\n' <<< "$archs" | sort | paste -sd,)" ]]; then
        aptly publish drop "$DIST" "$1" >/dev/null
        have=""
    fi
    if [[ -n "$have" ]]; then
        aptly publish switch -batch -gpg-key="$KEY" "$DIST" "$1" "$2"
    else
        aptly publish snapshot -batch -gpg-key="$KEY" -distribution="$DIST" -component=main \
            -architectures="$archs" "$2" "$1"
    fi
}

mkdir -p "$INCOMING/orbit" "$INCOMING/github" "$INCOMING/vendors"

log "GitHub releases"
api() { curl -fsSL ${GITHUB_TOKEN:+-H "Authorization: Bearer $GITHUB_TOKEN"} "https://api.github.com/$1"; }
while read -r repo pattern; do
    [[ -z "$repo" || "$repo" == \#* ]] && continue
    if ! releases=$(api "repos/$repo/releases?per_page=10"); then
        # Rate limit (60 an hour without GITHUB_TOKEN) or no network: what
        # is here already stays.
        echo "!! $repo: GitHub unavailable, keeping what is here"
        continue
    fi
    url=$(jq -r --arg re "$pattern" \
        '[.[] | select((.draft or .prerelease) | not) | .assets[] | select(.name | test($re))][0].browser_download_url // empty' <<< "$releases")
    if [[ -z "$url" ]]; then
        echo "!! $repo: no asset matching $pattern"
        continue
    fi
    file="$INCOMING/github/${url##*/}"
    if [[ ! -s "$file" ]]; then
        echo "-> $repo: ${url##*/}"
        curl -fsSL -o "$file.part" "$url" && mv "$file.part" "$file"
    fi
    # Only the newest of each package stays.
    pkg=$(dpkg-deb -f "$file" Package)
    for old in "$INCOMING"/github/*.deb; do
        [[ "$old" != "$file" && "$(dpkg-deb -f "$old" Package)" == "$pkg" ]] && rm -f "$old"
    done
done < "$DIR/github.list"

log "Vendors' repositories (the newest of each package only)"
# vendors.list: repository URL, suite, component, key URL, packages. The
# InRelease is checked against the vendor's key before anything is taken.
# A suite ending in "/" is a flat repository (no dists/, one Packages
# index, component "-"). A package is amd64 (or all) unless it says
# otherwise: steam-libs-i386:i386.
mkdir -p "$INCOMING/vendors"
while read -r url suite component keyurl pkgs; do
    [[ -z "$url" || "$url" == \#* ]] && continue
    tmp=$(mktemp -d)
    curl -fsSL "$keyurl" | gpg --dearmor > "$tmp/key.gpg" 2>/dev/null || curl -fsSL -o "$tmp/key.gpg" "$keyurl"
    if [[ "$suite" == */ ]]; then
        base=${suite#./}; base=${base%/}; base=${base#/}; base="$url${base:+/$base}"
        indexes="Packages"
    else
        base="$url/dists/$suite"
        indexes="$component/binary-amd64/Packages $component/binary-all/Packages"
        [[ "$pkgs" == *:i386* ]] && indexes+=" $component/binary-i386/Packages"
    fi
    curl -fsSL -o "$tmp/InRelease" "$base/InRelease" \
        || { echo "!! $url: no InRelease, skipped"; rm -rf "$tmp"; continue; }
    gpgv --keyring "$tmp/key.gpg" --output "$tmp/Release" "$tmp/InRelease" 2>/dev/null \
        || { echo "!! $url: bad signature, skipped"; rm -rf "$tmp"; continue; }
    # The amd64 index and the one of packages for every architecture.
    : > "$tmp/Packages"
    for index in $indexes; do
        sum=$(awk -v f="$index" '/^SHA256:/{s=1;next} /^[A-Za-z]/{s=0} s && $3==f {print $1}' "$tmp/Release")
        [[ -n "$sum" ]] || continue
        curl -fsSL -o "$tmp/part" "$base/$index"
        [[ "$(sha256sum < "$tmp/part" | cut -d' ' -f1)" == "$sum" ]] || { echo "!! $url: $index checksum"; continue; }
        { cat "$tmp/part"; echo; } >> "$tmp/Packages"
    done
    for spec in ${pkgs//,/ }; do
        pkg=${spec%%:*}
        best=$(python3 - "$tmp/Packages" "$spec" <<'PY'
import sys, subprocess
best = None
name, _, arch = sys.argv[2].partition(":")
archs = (arch,) if arch else ("amd64", "all")
for block in open(sys.argv[1], encoding="utf-8").read().split("\n\n"):
    f = dict(l.split(": ", 1) for l in block.splitlines() if ": " in l and not l.startswith(" "))
    if f.get("Package") != name or f.get("Architecture") not in archs:
        continue
    if best is None or subprocess.run(["dpkg", "--compare-versions", f["Version"], "gt", best["Version"]]).returncode == 0:
        best = f
if best:
    print(best["Filename"], best["SHA256"])
PY
)
        [[ -n "$best" ]] || { echo "!! $pkg: not in $url"; continue; }
        read -r path sha <<< "$best"
        file="$INCOMING/vendors/${path##*/}"
        if [[ ! -s "$file" ]]; then
            echo "-> $pkg: ${path##*/}"
            curl -fsSL -o "$file.part" "$url/$path"
            [[ "$(sha256sum < "$file.part" | cut -d' ' -f1)" == "$sha" ]] || { echo "!! $pkg: checksum"; rm -f "$file.part"; continue; }
            mv "$file.part" "$file"
        fi
        for old in "$INCOMING"/vendors/*.deb; do
            [[ "$old" != "$file" && "$(dpkg-deb -f "$old" Package)" == "$pkg" ]] && rm -f "$old"
        done
    done
    rm -rf "$tmp"
done < "$DIR/vendors.list"

log "Direct downloads"
# direct.list: package name, URL of its newest .deb (one that redirects to
# the versioned file). Taken again only when that file changes; HTTPS is
# all the checking there is.
mkdir -p "$INCOMING/direct"
while read -r pkg url; do
    [[ -z "$pkg" || "$pkg" == \#* ]] && continue
    real=$(curl -fsIL -o /dev/null -w '%{url_effective}' "$url") || { echo "!! $pkg: unavailable, keeping what is here"; continue; }
    compgen -G "$INCOMING/direct/${pkg}_*.deb" >/dev/null \
        && [[ "$(cat "$INCOMING/direct/$pkg.url" 2>/dev/null)" == "$real" ]] && continue
    echo "-> $pkg: $real"
    part="$INCOMING/direct/$pkg.part"
    curl -fsSL -o "$part" "$real" || { echo "!! $pkg: download failed"; continue; }
    [[ "$(dpkg-deb -f "$part" Package 2>/dev/null)" == "$pkg" ]] \
        || { echo "!! $pkg: not a .deb of $pkg"; rm -f "$part"; continue; }
    # Named by version, as aptly's pool keeps the file name: a new release
    # saved under the old name is "already exists and is different".
    rm -f "$INCOMING/direct/$pkg.deb" "$INCOMING/direct/${pkg}"_*.deb
    mv "$part" "$INCOMING/direct/${pkg}_$(dpkg-deb -f "$part" Version)_$(dpkg-deb -f "$part" Architecture).deb"
    echo "$real" > "$INCOMING/direct/$pkg.url"
done < "$DIR/direct.list"

log "Orbit's own and the GitHub packages"
aptly repo show orbit-local >/dev/null 2>&1 || aptly repo create -distribution="$DIST" -component=main orbit-local
for d in orbit github vendors direct; do
    compgen -G "$INCOMING/$d/*.deb" >/dev/null && aptly repo add -force-replace orbit-local "$INCOMING/$d" >/dev/null
done
# What left the incoming folders leaves the repository.
ours=$(mktemp)
for deb in "$INCOMING"/{orbit,github,vendors,direct}/*.deb; do
    [[ -f "$deb" ]] && dpkg-deb -f "$deb" Package
done | sort -u > "$ours"
aptly repo search orbit-local 2>/dev/null | while read -r ref; do
    name=${ref%%_*}
    grep -qx "$name" "$ours" || aptly repo remove orbit-local "Name (= $name)" >/dev/null
done
# And of each package only the newest version stays, per architecture: an
# arm64 package built less often (orbit-local-station for the Raspberry Pi
# stations) is not the older version of its amd64 twin.
aptly repo search -format='{{.Package}} {{.Architecture}}' orbit-local 2>/dev/null | sort -u | while read -r name arch; do
    which="Name (= $name), \$Architecture (= $arch)"
    versions=$(aptly repo search -format='{{.Version}}' orbit-local "$which")
    newest=""
    for v in $versions; do
        [[ -z "$newest" ]] || dpkg --compare-versions "$v" gt "$newest" && newest=$v
    done
    for v in $versions; do
        dpkg --compare-versions "$v" lt "$newest" && aptly repo remove orbit-local "$which, \$Version (= $v)" >/dev/null
    done
    true
done

log "What the packages need from Debian"
# Per architecture: amd64 takes debian.list and what the amd64 and all
# packages above need; i386 (Steam and Proton) takes i386.list and what the
# i386 packages above need (steam-libs-i386).
wants() { # arch
    if [[ $1 == amd64 ]]; then names "$DIR/debian.list"; else names "$DIR/i386.list" 2>/dev/null || true; fi
    for deb in "$INCOMING"/{orbit,github,vendors,direct}/*.deb; do
        [[ -f "$deb" ]] || continue
        a=$(dpkg-deb -f "$deb" Architecture)
        if [[ $1 == amd64 ]]; then [[ $a != i386 ]] || continue; else [[ $a == i386 ]] || continue; fi
        for field in Depends Pre-Depends Recommends; do
            dpkg-deb -f "$deb" "$field" 2>/dev/null
        done | tr ',|' '\n\n'
    done | sed -E 's/\(.*\)//; s/:(any|i386|amd64)//; s/^ +| +$//g' | sed '/^$/d'
}
declare -A FILTER
for arch in amd64 i386; do
    wanted=$(mktemp)
    wants $arch > "$wanted"
    if [[ $arch == i386 && ! -s "$wanted" ]]; then rm -f "$wanted"; continue; fi
    # Names other packages only provide are resolved to one that does.
    index=$(mktemp)
    for suite in "$DIST" "$DIST-updates"; do
        for c in $COMPONENTS; do
            curl -fsSL "$DEBIAN/dists/$suite/$c/binary-$arch/Packages.xz" 2>/dev/null | xz -dc
        done
    done > "$index"
    python3 - "$index" "$wanted" "$ours" > "$wanted.real" <<'PY'
import sys
real, provides = set(), {}
ours = {l.strip() for l in open(sys.argv[3]) if l.strip()}
for block in open(sys.argv[1], encoding="utf-8", errors="replace").read().split("\n\n"):
    fields = dict(l.split(": ", 1) for l in block.splitlines() if ": " in l and not l.startswith(" "))
    name = fields.get("Package")
    if not name:
        continue
    real.add(name)
    for p in fields.get("Provides", "").split(","):
        p = p.split("(")[0].strip()
        if p:
            provides.setdefault(p, []).append(name)
out, missing = set(), []
for want in {w.strip() for w in open(sys.argv[2]) if w.strip()}:
    if want in ours:
        continue
    if want in real:
        out.add(want)
    elif want in provides:
        out.add(sorted(provides[want], key=len)[0])
    else:
        missing.append(want)
for name in sorted(out):
    print(name)
if missing:
    print("not in Debian (left out): " + " ".join(sorted(missing)), file=sys.stderr)
PY
    if [[ $arch == amd64 ]]; then
        filter="Priority (required) | Priority (important) | Priority (standard)"
    else
        filter="Name (= __none__)"
    fi
    while read -r name; do filter+=" | Name (= $name)"; done < "$wanted.real"
    FILTER[$arch]=$filter
    echo "$arch: $(wc -l < "$wanted.real") packages asked for"
    rm -f "$wanted" "$wanted.real" "$index"
done
rm -f "$ours"

log "Debian mirrors (filtered, with dependencies)"
SNAPS=()
for arch in "${!FILTER[@]}"; do
    sfx=""; [[ $arch == amd64 ]] || sfx="-$arch"
    for spec in "debian $DEBIAN $DIST" "debian-updates $DEBIAN $DIST-updates" "debian-security $SECURITY $DIST-security"; do
        read -r name url suite <<< "$spec"
        name+=$sfx
        filter=${FILTER[$arch]}
        if [[ "$name" != "debian$sfx" ]]; then
            # Updates take every package the base has, so the binaries of one
            # source never mix versions (libpng-dev must match libpng16).
            filter="Name (= __none__)"
            while read -r have; do filter+=" | Name (= $have)"; done < <(aptly mirror search "debian$sfx" 'Name (% *)' | cut -d_ -f1 | sort -u)
        fi
        if mirror_exists "$name"; then
            aptly mirror edit -filter="$filter" -filter-with-deps "$name"
        else
            # The security archive lists its components its own way.
            aptly mirror create -architectures=$arch -filter="$filter" -filter-with-deps -force-components \
                "$name" "$url" "$suite" $COMPONENTS
        fi
        aptly mirror update -max-tries=3 "$name" | grep -E "^(Download|ERROR|WARNING)" | grep -vE "^Downloading: .*\.deb$" || true
        SNAPS+=("$name-$STAMP")
    done
done

log "Snapshots and publication"
for snap in "${SNAPS[@]}"; do
    aptly snapshot create "$snap" from mirror "${snap%-$STAMP}"
done
aptly snapshot merge -latest "debian-all-$STAMP" "${SNAPS[@]}"
DEBIAN_ARCHS=amd64,all; [[ -n "${FILTER[i386]:-}" ]] && DEBIAN_ARCHS=amd64,i386,all
aptly snapshot create "orbit-local-$STAMP" from repo orbit-local
public="Name (= __none__)"
for deb in "$INCOMING"/orbit/*.deb; do
    [[ -f "$deb" ]] || continue
    name=$(dpkg-deb -f "$deb" Package)
    names "$DIR/public.exclude" | grep -qx "$name" || public+=" | Name (= $name)"
done
aptly snapshot filter "orbit-local-$STAMP" "orbit-public-$STAMP" "$public"
publish debian "debian-all-$STAMP" "$DEBIAN_ARCHS"
# Orbit's own packages for the Raspberry Pi stations too (the Debian
# subset stays amd64, and i386 for Steam: a Pi takes Debian from its own
# sources).
publish orbit "orbit-local-$STAMP" amd64,arm64,i386,all
publish orbit-public "orbit-public-$STAMP" amd64,arm64,all
gpg --export "$KEY" > /srv/aptly/public/orbit-archive-keyring.gpg

log "Old snapshots"
# The last 7 of each kind stay (to switch back to); packages none of them
# holds leave the pool. The merged ones first: aptly refuses to drop a
# snapshot another one was made from. A refusal is said, not fatal.
for kind in debian-all orbit-public orbit-local debian debian-updates debian-security debian-i386 debian-updates-i386 debian-security-i386; do
    aptly snapshot list -raw -sort=time | grep -E "^$kind-[0-9]{8}-[0-9]{6}$" | head -n -7 | while read -r old; do
        if aptly snapshot drop "$old" >/dev/null 2>&1; then echo "dropped $old"; else echo "kept $old (still used)"; fi
    done
done
aptly db cleanup >/dev/null

log "Done"
aptly snapshot show "debian-all-$STAMP" | grep -i "number of packages" || true
aptly snapshot show "orbit-local-$STAMP" | grep -i "number of packages" || true
du -sh /srv/aptly/pool 2>/dev/null || true
