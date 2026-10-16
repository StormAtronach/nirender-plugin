"""The error of the engine's 32-bit translations in a capture: for each INSTANCE record, the
distance between the stored translation (the engine's floats) and the exact one (64-bit from
the scene graph). The retained table draws from the exact one since 2026-10-16.

    python -I tools/translation_error.py <capture.nircap> [...]

Prints, for each capture: the records, the largest and the mean error in units of the world,
the share of records with an error over 0.001 units, and the farthest instance from the
origin of the world (the error of a float grows with the distance).
"""
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import capture_dump  # noqa: E402


def main():
    for path in sys.argv[1:]:
        errors = []
        farthest = 0.0
        for _, tag, body in capture_dump.records(path):
            if tag != capture_dump.TAG_INSTANCE:
                continue
            exact = struct.unpack_from("<3d", body, 68)
            stored = struct.unpack_from("<3f", body, 92)
            error = sum((e - s) ** 2 for e, s in zip(exact, stored)) ** 0.5
            errors.append(error)
            farthest = max(farthest, sum(e * e for e in exact) ** 0.5)
        if not errors:
            print("%s: no instance record" % os.path.basename(path))
            continue
        errors.sort()
        print("%s: %d records; error largest %.5f, mean %.5f, median %.5f units; %.1f %% over 0.001; farthest instance %.0f units from the origin" % (
            os.path.basename(os.path.dirname(path)) + "/" + os.path.basename(path), len(errors), errors[-1], sum(errors) / len(errors), errors[len(errors) // 2],
            100.0 * sum(e > 0.001 for e in errors) / len(errors), farthest))


main()
