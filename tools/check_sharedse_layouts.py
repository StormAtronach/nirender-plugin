"""The IDA types of src/gen/layouts.rs against MWSE's own layouts (SharedSE NI*.h, MWSE TES3*.h).

MWSE states its layouts in two ways that a script can read: `static_assert(sizeof(X) == N)`
and an offset on a field (a `// 0xNN` comment, or a name `unknown_0xNN`). For each struct
that gen.py puts into the layouts, the size and each such field offset are compared with the
IDA export (tools/out/structs.json): a size that differs, or a field offset where no IDA
member starts. A difference means that one of the two types is wrong; IDA's disassembly
decides.

The headers are those of the MWSE commit that the test installs run (`mwse_tree.resolve`, W497), else of the sibling
checkout with a FALLBACK line.

Usage: python tools/check_sharedse_layouts.py [--all] [--mwse DIR | --mwse-commit SHA]   (--all: every struct MWSE
asserts, not only those of the layouts)
"""
import argparse
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "ports"))
from gen import LAYOUT_RE, OUT, flatten  # noqa: E402
import mwse_tree  # noqa: E402

CHECK = "sharedse layouts"
# Differences that were looked at in IDA and do not change an offset either side reads, with
# the reason. A struct here is still compared; its differences are listed as known.
KNOWN = {
    "NI::Texture::RendererData": "IDA ends the base at 0xC and puts +0xC (field_C) in NiDX8TextureData; MWSE puts it "
    "in the base (unknown_0xC). The derived types agree (renderer at 0x10, size 0x50). The engine stores the texture "
    "at +8 and the next renderer data at +4 (NiDX8TextureData::ctor_args, 0x6B6D0F); MWSE names +4 `texture` and "
    "never reads it (2026-10-24).",
}

SIZE = re.compile(r"static_assert\s*\(\s*sizeof\s*\(\s*([\w:]+)\s*\)\s*==\s*(0x[0-9A-Fa-f]+|\d+)")
STRUCT = re.compile(r"^\s*struct\s+(\w+)\b[^;]*\{")
FIELD_COMMENT = re.compile(r"^\s*[^/]*?\b(\w+)\s*(?:\[[^\]]*\])?\s*;\s*//\s*(0x[0-9A-Fa-f]+)\b")
FIELD_NAME = re.compile(r"^\s*[^/(]*?\bunknown_(0x[0-9A-Fa-f]+)\b\s*(?:\[[^\]]*\])?\s*;")


def headers(tree):
    """(path with "/", namespace) of the NI*.h of SharedSE and the TES3*.h of MWSE in the MWSE source `tree`."""
    for folder, prefix, namespace in (("SharedSE", "NI", "NI"), ("MWSE", "TES3", "TES3")):
        for name in tree.names(folder):
            if name.startswith(prefix) and name.endswith(".h"):
                yield "%s/%s" % (folder, name), namespace


def parse(text, namespace):
    """Sizes and field offsets of the structs of one header text, by their MWSE name (NS::A::B)."""
    sizes, fields = {}, {}
    stack, depth = [], 0
    for number, line in enumerate(text.replace("\r\n", "\n").splitlines(True), 1):
        match = STRUCT.match(line)
        if match:
            stack.append((match.group(1), depth))
        current = "::".join([namespace] + [s[0] for s in stack]) if stack else None
        if current:
            code = line.split("//")[0]
            if "(" not in code:  # a field, not a method
                found = FIELD_COMMENT.match(line)
                if found:
                    fields.setdefault(current, []).append((int(found.group(2), 16), found.group(1), number))
                found = FIELD_NAME.match(line)
                if found:
                    fields.setdefault(current, []).append((int(found.group(1), 16), "unknown_" + found.group(1), number))
        for match in SIZE.finditer(line):
            name = match.group(1)
            if not name.startswith(namespace + "::"):
                name = "::".join([namespace] + [s[0] for s in stack[:-0 or None]][:0] + [name]) if "::" not in name else namespace + "::" + name
            sizes[name] = (int(match.group(2), 0), number)
        depth += line.count("{") - line.count("}")
        while stack and depth <= stack[-1][1]:
            stack.pop()
    return sizes, fields


def ida_name(mwse_name, structs):
    namespace, *parts = mwse_name.split("::")
    head, rest = parts[0], parts[1:]
    # NI::X is NiX in the database (else X); TES3::X is X (else TES3X), never an Ni type.
    candidates = ("Ni" + head, head) if namespace == "NI" else (head, "TES3" + head)
    for candidate in candidates:
        full = "::".join([candidate] + rest)
        if full in structs:
            return full
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--all", action="store_true", help="every struct MWSE asserts, not only those of the layouts")
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--mwse", help="an MWSE tree on disk to read in place of the installs' commit")
    source.add_argument("--mwse-commit", help="an MWSE commit to read in place of the installs' commit")
    args = parser.parse_args()
    tree, fallback = mwse_tree.resolve(CHECK, folder=args.mwse, commit=args.mwse_commit)
    if fallback:
        print(fallback)
    structs = json.load(open(os.path.join(OUT, "structs.json")))
    everything = args.all
    problems, compared, sizes_compared, fields_compared = [], 0, 0, 0
    for rel, namespace in headers(tree):
        sizes, fields = parse(tree.read(rel).decode("utf-8", "replace"), namespace)
        for name in sorted(set(sizes) | set(fields)):
            ida = ida_name(name, structs)
            if not ida or not (everything or LAYOUT_RE.search(ida)):
                continue
            compared += 1
            sizes_compared += name in sizes
            fields_compared += len(fields.get(name, []))
            if name in sizes and sizes[name][0] != structs[ida]["size"]:
                problems.append("%s:%d %s: MWSE size 0x%x, IDA %s 0x%x" % (rel, sizes[name][1], name, sizes[name][0], ida, structs[ida]["size"]))
            starts = {offset for _, offset, _, _ in flatten(structs, ida)}
            for offset, field, number in fields.get(name, []):
                if offset >= structs[ida]["size"]:
                    problems.append("%s:%d %s::%s at 0x%x: past the end of %s (0x%x)" % (rel, number, name, field, offset, ida, structs[ida]["size"]))
                elif offset not in starts and not field.startswith("unknown_"):
                    # A placeholder inside a wider IDA member is less detail, not a contradiction.
                    problems.append("%s:%d %s::%s at 0x%x: no IDA member of %s starts there" % (rel, number, name, field, offset, ida))
    known = [p for p in problems if any((" %s:" % name) in p or (" %s::" % name) in p for name in KNOWN)]
    new = [p for p in problems if p not in known]
    for problem in new:
        print("sharedse layouts: " + problem)
    print("sharedse layouts: %d structs compared (%d sizes, %d field offsets), %d differences, %d known (KNOWN); MWSE: %s" % (
        compared, sizes_compared, fields_compared, len(new), len(known), tree.label))
    return 1 if new else 0


if __name__ == "__main__":
    sys.exit(main())
