"""Crops of the places where two pictures differ, side by side and enlarged, for a look at
a small difference (a few hundred pixels) that a whole-picture sheet does not show.

    python -I tools/diff_crops.py a.png b.png out.png [--threshold 8] [--most 6] [--zoom 4]

The pixels that differ by more than the threshold (the largest of the three channels) are
grouped in cells of 32 pixels; the cells with the most such pixels, and their neighbours,
give the boxes. Each row of the sheet: the box of a, of b, and the difference times 4.
Prints each box with its count.
"""
import argparse

import numpy
import os

# Pillow of OSGeo4W needs the DLLs of C:/OSGeo4W/bin. A shell of OSGeo4W gives them; a child
# process of another script does not (2026-10-16).
if os.name == "nt" and os.path.isdir(r"C:\OSGeo4W\bin"):
    os.add_dll_directory(r"C:\OSGeo4W\bin")
from PIL import Image


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("a")
    ap.add_argument("b")
    ap.add_argument("out")
    ap.add_argument("--threshold", type=int, default=8)
    ap.add_argument("--most", type=int, default=6)
    ap.add_argument("--zoom", type=int, default=4)
    args = ap.parse_args()
    a = numpy.asarray(Image.open(args.a).convert("RGB"), dtype=numpy.int16)
    b = numpy.asarray(Image.open(args.b).convert("RGB"), dtype=numpy.int16)
    diff = numpy.abs(a - b).max(axis=2)
    mask = diff > args.threshold
    h, w = mask.shape
    cell = 32
    counts = mask[: h // cell * cell, : w // cell * cell].reshape(h // cell, cell, w // cell, cell).sum(axis=(1, 3))
    order = numpy.argsort(counts, axis=None)[::-1]
    boxes = []
    for flat in order:
        cy, cx = divmod(int(flat), counts.shape[1])
        if counts[cy, cx] == 0 or len(boxes) >= args.most:
            break
        x0, y0 = max(0, (cx - 1) * cell), max(0, (cy - 1) * cell)
        if any(abs(x0 - bx) < 2 * cell and abs(y0 - by) < 2 * cell for bx, by, _ in boxes):
            continue
        boxes.append((x0, y0, int(counts[cy, cx])))
    size = 3 * cell
    rows = []
    for x0, y0, n in boxes:
        x1, y1 = min(w, x0 + size), min(h, y0 + size)
        print("box x %d-%d, y %d-%d: %d pixels over %d (in the middle cell)" % (x0, x1, y0, y1, n, args.threshold))
        parts = [a[y0:y1, x0:x1], b[y0:y1, x0:x1], numpy.clip(numpy.abs(a[y0:y1, x0:x1] - b[y0:y1, x0:x1]) * 4, 0, 255)]
        row = numpy.concatenate([numpy.pad(p, ((0, size - p.shape[0]), (0, size - p.shape[1]), (0, 0))) for p in parts], axis=1)
        rows.append(row)
    if not rows:
        print("no pixel over the threshold")
        return
    sheet = numpy.concatenate(rows, axis=0).astype(numpy.uint8)
    image = Image.fromarray(sheet)
    image = image.resize((image.width * args.zoom, image.height * args.zoom), Image.NEAREST)
    image.save(args.out)
    print("sheet %s (%d x %d)" % (args.out, image.width, image.height))


main()
