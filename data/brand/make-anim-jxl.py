#!/usr/bin/env python3
"""Orbit's mark as an animated JPEG XL, moving like the boot screen
(distro/release/plymouth/orbit/orbit.script): the moon travels the outer orbit
counter-clockwise, 3.6 s a turn, and the open inner orbit turns clockwise, 2.2 s a turn.

    python3 data/brand/make-anim-jxl.py data/brand/orbit-mark-anim.jxl
    python3 data/brand/make-anim-jxl.py data/brand/orbit-mark-anim-light.jxl --light

The loop is always exact: its length is the LCM of the two periods, so the last
frame flows into the first. The boot screen's 2.2 s would need a 39.6 s loop
(~2.6 MB at 128 px), so the inner orbit turns in 2.4 s here, barely slower: the
loop closes in 7.2 s and the file is ~480 KB. --inner 2.2 gives the exact one.

Needs pycairo, Pillow and the system libjxl 0.11 (Debian: libjxl0.11), which it
calls through ctypes because Debian ships no cjxl. Lossless: every frame is the
exact render. Each frame after the first only carries the inner orbit's box and
the moon's old and new places, painted over the previous frame.
"""
import argparse, ctypes as C, math, os, struct
from fractions import Fraction
import cairo
from PIL import Image

# --------------------------------------------------------------------- the mark
DARK  = dict(core="#4682b4", inner="#6ea8e0", outer="#d8dee9", moon="#ff9500")  # orbit-mark.svg
LIGHT = dict(core="#4682b4", inner="#3a6f9f", outer="#1b1f27", moon="#e07b00")  # orbit-mark-light.svg
MOON_PERIOD = 3.6
MOON_START = math.atan2(84.4 - 50, 66.1 - 50)   # where the SVG draws the moon
DASH = 100 / 24                                 # stroke-dasharray 100 on r=24, in radians
INNER_START = math.radians(200)                 # transform="rotate(200 50 50)"

def rgb(h): return tuple(int(h[i:i + 2], 16) / 255 for i in (1, 3, 5))

def moon_at(t):
    a = MOON_START - 2 * math.pi * t / MOON_PERIOD   # y grows down: smaller angle = counter-clockwise
    return 50 + 38 * math.cos(a), 50 + 38 * math.sin(a)

def render(t, size, pal, inner_period):
    surf = cairo.ImageSurface(cairo.FORMAT_ARGB32, size, size)
    c = cairo.Context(surf)
    c.scale(size / 100, size / 100)              # the mark's 100-unit box
    c.set_line_width(6)
    c.set_source_rgb(*rgb(pal["core"])); c.arc(50, 50, 10, 0, 2 * math.pi); c.fill()
    c.set_source_rgb(*rgb(pal["outer"])); c.arc(50, 50, 38, 0, 2 * math.pi); c.stroke()
    a0 = INNER_START + 2 * math.pi * t / inner_period          # clockwise
    c.set_line_cap(cairo.LINE_CAP_ROUND)
    c.set_source_rgb(*rgb(pal["inner"])); c.arc(50, 50, 24, a0, a0 + DASH); c.stroke()
    mx, my = moon_at(t)
    c.set_source_rgb(*rgb(pal["moon"])); c.arc(mx, my, 6.5, 0, 2 * math.pi); c.fill()
    # cairo is premultiplied BGRA in memory; libjxl wants straight RGBA
    return Image.frombuffer("RGBA", (size, size), bytes(surf.get_data()), "raw", "BGRa", 0, 1)

def loop_seconds(p, q):
    """LCM of two decimal periods, e.g. 3.6 and 2.2 -> 39.6."""
    p, q = Fraction(str(p)), Fraction(str(q))
    num = p.numerator * q.numerator // math.gcd(p.numerator, q.numerator)
    return float(Fraction(num, math.gcd(p.denominator, q.denominator)))

# ------------------------------------------------- libjxl 0.11 encoder, via ctypes
LIB = "/usr/lib/x86_64-linux-gnu/"
jxl = C.CDLL(LIB + "libjxl.so.0.11")
thr = C.CDLL(LIB + "libjxl_threads.so.0.11")
vp, i32 = C.c_void_p, C.c_int
for name, res, args in [
    ("JxlEncoderCreate", vp, [vp]), ("JxlEncoderDestroy", None, [vp]),
    ("JxlEncoderInitBasicInfo", None, [vp]), ("JxlEncoderSetBasicInfo", i32, [vp, vp]),
    ("JxlColorEncodingSetToSRGB", None, [vp, i32]), ("JxlEncoderSetColorEncoding", i32, [vp, vp]),
    ("JxlEncoderInitExtraChannelInfo", None, [i32, vp]), ("JxlEncoderSetExtraChannelInfo", i32, [vp, C.c_size_t, vp]),
    ("JxlEncoderFrameSettingsCreate", vp, [vp, vp]), ("JxlEncoderSetFrameLossless", i32, [vp, i32]),
    ("JxlEncoderFrameSettingsSetOption", i32, [vp, i32, C.c_int64]),
    ("JxlEncoderInitFrameHeader", None, [vp]), ("JxlEncoderSetFrameHeader", i32, [vp, vp]),
    ("JxlEncoderAddImageFrame", i32, [vp, vp, vp, C.c_size_t]), ("JxlEncoderCloseInput", None, [vp]),
    ("JxlEncoderProcessOutput", i32, [vp, C.POINTER(C.POINTER(C.c_uint8)), C.POINTER(C.c_size_t)]),
    ("JxlEncoderSetParallelRunner", i32, [vp, vp, vp]), ("JxlEncoderGetError", i32, [vp]),
]:
    f = getattr(jxl, name); f.restype = res; f.argtypes = args
thr.JxlThreadParallelRunnerCreate.restype = vp
thr.JxlThreadParallelRunnerCreate.argtypes = [vp, C.c_size_t]
thr.JxlThreadParallelRunnerDestroy.argtypes = [vp]

def ok(status, what, enc):
    if status != 0:
        raise RuntimeError(f"libjxl: {what} failed (status {status}, error {jxl.JxlEncoderGetError(enc)})")

def put(buf, off, fmt, val): struct.pack_into(fmt, buf, off, val)

def write_jxl(path, frames, size, effort=7):
    """frames: (straight RGBA bytes, crop or None, duration ms). crop = (x0, y0, w, h).
    Every frame is kept in reference slot 1 and the next is painted over it.
    Struct fields are set at their offsets in libjxl 0.11's headers, on top of
    what libjxl's own Init* functions fill in."""
    enc = jxl.JxlEncoderCreate(None)
    runner = thr.JxlThreadParallelRunnerCreate(None, os.cpu_count() or 4)
    try:
        ok(jxl.JxlEncoderSetParallelRunner(enc, C.cast(thr.JxlThreadParallelRunner, vp), runner), "runner", enc)
        info = C.create_string_buffer(512)                         # JxlBasicInfo
        jxl.JxlEncoderInitBasicInfo(info)
        put(info, 4, "I", size); put(info, 8, "I", size)           # xsize, ysize
        put(info, 12, "I", 8)                                      # bits_per_sample
        put(info, 36, "i", 1)                                      # uses_original_profile (lossless)
        put(info, 44, "i", 1)                                      # have_animation
        put(info, 52, "I", 3); put(info, 56, "I", 1)               # colour channels, extra channels
        put(info, 60, "I", 8)                                      # alpha_bits
        put(info, 80, "I", 1000); put(info, 84, "I", 1)            # 1000 ticks a second
        put(info, 88, "I", 0)                                      # num_loops: forever
        ok(jxl.JxlEncoderSetBasicInfo(enc, info), "SetBasicInfo", enc)
        color = C.create_string_buffer(512)
        jxl.JxlColorEncodingSetToSRGB(color, 0)
        ok(jxl.JxlEncoderSetColorEncoding(enc, color), "SetColorEncoding", enc)
        eci = C.create_string_buffer(512)
        jxl.JxlEncoderInitExtraChannelInfo(0, eci)                 # JXL_CHANNEL_ALPHA
        ok(jxl.JxlEncoderSetExtraChannelInfo(enc, 0, eci), "SetExtraChannelInfo", enc)
        fs = jxl.JxlEncoderFrameSettingsCreate(enc, None)
        ok(jxl.JxlEncoderSetFrameLossless(fs, 1), "lossless", enc)
        ok(jxl.JxlEncoderFrameSettingsSetOption(fs, 0, effort), "effort", enc)   # JXL_ENC_FRAME_SETTING_EFFORT
        # JxlPixelFormat {num_channels, data_type = JXL_TYPE_UINT8, endianness, align}
        fmt = C.create_string_buffer(struct.pack("IIIxxxxQ", 4, 2, 0, 0), 24)
        layers = 0
        for px, crop, ms in frames:
            # JxlFrameHeader {duration, timecode, name_length, is_last, JxlLayerInfo {have_crop,
            #   crop_x0, crop_y0, xsize, ysize, JxlBlendInfo {blendmode, source, alpha, clamp},
            #   save_as_reference}}
            hdr = C.create_string_buffer(256)
            jxl.JxlEncoderInitFrameHeader(hdr)
            put(hdr, 0, "I", ms)
            if crop:
                x0, y0, w, h = crop
                put(hdr, 16, "i", 1); put(hdr, 20, "i", x0); put(hdr, 24, "i", y0)
                put(hdr, 28, "I", w); put(hdr, 32, "I", h)
                put(hdr, 36, "i", 0)                               # JXL_BLEND_REPLACE inside the crop
                put(hdr, 40, "I", 1)                               # outside it: slot 1
            put(hdr, 52, "I", 1)                                   # keep the result in slot 1
            ok(jxl.JxlEncoderSetFrameHeader(fs, hdr), "SetFrameHeader", enc)
            buf = C.create_string_buffer(px, len(px))
            ok(jxl.JxlEncoderAddImageFrame(fs, fmt, buf, len(px)), "AddImageFrame", enc)
            layers += 1
        jxl.JxlEncoderCloseInput(enc)
        out, chunk = bytearray(), (C.c_uint8 * (1 << 20))()
        while True:
            p, avail = C.cast(chunk, C.POINTER(C.c_uint8)), C.c_size_t(len(chunk))
            st = jxl.JxlEncoderProcessOutput(enc, C.byref(p), C.byref(avail))
            out += bytes(chunk[: len(chunk) - avail.value])
            if st == 0: break                                      # JXL_ENC_SUCCESS
            if st != 2: ok(st, "ProcessOutput", enc)               # 2 = need more output
        with open(path, "wb") as f: f.write(out)
        return layers, len(out)
    finally:
        jxl.JxlEncoderDestroy(enc)
        thr.JxlThreadParallelRunnerDestroy(runner)

# --------------------------------------------------------------------- main
def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("out")
    ap.add_argument("--light", action="store_true", help="colours of orbit-mark-light.svg")
    ap.add_argument("--size", type=int, default=128)
    ap.add_argument("--fps", type=int, default=25)
    ap.add_argument("--inner", type=float, default=2.4, help="seconds a turn of the inner orbit (boot screen: 2.2)")
    a = ap.parse_args()

    pal = LIGHT if a.light else DARK
    secs = loop_seconds(MOON_PERIOD, a.inner)
    n, ms, k = round(secs * a.fps), round(1000 / a.fps), a.size / 100

    def box(cx, cy, r):   # pixel box around a disc of r mark units, one pixel of antialias margin
        x0, y0 = max(0, math.floor((cx - r) * k) - 1), max(0, math.floor((cy - r) * k) - 1)
        x1, y1 = min(a.size, math.ceil((cx + r) * k) + 1), min(a.size, math.ceil((cy + r) * k) + 1)
        return x0, y0, x1 - x0, y1 - y0
    def union(b, c):
        x0, y0 = min(b[0], c[0]), min(b[1], c[1])
        return x0, y0, max(b[0] + b[2], c[0] + c[2]) - x0, max(b[1] + b[3], c[1] + c[3]) - y0
    def cut(img, b): return img.crop((b[0], b[1], b[0] + b[2], b[1] + b[3])).tobytes()

    inner_box = box(50, 50, 27.5)                # r 24 + half the stroke, round caps
    def frames():
        yield render(0, a.size, pal, a.inner).tobytes(), None, ms
        for i in range(1, n):
            t = i / a.fps
            img = render(t, a.size, pal, a.inner)
            yield cut(img, inner_box), inner_box, 0                       # held 0 ms
            mb = union(box(*moon_at((i - 1) / a.fps), 7.5), box(*moon_at(t), 7.5))
            yield cut(img, mb), mb, ms

    layers, nbytes = write_jxl(a.out, frames(), a.size)
    print(f"{a.out}: {n} frames ({layers} layers), {a.size}px, {a.fps} fps, "
          f"{secs:g} s loop, {nbytes / 1024:.0f} KiB")

if __name__ == "__main__":
    main()
