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
- `replay.py --self-test`: the run folder of a replay (D530): its name, the copy of mgeXE.toml, the
  refusal of a picture inside an instance or the game folder, and what stays after a run (no
  helper starts).
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
with a line; with `--strict` (release.py and the merge step) such a check is NOT RUN, since a
release and a merge round need every check. The exit code is 1 when a check fails (its output
is printed), else `common.NOT_RUN_EXIT` (5) when a check is NOT RUN, else 0. `tools/docs.py`
compiles, so it is not here: release.py runs it (only its `--self-test` is here). `tools/install_hooks.py` writes the hooks.

    python -I tools/checks.py --self-test   # the places of the docs and MWSE
"""
import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common  # noqa: E402
import mwse_tree  # noqa: E402

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
# The IDA export of the layouts check: the main checkout's (this tree when it has no main checkout).
MAIN_EXPORT = Path(common.main_checkout() or PLUGIN, "tools", "out", "structs.json")
# How to make what a check needs, for its NOT RUN line.
HOW_TO_MAKE = {
    "sharedse layouts": "make the export in IDA (Morrowind2.i64) with tools/ports/ida_export_inventory.py of the main checkout",
}

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
    "replay run folder": (PLUGIN / "tools" / "replay.py", PLUGIN, ["--self-test"]),
    "mge contract": (PLUGIN / "tools" / "check_contract.py", REPOS / "MGE-XE-G7-render-link", ["--mge"]),
    # The version handshake: one home for each format number and each layout constant of the
    # stream.
    "handshake": (PLUGIN / "tools" / "check_handshake.py", PLUGIN),
    "handshake search": (PLUGIN / "tools" / "check_handshake.py", PLUGIN, ["--self-test"]),
    "mge handshake": (PLUGIN / "tools" / "check_handshake.py", REPOS / "MGE-XE-G7-render-link", ["--mge"]),
    # No new absolute clip product (a matrix times a world position) in the helper's WGSL: W49.
    "clip products": (PLUGIN / "tools" / "check_clip_products.py", REPOS / "MGE-XE-G7-render-link", ["--mge", str(REPOS / "MGE-XE-G7-render-link")]),
    "clip products parsing": (PLUGIN / "tools" / "check_clip_products.py", PLUGIN, ["--self-test"]),
    # The three MWSE checks read the MWSE commit that the test installs run, else the sibling MWSE with a FALLBACK line
    # (mwse_tree.py, W497).
    "mwse native": (PLUGIN / "tools" / "check_mwse_native.py", MWSE),
    "mwse native parsing": (PLUGIN / "tools" / "check_mwse_native.py", PLUGIN, ["--self-test"]),
    "mwse tree": (PLUGIN / "tools" / "mwse_tree.py", PLUGIN, ["--self-test"]),
    # A vtable slot of MWSE's own NI classes filled with a member of another name: W93.
    "mwse vtable fills": (PLUGIN / "tools" / "check_mwse_vtable_fills.py", MWSE),
    "mwse vtable fills parsing": (PLUGIN / "tools" / "check_mwse_vtable_fills.py", PLUGIN, ["--self-test"]),
    "module headers": (STATE / "build_module_map.py", STATE, ["--plugin", str(PLUGIN)]),
    "skill names": (STATE / "check_names.py", STATE, ["--plugin", str(PLUGIN)]),
    # No new hand-written engine offset (src/gen/layouts.rs has them), and each one moved there
    # keeps its value.
    "hand offsets": (PLUGIN / "tools" / "hand_offsets.py", PLUGIN, ["--check"]),
    "offset moves": (PLUGIN / "tools" / "check_offset_moves.py", PLUGIN, ["--cached"]),
    # The IDA types of the layouts against MWSE's SharedSE and TES3 layouts, MWSE read as for "mwse native" (W497).
    # It reads the one IDA export of the main checkout (git ignores it): the export is a fact of the IDA database, the
    # same for every tree (W527, the Architect's answer to Q1). The check names its path, md5 and time.
    "sharedse layouts": (PLUGIN / "tools" / "check_sharedse_layouts.py", (MWSE / "SharedSE", MAIN_EXPORT), ["--export", str(MAIN_EXPORT)]),
    "sharedse layouts parsing": (PLUGIN / "tools" / "check_sharedse_layouts.py", PLUGIN, ["--self-test"]),
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
    # The choice of crates of the doc build (W510): the helper skipped with a line without a render-link tree beside.
    "doc crates": (PLUGIN / "tools" / "docs.py", PLUGIN, ["--self-test"]),
    # The adapter record of the replay gate.
    "gate": (PLUGIN / "tools" / "validate_retained.py", PLUGIN, ["--self-test"]),
    # The tour's replays: a planted refusal of the slot check, and a planted fault of the stage shaders (W523).
    "tour": (PLUGIN / "tools" / "validate_tour.py", PLUGIN, ["--self-test"]),
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
    # The self-tests that no check ran before W527 (W524 F-G-20 C33): a test that no stage runs cannot fail a round.
    "lua checks": (PLUGIN / "tools" / "lua_checks.py", PLUGIN, ["--self-test"]),
    "build sets": (PLUGIN / "tools" / "build_set.py", PLUGIN, ["--self-test"]),
    "sets": (PLUGIN / "tools" / "sets.py", PLUGIN, ["--self-test"]),
    "mwse sites parsing": (PLUGIN / "tools" / "check_mwse_sites.py", PLUGIN, ["--self-test"]),
    "bc7": (PLUGIN / "tools" / "bc7.py", PLUGIN, ["--self-test"]),
    "frame census": (PLUGIN / "tools" / "frame_census.py", PLUGIN, ["--self-test"]),
}
# The tools with a --self-test that no check runs, each with the reason. A new tool with a --self-test is a check, or a
# line here (sibling of self_tests_without_check).
SELF_TESTS_NOT_CHECKED = {
    "symbols.py": "about 11 s and symstore.exe: too slow for the hook of each commit",
}
LISTS = {
    "plugin": ["readme", "contract", "contract parsing", "mge contract", "stream tags", "stream tags parsing", "capture A/B parsing", "replay run folder", "handshake", "handshake search", "clip products parsing", "mwse native", "mwse native parsing", "mwse tree", "mwse vtable fills", "mwse vtable fills parsing", "module headers", "skill names", "hand offsets", "offset moves", "sharedse layouts", "sharedse layouts parsing", "notices", "release", "local places", "runner", "the close of a game", "the play set", "the merge step", "the instance A/B", "clippy arguments", "doc crates", "gate", "tour", "stream rule", "clean worktrees", "clean results", "hooks", "check places", "fuzz", "format", "format self-test", "counted objects", "counted objects self-test", "plain guards self-test", "lua checks", "build sets", "sets", "mwse sites parsing", "bc7", "frame census"],
    "mge": ["mge readme", "mge handshake", "mge contract", "clip products", "mwse native", "module headers", "skill names", "mge format", "mge counted objects", "mge plain guards"],
}
LISTS["all"] = list(dict.fromkeys(LISTS["plugin"] + LISTS["mge"]))


# The option that gives each check of the MGE XE tree another tree than the render-link sibling
# (`checks.py mge --mge DIR`: the pre-commit hook of a worktree).
MGE_OPTION = {"mge readme": "--mge-dir", "mge contract": "--mge", "mge handshake": "--mge", "clip products": "--mge", "mwse native": "--mge", "module headers": "--mge", "skill names": "--mge", "mge format": "--mge", "mge counted objects": "--mge", "mge plain guards": "--mge"}


def self_tests_without_check(tools, checks, exempt):
    """The scripts of the folder `tools` and its subfolders, by their path in it, that take --self-test, that no check of
    `checks` runs with it, and that `exempt` does not name."""
    run = {Path(script).resolve() for script, _, *rest in checks.values() if rest and "--self-test" in rest[0]}
    found = []
    for path in sorted(Path(tools).glob("*.py")) + sorted(Path(tools).glob("*/*.py")):
        name = path.relative_to(tools).as_posix()
        if name not in exempt and path.resolve() not in run and "--self-test" in path.read_text(encoding="utf-8", errors="replace"):
            found.append(name)
    return found


# The checks that compare the tree with a commit (the index, or HEAD), and that take the base of a merge round with
# `checks.py --base COMMIT` (merge_step): the round's tree is at the merge, so its HEAD and index hold no change (W527,
# W524 F-G-6). "offset moves" runs in the plugin round, "mge handshake" in the render-link round.
BASE_OPTION = ("offset moves", "mge handshake")
# The checks that read MWSE through mwse_tree.resolve: the installs' commit, else the sibling checkout with a FALLBACK line.
# With --strict they are NOT RUN when mwse_tree.installs_source gives no commit.
MWSE_CHECKS = ("mwse native", "mwse vtable fills", "sharedse layouts")


def sibling_checks_without_option(checks, options):
    """The checks of `checks` that read the render-link sibling and have no entry in `options`: with `--mge DIR` they would
    still read the sibling, not the tree under test (W524 F-G-21)."""
    return [name for name, (_, needs, *_) in checks.items() if needs == REPOS / "MGE-XE-G7-render-link" and name not in options]


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
    # W527 (Q1): the layouts check reads the main checkout's export, in a worktree too, and names it.
    assert MAIN_EXPORT in CHECKS["sharedse layouts"][1] and CHECKS["sharedse layouts"][2] == ["--export", str(MAIN_EXPORT)]
    assert MAIN_EXPORT.parts[-3:] == ("tools", "out", "structs.json")

    # W497: the MWSE checks pick their MWSE themselves (no path from here), and the summary names a fallback.
    for name in ("mwse native", "mwse vtable fills", "sharedse layouts"):
        assert not any(argument.startswith("--mwse") for argument in (CHECKS[name][2] if len(CHECKS[name]) > 2 else [])), \
            "%s gets no MWSE path from checks.py" % name
    assert summary("plugin", ["a"], [], [], []) == "checks (plugin): 1 passed"
    line = summary("plugin", ["a", "mwse tree"], [], ["mwse native"], ["mwse native"])
    assert line == ("checks (plugin): 2 passed, MWSE FALLBACK in 1: mwse native (the sibling MWSE was read, not the installs' "
                    "commit), 1 FAILED: mwse native. Of these, mwse native compared the sibling MWSE: a difference there can be "
                    "the sibling's branch. Fix the drift, or commit with --no-verify."), line
    assert "Of these" not in summary("plugin", [], [], ["readme"], ["mwse native"]), "a fallback that passed is no cause"
    self_test_strict()
    # W527 (W524 F-G-21): with --mge DIR every check of the render-link sibling reads DIR. The null: before W527 the
    # handshake check of MGE XE had no entry, so a merge round of render-link compared the sibling's mgeversion.h.
    assert sibling_checks_without_option(CHECKS, MGE_OPTION) == [], sibling_checks_without_option(CHECKS, MGE_OPTION)
    without = {name: option for name, option in MGE_OPTION.items() if name != "mge handshake"}
    assert sibling_checks_without_option(CHECKS, without) == ["mge handshake"]
    # W527 (W524 F-G-6): with --base the checks that read the index or HEAD read the merge's base. The null: without
    # --base the offset check reads the index (--cached), which holds no change in a round's tree.
    assert check_command("offset moves")[2] == ["--cached"]
    assert check_command("offset moves", base="abc")[2] == ["--base", "abc"]
    assert check_command("mge handshake", "D", "abc")[2] == ["--mge", "--mge", "D", "--base", "abc"]
    assert check_command("readme", "D", "abc")[2] == [], "a check with no base option gets none"
    assert all(name in CHECKS for name in BASE_OPTION)
    # W527 (W524 F-G-20 C33): every self-test of the tools is a check, or a line of SELF_TESTS_NOT_CHECKED. The plant: a
    # new tool with a --self-test in a scratch folder. The null: before W527, eight tools of this folder were named.
    unrun = self_tests_without_check(PLUGIN / "tools", CHECKS, SELF_TESTS_NOT_CHECKED)
    assert unrun == [], "self-tests that no check runs: %s" % unrun
    with tempfile.TemporaryDirectory() as folder:
        with open(os.path.join(folder, "new_tool.py"), "w", encoding="utf-8") as file:
            file.write('if "--self-test" in sys.argv:\n    pass\n')
        assert self_tests_without_check(folder, CHECKS, SELF_TESTS_NOT_CHECKED) == ["new_tool.py"]
    print("checks self-test: ok")
    return 0


def self_test_strict():
    """W527 (W524 F-G-5): with --strict a check that cannot run is NOT RUN, and checks.py exits with NOT_RUN_EXIT, which
    fails a merge round. The plant: a list of one check whose script is not there. The null: without --strict the same
    list is skipped, and checks.py exits with 0, so a merge round passed on a check that did not run."""
    saved = CHECKS.copy(), LISTS.copy()
    try:
        CHECKS["planted"] = (PLUGIN / "tools" / "no_such_check.py", PLUGIN)
        LISTS["planted"] = ["planted"]
        assert main(["planted", "--strict"]) == common.NOT_RUN_EXIT
        assert main(["planted"]) == 0
        # A failure stays a failure (exit 1) beside a check that did not run.
        CHECKS["fails"] = (PLUGIN / "tools" / "checks.py", PLUGIN, ["--no-such-option"])
        LISTS["planted"] = ["planted", "fails"]
        assert main(["planted", "--strict"]) == 1
    finally:
        CHECKS.clear(), CHECKS.update(saved[0]), LISTS.clear(), LISTS.update(saved[1])
    # W527 (W524 F-G-15): with --strict the MWSE checks are NOT RUN while the installs name no single MWSE commit. The
    # plant: installs_source gives two commits. The null: without --strict the same check runs (and would fall back).
    global MWSE_CHECKS
    saved_mwse = MWSE_CHECKS, mwse_tree.installs_source
    try:
        CHECKS["planted mwse"] = (PLUGIN / "tools" / "bc7.py", PLUGIN, ["--self-test"])
        LISTS["planted"] = ["planted mwse"]
        MWSE_CHECKS = ("planted mwse",)
        mwse_tree.installs_source = lambda *args, **kwargs: (None, "no single MWSE record in the installs: A 1111, B 2222")
        assert main(["planted", "--strict"]) == common.NOT_RUN_EXIT
        assert main(["planted"]) == 0
        mwse_tree.installs_source = lambda *args, **kwargs: ("1111", None)
        assert main(["planted", "--strict"]) == 0, "one commit in the installs: the check runs"
    finally:
        MWSE_CHECKS, mwse_tree.installs_source = saved_mwse
        CHECKS.pop("planted mwse", None), LISTS.pop("planted", None)
    assert all(name in CHECKS for name in MWSE_CHECKS)
    line = summary("plugin", ["a"], [], [], [], ["b", "c"])
    assert line == "checks (plugin): 1 passed, 2 NOT RUN (--strict; not on this machine: b, c)", line


def summary(which, passed, skipped, failed, fell_back, not_run=()):
    """The last line of a run: the counts, the checks that read the sibling MWSE (W497), the checks that did not run, and
    the failures."""
    line = "checks (%s): %d passed" % (which, len(passed))
    if skipped:
        line += ", %d skipped (not on this machine: %s)" % (len(skipped), ", ".join(skipped))
    if not_run:
        line += ", %d NOT RUN (--strict; not on this machine: %s)" % (len(not_run), ", ".join(not_run))
    if fell_back:
        line += ", MWSE FALLBACK in %d: %s (the sibling MWSE was read, not the installs' commit)" % (len(fell_back), ", ".join(fell_back))
    if failed:
        line += ", %d FAILED: %s." % (len(failed), ", ".join(failed))
        both = [name for name in failed if name in fell_back]
        if both:
            line += " Of these, %s compared the sibling MWSE: a difference there can be the sibling's branch." % ", ".join(both)
        line += " Fix the drift, or commit with --no-verify."
    return line


def check_command(name, mge=None, base=None):
    """(script, needs, arguments) of the check `name`: with `mge`, the MGE XE tree of its option (MGE_OPTION); with `base`,
    the base of a merge round in place of the index or HEAD (BASE_OPTION)."""
    script, needs, *rest = CHECKS[name]
    args = list(rest[0]) if rest else []
    if mge and name in MGE_OPTION:
        args += [MGE_OPTION[name], mge]
        if needs == REPOS / "MGE-XE-G7-render-link":
            needs = Path(mge)
    if base and name in BASE_OPTION:
        args = [argument for argument in args if argument != "--cached"] + ["--base", base]
    return script, needs, args


USAGE = "usage: checks.py plugin|mge|all [--strict] [--mge DIR] [--base COMMIT]"


def main(argv):
    if argv[:1] == ["--self-test"]:
        return self_test()
    # `--strict`: a check that would be skipped is NOT RUN (a release and a merge round need every check).
    strict = "--strict" in argv
    values = {}
    for option in ("--mge", "--base"):
        if option in argv:
            if argv.index(option) + 1 >= len(argv):
                raise SystemExit("%s: %s needs a value" % (USAGE, option))
            values[option] = argv[argv.index(option) + 1]
    mge, base = values.get("--mge"), values.get("--base")
    names = [argument for index, argument in enumerate(argv)
             if argument not in ("--strict", "--mge", "--base") and (index == 0 or argv[index - 1] not in ("--mge", "--base"))]
    which = names[0] if names else "all"
    if which not in LISTS:
        raise SystemExit(USAGE)
    failed, passed, skipped, fell_back, not_run = [], [], [], [], []
    mwse_problem = None
    if strict and any(name in MWSE_CHECKS for name in LISTS[which]):
        mwse_problem = mwse_tree.installs_source()[1]
    for name in LISTS[which]:
        script, needs, args = check_command(name, mge, base)
        if mwse_problem and name in MWSE_CHECKS:
            # A check of another MWSE than the installs' judges nothing about them (W527, W524 F-G-15).
            not_run.append(name)
            print("== NOT RUN: the check '%s' (--strict): it would read the sibling MWSE: %s; %s"
                  % (name, mwse_problem, mwse_tree.DEPLOY_HINT))
            continue
        missing = [path for path in (script, *(needs if isinstance(needs, tuple) else (needs,))) if not path.exists()]
        if missing:
            if strict:
                not_run.append(name)
                how = "; %s" % HOW_TO_MAKE[name] if name in HOW_TO_MAKE else ""
                print("== NOT RUN: the check '%s' (--strict): %s is not on this machine%s" % (name, missing[0], how))
            else:
                skipped.append(name)
            continue
        run = subprocess.run([sys.executable, "-I", str(script), *args], cwd=script.parent, capture_output=True, text=True,
                             encoding="utf-8", errors="replace", env=dict(os.environ, PYTHONIOENCODING="utf-8"))
        # A check that read the sibling MWSE in place of the installs' commit says so in each case (W497).
        for fallback in mwse_tree.fallback_lines(run.stdout):
            print(fallback)
        if mwse_tree.fallback_lines(run.stdout):
            fell_back.append(name)
        if run.returncode == 0:
            passed.append(name)
        else:
            failed.append(name)
            print("== the check '%s' failed (%s):" % (name, script.name))
            print((run.stdout + run.stderr).rstrip()[-4000:])
    print(summary(which, passed, skipped, failed, fell_back, not_run))
    # A failure is 1. A check that did not run is NOT_RUN_EXIT: a merge round fails on it (merge_step.run_stage), and
    # release.py reads every code other than 0 as a failure.
    if failed:
        return 1
    return common.NOT_RUN_EXIT if not_run else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
