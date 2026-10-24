"""Lists the public items of the shared modules that no other code uses.

The crate is a library, so the compiler does not warn about a public item without a user.

    python tools/unused_shared.py [--delete]

With --delete, the script removes each unused item with its doc comment and attributes.
"""
import glob
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import REPO  # noqa: E402

SHARED = ("src/engine.rs", "src/d3d.rs", "src/ports/ni.rs", "src/ports/hash_map.rs", "src/x87.rs")
ITEM = re.compile(r"^pub(?:\(crate\))? (?:unsafe )?(?:const fn|fn|const|static) (\w+)", re.M)


def sources():
    out = {}
    for path in glob.glob(os.path.join(REPO, "src", "**", "*.rs"), recursive=True):
        rel = os.path.relpath(path, REPO).replace(os.sep, "/")
        if not rel.startswith("src/gen/"):
            out[rel] = open(path, encoding="utf-8").read()
    return out


def without_tests(text):
    cut = text.find("#[cfg(test)]\nmod tests")
    return text if cut < 0 else text[:cut]


def uses(name, texts):
    return sum(len(re.findall(r"\b%s\b" % re.escape(name), text)) for text in texts)


def remove_item(text, name):
    """Removes the item `name` with the comment and attribute lines above it."""
    m = re.search(r"^pub(?:\(crate\))? (?:unsafe )?(?:const fn|fn|const|static) %s\b" % re.escape(name), text, re.M)
    start = m.start()
    lines_before = text[:start].split("\n")
    # The last element is the empty text before the item line.
    index = len(lines_before) - 2
    while index >= 0 and (lines_before[index].startswith("///") or lines_before[index].startswith("#[")):
        index -= 1
    start = len("\n".join(lines_before[:index + 1])) + (1 if index >= 0 else 0)
    line_end = text.index("\n", m.start())
    if text[m.start():line_end].rstrip().endswith(";"):
        end = line_end + 1
    else:
        end = text.index("\n}\n", m.start()) + 3
    # One empty line stays between the neighbours.
    while text[end:end + 1] == "\n" and text[:start].endswith("\n\n"):
        end += 1
    return text[:start] + text[end:]


def main():
    delete = "--delete" in sys.argv
    texts = sources()
    total = 0
    for rel in SHARED:
        text = texts[rel]
        # A use is in code. A name in a comment, or in the tests of its own module, is not a use.
        others = [re.sub(r"//[^\n]*", "", without_tests(t) if r == rel else t) for r, t in texts.items()]
        unused = [name for name in ITEM.findall(without_tests(text)) if uses(name, others) <= 1]
        total += len(unused)
        print("%s: %d unused: %s" % (rel, len(unused), " ".join(unused)))
        if delete and unused:
            for name in unused:
                text = remove_item(text, name)
            open(os.path.join(REPO, rel), "w", encoding="utf-8", newline="\n").write(text)
            texts[rel] = text
    return 1 if total and not delete else 0


if __name__ == "__main__":
    sys.exit(main())
