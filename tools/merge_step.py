"""The checks of a merge, for the Coordinator, on the merged tree: one PASS or FAIL line for each stage.

    python -I tools/merge_step.py <repo> <commit> --base <tip> [--no-wait] [--dry-run]
    python -I tools/merge_step.py --self-test    # the tests of the pure parts: no build, no game

<repo> is the tree of nirender-plugin or MGE-XE-G7-render-link, at <commit> and with no tracked change. There is no
trial merge: the tool refuses another tree state. It takes the build slot for the whole of its work and releases it. It
waits in the queue of that slot; with --no-wait it stops with exit code 3. --dry-run prints the stages and takes nothing.

<tip> is the tip of the shared branch before the merge. It is required: the parents of <commit> do not tell the change, as
the first parent misses a fast-forward and a branch that merged the shared branch.

The stages of nirender-plugin: the plugin's tests, the host's tests, clippy_baseline.py --check, checks.py plugin. The
stages of render-link: the helper's and mge-config's tests, checks.py mge --mge <repo>. The diff from <tip> to <commit> adds
these:

- a d3d8 build and check_contract.py --mge when the change touches d3d8, mge-config or contract/;
- the replay gate, tools/validate_retained.py baseline with no game, when the change touches the host's source, the
  helper's renderer or the shaders of the assets, the source of the shaders in a build. The path decides: a stream or a
  table change can move a picture too.

All stages run, also after a failure. Exit code 1 on any failure. The output of each stage is in
<plugin>/target/merge-step-<commit>.log. It pushes nothing.

The gate needs what a diff cannot give:

- the two repos as siblings under their own names, as the helper's Cargo.toml names ../../nirender-plugin/host. A
  worktree with another name would build the host of another tree, so the gate refuses it;
- `cargo xtask dist` of the pair, as the gate replays with the helper and the shaders of a build. This is the longest stage;
- the sibling at the commit that the merge goes with: this tool does not know the other repo's tip;
- accepted pictures. A planned new look fails the gate until a baseline is accepted, and that is the owner's choice. The
  accepted pictures also hold the distant land and the mgeXE.toml of their time, from the testing instance.
"""
import argparse
import os
import subprocess
import sys
import tempfile
import time

# The modules of this folder, also under `python -I`.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from make_play_set import BASELINE, WAIT_SECONDS, load_slots, say  # noqa: E402

PLUGIN_NAME = "nirender-plugin"
MGE_NAME = "MGE-XE-G7-render-link"
SLOT_MINUTES = 60
# What a change must touch for the gate, as paths from the root of each repo: a file, or a folder with its slash.
GATE_PATHS = {"plugin": ("host/src/",), "mge": ("mgeHost64/src/renderer.rs", "mgeHost64/src/renderer/", "assets/Data Files/shaders/")}
D3D8_PATHS = {"plugin": ("contract/",), "mge": ("d3d8/", "mge-config/")}


def touches(paths, wanted):
    return any(path == item or (item.endswith("/") and path.startswith(item)) for path in paths for item in wanted)


def repo_kind(folder):
    """"plugin", "mge" or None, by what the tree holds."""
    if os.path.isfile(os.path.join(folder, "host", "src", "shader.wgsl")):
        return "plugin"
    return "mge" if os.path.isdir(os.path.join(folder, "mgeHost64")) else None


def stages_for(kind, repo, paths):
    """The stages of a merge, as (name, folder, command). The folder of the other repo is the sibling."""
    python = sys.executable
    parent = os.path.dirname(repo)
    plugin = repo if kind == "plugin" else os.path.join(parent, PLUGIN_NAME)
    mge = repo if kind == "mge" else os.path.join(parent, MGE_NAME)
    tool = lambda name, *args: [python, "-I", os.path.join("tools", name), *args]  # noqa: E731
    if kind == "plugin":
        stages = [("plugin tests", plugin, ["cargo", "test", "--quiet"]),
                  ("host tests", os.path.join(plugin, "host"), ["cargo", "test", "--quiet"]),
                  ("clippy baseline", plugin, tool("clippy_baseline.py", "--check")),
                  ("checks", plugin, tool("checks.py", "plugin"))]
    else:
        stages = [("helper tests", mge, ["cargo", "test", "--quiet", "-p", "mgeHost64", "-p", "mge-config"]),
                  ("checks", plugin, tool("checks.py", "mge", "--mge", mge))]
    if touches(paths, D3D8_PATHS[kind]):
        stages.append(("d3d8 build", mge, ["cargo", "build", "--quiet", "-p", "d3d8", "--profile", "release-dll"]))
        stages.append(("contract", plugin, tool("check_contract.py", "--mge", mge)))
    if touches(paths, GATE_PATHS[kind]):
        stages.append(("build for the gate", mge, ["cargo", "xtask", "dist"]))
        stages.append(("replay gate", plugin, tool("validate_retained.py", "baseline", "--mge-build", os.path.join(mge, "target", "dist-staging"), "--baseline", BASELINE)))
    return stages


def tree_problem(kind, repo, stages, head, commit, changed):
    """Why this tree cannot take the stages, as a text, or None."""
    if kind is None:
        return "%s is no tree of %s or %s" % (repo, PLUGIN_NAME, MGE_NAME)
    if head != commit:
        return "the tree is at %s, not %s: merge first, as this tool makes no trial merge" % (head[:10], commit[:10])
    if changed:
        return "the tree has tracked changes"
    gate = any(name == "replay gate" for name, _, _ in stages)
    if gate and os.path.basename(repo) != {"plugin": PLUGIN_NAME, "mge": MGE_NAME}[kind]:
        return "the gate needs the tree under its own name beside its sibling: the helper's Cargo.toml names ../../%s/host" % PLUGIN_NAME
    missing = [folder for _, folder, _ in stages if not os.path.isdir(folder)]
    if missing:
        return "no folder %s: the stages need the sibling" % missing[0]
    return None


def base_problem(base, commit, is_ancestor):
    """Why the diff from this base does not tell the change, as a text, or None."""
    if base == commit:
        return "the base is the commit, so the diff is empty and asks for no stage: give the tip before the merge"
    if not is_ancestor:
        return "the base %s is no ancestor of the commit: the diff would also hold the other side's changes" % base[:10]
    return None


def run_stage(name, folder, command, log):
    began = time.time()
    log.write("\n== %s: %s in %s\n" % (name, " ".join(command), folder))
    log.flush()
    env = dict(os.environ, PATH=os.path.join(os.path.expanduser("~"), ".cargo", "bin") + os.pathsep + os.environ["PATH"])
    code = subprocess.run(command, cwd=folder, env=env, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL).returncode
    say("%s %s (%d s)%s" % ("PASS" if code == 0 else "FAIL", name, time.time() - began, "" if code == 0 else ": exit code %d" % code))
    return code == 0


def take_build(slots, what, no_wait):
    """Takes the build slot, and asks again in its queue, so that the place stays. False with --no-wait, when it must wait."""
    os.makedirs(slots.STATE_DIR, exist_ok=True)
    while True:
        try:
            reply = slots.take("build", what, minutes=SLOT_MINUTES)
        except slots.StateBusy as error:
            say("the state of the slots is busy, and I ask again: %s" % error)
            time.sleep(WAIT_SECONDS)
            continue
        say(reply.text.splitlines()[0])
        if reply.code == slots.DONE:
            return True
        if no_wait:
            slots.leave("build", what)
            return False
        time.sleep(WAIT_SECONDS)


def git(repo, *args):
    done = subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True)
    if done.returncode != 0:
        raise SystemExit("git %s in %s: %s" % (" ".join(args), repo, done.stderr.strip()))
    return done.stdout.strip()


def self_test():
    with tempfile.TemporaryDirectory() as root:
        plugin = os.path.join(root, PLUGIN_NAME)
        mge = os.path.join(root, MGE_NAME)
        os.makedirs(os.path.join(plugin, "host", "src"))
        with open(os.path.join(plugin, "host", "src", "shader.wgsl"), "w", encoding="ascii"):
            pass
        os.makedirs(os.path.join(mge, "mgeHost64"))
        assert repo_kind(plugin) == "plugin" and repo_kind(mge) == "mge" and repo_kind(root) is None

        def names(kind, repo, *paths):
            return [name for name, _, _ in stages_for(kind, repo, paths)]

        base = ["plugin tests", "host tests", "clippy baseline", "checks"]
        assert names("plugin", plugin, "tools/x.py", "src/lib.rs") == base
        assert names("plugin", plugin, "host/src/shader.wgsl") == base + ["build for the gate", "replay gate"]
        assert names("plugin", plugin, "host/src/material.rs") == base + ["build for the gate", "replay gate"]
        assert names("plugin", plugin, "host/src/retained.rs") == base + ["build for the gate", "replay gate"]
        assert names("plugin", plugin, "host/src_old/x.rs") == base, "a name that only starts the same ran the gate"
        assert names("plugin", plugin, "contract/mge_render_link.h") == base + ["d3d8 build", "contract"]
        mge_base = ["helper tests", "checks"]
        assert names("mge", mge, "mge-config/src/lib.rs") == mge_base + ["d3d8 build", "contract"]
        assert names("mge", mge, "mgeHost64/src/renderer/water.rs") == mge_base + ["build for the gate", "replay gate"]
        assert names("mge", mge, "mgeHost64/src/renderer.rs") == mge_base + ["build for the gate", "replay gate"]
        assert names("mge", mge, "assets/Data Files/shaders/core/x.fx") == mge_base + ["build for the gate", "replay gate"]
        assert names("mge", mge, "assets/Data Files/other.txt") == mge_base
        assert names("mge", mge, "mgeHost64/src/other.rs") == mge_base
        assert names("mge", mge, "d3d8/cpp/mge/x.cpp") == mge_base + ["d3d8 build", "contract"]

        # Each stage runs in the right repo: the gate replays the build of the sibling, and the checks of the tools' repo.
        stages = {name: (folder, command) for name, folder, command in stages_for("mge", mge, ["mgeHost64/src/renderer.rs", "d3d8/x"])}
        assert stages["checks"][0] == plugin and stages["checks"][1][-1] == mge
        assert stages["replay gate"][0] == plugin and os.path.join(mge, "target", "dist-staging") in stages["replay gate"][1]
        assert stages["d3d8 build"][0] == mge and stages["contract"][1][-1] == mge

        # The state of the tree.
        plain = stages_for("plugin", plugin, ["src/lib.rs"])
        gate = stages_for("plugin", plugin, ["host/src/shader.wgsl"])
        os.makedirs(os.path.join(mge, "target"))
        assert tree_problem("plugin", plugin, plain, "abc", "abc", False) is None
        assert "not" in tree_problem("plugin", plugin, plain, "abc", "def", False)
        assert "tracked changes" in tree_problem("plugin", plugin, plain, "abc", "abc", True)
        assert "no tree" in tree_problem(None, root, [], "abc", "abc", False)
        assert tree_problem("plugin", plugin, gate, "abc", "abc", False) is None
        worktree = os.path.join(root, PLUGIN_NAME + "-x")
        assert "own name" in tree_problem("plugin", worktree, stages_for("plugin", worktree, ["host/src/shader.wgsl"]), "abc", "abc", False)
        # Without the gate, a worktree of another name passes; a missing sibling does not.
        assert tree_problem("plugin", worktree, [("a", plugin, [])], "abc", "abc", False) is None
        assert "sibling" in tree_problem("plugin", plugin, [("a", os.path.join(root, "gone"), [])], "abc", "abc", False)

        # The base.
        assert base_problem("abc", "def", True) is None
        assert "empty" in base_problem("abc", "abc", True)
        assert "no ancestor" in base_problem("abc", "def", False)

        # A stage's own exit code decides its line, and the log has its output.
        with open(os.path.join(root, "log.txt"), "w", encoding="utf-8") as log:
            assert run_stage("good", root, [sys.executable, "-c", "print('out')"], log)
            assert not run_stage("bad", root, [sys.executable, "-c", "raise SystemExit(4)"], log)
        with open(os.path.join(root, "log.txt"), encoding="utf-8") as log:
            assert "out" in log.read()
    print("merge_step self-test: ok")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("repo", nargs="?")
    parser.add_argument("commit", nargs="?")
    parser.add_argument("--base", help="the tip of the shared branch before the merge: the diff from it decides the stages")
    parser.add_argument("--no-wait", action="store_true", help="stop with exit code 3 where this would wait for the slot")
    parser.add_argument("--dry-run", action="store_true", help="print the stages: no slot, no build")
    parser.add_argument("--self-test", action="store_true", help="the tests of the pure parts: no build, no game")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.repo or not args.commit or not args.base:
        parser.error("it needs <repo>, <commit> and --base <the tip before the merge>")

    repo = os.path.abspath(args.repo)
    kind = repo_kind(repo)
    commit = git(repo, "rev-parse", args.commit)
    base = git(repo, "rev-parse", args.base)
    problem = base_problem(base, commit, subprocess.run(["git", "-C", repo, "merge-base", "--is-ancestor", base, commit]).returncode == 0)
    if problem:
        say("FAIL the base: %s" % problem)
        return 1
    paths = git(repo, "diff", "--name-only", base, commit).splitlines()
    stages = stages_for(kind, repo, paths) if kind else []
    problem = tree_problem(kind, repo, stages, git(repo, "rev-parse", "HEAD"), commit, bool(git(repo, "status", "--porcelain", "--untracked-files=no")))
    if problem:
        say("FAIL the tree: %s" % problem)
        return 1
    say("%s %s: %d changed paths from %s; stages: %s" % (os.path.basename(repo), commit[:10], len(paths), base[:10], ", ".join(name for name, _, _ in stages)))
    if args.dry_run:
        return 0

    slots = load_slots()
    what = "merge_step: %s %s" % (os.path.basename(repo), commit[:10])
    if not take_build(slots, what, args.no_wait):
        return 3
    plugin = repo if kind == "plugin" else os.path.join(os.path.dirname(repo), PLUGIN_NAME)
    os.makedirs(os.path.join(plugin, "target"), exist_ok=True)
    log_path = os.path.join(plugin, "target", "merge-step-%s.log" % commit[:10])
    try:
        with open(log_path, "w", encoding="utf-8", errors="replace") as log:
            results = [run_stage(name, folder, command, log) for name, folder, command in stages]
    finally:
        say(slots.release("build").text.splitlines()[0])
    say("%d of %d stages passed; the output is in %s" % (sum(results), len(results), log_path))
    return 0 if all(results) else 1


if __name__ == "__main__":
    sys.exit(main())
