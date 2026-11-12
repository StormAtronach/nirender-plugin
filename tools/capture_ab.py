"""Compares two capture files for a change of the stream's records: a capture of the base build
against a capture of the changed build, of the same place.

    python tools/capture_ab.py <base.nircap> <changed.nircap> [--mge-build DIR] [--no-replay]
    python tools/capture_ab.py --self-test

1. The fixed-length tags. A tag is fixed-length when each record that capture.rs writes with it
   has a whole length that the source fixes (`check_stream_tags.Source` with `strict`) and is not
   on the list RUN_TIME of check_stream_tags. For each such tag, the set of the lengths of its
   records is the same in both captures, and each length is one that capture.rs writes. These
   are the lengths that a change of the records can change.
2. The variable-length tags (a texture, a geometry, an instance and its name, the lists of the
   instances) have lengths that follow the content of the start: two starts at one place can
   draw other objects. The tool prints them with their counts and does not compare them.
3. The reader's check: tools/replay.py draws the last frame of each capture with the helper of
   --mge-build (replay.py's default without it). Each replay must exit with 0 and read all the
   frames of its file, and its report must give 0 records shorter than their tag needs and 0
   geometry records that did not parse, with no reads past the end and no tag that the build
   does not know.

The fixed-length tags come from the capture.rs of this tree: run the tool in the tree of the
change. A replay is build work: take the whole build side first (slot.py take build --whole).
The exit code is 1 when a check fails.
"""
import argparse
import collections
import os
import re
import struct
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import check_stream_tags  # noqa: E402
MAGIC = b"NIRCAP01"
FRAME_END = 10


def records(data):
    """[(tag, length)] of the records of a capture file's bytes. A record that does not fit in
    the file (the end of a cut file) is not read."""
    assert data[:len(MAGIC)] == MAGIC, "not a capture file: the magic is %r" % data[:len(MAGIC)]
    at, found = len(MAGIC), []
    while at + 8 <= len(data):
        tag, length = struct.unpack_from("<II", data, at)
        if at + 8 + length > len(data):
            break
        found.append((tag, length))
        at += 8 + length
    return found


def lengths_by_tag(found):
    """{tag: Counter of lengths}"""
    tags = collections.defaultdict(collections.Counter)
    for tag, length in found:
        tags[tag][length] += 1
    return tags


def fixed_lengths():
    """{tag number: (name, the set of the whole lengths that capture.rs writes)} of the
    fixed-length tags of this tree."""
    read = lambda path: open(path, encoding="utf-8").read()  # noqa: E731
    writer = read(check_stream_tags.WRITER)
    numbers = check_stream_tags.writer_tags(writer)
    source = check_stream_tags.Source(writer, read(check_stream_tags.LINK), strict=True)
    tags = {}
    for name, _, function, parts in source.records():
        whole = [length for _, length in parts]
        fixed = None not in whole and (name, function) not in check_stream_tags.RUN_TIME
        tags.setdefault(name, []).append(sum(whole) if fixed else None)
    return {numbers[name]: (name, set(lengths)) for name, lengths in tags.items() if None not in lengths}


def compare(fixed, base, changed, names=None):
    """The problems and the lines of the compare of two captures' {tag: Counter of lengths};
    `names` gives the names of the variable-length tags ({number: name})."""
    names = names or {}
    problems, lines = [], []
    for tag in sorted(set(base) | set(changed)):
        a, b = base.get(tag, collections.Counter()), changed.get(tag, collections.Counter())
        counts = "records %d / %d" % (sum(a.values()), sum(b.values()))
        if tag not in fixed:
            lines.append("tag %2d %s: variable length, %d lengths / %d, %s: not compared" % (tag, names.get(tag, "?"), len(a), len(b), counts))
            continue
        name, written = fixed[tag]
        lines.append("tag %2d %s: fixed length %s, %s" % (tag, name, sorted(written), counts))
        if set(a) != set(b):
            problems.append("%s (tag %d): the base has the lengths %s, the change %s" % (name, tag, sorted(a), sorted(b)))
        for side, seen in (("base", a), ("change", b)):
            other = sorted(set(seen) - written)
            if other:
                problems.append("%s (tag %d): the %s has the lengths %s, and capture.rs writes %s" % (name, tag, side, other, sorted(written)))
    absent = sorted(fixed[tag][0] for tag in set(fixed) - set(base) - set(changed))
    if absent:
        lines.append("fixed-length tags with no record in either capture, not checked: %s" % ", ".join(absent))
    return problems, lines


# The lines of the replay's report that the reader's check reads: (name, pattern, needed).
REPORT = (
    ("frames", re.compile(r"^(\d+) records, (\d+) frames", re.M), True),
    ("short", re.compile(r"(\d+) records of any tag shorter than their tag needs, refused"), True),
    ("unparsed", re.compile(r"(\d+) records that did not parse"), True),
    ("past the end", re.compile(r"reads past the end of a buffer: (\d+)"), False),
    ("unknown", re.compile(r"records of a tag this build does not know: (\d+)"), False),
)


def report_problems(output, code, frames):
    """The problems of a replay's output, its exit code and the frames of its file."""
    problems = []
    if code != 0:
        problems.append("the replay exited with %d" % code)
    for name, pattern, needed in REPORT:
        match = pattern.search(output)
        if not match:
            if needed:
                problems.append("the report has no line of %s" % name)
            continue
        value = int(match.group(match.lastindex))
        if name == "frames":
            if value != frames:
                problems.append("the replay read %d frames of %d" % (value, frames))
        elif value != 0:
            problems.append("%s: %d" % (name, value))
    return problems


def replay(capture, frames, mge_build):
    """The output and the exit code of a replay of the last frame of `capture`."""
    with tempfile.TemporaryDirectory() as folder:
        command = [sys.executable, "-B", os.path.join(HERE, "replay.py"), capture, os.path.join(folder, "frame"), "--frames-at", str(max(frames - 1, 0))]
        if mge_build:
            command += ["--mge-build", mge_build]
        done = subprocess.run(command, capture_output=True, text=True, errors="replace")
    return done.stdout + done.stderr, done.returncode


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("base", nargs="?")
    ap.add_argument("changed", nargs="?")
    ap.add_argument("--mge-build", help="the MGE XE build folder of the replays (replay.py's default without it)")
    ap.add_argument("--no-replay", action="store_true", help="only the compare of the fixed-length tags")
    ap.add_argument("--self-test", action="store_true", help="the tests of the compare and of the report's check, with the nulls")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.base or not args.changed:
        ap.error("give the base capture and the changed capture")
    fixed = fixed_lengths()
    writer = open(check_stream_tags.WRITER, encoding="utf-8").read()
    names = {number: name for name, number in check_stream_tags.writer_tags(writer).items()}
    found = {path: records(open(path, "rb").read()) for path in (args.base, args.changed)}
    for side, path in (("base", args.base), ("change", args.changed)):
        print("%s %s: %d records, %d frames" % (side, path, len(found[path]), sum(1 for tag, _ in found[path] if tag == FRAME_END)))
    problems, lines = compare(fixed, lengths_by_tag(found[args.base]), lengths_by_tag(found[args.changed]), names)
    for line in lines:
        print(line)
    if not args.no_replay:
        for side, path in (("base", args.base), ("change", args.changed)):
            frames = sum(1 for tag, _ in found[path] if tag == FRAME_END)
            output, code = replay(path, frames, args.mge_build)
            shown = [line for line in output.splitlines() if any(pattern.search(line) for _, pattern, _ in REPORT)]
            print("replay of the %s: exit code %d; %s" % (side, code, " | ".join(shown) or "no report line"))
            problems += ["the replay of the %s: %s" % (side, problem) for problem in report_problems(output, code, frames)]
    for problem in problems:
        print("FAIL: " + problem)
    if not problems:
        print("capture_ab ok: %d fixed-length tags compared%s" % (sum(1 for line in lines if " fixed length " in line), "" if args.no_replay else ", both replays read every record"))
    return 1 if problems else 0


def self_test():
    def capture(items):
        body = b"".join(struct.pack("<II", tag, len(data)) + data for tag, data in items)
        return MAGIC + body

    light, scene_end = 21, 4
    fixed = {light: ("LIGHT", {112}), scene_end: ("SCENE_END", {4})}
    frame = [(light, bytes(112)), (scene_end, bytes(4)), (1, bytes(40)), (FRAME_END, bytes(16))]
    base = lengths_by_tag(records(capture(frame * 3)))
    assert records(capture(frame) + b"\x01\x00\x00\x00\xff\x00\x00\x00ab")[-1] == (FRAME_END, 16), "a cut record at the end is not read"

    # The same fixed lengths and other variable lengths (another texture): a pass.
    content = lengths_by_tag(records(capture(frame * 2 + [(1, bytes(52)), (FRAME_END, bytes(16))])))
    problems, lines = compare({**fixed, 38: ("GEOMETRY_GONE", {8})}, base, content, {1: "TEXTURE"})
    assert problems == [] and any("tag  1 TEXTURE: variable length" in line for line in lines), (problems, lines)
    assert lines[-1] == "fixed-length tags with no record in either capture, not checked: GEOMETRY_GONE", lines
    # A fixed tag missing from one capture (no light drawn in the frames) differs too.
    assert compare(fixed, base, lengths_by_tag(records(capture([(scene_end, bytes(4))]))))[0]

    # The nulls: a fixed-length record 4 bytes longer, and a length that capture.rs does not write.
    longer = lengths_by_tag(records(capture([(light, bytes(116))] + frame[1:])))
    problems = compare(fixed, base, longer)[0]
    assert any("LIGHT (tag 21): the base has the lengths [112], the change [116]" in p for p in problems), problems
    assert any("the change has the lengths [116], and capture.rs writes [112]" in p for p in problems), problems
    both = lengths_by_tag(records(capture([(scene_end, bytes(8))])))
    problems = compare(fixed, both, both)[0]
    assert problems and all("capture.rs writes [4]" in p for p in problems), problems

    # The report of a replay: a clean one, and the nulls of each line.
    good = ("20752 records, 30 frames, frame 29 into 1920 x 1080\n"
            "geometry: 902 records (21.3 MB), 7350 draws from them, 0 records that did not parse, 0 draws of more index entries\n"
            "material: 176 records; 0 records of any tag shorter than their tag needs, refused\n")
    assert report_problems(good, 0, 30) == [], report_problems(good, 0, 30)
    nulls = {
        "the replay exited with 1": (good, 1, 30),
        "the replay read 30 frames of 31": (good, 0, 31),
        "short: 2": (good.replace("; 0 records of any", "; 2 records of any"), 0, 30),
        "unparsed: 1": (good.replace(", 0 records that did not parse", ", 1 records that did not parse"), 0, 30),
        "past the end: 3": (good + "reads past the end of a buffer: 3 (of a record)\n", 0, 30),
        "unknown: 4": (good + "records of a tag this build does not know: 4, skipped; the first tag 77\n", 0, 30),
        "the report has no line of short": (good.replace("records of any tag shorter", "records of any tag"), 0, 30),
    }
    for want, (output, code, frames) in nulls.items():
        assert want in report_problems(output, code, frames), (want, report_problems(output, code, frames))

    # This tree: the tags that the strict source fixes.
    tree = fixed_lengths()
    assert tree[21] == ("LIGHT", {112}) and tree[10] == ("FRAME_END", {16}), tree
    for variable in (1, 18, 20, 23, 42, 43, 45, 49):
        assert variable not in tree, (variable, tree.get(variable))
    print("capture_ab self-test: ok")


if __name__ == "__main__":
    sys.exit(main())
