"""Which NIF block types occur in the game's BSAs, in a mod folder or in a test level: a
census for the coverage of the renderer's tests against the engine's loadable types.

    python tools/nif_block_census.py <bsa or folder> ...

A block begins with its type name as a length-prefixed string; the scan counts, for each
type, the files that have it (the prefix must equal the name's length, which keeps node
names such as "NiNode" inside a name string out). Prints one line per type, most files
first, and the types of `C:/tmp/engine-block-types.txt` (the engine's CreateFromStream
list) that occur in none of the files.
"""
import os
import re
import struct
import sys
from collections import Counter

PATTERN = re.compile(rb"([\x04-\x28])\x00\x00\x00((?:Ni|BS|Avoid|Root)[A-Za-z0-9]{2,38})")


def scan(data, files):
    seen = set()
    for m in PATTERN.finditer(data):
        if ord(m.group(1)) == len(m.group(2)):
            seen.add(m.group(2).decode())
    for kind in seen:
        files[kind] += 1


def bsa_files(path):
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
    total = 0
    for root in sys.argv[1:]:
        if root.lower().endswith(".bsa"):
            for name, content in bsa_files(root):
                if name.lower().endswith(".nif"):
                    total += 1
                    scan(content, files)
        else:
            for folder, _, names in os.walk(root):
                for name in names:
                    if name.lower().endswith(".nif"):
                        total += 1
                        try:
                            scan(open(os.path.join(folder, name), "rb").read(), files)
                        except OSError:
                            pass
    print("%d NIF files in %s" % (total, ", ".join(os.path.basename(r) or r for r in sys.argv[1:])))
    for kind, n in files.most_common():
        print("  %6d  %s" % (n, kind))
    engine_list = "C:/tmp/engine-block-types.txt"
    if os.path.isfile(engine_list):
        engine = [line.strip() for line in open(engine_list) if line.strip()]
        missing = [t for t in engine if t not in files]
        print("engine types in no file (%d of %d): %s" % (len(missing), len(engine), ", ".join(missing)))
        extra = [t for t in files if t not in engine]
        print("types in files the engine cannot load (%d): %s" % (len(extra), ", ".join(extra)))


main()
