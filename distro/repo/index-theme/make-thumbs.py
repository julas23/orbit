#!/usr/bin/env python3
"""Thumbnails for an image folder of the public index (e.g. screenshots/).

    python3 distro/repo/index-theme/make-thumbs.py FOLDER

Writes FOLDER/.thumbs/<name>.webp, at most 960 px wide, for every png, jpg,
webp, gif and jxl in FOLDER. orbit.js shows these in the gallery grid (and as
the still picture of an animated JXL where the browser has no JPEG XL), so
the page never pulls the full files until one is opened. nginx's autoindex
hides names starting with a dot, so .thumbs/ stays out of the listing.
An animated JXL is decoded with ffmpeg (its middle frame); Pillow does the rest.
"""
import io, pathlib, subprocess, sys
from PIL import Image

WIDTH = 960
IMAGES = {".png", ".jpg", ".jpeg", ".webp", ".gif", ".jxl"}

def still(path):
    if path.suffix.lower() != ".jxl":
        im = Image.open(path)
        im.seek(0)
        return im.convert("RGB")
    n = subprocess.run(["ffprobe", "-v", "error", "-count_frames", "-show_entries", "stream=nb_read_frames",
                        "-of", "csv=p=0", str(path)], capture_output=True, text=True, check=True).stdout.strip()
    mid = max(0, int(n or 1) // 2)
    png = subprocess.run(["ffmpeg", "-v", "error", "-i", str(path), "-vf", f"select=eq(n\\,{mid})",
                          "-vsync", "0", "-frames:v", "1", "-f", "image2pipe", "-c:v", "png", "-"],
                         capture_output=True, check=True).stdout
    return Image.open(io.BytesIO(png)).convert("RGB")

def main(folder):
    folder = pathlib.Path(folder)
    out = folder / ".thumbs"
    out.mkdir(exist_ok=True)
    for f in sorted(folder.iterdir()):
        if f.suffix.lower() not in IMAGES:
            continue
        im = still(f)
        if im.width > WIDTH:
            im = im.resize((WIDTH, round(im.height * WIDTH / im.width)), Image.LANCZOS)
        dest = out / (f.stem + ".webp")
        im.save(dest, "WEBP", quality=82, method=6)
        print(f"{f.name} -> .thumbs/{dest.name} ({im.width}x{im.height}, {dest.stat().st_size // 1024} KiB)")

if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
