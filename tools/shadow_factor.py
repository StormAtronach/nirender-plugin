"""The shadow factor of each of two renderers, from four pictures of the same view: with the
shadows and without them, on each side. The factor of a pixel is its brightness with the
shadows over its brightness without them: 1 in the light, less in a shadow. The colour of
the surface and the light on it cancel, so the two factors compare directly.

    python tools/shadow_factor.py <a on> <a off> <b on> <b off> [out.png] [name:x,y,w,h ...]

Prints the factor of a against the factor of b in classes of the factor of b, and in the
regions named. The picture is the factor of a less the factor of b, grey where equal,
lighter where the shadow of a is weaker.
"""
import sys

import numpy as np
from PIL import Image


def load(path):
    return np.asarray(Image.open(path).convert("RGB"), dtype=np.float32)


def main():
    a_on, a_off, b_on, b_off = (load(p) for p in sys.argv[1:5])
    rest = sys.argv[5:]
    out = rest.pop(0) if rest and rest[0].lower().endswith(".png") else None
    h = min(p.shape[0] for p in (a_on, a_off, b_on, b_off))
    w = min(p.shape[1] for p in (a_on, a_off, b_on, b_off))
    a_on, a_off, b_on, b_off = (p[:h, :w].mean(axis=2) for p in (a_on, a_off, b_on, b_off))
    # A pixel too dark without the shadows has no factor to speak of: one step of 255 is
    # more than 4 % of it.
    lit = (a_off >= 24) & (b_off >= 24)
    factor_a = np.where(lit, a_on / np.maximum(a_off, 1), 1.0)
    factor_b = np.where(lit, b_on / np.maximum(b_off, 1), 1.0)
    print("pixels with a factor: %.1f %%" % (100 * lit.mean()))
    print("classes of the factor of b: share of the pixels, the mean factor of b, of a, and (1 - a) / (1 - b)")
    for lo, hi in ((0.0, 0.3), (0.3, 0.4), (0.4, 0.5), (0.5, 0.6), (0.6, 0.7), (0.7, 0.8), (0.8, 0.9), (0.9, 0.97), (0.97, 1.03)):
        mask = lit & (factor_b >= lo) & (factor_b < hi)
        if mask.sum() < 200:
            continue
        fa, fb = factor_a[mask].mean(), factor_b[mask].mean()
        depth = (1 - fa) / (1 - fb) if abs(1 - fb) > 0.02 else float("nan")
        print("  b in [%.2f, %.2f): %5.1f %%   b %.3f   a %.3f   depth of a over b %.3f" % (lo, hi, 100 * mask.mean(), fb, fa, depth))
    for spec in rest:
        name, box = spec.split(":")
        x, y, bw, bh = (int(v) for v in box.split(","))
        region = (slice(y, y + bh), slice(x, x + bw))
        fa = a_on[region].mean() / a_off[region].mean()
        fb = b_on[region].mean() / b_off[region].mean()
        depth = (1 - fa) / (1 - fb) if abs(1 - fb) > 0.02 else float("nan")
        print("  %-10s off a %6.1f b %6.1f   factor a %.3f b %.3f   depth of a over b %.3f" % (name, a_off[region].mean(), b_off[region].mean(), fa, fb, depth))
    if out:
        Image.fromarray(np.clip(128 + (factor_a - factor_b) * 400, 0, 255).astype(np.uint8)).save(out)


if __name__ == "__main__":
    main()
