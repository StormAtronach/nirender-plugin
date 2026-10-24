"""The engine field offsets that the plugin writes by hand, outside the generated layouts.

An engine field offset belongs in `src/gen/layouts.rs` (tools/ports/gen.py, from the IDA
types). A hand-written one can be wrong without a sign: `skin_data_complete` read the weight
array and the count at swapped offsets until nirender 666d099. This tool finds them:

- a hex literal as the offset argument of `rd`, `wr` or `ptr_at`;
- a `const NAME: usize = 0x...;` below 0x10000 (an offset, not an address) in a file that
  reads engine memory.

`python tools/hand_offsets.py` lists them by file. `--check` compares the counts with
`tools/hand_offsets.baseline` and fails when a file has more than its baseline (a new hand
offset); a file with fewer lowers its line with `--update`. The baseline only goes down.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import REPO  # noqa: E402

SRC = os.path.join(REPO, "src")
BASELINE = os.path.join(REPO, "tools", "hand_offsets.baseline")
CALL = re.compile(r"\b(?:rd|wr|ptr_at)(?:::<[^>]+>)?\(\s*[^,()]+(?:\([^()]*\))?\s*,\s*(0x[0-9A-Fa-f]+)\s*[,)]")
CONST = re.compile(r"\bconst\s+([A-Z0-9_]+)\s*:\s*usize\s*=\s*(0x[0-9A-Fa-f]+)\s*;")
# A decimal offset: a decimal constant under a doc line that names a field (`Class::field`).
DECIMAL = re.compile(r"\bconst\s+([A-Z0-9_]+)\s*:\s*usize\s*=\s*([0-9]+)\s*;")
FIELD_DOC = re.compile(r"^\s*///\s*`[A-Za-z0-9_]+(?:::[A-Za-z0-9_$\[\]]+)+(?:\.[A-Za-z0-9_]+)*`")
READS_ENGINE = re.compile(r"\b(?:rd|wr|ptr_at)(?:::<[^>]+>)?\(")
# Files whose offsets are not the engine's, with the reason.
EXEMPT = {
    # The slots of the Direct3D 8 interfaces: the published ABI of d3d8.h, each with its
    # method's name. The IDA export lacks some of their vtables (IDirect3DDevice8).
    "src/d3d.rs",
}


def scan():
    found = {}
    for root, _, files in os.walk(SRC):
        if os.path.relpath(root, SRC).split(os.sep)[0] == "gen":
            continue
        for name in files:
            if not name.endswith(".rs"):
                continue
            path = os.path.join(root, name)
            rel = os.path.relpath(path, REPO).replace("\\", "/")
            if rel in EXEMPT:
                continue
            text = open(path, encoding="utf-8").read()
            if not READS_ENGINE.search(text):
                continue
            hits = []
            lines = text.splitlines()
            for number, line in enumerate(lines, 1):
                code = line.split("//", 1)[0]
                for match in CALL.finditer(code):
                    hits.append((number, "read", match.group(1), line.strip()))
                for match in CONST.finditer(code):
                    if int(match.group(2), 16) < 0x10000:
                        hits.append((number, "const", match.group(2), line.strip()))
                match = DECIMAL.search(code)
                if match and number > 1 and FIELD_DOC.match(lines[number - 2]):
                    hits.append((number, "const", match.group(2), line.strip()))
            if hits:
                found[rel] = hits
    return found


def read_baseline():
    if not os.path.exists(BASELINE):
        return {}
    out = {}
    for line in open(BASELINE, encoding="utf-8"):
        line = line.strip()
        if line and not line.startswith("#"):
            path, count = line.rsplit(" ", 1)
            out[path] = int(count)
    return out


def write_baseline(found):
    with open(BASELINE, "w", encoding="utf-8", newline="\n") as fh:
        fh.write("# The hand-written engine offsets of each file (tools/hand_offsets.py). It only goes down.\n")
        for path in sorted(found):
            fh.write("%s %d\n" % (path, len(found[path])))


def main():
    found = scan()
    if "--update" in sys.argv:
        old = read_baseline()
        grown = [p for p in found if len(found[p]) > old.get(p, 0)] if old else []
        if grown:
            print("hand offsets: refusing to raise the baseline of %s" % ", ".join(grown))
            return 1
        write_baseline(found)
        print("hand offsets: baseline written, %d in %d files" % (sum(len(v) for v in found.values()), len(found)))
        return 0
    if "--check" in sys.argv:
        base = read_baseline()
        grown = [(p, len(v), base.get(p, 0)) for p, v in sorted(found.items()) if len(v) > base.get(p, 0)]
        if grown:
            for path, count, allowed in grown:
                print("hand offsets: %s has %d, the baseline %d: use src/gen/layouts.rs (tools/ports/gen.py)" % (path, count, allowed))
            return 1
        lower = sum(1 for p in base if len(found.get(p, [])) < base[p])
        print("hand offsets: %d in %d files, none new%s" % (sum(len(v) for v in found.values()), len(found),
                                                           "; %d files below the baseline (--update)" % lower if lower else ""))
        return 0
    for path in sorted(found, key=lambda p: -len(found[p])):
        print("%s: %d" % (path, len(found[path])))
        if "-v" in sys.argv:
            for number, kind, value, line in found[path]:
                print("  %5d %-5s %-7s %s" % (number, kind, value, line[:110]))
    print("total %d in %d files" % (sum(len(v) for v in found.values()), len(found)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
