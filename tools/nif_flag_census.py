"""How many NIFs of the game and of a mod folder carry a flat-shading, specular or dither
property, and with which flag: a census for the compatibility of honouring them.

    python tools/nif_flag_census.py <bsa or folder> ...

Scans each NIF (loose, or inside a Morrowind BSA) for the block names and reads the flags
word that follows a property block's name, extra-data and controller references (the
NetImmerse 4.0.0.2 layout, `nif40.py`). Prints, per property kind, the number of files
with the property, the count of each flags value, and a few file names of the rare value.
"""
import os
import re
import struct
import sys
from collections import Counter, defaultdict

KINDS = ["NiShadeProperty", "NiSpecularProperty", "NiDitherProperty"]
PATTERN = re.compile(rb"(\x0f\x00\x00\x00NiShadeProperty|\x12\x00\x00\x00NiSpecularProperty|\x10\x00\x00\x00NiDitherProperty)")
# The flag value that means "as the engine's global": smooth on, specular off, dither off.
USUAL = {"NiShadeProperty": 1, "NiSpecularProperty": 0, "NiDitherProperty": 0}


def flags_after(data, at):
    """The flags of a property block whose type name ends at `at`: name length and name,
    two references, then the flags."""
    try:
        n = struct.unpack_from("<I", data, at)[0]
        if n > 256:
            return None
        return struct.unpack_from("<H", data, at + 4 + n + 8)[0]
    except struct.error:
        return None


def scan(name, data, files, values, examples):
    seen = set()
    for m in PATTERN.finditer(data):
        kind = m.group(1)[4:].decode()
        flags = flags_after(data, m.end())
        if flags is None:
            continue
        values[kind][flags] += 1
        seen.add(kind)
        if flags != USUAL[kind] and len(examples[kind]) < 5:
            examples[kind].append(name)
    for kind in seen:
        files[kind] += 1


def bsa_files(path):
    """The files of a Morrowind BSA (version 0x100)."""
    data = open(path, "rb").read()
    version, hash_offset, count = struct.unpack_from("<III", data, 0)
    if version != 0x100:
        return
    at = 12
    sizes = [struct.unpack_from("<II", data, at + 8 * i) for i in range(count)]
    at += 8 * count
    name_offsets = struct.unpack_from("<%dI" % count, data, at)
    at += 4 * count
    names_start = at
    data_start = 12 + hash_offset + 8 * count
    for i in range(count):
        end = data.index(b"\0", names_start + name_offsets[i])
        name = data[names_start + name_offsets[i]:end].decode("latin-1")
        size, offset = sizes[i]
        yield name, data[data_start + offset:data_start + offset + size]


def main():
    files = Counter()
    values = {k: Counter() for k in KINDS}
    examples = defaultdict(list)
    total = 0
    for root in sys.argv[1:]:
        if root.lower().endswith(".bsa"):
            for name, content in bsa_files(root):
                if name.lower().endswith(".nif"):
                    total += 1
                    scan(os.path.basename(root) + ":" + name, content, files, values, examples)
        else:
            for folder, _, names in os.walk(root):
                for name in names:
                    if name.lower().endswith(".nif"):
                        total += 1
                        try:
                            scan(os.path.join(folder, name), open(os.path.join(folder, name), "rb").read(), files, values, examples)
                        except OSError:
                            pass
    print("%d NIF files in %s" % (total, ", ".join(os.path.basename(r) or r for r in sys.argv[1:])))
    for kind in KINDS:
        print("  %s: in %d files; flags %s; unusual in: %s" % (kind, files[kind], dict(values[kind].most_common(6)), examples[kind]))


main()
