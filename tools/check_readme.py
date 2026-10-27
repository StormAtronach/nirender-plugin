"""Checks README.md against the code.

    python tools/check_readme.py
    python tools/check_readme.py --mge     # MGE XE's switches against its RENDERER.md
    python tools/check_readme.py --mge --mge-dir DIR    # the same for another MGE XE tree

1. Every switch (`NIRENDER_*`, `MGE_XE_*`) that the code names in quotes (the plugin, the host
   crate, the d3d8 crate, the tools and the Lua mod) is in README.md: the quoted name is the
   read, whatever reads it (`env::var`, the helper's `is`, `os.getenv`, a tool's environment).
2. Every switch that README.md names in back-quotes is still named by the code.
3. Every path that README.md names in back-quotes and that looks like a file or a folder of
   the repo exists.
4. Every script `tools/*.py` has a row in tools/README.md (its name in back-quotes).

With `--mge`: checks 1 and 2 for MGE XE's code (`mgeHost64/src`, `d3d8/src` of the
MGE-XE-G7-render-link worktree next to this repo) against its RENDERER.md. A switch that the C++
of `d3d8/cpp` names counts as read for check 2 (a RENDERER.md row of a switch that d3d8.dll
reads is not "named but not read"); check 1 does not yet ask a row for each C++ read: the trees
of 2026-10-27 lack the rows of MGE_XE_STALL_LOG and NIRENDER_HOST_EXE.

The exit code is 0 when all checks pass.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import REPO  # noqa: E402

README = os.path.join(REPO, "README.md")
MGE = os.path.join(os.path.dirname(REPO), "MGE-XE-G7-render-link")
# A switch the code reads: its name in quotes.
READS = re.compile(r"[\"'](NIRENDER_[A-Z0-9_]+|MGE_XE_[A-Z0-9_]+)[\"']")
NAME = re.compile(r"\b(?:NIRENDER|MGE_XE)_[A-Z0-9_]+\b")
# A switch a document names: in back-quotes, alone or with its value.
NAMED = re.compile(r"`((?:NIRENDER|MGE_XE)_[A-Z0-9_]+)[=`]")
PATH = re.compile(r"`([A-Za-z0-9_./-]+/[A-Za-z0-9_./-]*|[A-Za-z0-9_-]+\.(?:md|py|rs|lua|toml|h|nircap))`")
# README names a file of a run or a build, not of the repo.
NOT_IN_REPO = ("results/", "target/", "tools/out", "MWSE/", "Data Files", "nirender-<", "<")


def code_files(repo, folders):
    for folder in folders:
        for root, _, names in os.walk(os.path.join(repo, folder)):
            for name in names:
                if name.endswith((".rs", ".py", ".lua", ".cpp", ".h")):
                    yield os.path.join(root, name)


def reads_in(repo, folders):
    """The switches that the code of `folders` names in quotes, with the first file of each."""
    read = {}
    for path in code_files(repo, folders):
        text = open(path, encoding="utf-8", errors="replace").read()
        for name in READS.findall(text):
            read.setdefault(name, os.path.relpath(path, repo).replace("\\", "/"))
    return read


def switches(repo, folders, document, also_read=()):
    """Checks 1 and 2: the problems, and the switches read. The code of `also_read` counts as
    read for check 2 only."""
    text_of_document = open(document, encoding="utf-8").read()
    documented = set(NAME.findall(text_of_document))
    read = reads_in(repo, folders)
    known = set(read) | set(reads_in(repo, also_read))
    doc = os.path.basename(document)
    problems = [f"{name} is read in {where} and not in {doc}" for name, where in sorted(read.items()) if name not in documented]
    problems += [f"{doc} names `{name}`, which no code reads" for name in sorted(set(NAMED.findall(text_of_document))) if name not in known]
    return problems, read


def main():
    if "--mge" in sys.argv[1:]:
        # `--mge-dir DIR`: another MGE XE tree than the render-link sibling (a worktree's hook).
        arguments = sys.argv[1:]
        mge = arguments[arguments.index("--mge-dir") + 1] if "--mge-dir" in arguments else MGE
        problems, read = switches(mge, ("mgeHost64/src", "d3d8/src"), os.path.join(mge, "RENDERER.md"), also_read=("d3d8/cpp",))
        for problem in problems:
            print(problem)
        print(f"{len(read)} switches read in MGE XE's code, {len(problems)} problems")
        return 1 if problems else 0
    readme = open(README, encoding="utf-8").read()
    problems, read = switches(REPO, ("src", "host/src", "d3d8/src", "tools", "mod"), README)
    for path in PATH.findall(readme):
        if path.startswith(NOT_IN_REPO) or "<" in path:
            continue
        if not os.path.exists(os.path.join(REPO, path)):
            problems.append(f"README.md names `{path}`, which does not exist")
    tools = os.path.join(REPO, "tools")
    listed = open(os.path.join(tools, "README.md"), encoding="utf-8").read()
    for name in sorted(os.listdir(tools)):
        if name.endswith(".py") and f"`{name}`" not in listed:
            problems.append(f"tools/{name} has no row in tools/README.md")
    for problem in problems:
        print(problem)
    print(f"{len(read)} switches read in the code, {len(problems)} problems")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
