#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Generate PNG smoke-test fixtures for thumbnail conversion memory testing.

What changed, and why the old fixtures no longer test the thing
---------------------------------------------------------------
This script used to emit only 8000×8000 squares, because the bound that
mattered was the decode: `maxConcurrentConversions × maxDecodedPixels × 8`.
The binding constraint is now the *resample*, and it is a byte budget
(`MAX_TRANSFORM_PEAK_BYTES`, 1 GiB) rather than a pixel count:

    source   = source_w × source_h × source_bytes_per_pixel
    working  = source_w × new_h × 16      # Rgba32F, image::imageops::resize
    result   = new_w × new_h × source_bytes_per_pixel
    peak     = source + working + result          (all three resident at once)

`working` takes its width from the *source*, so it does not shrink with the
requested output. A square source cannot make that term large: at 8000 wide
and a 512-high target it is 65 MiB. Every fixture this script used to produce
was therefore incapable of exercising the budget at all — they all pass with
three-quarters of it unused.

What makes the term large is a **wide** source, and only under a transform
that resizes to an exact target height. That is the matched pair below.

Aspect ratio decides the verdict, not size
------------------------------------------
`wide-64mp-over` and `wide-64mp-under` have *identical* pixel counts
(63,963,136), identical decode cost and identical source bytes. They differ
only in shape, and they land on opposite sides of the budget. If a change ever
makes those two behave the same way, the bound has stopped measuring shape.

`tall-64mp` is the transpose of `wide-64mp-over`, and which way the asymmetry
runs depends on the transform. Under SQUARE and RESIZE the working term follows
the source's *width* against a fixed target height, so wide is dangerous and
tall is not — the transpose passes with 249 MiB where the original needs 1269.
Under CROP the asymmetry disappears: CROP scales to *cover* the box, so
whichever dimension is already long is scaled up rather than down, and both
extremes are rejected. Do not read the wide/tall asymmetry as general.

FIT is safer than the transform it replaced
-------------------------------------------
FIT with `preserve_aspect_ratio` scales by `min(w_ratio, h_ratio)`, so for a
wide source `new_h = source_h × target / source_w` and the working term
collapses to `source_h × target × 16` — bounded by the *shorter* dimension.
SQUARE and RESIZE take the exact target height, and CROP scales to *cover*,
which makes both terms grow with the source's long side. So `wide-64mp-over`
is accepted under FIT (now the chart default) and rejected under SQUARE and
CROP (the previous default). Both paths still matter: a caller can request any
transform per specification.

JPEG: the cheap source, and the reason the wide fixtures are PNG
----------------------------------------------------------------
JPEG is the *least* expensive source per pixel, not the most. It carries no
alpha, so `flatten_for_output` returns early for any JPEG source regardless of
the requested output, and it decodes at 3 B/px (RGB8) or 1 B/px (Luma8) rather
than 4. The 8 B/px figure the worker memory limit is sized on belongs to the
*output* side: JPEG **output** is what forces an alpha-carrying source to
flatten. The pairing that reaches it is a La16 PNG source with JPEG output, not
a JPEG source at all.

JPEG also cannot express the dangerous shape. libjpeg refuses any dimension
above 65,500 — verified, not inferred from the 16-bit SOF field's 65,535 — and
that caps the working buffer. The widest JPEG that can exist, 65500×977 at
64 MP, peaks at 695 MiB against a 512 target: 68% of the budget, accepted.
**No JPEG source can exceed the transform budget at the configured
THUMBNAIL_SIZES.** The pathological-shape risk belongs to formats that allow
larger dimensions, which is why `wide-64mp-over` is a PNG and has no JPEG twin.

What JPEG is genuinely useful for here is the decompression-bomb axis. A smooth
gradient at 64 MP is about 1 MiB encoded and 183 MiB decoded, where the PNG
equivalent needs ~32 MiB on disk to force the same decode. That makes the two
gates visibly independent — `MAX_SOURCE_BYTES` measures the encoded file,
`MAX_DECODED_PIXELS` the decoded one — and makes bulk upload testing cheap.

On-disk size vs decoded size
----------------------------
File size on disk is not what drives peak memory; the decoded footprint is.
On-disk size matters only for the source-byte gate, `MAX_SOURCE_BYTES`
(64 MiB, checked against the encoded PNG). The per-stripe noise trick keeps
these files well under it while still forcing a full-size decode: within a
stripe the RGB bytes repeat, so the compressor finds back-references, while
across stripes the data is genuinely random.

Usage
-----
  python3 scripts/gen-smoke-pngs.py [output-dir] [--target N] [--stripe K]
                                    [--slots N] [--only NAME ...] [--list]

  output-dir   Destination (default: _testdata/64MP-png)
  --target N   Largest configured THUMBNAIL_SIZES dimension, used for the
               predicted verdicts (default: 512, the chart and dev maximum)
  --stripe K   Rows per unique noise stripe (default: 8). Higher K → smaller
               files; lower K → larger.
  --slots N    Copies of the square RGBA fixture, one per conversion slot
               (default: 2, matching maxConcurrentConversions)
  --only NAME  Generate only the named fixtures (repeatable)
  --list       Print the fixture table with predicted verdicts and exit
"""

import argparse
import os
import struct
import zlib

# Must track crates/thumbnail-convert/src/bounds.rs.
RESAMPLE_WORKING_BYTES_PER_PIXEL = 16
MAX_TRANSFORM_PEAK_BYTES = 1024 * 1024 * 1024  # 1 GiB
MAX_DECODED_PIXELS = 64 * 1_000_000
MAX_SOURCE_BYTES = 64 * 1024 * 1024
# libjpeg's hard ceiling per dimension. Lower than the 65,535 the SOF marker's
# 16-bit fields would allow; confirmed by libjpeg refusing 65,535 outright.
JPEG_MAX_DIMENSION = 65500

# Decoded bytes per pixel by fixture kind, which is what the peak model needs.
# JPEG decodes to RGB8 and never flattens (no alpha to flatten). La16 decodes at
# 4 B/px, which is why max_alloc admits it at 64 MP where 16-bit RGBA is refused.
BYTES_PER_PIXEL = {"rgba": 4, "rgb": 3, "la16": 4, "jpeg": 3}

MIB = 1024 * 1024


# ---------------------------------------------------------------------------
# Peak model — mirrors DecodeLimits::check_resample
# ---------------------------------------------------------------------------

def resample_peak(source_w, source_h, new_w, new_h, source_bpp=4, extra=0):
    """Bytes resident at the resample's peak, per the formula in bounds.rs."""
    source = source_w * source_h * source_bpp
    working = source_w * new_h * RESAMPLE_WORKING_BYTES_PER_PIXEL
    result = new_w * new_h * source_bpp
    return source + working + result + extra


def _scaled(value, scale):
    return max(1, round(value * scale))


def target_dims(transform, source_w, source_h, target):
    """The (new_w, new_h) each transform hands to the resample, for a square box."""
    if transform == "FIT":
        scale = min(target / source_w, target / source_h)
        return _scaled(source_w, scale), _scaled(source_h, scale)
    if transform in ("SQUARE", "RESIZE"):
        return target, target
    if transform == "CROP":
        # Scales to cover the box, then crops; the resample sees the cover size.
        scale = max(target / source_w, target / source_h)
        return _scaled(source_w, scale), _scaled(source_h, scale)
    raise ValueError(transform)


TRANSFORMS = ("FIT", "SQUARE", "CROP")


def verdicts(source_w, source_h, target, source_bpp=4):
    out = {}
    for t in TRANSFORMS:
        new_w, new_h = target_dims(t, source_w, source_h, target)
        peak = resample_peak(source_w, source_h, new_w, new_h, source_bpp)
        out[t] = (peak, peak <= MAX_TRANSFORM_PEAK_BYTES)
    return out


# ---------------------------------------------------------------------------
# PNG writer
# ---------------------------------------------------------------------------

def _chunk(tag, data):
    payload = tag + data
    crc = zlib.crc32(payload) & 0xFFFFFFFF
    return struct.pack(">I", len(data)) + payload + struct.pack(">I", crc)


def write_png(path, width, height, rows_iter, color_type=None, bit_depth=8):
    """Write a PNG.

    ``rows_iter`` yields one raw scanline per row, without a filter byte; the
    writer prepends filter 0 and compresses the stream.

    ``color_type`` is inferred for the 8-bit cases (3 bytes/px → RGB,
    4 → RGBA) but must be given explicitly for anything else: La16 is also
    4 bytes per pixel and would otherwise be mistaken for RGBA.
    """
    first_row = next(rows_iter)
    if color_type is None:
        color_type = 2 if len(first_row) // width == 3 else 6

    signature = b"\x89PNG\r\n\x1a\n"
    ihdr = _chunk(
        b"IHDR",
        struct.pack(">IIBBBBB", width, height, bit_depth, color_type, 0, 0, 0),
    )
    iend = _chunk(b"IEND", b"")

    # level=6 rather than 1: within a stripe rows share their RGB bytes, and
    # only the higher level finds those back-references.
    comp = zlib.compressobj(level=6, wbits=15)
    idat_data = bytearray()
    idat_data.extend(comp.compress(b"\x00" + first_row))
    for row in rows_iter:
        idat_data.extend(comp.compress(b"\x00" + row))
    idat_data.extend(comp.flush())

    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    with open(path, "wb") as f:
        f.write(signature + ihdr + _chunk(b"IDAT", bytes(idat_data)) + iend)

    size = os.path.getsize(path)
    gate = "" if size <= MAX_SOURCE_BYTES else "  *** EXCEEDS MAX_SOURCE_BYTES ***"
    names = {2: "RGB", 4: "gray+alpha", 6: "RGBA"}
    print(
        f"    {width}×{height} {names[color_type]}{bit_depth}, "
        f"{width * height / 1e6:.1f} MP, {size / MIB:.1f} MiB on disk{gate}"
    )


# ---------------------------------------------------------------------------
# Row generators
# ---------------------------------------------------------------------------

def rgba_rows(width, height, hue_offset=0, stripe=8):
    """RGBA rows: per-stripe random RGB, per-row alpha gradient 128→240.

    ``base`` is immutable and copied per row so in-place edits cannot compound
    across rows — that bug makes every row unique and destroys compression.
    Alpha varying per row is what gives the flatten path real blending work.
    """
    base = b""
    for y in range(height):
        if y % stripe == 0:
            raw = bytearray(os.urandom(width * 4))
            r_shift = (y * 80 // max(height - 1, 1) + hue_offset) & 0xFF
            raw[0::4] = bytes((b + r_shift) & 0xFF for b in raw[0::4])
            base = bytes(raw)
        row = bytearray(base)
        row[3::4] = bytes([128 + y * 112 // max(height - 1, 1)] * width)
        yield bytes(row)


def rgb_rows(width, height, stripe=8):
    """RGB rows — no alpha, so the flatten path is skipped entirely."""
    base = b""
    for y in range(height):
        if y % stripe == 0:
            raw = bytearray(os.urandom(width * 3))
            raw[1::3] = bytes((b + y * 80 // max(height - 1, 1)) & 0xFF for b in raw[1::3])
            base = bytes(raw)
        row = bytearray(base)
        row[1::3] = bytes((b + y * 100 // max(height - 1, 1)) & 0xFF for b in base[1::3])
        yield bytes(row)


def write_jpeg(path, width, height, quality=85):
    """Write a smooth-gradient JPEG via Pillow.

    Pillow rather than a hand-rolled encoder: baseline JPEG needs DCT,
    quantisation and Huffman coding, which is not worth carrying here when the
    PNG path — the one that produces the fixtures that actually probe the
    bound — stays stdlib-only and works without it.

    A gradient, not noise: the point of a JPEG fixture is a small encoded file
    with a large decode, so compressing well is the feature. The PNG fixtures
    want the opposite and use per-stripe noise.
    """
    try:
        from PIL import Image, ImageDraw
    except ImportError:
        print(f"    SKIPPED: {os.path.basename(path)} needs Pillow "
              f"(pip install Pillow); the PNG fixtures do not")
        return
    if max(width, height) > JPEG_MAX_DIMENSION:
        raise ValueError(
            f"{width}×{height} cannot be a JPEG: libjpeg caps a dimension at "
            f"{JPEG_MAX_DIMENSION}. This shape has to be a PNG.")

    Image.MAX_IMAGE_PIXELS = None
    row = Image.new("RGB", (width, 1))
    draw = ImageDraw.Draw(row)
    for x in range(0, width, 64):
        t = x * 255 // max(width - 1, 1)
        draw.rectangle([x, 0, x + 63, 0], fill=(t, 128, 255 - t))
    row.resize((width, height)).save(path, "JPEG", quality=quality)

    size = os.path.getsize(path)
    gate = "" if size <= MAX_SOURCE_BYTES else "  *** EXCEEDS MAX_SOURCE_BYTES ***"
    print(f"    {width}×{height} JPEG q{quality}, {width * height / 1e6:.1f} MP, "
          f"{size / MIB:.1f} MiB on disk (decodes to "
          f"{width * height * 3 / MIB:.0f} MiB RGB8){gate}")


def la16_rows(width, height, stripe=8):
    """16-bit gray+alpha rows — the documented 8 B/px worst case.

    This layout is why the decoder's budget is denominated in bytes. `max_alloc`
    is `pixels × 4`, and La16 decodes at exactly 4 B/px, so 64 MP passes; 16-bit
    RGBA (8 B/px) is refused at the same pixel count. Flattening La16 for an
    output encoding without alpha allocates a 4 B/px RGBA8 buffer while the
    original is still alive — 8 B/px resident, the figure worker memory limits
    are sized on. Request **JPEG** output to take that path; PNG skips it.
    """
    base = b""
    for y in range(height):
        if y % stripe == 0:
            base = bytes(os.urandom(width * 2))  # 16-bit gray, 2 B/px
        a = 32768 + y * 32767 // max(height - 1, 1)
        a_hi, a_lo = (a >> 8) & 0xFF, a & 0xFF
        row = bytearray(width * 4)
        row[0::4] = base[0::2]
        row[1::4] = base[1::2]
        row[2::4] = bytes([a_hi] * width)
        row[3::4] = bytes([a_lo] * width)
        yield bytes(row)


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------
# 131072×488 and 65536×976 are both exactly 63,963,136 pixels: same decode
# cost, opposite verdicts. That equality is the point — keep it if you edit.

FIXTURES = [
    ("wide-64mp-over", 131072, 488, "rgba",
     "Wide. Rejected by the transform budget under an exact-target transform; "
     "accepted under FIT, whose scale collapses new_h."),
    ("wide-64mp-under", 65536, 976, "rgba",
     "Half as wide, same pixel count as wide-64mp-over. Accepted under every "
     "transform — the matched half of the pair."),
    ("tall-64mp", 488, 131072, "rgba",
     "Transpose of wide-64mp-over. Accepted under SQUARE, where the working "
     "buffer follows the source width; still rejected under CROP, which scales "
     "to cover and so stretches the long dimension further."),
    ("pano-16x9-64mp", 10666, 6000, "rgba",
     "16:9. Under FIT the output is letterboxed with transparent padding "
     "top and bottom rather than cropped — the visible half of the FIT change."),
    ("portrait-9x16-64mp", 6000, 10666, "rgba",
     "9:16. FIT pads left and right instead."),
    ("square-64mp-rgb", 8000, 8000, "rgb",
     "No alpha; skips flatten. The decode-ceiling case, exactly 64 MP."),
    ("jpeg-64mp-widest", JPEG_MAX_DIMENSION, 977, "jpeg",
     "The widest JPEG that can exist (libjpeg refuses >65500). Accepted at 68% "
     "of the budget, which is the point: no JPEG reaches the bound at 512."),
    ("jpeg-64mp-square", 8000, 8000, "jpeg",
     "64 MP from about 1 MiB on disk. The decompression-bomb case, and the cheap "
     "fixture for bulk upload testing."),
    ("square-64mp-la16", 8000, 8000, "la16",
     "16-bit gray+alpha. The 8 B/px flatten worst case the memory limit is "
     "sized on, and which no fixture previously produced. Request JPEG output."),
]

SQUARE_RGBA = ("square-64mp-rgba", 8000, 8000, "rgba",
               "Baseline square RGBA; one copy per conversion slot.")


def emit_table(target):
    print(f"Predicted verdicts at target {target}×{target}, per-fixture source layout,")
    print(f"against MAX_TRANSFORM_PEAK_BYTES = {MAX_TRANSFORM_PEAK_BYTES // MIB} MiB.\n")
    print(f"  {'fixture':<22} {'dimensions':>14} {'MP':>6}  " +
          "  ".join(f"{t:>16}" for t in TRANSFORMS))
    for name, w, h, kind, _doc in FIXTURES + [SQUARE_RGBA]:
        v = verdicts(w, h, target, BYTES_PER_PIXEL[kind])
        cells = []
        for t in TRANSFORMS:
            peak, ok = v[t]
            cells.append(f"{'ok ' if ok else 'REJECT'} {peak / MIB:>7.0f}MiB")
        over = "" if w * h <= MAX_DECODED_PIXELS else "  (over decode limit!)"
        print(f"  {name:<22} {f'{w}×{h}':>14} {w * h / 1e6:>6.1f}  " +
              "  ".join(f"{c:>16}" for c in cells) + over)
    print("\n  ok = conversion succeeds; REJECT = permanent TransformBudgetExceeded,")
    print("  a clean failure with a reason rather than an OOM-killed replica.")


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("output_dir", nargs="?", default="_testdata/64MP-png")
    parser.add_argument("--target", type=int, default=512, metavar="N",
                        help="Largest THUMBNAIL_SIZES dimension (default: 512)")
    parser.add_argument("--stripe", type=int, default=8, metavar="K",
                        help="Rows per unique noise stripe (default: 8)")
    parser.add_argument("--slots", type=int, default=2, metavar="N",
                        help="Copies of the square RGBA fixture (default: 2)")
    parser.add_argument("--only", action="append", metavar="NAME",
                        help="Generate only these fixtures (repeatable)")
    parser.add_argument("--list", action="store_true",
                        help="Print the fixture table and exit")
    args = parser.parse_args()

    if args.stripe < 1:
        parser.error("--stripe must be >= 1")
    if args.slots < 0:
        parser.error("--slots must be >= 0")
    if args.target < 1:
        parser.error("--target must be >= 1")

    if args.list:
        emit_table(args.target)
        return

    planned = list(FIXTURES)
    for i in range(1, args.slots + 1):
        name, w, h, kind, doc = SQUARE_RGBA
        planned.append((f"{name}-{i}", w, h, kind, doc))

    if args.only:
        wanted = set(args.only)
        planned = [f for f in planned if f[0] in wanted]
        unknown = wanted - {f[0] for f in planned}
        if unknown:
            parser.error(f"unknown fixture(s): {', '.join(sorted(unknown))}")

    os.makedirs(args.output_dir, exist_ok=True)
    emit_table(args.target)
    print(f"\nGenerating {len(planned)} fixtures in '{args.output_dir}/' "
          f"(stripe={args.stripe})\n")

    for i, (name, w, h, kind, doc) in enumerate(planned, 1):
        print(f"[{i}/{len(planned)}] {name}.png — {doc}")
        path = os.path.join(args.output_dir, f"{name}.png")
        if kind == "rgba":
            write_png(path, w, h, rgba_rows(w, h, stripe=args.stripe))
        elif kind == "rgb":
            write_png(path, w, h, rgb_rows(w, h, stripe=args.stripe))
        elif kind == "la16":
            write_png(path, w, h, la16_rows(w, h, stripe=args.stripe),
                      color_type=4, bit_depth=16)
        elif kind == "jpeg":
            write_jpeg(os.path.join(args.output_dir, f"{name}.jpg"), w, h)
        else:
            raise ValueError(kind)

    print("\nDone. What to check when these run through the worker:")
    print("  * the REJECT cells fail permanently with TransformBudgetExceeded and")
    print("    the replica stays up — no restart, no redelivery of the same bytes")
    print("  * wide-64mp-over succeeds under FIT and fails under SQUARE/CROP,")
    print("    which is the whole point of the pair")
    print("  * pano/portrait thumbnails come back padded, not cropped")
    print("  * square-64mp-la16 with JPEG output exercises the 8 B/px flatten;")
    print("    a JPEG *source* never does, having no alpha to flatten")
    print("  * jpeg-64mp-square proves the two gates are independent: ~1 MiB")
    print("    encoded against a 64 MiB limit, 183 MiB decoded")


if __name__ == "__main__":
    main()
