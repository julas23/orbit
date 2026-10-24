#!/usr/bin/env bash
# mimeapps.sh DATA — Orbit's default applications, from the MIME types its
# applications' desktop files take (DATA/io.github.julas23.Orbit.*.desktop),
# as an orbit-mimeapps.list: what a session with XDG_CURRENT_DESKTOP=orbit
# opens files with unless the user chose otherwise. Later ones win a type
# more than one takes (text: Edit over Read); the web stays the browser's.
set -euo pipefail
DATA=${1:?the data folder}
echo "# Orbit's default applications (orbit-defaults); ~/.config/mimeapps.list wins."
echo "[Default Applications]"
for app in Read View Play Files Edit; do
    desktop="io.github.julas23.Orbit.$app.desktop"
    sed -n 's/^MimeType=//p' "$DATA/$desktop" | tr ';' '\n' | grep -v '^$' | sed "s|\$|=$desktop|"
done | awk -F= '{ last[$1] = $2; if (!($1 in seen)) { seen[$1] = 1; order[++n] = $1 } } END { for (i = 1; i <= n; i++) print order[i] "=" last[order[i]] }'
