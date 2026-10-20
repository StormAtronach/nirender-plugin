"""Checks README.md against the code.

    python tools/check_readme.py

1. Every `NIRENDER_*` name that the code reads from the environment (the plugin, the host
   crate, the d3d8 crate, the tools and the Lua mod) is in README.md.
2. Every path that README.md names in back-quotes and that looks like a file or a folder of
   the repo exists.

The exit code is 0 when both checks pass.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import REPO  # noqa: E402

README = os.path.join(REPO, "README.md")
# Where the code reads the environment: the function and the name on one line.
READS = re.compile(r"(?:env::var|var_os|getenv|environ)[^\n]*?\b(NIRENDER_[A-Z0-9_]+)")
NAME = re.compile(r"\bNIRENDER_[A-Z0-9_]+\b")
PATH = re.compile(r"`([A-Za-z0-9_./-]+/[A-Za-z0-9_./-]*|[A-Za-z0-9_-]+\.(?:md|py|rs|lua|toml|h|nircap))`")
# README names a file of a run or a build, not of the repo.
NOT_IN_REPO = ("results/", "target/", "tools/out", "MWSE/", "Data Files", "nirender-<", "<")


def code_files():
    for folder in ("src", "host/src", "d3d8/src", "tools", "mod"):
        for root, _, names in os.walk(os.path.join(REPO, folder)):
            for name in names:
                if name.endswith((".rs", ".py", ".lua")):
                    yield os.path.join(root, name)


def main():
    readme = open(README, encoding="utf-8").read()
    documented = set(NAME.findall(readme))
    read = {}
    for path in code_files():
        text = open(path, encoding="utf-8", errors="replace").read()
        for name in READS.findall(text):
            read.setdefault(name, os.path.relpath(path, REPO).replace("\\", "/"))
    problems = [f"{name} is read in {where} and not in README.md" for name, where in sorted(read.items()) if name not in documented]
    for path in PATH.findall(readme):
        if path.startswith(NOT_IN_REPO) or "<" in path:
            continue
        if not os.path.exists(os.path.join(REPO, path)):
            problems.append(f"README.md names `{path}`, which does not exist")
    for problem in problems:
        print(problem)
    print(f"{len(read)} switches read in the code, {len(problems)} problems")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
