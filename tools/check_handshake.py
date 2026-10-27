"""The version handshake of the four parts: each number has one home, and the copies agree.

    python -I tools/check_handshake.py            # this repository: the stream format
    python -I tools/check_handshake.py --mge DIR  # MGE XE: the extension format

This repository: `host/src/lib.rs` reads `src/stream_format.rs` as a `#[path]` module, so the
plugin and the host crate compare one STREAM_FORMAT. MGE XE: MGE_EXTENSION_FORMAT in
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


def check_plugin():
    problems = []
    home = os.path.join(REPO, "src", "stream_format.rs")
    if not os.path.isfile(home):
        problems.append("src/stream_format.rs, the home of STREAM_FORMAT, is missing")
    host = read(os.path.join(REPO, "host", "src", "lib.rs"))
    if '#[path = "../../src/stream_format.rs"]' not in host:
        problems.append("host/src/lib.rs does not read src/stream_format.rs as a #[path] module")
    if "STREAM_FORMAT" in host.replace("stream_format", ""):
        problems.append("host/src/lib.rs names a STREAM_FORMAT of its own")
    return problems


def check_mge(mge):
    header = read(os.path.join(mge, "d3d8", "cpp", "mge", "mgeversion.h"))
    renderer = read(os.path.join(mge, "mgeHost64", "src", "renderer.rs"))
    cpp = number(header, r"^#define MGE_EXTENSION_FORMAT (\d+)", "MGE_EXTENSION_FORMAT in mgeversion.h")
    rust = number(renderer, r"^const EXTENSION_FORMAT: u32 = (\d+);", "EXTENSION_FORMAT in renderer.rs")
    if cpp != rust:
        return ["MGE_EXTENSION_FORMAT is %d in mgeversion.h and EXTENSION_FORMAT %d in renderer.rs" % (cpp, rust)]
    return []


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mge", nargs="?", const=MGE, help="check MGE XE's extension format in this folder (default: the render-link worktree)")
    args = ap.parse_args()
    problems = check_mge(args.mge) if args.mge else check_plugin()
    for problem in problems:
        print(problem)
    print("handshake (%s): %d problems" % ("mge" if args.mge else "plugin", len(problems)))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
