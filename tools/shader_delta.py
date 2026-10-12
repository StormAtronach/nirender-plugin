"""The effect of a post shader on each renderer: (with - without) of the helper against
(with - without) of MGE XE on Direct3D 9, so that the tone difference of the two renderers
is out of the comparison. Four pictures of the same place, hour and weather:

    python -I tools/shader_delta.py host_with.png host_without.png ref_with.png ref_without.png

The NPCs that walk between the runs are in the numbers; a place without them is better.
"""
import sys

import numpy as np
from PIL import Image


def load(path):
    return np.asarray(Image.open(path).convert("RGB"), dtype=np.float32)


def main():
    if len(sys.argv) != 5:
        print(__doc__)
        return 2
    pictures = [load(p) for p in sys.argv[1:5]]
    h = min(p.shape[0] for p in pictures)
    w = min(p.shape[1] for p in pictures)
    host_with, host_without, ref_with, ref_without = (p[:h, :w] for p in pictures)
    host = host_with - host_without
    ref = ref_with - ref_without
    print("helper: the shader changes the picture by a mean of %.2f (signed %+.2f)" % (np.abs(host).mean(), host.mean()))
    print("d3d9:   the shader changes the picture by a mean of %.2f (signed %+.2f)" % (np.abs(ref).mean(), ref.mean()))
    print("the two changes differ by a mean of %.2f" % np.abs(host - ref).mean())
    print("with: helper against d3d9 %.2f; without: %.2f" % (np.abs(host_with - ref_with).mean(), np.abs(host_without - ref_without).mean()))
    return 0


if __name__ == "__main__":
    sys.exit(main())
