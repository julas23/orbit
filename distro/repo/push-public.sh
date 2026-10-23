#!/usr/bin/env bash
# push-public.sh — copies the public publication of the repository server
# (orbit-public, see sync.sh) to the public web server, where it is
# https://rationalazimuth.com/orbit. Run it from a machine that reaches both
# (the copy goes through it), after sync.sh has published:
#
#   distro/repo/push-public.sh [REPO_HOST] [WEB_HOST:DIR]
#
# The pool goes first and the indexes after, so a client never finds an
# index listing a file not there yet; what left the publication leaves the
# web server last.
#
# Clients:
#   curl -fsSL https://rationalazimuth.com/orbit/orbit-archive-keyring.gpg \
#       | sudo tee /usr/share/keyrings/orbit-archive-keyring.gpg >/dev/null
#   echo "deb [signed-by=/usr/share/keyrings/orbit-archive-keyring.gpg] https://rationalazimuth.com/orbit trixie main" \
#       | sudo tee /etc/apt/sources.list.d/orbit.list
set -euo pipefail

REPO=${1:-root@10.0.0.111}
WEB=${2:-ubuntu@157.151.17.181:/var/www/orbit}
WEB_SSH=${WEB_SSH:-ssh -i $HOME/.ssh/ryzen}
STAGE=${XDG_CACHE_HOME:-$HOME/.cache}/orbit-public

mkdir -p "$STAGE"
rsync -a --delete "$REPO:/srv/aptly/public/orbit-public/" "$STAGE/"
rsync -a "$REPO:/srv/aptly/public/orbit-archive-keyring.gpg" "$STAGE/"
rsync -a -e "$WEB_SSH" "$STAGE/pool" "$STAGE/orbit-archive-keyring.gpg" "$WEB/"
rsync -a -e "$WEB_SSH" --delete "$STAGE/dists" "$WEB/"
rsync -a -e "$WEB_SSH" --delete "$STAGE/" "$WEB/"
echo "published: $(find "$STAGE/pool" -name '*.deb' | wc -l) packages, $(du -sh "$STAGE" | cut -f1)"
