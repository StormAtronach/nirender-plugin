"""Merges a port branch into the current branch and registers its module.

Every port branch adds a line to the same two lists in src/ports/mod.rs, so the merges
conflict there. This takes our side of that file and adds the lines of the new module.

    python tools/merge_port.py <branch> <module> [--sites]
"""
import os
import re
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MOD = os.path.join(REPO, "src", "ports", "mod.rs")


def git(*args, check=True):
    return subprocess.run(["git", "-C", REPO, *args], capture_output=True, text=True, check=check)


def main():
    branch, module = sys.argv[1], sys.argv[2]
    sites = "--sites" in sys.argv
    merge = git("merge", "--no-commit", "--no-ff", branch, check=False)
    conflicts = git("diff", "--name-only", "--diff-filter=U").stdout.split()
    others = [c for c in conflicts if c != "src/ports/mod.rs"]
    if others:
        raise SystemExit("conflicts outside mod.rs: %s" % others)
    if "src/ports/mod.rs" in conflicts or merge.returncode != 0:
        git("checkout", "--ours", "src/ports/mod.rs")
    text = open(MOD, encoding="utf-8").read()
    if "<<<<<<<" in text:
        raise SystemExit("mod.rs still has conflict markers")

    if "pub mod %s;" % module not in text:
        mods = re.findall(r"^pub mod (\w+);$", text, re.M)
        ordered = sorted(mods + [module])
        block = "".join("pub mod %s;\n" % m for m in ordered)
        first = text.index("pub mod %s;" % mods[0])
        last = text.index("pub mod %s;\n" % mods[-1]) + len("pub mod %s;\n" % mods[-1])
        text = text[:first] + block + text[last:]
    if "%s::PORTS" % module not in text:
        start = text.index("static MODULES")
        end = text.index("];", start)
        text = text[:end] + "    %s::PORTS,\n" % module + text[end:]
    if sites and "%s::SITES" % module not in text:
        text = text.replace("static SITE_LISTS: &[&[usize]] = &[];", "static SITE_LISTS: &[&[usize]] = &[\n];")
        start = text.index("static SITE_LISTS")
        end = text.index("];", start)
        text = text[:end] + "    %s::SITES,\n" % module + text[end:]
    open(MOD, "w", encoding="utf-8", newline="\n").write(text)
    git("add", "-A")
    git("commit", "-q", "-m", "Merge the %s ports\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>" % module)
    print("merged %s as module %s" % (branch, module))


if __name__ == "__main__":
    main()
