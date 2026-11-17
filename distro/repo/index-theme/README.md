# Theme of the public index

https://rationalazimuth.com/orbit/ is nginx's `autoindex` of `/var/www/orbit/`
on the VPS. nginx injects these files into every listing page, and the
script rebuilds the page with Orbit's look (download card for the ISOs,
apt instructions, table of entries). Without JavaScript the plain listing
still gets the colours.

On the VPS the files live in `/var/www/orbit/.theme/` (autoindex hides
names starting with a dot). Inside `location /orbit/` of
`/etc/nginx/sites-available/zz-rationalazimuth`:

```nginx
sub_filter_once on;
sub_filter '<html>' '<!DOCTYPE html><html lang="en">';
sub_filter '<head>' '<head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><link rel="icon" href="/orbit/.theme/mark.svg"><link rel="stylesheet" href="/orbit/.theme/orbit.css"><script defer src="/orbit/.theme/orbit.js"></script>';
```

Updating the theme:

```sh
scp -i ~/.ssh/ryzen distro/repo/index-theme/{orbit.css,orbit.js,mark.svg,mark-light.svg} ubuntu@129.80.87.20:/tmp/
ssh -i ~/.ssh/ryzen ubuntu@129.80.87.20 'sudo install -m 644 /tmp/{orbit.css,orbit.js,mark.svg,mark-light.svg} /var/www/orbit/.theme/'
```

ISOs go straight into `/var/www/orbit/` with their `.sha256` next to them;
the script shows each `.iso` as a download card and reads its checksum.

## Screenshots

`/var/www/orbit/screenshots/` holds the pictures of the site's Orbit gallery
(png, and the wallpapers as animated JXL). orbit.js shows any folder with
pictures as a grid with a viewer, and puts four of them under the ISO at the
top level ("See Orbit before you download"). The grid uses the folder's hidden
`.thumbs/` (WebP, 960 px), so the full files only load when one is opened:

```sh
python3 distro/repo/index-theme/make-thumbs.py FOLDER     # writes FOLDER/.thumbs/
tar -C FOLDER -czf - . | ssh -i ~/.ssh/ryzen ubuntu@129.80.87.20 \
  'T=$(mktemp -d); tar -C "$T" -xzf -; sudo install -d -m 755 /var/www/orbit/screenshots/.thumbs;
   sudo install -m 644 "$T"/*.* /var/www/orbit/screenshots/; sudo install -m 644 "$T"/.thumbs/* /var/www/orbit/screenshots/.thumbs/; rm -rf "$T"'
```
