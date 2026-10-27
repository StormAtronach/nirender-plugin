"""A small BC7 encoder and decoder for the test textures (mode 6 only): the BC7 test NIFs of
`make_test_nifs.py`. Not a quality encoder: each block takes the bounding box of its colours
as its two end points and the nearest of the 16 colours between them for each pixel.

    python -I tools/bc7.py --self-test

BC7 mode 6 (one subset, RGBA): 7 mode bits (six 0, then 1), the end points R0 R1 G0 G1 B0
B1 A0 A1 of 7 bits each, one p-bit for each end point (the low bit of all its channels),
then 16 indices of 4 bits; the first index has 3 bits, its top bit is 0 (the anchor). The
bits go from the lowest bit of byte 0 up. A colour between the end points:
((64 - w) * e0 + w * e1 + 32) >> 6, with w from WEIGHTS.
"""
import struct
import sys

WEIGHTS = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64]

# DXGI_FORMAT_BC7_UNORM and DXGI_FORMAT_BC7_UNORM_SRGB.
DXGI_BC7 = 98
DXGI_BC7_SRGB = 99


def _palette(e0, e1):
    return [tuple(((64 - w) * a + w * b + 32) >> 6 for a, b in zip(e0, e1)) for w in WEIGHTS]


def _quantize(colour):
    """An end point of 8 bits a channel as 7 bits a channel and its p-bit (the low bit of all
    four channels): the p-bit with the smaller error."""
    best = None
    for p in (0, 1):
        q = tuple(max(0, min(127, (c - p + 1) >> 1)) for c in colour)
        error = sum((((v << 1) | p) - c) ** 2 for v, c in zip(q, colour))
        if best is None or error < best[0]:
            best = (error, q, p)
    return best[1], best[2]


def _distance(a, b):
    return sum((x - y) ** 2 for x, y in zip(a, b))


def encode_block(pixels):
    """16 RGBA pixels (row by row) as the 16 bytes of one mode-6 block."""
    # The end points: the two pixels farthest apart (a bounding box fails for two colours
    # whose channels move against each other, such as red and blue).
    low, high = max(((a, b) for a in pixels for b in pixels), key=lambda pair: _distance(*pair))
    (q0, p0), (q1, p1) = _quantize(low), _quantize(high)
    e0, e1 = tuple(c << 1 | p0 for c in q0), tuple(c << 1 | p1 for c in q1)
    palette = _palette(e0, e1)
    indices = [min(range(16), key=lambda i: sum((palette[i][k] - p[k]) ** 2 for k in range(4))) for p in pixels]
    if indices[0] >= 8:
        # The anchor's top bit must be 0: swap the end points and turn each index round.
        q0, q1, p0, p1 = q1, q0, p1, p0
        indices = [15 - i for i in indices]
    bits = 0
    at = 0

    def put(value, count):
        nonlocal bits, at
        bits |= (value & ((1 << count) - 1)) << at
        at += count

    put(1 << 6, 7)
    for k in range(4):
        put(q0[k], 7)
        put(q1[k], 7)
    put(p0, 1)
    put(p1, 1)
    put(indices[0], 3)
    for i in indices[1:]:
        put(i, 4)
    assert at == 128
    return bits.to_bytes(16, "little")


def decode_block(block):
    """The 16 RGBA pixels of one mode-6 block."""
    bits = int.from_bytes(block, "little")
    at = 0

    def take(count):
        nonlocal at
        value = (bits >> at) & ((1 << count) - 1)
        at += count
        return value

    assert take(7) == 1 << 6, "not a mode-6 block"
    ends = [[0, 0], [0, 0], [0, 0], [0, 0]]
    for k in range(4):
        ends[k][0] = take(7)
        ends[k][1] = take(7)
    p0, p1 = take(1), take(1)
    e0 = tuple(ends[k][0] << 1 | p0 for k in range(4))
    e1 = tuple(ends[k][1] << 1 | p1 for k in range(4))
    palette = _palette(e0, e1)
    indices = [take(3)] + [take(4) for _ in range(15)]
    return [palette[i] for i in indices]


def encode_level(width, height, pixel):
    """The blocks of one level; `pixel(x, y)` gives an RGBA tuple. Pixels outside a level
    smaller than a block repeat its last row and column."""
    out = bytearray()
    for by in range(0, max(height, 4), 4):
        for bx in range(0, max(width, 4), 4):
            block = [pixel(min(bx + dx, width - 1), min(by + dy, height - 1)) for dy in range(4) for dx in range(4)]
            out += encode_block(block)
    return bytes(out)


def write_dds(path, size, pixel, dxgi_format):
    """A square BC7 DDS with the DX10 header and all its mipmaps. `pixel(level, x, y)`."""
    levels = []
    width = size
    level = 0
    while True:
        levels.append(encode_level(width, width, lambda x, y, level=level: pixel(level, x, y)))
        if width == 1:
            break
        width //= 2
        level += 1
    # DDSD_CAPS, HEIGHT, WIDTH, PIXELFORMAT, MIPMAPCOUNT and LINEARSIZE; DDPF_FOURCC with
    # 'DX10'; DDSCAPS_TEXTURE, COMPLEX and MIPMAP.
    header = struct.pack("<4s7I11I2I4s5I5I", b"DDS ", 124, 0x1007 | 0x20000 | 0x80000, size, size, len(levels[0]), 0, len(levels),
                         *([0] * 11), 32, 0x4, b"DX10", 0, 0, 0, 0, 0, 0x1000 | 0x8 | 0x400000, 0, 0, 0, 0)
    assert len(header) == 128
    # The DX10 header: the format, a 2D texture (3), no flags, one array slice, no alpha mode.
    dx10 = struct.pack("<5I", dxgi_format, 3, 0, 1, 0)
    with open(path, "wb") as f:
        f.write(header + dx10 + b"".join(levels))


def self_test():
    # A block of two colours gives them back within 1: one p-bit serves all four channels of
    # an end point, so 0 and 255 in one end point cannot both be exact. A gradient comes back
    # within the steps of the 16 colours.
    two = [(255, 0, 0, 255) if i % 2 else (0, 0, 255, 255) for i in range(16)]
    back = decode_block(encode_block(two))
    assert all(abs(a - b) <= 1 for p, q in zip(two, back) for a, b in zip(p, q)), back
    ramp = [(i * 17, 255 - i * 17, 128, 255) for i in range(16)]
    back = decode_block(encode_block(ramp))
    assert all(abs(a - b) <= 6 for p, q in zip(ramp, back) for a, b in zip(p, q)), back
    # The anchor: a block whose first pixel is the bright end swaps the end points.
    first_high = [(250, 250, 250, 255)] + [(5, 5, 5, 255)] * 15
    assert decode_block(encode_block(first_high))[0][0] >= 248
    print("bc7 self-test: ok")


if __name__ == "__main__":
    if "--self-test" in sys.argv:
        self_test()
    else:
        print(__doc__)
