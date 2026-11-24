"""The formats of the replay gate's captures against the formats of the trees (W241; D126: a capture is made again,
never read back).

    python -I tools/capture_formats.py --baseline DIR --mge TREE   # from the plugin's tree
    python -I tools/capture_formats.py --self-test                  # planted captures, no game, no build

The helper refuses a capture of another stream format (exit 10) or of another extension format (exit 11, "make the
capture again with this build"). The replay gate replays the captures of the baseline folder and of its tour, so a
round that changes STREAM_FORMAT (`src/stream_format.rs`) or EXTENSION_FORMAT (`mgeHost64/src/renderer.rs` of the
render-link tree) with the old captures fails there, after the build of the set. This tool reads the two formats of
each capture with no replay and no build, so merge_step.py runs it before the set (the stage `capture formats`):

- the stream format: the word at HEADER_FORMAT_AT of the HEADER record;
- the extension format: the format word of the hello record (an EXTENSION record of the id HELLO_ID: the id, the
  version, then the format).

It reads the head of each record (8 bytes) and the first words of the HEADER and EXTENSION records only, skips every
other body, and stops at the hello record: it never reads a capture whole (verdict 751 5.1). Its line gives the bytes
read and the time. The numbers come from their one home in the trees (stream_format.rs, capture.rs, renderer.rs).

A capture of another format, or with no hello record (a capture from before the handshake), FAILs (exit 1), with one
line for each pair of formats: the count and the first three files. The captures of the instance A/B
(`instance/<name>/`) are not read: their rule is the A/B's own (W527: a FORMAT CHANGE passes with an evidence file).
NOT RUN (exit 5) when the baseline folder holds no capture.

A round that changes a format on purpose (verdict 751 5.2): this stage fails until the baseline's captures are of the
new format. The sequence: the worker makes the captures again with the build of the branch, the Coordinator puts them
into the baseline (D126), and then the round runs. Until the new captures are in the baseline, every round with the
gate fails here, the rounds of other branches too (their trees read the old format, the new captures do not).
"""
import argparse
import os
import re
import struct
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
sys.path.insert(0, HERE)
from common import NOT_RUN_EXIT  # noqa: E402

MAGIC = b"NIRCAP01"
STREAM_FORMAT_FILE = os.path.join("src", "stream_format.rs")
CAPTURE_FILE = os.path.join("src", "capture.rs")
RENDERER = os.path.join("mgeHost64", "src", "renderer.rs")
# The words of an extension record that the tool reads: the id, then the hello's version and its format.
EXTENSION_HEAD = 12


def constant(text, name, where):
    """The value of `const NAME: <type> = <number>;` in `text` (Rust)."""
    found = re.search(r"^\s*(?:pub(?:\(crate\))? )?const %s: \w+ = (\d+);" % re.escape(name), text, re.MULTILINE)
    if not found:
        raise SystemExit("capture formats: %s not found in %s" % (name, where))
    return int(found.group(1))


def read_text(path):
    with open(path, encoding="utf-8") as file:
        return file.read()


def layout(plugin, mge):
    """The numbers of the trees: {stream_format, extension_format, header_format_at, tag_header, tag_extension,
    hello_id}."""
    stream = read_text(os.path.join(plugin, STREAM_FORMAT_FILE))
    capture = read_text(os.path.join(plugin, CAPTURE_FILE))
    renderer = read_text(os.path.join(mge, RENDERER))
    return {
        "stream_format": constant(stream, "STREAM_FORMAT", STREAM_FORMAT_FILE),
        "header_format_at": constant(stream, "HEADER_FORMAT_AT", STREAM_FORMAT_FILE),
        "tag_header": constant(capture, "HEADER", CAPTURE_FILE),
        "tag_extension": constant(capture, "EXTENSION", CAPTURE_FILE),
        "extension_format": constant(renderer, "EXTENSION_FORMAT", RENDERER),
        "hello_id": constant(renderer, "HELLO_ID", RENDERER),
    }


def capture_formats(path, numbers):
    """(stream format, extension format, bytes read) of a capture; None for a format that the capture does not give.
    It reads each record's head, the first words of the HEADER and EXTENSION records, and stops at the hello record."""
    stream = extension = None
    read = 0
    with open(path, "rb") as file:
        magic = file.read(len(MAGIC))
        read += len(magic)
        if magic != MAGIC:
            return None, None, read
        header_words = numbers["header_format_at"] + 4
        while True:
            head = file.read(8)
            read += len(head)
            if len(head) < 8:
                break
            tag, length = struct.unpack("<II", head)
            start = file.tell()
            if tag == numbers["tag_header"] and stream is None:
                body = file.read(min(length, header_words))
                read += len(body)
                if len(body) == header_words:
                    stream = struct.unpack_from("<I", body, numbers["header_format_at"])[0]
            elif tag == numbers["tag_extension"]:
                body = file.read(min(length, EXTENSION_HEAD))
                read += len(body)
                if len(body) >= 4 and struct.unpack_from("<I", body, 0)[0] == numbers["hello_id"]:
                    if len(body) == EXTENSION_HEAD:
                        extension = struct.unpack_from("<I", body, 8)[0]
                    break
            file.seek(start + length)
    return stream, extension, read


def gate_captures(baseline):
    """The captures that the replay gate replays: those of the baseline folder, then those of its tour (not the
    instance A/B's). The same list as `order_captures` of validate_retained.py, which the gate's stages read; a change
    of one is a change of the other (verdict 752 4.1)."""
    found = []
    for folder in (baseline, os.path.join(baseline, "tour")):
        if os.path.isdir(folder):
            found += [os.path.join(folder, name) for name in sorted(os.listdir(folder)) if name.endswith(".nircap")]
    return found


def judge(rows, numbers):
    """(exit code, lines) for rows of (path, stream format, extension format)."""
    if not rows:
        return NOT_RUN_EXIT, ["NOT RUN capture formats: the baseline folder holds no capture"]
    want = (numbers["stream_format"], numbers["extension_format"])
    groups = {}
    for path, stream, extension in rows:
        if (stream, extension) != want:
            groups.setdefault((stream, extension), []).append(os.path.basename(path))
    if not groups:
        return 0, ["PASS capture formats: %d captures of stream format %d and extension format %d, as the trees" % (len(rows), *want)]
    lines = []
    for (stream, extension), names in sorted(groups.items(), key=lambda item: str(item[0])):
        first = ", ".join(names[:3]) + (", ..." if len(names) > 3 else "")
        if extension is None:
            what = "with no hello record (a capture from before the handshake)"
        else:
            what = "of stream format %s and extension format %d" % ("none" if stream is None else stream, extension)
        lines.append("FAIL capture formats: %d captures %s, the trees read stream format %d and extension format %d (%s)"
                     % (len(names), what, want[0], want[1], first))
    lines.append("FAIL capture formats: make the captures again with this round's build, and the Coordinator puts them into "
                 "the baseline (D126); until then every round with the gate fails here, the rounds of other branches too")
    return 1, lines


def run(baseline, plugin, mge, out=print):
    numbers = layout(plugin, mge)
    began = time.time()
    rows, total = [], 0
    for path in gate_captures(baseline):
        stream, extension, read = capture_formats(path, numbers)
        rows.append((path, stream, extension))
        total += read
    code, lines = judge(rows, numbers)
    for line in lines:
        out(line)
    out("capture formats: %d captures, %d bytes read in %.2f s" % (len(rows), total, time.time() - began))
    return code


# --- the tests -----------------------------------------------------------------------------

NUMBERS = {"stream_format": 10, "extension_format": 4, "header_format_at": 16, "tag_header": 11, "tag_extension": 16,
           "hello_id": 11}


def record(tag, body):
    return struct.pack("<II", tag, len(body)) + body


def plant(stream=10, extension=4, hello=True, before_hello=b"", after=b""):
    """The bytes of a planted capture: the HEADER record, a record before the hello, the hello, the rest."""
    header = struct.pack("<5I", 64, 64, 0, 1, stream) + bytes(64)
    data = MAGIC + record(11, header) + record(1, before_hello) + record(16, struct.pack("<I", 2) + bytes(8))
    if hello:
        data += record(16, struct.pack("<III", 11, 1, extension) + bytes(64))
    return data + after


def self_test():
    with tempfile.TemporaryDirectory() as folder:
        def write(name, data):
            path = os.path.join(folder, *name.split("/"))
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "wb") as file:
                file.write(data)
            return path

        big = bytes(1 << 20)
        # The formats, and a body of 1 MB before the hello that is skipped, not read.
        path = write("a.nircap", plant(before_hello=big, after=record(9, big)))
        stream, extension, read = capture_formats(path, NUMBERS)
        assert (stream, extension) == (10, 4) and read < 300, (stream, extension, read)
        # A tail that is cut after the hello is not read: the capture passes (verdict 751 5.1).
        cut = plant(after=struct.pack("<II", 9, 1 << 30) + b"x")
        assert capture_formats(write("cut.nircap", cut), NUMBERS)[:2] == (10, 4)
        # Each kind of fault, and the null.
        for planted, expect in ((plant(), None), (plant(stream=9), "of stream format 9 and extension format 4"),
                                (plant(extension=3), "of stream format 10 and extension format 3"),
                                (plant(hello=False), "with no hello record")):
            path = write("one/x.nircap", planted)
            rows = [(path,) + capture_formats(path, NUMBERS)[:2]]
            code, lines = judge(rows, NUMBERS)
            if expect is None:
                assert code == 0 and lines[0].startswith("PASS capture formats: 1 captures"), lines
            else:
                assert code == 1 and expect in lines[0] and "x.nircap" in lines[0] and "D126" in lines[-1], lines
        # Not a capture: no formats.
        assert capture_formats(write("not.nircap", b"NOTACAP0" + bytes(8)), NUMBERS)[:2] == (None, None)
        # The list: the folder and its tour, not the instance captures; an empty folder is NOT RUN.
        base = os.path.join(folder, "base")
        for name in ("a.nircap", "tour/t.nircap", "instance/grass/g.nircap"):
            write("base/" + name, plant())
        assert [os.path.basename(p) for p in gate_captures(base)] == ["a.nircap", "t.nircap"]
        assert judge([], NUMBERS)[0] == NOT_RUN_EXIT
        # Four captures of another format: the count, the first three, and "...".
        rows = [(os.path.join(base, "c%d.nircap" % i), 9, 4) for i in range(4)]
        code, lines = judge(rows, NUMBERS)
        assert code == 1 and lines[0].startswith("FAIL capture formats: 4 captures") and "c0.nircap, c1.nircap, c2.nircap, ..." in lines[0], lines
    # The numbers of this tree come from their homes.
    numbers = layout(REPO, os.path.join(os.path.dirname(REPO), "MGE-XE-G7-render-link"))
    assert numbers["header_format_at"] == 16 and numbers["tag_header"] == 11 and numbers["tag_extension"] == 16 and numbers["hello_id"] == 11, numbers
    print("capture_formats self-test: ok")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--baseline", help="the baseline folder of the replay gate")
    parser.add_argument("--mge", help="the render-link tree (EXTENSION_FORMAT, HELLO_ID)")
    parser.add_argument("--plugin", default=REPO, help="the plugin tree (STREAM_FORMAT, the record tags); this tree by default")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.baseline or not args.mge:
        parser.error("give --baseline and --mge, or --self-test")
    return run(args.baseline, args.plugin, args.mge)


if __name__ == "__main__":
    sys.exit(main())
