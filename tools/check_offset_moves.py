"""Proves that the offsets moved into the generated layouts keep their values.

For each constant whose literal the working tree (or `--cached`) replaced with a path into
`src/gen/layouts.rs`, the old literal must equal the generated value. A difference is not
a typo to fix here: the IDA type or the old literal is wrong, and IDA decides
(tools/ports/layout_lookup.py). Arrays of offsets are compared element by element.

Usage: python tools/check_offset_moves.py [--cached] [--base <rev>]
"""
import os
import re
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import REPO  # noqa: E402

LAYOUTS = os.path.join(REPO, "src", "gen", "layouts.rs")
CONST = re.compile(r"\bconst\s+([A-Za-z0-9_]+)\s*:\s*([^=]+?)\s*=\s*(.+?);\s*$")
MOD = re.compile(r"^pub mod (\w+) \{")
MEMBER = re.compile(r"^\s+pub const (\w+): usize = (0x[0-9a-f]+);")
USE = re.compile(r"use crate::gen::layouts::(\{[^}]*\}|[\w:]+(?:\s+as\s+\w+)?)\s*;")


def layouts():
    values, module = {}, None
    for line in open(LAYOUTS, encoding="utf-8"):
        match = MOD.match(line)
        if match:
            module = match.group(1)
            values[module] = {}
            continue
        match = MEMBER.match(line)
        if match and module:
            values[module][match.group(1)] = int(match.group(2), 16)
    return values


def aliases(text):
    out = {}
    for match in USE.finditer(text):
        body = match.group(1).strip("{}")
        for part in body.split(","):
            part = part.strip()
            if not part:
                continue
            if " as " in part:
                name, alias = [p.strip() for p in part.split(" as ")]
            else:
                name = alias = part.split("::")[-1]
            out[alias] = name.split("::")[-1]
    return out


def number(text):
    text = text.strip()
    return int(text, 16) if text.lower().startswith("0x") else int(text)


PATH = re.compile(r"(?:(?:crate::gen::)?layouts::)?([A-Za-z_][A-Za-z0-9_]*)::([A-Za-z_][A-Za-z0-9_]*)")
ARITHMETIC = re.compile(r"^[0-9a-fA-Fx+\-*/ ()]+$")


D3D_CONST = re.compile(r"^pub const ([A-Z0-9_]+): (?:u32|usize) = (0x[0-9A-Fa-f_]+|[0-9_]+);", re.M)
NAME = re.compile(r"\b(?:crate::d3d::)?([A-Z][A-Z0-9_]+)\b")


def d3d_constants():
    text = open(os.path.join(REPO, "src", "d3d.rs"), encoding="utf-8").read()
    return {name: int(value.replace("_", ""), 0) for name, value in D3D_CONST.findall(text)}


D3D = d3d_constants()


def resolve(expr, mods, alias):
    """The value of a generated path, or of integer arithmetic over such paths and the
    constants of `d3d.rs` (`32 * X::SIZE`, `D3DRS_VERTEXBLEND as usize * X::SIZE`)."""
    missing = []

    def value(match):
        found = mods.get(alias.get(match.group(1), match.group(1)), {}).get(match.group(2))
        if found is None:
            missing.append(match.group(0))
            return "0"
        return str(found)

    def constant(match):
        if match.group(1) not in D3D:
            missing.append(match.group(0))
            return "0"
        return str(D3D[match.group(1)])

    text = PATH.sub(value, expr.strip()).replace(" as usize", "")
    text = NAME.sub(constant, text)
    if missing or not ARITHMETIC.match(text):
        return None
    # Digits, hex literals, + - * / and parentheses only; Rust's `/` on usize is a floor division.
    return eval(text.replace("/", "//"), {"__builtins__": {}})


def elements(value):
    value = value.strip()
    if value.startswith("[") and value.endswith("]"):
        return [v for v in value[1:-1].split(",") if v.strip()]
    return [value]


def main():
    cached = ["--cached"] if "--cached" in sys.argv else []
    base = sys.argv[sys.argv.index("--base") + 1] if "--base" in sys.argv else None
    rev = [base] if base else []
    diff = subprocess.run(["git", "-C", REPO, "diff", *cached, *rev, "-U0", "--no-color", "--", "src"], capture_output=True, text=True, encoding="utf-8").stdout
    mods = layouts()
    checked, problems, path = 0, [], None
    removed = {}
    added = []
    for line in diff.splitlines():
        if line.startswith("+++ b/"):
            path = line[6:]
            continue
        if line.startswith("---"):
            continue
        if line.startswith("-"):
            match = CONST.search(line[1:])
            if match:
                removed[(path, match.group(1))] = match.group(3)
        elif line.startswith("+"):
            match = CONST.search(line[1:])
            if match:
                added.append((path, match.group(1), match.group(3)))
    texts = {}
    for path_, name, new in added:
        old = removed.get((path_, name))
        if old is None or "::" not in new:
            continue
        try:
            old_values = [number(v) for v in elements(old)]
        except ValueError:
            continue
        if path_ not in texts:
            texts[path_] = open(os.path.join(REPO, path_), encoding="utf-8").read()
        alias = aliases(texts[path_])
        # Only a replacement that takes a generated layout is this check's (not `size_of`).
        if not any(alias.get(m.group(1), m.group(1)) in mods for m in PATH.finditer(new)):
            continue
        new_parts = elements(new)
        if len(new_parts) != len(old_values):
            problems.append("%s %s: %d values became %d" % (path_, name, len(old_values), len(new_parts)))
            continue
        for old_value, part in zip(old_values, new_parts):
            value = resolve(part, mods, alias)
            checked += 1
            if value is None:
                problems.append("%s %s: %s is not a generated offset" % (path_, name, part.strip()))
            elif value != old_value:
                problems.append("%s %s: was 0x%x, %s is 0x%x" % (path_, name, old_value, part.strip(), value))
    for problem in problems:
        print("offset moves: " + problem)
    print("offset moves: %d values checked, %d problems" % (checked, len(problems)))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
