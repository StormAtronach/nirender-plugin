"""Runs the content checks that need no compile, for a commit hook or a release.

    python -I tools/checks.py plugin    # the hook of this repository
    python -I tools/checks.py mge       # the hook of the MGE XE render-link worktree
    python -I tools/checks.py all       # both lists once (release.py)
    python -I tools/checks.py mge --mge DIR    # the mge list against another MGE XE tree (a worktree's hook)

The checks of each list:

- `check_readme.py`: every switch the code reads is in README.md, and every switch README.md
  names is read; every path README names exists. With `--mge` (the MGE XE list): MGE XE's
  switches against its RENDERER.md.
- `check_contract.py`: `contract/mge_render_link.h` against `src/link.rs`.
- `check_mwse_native.py`: the plugin's and MGE XE's copies of MWSE's native ABI against MWSE's
  header.
- `check_mwse_vtable_fills.py`: each vtable slot that MWSE fills from a member function gets the
  member of its own name (W93).
- `check_handshake.py`: the host crate reads the plugin's `src/stream_format.rs`, and no
  constant of it has a second definition in the two crates; with `--mge` (the MGE XE list):
  MGE_EXTENSION_FORMAT of mgeversion.h equals the helper's EXTENSION_FORMAT, and mgeHost64 has
  no copy of a constant of `src/stream_format.rs`.
- `check_clip_products.py` (the MGE XE list): the helper's WGSL has no new product of a matrix
  with a world position (`matrix * vec4f(world, 1.0)`); the known products are on an allow-list.
  Use `clip_with` (W49). Also: each vertex stage ends in `clip_of` or `clip_with`, and none reads
  `u.view_projection`.
- `build_module_map.py` (docs): every module of ours has a `//!` header.
- `check_names.py` (docs): no skill names a switch or a Lua call the code lost.

The docs and MWSE are the siblings of this tree, else the siblings of its main checkout
(`common.sibling`): a group of worktrees in C:/tmp has neither beside it. The docs checks read
the code of this tree (`--plugin`), not the code of the checkout beside the docs.

Each takes well under a second. A check whose repository is not on this machine is skipped
with a line; with `--strict` (release.py) such a check is a failure, since a release needs
every check. The exit code is 1 when a check fails; its output is printed. `tools/docs.py`
compiles, so it is not here: release.py runs it. `tools/install_hooks.py` writes the hooks.

    python -I tools/checks.py --self-test   # the places of the docs and MWSE
"""
import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common  # noqa: E402

PLUGIN = Path(__file__).resolve().parents[1]
REPOS = PLUGIN.parent
STATE_PARTS = ("moreFPS", "docs", "Numidium Project", "scripts", "state-page")
MWSE_PARTS = ("MWSE", "MWSE")


def places(siblings, main):
    """The folder of the docs scripts and of the MWSE repository: beside `siblings`, else beside the main checkout
    `main` (`common.find_sibling`)."""
    return Path(common.find_sibling(siblings, main, STATE_PARTS)), Path(common.find_sibling(siblings, main, MWSE_PARTS))


STATE, MWSE = places(str(REPOS), common.main_checkout())
MWSE_HEADER = MWSE / "MWSE" / "MWSENativeApi.h"

CHECKS = {
    "readme": (PLUGIN / "tools" / "check_readme.py", PLUGIN),
    # MGE XE's switches against its RENDERER.md.
    "mge readme": (PLUGIN / "tools" / "check_readme.py", REPOS / "MGE-XE-G7-render-link", ["--mge"]),
    "contract": (PLUGIN / "tools" / "check_contract.py", PLUGIN),
    "contract parsing": (PLUGIN / "tools" / "check_contract.py", PLUGIN, ["--self-test"]),
    "mge contract": (PLUGIN / "tools" / "check_contract.py", REPOS / "MGE-XE-G7-render-link", ["--mge"]),
    # The version handshake: one home for each format number and each layout constant of the
    # stream.
    "handshake": (PLUGIN / "tools" / "check_handshake.py", PLUGIN),
    "handshake search": (PLUGIN / "tools" / "check_handshake.py", PLUGIN, ["--self-test"]),
    "mge handshake": (PLUGIN / "tools" / "check_handshake.py", REPOS / "MGE-XE-G7-render-link", ["--mge"]),
    # No new absolute clip product (a matrix times a world position) in the helper's WGSL: W49.
    "clip products": (PLUGIN / "tools" / "check_clip_products.py", REPOS / "MGE-XE-G7-render-link", ["--mge", str(REPOS / "MGE-XE-G7-render-link")]),
    "clip products parsing": (PLUGIN / "tools" / "check_clip_products.py", PLUGIN, ["--self-test"]),
    "mwse native": (PLUGIN / "tools" / "check_mwse_native.py", MWSE_HEADER),
    # A vtable slot of MWSE's own NI classes filled with a member of another name: W93.
    "mwse vtable fills": (PLUGIN / "tools" / "check_mwse_vtable_fills.py", MWSE, ["--mwse", str(MWSE)]),
    "mwse vtable fills parsing": (PLUGIN / "tools" / "check_mwse_vtable_fills.py", PLUGIN, ["--self-test"]),
    "module headers": (STATE / "build_module_map.py", STATE, ["--plugin", str(PLUGIN)]),
    "skill names": (STATE / "check_names.py", STATE, ["--plugin", str(PLUGIN)]),
    # No new hand-written engine offset (src/gen/layouts.rs has them), and each one moved there
    # keeps its value.
    "hand offsets": (PLUGIN / "tools" / "hand_offsets.py", PLUGIN, ["--check"]),
    "offset moves": (PLUGIN / "tools" / "check_offset_moves.py", PLUGIN, ["--cached"]),
    # The IDA types of the layouts against MWSE's SharedSE and TES3 layouts.
    # It also needs this tree's IDA export, which git ignores: a new worktree has none, and the export of another
    # checkout is not this tree's.
    "sharedse layouts": (PLUGIN / "tools" / "check_sharedse_layouts.py", (MWSE / "SharedSE", PLUGIN / "tools" / "out" / "structs.json")),
    # The fixture test of the third-party notices writer of the release archive.
    "notices": (PLUGIN / "tools" / "third_party_notices.py", PLUGIN, ["--self-test"]),
    # The tests of the pure parts of the release tool (the untracked-file stop, the tag name,
    # the notes' Tested block).
    "release": (PLUGIN / "tools" / "release.py", PLUGIN, ["--self-test"]),
    # The local.json and the siblings of a worktree (common.find_local, find_sibling, checkout_of).
    "local places": (PLUGIN / "tools" / "common.py", PLUGIN, ["--self-test"]),
    # The argument handling of the runner (--plugin-dll, --log).
    "runner": (PLUGIN / "tools" / "run_own_device.py", PLUGIN, ["--self-test"]),
    "the close of a game": (PLUGIN / "tools" / "play.py", PLUGIN, ["--self-test"]),
    "the play set": (PLUGIN / "tools" / "make_play_set.py", PLUGIN, ["--self-test"]),
    "the merge step": (PLUGIN / "tools" / "merge_step.py", PLUGIN, ["--self-test"]),
    # The scan of NIFs for the exhibits that depend on the order of the draws (small NIFs made by the test).
    "the NIF order scan": (PLUGIN / "tools" / "nif_order_scan.py", PLUGIN, ["--self-test"]),
    # The deploy of the resting build into the test installs (scratch installs; the real ones are not touched).
    "the resting deploy": (PLUGIN / "tools" / "deploy_rest.py", PLUGIN, ["--self-test"]),
    "the instance A/B": (PLUGIN / "tools" / "instance_ab.py", PLUGIN, ["--self-test"]),
    "clippy arguments": (PLUGIN / "tools" / "clippy_baseline.py", PLUGIN, ["--self-test"]),
    # The adapter record of the replay gate.
    "gate": (PLUGIN / "tools" / "validate_retained.py", PLUGIN, ["--self-test"]),
    # The stream rule of the gate: the planted fault of risk 76 and the passes.
    "stream rule": (PLUGIN / "tools" / "stream_rule.py", PLUGIN, ["--self-test"]),
    "clean worktrees": (PLUGIN / "tools" / "clean_merged_worktrees.py", PLUGIN, ["--self-test"]),
    "clean results": (PLUGIN / "tools" / "clean_results.py", PLUGIN, ["--self-test"]),
    # The hook texts: the checks of the tree the commit is in.
    "hooks": (PLUGIN / "tools" / "install_hooks.py", PLUGIN, ["--self-test"]),
    # The places of the docs and MWSE for a tree with no siblings.
    "check places": (PLUGIN / "tools" / "checks.py", PLUGIN, ["--self-test"]),
    # The pure parts of the corrupted-capture tool (the record walk, the mutations, the verdicts).
    "fuzz": (PLUGIN / "tools" / "fuzz_capture.py", PLUGIN, ["--self-test"]),
}
LISTS = {
    "plugin": ["readme", "contract", "contract parsing", "handshake", "handshake search", "clip products parsing", "mwse native", "mwse vtable fills", "mwse vtable fills parsing", "module headers", "skill names", "hand offsets", "offset moves", "sharedse layouts", "notices", "release", "local places", "runner", "the close of a game", "the play set", "the merge step", "the instance A/B", "clippy arguments", "gate", "stream rule", "clean worktrees", "clean results", "hooks", "check places", "fuzz"],
    "mge": ["mge readme", "mge handshake", "mge contract", "clip products", "mwse native", "module headers", "skill names"],
}
LISTS["all"] = list(dict.fromkeys(LISTS["plugin"] + LISTS["mge"]))


# The option that gives each check of the MGE XE tree another tree than the render-link sibling
# (`checks.py mge --mge DIR`: the pre-commit hook of a worktree).
MGE_OPTION = {"mge readme": "--mge-dir", "clip products": "--mge", "mwse native": "--mge", "module headers": "--mge", "skill names": "--mge"}


def self_test():
    """The places of the docs and MWSE for a tree with no siblings (a group of worktrees in C:/tmp): those beside the
    main checkout. With none there either, those beside the tree, so that the checks are skipped."""
    import tempfile

    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp:
        group = os.path.join(temp, "group")
        repos = os.path.join(temp, "repos")
        main = os.path.join(repos, "nirender-plugin")
        os.makedirs(os.path.join(group, "nirender-plugin"))
        os.makedirs(main)
        beside_tree = (Path(group, *STATE_PARTS), Path(group, *MWSE_PARTS))
        assert places(group, main) == beside_tree, "nothing anywhere: the places beside the tree, which do not exist"

        os.makedirs(os.path.join(repos, *STATE_PARTS))
        os.makedirs(os.path.join(repos, *MWSE_PARTS))
        assert places(group, main) == (Path(repos, *STATE_PARTS), Path(repos, *MWSE_PARTS)), "no sibling: the main checkout's"

        os.makedirs(os.path.join(group, *STATE_PARTS))
        assert places(group, main)[0] == beside_tree[0], "a sibling of the tree comes first"

    for name in ("module headers", "skill names"):
        assert CHECKS[name][2] == ["--plugin", str(PLUGIN)], "the docs checks read the code of this tree"
    assert PLUGIN / "tools" / "out" / "structs.json" in CHECKS["sharedse layouts"][1], "the layouts check needs this tree's export"
    print("checks self-test: ok")
    return 0


def main(argv):
    if argv[:1] == ["--self-test"]:
        return self_test()
    # `--strict`: a check that would be skipped is a failure (a release needs every check).
    strict = "--strict" in argv
    if "--mge" in argv and argv.index("--mge") + 1 >= len(argv):
        raise SystemExit("usage: checks.py plugin|mge|all [--strict] [--mge DIR]: --mge needs the folder of the MGE XE tree")
    mge = argv[argv.index("--mge") + 1] if "--mge" in argv else None
    names = [argument for index, argument in enumerate(argv) if argument not in ("--strict", "--mge") and (index == 0 or argv[index - 1] != "--mge")]
    which = names[0] if names else "all"
    if which not in LISTS:
        raise SystemExit("usage: checks.py plugin|mge|all [--strict] [--mge DIR]")
    failed, passed, skipped = [], [], []
    for name in LISTS[which]:
        script, needs, *rest = CHECKS[name]
        args = list(rest[0]) if rest else []
        if mge and name in MGE_OPTION:
            args += [MGE_OPTION[name], mge]
            if needs == REPOS / "MGE-XE-G7-render-link":
                needs = Path(mge)
        missing = [path for path in (script, *(needs if isinstance(needs, tuple) else (needs,))) if not path.exists()]
        if missing:
            if strict:
                failed.append(name)
                print("== the check '%s' cannot run (--strict): %s is not on this machine" % (name, missing[0]))
            else:
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
