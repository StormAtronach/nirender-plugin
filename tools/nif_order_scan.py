"""Lists the NIFs of a folder whose picture can depend on the order of the draws, for the order-dependent exhibits of a test level.

    python -I tools/nif_order_scan.py <meshes folder> [--tour <folder of the tour pictures>] [--all]
    python -I tools/nif_order_scan.py --self-test    # the tests, on small NIFs made here

For each NIF (NetImmerse 4.0.0.2, the Morrowind format) the row gives:
- sortAdjust: how many NiSortAdjustNode blocks; modes: the sorting mode word of each, parsed from the node with the layout of
  nif40.py; sortActive: how many of them change the sorting. The engine (NiSortAdjustNode::Display 0x6DE030 [V IDA]) takes 1 as
  "off" and 2 as "subsort"; any other value, 0 and the 64 of some files of the test level too, takes the inherit path, so such a
  node sorts as its parent does;
- blended: NiAlphaProperty blocks with the blend bit (flag bit 0);
- noSorter: of those, the ones with the "no sorter" bit (flag bit 0x2000);
- zTestOff, zWriteOff: NiZBufferProperty blocks with the depth test (flag bit 0) or the depth write (flag bit 1) off;
- particles: particle systems (NiAutoNormalParticles, NiRotatingParticles and the base type NiParticles);
- read: "blocks" when nif40.py read every block of the file; "pattern: <why>" when it did not (a block of a type that nif40.py has no
  layout for, so the blocks are found by their type names in the bytes: the counts are those of the pattern, and a block that the
  pattern takes for a type name inside other data would be a false hit); "unreadable" when the flags or the mode could not be parsed.

A NIF with none of these counts is left out unless --all. With --tour, the stop of the tour (`1-03` of the picture `1-03-_nif_002.png`)
is named for the NIF `002.nif`; the stop is "-" without a picture. The tool reads files only, and no place of a machine is in it.
"""
import argparse
import os
import re
import struct
import sys

# The modules of this folder, also under `python -I`.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import nif40  # noqa: E402

SORT_NODE = "NiSortAdjustNode"
ALPHA = "NiAlphaProperty"
ZBUFFER = "NiZBufferProperty"
PARTICLES = ("NiAutoNormalParticles", "NiRotatingParticles", "NiParticles")
ALPHA_BLEND = 0x0001
ALPHA_NO_SORTER = 0x2000
ZBUFFER_TEST = 0x0001
ZBUFFER_WRITE = 0x0002
# The sorting modes that change the sorting: NiSortAdjustNode::Display reads 1 as off and 2 as subsort. Any other word inherits.
SORT_OFF = 1
SORT_SUBSORT = 2
ACTIVE_MODES = (SORT_OFF, SORT_SUBSORT)
# The tour pictures: `<stop>-_nif_<number>.png`, the stop as 1-03.
TOUR_PICTURE = re.compile(r"(\d-\d\d)-_nif_(\d+)\.png$")
# A block type in the bytes: a length word and a type name of that length.
TYPE_NAME = re.compile(rb"(?s)(.{4})(Ni[A-Za-z]{2,40})")


def tour_stops(folder):
    """{the number of the NIF as an int: the stop} from the names of the tour pictures."""
    stops = {}
    for name in os.listdir(folder):
        found = TOUR_PICTURE.match(name)
        if found:
            stops[int(found.group(2))] = found.group(1)
    return stops


def counts_of(rows):
    """The counts of one file from (type, fields) pairs: fields is the dict of the block, or None when the block could not be parsed."""
    counts = {"sort_modes": [], "blended": 0, "no_sorter": 0, "z_test_off": 0, "z_write_off": 0, "particles": 0, "unparsed": 0}
    for kind, fields in rows:
        if kind == SORT_NODE:
            mode = fields.get("sorting_mode") if fields else None
            counts["sort_modes"].append(mode)
            counts["unparsed"] += mode is None
        elif kind == ALPHA:
            flags = fields.get("flags") if fields else None
            if flags is None:
                counts["unparsed"] += 1
            elif flags & ALPHA_BLEND:
                counts["blended"] += 1
                counts["no_sorter"] += bool(flags & ALPHA_NO_SORTER)
        elif kind == ZBUFFER:
            flags = fields.get("flags") if fields else None
            if flags is None:
                counts["unparsed"] += 1
            else:
                counts["z_test_off"] += not flags & ZBUFFER_TEST
                counts["z_write_off"] += not flags & ZBUFFER_WRITE
        elif kind in PARTICLES:
            counts["particles"] += 1
    return counts


def pattern_rows(data):
    """The interesting blocks found by their type names in the bytes, each with its fields parsed from the bytes after the name
    (None when they do not parse). For a file that nif40.py cannot read whole."""
    rows = []
    for found in TYPE_NAME.finditer(data):
        kind = found.group(2).decode()
        if int.from_bytes(found.group(1), "little") != len(found.group(2)) or kind not in (SORT_NODE, ALPHA, ZBUFFER, *PARTICLES):
            continue
        fields = None
        if kind in (SORT_NODE, ALPHA, ZBUFFER):
            reader = nif40.Reader(data)
            reader.at = found.end()
            last = "sorting_mode" if kind == SORT_NODE else "flags"
            try:
                fields = {}
                for name, layout_kind in nif40.LAYOUTS[kind]:
                    fields[name] = reader.field(layout_kind)
                    if name == last:
                        break
            except (struct.error, ValueError, IndexError, UnicodeDecodeError, MemoryError):
                fields = None
        rows.append((kind, fields))
    return rows


def scan(data):
    """(counts, how the file was read) of the bytes of one NIF."""
    try:
        blocks, _ = nif40.read(data)
        return counts_of((block["type"], block) for block in blocks), "blocks"
    except nif40.Unknown as unknown:
        reason = "pattern: no layout for %s" % str(unknown).split(": ")[-1]
    except Exception as error:  # noqa: BLE001 - a NIF that nif40.py cannot read is scanned by its type names
        reason = "pattern: nif40 cannot read it (%s)" % str(error)[:60]
    counts = counts_of(pattern_rows(data))
    return counts, "unreadable" if counts["unparsed"] else reason


def interesting(counts):
    return bool(counts["sort_modes"] or counts["no_sorter"] or counts["z_test_off"] or counts["z_write_off"])


def mode_text(modes):
    return ",".join("?" if mode is None else "%d" % mode for mode in modes) or "-"


def active_sorters(modes):
    """How many of the sorting modes change the sorting (the engine reads 1 and 2 only)."""
    return sum(mode in ACTIVE_MODES for mode in modes)


def rows_of(meshes, stops=None, show_all=False):
    """[(stop, number, counts, read)] for the NIFs of the folder, in name order."""
    stops = stops or {}
    rows = []
    for name in sorted(os.listdir(meshes)):
        if not name.lower().endswith(".nif"):
            continue
        with open(os.path.join(meshes, name), "rb") as file:
            counts, read = scan(file.read())
        if show_all or interesting(counts):
            number = name[:-4]
            stop = stops.get(int(number), "-") if number.isdigit() else "-"
            rows.append((stop, number, counts, read))
    return rows


def table(rows):
    lines = ["stop  nif  sortAdjust  sortActive  modes  blended  noSorter  zTestOff  zWriteOff  particles  read"]
    for stop, number, c, read in rows:
        lines.append("%-5s %-4s %10d %10d %6s %8d %9d %9d %10d %10d  %s" % (
            stop, number, len(c["sort_modes"]), active_sorters(c["sort_modes"]), mode_text(c["sort_modes"]), c["blended"], c["no_sorter"],
            c["z_test_off"], c["z_write_off"], c["particles"], read))
    lines.append("NIFs listed: %d. modes: 1 off, 2 subsort, any other word inherits (NiSortAdjustNode::Display 0x6DE030); ? = not parsed" % len(rows))
    return "\n".join(lines)


# --- the tests ---------------------------------------------------------------------------

def self_test():
    import tempfile

    import make_test_nifs as made

    def blocks_of():
        """A root with a shape: blocks 0 root, 1 shape, 2 data, 3 material, 4 texturing, 5 texture."""
        blocks = [made.node("root"), None, made.tri_shape_data(made.box(10.0)), made.material(), made.texturing(5), made.source_texture(made.BASE_TEXTURE)]
        shape = made.av_object("shape", 0)
        shape.update({"type": "NiTriShape", "data": 2, "skin": -1, "properties": [3, 4]})
        blocks[1] = shape
        blocks[0]["children"] = [1]
        return blocks

    def sorted_file(modes, alpha_flags=(), z_flags=(), particles=0):
        """A file with a NiSortAdjustNode of each mode (the shape under the first) and the given properties on the shape."""
        blocks = blocks_of()
        for flags in alpha_flags:
            blocks[1]["properties"].append(len(blocks))
            blocks.append(made.one_flag(ALPHA, flags) | {"threshold": 0})
        for flags in z_flags:
            blocks[1]["properties"].append(len(blocks))
            blocks.append(made.one_flag(ZBUFFER, flags))
        for _ in range(particles):
            blocks[0]["children"].append(len(blocks))
            shape = made.av_object("sparks", 0)
            shape.update({"type": "NiAutoNormalParticles", "data": -1, "skin": -1})
            blocks.append(shape)
        for mode in modes:
            node = made.node("sort")
            node.update({"type": SORT_NODE, "sorting_mode": mode, "sub_sorter": -1})
            blocks[0]["children"].append(len(blocks))
            blocks.append(node)
        return nif40.write(blocks, [0])

    # The exact path: every count and every mode.
    data = sorted_file([2, 1], alpha_flags=(0x0001 | 0x2000, 0x0001, 0x0000), z_flags=(0, 1, 3), particles=2)
    counts, read = scan(data)
    assert read == "blocks", read
    assert counts["sort_modes"] == [2, 1] and counts["blended"] == 2 and counts["no_sorter"] == 1, counts
    assert counts["z_test_off"] == 1 and counts["z_write_off"] == 2 and counts["particles"] == 2 and counts["unparsed"] == 0, counts
    assert mode_text(counts["sort_modes"]) == "2,1" and mode_text([]) == "-" and mode_text([None, 0]) == "?,0"
    # Only 1 and 2 change the sorting: 0 and a word such as 64 (as in some files of the test level) inherit.
    assert active_sorters([2, 1]) == 2 and active_sorters([0, 64, None]) == 0 and active_sorters([64, 2]) == 1
    odd, read = scan(sorted_file([64, 0, 1]))
    assert read == "blocks" and odd["sort_modes"] == [64, 0, 1] and active_sorters(odd["sort_modes"]) == 1, odd
    unknown = bytearray(sorted_file([64, 2]))
    at = unknown.index(b"NiMaterialProperty")
    unknown[at:at + len(b"NiMaterialProperty")] = b"NiMaterialPropertz"
    odd, read = scan(bytes(unknown))
    assert read.startswith("pattern") and odd["sort_modes"] == [64, 2], (odd, read)
    # A file with none of it is not interesting; its counts are zero.
    plain, _ = scan(sorted_file([]))
    assert not interesting(plain) and plain["sort_modes"] == [] and plain["blended"] == 0, plain
    # A blend with the no-sorter bit alone, and a depth write off alone, make a file interesting.
    assert interesting(scan(sorted_file([], alpha_flags=(0x2001,)))[0]) and interesting(scan(sorted_file([], z_flags=(1,)))[0])
    assert not interesting(scan(sorted_file([], alpha_flags=(0x0001,), z_flags=(3,)))[0]), "a plain blend and a full depth test are no exhibit"

    # The pattern path: a block type that nif40.py has no layout for (here a type name of equal length, so that the file stays a file).
    other = bytearray(sorted_file([1, 2], alpha_flags=(0x2001,), z_flags=(0,), particles=1))
    at = other.index(b"NiMaterialProperty")
    other[at:at + len(b"NiMaterialProperty")] = b"NiMaterialPropertz"
    counts, read = scan(bytes(other))
    assert read == "pattern: no layout for NiMaterialPropertz", read
    assert counts["sort_modes"] == [1, 2] and counts["blended"] == 1 and counts["no_sorter"] == 1, counts
    assert counts["z_test_off"] == 1 and counts["z_write_off"] == 1 and counts["particles"] == 1, counts
    # The base type NiParticles counts as a particle system: the type name of a made file is replaced by it, with its length word.
    renamed = sorted_file([], particles=1).replace(struct.pack("<I", 21) + b"NiAutoNormalParticles", struct.pack("<I", 11) + b"NiParticles")
    counts, read = scan(renamed)
    assert counts["particles"] == 1 and read.startswith("pattern"), (counts, read)
    # The pattern path says so when the mode cannot be parsed: a sort node cut off after its name.
    cut = sorted_file([2])
    cut = cut[:cut.index(SORT_NODE.encode()) + len(SORT_NODE) + 6]
    counts, read = scan(cut)
    assert counts["sort_modes"] == [None] and read == "unreadable", (counts, read)

    # The folder: the stop from the picture names, the file order, --all, a name that is no number.
    with tempfile.TemporaryDirectory(prefix="nif-order-scan-test-") as root:
        meshes, tour = os.path.join(root, "meshes"), os.path.join(root, "tour")
        os.makedirs(meshes)
        os.makedirs(tour)
        for name, file_data in (("002.nif", sorted_file([2])), ("010.nif", sorted_file([], z_flags=(1,))), ("003.nif", sorted_file([])), ("misc.nif", sorted_file([0]))):
            with open(os.path.join(meshes, name), "wb") as file:
                file.write(file_data)
        for name in ("1-03-_nif_002.png", "1-04-_nif_010.png", "notes.txt"):
            open(os.path.join(tour, name), "w").close()
        assert tour_stops(tour) == {2: "1-03", 10: "1-04"}, tour_stops(tour)
        rows = rows_of(meshes, tour_stops(tour))
        assert [(stop, number) for stop, number, _, _ in rows] == [("1-03", "002"), ("1-04", "010"), ("-", "misc")], rows
        assert [number for _, number, _, _ in rows_of(meshes, {}, show_all=True)] == ["002", "003", "010", "misc"]
        text = table(rows)
        assert text.splitlines()[0].startswith("stop") and "NIFs listed: 3" in text and "1-03" in text, text
    print("nif_order_scan self-test: all passed")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("meshes", nargs="?", help="the folder of the NIFs")
    parser.add_argument("--tour", help="the folder of the tour pictures (<stop>-_nif_<number>.png), to name the stop of each NIF")
    parser.add_argument("--all", action="store_true", help="list the NIFs with none of the counts too")
    parser.add_argument("--self-test", action="store_true", help="the tests, on small NIFs made here")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.meshes:
        parser.error("it needs the folder of the NIFs")
    print(table(rows_of(args.meshes, tour_stops(args.tour) if args.tour else None, args.all)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
