"""Compares two pictures of the same size and prints how different they are.

    python tools/picture_diff.py a.png b.png [--sheet sheet.png] [--mask x,y,w,h ...]

The numbers: the part of the pixels that are different at all, the part that are different by
more than 2, 8 and 32 of 255 in a colour, the mean difference, and the largest one. `--sheet`
saves the two pictures and their difference, made 8 times stronger, side by side. `--mask`
leaves a rectangle out of the comparison: a clock, a frame counter.

The exit code is 0. A caller reads the numbers with `compare`.
"""
import argparse
import sys

import numpy
from PIL import Image


def load(path):
    return numpy.asarray(Image.open(path).convert("RGB"), dtype=numpy.int16)


def compare(a, b, masks=()):
    """Returns the numbers of the comparison of two pictures as a dict."""
    if a.shape != b.shape:
        return {"problem": "sizes %s and %s" % (a.shape[1::-1], b.shape[1::-1])}
    difference = numpy.abs(a - b).max(axis=2)
    keep = numpy.ones(difference.shape, dtype=bool)
    for x, y, w, h in masks:
        keep[y:y + h, x:x + w] = False
    values = difference[keep]
    pixels = values.size
    return {
        "pixels": int(pixels),
        "different": float((values > 0).sum()) / pixels,
        "over2": float((values > 2).sum()) / pixels,
        "over8": float((values > 8).sum()) / pixels,
        "over32": float((values > 32).sum()) / pixels,
        "mean": float(numpy.abs(a - b).mean(axis=2)[keep].mean()),
        "largest": int(values.max()) if pixels else 0,
    }


def text(numbers):
    if "problem" in numbers:
        return numbers["problem"]
    return "different %.3f %%, over 2: %.3f %%, over 8: %.3f %%, over 32: %.3f %%, mean %.3f, largest %d" % (
        numbers["different"] * 100, numbers["over2"] * 100, numbers["over8"] * 100, numbers["over32"] * 100, numbers["mean"],
        numbers["largest"])


def sheet(a, b, path):
    """Saves a, b and their difference side by side, at half size."""
    difference = numpy.clip(numpy.abs(a - b) * 8, 0, 255)
    row = numpy.concatenate([a, b, difference], axis=1).astype(numpy.uint8)
    image = Image.fromarray(row)
    image.resize((image.width // 2, image.height // 2)).save(path)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("a")
    ap.add_argument("b")
    ap.add_argument("--sheet")
    ap.add_argument("--mask", action="append", default=[], help="x,y,w,h of a rectangle to leave out")
    args = ap.parse_args()
    a, b = load(args.a), load(args.b)
    masks = [tuple(int(v) for v in mask.split(",")) for mask in args.mask]
    print(text(compare(a, b, masks)))
    if args.sheet and a.shape == b.shape:
        sheet(a, b, args.sheet)
    return 0


if __name__ == "__main__":
    sys.exit(main())
