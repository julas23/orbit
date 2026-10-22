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
#   http://SERVER/orbit  trixie main   Orbit's own packages (dropped in
#       /srv/orbit-incoming/orbit by the build), the GitHub releases in
#       github.list (ArtCraft, third parties) and the newest packages of the
#       vendors' repositories in vendors.list (VS Code, Firefox) — only the
#       newest: those repositories keep every version ever made
#
# GitHub's API allows 60 requests an hour without a token: set GITHUB_TOKEN
# (e.g. in /etc/orbit-repo.env, read below) for more.
#
# Each run makes dated snapshots and switches the publications to them, so
# a bad update can be switched back (`aptly publish switch`).
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
publish() { # prefix snapshot
    if aptly publish show "$DIST" "$1" >/dev/null 2>&1; then
        aptly publish switch -batch -gpg-key="$KEY" "$DIST" "$1" "$2"
    else
        aptly publish snapshot -batch -gpg-key="$KEY" -distribution="$DIST" -component=main \
            -architectures=amd64,all "$2" "$1"
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
mkdir -p "$INCOMING/vendors"
while read -r url suite component keyurl pkgs; do
    [[ -z "$url" || "$url" == \#* ]] && continue
    tmp=$(mktemp -d)
    curl -fsSL "$keyurl" | gpg --dearmor > "$tmp/key.gpg" 2>/dev/null || curl -fsSL -o "$tmp/key.gpg" "$keyurl"
    curl -fsSL -o "$tmp/InRelease" "$url/dists/$suite/InRelease"
    gpgv --keyring "$tmp/key.gpg" --output "$tmp/Release" "$tmp/InRelease" 2>/dev/null \
        || { echo "!! $url: bad signature, skipped"; rm -rf "$tmp"; continue; }
    # The amd64 index and the one of packages for every architecture.
    : > "$tmp/Packages"
    for arch in amd64 all; do
        index="$component/binary-$arch/Packages"
        sum=$(awk -v f="$index" '/^SHA256:/{s=1;next} /^[A-Za-z]/{s=0} s && $3==f {print $1}' "$tmp/Release")
        [[ -n "$sum" ]] || continue
        curl -fsSL -o "$tmp/part" "$url/dists/$suite/$index"
        [[ "$(sha256sum < "$tmp/part" | cut -d' ' -f1)" == "$sum" ]] || { echo "!! $url: $index checksum"; continue; }
        { cat "$tmp/part"; echo; } >> "$tmp/Packages"
    done
    for pkg in ${pkgs//,/ }; do
        best=$(python3 - "$tmp/Packages" "$pkg" <<'PY'
import sys, subprocess
best = None
for block in open(sys.argv[1], encoding="utf-8").read().split("\n\n"):
    f = dict(l.split(": ", 1) for l in block.splitlines() if ": " in l and not l.startswith(" "))
    if f.get("Package") != sys.argv[2] or f.get("Architecture") not in ("amd64", "all"):
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

log "Orbit's own and the GitHub packages"
aptly repo show orbit-local >/dev/null 2>&1 || aptly repo create -distribution="$DIST" -component=main orbit-local
for d in orbit github vendors; do
    compgen -G "$INCOMING/$d/*.deb" >/dev/null && aptly repo add -force-replace orbit-local "$INCOMING/$d" >/dev/null
done
# What left the incoming folders leaves the repository.
ours=$(mktemp)
for deb in "$INCOMING"/{orbit,github,vendors}/*.deb; do
    [[ -f "$deb" ]] && dpkg-deb -f "$deb" Package
done | sort -u > "$ours"
aptly repo search orbit-local 2>/dev/null | while read -r ref; do
    name=${ref%%_*}
    grep -qx "$name" "$ours" || aptly repo remove orbit-local "Name (= $name)" >/dev/null
done
# And of each package only the newest version stays.
aptly repo search -format='{{.Package}} {{.Version}}' orbit-local 2>/dev/null | sort -k1,1 | while read -r name version; do
    versions=$(aptly repo search -format='{{.Version}}' orbit-local "Name (= $name)")
    newest=""
    for v in $versions; do
        [[ -z "$newest" ]] || dpkg --compare-versions "$v" gt "$newest" && newest=$v
    done
    for v in $versions; do
        dpkg --compare-versions "$v" lt "$newest" && aptly repo remove orbit-local "Name (= $name), \$Version (= $v)" >/dev/null
    done
    true
done

log "What the packages need from Debian"
wanted=$(mktemp)
names "$DIR/debian.list" > "$wanted"
{
    for deb in "$INCOMING"/{orbit,github,vendors}/*.deb; do
        [[ -f "$deb" ]] || continue
        for field in Depends Pre-Depends Recommends; do
            dpkg-deb -f "$deb" "$field" 2>/dev/null
        done
    done
} | tr ',|' '\n\n' | sed -E 's/\(.*\)//; s/:any//; s/^ +| +$//g' | sed '/^$/d' >> "$wanted"
# Names other packages only provide are resolved to one that does.
index=$(mktemp)
for suite in "$DIST" "$DIST-updates"; do
    for c in $COMPONENTS; do
        curl -fsSL "$DEBIAN/dists/$suite/$c/binary-amd64/Packages.xz" 2>/dev/null | xz -dc
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
filter="Priority (required) | Priority (important) | Priority (standard)"
while read -r name; do filter+=" | Name (= $name)"; done < "$wanted.real"
echo "$(wc -l < "$wanted.real") packages asked for, besides the base priorities"
rm -f "$wanted" "$wanted.real" "$index" "$ours"

log "Debian mirrors (filtered, with dependencies)"
for spec in "debian $DEBIAN $DIST" "debian-updates $DEBIAN $DIST-updates" "debian-security $SECURITY $DIST-security"; do
    read -r name url suite <<< "$spec"
    if [[ "$name" != debian ]]; then
        # Updates take every package the base has, so the binaries of one
        # source never mix versions (libpng-dev must match libpng16).
        filter="Name (= __none__)"
        while read -r have; do filter+=" | Name (= $have)"; done < <(aptly mirror search debian 'Name (% *)' | cut -d_ -f1 | sort -u)
    fi
    if mirror_exists "$name"; then
        aptly mirror edit -filter="$filter" -filter-with-deps "$name"
    else
        # The security archive lists its components its own way.
        aptly mirror create -architectures=amd64 -filter="$filter" -filter-with-deps -force-components \
            "$name" "$url" "$suite" $COMPONENTS
    fi
    aptly mirror update -max-tries=3 "$name" | grep -E "^(Download|ERROR|WARNING)" | grep -vE "^Downloading: .*\.deb$" || true
done

log "Snapshots and publication"
aptly snapshot create "debian-$STAMP" from mirror debian
aptly snapshot create "debian-updates-$STAMP" from mirror debian-updates
aptly snapshot create "debian-security-$STAMP" from mirror debian-security
aptly snapshot merge -latest "debian-all-$STAMP" "debian-$STAMP" "debian-updates-$STAMP" "debian-security-$STAMP"
aptly snapshot create "orbit-local-$STAMP" from repo orbit-local
publish debian "debian-all-$STAMP"
publish orbit "orbit-local-$STAMP"
gpg --export "$KEY" > /srv/aptly/public/orbit-archive-keyring.gpg

log "Old snapshots"
# The last 7 of each kind stay (to switch back to); packages none of them
# holds leave the pool.
for kind in debian debian-updates debian-security debian-all orbit-local; do
    aptly snapshot list -raw -sort=time | grep -E "^$kind-[0-9]{8}-[0-9]{6}$" | head -n -7 | while read -r old; do
        aptly snapshot drop "$old" >/dev/null 2>&1 && echo "dropped $old"
    done
done
aptly db cleanup >/dev/null

log "Done"
aptly snapshot show "debian-all-$STAMP" | grep -i "number of packages" || true
aptly snapshot show "orbit-local-$STAMP" | grep -i "number of packages" || true
du -sh /srv/aptly/pool 2>/dev/null || true
