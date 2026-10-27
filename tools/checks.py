"""Runs the content checks that need no compile, for a commit hook or a release.

    python -I tools/checks.py plugin    # the hook of this repository
    python -I tools/checks.py mge       # the hook of the MGE XE render-link worktree
    python -I tools/checks.py all       # both lists once (release.py)

The checks of each list:

- `check_readme.py`: every switch the code reads is in README.md, and every switch README.md
  names is read; every path README names exists. With `--mge` (the MGE XE list): MGE XE's
  switches against its RENDERER.md.
- `check_contract.py`: `contract/mge_render_link.h` against `src/link.rs`.
- `check_mwse_native.py`: the plugin's and MGE XE's copies of MWSE's native ABI against MWSE's
  header.
- `build_module_map.py` (docs): every module of ours has a `//!` header.
- `check_names.py` (docs): no skill names a switch or a Lua call the code lost.

Each takes well under a second. A check whose repository is not on this machine is skipped
with a line. The exit code is 1 when a check fails; its output is printed. `tools/docs.py`
compiles, so it is not here: release.py runs it. `tools/install_hooks.py` writes the hooks.
"""
import os
import subprocess
import sys
from pathlib import Path

PLUGIN = Path(__file__).resolve().parents[1]
REPOS = PLUGIN.parent
STATE = REPOS / "moreFPS" / "docs" / "Numidium Project" / "scripts" / "state-page"
MWSE_HEADER = REPOS / "MWSE" / "MWSE" / "MWSE" / "MWSENativeApi.h"

CHECKS = {
    "readme": (PLUGIN / "tools" / "check_readme.py", PLUGIN),
    # MGE XE's switches against its RENDERER.md.
    "mge readme": (PLUGIN / "tools" / "check_readme.py", REPOS / "MGE-XE-G7-render-link", ["--mge"]),
    "contract": (PLUGIN / "tools" / "check_contract.py", PLUGIN),
    "mwse native": (PLUGIN / "tools" / "check_mwse_native.py", MWSE_HEADER),
    "module headers": (STATE / "build_module_map.py", STATE),
    "skill names": (STATE / "check_names.py", STATE),
    # No new hand-written engine offset (src/gen/layouts.rs has them), and each one moved there
    # keeps its value.
    "hand offsets": (PLUGIN / "tools" / "hand_offsets.py", PLUGIN, ["--check"]),
    "offset moves": (PLUGIN / "tools" / "check_offset_moves.py", PLUGIN, ["--cached"]),
    # The IDA types of the layouts against MWSE's SharedSE and TES3 layouts.
    "sharedse layouts": (PLUGIN / "tools" / "check_sharedse_layouts.py", REPOS / "MWSE" / "MWSE" / "SharedSE"),
    # The fixture test of the third-party notices writer of the release archive.
    "notices": (PLUGIN / "tools" / "third_party_notices.py", PLUGIN, ["--self-test"]),
    # The tests of the pure parts of the release tool (the untracked-file stop, the tag name,
    # the notes' Tested block).
    "release": (PLUGIN / "tools" / "release.py", PLUGIN, ["--self-test"]),
}
LISTS = {
    "plugin": ["readme", "contract", "mwse native", "module headers", "skill names", "hand offsets", "offset moves", "sharedse layouts", "notices", "release"],
    "mge": ["mge readme", "mwse native", "module headers", "skill names"],
}
LISTS["all"] = list(dict.fromkeys(LISTS["plugin"] + LISTS["mge"]))


def main(argv):
    which = argv[0] if argv else "all"
    if which not in LISTS:
        raise SystemExit("usage: checks.py plugin|mge|all")
    failed, passed, skipped = [], [], []
    for name in LISTS[which]:
        script, needs, *rest = CHECKS[name]
        args = rest[0] if rest else []
        if not script.exists() or not needs.exists():
            skipped.append(name)
            continue
        run = subprocess.run([sys.executable, "-I", str(script), *args], cwd=script.parent, capture_output=True, text=True,
                             encoding="utf-8", errors="replace", env=dict(os.environ, PYTHONIOENCODING="utf-8"))
        if run.returncode == 0:
            passed.append(name)
        else:
            failed.append(name)
            print("== the check '%s' failed (%s):" % (name, script.name))
            print((run.stdout + run.stderr).rstrip()[-4000:])
    line = "checks (%s): %d passed" % (which, len(passed))
    if skipped:
        line += ", %d skipped (not on this machine: %s)" % (len(skipped), ", ".join(skipped))
    if failed:
        line += ", %d FAILED: %s. Fix the drift, or commit with --no-verify." % (len(failed), ", ".join(failed))
    print(line)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
