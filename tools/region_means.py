"""The mean colour of named regions of two pictures, and the ratio a/b: for the question
whether a solid surface differs, or only its edges. Arguments: a b name:x,y,w,h ..."""
import sys

import numpy as np
from PIL import Image

a = np.asarray(Image.open(sys.argv[1]).convert("RGB"), dtype=np.float32)
b = np.asarray(Image.open(sys.argv[2]).convert("RGB"), dtype=np.float32)
for spec in sys.argv[3:]:
    name, box = spec.split(":")
    x, y, w, h = (int(v) for v in box.split(","))
    ra, rb = a[y:y + h, x:x + w], b[y:y + h, x:x + w]
    ma, mb = ra.mean(axis=(0, 1)), rb.mean(axis=(0, 1))
    d = ra - rb
    print(f"{name:10s} a {ma[0]:6.1f} {ma[1]:6.1f} {ma[2]:6.1f}  b {mb[0]:6.1f} {mb[1]:6.1f} {mb[2]:6.1f}  a/b {ma[0] / mb[0]:.3f} {ma[1] / mb[1]:.3f} {ma[2] / mb[2]:.3f}  |d| {np.abs(d).mean():.2f}  pixels with |d|>6: {100 * (np.abs(d).max(axis=2) > 6).mean():.0f} %")
