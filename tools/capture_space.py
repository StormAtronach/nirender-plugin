"""Which space the records of a capture are in: how many SCENE_BEGIN records carry an origin,
how many INSTANCE records are relative (flag 1), how many SHAPE records are relative.

    python -I tools/capture_space.py <capture.nircap> [...]
"""
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import capture_dump  # noqa: E402

TAG_SCENE_BEGIN = 1


def main():
    for path in sys.argv[1:]:
        scenes = relative_scenes = instances = relative_instances = shapes = relative_shapes = 0
        for _, tag, body in capture_dump.records(path):
            if tag == TAG_SCENE_BEGIN:
                scenes += 1
                # The scene record: kind at 0, flags at 20 (bit 1: an origin follows at 24).
                if len(body) >= 48 and struct.unpack_from("<I", body, 20)[0] & 1:
                    relative_scenes += 1
            elif tag == capture_dump.TAG_INSTANCE:
                instances += 1
                relative_instances += struct.unpack_from("<I", body, 8)[0] & 1
            elif tag == capture_dump.TAG_SHAPE:
                shapes += 1
                relative_shapes += struct.unpack_from("<I", body, 8)[0] & 1
        print("%s: scenes %d (%d with an origin), instances %d (%d relative), shapes %d (%d relative)" % (
            os.path.basename(os.path.dirname(path)), scenes, relative_scenes, instances, relative_instances, shapes, relative_shapes))


main()
