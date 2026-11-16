"""Checks the plugin's copy of MWSE's native ABI against the header of MWSE.

    python tools/check_mwse_native.py [--header PATH | --mwse DIR | --mwse-commit SHA] [--mge DIR]
    python tools/check_mwse_native.py --self-test

1. Each `#define MWSE_NATIVE_*` number of the header has the same value in `src/mwse_native.rs`
   (the Rust name is the header name without `MWSE_`).
2. Each status of `MwseNativeStatus` has the same value in `src/mwse_native.rs`.
3. MGE XE's copy of the header (`d3d8/cpp/mwse/MWSENativeApi.h` of the render-link worktree)
   has the same text, line ends aside.

The header is `MWSE/MWSENativeApi.h` of the MWSE commit that the test installs run (`mwse_tree.resolve`, W497), else of
the sibling checkout with a FALLBACK line. The exit code is 0 when all checks pass.
"""
import argparse
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import REPO, sibling  # noqa: E402
import mwse_tree  # noqa: E402

CHECK = "mwse native"
HEADER = "MWSE/MWSENativeApi.h"
RUST = os.path.join(REPO, "src", "mwse_native.rs")
DEFAULT_MGE = sibling("MGE-XE-G7-render-link")
COPY_IN_MGE = os.path.join("d3d8", "cpp", "mwse", "MWSENativeApi.h")


def compare(header, rust, copies):
    """The count of the numbers of the header text `header`, and a line for each difference from the Rust text `rust`
    and from the files `copies` (each as {path: bytes, or None when there is none})."""
    # The text can have CRLF line ends: the patterns end at "$" of a line.
    text = header.replace("\r\n", "\n")
    rust_consts = {m.group(1): int(m.group(2), 0) for m in re.finditer(r"pub const (\w+): u32 = (\w+);", rust)}
    problems = []

    names = re.findall(r"^#define MWSE_(NATIVE_\w+) (0x[0-9A-Fa-f]+|\d+)u$", text, re.M)
    names += re.findall(r"^\s*MWSE_(NATIVE_\w+) = (\d+),$", text, re.M)
    for name, value in names:
        if name not in rust_consts:
            problems.append("MWSE_%s has no constant %s in mwse_native.rs" % (name, name))
        elif rust_consts[name] != int(value, 0):
            problems.append("MWSE_%s is %s in the header and %d in mwse_native.rs" % (name, value, rust_consts[name]))
    if not names:
        # A pattern that finds nothing judges nothing (W527, W524 F-G-20 C23).
        problems.append("the header gives no MWSE_NATIVE_ number: the check judged none")

    for path, data in copies.items():
        if data is None:
            problems.append("no copy at %s" % path)
        elif data.replace(b"\r\n", b"\n") != text.encode("ascii"):
            problems.append("%s differs from the header of MWSE" % path)
    return len(names), problems


def self_test():
    rust = "pub const NATIVE_A: u32 = 2;\npub const NATIVE_B: u32 = 7;\n"
    # The plant of W497: the old header (MWSE -9) against a copy of the new one (-23, the request 7 of W489).
    old = "#define MWSE_NATIVE_A 2u\r\n"
    new = "#define MWSE_NATIVE_A 2u\r\n#define MWSE_NATIVE_B 7u\r\n"
    copy = {"copy.h": new.encode("ascii")}
    assert compare(new, rust, copy) == (2, []), "the header of the copy passes, line ends aside"
    assert compare(new, rust, {"copy.h": new.replace("\r\n", "\n").encode("ascii")}) == (2, []), "LF against CRLF passes"
    assert compare(old, rust, copy) == (1, ["copy.h differs from the header of MWSE"]), "the old header fails"
    assert compare(new, "pub const NATIVE_A: u32 = 2;\n", copy)[1] == ["MWSE_NATIVE_B has no constant NATIVE_B in mwse_native.rs"]
    assert compare(new, rust.replace("7;", "8;"), copy)[1] == ["MWSE_NATIVE_B is 7 in the header and 8 in mwse_native.rs"]
    assert compare(new, rust, {"none.h": None})[1] == ["no copy at none.h"]
    # W527 (W524 F-G-20 C23): a header with no number fails. The null: before W527 it gave (0, []), "mwse native ok: 0
    # numbers", a pass.
    reworded = new.replace("#define MWSE_", "#define MWSE_X_")
    assert compare(reworded, rust, {"copy.h": reworded.encode("ascii")}) == (0, ["the header gives no MWSE_NATIVE_ number: the check judged none"])
    print("mwse native self-test: ok")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--header", help="a header file to read in place of MWSE's")
    source.add_argument("--mwse", help="an MWSE tree on disk to read in place of the installs' commit")
    source.add_argument("--mwse-commit", help="an MWSE commit to read in place of the installs' commit")
    parser.add_argument("--mge", default=DEFAULT_MGE, help="the MGE XE tree whose copy of the header is checked (the render-link sibling without this)")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()

    if args.header:
        label = "the file %s" % args.header
        header = open(args.header, "rb").read()
    else:
        tree, fallback = mwse_tree.resolve(CHECK, folder=args.mwse, commit=args.mwse_commit)
        if fallback:
            print(fallback)
        label = tree.label
        header = tree.read(HEADER)
    copy = os.path.join(args.mge, COPY_IN_MGE)
    copies = {copy: open(copy, "rb").read() if os.path.exists(copy) else None}
    count, problems = compare(header.decode("ascii"), open(RUST, encoding="utf-8").read(), copies)

    for problem in problems:
        print("FAIL: " + problem)
    if not problems:
        print("mwse native ok: %d numbers; MWSE: %s" % (count, label))
    else:
        print("mwse native: MWSE: %s" % label)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
