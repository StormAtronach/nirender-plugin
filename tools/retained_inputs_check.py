"""The inputs of the retained table against the record path, from two captures of one view:
one in mode gpu (the table), one in mode off (the engine's draws).

    python -I tools/retained_inputs_check.py <gpu.nircap> <off.nircap>

Prints: the ambient and the fog of the RETAINED_SCENE records of the gpu capture against
the ambient and fog words of the SHAPE records of the off capture; and, for each geometry
handle that is an instance in the gpu capture and a shape in the off capture, the light
keys the table has (INSTANCE_LIGHTS) against the keys of the shape's draw.
"""
import os
import struct
import sys
from collections import Counter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import capture_dump  # noqa: E402

TAG_RETAINED_SCENE = 28
TAG_INSTANCE_LIGHTS = 26
TAG_INSTANCE = 23
SHAPE_HEAD_WORDS = 36


def shape_keys(body):
    count = struct.unpack_from("<I", body, 12)[0]
    at = SHAPE_HEAD_WORDS * 4
    return struct.unpack_from("<%dI" % count, body, at) if count else ()


def main():
    gpu, off = sys.argv[1], sys.argv[2]
    scenes = []
    instance_geometry = {}
    instance_keys = {}
    gpu_shape_ambient = Counter()
    for frame, tag, body in capture_dump.records(gpu):
        if tag == TAG_RETAINED_SCENE:
            scenes.append((frame, struct.unpack_from("<5I", body, 0)))
        elif tag == TAG_INSTANCE:
            i, cls, flags, g = struct.unpack_from("<4I", body, 0)
            instance_geometry[i] = g
        elif tag == TAG_INSTANCE_LIGHTS:
            i, n = struct.unpack_from("<2I", body, 0)
            instance_keys[i] = tuple(sorted(struct.unpack_from("<%dI" % n, body, 8))) if n else ()
        elif tag == capture_dump.TAG_SHAPE:
            flags = struct.unpack_from("<I", body, 8)[0]
            if flags & 1:
                gpu_shape_ambient[struct.unpack_from("<I", body, 16)[0]] += 1
    off_ambient = Counter()
    off_fog = Counter()
    off_keys = {}
    for frame, tag, body in capture_dump.records(off):
        if tag == capture_dump.TAG_SHAPE:
            g, m, flags, count = struct.unpack_from("<4I", body, 0)
            if flags & 1:
                off_ambient[struct.unpack_from("<I", body, 16)[0]] += 1
                off_keys.setdefault(g, set()).add(tuple(sorted(shape_keys(body))))
                if flags & 32:
                    _, _, _, fog = capture_dump.shape_tail(body)
                    if fog is not None:
                        off_fog[struct.unpack_from("<4I", body, fog)] += 1
    print("gpu RETAINED_SCENE records: %d; distinct (ambient, fog0, fog1, fog2, fog3): %s" % (len(scenes), sorted({s for _, s in scenes})[:6]))
    print("gpu SHAPE (relative) ambient words: %s" % [("%#010x" % a, n) for a, n in gpu_shape_ambient.most_common(4)])
    print("off SHAPE (relative) ambient words: %s" % [("%#010x" % a, n) for a, n in off_ambient.most_common(4)])
    print("off SHAPE fog words: %s" % [(tuple("%#x" % w for w in f), n) for f, n in off_fog.most_common(3)])
    same = different = missing = 0
    shown = 0
    for i, g in instance_geometry.items():
        keys = instance_keys.get(i)
        if keys is None or g not in off_keys:
            missing += 1
            continue
        if keys in off_keys[g]:
            same += 1
        else:
            different += 1
            if shown < 6:
                shown += 1
                print("  instance %d geometry %#x: table keys %s, draw keys %s" % (i, g, keys, sorted(off_keys[g])[0]))
    print("instances with the draw's light keys: %d same, %d different, %d not comparable" % (same, different, missing))


main()
