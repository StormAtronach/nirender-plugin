"""The checks of a merge, for the Coordinator, on the merged tree: one PASS or FAIL line for each stage.

    python -I tools/merge_step.py <repo> <commit> --base <tip> [--baseline <folder>] [--no-wait] [--dry-run]
    python -I tools/merge_step.py --self-test    # the tests of the pure parts: no build, no game

<repo> is the tree of nirender-plugin or MGE-XE-G7-render-link, at <commit> and with no tracked change. There is no
trial merge: the tool refuses another tree state. It takes the build slot for the whole of its work and releases it. It
waits in the queue of that slot; with --no-wait it stops with exit code 3. --dry-run prints the stages and takes nothing.

<tip> is the tip of the shared branch before the merge. It is required: the parents of <commit> do not tell the change, as
the first parent misses a fast-forward and a branch that merged the shared branch. The tool refuses a <tip> that is newer
than <branch>@{1}, the tip before the last move of the branch, as such a diff skips a part of the merge. An older <tip> only
widens the diff. The tree must be on the branch, and a branch with no earlier tip is refused. A second run on one merge
gives the same answer.

After a later commit, <branch>@{1} moves past a merge that failed or had no run. So the ref refs/merge-step/<branch> keeps
the last commit that passed all stages, and the tool also refuses a <tip> newer than it. A branch with no such ref has only
the rule of <branch>@{1}: the tool cannot see a merge with no run before the first pass of a branch.

The stages of nirender-plugin: the plugin's tests, the host's tests, clippy_baseline.py --check, checks.py plugin. The
stages of render-link: the tests of the helper, mge-config and mge-probe, clippy_baseline.py --check --mge <repo>, checks.py
mge --mge <repo>. The diff from <tip> to <commit> adds these:

- a d3d8 build and check_contract.py --mge when the change touches d3d8, mge-config or contract/;
- the replay gate, tools/validate_retained.py baseline with no game, when the change touches the host's source, the
  helper's renderer, the shaders of the assets (the source of the shaders in a build), or a manifest or a lock of the
  build. The path decides: a stream or a table change can move a picture too. The gate does not run when its build failed.

These two stages also need a sibling with no tracked change. The first line gives the hash of the sibling.

--baseline names the folder that the gate compares with: the accepted folder without it, or a copy in the same form, as a
reference and not an acceptance. The first line names it. A pass against a named folder does not move the ref. The gate
writes nothing in it: its pictures go to <plugin>/target/merge-step-now, which each gate of this tool clears first, under the
build slot, so that a stale picture cannot compare.

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
GATE_PATHS = {"plugin": ("host/src/", "Cargo.toml", "Cargo.lock", "host/Cargo.toml", "host/Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml",
                         "host/.cargo/config.toml"),
              "mge": ("mgeHost64/src/renderer.rs", "mgeHost64/src/renderer/", "assets/Data Files/shaders/", "Cargo.toml", "Cargo.lock",
                      "mgeHost64/Cargo.toml", "mge-config/Cargo.toml", "rust-toolchain.toml", ".cargo/config.toml")}
D3D8_PATHS = {"plugin": ("contract/",), "mge": ("d3d8/", "mge-config/")}
# These move a build out of its tree. Cargo then returns 0, and the gate replays the old file of the build folder.
BUILD_MOVERS = ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "NIRENDER_PLUGIN_DIR")


def touches(paths, wanted):
    return any(path == item or (item.endswith("/") and path.startswith(item)) for path in paths for item in wanted)


def repo_kind(folder):
    """"plugin", "mge" or None, by what the tree holds."""
    if os.path.isfile(os.path.join(folder, "host", "src", "shader.wgsl")):
        return "plugin"
    return "mge" if os.path.isdir(os.path.join(folder, "mgeHost64")) else None


def sibling_of(kind, repo):
    return os.path.join(os.path.dirname(repo), MGE_NAME if kind == "plugin" else PLUGIN_NAME)


def stages_for(kind, repo, paths, baseline=BASELINE):
    """The stages of a merge, as (name, folder, command). The folder of the other repo is the sibling."""
    python = sys.executable
    plugin = repo if kind == "plugin" else sibling_of(kind, repo)
    mge = repo if kind == "mge" else sibling_of(kind, repo)
    tool = lambda name, *args: [python, "-I", os.path.join("tools", name), *args]  # noqa: E731
    if kind == "plugin":
        stages = [("plugin tests", plugin, ["cargo", "test", "--quiet"]),
                  ("host tests", os.path.join(plugin, "host"), ["cargo", "test", "--quiet"]),
                  ("clippy baseline", plugin, tool("clippy_baseline.py", "--check")),
                  ("checks", plugin, tool("checks.py", "plugin")),
                  ("docs", plugin, tool("docs.py", "--mge", mge))]
    else:
        stages = [("helper tests", mge, ["cargo", "test", "--quiet", "-p", "mgeHost64", "-p", "mge-config", "-p", "mge-probe"]),
                  ("clippy baseline", plugin, tool("clippy_baseline.py", "--check", "--mge", mge)),
                  ("checks", plugin, tool("checks.py", "mge", "--mge", mge)),
                  ("docs", plugin, tool("docs.py", "--mge", mge))]
    if touches(paths, D3D8_PATHS[kind]):
        stages.append(("d3d8 build", mge, ["cargo", "build", "--quiet", "-p", "d3d8", "--profile", "release-dll"]))
        stages.append(("contract", plugin, tool("check_contract.py", "--mge", mge)))
    if touches(paths, GATE_PATHS[kind]):
        stages.append(("build for the gate", mge, ["cargo", "xtask", "dist"]))
        stages.append(("replay gate", plugin, tool("validate_retained.py", "baseline", "--mge-build", os.path.join(mge, "target", "dist-staging"), "--baseline", baseline, "--now", os.path.join(plugin, "target", "merge-step-now"))))
    return stages


def needs_clean_sibling(stages):
    return any(name in ("replay gate", "d3d8 build") for name, _, _ in stages)


def tree_problem(kind, repo, stages, head, commit, changed, sibling_changed=False):
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
    if sibling_changed and needs_clean_sibling(stages):
        return "the sibling %s has tracked changes: a build would hold them" % sibling_of(kind, repo)
    return None


def base_problem(base, commit, is_ancestor, tip_before, within_tip_before, last_passed=None, within_last_passed=True):
    """Why the diff from this base does not tell the change, as a text, or None. tip_before is <branch>@{1}, or None."""
    if base == commit:
        return "the base is the commit, so the diff is empty and asks for no stage: give the tip before the merge"
    if not is_ancestor:
        return "the base %s is no ancestor of the commit: the diff would also hold the other side's changes" % base[:10]
    if tip_before is None:
        return "no <branch>@{1}: the tree is on no branch, or the branch has no earlier tip, so nothing shows that the base holds the whole merge"
    if not within_tip_before:
        return "the base %s is newer than the tip before the last move of the branch, %s: the diff would skip a part of the merge" % (base[:10], tip_before[:10])
    if last_passed and not within_last_passed:
        return "the base %s is newer than the last commit that passed, %s: the diff would skip a merge that failed or had no run" % (base[:10], last_passed[:10])
    return None


def stage_env(environ):
    env = {name: value for name, value in environ.items() if name not in BUILD_MOVERS}
    env["PATH"] = os.path.join(os.path.expanduser("~"), ".cargo", "bin") + os.pathsep + environ["PATH"]
    return env


def run_stage(name, folder, command, log):
    began = time.time()
    log.write("\n== %s: %s in %s\n" % (name, " ".join(command), folder))
    log.flush()
    code = subprocess.run(command, cwd=folder, env=stage_env(os.environ), stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL).returncode
    say("%s %s (%d s)%s" % ("PASS" if code == 0 else "FAIL", name, time.time() - began, "" if code == 0 else ": exit code %d" % code))
    return code == 0


def clear_pictures(folder):
    """Removes the files of the gate's folder of pictures and of its tour folder, one by one."""
    for part in (folder, os.path.join(folder, "tour")):
        if os.path.isdir(part):
            for entry in os.scandir(part):
                if entry.is_file(follow_symlinks=False):
                    os.remove(entry.path)


def run_stages(stages, log, note_stage=lambda: None):
    """All stages, also after a failure, but the gate has no build to replay after a failure of its build.
    `note_stage` runs before each stage that runs: the hold of the build slot lapses after 15 minutes with
    no compile seen, and a round can pass that with a stage that does not compile."""
    results = {}
    for name, folder, command in stages:
        if name == "replay gate" and not results.get("build for the gate", True):
            say("NOT RUN %s: its build failed" % name)
            results[name] = False
            continue
        if name == "replay gate":
            clear_pictures(command[command.index("--now") + 1])
        note_stage()
        results[name] = run_stage(name, folder, command, log)
    return list(results.values())


def moves_the_ref(baseline):
    """Only a pass against the accepted folder is a pass of the merge."""
    return os.path.normcase(os.path.abspath(baseline)) == os.path.normcase(os.path.abspath(BASELINE))


def resolve(repo, name):
    """The commit of a name, or None."""
    return subprocess.run(["git", "-C", repo, "rev-parse", "--verify", "-q", name], capture_output=True, text=True).stdout.strip() or None


def branch_of(repo):
    """The branch of the tree, or None."""
    return subprocess.run(["git", "-C", repo, "symbolic-ref", "-q", "--short", "HEAD"], capture_output=True, text=True).stdout.strip() or None


def is_ancestor(repo, older, newer):
    return subprocess.run(["git", "-C", repo, "merge-base", "--is-ancestor", older, newer]).returncode == 0


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

        base = ["plugin tests", "host tests", "clippy baseline", "checks", "docs"]
        assert names("plugin", plugin, "tools/x.py", "src/lib.rs") == base
        assert names("plugin", plugin, "host/src/shader.wgsl") == base + ["build for the gate", "replay gate"]
        assert names("plugin", plugin, "host/src/material.rs") == base + ["build for the gate", "replay gate"]
        assert names("plugin", plugin, "host/src/retained.rs") == base + ["build for the gate", "replay gate"]
        assert names("plugin", plugin, "host/src_old/x.rs") == base, "a name that only starts the same ran the gate"
        assert names("plugin", plugin, "contract/mge_render_link.h") == base + ["d3d8 build", "contract"]
        for path in ("Cargo.toml", "Cargo.lock", "host/Cargo.toml", "host/Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml", "host/.cargo/config.toml"):
            assert names("plugin", plugin, path) == base + ["build for the gate", "replay gate"], path
        mge_base = ["helper tests", "clippy baseline", "checks", "docs"]
        for path in ("Cargo.toml", "Cargo.lock", "mgeHost64/Cargo.toml", "rust-toolchain.toml", ".cargo/config.toml"):
            assert names("mge", mge, path) == mge_base + ["build for the gate", "replay gate"], path
        assert names("mge", mge, "mge-config/Cargo.toml") == mge_base + ["d3d8 build", "contract", "build for the gate", "replay gate"]
        assert "mge-probe" in stages_for("mge", mge, [])[0][2]
        assert stages_for("mge", mge, [])[1][2][-2:] == ["--mge", mge]
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

        # The gate compares with the folder that it is given, and neither the accepted folder nor a copy takes its pictures.
        for folder, given in ((BASELINE, ()), (os.path.join(root, "copy"), (os.path.join(root, "copy"),))):
            command = stages_for("plugin", plugin, ["host/src/shader.wgsl"], *given)[-1][2]
            assert command[command.index("--baseline") + 1] == folder and "--accept" not in command
            assert not command[command.index("--now") + 1].startswith(folder + os.sep), folder

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
        # A change in the sibling stops the stages that build from it, and only these.
        assert "tracked changes" in tree_problem("plugin", plugin, gate, "abc", "abc", False, True)
        d3d8 = stages_for("plugin", plugin, ["contract/x.h"])
        assert "tracked changes" in tree_problem("plugin", plugin, d3d8, "abc", "abc", False, True)
        assert tree_problem("plugin", plugin, plain, "abc", "abc", False, True) is None

        # The base. An older base than the tip before the move only widens the diff.
        assert base_problem("abc", "def", True, "abc", True) is None
        assert base_problem("old", "def", True, "abc", True) is None
        assert "empty" in base_problem("abc", "abc", True, "abc", True)
        assert "no ancestor" in base_problem("abc", "def", False, "abc", True)
        # A fast-forward of two commits with --base HEAD^: the base is newer than the tip before the merge.
        assert "skip a part" in base_problem("head^", "head", True, "abc", False)
        assert "no <branch>@{1}" in base_problem("abc", "def", True, None, False)
        # A later commit moved the tip past a merge that failed or had no run.
        assert "that failed or had no run" in base_problem("merge", "later", True, "merge", True, "passed", False)
        assert base_problem("passed", "later", True, "merge", True, "passed", True) is None

        # A stage's build stays in its tree.
        moved = stage_env({"PATH": "p", "OTHER": "o", "CARGO_TARGET_DIR": "t", "CARGO_BUILD_TARGET_DIR": "t", "NIRENDER_PLUGIN_DIR": "d"})
        assert moved["OTHER"] == "o" and moved["PATH"].endswith(os.pathsep + "p") and not set(BUILD_MOVERS) & set(moved)

        # A stage's own exit code decides its line, and the log has its output.
        with open(os.path.join(root, "log.txt"), "w", encoding="utf-8") as log:
            assert run_stage("good", root, [sys.executable, "-c", "print('out')"], log)
            assert not run_stage("bad", root, [sys.executable, "-c", "raise SystemExit(4)"], log)
        with open(os.path.join(root, "log.txt"), encoding="utf-8") as log:
            assert "out" in log.read()

        # The gate does not run on a failed build, and a failure stops no other stage.
        good, bad = [sys.executable, "-c", "pass"], [sys.executable, "-c", "raise SystemExit(4)"]
        now = os.path.join(root, "now")
        gate_stage = ("replay gate", root, good + ["--now", now])
        os.makedirs(os.path.join(now, "tour"))
        with open(os.path.join(root, "log2.txt"), "w", encoding="utf-8") as log:
            assert run_stages([("a", root, bad), ("build for the gate", root, bad), gate_stage], log) == [False, False, False]
        with open(os.path.join(root, "log2.txt"), encoding="utf-8") as log:
            assert "== replay gate" not in log.read()
        # A stale picture cannot compare: the gate clears its folder and the tour's.
        for stale in (os.path.join(now, "old.png"), os.path.join(now, "tour", "old.png")):
            open(stale, "w", encoding="ascii").close()
        with open(os.path.join(root, "log3.txt"), "w", encoding="utf-8") as log:
            assert run_stages([("a", root, bad), ("build for the gate", root, good), gate_stage], log) == [False, True, True]
        assert os.listdir(now) == ["tour"] and os.listdir(os.path.join(now, "tour")) == []

        # Each stage that runs notes the slot's work, and a gate that does not run does not.
        notes = []
        with open(os.path.join(root, "log4.txt"), "w", encoding="utf-8") as log:
            run_stages([("a", root, good), ("build for the gate", root, bad), gate_stage], log, lambda: notes.append(1))
        assert len(notes) == 2

        # A pass against a named folder does not move the ref.
        assert moves_the_ref(BASELINE) and not moves_the_ref(os.path.join(root, "copy"))
    print("merge_step self-test: ok")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("repo", nargs="?")
    parser.add_argument("commit", nargs="?")
    parser.add_argument("--base", help="the tip of the shared branch before the merge: the diff from it decides the stages")
    parser.add_argument("--baseline", default=BASELINE, help="the folder that the gate compares with; without it, the accepted folder")
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
    branch = branch_of(repo)
    tip_before = resolve(repo, branch + "@{1}") if branch else None
    passed_ref = "refs/merge-step/%s" % branch
    last_passed = resolve(repo, passed_ref) if branch else None
    problem = base_problem(base, commit, is_ancestor(repo, base, commit), tip_before, bool(tip_before) and is_ancestor(repo, base, tip_before),
                           last_passed, bool(last_passed) and is_ancestor(repo, base, last_passed))
    if problem:
        say("FAIL the base: %s" % problem)
        return 1
    paths = git(repo, "diff", "--name-only", base, commit).splitlines()
    baseline = os.path.abspath(args.baseline)
    stages = stages_for(kind, repo, paths, baseline) if kind else []
    sibling = sibling_of(kind, repo) if kind else ""
    has_sibling = os.path.isdir(sibling)
    sibling_changed = has_sibling and bool(git(sibling, "status", "--porcelain", "--untracked-files=no"))
    problem = tree_problem(kind, repo, stages, git(repo, "rev-parse", "HEAD"), commit, bool(git(repo, "status", "--porcelain", "--untracked-files=no")), sibling_changed)
    if problem:
        say("FAIL the tree: %s" % problem)
        return 1
    sibling_hash = git(sibling, "rev-parse", "--short=10", "HEAD") if has_sibling else "none"
    say("%s %s, sibling %s, baseline %s: %d changed paths from %s; stages: %s" % (os.path.basename(repo), commit[:10], sibling_hash, baseline, len(paths), base[:10], ", ".join(name for name, _, _ in stages)))
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
            results = run_stages(stages, log, lambda: slots.note_active("build", time.time()))
    finally:
        say(slots.release("build").text.splitlines()[0])
    say("%d of %d stages passed; the output is in %s" % (sum(results), len(results), log_path))
    if not all(results):
        return 1
    if moves_the_ref(baseline):
        git(repo, "update-ref", passed_ref, commit)
    else:
        say("%s stays: this pass is against %s, not the accepted folder" % (passed_ref, baseline))
    return 0


if __name__ == "__main__":
    sys.exit(main())
