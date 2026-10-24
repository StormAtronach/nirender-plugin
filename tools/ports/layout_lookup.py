"""The members of an IDA struct at given offsets, as tools/ports/gen.py names them.

For the move of a hand-written engine offset into `src/gen/layouts.rs`: the name to use, or
a sign that the IDA type disagrees with the hand-written value (then the IDA type or the
hand-written value is wrong; look before changing either).

Usage: python tools/ports/layout_lookup.py <Struct> [<offset> ...]
       (no offset: all members; the struct's size; whether gen.py's filter takes it)
"""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from gen import LAYOUT_RE, OUT, flatten, ident  # noqa: E402


def main():
    structs = json.load(open(os.path.join(OUT, "structs.json")))
    name = sys.argv[1]
    if name not in structs:
        close = [s for s in structs if name.lower() in s.lower()][:12]
        print("no struct %s; close: %s" % (name, ", ".join(close)))
        return 1
    module = ident(name.replace("::", "_"))
    print("%s: size 0x%x; module %s; %s" % (name, structs[name]["size"], module,
                                           "in the layouts" if LAYOUT_RE.search(name) else "NOT in gen.py's filter (LAYOUT_RE)"))
    members = list(flatten(structs, name))
    wanted = [int(a, 16) for a in sys.argv[2:]]
    for offset in wanted or sorted({m[1] for m in members}):
        hits = [m for m in members if m[1] == offset]
        if not hits:
            print("  0x%x: no member starts there" % offset)
        for mname, off, size, mtype in hits:
            print("  0x%x: %s::%s (%s, 0x%x bytes)" % (off, module, ident(mname), mtype, size))
    return 0


if __name__ == "__main__":
    sys.exit(main())
