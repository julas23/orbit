#!/usr/bin/env bash
# install-login.sh — installs Orbit-Login in the place of the getty's
# autologin on tty1 (PLANO-CORE §7.4). As root, after a release build:
#
#   cargo build --release --workspace
#   su -c 'scripts/install-login.sh juliano'
#
# Root runs orbit-login, so it goes to /usr/local/bin, root's own; so does
# the whole rack (core, greet, panel, dock, the applications…), for the
# users without a copy in ~/.local/bin: any user who logs in gets the
# desktop, dark and with its font. /etc/orbit/login.yaml is written only
# if missing.
#
#   scripts/install-login.sh --remove    back to the getty's autologin
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
[ "$(id -u)" = 0 ] || { echo "install-login.sh: run it as root" >&2; exit 1; }

if [ "${1:-}" = --remove ]; then
  systemctl disable orbit-login.service
  rm -f /etc/systemd/system/orbit-login.service /usr/local/bin/orbit-login /etc/pam.d/orbit-login
  systemctl daemon-reload
  echo "removed; tty1 is the getty's again on the next boot"
  exit 0
fi

USER_NAME=${1:?the user opened at boot, e.g. juliano}
RACK="orbit-login orbit-core orbit-greet orbit-polkit orbit-run orbit-prompter orbit-panel orbit-dock orbit-notify
  orbit-calc orbit-clip orbit-blink orbit-view orbit-read orbit-play orbit-edit orbit-files orbit-pathfinder
  orbit-term orbitctl orbit-local-station"
for bin in $RACK; do
  [ -x "$ROOT/target/release/$bin" ] || { echo "no target/release/$bin: build it first" >&2; exit 1; }
done
for bin in $RACK; do
  install -o root -g root -m 755 "$ROOT/target/release/$bin" /usr/local/bin/
done
install -o root -g root -m 755 "$ROOT/scripts/orbit-session" /usr/local/bin/
mkdir -p /usr/local/share/applications /usr/local/share/dbus-1/services
install -o root -g root -m 644 "$ROOT"/data/io.github.julas23.Orbit.*.desktop /usr/local/share/applications/
command -v update-desktop-database >/dev/null && update-desktop-database -q /usr/local/share/applications || true
sed 's|^Exec=/usr/bin/|Exec=/usr/local/bin/|' "$ROOT/data/dbus/org.gnome.keyring.SystemPrompter.service" \
  > /usr/local/share/dbus-1/services/org.gnome.keyring.SystemPrompter.service
# Pathfinder's Ferdium recipes, in the system's data dirs.
rm -rf /usr/local/share/orbit-pathfinder/recipes
mkdir -p /usr/local/share/orbit-pathfinder
cp -r "$ROOT/data/pathfinder/recipes" /usr/local/share/orbit-pathfinder/recipes
# Dark by default, as Orbit is drawn, for users who have not chosen.
install -o root -g root -m 644 "$ROOT/data/gsettings/90_orbit.gschema.override" /usr/share/glib-2.0/schemas/
glib-compile-schemas /usr/share/glib-2.0/schemas
# The panel's and the apps' font (its icons too), for every user: taken
# from USER_NAME's own fonts until Orbit ships it (orbit-fonts).
HOME_DIR=$(getent passwd "$USER_NAME" | cut -d: -f6)
FONTS=$(find "$HOME_DIR/.fonts" "$HOME_DIR/.local/share/fonts" -name 'CaskaydiaCoveNerdFont-*.ttf' -o -name 'CaskaydiaCoveNerdFontMono-*.ttf' 2>/dev/null || true)
if [ -n "$FONTS" ]; then
  mkdir -p /usr/local/share/fonts/orbit
  echo "$FONTS" | xargs -d '\n' install -o root -g root -m 644 -t /usr/local/share/fonts/orbit/
  fc-cache -f /usr/local/share/fonts/orbit
else
  echo "no CaskaydiaCove Nerd Font in $USER_NAME's fonts: other users' panels will miss their icons" >&2
fi
install -o root -g root -m 644 "$ROOT/data/pam/orbit-login" /etc/pam.d/orbit-login
[ -e /etc/pam.d/orbit-greet ] || install -o root -g root -m 644 "$ROOT/data/pam/orbit-greet" /etc/pam.d/orbit-greet
mkdir -p /etc/orbit
[ -e /etc/orbit/login.yaml ] || sed "s/^user: .*/user: $USER_NAME/" "$ROOT/data/login.yaml" > /etc/orbit/login.yaml
sed 's|/usr/bin/orbit-login|/usr/local/bin/orbit-login|' "$ROOT/data/systemd/orbit-login.service" > /etc/systemd/system/orbit-login.service
systemctl daemon-reload
systemctl enable orbit-login.service
echo "installed; on the next boot orbit-login opens $USER_NAME's session on tty1"
echo "(the getty's autologin drop-in is left as it is: Conflicts= keeps it off)"
