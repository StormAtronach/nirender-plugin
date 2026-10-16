"""The light values of a capture, two ways: the LIGHT records the plugin computes for the
retained table (keyed by the light index), and the lights inline in each SHAPE record, as
the engine set them for the draw. For each key seen both ways, the largest difference of
each part of the D3DLIGHT8 (diffuse, specular, ambient, position, direction, range,
falloff, attenuation, theta, phi).

    python -I tools/light_values_check.py <capture.nircap>
"""
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import capture_dump  # noqa: E402

TAG_LIGHT = 21
STATE_LIGHT_SIZE = 108
PARTS = [("type", 0, 1), ("diffuse", 1, 4), ("specular", 5, 4), ("ambient", 9, 4), ("position", 13, 3), ("direction", 16, 3), ("range", 19, 1),
         ("falloff", 20, 1), ("attenuation", 21, 3), ("theta", 24, 1), ("phi", 25, 1)]


def floats(words, at, n):
    return [struct.unpack("<f", struct.pack("<I", w))[0] for w in words[at:at + n]]


def main():
    path = sys.argv[1]
    records = {}
    inline = {}
    shapes_with_lights = 0
    for _, tag, body in capture_dump.records(path):
        if tag == TAG_LIGHT and len(body) >= 4 + 26 * 4:
            key = struct.unpack_from("<I", body, 0)[0]
            records[key] = struct.unpack_from("<26I", body, 4)
        elif tag == capture_dump.TAG_SHAPE:
            g, m, flags, count = struct.unpack_from("<4I", body, 0)
            at = 16
            if count:
                shapes_with_lights += 1
            for _ in range(count):
                if at + STATE_LIGHT_SIZE > len(body):
                    break
                key = struct.unpack_from("<I", body, at)[0]
                inline[key] = struct.unpack_from("<26I", body, at + 4)
                at += STATE_LIGHT_SIZE
    both = sorted(set(records) & set(inline))
    print("%s: %d LIGHT records, %d light keys inline in %d shapes, %d keys both ways" % (os.path.basename(path), len(records), len(inline), shapes_with_lights, len(both)))
    worst = {}
    for key in both:
        a, b = records[key], inline[key]
        for name, at, n in PARTS:
            if name == "type":
                d = abs(a[0] - b[0])
            else:
                fa, fb = floats(a, at, n), floats(b, at, n)
                d = max(abs(x - y) for x, y in zip(fa, fb))
            if d > worst.get(name, (0, None))[0]:
                worst[name] = (d, key)
    for name, _, _ in PARTS:
        d, key = worst.get(name, (0, None))
        print("  %-12s largest difference %.6g%s" % (name, d, "" if key is None else " (key %d)" % key))
    if both:
        key = max(both, key=lambda k: max(abs(x - y) for x, y in zip(floats(records[k], 1, 4), floats(inline[k], 1, 4))))
        print("  the light with the largest diffuse difference, key %d:" % key)
        print("    record diffuse %s position %s attenuation %s range %s" % (floats(records[key], 1, 3), floats(records[key], 13, 3), floats(records[key], 21, 3), floats(records[key], 19, 1)))
        print("    inline diffuse %s position %s attenuation %s range %s" % (floats(inline[key], 1, 3), floats(inline[key], 13, 3), floats(inline[key], 21, 3), floats(inline[key], 19, 1)))


main()
