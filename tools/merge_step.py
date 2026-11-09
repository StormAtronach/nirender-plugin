"""The checks of a merge, for the Coordinator, on the merged tree: one PASS or FAIL line for each stage.

    python -I tools/merge_step.py <repo> <commit> --base <tip> [--baseline <folder>] [--no-wait] [--dry-run]
    python -I tools/merge_step.py --self-test    # the tests of the pure parts: no build, no game

<repo> is the tree of nirender-plugin or MGE-XE-G7-render-link, at <commit> and with no tracked change. There is no
trial merge: the tool refuses another tree state. It takes the build slot for the whole of its work and releases it. It
waits in the queue of that slot; with --no-wait it stops with exit code 3. --dry-run prints the stages and takes nothing.

It also takes the game slot (a lease of 4 hours) for the whole check (the owner's D161). It asks for it at each wait for
the build slot, so that the check keeps its place in the game queue (it holds the game slot when that is free first). The stages write their logs to
E:, and a game that starts meanwhile stalls on its own log writes. The label is the build label plus " (game slot)". Both
slots are released on every exit path: a pass, a failing stage, an exception and Ctrl-C. While the stages run, a thread notes
the game slot's work each minute, as no game runs and the slot lapses after 15 idle minutes. --dry-run and --self-test take
no slot (the self-test uses slot.py on a scratch state folder). A hold of the game slot by another session, or by an earlier
run of this session, makes this run wait in the queue; with --no-wait it stops with exit code 3 and releases the build slot.

<tip> is the tip of the shared branch before the merge. It is required: the parents of <commit> do not tell the change, as
the first parent misses a fast-forward and a branch that merged the shared branch. The tool refuses a <tip> that is newer
than <branch>@{1}, the tip before the last move of the branch, as such a diff skips a part of the merge. An older <tip> only
widens the diff. The tree must be on the branch, and a branch with no earlier tip is refused. A second run on one merge
gives the same answer.

After a later commit, <branch>@{1} moves past a merge that failed or had no run. So the ref refs/merge-step/<branch> keeps
the last commit that passed all stages, and the tool also refuses a <tip> newer than it. A branch with no such ref has only
the rule of <branch>@{1}: the tool cannot see a merge with no run before the first pass of a branch.

The stages of nirender-plugin: the plugin's tests, the host's tests, the host's GPU tests (its ignored tests that need a
GPU, as below, with the tests that run only in a child process left out), clippy_baseline.py --check, checks.py plugin. The
stages of render-link: the tests of the helper, mge-config and mge-probe; the helper's GPU tests (cargo test -p mgeHost64 --
--ignored --test-threads=1: the tests marked #[ignore] that need a GPU, one at a time, run through "merge_step.py
--require-tests <command>", which fails when no test ran); clippy_baseline.py --check --mge <repo>, checks.py mge --mge <repo>. The diff from <tip> to <commit> adds these:

- a d3d8 build and check_contract.py --mge when the change touches d3d8, mge-config or contract/;
- the replay gate, tools/validate_retained.py baseline with no game, when the change touches the host's source, the
  helper's renderer, the shaders of the assets (the source of the shaders in a build), or a manifest or a lock of the
  build. The path decides: a stream or a table change can move a picture too. The gate does not run when its build failed;
- with the gate, the A/B stages of tools/instance_ab.py: `base build` keeps the build of the current tips of the target
  branches of BOTH repos in E:\\nirender-keep\\merge-step\\base-<branch> (key: both commits and the profile; a tip that
  moved builds again), and `instance A/B grass` replays a capture of Outlander-test with the base, the build under test and
  the base again, on the same data, and compares the pictures and the hash lines of the grass calls. NOT RUN is not a PASS:
  it counts as a stage that did not pass (exit code 5 of the tool). The base is built at <tip> for the tree's repo and at
  the HEAD of the sibling, which the build under test has too, so that the A/B shows the change of the tree's repo;
- the build of the dist alone, `cargo xtask dist`, when the change touches the mod's Lua (xtask checks it against the list
  of the files it ships) or xtask, and not the paths of the gate: the replay cannot see these.

These stages also need a sibling with no tracked change. The first line gives the hash of the sibling.

--baseline names the folder that the gate compares with: the accepted folder without it, or a copy in the same form, as a
reference and not an acceptance. The first line names it. A pass against a named folder does not move the ref. The gate
writes nothing in it: its pictures go to <plugin>/target/merge-step-now, which each gate of this tool clears first, under the
build slot, so that a stale picture cannot compare.

All stages run, also after a failure. Exit code 1 on any failure. The output of each stage, the PASS, FAIL or NOT RUN line of
each stage and the closing "N of N stages passed" line are in
E:\\nirender-keep\\merge-step\\merge-step-<commit>.log, where the data rule on build output does not reach. It pushes nothing.

The gate needs what a diff cannot give:

- the two repos as siblings under their own names, as the helper's Cargo.toml names ../../nirender-plugin/host. A
  worktree with another name would build the host of another tree, so the gate refuses it;
- `cargo xtask dist` of the pair, as the gate replays with the helper and the shaders of a build. This is the longest stage;
- the sibling at the commit that the merge goes with: this tool does not know the other repo's tip;
- accepted pictures. A planned new look fails the gate until a baseline is accepted, and that is the owner's choice. The
  accepted pictures also hold the distant land and the mgeXE.toml of their time, from the testing instance.
"""
import argparse
import contextlib
import importlib.util
import inspect
import io
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
import types

# The modules of this folder, also under `python -I`.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from instance_ab import NOT_RUN as NOT_RUN_CODE  # noqa: E402
from common import git_env  # noqa: E402
from make_play_set import BASELINE, HARNESS, WAIT_SECONDS, has_captures, load_slots, say, slot_asker  # noqa: E402

PLUGIN_NAME = "nirender-plugin"
MGE_NAME = "MGE-XE-G7-render-link"
SLOT_MINUTES = 60
# The lease of the game slot: the stages of a round run for up to an hour on a slow day (the longest of 2026-11-09 ran 38
# minutes), and the lease does not renew while they run.
GAME_SLOT_MINUTES = 240
# The label of this tool's hold of the game slot, from the label of its hold of the build slot.
GAME_LABEL = "%s (game slot)"
# How often a round notes the work of the game slot while its stages run (the slot's idle limit is 15 minutes).
GAME_NOTE_SECONDS = 60
LOG_FOLDER = "E:\\nirender-keep\\merge-step"
# The stages that replay the build of the gate or the kept base build.
INSTANCE_STAGE = "instance A/B grass"
BASE_STAGE = "base build"
# The stage of the helper's tests that need a GPU (marked #[ignore]); `merge_step.py REQUIRE_TESTS <command>` runs one
# command and fails when no test ran.
GPU_STAGE = "helper GPU tests"
HOST_GPU_STAGE = "host GPU tests"
REQUIRE_TESTS = "--require-tests"
# What a change must touch for the gate, as paths from the root of each repo: a file, or a folder with its slash.
GATE_PATHS = {"plugin": ("host/src/", "Cargo.toml", "Cargo.lock", "host/Cargo.toml", "host/Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml",
                         "host/.cargo/config.toml"),
              "mge": ("mgeHost64/src/renderer.rs", "mgeHost64/src/renderer/", "assets/Data Files/shaders/", "Cargo.toml", "Cargo.lock",
                      "mgeHost64/Cargo.toml", "mge-config/Cargo.toml", "rust-toolchain.toml", ".cargo/config.toml")}
D3D8_PATHS = {"plugin": ("contract/",), "mge": ("d3d8/", "mge-config/")}
# What a change must touch for the build of the dist alone: what the dist ships or checks and the replay cannot see (the
# mod's Lua, which xtask checks against its list of shipped files; xtask itself).
DIST_PATHS = {"plugin": ("mod/MWSE/mods/nirender/",), "mge": ("xtask/",)}
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


def gpu_command(plugin, package_args, skipped=()):
    """The command of a stage of ignored tests that need a GPU: cargo test --ignored, through the wrapper that fails when no
    test ran. One test thread: each module's GPU tests hold a lock of their own, so side by side they ran at the same
    time on one device and a timing test failed once under load. `skipped` names tests that stay out, by a part of their
    path (the tests of the host that dump or crash this process run alone in a child; their names end in _in_this_process)."""
    skip = [argument for name in skipped for argument in ("--skip", name)]
    return [sys.executable, "-I", os.path.join(plugin, "tools", "merge_step.py"), REQUIRE_TESTS, "cargo", "test", "--quiet", *package_args,
            "--", "--ignored", *skip, "--test-threads=1"]


def stages_for(kind, repo, paths, baseline=BASELINE, tips=None, base_folder=None):
    """The stages of a merge, as (name, folder, command). The folder of the other repo is the sibling. `tips` are the
    commits of the base build, {"plugin": sha, "mge": sha}, and `base_folder` holds it: with both, the gate is followed by
    the A/B stages."""
    python = sys.executable
    plugin = repo if kind == "plugin" else sibling_of(kind, repo)
    mge = repo if kind == "mge" else sibling_of(kind, repo)
    tool = lambda name, *args: [python, "-I", os.path.join("tools", name), *args]  # noqa: E731
    if kind == "plugin":
        stages = [("plugin tests", plugin, ["cargo", "test", "--quiet"]),
                  ("host tests", os.path.join(plugin, "host"), ["cargo", "test", "--quiet"]),
                  (HOST_GPU_STAGE, os.path.join(plugin, "host"), gpu_command(plugin, [], ["_in_this_process"])),
                  ("clippy baseline", plugin, tool("clippy_baseline.py", "--check")),
                  ("checks", plugin, tool("checks.py", "plugin")),
                  ("docs", plugin, tool("docs.py", "--mge", mge))]
    else:
        stages = [("helper tests", mge, ["cargo", "test", "--quiet", "-p", "mgeHost64", "-p", "mge-config", "-p", "mge-probe"]),
                  (GPU_STAGE, mge, gpu_command(plugin, ["-p", "mgeHost64"])),
                  ("clippy baseline", plugin, tool("clippy_baseline.py", "--check", "--mge", mge)),
                  ("checks", plugin, tool("checks.py", "mge", "--mge", mge)),
                  ("docs", plugin, tool("docs.py", "--mge", mge))]
    if touches(paths, D3D8_PATHS[kind]):
        stages.append(("d3d8 build", mge, ["cargo", "build", "--quiet", "-p", "d3d8", "--profile", "release-dll"]))
        stages.append(("contract", plugin, tool("check_contract.py", "--mge", mge)))
    if touches(paths, GATE_PATHS[kind]):
        stages.append(("build for the gate", mge, ["cargo", "xtask", "dist"]))
        staging = os.path.join(mge, "target", "dist-staging")
        stages.append(("replay gate", plugin, tool("validate_retained.py", "baseline", "--mge-build", staging, "--baseline", baseline, "--now", os.path.join(plugin, "target", "merge-step-now"))))
        if tips and base_folder:
            stages.append((BASE_STAGE, plugin, tool("instance_ab.py", "base-build", "--plugin-repo", plugin, "--mge-repo", mge, "--plugin-tip", tips["plugin"], "--mge-tip", tips["mge"], "--out", base_folder)))
            stages.append((INSTANCE_STAGE, plugin, tool("instance_ab.py", "run", "--stage", "grass", "--baseline", baseline, "--base-build", os.path.join(base_folder, "build"), "--now-build", staging, "--out", os.path.join(plugin, "target", "merge-step-instance"))))
    elif touches(paths, DIST_PATHS[kind]):
        stages.append(("build of the dist", mge, ["cargo", "xtask", "dist"]))
    return stages


# The summary line of one test binary in the output of cargo test, quiet or not.
TEST_RESULT = re.compile(r"^test result: \w+\. (\d+) passed; (\d+) failed;", re.MULTILINE)


def tests_ran(output):
    """How many tests the output of `cargo test` says ran: passed and failed, over every test binary."""
    return sum(int(passed) + int(failed) for passed, failed in TEST_RESULT.findall(output))


def require_tests(command):
    """Runs one command (cargo test) and echoes its output. Exit code of the command, or 1 when it passed and no test ran:
    a filter or a lost #[ignore] leaves such a stage with nothing to judge, and 0 tests are no pass."""
    done = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
    output = done.stdout.decode("utf-8", "replace")
    # What the console or the log file cannot encode becomes a "?", as the write would fail on it.
    encoding = sys.stdout.encoding or "utf-8"
    sys.stdout.write(output.encode(encoding, "replace").decode(encoding))
    sys.stdout.flush()
    ran = tests_ran(output)
    say("%d tests ran" % ran)
    if done.returncode == 0 and ran == 0:
        say("FAIL: no test ran, so the stage judged nothing")
        return 1
    return done.returncode


def needs_clean_sibling(stages):
    return any(name in ("replay gate", "d3d8 build", "build of the dist") for name, _, _ in stages)


def baseline_problem(stages, baseline):
    """Why the replay gate of these stages cannot compare, as a text, or None. main() asks before the take and the
    builds: a baseline with no captures failed the gate only after them (2026-11-09, Rob's run 8b51a23d59)."""
    if not any(name == "replay gate" for name, _, _ in stages) or has_captures(baseline):
        return None
    return "%s has no capture (*.nircap), so the replay gate cannot compare: give --baseline a folder with the captures" % baseline


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


def verdict(log, text):
    """One line of the round's verdict: on the console, and in the log, so that a kept log shows it."""
    say(text)
    log.write("== %s\n" % text)
    log.flush()


# The start of the refusal of slot.py's `refuse_build_tool`, which replay.py calls for each replay of the gate.
SLOT_REFUSAL = "Refused by the slot check: "


def slot_refusal_in(output):
    """The first sentence of the last refusal of the slot check in a stage's output, or None. A replay that the slot
    check refused (no room on C, another holder) is no fault of the pictures, and the FAIL line must say so."""
    refusal = None
    for line in output.splitlines():
        at = line.find(SLOT_REFUSAL)
        if at >= 0:
            refusal = line[at + len(SLOT_REFUSAL):]
    if refusal is None:
        return None
    # The refusal names its tool first: "replay.py: only 29.0 GB free on C, ...".
    tool, colon, reason = refusal.partition(": ")
    if colon and (tool.endswith(".py") or tool == "build tool"):
        refusal = reason
    return refusal.split(". ")[0].strip()


def output_since(log, start):
    """What the log file holds after the byte `start`: the output of the stage that wrote there."""
    with open(log.name, "rb") as file:
        file.seek(start)
        return file.read().decode("utf-8", "replace")


def not_run_reason(output):
    """The reason in the last "NOT RUN" line of a stage's output, or a plain text."""
    reason = "the stage could not judge"
    for line in output.splitlines():
        if line.startswith("NOT RUN"):
            reason = line.partition(": ")[2] or line
    return reason


def run_stage(name, folder, command, log):
    began = time.time()
    log.write("\n== %s: %s in %s\n" % (name, " ".join(command), folder))
    log.flush()
    start = log.tell()
    code = subprocess.run(command, cwd=folder, env=stage_env(os.environ), stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL).returncode
    failure = ""
    if code == NOT_RUN_CODE:
        verdict(log, "NOT RUN %s (%d s): %s" % (name, time.time() - began, not_run_reason(output_since(log, start))))
        return False
    if code != 0:
        refusal = slot_refusal_in(output_since(log, start))
        failure = ": exit code %d%s" % (code, "; refused by the slot check: %s" % refusal if refusal else "")
    verdict(log, "%s %s (%d s)%s" % ("PASS" if code == 0 else "FAIL", name, time.time() - began, failure))
    return code == 0


def log_path_of(commit, folder=LOG_FOLDER):
    """The file of the output of a round, in a folder that exists."""
    os.makedirs(folder, exist_ok=True)
    return os.path.join(folder, "merge-step-%s.log" % commit[:10])


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
        if name in ("replay gate", INSTANCE_STAGE) and not results.get("build for the gate", True):
            verdict(log, "NOT RUN %s: its build failed" % name)
            results[name] = False
            continue
        if name == INSTANCE_STAGE and not results.get(BASE_STAGE, True):
            verdict(log, "NOT RUN %s: the base build failed" % name)
            results[name] = False
            continue
        if name == "replay gate":
            clear_pictures(command[command.index("--now") + 1])
        note_stage()
        results[name] = run_stage(name, folder, command, log)
    return list(results.values())


def summary_line(results, log_path):
    return "%d of %d stages passed; the output is in %s" % (sum(results), len(results), log_path)


def moves_the_ref(baseline):
    """Only a pass against the accepted folder is a pass of the merge."""
    return os.path.normcase(os.path.abspath(baseline)) == os.path.normcase(os.path.abspath(BASELINE))


def resolve(repo, name):
    """The commit of a name, or None."""
    return subprocess.run(["git", "-C", repo, "rev-parse", "--verify", "-q", name], capture_output=True, text=True, env=git_env()).stdout.strip() or None


def branch_of(repo):
    """The branch of the tree, or None."""
    return subprocess.run(["git", "-C", repo, "symbolic-ref", "-q", "--short", "HEAD"], capture_output=True, text=True).stdout.strip() or None


def is_ancestor(repo, older, newer):
    return subprocess.run(["git", "-C", repo, "merge-base", "--is-ancestor", older, newer]).returncode == 0


def take_options(slots, stages):
    """The options of the take: the whole build side (both places) when a stage replays, as a replay needs it (the
    owner's D139). A slot.py from before D139 has one place, and its take has no such option."""
    replays = any(name in ("replay gate", INSTANCE_STAGE) for name, _, _ in stages)
    if replays and "whole" in inspect.signature(slots.take).parameters:
        return {"whole": True}
    return {}


# The refusal of slot.py when this session already holds the build slot under another label: another run of this
# session, usually an earlier merge_step that still runs. The first form is slot.py's until 2026-11-09, the second its
# plainer form after it.
OWN_HOLD = ("this session holds it as", "your session already holds the build slot as")


def take_slot(slots, slot, what, no_wait, options=None, minutes=SLOT_MINUTES, ask=None, while_waiting=None):
    """Takes a slot ("build" or "game"), and asks again in its queue, so that the place stays. A hold of another run of
    this session (an earlier merge_step) cannot overlap this run: it waits for that release too. False, with a "NOT RUN"
    line, with --no-wait when it must wait, on the end of the wait after a `leave` of its place, and on any other
    refusal, as waiting cannot pass that. A Ctrl-C in the wait leaves the queue. `ask` is an asker of the caller (see
    `slot_asker`), which keeps its place across takes. `while_waiting()` runs at each wait of a take that waits."""
    os.makedirs(slots.STATE_DIR, exist_ok=True)
    waiting_for_own = False
    with contextlib.ExitStack() as stack:
        if ask is None:
            ask = stack.enter_context(slot_asker(slots, slot))
        while True:
            try:
                reply = ask(what, minutes=minutes, **(options or {}))
            except slots.StateBusy as error:
                say("the state of the slots is busy, and I ask again: %s" % error)
                time.sleep(WAIT_SECONDS)
                continue
            except KeyboardInterrupt:
                slots.leave(slot, what)
                raise
            if reply.code == slots.DONE:
                # All lines: a take of the build slot names the free space on C, and warns when the cleaning is due.
                say(reply.text)
                return True
            first = reply.text.splitlines()[0]
            if reply.code == getattr(slots, "WAIT_ENDED", None):
                say("NOT RUN: this run left the queue of the %s slot: %s" % (slot, first))
                return False
            own = reply.code == slots.REFUSED and any(phrase in first for phrase in OWN_HOLD)
            if reply.code == slots.REFUSED and not own:
                say("NOT RUN: the take of the %s slot was refused: %s" % (slot, first))
                return False
            if no_wait:
                if not own:
                    slots.leave(slot, what)
                say("NOT RUN: --no-wait, and the %s slot is not free: %s" % (slot, first))
                return False
            if own and not waiting_for_own:
                say("waiting: another run of this session holds the %s slot, and this run starts after its release: %s" % (slot, first))
            elif not own:
                say(first)
            waiting_for_own = own
            if while_waiting:
                while_waiting()
            time.sleep(WAIT_SECONDS)


def take_build(slots, what, no_wait, options=None, while_waiting=None):
    return take_slot(slots, "build", what, no_wait, options, while_waiting=while_waiting)


class GamePlace:
    """The game slot of one round, asked for from the start of the round. A check that waits for the build side keeps its
    place in the game queue meanwhile: with no ask, another session's game take comes first, and this check then waits
    for that game as well (2026-11-09, a merge check behind a timing take of 50 minutes). When the game is free for the
    check before the build side is, it holds the game and notes its work, as no game runs and the slot lapses after 15 idle
    minutes. The label is this run's own, so that its release names it."""

    def __init__(self, slots, label, ask):
        self.slots = slots
        self.label = label
        self.ask = ask
        self.held = False
        self.closed = False

    def ask_in_queue(self):
        """One ask while the build take waits: a place in the queue, or the hold, and then the note of its work."""
        if self.held:
            self.slots.note_active("game", time.time())
            return
        try:
            reply = self.ask(self.label, minutes=GAME_SLOT_MINUTES)
        except self.slots.StateBusy:
            return
        if reply.code == self.slots.DONE:
            self.held = True
            say("the game slot is free for this run before the build side: %s" % reply.text.splitlines()[0])

    def take(self, no_wait):
        """Takes the game slot for the whole check (the owner's D161): the stages write logs to E:, and a game that starts in
        that time stalls on its own log writes."""
        if self.held:
            return True
        self.held = take_slot(self.slots, "game", self.label, no_wait, minutes=GAME_SLOT_MINUTES, ask=self.ask)
        return self.held

    def release(self):
        """Releases the hold, or leaves the queue when the run holds nothing. Prints every line of the reply."""
        if self.closed:
            return
        self.closed = True
        if self.held:
            self.held = False
            say(self.slots.release("game", self.label).text)
        else:
            self.slots.leave("game", self.label)


def take_game(slots, label, no_wait):
    """Takes the game slot at once, for a caller with no wait before it (the self-test)."""
    return take_slot(slots, "game", label, no_wait, minutes=GAME_SLOT_MINUTES)


def release_build(slots):
    """Releases the build slot and prints all lines of the reply: after a build, slot.py names the free space on C and
    warns when the cleaning is due (the owner's D138 and D141)."""
    say(slots.release("build").text)


def release_game(slots, label):
    """Releases the game hold of this run by its label: from a terminal with no session, slot.py owns a hold only by its
    label. Prints every line of the reply."""
    say(slots.release("game", label).text)


@contextlib.contextmanager
def game_hold_noted(slots, seconds=GAME_NOTE_SECONDS):
    """Notes the work of the game slot every `seconds` while the stages run: its hold lapses after 15 minutes with no game
    seen, and the stages start none. A run that dies stops the notes, and its hold lapses by itself."""
    stop = threading.Event()

    def note():
        while True:
            slots.note_active("game", time.time())
            if stop.wait(seconds):
                return

    thread = threading.Thread(target=note, daemon=True)
    thread.start()
    try:
        yield
    finally:
        stop.set()
        thread.join()


def run_round(slots, stages, what, log_path, no_wait):
    """The stages of one round with the build slot and the game slot held, both released on every exit path: a failing
    stage, an exception and Ctrl-C. (results, summary), or None when a take did not give a slot. The run asks for the game
    slot at each wait for the build side (see `GamePlace`), so that it keeps its place in the game queue."""
    with slot_asker(slots, "game") as game_ask:
        game = GamePlace(slots, GAME_LABEL % what, game_ask)
        try:
            if not take_build(slots, what, no_wait, take_options(slots, stages), while_waiting=game.ask_in_queue):
                return None
            try:
                if not game.take(no_wait):
                    return None
                with game_hold_noted(slots), open(log_path, "w", encoding="utf-8", errors="replace") as log:
                    results = run_stages(stages, log, lambda: slots.note_active("build", time.time()))
                    # In the log before it closes: the last line of a kept log is the round's verdict. The console gets it after the slot's line.
                    summary = summary_line(results, log_path)
                    log.write("\n== %s\n" % summary)
            finally:
                game.release()
                release_build(slots)
        finally:
            game.release()
    return results, summary


def git(repo, *args):
    done = subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True)
    if done.returncode != 0:
        raise SystemExit("git %s in %s: %s" % (" ".join(args), repo, done.stderr.strip()))
    return done.stdout.strip()


def base_folder_of(branch):
    """The folder of the kept base build of a target branch: one for each branch, replaced at each pass."""
    return os.path.join(LOG_FOLDER, "base-" + branch.replace("/", "-"))


def tips_for(kind, repo, base):
    """The commits of the base build: `base` for the tree's repo, and for the sibling the HEAD of its tree, which the build
    under test has too: the two builds differ in the tree's repo only. (tips, None), or (None, why not)."""
    other = "mge" if kind == "plugin" else "plugin"
    sibling = sibling_of(kind, repo)
    if not os.path.isdir(sibling):
        return None, "no sibling %s for the base build" % sibling
    sibling_head = resolve(sibling, "HEAD")
    if sibling_head is None:
        return None, "the sibling %s has no HEAD for the base build" % sibling
    return {kind: base, other: sibling_head}, None


def two_places_take_for_ab(slot, who, *, minutes=None, timing=False, whole=False):
    pass


def scratch_slots(state):
    """slot.py on a scratch state folder, as a module of its own: it reads SLOT_DIR and the process names when it loads. A
    fake session holds the slots, and the names of the game and the compilers match no process, so that a real game or a
    real compile cannot refuse a take of the test. The environment is the test's to keep."""
    os.makedirs(state, exist_ok=True)
    os.environ.update(SLOT_DIR=state, SLOT_GAME_IMAGE="no-such-program.exe", SLOT_COMPILERS="no-such-program.exe", SLOT_PROJECT_FOLDERS="",
                      SLOT_FREE_MEMORY_GB="64", SLOT_REPLAY_IMAGE="no-such-program.exe", CLAUDE_CODE_SESSION_ID="merge-step-self-test")
    os.environ.pop("SLOT_OVERRIDE", None)
    sys.path.insert(0, HARNESS)
    spec = importlib.util.spec_from_file_location("slot_of_the_self_test", os.path.join(HARNESS, "slot.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


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

        base = ["plugin tests", "host tests", HOST_GPU_STAGE, "clippy baseline", "checks", "docs"]
        assert names("plugin", plugin, "tools/x.py", "src/lib.rs") == base
        assert names("plugin", plugin, "host/src/shader.wgsl") == base + ["build for the gate", "replay gate"]
        assert names("plugin", plugin, "host/src/material.rs") == base + ["build for the gate", "replay gate"]
        assert names("plugin", plugin, "host/src/retained.rs") == base + ["build for the gate", "replay gate"]
        assert names("plugin", plugin, "host/src_old/x.rs") == base, "a name that only starts the same ran the gate"
        assert names("plugin", plugin, "contract/mge_render_link.h") == base + ["d3d8 build", "contract"]
        # The mod's Lua: xtask dist checks it against its list of shipped files (7b7b72d broke the dist that way).
        assert names("plugin", plugin, "mod/MWSE/mods/nirender/main.lua") == base + ["build of the dist"]
        assert names("plugin", plugin, "mod/MWSE/mods/nirender/main.lua", "host/src/retained.rs") == base + ["build for the gate", "replay gate"]
        assert needs_clean_sibling(stages_for("plugin", plugin, ["mod/MWSE/mods/nirender/noplugin.lua"]))
        for path in ("Cargo.toml", "Cargo.lock", "host/Cargo.toml", "host/Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml", "host/.cargo/config.toml"):
            assert names("plugin", plugin, path) == base + ["build for the gate", "replay gate"], path
        mge_base = ["helper tests", GPU_STAGE, "clippy baseline", "checks", "docs"]
        assert names("mge", mge, "xtask/src/main.rs") == mge_base + ["build of the dist"]
        for path in ("Cargo.toml", "Cargo.lock", "mgeHost64/Cargo.toml", "rust-toolchain.toml", ".cargo/config.toml"):
            assert names("mge", mge, path) == mge_base + ["build for the gate", "replay gate"], path
        assert names("mge", mge, "mge-config/Cargo.toml") == mge_base + ["d3d8 build", "contract", "build for the gate", "replay gate"]
        assert "mge-probe" in stages_for("mge", mge, [])[0][2]
        # The GPU tests run the ignored tests of the helper in its tree, through the wrapper that fails on 0 tests, and the
        # plugin's tests do not run them.
        gpu_folder, gpu_command = stages_for("mge", mge, [])[1][1:]
        assert gpu_folder == mge and gpu_command[3:] == [REQUIRE_TESTS, "cargo", "test", "--quiet", "-p", "mgeHost64", "--", "--ignored", "--test-threads=1"]
        assert gpu_command[:2] == [sys.executable, "-I"] and gpu_command[2] == os.path.join(plugin, "tools", "merge_step.py")
        assert all(name != GPU_STAGE for name, _, _ in stages_for("plugin", plugin, []))
        # The host's GPU tests run in the host's folder, with the child-only tests (which dump this process; their names end in _in_this_process) left out, and the
        # helper's stage is not in the plugin's list or the other way round.
        host_folder, host_command = stages_for("plugin", plugin, [])[2][1:]
        assert stages_for("plugin", plugin, [])[2][0] == HOST_GPU_STAGE and host_folder == os.path.join(plugin, "host")
        assert host_command[3:] == [REQUIRE_TESTS, "cargo", "test", "--quiet", "--", "--ignored", "--skip", "_in_this_process", "--test-threads=1"], host_command
        assert all(name != HOST_GPU_STAGE for name, _, _ in stages_for("mge", mge, []))
        assert stages_for("mge", mge, [])[2][2][-2:] == ["--mge", mge]
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
            text = log.read()
        assert "out" in text
        # The verdict of each stage is in the log too, so that a kept log shows it.
        assert "== PASS good (" in text and "== FAIL bad (" in text and ": exit code 4" in text, text
        assert "refused" not in text, "a stage with no refusal names none"
        assert summary_line([True, False, True], "x.log") == "2 of 3 stages passed; the output is in x.log"

        # A replay that the slot check refused: the FAIL line names the reason, as validate_retained prints it (indented,
        # the last lines of replay.py). An older refusal in the log before the stage does not count.
        refused = "       Refused by the slot check: replay.py: only 29.0 GB free on C, and a build needs 30 GB. Ask the Coordinator to read the lists"
        with open(os.path.join(root, "log5.txt"), "w", encoding="utf-8") as log:
            assert not run_stage("replay gate", root, [sys.executable, "-c", "print(%r); raise SystemExit(1)" % refused], log)
            assert not run_stage("after", root, [sys.executable, "-c", "raise SystemExit(4)"], log)
        with open(os.path.join(root, "log5.txt"), encoding="utf-8") as log:
            text = log.read()
        assert ": exit code 1; refused by the slot check: only 29.0 GB free on C, and a build needs 30 GB\n" in text, text
        assert "== FAIL after (" in text and text.count("refused by the slot check") == 1, text
        assert slot_refusal_in("x\nRefused by the slot check: build tool: the build slot is held by A: x, since t. More") == "the build slot is held by A: x, since t"
        assert slot_refusal_in("Refused by the slot check: game: x") == "game: x" and slot_refusal_in("FAIL x") is None

        # The release prints every line of the reply: the free space on C and the warning follow the first line.
        class Released:
            @staticmethod
            def release(slot):
                return types.SimpleNamespace(code=0, text="%s: released: x\ndisk C: 29.0 GB free\nWARNING: below 30 GB" % slot)

        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            release_build(Released)
        assert shown.getvalue() == "build: released: x\ndisk C: 29.0 GB free\nWARNING: below 30 GB\n", shown.getvalue()

        # The gate does not run on a failed build, and a failure stops no other stage.
        good, bad = [sys.executable, "-c", "pass"], [sys.executable, "-c", "raise SystemExit(4)"]
        now = os.path.join(root, "now")
        gate_stage = ("replay gate", root, good + ["--now", now])
        os.makedirs(os.path.join(now, "tour"))
        with open(os.path.join(root, "log2.txt"), "w", encoding="utf-8") as log:
            assert run_stages([("a", root, bad), ("build for the gate", root, bad), gate_stage], log) == [False, False, False]
        with open(os.path.join(root, "log2.txt"), encoding="utf-8") as log:
            text = log.read()
        assert "== replay gate:" not in text and "== NOT RUN replay gate: its build failed" in text, text
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

        # The A/B stages follow the gate, with the tips of both repos, and only with them.
        tips = {"plugin": "p" * 40, "mge": "m" * 40}
        kept = os.path.join(root, "kept")
        with_tips = stages_for("plugin", plugin, ["host/src/retained.rs"], BASELINE, tips, kept)
        assert [name for name, _, _ in with_tips][-4:] == ["build for the gate", "replay gate", BASE_STAGE, INSTANCE_STAGE]
        by_name = {name: (folder, command) for name, folder, command in with_tips}
        build = by_name[BASE_STAGE][1]
        assert build[build.index("--plugin-tip") + 1] == "p" * 40 and build[build.index("--mge-tip") + 1] == "m" * 40 and build[build.index("--out") + 1] == kept
        run = by_name[INSTANCE_STAGE][1]
        assert run[run.index("--base-build") + 1] == os.path.join(kept, "build") and run[run.index("--now-build") + 1] == os.path.join(mge, "target", "dist-staging")
        assert run[run.index("--baseline") + 1] == BASELINE and not run[run.index("--out") + 1].startswith(BASELINE)
        assert [name for name, _, _ in stages_for("plugin", plugin, ["src/lib.rs"], BASELINE, tips, kept)] == base, "no gate, no A/B stage"
        assert [name for name, _, _ in stages_for("plugin", plugin, ["host/src/retained.rs"], BASELINE, tips)][-1] == "replay gate", "no folder, no A/B stage"
        assert base_folder_of("feature/wgpu-renderer") == os.path.join(LOG_FOLDER, "base-feature-wgpu-renderer") and base_folder_of("master").endswith("base-master")
        assert tips_for("plugin", os.path.join(root, "alone", PLUGIN_NAME), "b")[0] is None
        # The sibling's side of the base is its HEAD, also when HEAD is behind its branch.
        pair = os.path.join(root, "pair")
        sibling_tree = os.path.join(pair, MGE_NAME)
        os.makedirs(sibling_tree)
        author = ["-c", "user.name=t", "-c", "user.email=t@example.com"]
        for command in (["init", "-q"], author + ["commit", "-q", "--allow-empty", "-m", "one"], author + ["commit", "-q", "--allow-empty", "-m", "two"]):
            subprocess.run(["git", "-C", sibling_tree, *command], check=True, env=git_env())
        second = resolve(sibling_tree, "HEAD")
        subprocess.run(["git", "-C", sibling_tree, "checkout", "-q", "--detach", "HEAD~1"], check=True, env=git_env())
        first = resolve(sibling_tree, "HEAD")
        assert first != second
        assert tips_for("plugin", os.path.join(pair, PLUGIN_NAME), "base") == ({"plugin": "base", "mge": first}, None)
        assert tips_for("mge", sibling_tree, "base")[1].startswith("no sibling")
        # Both A/B stages replay, so they take the whole build side.
        two = types.SimpleNamespace(take=two_places_take_for_ab)
        assert take_options(two, [("a", root, good), (INSTANCE_STAGE, root, good)]) == {"whole": True}

        # The exit code 5 of a stage is NOT RUN, with its reason, and the stage does not count as passed.
        not_run = [sys.executable, "-c", "print('replay base: exit code 0'); print('NOT RUN grass: the game slot is held by a run on Outlander-test: x'); raise SystemExit(%d)" % NOT_RUN_CODE]
        with open(os.path.join(root, "log6.txt"), "w", encoding="utf-8") as log:
            assert not run_stage("instance A/B grass", root, not_run, log)
        with open(os.path.join(root, "log6.txt"), encoding="utf-8") as log:
            text = log.read()
        assert "== NOT RUN instance A/B grass (" in text and "): the game slot is held by a run on Outlander-test: x\n" in text and "== FAIL" not in text and "== PASS" not in text, text
        assert not_run_reason("a\nb") == "the stage could not judge"

        # The A/B stage does not run on a failed build of the gate or of the base; a failed stage stops no other.
        with open(os.path.join(root, "log7.txt"), "w", encoding="utf-8") as log:
            assert run_stages([("build for the gate", root, bad), gate_stage, (BASE_STAGE, root, good), (INSTANCE_STAGE, root, good)], log) == [False, False, True, False]
        with open(os.path.join(root, "log7.txt"), encoding="utf-8") as log:
            text = log.read()
        assert "== NOT RUN instance A/B grass: its build failed" in text and "== PASS base build (" in text, text
        with open(os.path.join(root, "log8.txt"), "w", encoding="utf-8") as log:
            assert run_stages([("build for the gate", root, good), gate_stage, (BASE_STAGE, root, bad), (INSTANCE_STAGE, root, good)], log) == [True, True, False, False]
        with open(os.path.join(root, "log8.txt"), encoding="utf-8") as log:
            text = log.read()
        assert "== NOT RUN instance A/B grass: the base build failed" in text, text

        # A pass against a named folder does not move the ref.
        assert moves_the_ref(BASELINE) and not moves_the_ref(os.path.join(root, "copy"))

        # A baseline with no capture refuses a round with the replay gate before its take and builds (Rob's run
        # 8b51a23d59 failed the gate at 0 s after its builds); a round with no gate needs no baseline.
        empty, filled = os.path.join(root, "empty-baseline"), os.path.join(root, "filled-baseline")
        os.makedirs(empty)
        os.makedirs(filled)
        open(os.path.join(filled, "seyda.nircap"), "wb").close()
        gate_round = stages_for("plugin", plugin, ["host/src/retained.rs"], empty)
        problem = baseline_problem(gate_round, empty)
        assert problem and empty in problem and "no capture" in problem, problem
        assert baseline_problem(gate_round, os.path.join(root, "no-such-folder")), "a missing folder"
        assert baseline_problem(stages_for("plugin", plugin, ["host/src/retained.rs"], filled), filled) is None
        assert baseline_problem(stages_for("plugin", plugin, ["src/lib.rs"], empty), empty) is None, "no gate, no baseline"

        # A refusal for lack of room ends the wait, and a place in the queue goes on.
        class Slots:
            DONE, REFUSED, QUEUED = 0, 1, 3
            STATE_DIR = os.path.join(root, "slots")
            StateBusy = OSError
            replies = []

            @classmethod
            def take(cls, *_, **__):
                reply = cls.replies.pop(0)
                code, text = reply if isinstance(reply, tuple) else (reply, "a reply\nmore")
                return types.SimpleNamespace(code=code, text=text)

            @staticmethod
            def leave(*_):
                pass

        Slots.replies = [Slots.REFUSED]
        assert take_build(Slots, "x", False) is False
        # A take prints every line of the reply (the free space on C follows); a refusal or a wait prints the first.
        Slots.replies = [Slots.DONE]
        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            assert take_build(Slots, "x", False) is True
        assert shown.getvalue() == "a reply\nmore\n", shown.getvalue()
        Slots.replies = [Slots.REFUSED]
        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            assert take_build(Slots, "x", False) is False
        assert shown.getvalue() == "NOT RUN: the take of the build slot was refused: a reply\n", shown.getvalue()
        Slots.replies = [Slots.QUEUED]
        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            assert take_build(Slots, "x", True) is False and Slots.replies == []
        assert shown.getvalue() == "NOT RUN: --no-wait, and the build slot is not free: a reply\n", shown.getvalue()

        # Another run of this session holds the slot (2026-11-08 23:22:44, the Coordinator): this run waits for its
        # release, with one line, and then takes the slot. With --no-wait it ends with NOT RUN.
        own = (Slots.REFUSED, 'build: refused: this session holds it as "merge_step: other" (since t); a take with another label is a second run.\nRun...')
        Slots.replies = [own, own, own, Slots.DONE]
        sleeps = []
        real_sleep, time.sleep = time.sleep, sleeps.append
        try:
            shown = io.StringIO()
            with contextlib.redirect_stdout(shown):
                assert take_build(Slots, "x", False) is True and Slots.replies == []
        finally:
            time.sleep = real_sleep
        lines = shown.getvalue().splitlines()
        assert len(sleeps) == 3 and lines[0].startswith("waiting: another run of this session holds the build slot") and lines[1:] == ["a reply", "more"], lines
        Slots.replies = [own]
        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            assert take_build(Slots, "x", True) is False
        assert shown.getvalue().startswith("NOT RUN: --no-wait, and the build slot is not free: build: refused: this session holds it"), shown.getvalue()
        # The plainer form of the refusal waits alike.
        plainer = (Slots.REFUSED, 'build: refused: your session already holds the build slot as "merge_step: other" (since t); work in this session runs under that hold')
        Slots.replies = [plainer, Slots.DONE]
        real_sleep, time.sleep = time.sleep, sleeps.append
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                assert take_build(Slots, "x", False) is True and Slots.replies == []
        finally:
            time.sleep = real_sleep

        # A round with the replay gate takes the whole build side (D139); one without takes a place; a slot.py from
        # before D139 has no such option, and its take gets none.
        def two_places_take(slot, who, *, minutes=None, timing=False, whole=False):
            pass

        def one_place_take(slot, who, *, minutes=None, timing=False):
            pass

        gate = [("plugin tests", root, good), ("replay gate", root, good)]
        assert take_options(types.SimpleNamespace(take=two_places_take), gate) == {"whole": True}
        assert take_options(types.SimpleNamespace(take=two_places_take), gate[:1]) == {}
        assert take_options(types.SimpleNamespace(take=one_place_take), gate) == {}
        asked = []
        Slots.take = classmethod(lambda cls, *args, **options: asked.append(options) or types.SimpleNamespace(code=cls.DONE, text="a reply"))
        assert take_build(Slots, "x", False, {"whole": True}) is True and asked == [{"minutes": SLOT_MINUTES, "whole": True}], asked
        # The game slot's lease is longer than the build slot's.
        asked.clear()
        assert take_game(Slots, "x (game slot)", False) is True and asked == [{"minutes": GAME_SLOT_MINUTES}], asked
        assert GAME_SLOT_MINUTES > SLOT_MINUTES

        # With slot.py's LoopWaiter, a leave of the place ends the wait: the next ask returns WAIT_ENDED, and the run is
        # NOT RUN with no take (2026-11-09 07:51: a loop of plain takes took the whole side after its leave). The plain
        # take of this fake gives the slot, so a loop that does not ask through the waiter takes it.
        class Waiting(Slots):
            WAIT_ENDED = 5
            asks = []

            class LoopWaiter:
                def __init__(self, slot):
                    Waiting.asks.append(("enter", slot))

                def __enter__(self):
                    return self

                def __exit__(self, *_):
                    Waiting.asks.append(("exit",))

                def take(self, who, **_):
                    Waiting.asks.append(("ask", who))
                    return types.SimpleNamespace(code=Waiting.replies.pop(0), text="build: this wait ended without the slot: the place was left or dropped\nmore")

        Waiting.replies = [Slots.QUEUED, Waiting.WAIT_ENDED]
        real_sleep, time.sleep = time.sleep, sleeps.append
        try:
            shown = io.StringIO()
            with contextlib.redirect_stdout(shown):
                assert take_build(Waiting, "x", False) is False and Waiting.replies == []
        finally:
            time.sleep = real_sleep
        assert Waiting.asks == [("enter", "build"), ("ask", "x"), ("ask", "x"), ("exit",)], Waiting.asks
        assert shown.getvalue().splitlines()[-1] == "NOT RUN: this run left the queue of the build slot: build: this wait ended without the slot: the place was left or dropped", shown.getvalue()

        # A stage of tests fails when no test ran: cargo passes a filter that matches nothing, and a lost #[ignore] leaves 0.
        one = "running 1 test\n.\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 340 filtered out; finished in 0.1s\n"
        none = "running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 341 filtered out; finished in 0.0s\n"
        failed = "test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.1s\n"
        assert tests_ran(one) == 1 and tests_ran(none) == 0 and tests_ran(failed) == 3 and tests_ran(one + none + failed) == 4 and tests_ran("") == 0

        def prints(text, code=0):
            return [sys.executable, "-c", "import sys; sys.stdout.write(%r); sys.exit(%d)" % (text, code)]

        for output, code, wanted in ((one, 0, 0), (none, 0, 1), ("", 0, 1), (failed, 101, 101), (none, 101, 101)):
            shown = io.StringIO()
            with contextlib.redirect_stdout(shown):
                assert require_tests(prints(output, code)) == wanted, (output, code)
        assert "0 tests ran" in shown.getvalue()
        # Through the stage: the same exit codes decide the line.
        wrapper = [sys.executable, "-I", os.path.abspath(__file__), REQUIRE_TESTS]
        with open(os.path.join(root, "log9.txt"), "w", encoding="utf-8") as log:
            assert run_stage("with a test", root, wrapper + prints(one), log)
            assert not run_stage("with none", root, wrapper + prints(none), log)
        with open(os.path.join(root, "log9.txt"), encoding="utf-8") as log:
            text = log.read()
        assert "== PASS with a test (" in text and "== FAIL with none (" in text and "no test ran" in text, text

        # The game slot (the owner's D161): a round holds the build slot and the game slot for all its stages, and releases
        # both on every exit path. This uses the real slot.py on a scratch state folder, so that no slot of the day is touched.
        slots = scratch_slots(os.path.join(root, "slot-state"))
        state = slots.STATE_DIR
        assert state.startswith(root)

        def holders():
            return sorted(name for name in os.listdir(state) if name.endswith(".holder"))

        both_held = [sys.executable, "-c", "import os, sys; sys.exit(0 if os.path.exists(%r) and os.path.exists(%r) else 7)" % (slots.holder_path("game"), slots.holder_path("build"))]
        round_log = os.path.join(root, "round.log")
        shown = io.StringIO()
        with contextlib.redirect_stdout(shown):
            # The take: both slots are held while the stage runs, and the stage checks it.
            assert run_round(slots, [("both held", root, both_held)], "merge_step: x aaaaaaaaaa", round_log, True)[0] == [True]
            assert holders() == [], "a pass releases both"
            # A failing stage.
            assert run_round(slots, [("bad", root, bad), ("both held", root, both_held)], "merge_step: x bbbbbbbbbb", round_log, True)[0] == [False, True]
            assert holders() == [], "a failing stage releases both"
            # An exception in a stage (no such folder), and a Ctrl-C.
            try:
                run_round(slots, [("lost", os.path.join(root, "no-such-folder"), good)], "merge_step: x cccccccccc", round_log, True)
                raise AssertionError("the exception did not reach the caller")
            except OSError:
                pass
            assert holders() == [], "an exception releases both"
            real_run_stages = globals()["run_stages"]

            def interrupted(*_, **__):
                raise KeyboardInterrupt

            globals()["run_stages"] = interrupted
            try:
                run_round(slots, [("a", root, good)], "merge_step: x dddddddddd", round_log, True)
                raise AssertionError("the interrupt did not reach the caller")
            except KeyboardInterrupt:
                pass
            finally:
                globals()["run_stages"] = real_run_stages
            assert holders() == [], "a Ctrl-C releases both"
            # Another session holds the game slot, and the run does not wait (--no-wait): it takes nothing, and the build
            # slot that it took stays no longer than the take of the game.
            os.environ["CLAUDE_CODE_SESSION_ID"] = "merge-step-self-test-other"
            assert slots.take("game", "other: a game", minutes=5).code == slots.DONE
            os.environ["CLAUDE_CODE_SESSION_ID"] = "merge-step-self-test"
            assert run_round(slots, [("a", root, good)], "merge_step: x eeeeeeeeee", round_log, True) is None
            assert holders() == ["game.holder"], holders()
            os.environ["CLAUDE_CODE_SESSION_ID"] = "merge-step-self-test-other"
            assert slots.release("game", "other: a game").code == slots.DONE
            os.environ["CLAUDE_CODE_SESSION_ID"] = "merge-step-self-test"
        assert "merge_step: x aaaaaaaaaa (game slot)" in shown.getvalue(), shown.getvalue()
        assert "NOT RUN: --no-wait, and the game slot is not free" in shown.getvalue(), shown.getvalue()

        # A check that waits for the build side keeps its place in the game queue: it holds the free game slot at its first
        # wait, and another session that asks for the game meanwhile gets no place before it. (2026-11-09: the check waited
        # for the build side with no ask for the game, and a timing take got the game slot first.)
        own_session = os.environ["CLAUDE_CODE_SESSION_ID"]

        def as_session(name):
            os.environ["CLAUDE_CODE_SESSION_ID"] = name

        for other in ("builder-1", "builder-2"):
            as_session("merge-step-self-test-" + other)
            assert slots.take("build", "other: " + other, minutes=5).code == slots.DONE
        as_session(own_session)
        seen = []
        real_ask_in_queue = GamePlace.ask_in_queue
        real_wait_seconds = globals()["WAIT_SECONDS"]

        def ask_then_let_others_act(self):
            real_ask_in_queue(self)
            if seen:
                return
            seen.append(self.held)
            as_session("merge-step-self-test-gamer")
            seen.append(slots.take("game", "other: a game", minutes=5).code)
            # A take that got the slot (the fault under test) is released, so that the round ends and the asserts below fail.
            slots.release("game", "other: a game")
            slots.leave("game", "other: a game")
            for other in ("builder-1", "builder-2"):
                as_session("merge-step-self-test-" + other)
                assert slots.release("build", "other: " + other).code == slots.DONE
            as_session(own_session)

        GamePlace.ask_in_queue = ask_then_let_others_act
        globals()["WAIT_SECONDS"] = 0.05
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                waiting_round = run_round(slots, [("both held", root, both_held)], "merge_step: x ffffffffff", round_log, False)
        finally:
            GamePlace.ask_in_queue = real_ask_in_queue
            globals()["WAIT_SECONDS"] = real_wait_seconds
            as_session(own_session)
        assert seen, "the round did not wait for the build side: the test does not test the wait"
        assert seen[0] is True, "the check does not hold the free game slot while it waits for the build side"
        assert seen[1] != slots.DONE, "another session got the game slot before the check"
        assert waiting_round is not None and waiting_round[0] == [True], waiting_round
        assert holders() == [], holders()

        # The notes of the game slot's work: one at once, then one each period, and none after the round.
        noted = []
        counting = types.SimpleNamespace(note_active=lambda slot, now: noted.append(slot))
        with game_hold_noted(counting, 0.01):
            time.sleep(0.2)
        after = len(noted)
        time.sleep(0.1)
        assert after >= 3 and set(noted) == {"game"} and len(noted) == after, noted

        # The log keeps its name, and its folder is made.
        logs = os.path.join(root, "new", "logs")
        assert log_path_of("0123456789abcdef", logs) == os.path.join(logs, "merge-step-0123456789.log") and os.path.isdir(logs)
        assert "target" not in LOG_FOLDER.lower()
    print("merge_step self-test: ok")


def main():
    if len(sys.argv) > 2 and sys.argv[1] == REQUIRE_TESTS:
        return require_tests(sys.argv[2:])
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
    if kind and any(name == "replay gate" for name, _, _ in stages):
        tips, why = tips_for(kind, repo, base) if branch else (None, "the tree is on no branch, and the kept base build is one for each branch")
        if why:
            say("FAIL the tree: %s" % why)
            return 1
        stages = stages_for(kind, repo, paths, baseline, tips, base_folder_of(branch))
    sibling = sibling_of(kind, repo) if kind else ""
    has_sibling = os.path.isdir(sibling)
    sibling_changed = has_sibling and bool(git(sibling, "status", "--porcelain", "--untracked-files=no"))
    problem = tree_problem(kind, repo, stages, git(repo, "rev-parse", "HEAD"), commit, bool(git(repo, "status", "--porcelain", "--untracked-files=no")), sibling_changed)
    if problem:
        say("FAIL the tree: %s" % problem)
        return 1
    problem = baseline_problem(stages, baseline)
    if problem:
        say("FAIL the baseline: %s" % problem)
        return 1
    sibling_hash = git(sibling, "rev-parse", "--short=10", "HEAD") if has_sibling else "none"
    say("%s %s, sibling %s, baseline %s: %d changed paths from %s; stages: %s" % (os.path.basename(repo), commit[:10], sibling_hash, baseline, len(paths), base[:10], ", ".join(name for name, _, _ in stages)))
    if args.dry_run:
        return 0

    slots = load_slots()
    what = "merge_step: %s %s" % (os.path.basename(repo), commit[:10])
    done = run_round(slots, stages, what, log_path_of(commit), args.no_wait)
    if done is None:
        return 3
    results, summary = done
    say(summary)
    if not all(results):
        return 1
    if moves_the_ref(baseline):
        git(repo, "update-ref", passed_ref, commit)
    else:
        say("%s stays: this pass is against %s, not the accepted folder" % (passed_ref, baseline))
    return 0


if __name__ == "__main__":
    sys.exit(main())
