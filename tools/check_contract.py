"""Checks the render link contract.

    python tools/check_contract.py

1. Each `#define` number of the header has the same value in `src/link.rs`.
2. Each struct size of the header has the same value in `src/link.rs`.

The exit code is 0 when all checks pass.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import REPO  # noqa: E402

HEADER = os.path.join(REPO, "contract", "mge_render_link.h")
RUST = os.path.join(REPO, "src", "link.rs")

# Header prefixes, longest first. The Rust name is the header name without its prefix.
PREFIXES = ("MGE_RENDER_LINK_", "MGE_LINK_")


def rust_name(name):
    for prefix in PREFIXES:
        if name.startswith(prefix):
            return name[len(prefix):]
    return name


def camel(name):
    """`MgeLinkSceneV1` and `MgeRenderLinkHostV1` to `SceneV1` and `HostV1`."""
    return re.sub(r"^Mge(Render)?Link", "", name)


def main():
    header = open(HEADER, "rb").read()
    rust = open(RUST, encoding="utf-8").read()
    problems = []

    text = header.decode("ascii")
    rust_consts = {m.group(1): int(m.group(2), 0) for m in re.finditer(r"pub const (\w+): u32 = (\w+);", rust)}
    defines = re.findall(r"^#define (MGE_\w+) (0x[0-9A-Fa-f]+|\d+)u$", text, re.M)
    for name, value in defines:
        short = rust_name(name)
        if short not in rust_consts:
            problems.append("%s has no constant %s in link.rs" % (name, short))
        elif rust_consts[short] != int(value, 0):
            problems.append("%s is %s in the header and %d in link.rs" % (name, value, rust_consts[short]))

    header_sizes = dict(re.findall(r"static_assert\(sizeof\((\w+)\) == (\d+)", text))
    header_sizes.pop("void", None)
    rust_sizes = dict(re.findall(r"assert!\(size_of::<(\w+)>\(\) == (\d+)\);", rust))
    for name, size in header_sizes.items():
        short = camel(name)
        if rust_sizes.get(short) != size:
            problems.append("%s is %s bytes in the header and %s in link.rs" % (name, size, rust_sizes.get(short)))

    for problem in problems:
        print("FAIL: " + problem)
    if not problems:
        print("contract ok: %d constants, %d structs" % (len(defines), len(header_sizes)))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
