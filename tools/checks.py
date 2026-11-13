"""Runs the content checks that need no compile, for a commit hook or a release.

    python -I tools/checks.py plugin    # the hook of this repository
    python -I tools/checks.py mge       # the hook of the MGE XE render-link worktree
    python -I tools/checks.py all       # both lists once (release.py)
    python -I tools/checks.py mge --mge DIR    # the mge list against another MGE XE tree (a worktree's hook)

The checks of each list:

- `check_readme.py`: every switch the code reads is in README.md, and every switch README.md
  names is read; every path README names exists. With `--mge` (the MGE XE list): MGE XE's
  switches against its RENDERER.md.
- `check_contract.py`: `contract/mge_render_link.h` against `src/link.rs`; with `--mge` (the MGE XE
  list): also each export of the plugin that d3d8.dll looks up by name against the plugin's
  export of that name.
- `check_stream_tags.py`: the tag numbers of the plugin's writer (`src/capture.rs`) against the
  helper's reader (`host/src/tag.rs`), and the fixed length of each record against the reader's
  `least_length`.
- `capture_ab.py --self-test`: the compare of two captures by their fixed-length tags and the
  check of a replay's report (the tool itself needs two captures and replays).
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
- `check_format.py`: the Rust code is formatted as `cargo fmt` writes it, in each Rust root of
  this repository; with `--mge` (the MGE XE list): in the MGE XE workspace.
- `check_counted_objects.py`: the host crate makes its GPU objects through the counted calls of
  `host/src/objects.rs` (no `create_*` call of the device or a texture elsewhere, D288); with
  `--mge` (the MGE XE list): the helper too.
- `check_plain_guards.py` (the MGE XE list): each MWScript instruction of `d3d8/cpp/mwse` and each
  macro function has the `Session::plain()` guard of the plain game in its place (W462).

The docs scripts come from the docs checkout beside the main checkout, or from the tree that
the local.json key "docs" names (`common.docs_place`), never from beside a worktree. MWSE is
the sibling of this tree, else the sibling of its main checkout (`common.sibling`). The docs
checks read the code of this tree (`--plugin`), not the code of the checkout beside the docs.

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
STATE_PARTS = ("docs", "Numidium Project", "scripts", "state-page")
MWSE_PARTS = ("MWSE", "MWSE")


def places(siblings, main, docs=None):
    """The folder of the docs scripts and of the MWSE repository. The docs scripts come from the docs checkout beside
    the main checkout `main`, or from `docs` (the key "docs"; `common.find_docs`); MWSE from beside `siblings`, else
    beside the main checkout (`common.find_sibling`)."""
    return Path(common.find_docs(main, docs), *STATE_PARTS), Path(common.find_sibling(siblings, main, MWSE_PARTS))


STATE, MWSE = Path(common.docs_place(*STATE_PARTS)), places(str(REPOS), common.main_checkout())[1]
MWSE_HEADER = MWSE / "MWSE" / "MWSENativeApi.h"

CHECKS = {
    "readme": (PLUGIN / "tools" / "check_readme.py", PLUGIN),
    # MGE XE's switches against its RENDERER.md.
    "mge readme": (PLUGIN / "tools" / "check_readme.py", REPOS / "MGE-XE-G7-render-link", ["--mge"]),
    "contract": (PLUGIN / "tools" / "check_contract.py", PLUGIN),
    "contract parsing": (PLUGIN / "tools" / "check_contract.py", PLUGIN, ["--self-test"]),
    # The tags and the fixed record lengths of the plugin's writer against the helper's reader.
    "stream tags": (PLUGIN / "tools" / "check_stream_tags.py", PLUGIN),
    "stream tags parsing": (PLUGIN / "tools" / "check_stream_tags.py", PLUGIN, ["--self-test"]),
    "capture A/B parsing": (PLUGIN / "tools" / "capture_ab.py", PLUGIN, ["--self-test"]),
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
    # cargo fmt --check in each Rust root of this repository, and in the MGE XE workspace.
    "format": (PLUGIN / "tools" / "check_format.py", PLUGIN),
    "mge format": (PLUGIN / "tools" / "check_format.py", REPOS / "MGE-XE-G7-render-link", ["--mge", str(REPOS / "MGE-XE-G7-render-link")]),
    "format self-test": (PLUGIN / "tools" / "check_format.py", PLUGIN, ["--self-test"]),
    # The GPU objects are made through the counted calls of host/src/objects.rs (D288), in the
    # crate and in the helper.
    "counted objects": (PLUGIN / "tools" / "check_counted_objects.py", PLUGIN),
    "mge counted objects": (PLUGIN / "tools" / "check_counted_objects.py", REPOS / "MGE-XE-G7-render-link", ["--mge", str(REPOS / "MGE-XE-G7-render-link")]),
    # MGE XE's MWScript instructions and macro functions do nothing in the plain game (W462).
    "mge plain guards": (PLUGIN / "tools" / "check_plain_guards.py", REPOS / "MGE-XE-G7-render-link", ["--mge", str(REPOS / "MGE-XE-G7-render-link")]),
    "plain guards self-test": (PLUGIN / "tools" / "check_plain_guards.py", PLUGIN, ["--self-test"]),
    "counted objects self-test": (PLUGIN / "tools" / "check_counted_objects.py", PLUGIN, ["--self-test"]),
    # The places of the docs and MWSE for a tree with no siblings.
    "check places": (PLUGIN / "tools" / "checks.py", PLUGIN, ["--self-test"]),
    # The pure parts of the corrupted-capture tool (the record walk, the mutations, the verdicts).
    "fuzz": (PLUGIN / "tools" / "fuzz_capture.py", PLUGIN, ["--self-test"]),
}
LISTS = {
    "plugin": ["readme", "contract", "contract parsing", "mge contract", "stream tags", "stream tags parsing", "capture A/B parsing", "handshake", "handshake search", "clip products parsing", "mwse native", "mwse vtable fills", "mwse vtable fills parsing", "module headers", "skill names", "hand offsets", "offset moves", "sharedse layouts", "notices", "release", "local places", "runner", "the close of a game", "the play set", "the merge step", "the instance A/B", "clippy arguments", "gate", "stream rule", "clean worktrees", "clean results", "hooks", "check places", "fuzz", "format", "format self-test", "counted objects", "counted objects self-test", "plain guards self-test"],
    "mge": ["mge readme", "mge handshake", "mge contract", "clip products", "mwse native", "module headers", "skill names", "mge format", "mge counted objects", "mge plain guards"],
}
LISTS["all"] = list(dict.fromkeys(LISTS["plugin"] + LISTS["mge"]))


# The option that gives each check of the MGE XE tree another tree than the render-link sibling
# (`checks.py mge --mge DIR`: the pre-commit hook of a worktree).
MGE_OPTION = {"mge readme": "--mge-dir", "mge contract": "--mge", "clip products": "--mge", "mwse native": "--mge", "module headers": "--mge", "skill names": "--mge", "mge format": "--mge", "mge counted objects": "--mge", "mge plain guards": "--mge"}


def self_test():
    """The places of the docs and MWSE for a tree in a group of worktrees in C:/tmp: the docs scripts always those of
    the docs checkout beside the main checkout, also when a stale docs tree lies beside the tree (W355); MWSE beside the
    tree, else beside the main checkout. With none anywhere, the checks are skipped."""
    import tempfile

    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp:
        group = os.path.join(temp, "group")
        repos = os.path.join(temp, "repos")
        main = os.path.join(repos, "nirender-plugin")
        os.makedirs(os.path.join(group, "nirender-plugin"))
        os.makedirs(main)
        main_state = Path(repos, common.DOCS_REPO, *STATE_PARTS)
        assert places(group, main) == (main_state, Path(group, *MWSE_PARTS)), "nothing anywhere: places that do not exist"

        os.makedirs(main_state)
        os.makedirs(os.path.join(repos, *MWSE_PARTS))
        assert places(group, main) == (main_state, Path(repos, *MWSE_PARTS)), "no sibling: the main checkout's"

        # The null of W355: a stale docs tree named moreFPS beside the tree is not taken; MWSE beside it is.
        stale = Path(group, common.DOCS_REPO)
        os.makedirs(stale / Path(*STATE_PARTS))
        os.makedirs(os.path.join(group, *MWSE_PARTS))
        assert places(group, main) == (main_state, Path(group, *MWSE_PARTS)), places(group, main)
        assert places(group, main, str(stale))[0] == stale / Path(*STATE_PARTS), "the key names a docs tree on purpose"

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
