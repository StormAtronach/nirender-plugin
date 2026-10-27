"""The version handshake of the four parts: each number has one home, and the copies agree.

    python -I tools/check_handshake.py            # this repository: the stream format
    python -I tools/check_handshake.py --mge DIR  # MGE XE: the extension format

This repository: `host/src/lib.rs` reads `src/stream_format.rs` as a `#[path]` module, so the
plugin and the host crate compare one STREAM_FORMAT. The file is also the one home of the
layout of the HEADER record and of the shared memories (the ring head words, the reply, tile
and message memories), of the suffixes of the names of a stream's events and memories, and of
the exit codes: no constant of it may have a second definition in the two crates, or with
--mge in mgeHost64, and no name suffix may come back as a literal. MGE XE:
MGE_EXTENSION_FORMAT in
`d3d8/cpp/mge/mgeversion.h` equals EXTENSION_FORMAT in `mgeHost64/src/renderer.rs` (a unit
test of mgeHost64 compares them too; this check needs no compile). `checks.py` runs both.
The design: handshake-design.md of the Numidium notes (2026-10-27).
"""
import argparse
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
MGE = os.path.join(os.path.dirname(REPO), "MGE-XE-G7-render-link")


def read(path):
    with open(path, encoding="utf-8") as file:
        return file.read()


def number(text, pattern, what):
    found = re.search(pattern, text, re.MULTILINE)
    if not found:
        raise SystemExit("%s not found" % what)
    return int(found.group(1))


HOME = os.path.join(REPO, "src", "stream_format.rs")


def home_names():
    """The constants of src/stream_format.rs: each has its one home there."""
    return set(re.findall(r"^pub const ([A-Z][A-Z0-9_]*)\s*:", read(HOME), re.MULTILINE))


def home_strings():
    """The string constants of src/stream_format.rs (the suffixes of the names of a stream's
    events and memories), as {value: name}."""
    return {value: name for name, value in re.findall(r'^pub const ([A-Z][A-Z0-9_]*)\s*:\s*&str\s*=\s*"([^"]+)"', read(HOME), re.MULTILINE)}


def literal_copies(text, strings):
    """The names of `strings` whose value `text` writes as a literal right after a `"` or
    after a `}` of a format string (`"{name}-read"`), and ends with a `"` or a `{`: a copy of
    the name that the home constant should give."""
    found = set()
    for value, name in strings.items():
        if re.search(r'(?<=["}])%s(?=["{])' % re.escape(value), text):
            found.add(name)
    return sorted(found)


def second_definitions(text, names):
    """The names of `names` that `text` defines again, as a const or a static."""
    if not names:
        return []
    pattern = re.compile(r"\b(?:const|static)\s+(%s)\s*:" % "|".join(sorted(names)))
    return sorted(set(pattern.findall(text)))


def check_folders(base, folders, names):
    """A problem for each second definition of a home constant in the .rs files of `folders`,
    and for each literal copy of a home string."""
    problems = []
    strings = home_strings()
    for folder in folders:
        for root, _, files in os.walk(os.path.join(base, folder)):
            if os.sep + "target" in root:
                continue
            for name in files:
                path = os.path.join(root, name)
                if not name.endswith(".rs") or os.path.samefile(path, HOME):
                    continue
                text = read(path)
                where = os.path.relpath(path, base).replace("\\", "/")
                for constant in second_definitions(text, names):
                    problems.append("%s defines %s of its own: its one home is src/stream_format.rs" % (where, constant))
                for constant in literal_copies(text, strings):
                    problems.append("%s writes the value of %s as a literal: use the constant of src/stream_format.rs" % (where, constant))
    return problems


def check_plugin():
    problems = []
    if not os.path.isfile(HOME):
        return ["src/stream_format.rs, the home of STREAM_FORMAT, is missing"]
    host = read(os.path.join(REPO, "host", "src", "lib.rs"))
    if '#[path = "../../src/stream_format.rs"]' not in host:
        problems.append("host/src/lib.rs does not read src/stream_format.rs as a #[path] module")
    names = home_names()
    if "STREAM_FORMAT" not in names:
        problems.append("src/stream_format.rs has no STREAM_FORMAT")
    # One definition only: a second definition anywhere in the two crates would be a second
    # home that can drift.
    problems += check_folders(REPO, ("src", os.path.join("host", "src")), names)
    return problems


def self_test():
    """The tests of the search: no files read but the home."""
    names = {"RING_READ", "STREAM_FORMAT", "TILE_HEAD"}
    assert second_definitions("const RING_READ: usize = 16;", names) == ["RING_READ"]
    assert second_definitions("pub(crate) const TILE_HEAD: usize = 16;\npub const STREAM_FORMAT: u32 = 1;", names) == ["STREAM_FORMAT", "TILE_HEAD"]
    assert second_definitions("static RING_READ: AtomicU64 = AtomicU64::new(0);", names) == ["RING_READ"]
    # A use is no definition, nor a longer name.
    assert second_definitions("let at = RING_READ; const RING_READ_BEFORE: usize = 0;", names) == []
    assert {"RING_READ", "RING_ENDING", "HEADER_ABILITIES_AT", "MESSAGES_MAGIC", "STREAM_FORMAT"} <= home_names()
    strings = {"-read": "READ_EVENT", ".reply": "REPLY_MEMORY"}
    assert literal_copies('format!("{name}-read")', strings) == ["READ_EVENT"]
    assert literal_copies('format!("{ring}.reply{generation}")', strings) == ["REPLY_MEMORY"]
    assert literal_copies('let suffix = ".reply";', strings) == ["REPLY_MEMORY"]
    # A longer word or a use of the constant is no copy.
    assert literal_copies('println!("{name}-ready"); format!("{name}{READ_EVENT}")', strings) == []
    assert literal_copies('"the pipeline-read"', strings) == []
    assert home_strings()["-records"] == "RECORDS_EVENT" and len(home_strings()) == 6
    print("check_handshake self-test: ok")


def check_mge(mge):
    header = read(os.path.join(mge, "d3d8", "cpp", "mge", "mgeversion.h"))
    renderer = read(os.path.join(mge, "mgeHost64", "src", "renderer.rs"))
    cpp = number(header, r"^#define MGE_EXTENSION_FORMAT (\d+)", "MGE_EXTENSION_FORMAT in mgeversion.h")
    rust = number(renderer, r"^const EXTENSION_FORMAT: u32 = (\d+);", "EXTENSION_FORMAT in renderer.rs")
    problems = []
    if cpp != rust:
        problems.append("MGE_EXTENSION_FORMAT is %d in mgeversion.h and EXTENSION_FORMAT %d in renderer.rs" % (cpp, rust))
    # mgeHost64 reads the stream's constants through the host crate, never as a copy.
    problems += check_folders(mge, (os.path.join("mgeHost64", "src"),), home_names())
    return problems


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mge", nargs="?", const=MGE, help="check MGE XE's extension format, and that mgeHost64 has no copy of a constant "
                    "of src/stream_format.rs, in this folder (default: the render-link worktree)")
    ap.add_argument("--self-test", action="store_true", help="the tests of the search for a second definition")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    problems = check_mge(args.mge) if args.mge else check_plugin()
    for problem in problems:
        print(problem)
    print("handshake (%s): %d problems" % ("mge" if args.mge else "plugin", len(problems)))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
