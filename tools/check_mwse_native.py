"""Checks the plugin's copy of MWSE's native ABI against the header of MWSE.

    python tools/check_mwse_native.py [--header PATH]

1. Each `#define MWSE_NATIVE_*` number of the header has the same value in `src/mwse_native.rs`
   (the Rust name is the header name without `MWSE_`).
2. Each status of `MwseNativeStatus` has the same value in `src/mwse_native.rs`.
3. MGE XE's copy of the header (`d3d8/cpp/mwse/MWSENativeApi.h` of the render-link worktree)
   has the same text, line ends aside.

The header is `MWSE/MWSENativeApi.h` of the MWSE repository, a sibling of this repository by
default. The exit code is 0 when all checks pass.
"""
import argparse
import os
import re
import sys

from common import REPO

DEFAULT_HEADER = os.path.join(os.path.dirname(REPO), "MWSE", "MWSE", "MWSE", "MWSENativeApi.h")
RUST = os.path.join(REPO, "src", "mwse_native.rs")
COPIES = [os.path.join(os.path.dirname(REPO), "MGE-XE-G7-render-link", "d3d8", "cpp", "mwse", "MWSENativeApi.h")]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--header", default=DEFAULT_HEADER)
    args = parser.parse_args()

    text = open(args.header, "rb").read().decode("ascii")
    rust = open(RUST, encoding="utf-8").read()
    rust_consts = {m.group(1): int(m.group(2), 0) for m in re.finditer(r"pub const (\w+): u32 = (\w+);", rust)}
    problems = []

    names = re.findall(r"^#define MWSE_(NATIVE_\w+) (0x[0-9A-Fa-f]+|\d+)u$", text, re.M)
    names += re.findall(r"^\s*MWSE_(NATIVE_\w+) = (\d+),$", text, re.M)
    for name, value in names:
        if name not in rust_consts:
            problems.append("MWSE_%s has no constant %s in mwse_native.rs" % (name, name))
        elif rust_consts[name] != int(value, 0):
            problems.append("MWSE_%s is %s in the header and %d in mwse_native.rs" % (name, value, rust_consts[name]))

    for copy in COPIES:
        if not os.path.exists(copy):
            problems.append("no copy at %s" % copy)
        elif open(copy, "rb").read().replace(b"\r\n", b"\n") != text.replace("\r\n", "\n").encode("ascii"):
            problems.append("%s differs from the header of MWSE" % copy)

    for problem in problems:
        print("FAIL: " + problem)
    if not problems:
        print("mwse native ok: %d numbers" % len(names))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
