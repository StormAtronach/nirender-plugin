"""Checks that each vtable slot MWSE fills from a member function gets the member of its own name.

    python -I tools/check_mwse_vtable_fills.py [--mwse DIR]   (DIR: the MWSE repository)
    python -I tools/check_mwse_vtable_fills.py --self-test

MWSE builds the vtables of its own NI classes in C++: the constructor `X_vTable::X_vTable()`
fills one slot for each line, `slot = ConvertThisCall<decltype(slot)>(&Class::member);`. The
compiler accepts any member of the same size, so a copied line whose member was not changed
fills the slot with the wrong function. W93: the slot targetIsRequiredType of
NICopyTransformController.cpp got getRunTimeTypeInformation, and it answered true for every
target.

The rule: in the body of a vtable constructor, each slot that gets a member function gets the
member with the slot's name. The pairs in KNOWN are the only exceptions. The files are the .cpp
and .h files of SharedSE, MWSE and CSSE. The MWSE repository is a sibling of this repository by
default, else a sibling of its main checkout. The exit code is 0 when no slot breaks the rule.
"""
import argparse
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import sibling  # noqa: E402

# Beside this tree, else beside its main checkout (a group of worktrees in C:/tmp has neither beside it).
DEFAULT_MWSE = sibling("MWSE", "MWSE")
FOLDERS = ("SharedSE", "MWSE", "CSSE")
# (slot, member): the slot of the engine's vtable has another name than MWSE's member.
KNOWN = {("getRTTI", "getRunTimeTypeInformation")}

CONSTRUCTOR = re.compile(r"\b(\w+_vTable)::\1\s*\(\s*\)")
FILL = re.compile(r"^\s*(\w+)\s*=\s*[^;]*&\s*(?:\w+::)*\w+::(\w+)\s*\)?\s*;")


def read_fills(lines):
    """The count of the fills in the vtable constructors of `lines`, and (line number, slot, member) for each fill
    whose member has another name."""
    count, wrong = 0, []
    depth, body = 0, None
    for number, line in enumerate(lines, 1):
        code = line.split("//")[0]
        if body is None and CONSTRUCTOR.search(code):
            body = depth
        if body is not None and depth > body:
            match = FILL.match(code)
            if match:
                count += 1
                if match.group(1) != match.group(2) and (match.group(1), match.group(2)) not in KNOWN:
                    wrong.append((number, match.group(1), match.group(2)))
        depth += code.count("{") - code.count("}")
        if body is not None and depth <= body and "}" in code:
            body = None
    return count, wrong


def wrong_fills(lines):
    return read_fills(lines)[1]


def sources(mwse):
    for folder in FOLDERS:
        place = os.path.join(mwse, folder)
        if not os.path.isdir(place):
            continue
        for name in sorted(os.listdir(place)):
            if name.endswith((".cpp", ".h")):
                yield os.path.join(place, name)


def self_test():
    head = ["namespace NI {", "\tCopyTransformController_vTable::CopyTransformController_vTable() {"]
    tail = ["\t}", "}"]
    good = [
        "\t\tgetRTTI = ConvertThisCall<decltype(getRTTI)>(&CopyTransformController::getRunTimeTypeInformation);",
        "\t\ttargetIsRequiredType = ConvertThisCall<decltype(targetIsRequiredType)>(&CopyTransformController::targetIsRequiredType);",
        "\t\tupdate = &NI::CopyTransformController::update;",
    ]
    assert read_fills(head + good + tail) == (3, []), "the slots of their own names and the known pair pass, all 3 read"

    # The plant: line 129 of NICopyTransformController.cpp as it was before W93.
    planted = "\t\ttargetIsRequiredType = ConvertThisCall<decltype(targetIsRequiredType)>(&CopyTransformController::getRunTimeTypeInformation);"
    assert wrong_fills(head + [planted] + tail) == [(3, "targetIsRequiredType", "getRunTimeTypeInformation")], "the W93 line fails"
    assert wrong_fills(head + ["\t\tupdate = &CopyTransformController::isEqual;"] + tail) == [(3, "update", "isEqual")], \
        "a fill with no converter fails too"

    outside = ["\tCopyTransformController::CopyTransformController(AVObject* from) {",
               "\t\tvTable.asController = &CopyTransformController_vTable::Instance;", "\t\tcopy = &Other::thing;", "\t}"]
    assert wrong_fills(outside + head + good + tail + outside) == [], "a line outside a vtable constructor is no fill"
    assert wrong_fills(head + ["\t\tif (x) {", "\t\t}", planted] + tail) == [(5, "targetIsRequiredType", "getRunTimeTypeInformation")], \
        "an inner block does not end the body"

    # The scan of a tree: MWSE's only vtable constructor is in SharedSE, so each folder of FOLDERS must be read.
    import tempfile

    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as mwse:
        for folder in FOLDERS:
            os.makedirs(os.path.join(mwse, folder))
        source = os.path.join(mwse, "SharedSE", "NICopyTransformController.cpp")
        with open(source, "w", newline="\r\n") as out:
            out.write("\n".join(head + good + tail) + "\n")
        assert scan(mwse) == (1, 3, []), "the fills of SharedSE are read"
        with open(source, "w", newline="\r\n") as out:
            out.write("\n".join(head + [planted] + tail) + "\n")
        files, fills, problems = scan(mwse)
        assert fills == 1 and len(problems) == 1 and ":3: the slot targetIsRequiredType" in problems[0], "the W93 line fails"
        os.remove(source)
        files, fills, problems = check(mwse)
        assert fills == 0 and len(problems) == 1 and "the scan reads nothing" in problems[0], "a tree with no fill fails"
    print("mwse vtable fills self-test: ok")
    return 0


def scan(mwse):
    """The count of the files and of the fills of the tree `mwse`, and a line for each fill that breaks the rule."""
    files, fills, problems = 0, 0, []
    for path in sources(mwse):
        files += 1
        count, wrong = read_fills(open(path, encoding="utf-8", errors="replace").read().splitlines())
        fills += count
        for number, slot, member in wrong:
            problems.append("%s:%d: the slot %s gets the member %s" % (os.path.relpath(path, mwse), number, slot, member))
    return files, fills, problems


def check(mwse):
    """`scan`, with a problem when the scan reads no fill: a folder or a pattern that misses MWSE's vtables."""
    files, fills, problems = scan(mwse)
    if fills == 0:
        problems.append("no vtable fill in %d files of %s in %s: the scan reads nothing" % (files, "/".join(FOLDERS), mwse))
    return files, fills, problems


def main_with(mwse):
    files, fills, problems = check(mwse)
    for problem in problems:
        print("FAIL: " + problem)
    if not problems:
        print("mwse vtable fills ok: %d fills in %d files of %s" % (fills, files, mwse))
    return 1 if problems else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--mwse", default=DEFAULT_MWSE, help="the MWSE repository (the sibling without this)")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    return main_with(args.mwse)


if __name__ == "__main__":
    sys.exit(main())
