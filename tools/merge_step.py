"""The checks of a merge, for the Coordinator, on the merged tree: one PASS or FAIL line for each stage.

    python -I tools/merge_step.py <repo> <commit> --base <tip> [--baseline <folder>] [--sibling-base <commit>] [--no-wait] [--dry-run]
    python -I tools/merge_step.py --self-test    # the tests of the pure parts: no build, no game

<repo> is the tree of nirender-plugin or MGE-XE-G7-render-link, at <commit> and with no tracked change. There is no
trial merge: the tool refuses another tree state. It takes the build slot for the whole of its work and releases it. It
waits in the queue of that slot; with --no-wait it stops with exit code 3. --dry-run prints the stages and takes nothing.

When a stage needs the game slot (the replay gate, the instance A/B: GAME_STAGES), it also takes the game slot (a lease of 4
hours) for the whole check (the owner's D161, D183). A check with no such stage leaves the game slot free, so a game can run
beside it: none of its stages may then name a path on E: (a base build, a copy to E:\\nirender-keep), and the tool refuses
such a stage list (only its own log and closes.tsv, small appends, go to E: then). It asks for it at
each wait for the build slot, so that the check keeps its place in the game queue (it holds the game slot when that is free
first). The label is the build label plus " (game slot)". Both
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

- for nirender-plugin, "mwse sites <instance>": tools/check_mwse_sites.py on the MWSE.dll deployed in each test install
  (Morrowind75testing, Outlander-test), when the change touches the ports, the hooks, the install, the function table,
  the call sites or the baseline of the check (Brief 22): a new port over an MWSE patch that is already deployed fails;
- a d3d8 build and check_contract.py --mge when the change touches d3d8, mge-config or contract/;
- the replay gate, tools/validate_retained.py baseline with no game, when the change touches the host's source, the
  helper's renderer, the shaders of the assets (the source of the shaders in a build), or a manifest or a lock of the
  build. The path decides: a stream or a table change can move a picture too. The gate does not run when its build failed;
- with the gate, the A/B stages of tools/instance_ab.py: `base build` keeps the build of the current tips of the target
  branches of BOTH repos in E:\\nirender-keep\\merge-step\\base-<branch> (key: both commits and the profile; a tip that
  moved builds again), and `instance A/B grass` replays a capture of Outlander-test with the base, the build under test and
  the base again, on the same data, and compares the pictures and the hash lines of the grass calls. NOT RUN is not a PASS:
  it counts as a stage that did not pass (exit code 5 of the tool). The base is built at <tip> for the tree's repo and at
  the sibling's commit that passed with <tip>: the pair as it passed, so the A/B shows the change of both repos since that
  pass. The sibling's HEAD is not the base, as it can hold code that <tip> lacks (a render-link that needs a newer nirender
  fails the base build). A pass (against the accepted folder) keeps its pair in the sibling's repo, in the ref
  refs/merge-step-pair/<plugin|mge>/<commit>, when the sibling's HEAD did not move during the round. A base with no kept
  pair (a pass from before these refs) stops the tool: --sibling-base <commit> names the sibling's commit. It also takes the
  place of a kept pair. The sibling's commit must be an ancestor of the sibling's HEAD. The tool prints the option in the
  first line, and the key file of the base build names both commits;
- the build of the dist alone, `cargo xtask dist`, when the change touches the mod's Lua (xtask checks it against the list
  of the files it ships) or xtask, and not the paths of the gate: the replay cannot see these;
- with the gate, `deploy rest` (tools/deploy_rest.py, the owner's D193: the test installs hold the latest build): after
  the build and before the replay gate it puts the dist-staging folder into both test installs as their resting build, and
  keeps a copy as the next E:\\nirender-keep\\mgebuilds\\f<N> with a WHAT.txt. It runs only when every stage before it
  passed, and the gate's result does not stop it: a build that the gate rejects is deployed too. It never copies a pdb, an
  esp, mgeXE.toml or d3d9.dll, backs up each replaced file once for each tag, compares each copy by MD5, and prints the MD5
  of d3d8.dll, numidium.exe and akulakhan.dll. While Morrowind, numidium or Mod Organizer runs, the stage FAILs with a
  line "refused: ..." and does not wait. The round holds the game slot (the gate is one of its stages), so that the write
  to E: is allowed (evidence_problem);

These stages also need a sibling with no tracked change. The first line gives the hash of the sibling.

--baseline names the folder that the gate compares with: the accepted folder without it, or a copy in the same form, as a
reference and not an acceptance. The first line names it. A pass against a named folder does not move the ref. The gate
writes nothing in it: its pictures go to <plugin>/target/merge-step-now, which each gate of this tool clears first, under the
build slot, so that a stale picture cannot compare.

All stages run, also after a failure. Exit code 1 on any failure. The output of each stage, the PASS, FAIL or NOT RUN line of
each stage and the closing "N of N stages passed" line are in
E:\\nirender-keep\\merge-step\\merge-step-<commit>.log, where the data rule on build output does not reach. A rerun for the same
commit writes merge-step-<commit>-2.log, then -3, and so on: it never overwrites the log of an earlier round. It pushes nothing.

A pass of the merge (every stage passed, against the accepted folder) closes the items that the merge subjects name (D180):
a subject that ends with "(W<n>)" or "(W<n> <part>)", e.g. "(W96)" or "(W234 a)". The subjects are those of each merge
commit on the first-parent path from <tip> to <commit> and of <commit> itself, so one check of a batch of merges closes
each of them (D183). A path with no merge commit with such a subject, to a <commit> that is no merge, gets a WARNING line
before the round: a fast-forward closes only the item of <commit>. Each item appends one line to
E:\\nirender-keep\\merge-step\\closes.tsv, which the Archivist reads in its batch. The columns are time (ISO, with the offset),
item, repo, commit, stages ("6 of 6"), log and subject; the file starts with a header line. A failed stage writes nothing, nor
does any other subject. A rerun of the same commit adds no line for an item that the file has for it. A reader that holds the
file makes the append try again, then give up with a line: a missing close never fails the check.

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
import datetime
import importlib.util
import inspect
import io
import itertools
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
from instance_ab import NOT_RUN as NOT_RUN_CODE, base_key  # noqa: E402
from common import git_env, local  # noqa: E402
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
# The closing lines of the items (D180): the Archivist reads this file in its batch. A pass of the merge appends one line for
# each item that its merge subject names.
CLOSES_FILE = os.path.join(LOG_FOLDER, "closes.tsv")
CLOSES_HEADER = "time\titem\trepo\tcommit\tstages\tlog\tsubject"
# The item of a merge subject: the subject ends with "(W<n>)" or "(W<n> <part>)", the form of the Coordinator's merge
# subjects ("(W96)", "(W234 a)"). Any other subject names no item and closes nothing. A part that starts with "dropped" is
# no part: master has "Merge: log pipe follow-ups W219 and W221 (W220 dropped)".
ITEM = re.compile(r"\((W\d+)(?: (?!dropped\b)[^()]+)?\)$")
# The tries of an append to the closes file while a reader holds it, and the wait between them.
CLOSE_TRIES = 5
CLOSE_WAIT_SECONDS = 0.5
# The stages that replay the build of the gate or the kept base build.
INSTANCE_STAGE = "instance A/B grass"
BASE_STAGE = "base build"
# The stage that puts the build of the gate into both test installs (tools/deploy_rest.py, the owner's D193). It stands after
# the build and before the replay gate, and it runs only when every stage before it passed: the gate's result does not stop it.
DEPLOY_STAGE = "deploy rest"
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
# What a plugin change must touch for the check of MWSE's patch sites against the ports (check_mwse_sites.py, Brief 22):
# a new port, hook or follow rule can meet an MWSE patch that is already deployed.
MWSE_SITES_PATHS = ("src/ports/", "src/hook.rs", "src/install.rs", "src/gen/", "src/site.rs", "tools/mwse_sites.baseline")
# The MWSE.dll that each test install has deployed (the line build), by install: the stage checks these. Never a
# `Harness - *` mod: a live run puts its own MWSE.dll there for its game hold, and the stage takes no game slot.
# The key "mwse_deploys" of local.json changes the places.
MWSE_DEPLOYS = local("mwse_deploys", {
    "Morrowind75testing": "D:/Modlists/Morrowind75testing/mods/MWSE/Root/MWSE.dll",
    "Outlander-test": "D:/Modlists/Outlander-test/mods/MWSE-Update for Nightly Builds/Root/MWSE.dll",
})
HARNESS_MOD = re.compile(r"[/\\]mods[/\\]Harness - [^/\\]*[/\\]", re.IGNORECASE)
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


def deployed_mwse_dlls(deploys=None):
    """The deployed MWSE.dll of each test install, as (install name, path), from `MWSE_DEPLOYS`. A missing file stays in
    the list, so that its stage fails and names it. A path in a `Harness - *` mod is refused: that is a run's DLL."""
    deploys = MWSE_DEPLOYS if deploys is None else deploys
    for name, path in deploys.items():
        if HARNESS_MOD.search(path):
            raise ValueError("mwse sites %s: %s is in a Harness mod, the DLL of a live run, not the deployed one" % (name, path))
    return list(deploys.items())


def stages_for(kind, repo, paths, baseline=BASELINE, tips=None, base_folder=None, mwse_dlls=None):
    """The stages of a merge, as (name, folder, command). The folder of the other repo is the sibling. `tips` are the
    commits of the base build, {"plugin": sha, "mge": sha}, and `base_folder` holds it: with both, the gate is followed by
    the A/B stages. `mwse_dlls` are the (instance, MWSE.dll) of the "mwse sites" stages, by default those deployed now."""
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
    if kind == "plugin" and touches(paths, MWSE_SITES_PATHS):
        for instance, dll in deployed_mwse_dlls() if mwse_dlls is None else mwse_dlls:
            stages.append(("mwse sites " + instance, plugin, tool("check_mwse_sites.py", dll)))
    if touches(paths, D3D8_PATHS[kind]):
        stages.append(("d3d8 build", mge, ["cargo", "build", "--quiet", "-p", "d3d8", "--profile", "release-dll"]))
        stages.append(("contract", plugin, tool("check_contract.py", "--mge", mge)))
    if touches(paths, GATE_PATHS[kind]):
        stages.append(("build for the gate", mge, ["cargo", "xtask", "dist"]))
        staging = os.path.join(mge, "target", "dist-staging")
        stages.append((DEPLOY_STAGE, plugin, tool("deploy_rest.py", "--build", staging, "--plugin-repo", plugin, "--mge-repo", mge)))
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


def deploy_refusal_in(output):
    """The text of the last line 'REFUSED: <why>' in a stage's output (tools/deploy_rest.py: a game or Mod Organizer runs), or None."""
    refusal = None
    for line in output.splitlines():
        if line.startswith("REFUSED: "):
            refusal = line[len("REFUSED: "):].strip()
    return refusal


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
        output = output_since(log, start)
        refusal, deploy_refusal = slot_refusal_in(output), deploy_refusal_in(output)
        failure = ": exit code %d%s" % (code, "; refused by the slot check: %s" % refusal if refusal else "; refused: %s" % deploy_refusal if deploy_refusal else "")
    verdict(log, "%s %s (%d s)%s" % ("PASS" if code == 0 else "FAIL", name, time.time() - began, failure))
    return code == 0


def log_path_of(commit, folder=LOG_FOLDER):
    """The file of the output of a round, in a folder that exists. A round never writes into the log of an earlier one: the
    first round of a commit has `merge-step-<commit>.log`, the next ones `merge-step-<commit>-2.log` and so on (2026-11-09:
    a rerun overwrote the log of a refused round). The file is made here, empty, so that two rounds cannot take one name."""
    os.makedirs(folder, exist_ok=True)
    for number in itertools.count(1):
        path = os.path.join(folder, "merge-step-%s%s.log" % (commit[:10], "" if number == 1 else "-%d" % number))
        try:
            with open(path, "x", encoding="utf-8"):
                return path
        except FileExistsError:
            continue


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
    no compile seen, and a round can pass that with a stage that does not compile. It returns False when the holds could
    not be taken again: the stage is then NOT RUN, and counts as not passed."""
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
        if name == DEPLOY_STAGE and not all(results.values()):
            verdict(log, "NOT RUN %s: %s did not pass, and a build is put into the test installs only when every stage before it passed" % (name, next(stage for stage, passed in results.items() if not passed)))
            results[name] = False
            continue
        if name == "replay gate":
            clear_pictures(command[command.index("--now") + 1])
        if note_stage() is False:
            verdict(log, "NOT RUN %s: the slot hold of this run ended and was not taken again" % name)
            results[name] = False
            continue
        results[name] = run_stage(name, folder, command, log)
    return list(results.values())


def summary_line(results, log_path):
    return "%d of %d stages passed; the output is in %s" % (sum(results), len(results), log_path)


def moves_the_ref(baseline):
    """Only a pass against the accepted folder is a pass of the merge."""
    return os.path.normcase(os.path.abspath(baseline)) == os.path.normcase(os.path.abspath(BASELINE))


def items_of(subject):
    """The item that a merge subject names at its end, as a list of one, or no item."""
    match = ITEM.search(subject.strip())
    return [match.group(1)] if match else []


def closing_commits(entries, tip):
    """The commits of a check that can close an item, oldest first: each merge on the first-parent path, and the checked
    commit itself. `entries` are (commit, parents, subject) of that path, oldest first, as `first_parent_path` gives them.
    A batch of merges with one check (the owner's D183) closes each merge's item, not only the tip's. A commit that is no
    merge closes nothing unless it is the checked commit, and a commit of a merged branch is not on the path."""
    return [(commit, subject) for commit, parents, subject in entries if len(parents.split()) > 1 or commit == tip]


def first_parent_path(repo, base, commit):
    """(commit, parents, subject) of each commit on the first-parent path from `base` (left out) to `commit`, oldest first."""
    out = git(repo, "log", "--first-parent", "--reverse", "--format=%H%x09%P%x09%s", "%s..%s" % (base, commit))
    return [tuple(line.split("\t", 2)) for line in out.splitlines() if line.count("\t") >= 2]


def fast_forward_warning(entries, tip):
    """A warning line when the first-parent path holds no merge commit with an item and the checked commit is no merge, or
    None. A fast-forward of a batch leaves no merge commit, and then only the tip's item closes (the Coordinator merges
    with --no-ff, D183)."""
    def is_merge(parents):
        return len(parents.split()) > 1

    if any(is_merge(parents) and items_of(subject) for _, parents, subject in entries):
        return None
    if any(commit == tip and is_merge(parents) for commit, parents, _ in entries):
        return None
    return ("WARNING: base..%s holds no merge commit with a (W<n>) subject, and %s is no merge: a fast-forward closes only "
            "the item of the checked commit (merge with --no-ff)" % (tip[:10], tip[:10]))


def closing_merges(repo, base, commit):
    """The (commit, subject) of a check of base..commit that can close an item, oldest first, and the fast-forward warning
    of that path (or None)."""
    entries = first_parent_path(repo, base, commit)
    return closing_commits(entries, commit), fast_forward_warning(entries, commit)


def close_lines(existing, when, repo_name, merges, stages, log_path):
    """The closing lines of a pass: one for each (commit, subject) of `merges` whose subject names an item that the lines
    `existing` of the closes file do not have for that commit. A rerun of a commit adds nothing; a later commit of the same
    item (its part b) adds a line."""
    have = set()
    for line in existing:
        columns = line.rstrip("\r\n").split("\t")
        if len(columns) >= 4:
            have.add((columns[1], columns[3]))
    lines = []
    for commit, subject in merges:
        commit, subject = commit[:10], subject.replace("\t", " ")
        for item in items_of(subject):
            if (item, commit) not in have:
                have.add((item, commit))
                lines.append("\t".join((when, item, repo_name, commit, stages, log_path, subject)))
    return lines


def write_closes(path, when, repo_name, merges, stages, log_path, tries=CLOSE_TRIES, wait=CLOSE_WAIT_SECONDS):
    """Appends the closing lines of a pass to `path` (CRLF, ASCII), with the header first when the file is new. The open
    is a plain append, which leaves the file to its readers. A reader that holds the file makes a PermissionError: the
    append is tried again, then given up with a line, as a missing close must not fail the check. The lines written, or
    None when the append was given up."""
    for attempt in range(tries):
        try:
            existing = []
            if os.path.exists(path):
                with open(path, encoding="ascii", errors="replace") as closes:
                    existing = closes.readlines()
            lines = close_lines(existing, when, repo_name, merges, stages, log_path)
            if lines:
                with open(path, "a", encoding="ascii", errors="replace", newline="\r\n") as closes:
                    if not existing:
                        closes.write(CLOSES_HEADER + "\n")
                    closes.write("".join(line + "\n" for line in lines))
            return lines
        except PermissionError as error:
            if attempt + 1 == tries:
                say("closes: NOT WRITTEN to %s after %d tries (%s); the check stands, write the line by hand" % (path, tries, error))
                return None
            time.sleep(wait)


def close_after(results, baseline, merges, write):
    """The closing lines of a round, through `write(merges, stages)`: only when every stage passed and the pass is
    against the accepted folder, as only that is a pass of the merge. None when the round writes no close."""
    if not results or not all(results) or not moves_the_ref(baseline):
        return None
    return write(merges, "%d of %d" % (sum(results), len(results)))


def resolve(repo, name):
    """The commit of a name, or None. A name of a tree or a blob is no commit; a tag gives the commit it points to."""
    return subprocess.run(["git", "-C", repo, "rev-parse", "--verify", "-q", name + "^{commit}"], capture_output=True, text=True, env=git_env()).stdout.strip() or None


def branch_of(repo):
    """The branch of the tree, or None."""
    return subprocess.run(["git", "-C", repo, "symbolic-ref", "-q", "--short", "HEAD"], capture_output=True, text=True, env=git_env()).stdout.strip() or None


def is_ancestor(repo, older, newer):
    return subprocess.run(["git", "-C", repo, "merge-base", "--is-ancestor", older, newer], env=git_env()).returncode == 0


def take_options(slots, stages):
    """The options of the take: the whole build side (both places) when a stage replays, as a replay needs it (the
    owner's D139). A slot.py from before D139 has one place, and its take has no such option."""
    replays = any(name in GAME_STAGES for name, _, _ in stages)
    if replays and "whole" in inspect.signature(slots.take).parameters:
        return {"whole": True}
    return {}


# The stages that need the game slot (the owner's D183): the replay gate replays on the GPU, and the instance A/B replays
# a capture of the game with three builds. A new stage that starts the game or times the GPU belongs here. A check with
# none of them leaves the game slot to the game runs; before D183 every check held it (D161), and the workers waited.
GAME_STAGES = ("replay gate", INSTANCE_STAGE)


def needs_game(stages):
    return any(name in GAME_STAGES for name, _, _ in stages)


# A path on the evidence disk E:, as a whole argument or after "=" (--out=E:/x): E: with any rest (E:x is relative to the
# folder of E:), or /e/ of Git Bash.
EVIDENCE_PATH = re.compile(r"(?i)(?:^|=)(?:e:|/e/)")


def evidence_problem(stages):
    """Why these stages may not run as one round, as a text, or None. A round with no stage of GAME_STAGES leaves the game
    slot free, so a game can run beside it, and none of its stages may then write to E: (the Coordinator's D183): a base
    build or a copy to E:\\nirender-keep belongs in a round that holds the game slot. A stage counts as a writer when its
    folder or an argument of its command is a path on E:. The log and the closes file of this tool are small appends,
    and they stay allowed."""
    if needs_game(stages):
        return None
    for name, folder, command in stages:
        on_e = [text for text in (folder, *command) if EVIDENCE_PATH.search(str(text))]
        if on_e:
            return "the stage %r names %s on E:, and no stage of the round holds the game slot (%s)" % (name, on_e[0], ", ".join(GAME_STAGES))
    return None


class NoGame:
    """The game place of a round whose stages need no game slot (D183): it asks for nothing, holds nothing and releases
    nothing, so that the holds of the build side work as with a `GamePlace`."""
    held = False
    retakes = 0

    def ask_in_queue(self):
        pass

    def take(self, no_wait):
        return True

    def keep(self, no_wait, log):
        return True

    def release(self):
        pass


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
        self.retakes = 0

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
            say("game held for the merge check, waiting for the build side since %s: %s" % (time.strftime("%H:%M"), reply.text.splitlines()[0]))

    def take(self, no_wait):
        """Takes the game slot for the whole check, when a stage needs it (the owner's D161, D183): such a stage replays on
        the GPU, and a game that starts in that time competes with it."""
        if self.held:
            return True
        self.held = take_slot(self.slots, "game", self.label, no_wait, minutes=GAME_SLOT_MINUTES, ask=self.ask)
        return self.held

    def keep(self, no_wait, log):
        """True when the run still holds the game slot, or took it again: the hold lapses after 15 minutes with no game seen,
        and the notes of its work (`ask_in_queue`, `game_hold_noted`) can stop for as long as a take of the build side waits.
        A run that held the game slot holds it for the whole check, so a lapsed hold is taken again, in the queue. A hold
        that the state cannot show now counts as kept."""
        if not self.held or places_held(self.slots, "game", self.label) != 0:
            return True
        self.held = False
        verdict(log, "the game slot hold of this run ended (it lapsed); taking it again")
        if not self.take(no_wait):
            return False
        self.retakes += 1
        verdict(log, "re-took the game slot at %s" % time.strftime("%H:%M"))
        return True

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


def places_held(slots, slot, who):
    """How many places of a slot the run `who` holds, or None when the state cannot be read now (a hold is then taken as
    kept: the next stage asks again). The build side has two places; the game slot has one."""
    read_record = getattr(slots, "read_record", None)
    if read_record is None:
        return None
    owner = slots.owner_of(slots.session_id(), who)
    count = 0
    for unit in slots.units_of(slot):
        try:
            holder = read_record(slots.holder_path(unit), slots.Holder)
        except slots.StateError:
            return None
        count += holder is not None and holder.session == owner and holder.who == who
    return count


# How often `keep_holds` looks at both slots again after a take that waited: a wait for one can lapse the other.
KEEP_ROUNDS = 5


def keep_holds(slots, what, game, options, no_wait, log):
    """Before a stage: the run still holds the build side and the game slot, and takes again one that lapsed. A wait does
    not stop the idle lapse of the hold that the waiting run already has (slot.py): a take of the game that waited 34
    minutes lost the build side after 15 (2026-11-09, the W206 render-link check), and the replay gate and the instance
    A/B were then refused by the slot check. The take again waits in the queue like the first take, keeps the run's place
    in the game queue meanwhile, and logs one line for each take. False when a take again did not give the slot."""
    needed = len(slots.units_of("build")) if options.get("whole") else 1
    owner = slots.owner_of(slots.session_id(), what)
    for _ in range(KEEP_ROUNDS):
        # The build work of the places of this run, as the hook and the tool checks of slot.py note it: a hold of another run
        # gets no note.
        slots.note_holder_build(owner, time.time())
        lost = False
        held = places_held(slots, "build", what)
        if held is not None and held < needed:
            lost = True
            verdict(log, "the build hold of this run ended (it lapsed; %d of %d place(s) held); taking it again" % (held, needed))
            if not take_build(slots, what, no_wait, options, while_waiting=game.ask_in_queue):
                return False
            verdict(log, "re-took the build side at %s" % time.strftime("%H:%M"))
        game_takes = game.retakes
        if not game.keep(no_wait, log):
            return False
        # A take that waited can have lapsed the other hold: look at both again.
        if not lost and game.retakes == game_takes:
            break
    return True


def run_round(slots, stages, what, log_path, no_wait):
    """The stages of one round with the build slot held, and the game slot when a stage needs it (`needs_game`), both
    released on every exit path: a failing stage, an exception and Ctrl-C. (results, summary), or None when a take did not
    give a slot. A run that needs the game asks for it at each wait for the build side (see `GamePlace`), so that it keeps
    its place in the game queue."""
    problem = evidence_problem(stages)
    if problem:
        raise ValueError("merge_step refuses the round: %s" % problem)
    game_needed = needs_game(stages)
    if not game_needed:
        say("the stages need no game slot: the game stays free for the game runs (D183)")
    with slot_asker(slots, "game") if game_needed else contextlib.nullcontext() as game_ask:
        game = GamePlace(slots, GAME_LABEL % what, game_ask) if game_needed else NoGame()
        try:
            options = take_options(slots, stages)
            if not take_build(slots, what, no_wait, options, while_waiting=game.ask_in_queue):
                return None
            try:
                if not game.take(no_wait):
                    return None
                with game_hold_noted(slots) if game_needed else contextlib.nullcontext(), open(log_path, "w", encoding="utf-8", errors="replace") as log:
                    results = run_stages(stages, log, lambda: keep_holds(slots, what, game, options, no_wait, log))
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
    done = subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True, env=git_env())
    if done.returncode != 0:
        raise SystemExit("git %s in %s: %s" % (" ".join(args), repo, done.stderr.strip()))
    return done.stdout.strip()


def base_folder_of(branch):
    """The folder of the kept base build of a target branch: one for each branch, replaced at each pass."""
    return os.path.join(LOG_FOLDER, "base-" + branch.replace("/", "-"))


PAIR_REFS = "refs/merge-step-pair"


def pair_ref(kind, commit):
    """The ref that keeps the sibling's commit of the last pass of `commit`, a commit of the repo of `kind`: the pair that
    passed. It is a ref of the sibling's repo, as only that repo has the commit it names."""
    return "%s/%s/%s" % (PAIR_REFS, kind, commit)


def keep_pair(kind, sibling, commit, sibling_commit):
    """Keeps the pair of a pass: pair_ref(kind, commit) in the sibling's repo names `sibling_commit`."""
    git(sibling, "update-ref", pair_ref(kind, commit), sibling_commit)


def tips_for(kind, repo, base, sibling_base=None):
    """The commits of the base build: `base` for the tree's repo, and for the sibling the commit that passed with `base`,
    which the pass kept in pair_ref(kind, base). The sibling's HEAD is not the base: it can hold code that `base` lacks
    (2026-11-10, a render-link tip that needs W84 with the plugin's last pass: "no field `like_exterior`"; 2026-11-09,
    nirender 9f68cd1 with a render-link that calls the spot cone). `sibling_base` names the sibling's commit in place of the
    kept one, for a base with no kept pair. The sibling's commit must be an ancestor of the sibling's HEAD (or HEAD
    itself). (tips, None), or (None, why not)."""
    other = "mge" if kind == "plugin" else "plugin"
    sibling = sibling_of(kind, repo)
    if not os.path.isdir(sibling):
        return None, "no sibling %s for the base build" % sibling
    sibling_head = resolve(sibling, "HEAD")
    if sibling_head is None:
        return None, "the sibling %s has no HEAD for the base build" % sibling
    if sibling_base is None:
        source = "the kept pair %s" % pair_ref(kind, base)
        sibling_commit = resolve(sibling, pair_ref(kind, base))
        if sibling_commit is None:
            return None, "the sibling %s keeps no pair for the base %s (%s): give --sibling-base, the sibling's commit that passed with the base" % (sibling, base[:10], pair_ref(kind, base))
    else:
        source = "--sibling-base %s" % sibling_base
        sibling_commit = resolve(sibling, sibling_base)
        if sibling_commit is None:
            return None, "--sibling-base %s is no commit of the sibling %s" % (sibling_base, sibling)
    if not is_ancestor(sibling, sibling_commit, sibling_head):
        return None, "%s is not an ancestor of the HEAD of the sibling %s (%s)" % (source, sibling, sibling_head[:10])
    return {kind: base, other: sibling_commit}, None


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
    # The names match no process, so the test's process list is empty. The real list can fail: tasklist has 5 s, a busy
    # machine can pass that, and with no list an idle hold does not lapse (case 6 failed so once).
    module.running_images = lambda: []
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
        assert names("plugin", plugin, "host/src/shader.wgsl") == base + ["build for the gate", DEPLOY_STAGE, "replay gate"]
        assert names("plugin", plugin, "host/src/material.rs") == base + ["build for the gate", DEPLOY_STAGE, "replay gate"]
        assert names("plugin", plugin, "host/src/retained.rs") == base + ["build for the gate", DEPLOY_STAGE, "replay gate"]
        assert names("plugin", plugin, "host/src_old/x.rs") == base, "a name that only starts the same ran the gate"
        assert names("plugin", plugin, "contract/mge_render_link.h") == base + ["d3d8 build", "contract"]
        # A change of the ports, the hooks, the install or the table: the MWSE.dll of each test install against the ports.
        dlls = [("Morrowind75testing", "D:/m75/MWSE.dll"), ("Outlander-test", "D:/o/MWSE.dll")]
        for path in ("src/ports/device_setup.rs", "src/hook.rs", "src/install.rs", "src/gen/funcs.rs", "src/site.rs", "tools/mwse_sites.baseline"):
            stages = stages_for("plugin", plugin, [path], mwse_dlls=dlls)
            assert [name for name, _, _ in stages] == base + ["mwse sites Morrowind75testing", "mwse sites Outlander-test"], path
            assert stages[-1][2][-2:] == [os.path.join("tools", "check_mwse_sites.py"), "D:/o/MWSE.dll"] and stages[-1][1] == plugin
        assert [name for name, _, _ in stages_for("plugin", plugin, ["src/lib.rs"], mwse_dlls=dlls)] == base
        assert [name for name, _, _ in stages_for("mge", mge, ["src/hook.rs"], mwse_dlls=dlls)] == ["helper tests", GPU_STAGE, "clippy baseline", "checks", "docs"]
        # The deployed DLL, never the copy of a live run in a Harness mod (2026-11-10: the stage read Ryker's Brief 6 DLL,
        # which sorts before mods/MWSE).
        assert not any(HARNESS_MOD.search(path) for path in MWSE_DEPLOYS.values()), MWSE_DEPLOYS
        assert [name for name, _ in deployed_mwse_dlls()] == list(MWSE_DEPLOYS)
        deploys = {"A": "D:/m/A/mods/MWSE/Root/MWSE.dll", "B": "D:/m/B/mods/missing/Root/MWSE.dll"}
        assert deployed_mwse_dlls(deploys) == list(deploys.items()), "a missing deploy left the list: its stage did not run"
        for harness in ("D:/m/A/mods/Harness - nirender/Root/MWSE.dll", "D:\\m\\A\\mods\\harness - other\\Root\\MWSE.dll"):
            try:
                deployed_mwse_dlls({"A": harness})
            except ValueError as error:
                assert "Harness mod" in str(error), error
            else:
                raise AssertionError("a Harness copy was read: " + harness)
        # The mod's Lua: xtask dist checks it against its list of shipped files (7b7b72d broke the dist that way).
        assert names("plugin", plugin, "mod/MWSE/mods/nirender/main.lua") == base + ["build of the dist"]
        assert names("plugin", plugin, "mod/MWSE/mods/nirender/main.lua", "host/src/retained.rs") == base + ["build for the gate", DEPLOY_STAGE, "replay gate"]
        assert needs_clean_sibling(stages_for("plugin", plugin, ["mod/MWSE/mods/nirender/noplugin.lua"]))
        for path in ("Cargo.toml", "Cargo.lock", "host/Cargo.toml", "host/Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml", "host/.cargo/config.toml"):
            assert names("plugin", plugin, path) == base + ["build for the gate", DEPLOY_STAGE, "replay gate"], path
        mge_base = ["helper tests", GPU_STAGE, "clippy baseline", "checks", "docs"]
        assert names("mge", mge, "xtask/src/main.rs") == mge_base + ["build of the dist"]
        for path in ("Cargo.toml", "Cargo.lock", "mgeHost64/Cargo.toml", "rust-toolchain.toml", ".cargo/config.toml"):
            assert names("mge", mge, path) == mge_base + ["build for the gate", DEPLOY_STAGE, "replay gate"], path
        assert names("mge", mge, "mge-config/Cargo.toml") == mge_base + ["d3d8 build", "contract", "build for the gate", DEPLOY_STAGE, "replay gate"]
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
        assert names("mge", mge, "mgeHost64/src/renderer/water.rs") == mge_base + ["build for the gate", DEPLOY_STAGE, "replay gate"]
        assert names("mge", mge, "mgeHost64/src/renderer.rs") == mge_base + ["build for the gate", DEPLOY_STAGE, "replay gate"]
        assert names("mge", mge, "assets/Data Files/shaders/core/x.fx") == mge_base + ["build for the gate", DEPLOY_STAGE, "replay gate"]
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

        # The deploy of the build (D193) stands between the build and the gate. It runs when every stage before it passed,
        # whatever the gate says; it does not run after a failed code stage, and a failed deploy stops no other stage.
        deploy_stage = (DEPLOY_STAGE, root, good)
        with open(os.path.join(root, "log5.txt"), "w", encoding="utf-8") as log:
            assert run_stages([("a", root, good), ("build for the gate", root, good), deploy_stage, ("replay gate", root, bad + ["--now", now])], log) == [True, True, True, False]
            assert run_stages([("a", root, good), ("build for the gate", root, good), (DEPLOY_STAGE, root, bad), gate_stage], log) == [True, True, False, True]
            assert run_stages([("a", root, bad), ("build for the gate", root, good), deploy_stage, gate_stage], log) == [False, True, False, True]
            assert run_stages([("a", root, good), ("build for the gate", root, bad), deploy_stage, gate_stage], log) == [True, False, False, False]
        with open(os.path.join(root, "log5.txt"), encoding="utf-8") as log:
            text = log.read()
        assert "== NOT RUN deploy rest: a did not pass" in text and "== NOT RUN deploy rest: build for the gate did not pass" in text, text
        gate_names = [name for name, _, _ in stages_for("plugin", plugin, ["host/src/retained.rs"])]
        assert gate_names.index("build for the gate") + 1 == gate_names.index(DEPLOY_STAGE) == gate_names.index("replay gate") - 1
        assert DEPLOY_STAGE not in [name for name, _, _ in stages_for("plugin", plugin, ["mod/MWSE/mods/nirender/main.lua"])], "a build of the dist alone is not deployed"
        assert DEPLOY_STAGE not in [name for name, _, _ in stages_for("plugin", plugin, ["src/lib.rs"])]
        deploy = next(stage for stage in stages_for("plugin", plugin, ["host/src/retained.rs"]) if stage[0] == DEPLOY_STAGE)
        assert deploy[1] == plugin and "--build" in deploy[2] and "dist-staging" in deploy[2][deploy[2].index("--build") + 1], deploy
        assert not any(EVIDENCE_PATH.search(str(text)) for text in deploy[2]), "the stage names no path on E: (the tool keeps its set folder itself)"
        assert deploy_refusal_in("x\nREFUSED: Morrowind.exe runs, and the deploy does not wait: end it\nFAIL: y") == "Morrowind.exe runs, and the deploy does not wait: end it"
        assert deploy_refusal_in("FAIL: x") is None

        # Each stage that runs notes the slot's work, and a gate that does not run does not.
        notes = []
        with open(os.path.join(root, "log4.txt"), "w", encoding="utf-8") as log:
            run_stages([("a", root, good), ("build for the gate", root, bad), gate_stage], log, lambda: notes.append(1))
        assert len(notes) == 2

        # The A/B stages follow the gate, with the tips of both repos, and only with them.
        tips = {"plugin": "p" * 40, "mge": "m" * 40}
        kept = os.path.join(root, "kept")
        with_tips = stages_for("plugin", plugin, ["host/src/retained.rs"], BASELINE, tips, kept)
        assert [name for name, _, _ in with_tips][-5:] == ["build for the gate", DEPLOY_STAGE, "replay gate", BASE_STAGE, INSTANCE_STAGE]
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
        # The sibling's side of the base is the pair that passed with the base, kept in the sibling's repo.
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
        plugin_of_pair = os.path.join(pair, PLUGIN_NAME)
        no_pair = tips_for("plugin", plugin_of_pair, "base")
        assert no_pair[0] is None and "keeps no pair for the base" in no_pair[1] and "--sibling-base" in no_pair[1], "no kept pair, no option: no base build"
        keep_pair("plugin", sibling_tree, "base", first)
        assert resolve(sibling_tree, pair_ref("plugin", "base")) == first
        assert tips_for("plugin", plugin_of_pair, "base") == ({"plugin": "base", "mge": first}, None), "the kept pair, HEAD itself"
        assert tips_for("mge", sibling_tree, "base")[1].startswith("no sibling")
        # --sibling-base: the sibling's side of the base is that commit, when it is HEAD or an ancestor of HEAD.
        assert tips_for("plugin", plugin_of_pair, "base", first) == ({"plugin": "base", "mge": first}, None), "HEAD itself"
        subprocess.run(["git", "-C", sibling_tree, "checkout", "-q", "--detach", second], check=True, env=git_env())
        subprocess.run(["git", "-C", sibling_tree, *author, "commit", "-q", "--allow-empty", "-m", "three"], check=True, env=git_env())
        third = resolve(sibling_tree, "HEAD")
        # The fault of 2026-11-10: the sibling's HEAD moved on after the pass of the base; the base stays at the kept pair.
        assert tips_for("plugin", plugin_of_pair, "base") == ({"plugin": "base", "mge": first}, None), "without the option: the kept pair, not the sibling's HEAD"
        assert tips_for("plugin", plugin_of_pair, "base", second) == ({"plugin": "base", "mge": second}, None), "an ancestor of HEAD, in place of the kept pair"
        assert tips_for("plugin", plugin_of_pair, "base", second[:10]) == ({"plugin": "base", "mge": second}, None), "a short name gives the full commit"
        assert base_key("base", second) != base_key("base", third), "the key names the sibling's commit"
        subprocess.run(["git", "-C", sibling_tree, "checkout", "-q", "--detach", first], check=True, env=git_env())
        assert tips_for("plugin", plugin_of_pair, "base", second)[1].endswith("(%s)" % first[:10]) and "not an ancestor" in tips_for("plugin", plugin_of_pair, "base", second)[1], "a commit ahead of HEAD"
        keep_pair("plugin", sibling_tree, "later", second)
        assert tips_for("plugin", plugin_of_pair, "later")[1].startswith("the kept pair %s is not an ancestor" % pair_ref("plugin", "later")), "a kept pair ahead of HEAD"
        assert "is no commit" in tips_for("plugin", plugin_of_pair, "base", "no-such-name")[1], "an unknown name"
        # resolve gives a commit: a tree is none, and an annotated tag gives the commit it points to.
        tree_of_head = subprocess.run(["git", "-C", sibling_tree, "rev-parse", "HEAD^{tree}"], capture_output=True, text=True, env=git_env()).stdout.strip()
        assert len(tree_of_head) == 40 and resolve(sibling_tree, tree_of_head) is None, "the name of a tree is no commit"
        assert "is no commit" in tips_for("plugin", plugin_of_pair, "base", tree_of_head)[1], "--sibling-base with a tree"
        subprocess.run(["git", "-C", sibling_tree, *author, "tag", "-a", "-m", "a tag", "a-tag", first], check=True, env=git_env())
        assert resolve(sibling_tree, "a-tag") == first, "an annotated tag gives its commit, not the tag object"
        assert tips_for("plugin", plugin_of_pair, "base", "a-tag") == ({"plugin": "base", "mge": first}, None)
        # git() and branch_of answer for the folder they get, also with the GIT_ names of a hook in the environment.
        alpha, beta = os.path.join(root, "alpha"), os.path.join(root, "beta")
        for folder, branch in ((alpha, "alpha-branch"), (beta, "beta-branch")):
            os.makedirs(folder)
            for command in (["init", "-q", "-b", branch], author + ["commit", "-q", "--allow-empty", "-m", branch]):
                subprocess.run(["git", "-C", folder, *command], check=True, env=git_env())
        beta_head = resolve(beta, "HEAD")
        os.environ["GIT_DIR"] = os.path.join(alpha, ".git")
        try:
            assert branch_of(beta) == "beta-branch", "branch_of answers for the hook's repository: %s" % branch_of(beta)
            assert git(beta, "rev-parse", "HEAD") == beta_head, "git() answers for the hook's repository"
        finally:
            del os.environ["GIT_DIR"]
        assert tips_for("mge", sibling_tree, "base", first)[1].startswith("no sibling"), "no sibling tree"
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
        # The rounds of this part test the holds of a check that needs the game slot; their stages have test names, which
        # GAME_STAGES does not hold. The part of D183 below uses the real rule.
        real_needs_game = globals()["needs_game"]
        globals()["needs_game"] = lambda stages: True

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

        # A check that waits for the build side keeps its place in the game queue (2026-11-09: the check waited for the build
        # side with no ask for the game, and a timing take got the game slot first). Two other sessions hold both build places
        # and free them from inside the wait, at the wait that each case names. A case has a bound: a fault that keeps the
        # build side held ends the case with an assert and not with a wait with no end.
        own_session = os.environ["CLAUDE_CODE_SESSION_ID"]
        builders = ("builder-1", "builder-2")
        real_ask_in_queue = GamePlace.ask_in_queue
        real_note_active = slots.note_active
        real_wait_seconds = globals()["WAIT_SECONDS"]
        DEADLINE_SECONDS = 30

        def as_session(name):
            os.environ["CLAUDE_CODE_SESSION_ID"] = name

        def free_build_side():
            for other in builders:
                as_session("merge-step-self-test-" + other)
                assert slots.release("build", "other: " + other).code == slots.DONE
            as_session(own_session)

        def round_that_waits(what, on_wait):
            """The output and the game notes of a round that waits for the build side. `on_wait(count, place)` runs at
            each wait of the build take, after the round's own ask for the game. An exception of `on_wait` ends the round
            and reaches the caller."""
            for other in builders:
                as_session("merge-step-self-test-" + other)
                assert slots.take("build", "other: " + other, minutes=5).code == slots.DONE
            as_session(own_session)
            waits = []
            game_notes = []

            def asked(place):
                real_ask_in_queue(place)
                waits.append(place)
                on_wait(len(waits), place, game_notes)

            def noted(slot, now):
                if slot == "game":
                    game_notes.append(now)
                return real_note_active(slot, now)

            outcome = {}

            def run():
                try:
                    outcome["round"] = run_round(slots, [("both held", root, both_held)], what, round_log, False)
                except BaseException as error:  # a KeyboardInterrupt of `on_wait` too
                    outcome["error"] = error

            # The round runs in a thread that the case gives a deadline: a fault that keeps the build side held (the ask for
            # the game removed, so that nothing frees it) ends the case with an assert and not with a wait with no end.
            GamePlace.ask_in_queue = asked
            slots.note_active = noted
            globals()["WAIT_SECONDS"] = 0.05
            output = io.StringIO()
            try:
                with contextlib.redirect_stdout(output):
                    thread = threading.Thread(target=run, daemon=True)
                    thread.start()
                    thread.join(DEADLINE_SECONDS)
                    stuck = thread.is_alive()
                    if stuck:
                        # Free the build side, so that the round ends: a thread that waits on holds a file of the temporary folder,
                        # and the cleanup of the folder fails with a PermissionError that hides the assert below.
                        free_build_side()
                        thread.join(DEADLINE_SECONDS)
            finally:
                GamePlace.ask_in_queue = real_ask_in_queue
                slots.note_active = real_note_active
                globals()["WAIT_SECONDS"] = real_wait_seconds
                as_session(own_session)
            assert not stuck, "the round still waited for the build side after %d s (%d waits): nothing freed it" % (DEADLINE_SECONDS, len(waits))
            assert not thread.is_alive(), "the round did not end after the build side was freed"
            if "error" in outcome:
                raise outcome["error"]
            return outcome["round"], output.getvalue()

        # Case 1: the check holds the free game slot at its first wait, notes its work at the next wait, and another session
        # that asks for the game meanwhile gets no place before it.
        seen = {}

        def let_others_act(count, place, game_notes):
            if count == 1:
                seen["held at the first wait"] = place.held
                as_session("merge-step-self-test-gamer")
                seen["game take of another session"] = slots.take("game", "other: a game", minutes=5).code
                # A take that got the slot (the fault under test) is released, so that the round ends and the asserts below fail.
                slots.release("game", "other: a game")
                slots.leave("game", "other: a game")
                as_session(own_session)
            elif count == 2:
                seen["game notes at the second wait"] = len(game_notes)
                free_build_side()

        waiting_round, waiting_output = round_that_waits("merge_step: x ffffffffff", let_others_act)
        assert seen.get("held at the first wait") is True, "the check does not hold the free game slot while it waits for the build side: %s" % seen
        assert seen.get("game take of another session") != slots.DONE, "another session got the game slot before the check: %s" % seen
        assert seen.get("game notes at the second wait", 0) >= 1, "the check does not note the work of the game slot it holds while it waits: %s" % seen
        assert "game held for the merge check, waiting for the build side since " in waiting_output, waiting_output
        assert waiting_round is not None and waiting_round[0] == [True], waiting_round
        assert holders() == [], holders()

        # Case 2: a Ctrl-C in the wait, after the early hold, releases the game slot (else it stays held for its whole lease).
        def interrupt_at_the_second_wait(count, place, game_notes):
            if count == 2:
                raise KeyboardInterrupt

        try:
            round_that_waits("merge_step: x gggggggggg", interrupt_at_the_second_wait)
            raise AssertionError("the interrupt did not reach the caller")
        except KeyboardInterrupt:
            pass
        assert "game.holder" not in holders(), "a Ctrl-C in the wait leaves the game slot held: %s" % holders()
        free_build_side()
        assert holders() == [], holders()

        # The holds of the run lapse while it waits (2026-11-09, the W206 render-link check: a take of the game that waited 34
        # minutes lost the build side after 15 minutes, and the replay gate and the instance A/B were refused by the slot check).
        # Before each stage the run looks at its holds, and takes a lapsed one again.
        def lapse_now():
            slots.lapse_idle_holds(time.time() + 20 * 60)

        # Case 3: both holds lapse before the second stage. The stage checks that it holds both, and passes.
        build_notes = []

        def note_then_lapse_before_the_second_stage(slot, now):
            real_note_active(slot, now)
            if slot == "build":
                build_notes.append(now)
                if len(build_notes) == 2:
                    lapse_now()

        slots.note_active = note_then_lapse_before_the_second_stage
        shown = io.StringIO()
        try:
            with contextlib.redirect_stdout(shown):
                lapsed_round = run_round(slots, [("first", root, both_held), ("second", root, both_held), ("third", root, both_held)],
                                         "merge_step: x iiiiiiiiii", round_log, True)
        finally:
            slots.note_active = real_note_active
        assert lapsed_round is not None and lapsed_round[0] == [True, True, True], (lapsed_round, shown.getvalue())
        assert "re-took the build side" in shown.getvalue() and "re-took the game slot" in shown.getvalue(), shown.getvalue()
        with open(round_log, encoding="utf-8") as log:
            kept = log.read()
        assert "== re-took the build side at " in kept and "== re-took the game slot at " in kept, "the kept log names each take again: " + kept
        assert holders() == [], holders()

        # Case 4: the whole side lapsed, and another session took a place meanwhile (the case of the incident): the run waits
        # in the queue for it, with its ask for the game, and then holds both places.
        what = "merge_step: x jjjjjjjjjj"
        assert slots.take("build", what, minutes=5, whole=True).code == slots.DONE
        lapse_now()
        assert holders() == [], holders()
        as_session("merge-step-self-test-" + builders[0])
        assert slots.take("build", "other: " + builders[0], minutes=5).code == slots.DONE
        as_session(own_session)
        log = io.StringIO()
        waits_for_the_place = []
        sleeps_of_the_take = []

        def bounded_sleep(seconds):
            # A fault that does not ask for the game at the wait leaves nothing to free the place: the case ends with an assert.
            sleeps_of_the_take.append(seconds)
            assert len(sleeps_of_the_take) < 40, "the run waits for a place that nothing frees"

        real_sleep, time.sleep = time.sleep, bounded_sleep
        try:
            with slot_asker(slots, "game") as game_ask, contextlib.redirect_stdout(io.StringIO()):
                game = GamePlace(slots, GAME_LABEL % what, game_ask)

                def free_the_place():
                    waits_for_the_place.append(1)
                    if len(waits_for_the_place) == 2:
                        as_session("merge-step-self-test-" + builders[0])
                        assert slots.release("build", "other: " + builders[0]).code == slots.DONE
                        as_session(own_session)

                game.ask_in_queue = free_the_place
                assert keep_holds(slots, what, game, {"whole": True}, False, log) is True
        finally:
            time.sleep = real_sleep
            as_session(own_session)
        assert places_held(slots, "build", what) == 2, "the run holds both places again"
        assert places_held(slots, "build", "merge_step: x some-other-run") == 0, "a hold of another label is no hold of the run"
        assert len(waits_for_the_place) >= 2 and "== the build hold of this run ended" in log.getvalue() and "== re-took the build side at " in log.getvalue(), log.getvalue()
        assert slots.release("build", what).code == slots.DONE
        assert holders() == [], holders()

        # Case 5: with --no-wait a lapsed hold is not waited for: the stage is NOT RUN. A hold that is intact is left alone.
        assert slots.take("build", what, minutes=5).code == slots.DONE
        intact = io.StringIO()
        with slot_asker(slots, "game") as game_ask, contextlib.redirect_stdout(io.StringIO()):
            game = GamePlace(slots, GAME_LABEL % what, game_ask)
            assert keep_holds(slots, what, game, {}, True, intact) is True and intact.getvalue() == "", intact.getvalue()
            slots.release("build", what)
            as_session("merge-step-self-test-" + builders[0])
            assert slots.take("build", "other: " + builders[0], minutes=5, whole=True).code == slots.DONE
            as_session(own_session)
            refused = io.StringIO()
            seen_before = [slots.last_active(unit) for unit in slots.units_of("build")]
            assert keep_holds(slots, what, game, {}, True, refused) is False, "no wait: nothing is taken again"
            assert [slots.last_active(unit) for unit in slots.units_of("build")] == seen_before, "the run notes build work for a hold of another session"
        assert "taking it again" in refused.getvalue()
        as_session("merge-step-self-test-" + builders[0])
        assert slots.release("build", "other: " + builders[0]).code == slots.DONE
        as_session(own_session)
        assert holders() == [], holders()
        with open(os.path.join(root, "stopped.log"), "w+", encoding="utf-8") as stopped:
            assert run_stages([("a", root, good), ("b", root, good)], stopped, lambda: False) == [False, False]
            stopped.seek(0)
            assert "== NOT RUN a: the slot hold of this run ended and was not taken again" in stopped.read()

        # Case 6: both holds lapse, the game is taken by another session, and the build side lapses again while the run waits for
        # the game. The run takes the game, then looks again, and takes the build side a second time.
        what = "merge_step: x kkkkkkkkkk"
        with slot_asker(slots, "game") as game_ask, contextlib.redirect_stdout(io.StringIO()):
            game = GamePlace(slots, GAME_LABEL % what, game_ask)
            assert slots.take("build", what, minutes=5).code == slots.DONE and game.take(False)
            lapse_now()
            assert holders() == [], holders()
            as_session("merge-step-self-test-gamer")
            assert slots.take("game", "other: a game", minutes=5).code == slots.DONE
            as_session(own_session)
            lapsed_while_waiting = []
            sleeps_for_the_game = []

            def lapse_while_waiting(seconds):
                # The first wait for the game: the build side of the run lapses, and so does the hold of the other session.
                sleeps_for_the_game.append(seconds)
                assert len(sleeps_for_the_game) < 40, "the run waits for a game that nothing frees"
                if not lapsed_while_waiting:
                    lapsed_while_waiting.append(1)
                    lapse_now()

            twice = io.StringIO()
            real_sleep, time.sleep = time.sleep, lapse_while_waiting
            try:
                assert keep_holds(slots, what, game, {}, False, twice) is True
            finally:
                time.sleep = real_sleep
            assert places_held(slots, "build", what) == 1 and places_held(slots, "game", game.label) == 1, "both holds are the run's after the second look: " + twice.getvalue()
            assert twice.getvalue().count("re-took the build side") == 2 and game.retakes == 1, twice.getvalue()
            game.release()
            slots.release("build", what)
        assert holders() == [], holders()
        globals()["needs_game"] = real_needs_game

        # D183: the game slot only for a stage that needs it. The stage lists of the tool first.
        assert not needs_game(stages_for("plugin", plugin, ["tools/x.py", "src/lib.rs"])), "the checks of a tools change took the game"
        assert needs_game(stages_for("plugin", plugin, ["host/src/retained.rs"])), "the replay gate did not take the game"
        assert not needs_game(stages_for("mge", mge, ["xtask/src/main.rs"]))
        assert needs_game([(INSTANCE_STAGE, root, good)]) and not needs_game([("checks", root, good)])
        # The stage passes only when the round holds the build side, and has no hold and no waiter of the game slot.
        build_only = [sys.executable, "-c", "import os, sys; sys.exit(0 if os.path.exists(%r) and not os.path.exists(%r) and not "
                      "any(name.startswith('game.waiter.') for name in os.listdir(%r)) else 7)"
                      % (slots.holder_path("build"), slots.holder_path("game"), slots.STATE_DIR)]
        real_note_active = slots.note_active
        noted_slots = []
        slots.note_active = lambda slot, now: (noted_slots.append(slot), real_note_active(slot, now))[1]
        with contextlib.redirect_stdout(io.StringIO()) as quiet:
            # A round with no game stage holds the build side only, notes no work of the game, and says so.
            try:
                assert run_round(slots, [("checks", root, build_only)], "merge_step: x ffffffffff", round_log, True)[0] == [True], "the round held the game"
            finally:
                slots.note_active = real_note_active
            assert "game" not in noted_slots, "a round with no game stage noted the work of the game: %s" % noted_slots
            assert holders() == [], holders()
            assert "need no game slot" in quiet.getvalue(), quiet.getvalue()
            # Another session's game does not stop it, and keeps its hold.
            os.environ["CLAUDE_CODE_SESSION_ID"] = "merge-step-self-test-other"
            assert slots.take("game", "other: a game", minutes=5).code == slots.DONE
            os.environ["CLAUDE_CODE_SESSION_ID"] = "merge-step-self-test"
            assert run_round(slots, [("checks", root, good)], "merge_step: x 1111111111", round_log, True)[0] == [True], "a game stopped a check with no game stage"
            assert holders() == ["game.holder"], holders()
            # A round with a stage that needs the game still waits for it: with --no-wait it stops.
            assert run_round(slots, [(INSTANCE_STAGE, root, good)], "merge_step: x 2222222222", round_log, True) is None, "a game stage ran beside a game"
            os.environ["CLAUDE_CODE_SESSION_ID"] = "merge-step-self-test-other"
            assert slots.release("game", "other: a game").code == slots.DONE
            os.environ["CLAUDE_CODE_SESSION_ID"] = "merge-step-self-test"
            # With the game free, it holds both while the stage runs, and releases both.
            assert run_round(slots, [(INSTANCE_STAGE, root, both_held)], "merge_step: x 3333333333", round_log, True)[0] == [True]
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
        first_log = log_path_of("0123456789abcdef", logs)
        assert first_log == os.path.join(logs, "merge-step-0123456789.log") and os.path.isdir(logs)
        # A rerun for the same commit never writes into the log of an earlier round.
        with open(first_log, "w", encoding="utf-8") as log:
            log.write("the refused round\n")
        second_log = log_path_of("0123456789abcdef", logs)
        third_log = log_path_of("0123456789abcdef", logs)
        assert (second_log, third_log) == (os.path.join(logs, "merge-step-0123456789-2.log"), os.path.join(logs, "merge-step-0123456789-3.log"))
        with open(second_log, "w", encoding="utf-8") as log:
            log.write("the rerun\n")
        with open(first_log, encoding="utf-8") as log:
            assert log.read() == "the refused round\n", "the rerun wrote into the log of the first round"
        assert log_path_of("fedcba9876543210", logs) == os.path.join(logs, "merge-step-fedcba9876.log"), "another commit starts at the plain name"
        assert "target" not in LOG_FOLDER.lower()

        # The closing lines of the items (D180): only a subject that ends with "(W<n>)" or "(W<n> <part>)" names an item.
        assert items_of("Merge: a check that MWSE's vtable fills name their own slot (W93)") == ["W93"]
        assert items_of("Merge: the second part (W234 a)") == ["W234"] and items_of("Merge: the crate half (W229 crate half) ") == ["W229"]
        for no_item in ("Merge: W96 in the middle of the subject", "Merge: W96 in the middle (D161)", "Merge: dropped W220",
                        "Merge: log pipe follow-ups W219 and W221 (W220 dropped) and more", "Merge: lower case (w96)",
                        "Merge: log pipe follow-ups W219 and W221 (W220 dropped)", "Merge: a nested part (W234 (a))",
                        "Merge: a dropped item (W96 dropped)", "Merge: the item (W96) is not at the end",
                        "Merge: no parentheses W96", "Merge: two items (W222, W226)", "Merge: the plan (D161)",
                        "Merge: say which views draw the table", "Merge: a letter on the number (W234a)", "Merge: empty ()"):
            assert items_of(no_item) == [], no_item

        closes = os.path.join(root, "closes", "closes.tsv")
        os.makedirs(os.path.dirname(closes))
        when = "2026-11-09T20:15:00+04:00"
        round_log = "E:\\nirender-keep\\merge-step\\merge-step-862873b012.log"

        def close_batch(results, merges, baseline=BASELINE):
            return close_after(results, baseline, merges, lambda merges, stages: write_closes(closes, when, PLUGIN_NAME, merges, stages, round_log, wait=0))

        def close(results, subject, commit="862873b0123456789", baseline=BASELINE):
            return close_batch(results, [(commit, subject)], baseline)

        def closes_bytes():
            with open(closes, "rb") as written:
                return written.read()

        subject = "Merge: a check that MWSE's vtable fills name their own slot (W93)"
        # A failed stage writes nothing and makes no file; so do a pass against another folder and a subject with no item.
        assert close([True, False, True, True, True, True], subject) is None and not os.path.exists(closes), "a fail wrote a close"
        assert close([], subject) is None and not os.path.exists(closes), "a round with no stage wrote a close"
        assert close([True] * 6, subject, baseline=os.path.join(root, "other")) is None and not os.path.exists(closes)
        assert close([True] * 6, "Merge: say which views draw the table") == [] and not os.path.exists(closes), "a subject with no item wrote"
        # A pass writes the header and one line, CRLF.
        line = "\t".join((when, "W93", PLUGIN_NAME, "862873b012", "6 of 6", round_log, subject))
        assert close([True] * 6, subject) == [line]
        assert closes_bytes() == (CLOSES_HEADER + "\r\n" + line + "\r\n").encode("ascii"), closes_bytes()
        # A rerun of the commit writes nothing; a later commit of the item writes its line, also in the part form; an item
        # in the middle of a subject writes nothing.
        assert close([True] * 6, subject) == [], "a rerun wrote a second line"
        assert len(close([True] * 5, "Merge: part b (W93)", commit="abcdef0123456")) == 1
        assert [written.split("\t")[1] for written in close([True] * 4, "Merge: the second part (W234 b)", commit="1111111111aa")] == ["W234"]
        assert close([True] * 6, "Merge: W96 in the middle of the subject", commit="3333333333") == []
        data = closes_bytes()
        assert data.count(b"\r\n") == 4 and data.count(CLOSES_HEADER.encode("ascii")) == 1 and b"\n" not in data.replace(b"\r\n", b""), data

        # A reader that holds the file: the append is tried again, then given up with None, and nothing else happens.
        tries = []

        def held(*args, **kwargs):
            tries.append(args[0])
            raise PermissionError("held by a reader")

        globals()["open"] = held
        try:
            given_up = write_closes(closes, when, PLUGIN_NAME, [("2222222222", "Merge: x (W300)")], "6 of 6", round_log, tries=3, wait=0)
        finally:
            del globals()["open"]
        assert given_up is None and len(tries) == 3, tries
        assert closes_bytes() == data, "a given-up append changed the file"

        # D183: a batch of merges with one check closes the item of each merge on the first-parent path, and the checked
        # commit's own; a commit of a merged branch closes nothing.
        entries = [("a1a1a1a1a1aa", "0000", "Fix a thing on the branch (W90)"), ("m1m1m1m1m1mm", "a0a0 b1b1", "Merge: one (W96)"),
                   ("m2m2m2m2m2mm", "m1m1 c1c1", "Merge: two (W97 a)"), ("m3m3m3m3m3mm", "m2m2 d1d1", "Merge: no item")]
        assert closing_commits(entries, "m3m3m3m3m3mm") == [(sha, subject) for sha, _, subject in entries[1:]], "the merges of the path"
        assert closing_commits([("t1t1t1t1t1tt", "0000", "Merge: a fast-forward (W98)")], "t1t1t1t1t1tt") == [("t1t1t1t1t1tt", "Merge: a fast-forward (W98)")]
        batch = closing_commits(entries, "m3m3m3m3m3mm")
        assert close_batch([True, False], batch) is None, "a failed batch wrote a close"
        assert [line.split("\t")[1:4] for line in close_batch([True] * 6, batch)] == [["W96", PLUGIN_NAME, "m1m1m1m1m1"], ["W97", PLUGIN_NAME, "m2m2m2m2m2"]], "a batch"
        assert close_batch([True] * 6, batch) == [], "a rerun of the batch wrote again"
        assert close_batch([True] * 6, [("m4m4m4m4m4mm", "Merge: three (W99)"), ("m4m4m4m4m4mm", "Merge: three (W99)")]) != [], "one line, not two"
        assert closes_bytes().count(b"\tW99\t") == 1, "one commit and item twice in a batch wrote two lines"

        # The first-parent path of a real history: two merges of two branches, each branch with an item in its own subject,
        # a commit on main between them, and a merge of main into the second branch (that merge is not on the path).
        history = os.path.join(root, "history")
        os.makedirs(history)

        def in_history(*args):
            return subprocess.run(["git", "-C", history, "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", *args],
                                  capture_output=True, text=True, check=True, env=git_env()).stdout.strip()

        in_history("init", "-q", "-b", "main")
        in_history("commit", "-q", "--allow-empty", "-m", "the base")
        history_base = in_history("rev-parse", "HEAD")
        in_history("switch", "-q", "-c", "one")
        in_history("commit", "-q", "--allow-empty", "-m", "Fix on the branch (W12)")
        in_history("switch", "-q", "main")
        in_history("merge", "-q", "--no-ff", "-m", "Merge: one (W96)", "one")
        in_history("switch", "-q", "-c", "two")
        in_history("commit", "-q", "--allow-empty", "-m", "Fix on the branch (W13)")
        in_history("switch", "-q", "main")
        in_history("commit", "-q", "--allow-empty", "-m", "A fix on main (W14)")
        in_history("switch", "-q", "two")
        in_history("merge", "-q", "--no-ff", "-m", "Merge: main into two (W15)", "main")
        in_history("switch", "-q", "main")
        in_history("merge", "-q", "--no-ff", "-m", "Merge: two (W97 a)", "two")
        history_tip = in_history("rev-parse", "HEAD")
        path, warning = closing_merges(history, history_base, history_tip)
        assert [subject for _, subject in path] == ["Merge: one (W96)", "Merge: two (W97 a)"], path
        assert path[-1][0] == history_tip and [items_of(subject) for _, subject in path] == [["W96"], ["W97"]]
        assert warning is None, warning
        # A fast-forward of a branch with two commits: only the tip closes, and the check warns.
        in_history("switch", "-q", "-c", "three")
        in_history("commit", "-q", "--allow-empty", "-m", "First fix (W16)")
        in_history("commit", "-q", "--allow-empty", "-m", "Second fix (W17)")
        in_history("switch", "-q", "main")
        in_history("merge", "-q", "--ff-only", "three")
        fast_tip = in_history("rev-parse", "HEAD")
        path, warning = closing_merges(history, history_tip, fast_tip)
        assert path == [(fast_tip, "Second fix (W17)")], path
        assert warning and "--no-ff" in warning and fast_tip[:10] in warning, warning

        # The warning on fixed paths: none when a merge with an item is on the path, or when the checked commit is a merge.
        plain = ("p1p1p1p1p1pp", "0000", "A fix (W20)")
        merge_with_item = ("m5m5m5m5m5mm", "p1p1 e1e1", "Merge: five (W21)")
        merge_no_item = ("m6m6m6m6m6mm", "m5m5 f1f1", "Merge: no item")
        assert fast_forward_warning([plain, merge_with_item], "m5m5m5m5m5mm") is None
        assert fast_forward_warning([merge_with_item, plain], "p1p1p1p1p1pp") is None, "a merge with an item before a plain tip"
        assert fast_forward_warning([plain, merge_no_item], "m6m6m6m6m6mm") is None, "the tip is a merge"
        assert fast_forward_warning([merge_no_item, plain], "p1p1p1p1p1pp"), "no merge with an item, and a plain tip"
        assert fast_forward_warning([plain], "p1p1p1p1p1pp"), "a fast-forward of one commit"

        # D183: no stage writes to E: in a round with no game stage. The stage lists of the tool pass the rule.
        for kind, repo, paths in (("plugin", plugin, ["tools/x.py"]), ("plugin", plugin, ["mod/MWSE/mods/nirender/main.lua"]),
                                  ("plugin", plugin, ["contract/x"]), ("mge", mge, ["xtask/src/main.rs"]), ("mge", mge, ["d3d8/x.rs"])):
            assert evidence_problem(stages_for(kind, repo, paths)) is None, (kind, paths)
        with_base = stages_for("plugin", plugin, ["host/src/retained.rs"], tips={"plugin": "a", "mge": "b"}, base_folder="E:\\nirender-keep\\merge-step\\base-x")
        assert BASE_STAGE in [name for name, _, _ in with_base] and evidence_problem(with_base) is None, "the base build holds the game"
        for writer in (["robocopy", "x", "E:\\nirender-keep\\x"], ["tool", "--out=E:/nirender-keep/x"], ["cp", "x", "/e/nirender-keep/x"],
                       ["cp", "x", "e:/nirender-keep"], ["cp", "x", "E:"], ["cp", "x", "E:\\"], ["cp", "x", "E:x.txt"]):
            assert evidence_problem([("checks", root, good), ("a copy", root, writer)]), writer
            assert evidence_problem([("a copy", root, writer), (INSTANCE_STAGE, root, good)]) is None, writer
        assert evidence_problem([("a copy", "E:\\nirender-keep", ["cp", "x", "y"])]), "a stage in a folder on E:"
        for harmless in (["cp", "x", "D:/e/x"], ["cp", "-e", "x"], ["cp", "notes/e/x", "C:/x/e:"], ["cp", "x", "--e:x"]):
            assert evidence_problem([("a copy", root, harmless)]) is None, harmless
        # The round refuses such a list before any take, and holds nothing.
        try:
            run_round(slots, [("a copy", root, ["cp", "x", "E:\\nirender-keep\\x"])], "merge_step: x 4444444444", round_log, True)
        except ValueError as error:
            assert "on E:" in str(error), error
        else:
            raise AssertionError("a round with no game stage ran a stage that writes to E:")
        assert holders() == [], holders()
    print("merge_step self-test: ok")


def main():
    if len(sys.argv) > 2 and sys.argv[1] == REQUIRE_TESTS:
        return require_tests(sys.argv[2:])
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("repo", nargs="?")
    parser.add_argument("commit", nargs="?")
    parser.add_argument("--base", help="the tip of the shared branch before the merge: the diff from it decides the stages")
    parser.add_argument("--sibling-base", help="the commit of the sibling repo for the base build, an ancestor of its HEAD: for a base with no kept pair; without it, the sibling's commit that passed with --base")
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
    tips = None
    if kind and any(name == "replay gate" for name, _, _ in stages):
        tips, why = tips_for(kind, repo, base, args.sibling_base) if branch else (None, "the tree is on no branch, and the kept base build is one for each branch")
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
    problem = evidence_problem(stages)
    if problem:
        say("FAIL the stages: %s" % problem)
        return 1
    sibling_hash = git(sibling, "rev-parse", "--short=10", "HEAD") if has_sibling else "none"
    sibling_head = resolve(sibling, "HEAD") if has_sibling else None
    sibling_base_note = ""
    if tips:
        sibling_base_note = " (base build at %s%s)" % (tips["mge" if kind == "plugin" else "plugin"][:10], ", --sibling-base" if args.sibling_base else ", the kept pair")
    elif args.sibling_base:
        sibling_base_note = " (--sibling-base unused: no base build in these stages)"
    say("%s %s, sibling %s%s, baseline %s: %d changed paths from %s; stages: %s" % (os.path.basename(repo), commit[:10], sibling_hash, sibling_base_note, baseline, len(paths), base[:10], ", ".join(name for name, _, _ in stages)))
    # The merges that a pass closes (D183), read before the round so that the warning of a fast-forward comes first.
    merges, warning = closing_merges(repo, base, commit)
    if warning:
        say(warning)
    if args.dry_run:
        return 0

    slots = load_slots()
    what = "merge_step: %s %s" % (os.path.basename(repo), commit[:10])
    log_path = log_path_of(commit)
    done = run_round(slots, stages, what, log_path, args.no_wait)
    if done is None:
        return 3
    results, summary = done
    say(summary)
    # close_after is the one gate of the closing lines: a failed stage or a pass against another folder writes none. A batch
    # of merges with one check closes the item of each merge on the first-parent path (D183).
    when = datetime.datetime.now().astimezone().isoformat(timespec="seconds")
    written = close_after(results, baseline, merges,
                          lambda merges, stages: write_closes(CLOSES_FILE, when, os.path.basename(repo), merges, stages, log_path))
    if written:
        say("closes: %s in %s" % (", ".join(line.split("\t")[1] for line in written), CLOSES_FILE))
    elif written == []:
        say("closes: nothing to write (no new item in the subjects of %d merge(s): %s)" % (len(merges), "; ".join(subject for _, subject in merges)))
    if not all(results):
        return 1
    if moves_the_ref(baseline):
        git(repo, "update-ref", passed_ref, commit)
        # The pair of the pass is the base pair of the next check (tips_for).
        if sibling_head and resolve(sibling, "HEAD") == sibling_head:
            keep_pair(kind, sibling, commit, sibling_head)
            say("%s in the sibling: %s" % (pair_ref(kind, commit), sibling_head[:10]))
        elif sibling_head:
            say("no pair kept: the HEAD of the sibling moved during the round (%s at the start)" % sibling_head[:10])
    else:
        say("%s stays: this pass is against %s, not the accepted folder" % (passed_ref, baseline))
    return 0


if __name__ == "__main__":
    sys.exit(main())
