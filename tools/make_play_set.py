"""Makes the owner's play set: a build of what is ready now, tested, and named in local.bat.

    python -I tools/make_play_set.py [--force] [--no-wait]
    python -I tools/make_play_set.py --build-only   # warms the cargo targets: no set, no slot of the game
    python -I tools/make_play_set.py --bats-only    # the step of the owner's start alone, on local.bat's set
    python -I tools/make_play_set.py --self-test    # the tests of the pure parts: no build, no game

It builds nirender-plugin `master` and render-link `feature/wgpu-renderer` at their local tips, in the
worktrees of C:/tmp/playset, which keep their cargo targets for the next set. It copies the stage of
`cargo xtask dist` to a new E:/nirender-keep/mgebuilds/f<N>, with no pdb, and the plugin beside it as
akulakhan-<commit>.dll. Then it takes the game slot and runs the smoke on the testing instance:

- live: tools/run_own_device.py with batch-play-smoke.lua: teleports over 5 cell borders, a save, a move to
  another cell, a load, the map, the inventory and the character. The runner must end well, each answer of
  the game must be the asked one, no step picture may be black, and host.txt must have no `gpu error`.
- gate: tools/validate_retained.py baseline, with no game: the replays against the accepted pictures. This
  sees a fault that both modes share. A planned new look stops the set until a baseline is accepted.
- tour: the 3 test cells with --tour-dual, and tools/validate_tour.py: no picture may fail, no pair may lack
  its numbers, and no pair of mode gpu against mode off may differ by more than MEAN_LIMIT.
- the owner's start: both play-manual-check*.bat with the real tools/play.py and --stage-only, so that
  they stop just before the game. The bat must pass the slot check, with this session's variable in its
  terminal, and each instance's mod list must be as before.

This is the smoke of f185 with two changes: a batch replaces the live bridge, as a script cannot call it,
and the gate is added. A crash when the game closes, after the last step, fails nothing: it is the parked
fault at exit, and the note says so. A new fault at exit reads the same.

Only on a pass, it writes MGE_BUILD and NIRENDER_PLUGIN_DLL in local.bat of the main checkout. A failed
smoke leaves local.bat on the last good set, says why, and exit code 1. Each set has a one-line WHAT.txt,
which names the MD5 of the MWSE.dll that the smoke ran on, from the lines of the runner in progress.log.
No set is deleted: older sets stay until the owner's cleanup, and a failed set stays with its reason. The
only file it deletes is the smoke's own save. The smoke's runs are copied to E:/nirender-keep/runs-<date>.

It reads the tips again when it holds the slot, and builds again if they moved, as the wait in the queue can
be long. If they move during the smoke, the note says that the set is not at the tips.

With the tips of the set in local.bat it does nothing, unless --force. The owner's checkout must be at the
plugin's commit, as a game by the bat takes the Lua of mod/ from there: otherwise it stops, unless --force,
and the note says so. A build waits while the slots refuse a build, and the smoke waits in the queue of the
game slot: with --no-wait each stops with exit code 3. It needs the slot tool, and a session when
enforcement is on: nobody waits for this job, and a game with no slot can spoil a timing run.
"""
import argparse
import datetime
import glob
import hashlib
import json
import msvcrt
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

# The modules of this folder, also under `python -I`.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import SIBLINGS, local  # noqa: E402

REPOS = local("repos", SIBLINGS)
PLUGIN_REPO = os.path.join(REPOS, "nirender-plugin")
MGE_REPO = os.path.join(REPOS, "MGE-XE-G7-render-link")
PLUGIN_BRANCH = "master"
MGE_BRANCH = "feature/wgpu-renderer"
TREES = "C:/tmp/playset"
SETS = "E:/nirender-keep/mgebuilds"
KEPT_RUNS = "E:/nirender-keep/runs-%s"
HARNESS = os.path.join(REPOS, "moreFPS", "docs", "scripts", "test-harness-2026-10-03")
BATCH = os.path.join(REPOS, "moreFPS", "docs", "Numidium Project", "scripts", "wgpu-scene-audit", "batch-play-smoke.lua")
LIVECODING = os.path.join(REPOS, "moreFPS", "docs", "Numidium Project", "scripts", "wgpu-scene-audit", "livecoding-mark.lua")
BASELINE = os.path.join(PLUGIN_REPO, "results", "baseline-records")
PLUGIN_TARGET = os.path.join("target", "i686-pc-windows-msvc", "release", "akulakhan.dll")
TOUR_CELLS = "@nifTest_1;@nifTest_2;@nifTest_3"
TESTING_MODLIST = "D:/Modlists/Morrowind75testing/profiles/Default/modlist.txt"
OUTLANDER_PROFILE = "Outlander"
SMOKE_SAVE = "D:/Modlists/Morrowind75testing/profiles/Default/saves/playsmoke.ess"
WAIT_SECONDS = 60
SLOT_MINUTES = 45
# f185 had a largest mean of 0.001 at the tour's 103 stops. A change of the picture between two modes that
# is larger than this is no noise. [I] The value is a judgement, with room for small differences. The gate
# is the exact test: this is a second net.
MEAN_LIMIT = 0.05
# The tour has 103 stops: a much shorter tour did not run through.
LEAST_STOPS = 100
# Below this mean level (0 to 255) a step picture counts as black.
BLACK_LEVEL = 3.0
OWNER_BATS = ("play-manual-check.bat", "play-manual-check-outlander.bat")
BAT_CLEARS = 'set "CLAUDE_CODE_SESSION_ID="'
# The MD5 of the MWSE.dll of each run of this smoke.
SMOKE_MWSE = set()


class SmokeFailed(Exception):
    """The reason why a set does not become the play set."""


def say(text):
    print(text, flush=True)


def git(folder, *args):
    done = subprocess.run(["git", "-C", folder, *args], capture_output=True, text=True)
    if done.returncode != 0:
        raise SystemExit("git %s in %s: %s" % (" ".join(args), folder, done.stderr.strip()))
    return done.stdout.strip()


def md5_of(path):
    with open(path, "rb") as file:
        return hashlib.md5(file.read()).hexdigest()


# --- the pure parts ---------------------------------------------------------------------

def current_tips():
    """The commits of the two branches now, as (plugin, render-link)."""
    return git(PLUGIN_REPO, "rev-parse", "--short=7", PLUGIN_BRANCH), git(MGE_REPO, "rev-parse", "--short=7", MGE_BRANCH)


def tips_moved(built, now):
    """A text that says which tips differ from the built commits, or an empty text."""
    moved = ["%s moved from %s to %s" % (branch, old, new) for branch, old, new in zip((PLUGIN_BRANCH, MGE_BRANCH), built, now, strict=True) if old != new]
    return ", ".join(moved)


def next_set_number(names):
    """The next number of a set folder. The highest number as a number: a text sort puts f99 after f100."""
    numbers = [int(match.group(1)) for match in (re.fullmatch(r"f(\d+)", name) for name in names) if match]
    return max(numbers, default=0) + 1


def local_bat_values(text):
    """The values of the set lines in the text of local.bat, as a dict."""
    return dict(re.findall(r'^set "(\w+)=([^"\r\n]*)"', text, re.MULTILINE))


def with_set_lines(text, set_folder, plugin_dll):
    """The text of local.bat with the two lines of the set, and all else as it was. Each line must exist."""
    for key, value in (("MGE_BUILD", set_folder), ("NIRENDER_PLUGIN_DLL", plugin_dll)):
        # A function for the replacement: a backslash in the value is no escape.
        text, count = re.subn(r'^set "%s=[^"\r\n]*"' % key, lambda _: 'set "%s=%s"' % (key, value), text, flags=re.MULTILINE)
        if count != 1:
            raise SmokeFailed("local.bat has %d lines of %s" % (count, key))
    return text


def run_mwse(progress):
    """The MD5 of the MWSE.dll of a run in the text of its progress.log, or None."""
    found = re.search(r"^\[runner\] MWSE\.dll (\w+)", progress, re.MULTILINE)
    return found.group(1) if found else None


def mwse_text(digests):
    """What the note says of the MWSE.dll of the smoke, from the MD5 of each of its runs."""
    if not digests:
        return "no run of the smoke yet to name its MWSE.dll"
    if len(digests) > 1:
        return "the MWSE.dll changed during the smoke: %s" % ", ".join(sorted(digests))
    return "MWSE.dll %s" % next(iter(digests))


def set_note(name, plugin_commit, mge_commit, result, digests=()):
    """The one line of WHAT.txt."""
    stamp = datetime.datetime.now().strftime("%Y-%m-%d %H:%M")
    return "%s %s: the owner's play set, nirender %s %s, render-link %s %s, no pdb, %s; smoke: %s\n" % (
        name, stamp, PLUGIN_BRANCH, plugin_commit, MGE_BRANCH, mge_commit, mwse_text(digests), result)


def batch_answers(progress):
    """The answer of each set of a batch in the text of progress.log, and the steps that failed."""
    answers = {name: text.strip() for name, ok, text in re.findall(r"shader batch: ([\w-]+): run (true|false) ?(.*)", progress) if ok == "true"}
    failed = [name for name, ok, _ in re.findall(r"shader batch: ([\w-]+): run (true|false) ?(.*)", progress) if ok == "false"]
    failed += [name for name in re.findall(r"step (sh-[\w-]+): false", progress)]
    return answers, failed


def live_faults(progress):
    """What is wrong with the answers of batch-play-smoke.lua in progress.log, as a list of texts."""
    answers, failed = batch_answers(progress)
    faults = ["the set %s failed" % name for name in failed]

    for index in range(1, 6):
        answer = answers.get("at-%d" % index)
        match = answer and re.fullmatch(r"asked (-?\d+),(-?\d+) grid (-?\d+),(-?\d+)", answer)
        if not match:
            faults.append("the walk's cell %d: %r" % (index, answer))
        elif match.group(1, 2) != match.group(3, 4):
            faults.append("the walk's cell %d is at another grid: %s" % (index, answer))

    saved = answers.get("save", "")
    if not saved.startswith("saved: true"):
        faults.append("the save: %r" % saved)
    # The load is proven by the way back from another cell, and by the game's event.
    loaded = re.fullmatch(r"saved in (grid \S+), now (grid \S+), loads (\d+)", answers.get("loaded", ""))
    away = re.fullmatch(r"away at (grid \S+)", answers.get("away", ""))
    if not loaded or not away:
        faults.append("the load: %r after %r" % (answers.get("loaded"), answers.get("away")))
    elif away.group(1) == loaded.group(1):
        faults.append("the player did not leave the saved cell before the load: %s" % away.group(1))
    elif loaded.group(1) != loaded.group(2) or int(loaded.group(3)) < 1:
        faults.append("the load did not bring the player back: %s" % answers.get("loaded"))

    for name, menu in (("map", "MenuMap"), ("inventory", "MenuInventory"), ("character", "MenuStat")):
        if menu not in answers.get(name, ""):
            faults.append("the set %s has no %s open: %r" % (name, menu, answers.get(name)))
    return faults


def tour_faults(output, code):
    """What is wrong with the output of validate_tour.py, as a list of texts."""
    faults = []
    if code != 0:
        faults.append("validate_tour.py gave exit code %d" % code)
    pictures = re.search(r"(\d+) pictures, (\d+) failed", output)
    if not pictures:
        faults.append("validate_tour.py has no count of pictures")
    elif int(pictures.group(2)):
        faults.append("%s replays of the tour failed" % pictures.group(2))
    stops = re.search(r"(\d+) stops to replay", output)
    if not stops or int(stops.group(1)) < LEAST_STOPS:
        faults.append("the tour has too few stops: %s" % (stops.group(1) if stops else "none"))
    # A pair with no numbers (a missing picture, a size mismatch) lists first, and its exit code is 0.
    pair_rows = re.findall(r"^\s+(\S+\.png)\s+(.*)$", output, re.MULTILINE)
    unmeasured = [name for name, text in pair_rows if not re.match(r"[\d.]+ mean", text)]
    if unmeasured:
        faults.append("%d pairs have no numbers, first %s" % (len(unmeasured), unmeasured[0]))
    means = [(float(mean), name) for name, mean in re.findall(r"^\s+(\S+\.png)\s+([\d.]+) mean", output, re.MULTILINE)]
    if not means:
        faults.append("validate_tour.py has no differences of mode gpu against mode off")
    elif max(means)[0] > MEAN_LIMIT:
        faults.append("mode gpu against mode off differs by %.3f at %s, over the limit %.3f" % (*max(means), MEAN_LIMIT))
    return faults


def closing_crash(mwse_log):
    return "Morrowind has crashed" in mwse_log


# --- the slots -----------------------------------------------------------------------------

def load_slots():
    """The slot tool. A tool that does not load stops this job: unlike a hook, it holds up nobody, and a
    stop costs one rerun, while a game with no slot can spoil a timing run."""
    sys.path.insert(0, HARNESS)
    try:
        import slot
    except Exception as error:  # noqa: BLE001 - any fault of the tool means the same
        say("the slot tool did not load: %s" % error)
        sys.exit(3)
    if slot.enforcing() and not slot.session_id():
        # A take would store the text as its owner, and the runner's check would then refuse this job.
        say("enforcement is on, and this process has no session: run this from a session")
        sys.exit(3)
    return slot


def wait_for_build(slots, no_wait):
    """Waits while the state refuses a build: this tool's cargo is a child, and the hook does not see it."""
    while slots.enforcing():
        found = slots.refusal("build", slots.session_id())
        if found is None:
            return
        if no_wait:
            say("no build now: %s" % found.text)
            sys.exit(3)
        say("waiting for the build: %s" % found.text)
        time.sleep(WAIT_SECONDS)


def take_game(slots, what, no_wait):
    """Takes the game slot, and waits in its queue. It asks again at each wait, so that it keeps its place."""
    os.makedirs(slots.STATE_DIR, exist_ok=True)
    while True:
        try:
            reply = slots.take("game", "make_play_set: %s" % what, minutes=SLOT_MINUTES)
        except slots.StateBusy as error:
            say("the state of the slots is busy, and I ask again: %s" % error)
            time.sleep(WAIT_SECONDS)
            continue
        say(reply.text.splitlines()[0])
        if reply.code == slots.DONE:
            return
        if no_wait:
            sys.exit(3)
        time.sleep(WAIT_SECONDS)


def hold_run_lock(path):
    """Keeps a second run of this tool out for the whole of this one: it would move the trees and their
    cargo targets under the first. The system drops the lock when this process ends. The descriptor stays open
    on purpose. Returns it, or None when another run holds the lock."""
    descriptor = os.open(path, os.O_RDWR | os.O_CREAT)
    try:
        msvcrt.locking(descriptor, msvcrt.LK_NBLCK, 1)
    except OSError:
        os.close(descriptor)
        return None
    return descriptor


# --- the build ------------------------------------------------------------------------------

def ready_tree(name, repo, commit):
    """A detached worktree at this commit in TREES. It keeps its target folder between sets."""
    tree = os.path.join(TREES, name)
    if not os.path.exists(os.path.join(tree, ".git")):
        os.makedirs(TREES, exist_ok=True)
        git(repo, "worktree", "add", "--detach", tree, commit)
    else:
        changed = git(tree, "status", "--porcelain", "--untracked-files=no")
        if changed:
            raise SystemExit("the tree %s has changes, and nothing is built:\n%s" % (tree, changed))
        git(tree, "checkout", "--quiet", "--detach", commit)
    return tree


def build_step(command, folder, log_path):
    """Runs one cargo command with its output in the log. A set's folder needs the target of its own tree."""
    env = dict(os.environ)
    env.pop("CARGO_TARGET_DIR", None)
    say("building: %s in %s" % (" ".join(command), folder))
    with open(log_path, "a", encoding="utf-8") as log:
        log.write("\n$ %s\n" % " ".join(command))
        log.flush()
        done = subprocess.run(command, cwd=folder, env=env, stdout=log, stderr=subprocess.STDOUT)
    if done.returncode != 0:
        raise SystemExit("the build failed with exit code %d: see %s" % (done.returncode, log_path))


def new_set_folder():
    """Makes the next set folder. os.mkdir fails on a name that another session has just made."""
    while True:
        name = "f%d" % next_set_number(os.listdir(SETS))
        try:
            os.mkdir(os.path.join(SETS, name))
        except FileExistsError:
            continue
        return name


def copy_stage(stage, plugin_tree, folder, plugin_commit):
    """Copies the stage to the set, with no pdb, and the plugin beside it. Returns the plugin's path."""
    shutil.copytree(stage, folder, dirs_exist_ok=True, ignore=shutil.ignore_patterns("*.pdb"))
    built = os.path.join(plugin_tree, PLUGIN_TARGET)
    staged = os.path.join(folder, "Data Files", "mwse", "lib", "akulakhan.dll")
    if md5_of(built) != md5_of(staged):
        raise SystemExit("the plugin of the stage is not the plugin of the tree: %s" % staged)
    plugin_dll = os.path.join(folder, "akulakhan-%s.dll" % plugin_commit)
    shutil.copy2(built, plugin_dll)
    for name in ("d3d8.dll", "numidium.exe"):
        if md5_of(os.path.join(stage, name)) != md5_of(os.path.join(folder, name)):
            raise SystemExit("the copy of %s differs from the stage" % name)
    pdbs = glob.glob(os.path.join(folder, "**", "*.pdb"), recursive=True)
    if pdbs:
        raise SystemExit("the set holds a pdb: %s" % pdbs[0])
    return plugin_dll


# --- the smoke ------------------------------------------------------------------------------

def wait_clean():
    """Waits until the instance is free of the run before: no game, no Mod Organizer of it, no Harness mod."""
    sys.path.insert(0, HARNESS)
    import harness

    for _ in range(90):
        with open(TESTING_MODLIST, encoding="utf-8", errors="replace") as file:
            harness_on = any(line.startswith("+" + harness.RUN_MOD_PREFIX) for line in file)
        if not harness_on and not harness.processes("Morrowind.exe") and not harness.testing_mo2():
            time.sleep(2)
            return
        time.sleep(2)
    raise SmokeFailed("the instance did not become free in 180 s")


def newest_run(tree, label, since):
    """The folder of the run that began after `since`. The tree's results stay between sets."""
    folders = [path for path in sorted(glob.glob(os.path.join(tree, "results", "*-" + label))) if os.path.getmtime(path) >= since]
    if not folders:
        raise SmokeFailed("no new run folder of %s" % label)
    return folders[-1]


def keep_run(run):
    """Copies a run to the kept runs: the results of a tree in TREES are a real folder that a cleanup may delete."""
    target = os.path.join(KEPT_RUNS % datetime.date.today().isoformat(), os.path.basename(run))
    shutil.copytree(run, target, dirs_exist_ok=True)
    return target


def run_runner(tree, label, folder, plugin_dll, budget, *args):
    """Runs the runner of the tree, whose Lua and tools are of the built commit. Returns the run folder, kept
    also when the run failed, as a failed set's reason names it."""
    wait_clean()
    log = os.path.join(tree, "results", label + ".txt")
    say("smoke: %s" % label)
    began = time.time()
    # With the renderer's probe on, as the owner's start has it: a probe that says no is a fault of the set.
    done = subprocess.run([sys.executable, "tools/run_own_device.py", "--label", label, "--plugin-dll", plugin_dll, "--mge-build", folder,
                           "--no-build", "--budget", str(budget), *args, "--log", log],
                          cwd=tree, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    run = newest_run(tree, label, began)
    say("smoke: the run is kept in %s" % keep_run(run))
    digest = run_mwse(read_text(os.path.join(run, "progress.log")))
    # A run with no such line is not named here: a smoke with no name says so in the note.
    if digest:
        SMOKE_MWSE.add(digest)
    if done.returncode != 0:
        raise SmokeFailed("the runner of %s gave exit code %d: %s" % (label, done.returncode, run))
    return run


def read_text(path):
    with open(path, encoding="utf-8", errors="replace") as file:
        return file.read()


def check_run_files(run, notes):
    """The checks of every run: no GPU error, and a crash counts only before the last step ended."""
    gpu_errors = read_text(os.path.join(run, "host.txt")).count("gpu error")
    if gpu_errors:
        raise SmokeFailed("%d gpu errors in %s" % (gpu_errors, run))
    # The runner's exit code 0 says that the last step ended, so a crash in this log came at the close.
    if closing_crash(read_text(os.path.join(run, "MWSE.log"))):
        notes.append("crash at the close of %s: the parked fault at exit, or a new one, as this cannot tell" % os.path.basename(run))


def picture_is_black(path):
    # Here and not at the top: only this needs Pillow. picture_diff finds its DLLs under -I.
    import picture_diff  # noqa: F401
    from PIL import Image, ImageStat

    with Image.open(path) as picture:
        return max(ImageStat.Stat(picture.convert("RGB")).mean) < BLACK_LEVEL


def smoke_live(tree, folder, plugin_dll, notes):
    run = run_runner(tree, "playset-live", folder, plugin_dll, 400, "--steps", "exterior,shaders", "--exterior", "Balmora",
                     "--hour", "12", "--weather", "0", "--shader-batch", BATCH)
    check_run_files(run, notes)
    faults = live_faults(read_text(os.path.join(run, "progress.log")))
    if faults:
        raise SmokeFailed("%s: %s" % (run, "; ".join(faults)))
    pictures = glob.glob(os.path.join(run, "host-sh-*.png"))
    if len(pictures) < 10:
        raise SmokeFailed("%s has only %d step pictures" % (run, len(pictures)))
    black = [os.path.basename(path) for path in pictures if picture_is_black(path)]
    if black:
        raise SmokeFailed("%s has black pictures: %s" % (run, ", ".join(black)))
    if os.path.exists(SMOKE_SAVE):
        os.remove(SMOKE_SAVE)
    return run


def smoke_gate(tree, folder):
    """The replays against the accepted pictures, with no game. Its exit code is honest since 849c9b3."""
    say("smoke: the gate")
    done = subprocess.run([sys.executable, "-I", "tools/validate_retained.py", "baseline", "--mge-build", folder, "--baseline", BASELINE],
                          cwd=tree, capture_output=True, text=True, errors="replace")
    keep = os.path.join(KEPT_RUNS % datetime.date.today().isoformat(), "gate-%s.txt" % os.path.basename(folder))
    os.makedirs(os.path.dirname(keep), exist_ok=True)
    with open(keep, "w", encoding="utf-8") as file:
        file.write(done.stdout + done.stderr)
    if done.returncode != 0:
        raise SmokeFailed("the gate gave exit code %d, see %s: %s" % (done.returncode, keep, (done.stdout + done.stderr).strip()[-200:]))
    return os.path.basename(keep)


def smoke_tour(tree, folder, plugin_dll, notes):
    run = run_runner(tree, "playset-tour", folder, plugin_dll, 900, "--steps", "tour", "--tour-cells", TOUR_CELLS, "--tour-dual")
    check_run_files(run, notes)
    done = subprocess.run([sys.executable, "tools/validate_tour.py", run, "--mge-build", folder, "--jobs", "3"],
                          cwd=tree, capture_output=True, text=True, errors="replace")
    with open(os.path.join(run, "validate.txt"), "w", encoding="utf-8") as file:
        file.write(done.stdout + done.stderr)
    keep_run(run)
    faults = tour_faults(done.stdout, done.returncode)
    if faults:
        raise SmokeFailed("%s: %s" % (run, "; ".join(faults)))
    return run


def smoke_owner_bats(tree, folder, plugin_dll, owner_local_bat, notes):
    """Runs both real bats through the real tools/play.py up to the game's start. The tree's local.bat names a
    small command in place of Python: it adds --stage-only, and so the last lines of the bat do not run.
    Returns the number of bats that reached the game's start."""
    wait_clean()
    tested = 0
    owner_values = local_bat_values(read_text(owner_local_bat))
    outlander = owner_values.get("OUTLANDER_INSTANCE", "")
    # Without this the bat looks for it next to the tree, where no moreFPS is.
    livecoding = owner_values.get("LIVECODING", LIVECODING)
    outlander_modlist = os.path.join(outlander, "profiles", OUTLANDER_PROFILE, "modlist.txt")
    modlists = [path for path in (TESTING_MODLIST, outlander_modlist) if os.path.isfile(path)]
    before = {path: md5_of(path) for path in modlists}

    stage_only = os.path.join(tree, "stage_only_python.cmd")
    with open(stage_only, "w", encoding="ascii", newline="\r\n") as file:
        file.write('@"%s" %%* --stage-only\n' % sys.executable)
    with open(os.path.join(tree, "local.bat"), "w", encoding="ascii", newline="\r\n") as file:
        file.write('@set "PYTHON=%s"\n@set "MGE_BUILD=%s"\n@set "NIRENDER_PLUGIN_DLL=%s"\n@set "OUTLANDER_INSTANCE=%s"\n@set "LIVECODING=%s"\n' % (
            stage_only, folder, plugin_dll, outlander, livecoding))

    for bat in OWNER_BATS:
        bat_path = os.path.join(tree, bat)
        if BAT_CLEARS not in read_text(bat_path):
            notes.append("%s does not clear the session variable yet, and is not tested" % bat)
            continue
        done = subprocess.run(["cmd", "/c", bat_path], stdin=subprocess.DEVNULL, capture_output=True, text=True, errors="replace", timeout=180)
        output = done.stdout + done.stderr
        if "Refused by the slot check" in output:
            raise SmokeFailed("%s was refused by the slot check: %s" % (bat, output[-300:]))
        if "Mod Organizer is open" in output:
            notes.append("%s is not tested: the instance's Mod Organizer is open" % bat)
            continue
        if "no game (--stage-only)" not in output:
            raise SmokeFailed("%s did not reach the game's start: %s" % (bat, output[-400:]))
        say("smoke: %s reached the game's start" % bat)
        tested += 1

    for path, digest in before.items():
        if md5_of(path) != digest:
            raise SmokeFailed("the bats changed the mod list %s" % path)
    return tested


def run_smoke(tree, folder, plugin_dll, owner_local_bat, notes):
    """The quick part first: a fault costs less time."""
    live = smoke_live(tree, folder, plugin_dll, notes)
    gate = smoke_gate(tree, folder)
    tour = smoke_tour(tree, folder, plugin_dll, notes)
    tested = smoke_owner_bats(tree, folder, plugin_dll, owner_local_bat, notes)
    return "passed (%s, %s, %s, %d of %d bats)" % (os.path.basename(live), gate, os.path.basename(tour), tested, len(OWNER_BATS))


def write_local_bat(path, set_folder, plugin_dll):
    """Replaces local.bat in one step. A game that starts now reads the old text or the new. The line ends stay."""
    with open(path, encoding="utf-8", errors="replace", newline="") as file:
        text = file.read()
    updated = with_set_lines(text, set_folder.replace("/", "\\"), plugin_dll.replace("/", "\\"))
    temporary = path + ".new"
    with open(temporary, "w", encoding="ascii", newline="") as file:
        file.write(updated)
    os.replace(temporary, path)


def current_set_commits(owner_local_bat):
    """The commits in the note of the set that local.bat names, or None."""
    if not os.path.isfile(owner_local_bat):
        return None
    folder = local_bat_values(read_text(owner_local_bat)).get("MGE_BUILD", "")
    note_path = os.path.join(folder, "WHAT.txt")
    if not os.path.isfile(note_path):
        return None
    match = re.search(r"nirender %s (\w+), render-link %s (\w+),.*smoke: passed" % (PLUGIN_BRANCH, re.escape(MGE_BRANCH)), read_text(note_path))
    return match.groups() if match else None


# --- the tests of the pure parts ------------------------------------------------------------

def self_test():
    assert next_set_number(["f99", "f100", "f7", "other", "f12x"]) == 101
    assert next_set_number([]) == 1
    assert tips_moved(("a", "b"), ("a", "b")) == ""
    assert PLUGIN_BRANCH in tips_moved(("a", "b"), ("c", "b")) and MGE_BRANCH not in tips_moved(("a", "b"), ("c", "b"))
    assert MGE_BRANCH in tips_moved(("a", "b"), ("a", "d"))

    text = '@echo off\r\nset "PYTHON=C:\\p.exe"\r\nset "MGE_BUILD=E:\\a\\f185"\r\nset "NIRENDER_PLUGIN_DLL=E:\\a\\f185\\x.dll"\r\nset "OUTLANDER_INSTANCE=D:\\o"\r\n'
    updated = with_set_lines(text, "E:\\a\\f186", "E:\\a\\f186\\y.dll")
    assert local_bat_values(updated) == {"PYTHON": "C:\\p.exe", "MGE_BUILD": "E:\\a\\f186", "NIRENDER_PLUGIN_DLL": "E:\\a\\f186\\y.dll", "OUTLANDER_INSTANCE": "D:\\o"}
    assert updated.count("\r\n") == text.count("\r\n")
    try:
        with_set_lines('set "MGE_BUILD=x"\r\n', "a", "b")
    except SmokeFailed:
        pass
    else:
        raise AssertionError("a missing line of local.bat was taken")

    with tempfile.TemporaryDirectory() as folder:
        bat = os.path.join(folder, "local.bat")
        with open(bat, "wb") as file:
            file.write(text.encode("ascii"))
        write_local_bat(bat, "E:/a/f186", "E:/a/f186/y.dll")
        with open(bat, "rb") as file:
            written = file.read()
        assert written.count(b"\r\n") == text.count("\r\n") and b'set "MGE_BUILD=E:\\a\\f186"' in written, written

    good = "\n".join(["[t] shader batch: at-%d: run true asked %d,0 grid %d,0" % (i, i, i) for i in range(1, 6)] + [
        "[t] shader batch: save: run true saved: true grid 1,0", "[t] shader batch: away: run true away at grid 2,0",
        "[t] shader batch: loaded: run true saved in grid 1,0, now grid 1,0, loads 1",
        "[t] shader batch: map: run true open: MenuMulti, MenuMap", "[t] shader batch: inventory: run true open: MenuInventory",
        "[t] shader batch: character: run true open: MenuStat", "[t] step sh-map: true "])
    assert live_faults(good) == [], live_faults(good)
    assert any("another grid" in fault for fault in live_faults(good.replace("asked 2,0 grid 2,0", "asked 2,0 grid 9,0")))
    assert any("did not bring" in fault for fault in live_faults(good.replace("now grid 1,0", "now grid 2,0")))
    assert any("did not bring" in fault for fault in live_faults(good.replace("loads 1", "loads 0")))
    assert any("did not leave" in fault for fault in live_faults(good.replace("away at grid 2,0", "away at grid 1,0")))
    assert any("the load" in fault for fault in live_faults(good.replace("shader batch: away:", "shader batch: other:")))
    assert any("MenuMap" in fault for fault in live_faults(good.replace("MenuMulti, MenuMap", "MenuMulti")))
    assert any("the set map failed" in fault for fault in live_faults(good + "\n[t] shader batch: map: run false broken"))
    assert any("sh-map failed" in fault for fault in live_faults(good.replace("step sh-map: true", "step sh-map: false")))
    assert live_faults("") != []

    tour = "103 stops to replay, gpu and off at each\n206 pictures, 0 failed\n  2-13.png    0.001 mean, 0.08 % different\n  1-01.png   0.000 mean, 0.0 % different\npairs: 18 of 103 equal\n"
    assert tour_faults(tour, 0) == [], tour_faults(tour, 0)
    assert tour_faults(tour, 1) != []
    assert tour_faults(tour.replace("0 failed", "3 failed"), 0) != []
    assert tour_faults(tour.replace("103 stops", "4 stops"), 0) != []
    assert tour_faults(tour.replace("103 stops", "60 stops"), 0) != []
    assert tour_faults(tour.replace("0.001 mean", "0.200 mean"), 0) != []
    assert any("no numbers" in fault for fault in tour_faults(tour.replace("0.001 mean, 0.08 % different", "missing"), 0))
    assert any("no numbers" in fault for fault in tour_faults(tour.replace("0.001 mean, 0.08 % different", "the sizes differ"), 0))
    assert tour_faults("", 0) != []

    note = set_note("f186", "abc", "def", "passed (a, b, 2 of 2 bats)", {"aae6"})
    assert note.count("\n") == 1 and note.endswith("2 of 2 bats)\n") and ", MWSE.dll aae6; smoke:" in note, note
    assert "no run of the smoke" in set_note("f186", "abc", "def", "pending")
    assert "changed during the smoke: a1, b2" in mwse_text({"b2", "a1"})
    assert run_mwse("[runner] instance D:\\m, profile Default\n[runner] MWSE.dll aae6\n[t] MWSE.dll zz\n") == "aae6"
    assert run_mwse("[t] MWSE.dll zz\n") is None
    with tempfile.TemporaryDirectory() as folder:
        with open(os.path.join(folder, "WHAT.txt"), "w", encoding="ascii") as file:
            file.write(note)
        bat = os.path.join(folder, "local.bat")
        with open(bat, "w", encoding="ascii") as file:
            file.write('set "MGE_BUILD=%s"\n' % folder)
        assert current_set_commits(bat) == ("abc", "def")
        with open(os.path.join(folder, "WHAT.txt"), "w", encoding="ascii") as file:
            file.write(set_note("f186", "abc", "def", "FAILED: a reason"))
        assert current_set_commits(bat) is None, "a failed set is no current set"

    # A second run stops, in another process as the lock is per handle, and the lock goes with the first.
    with tempfile.TemporaryDirectory() as folder:
        lock = os.path.join(folder, "lock")
        first = hold_run_lock(lock)
        assert first is not None
        probe = "import sys; sys.path.insert(0, %r); import make_play_set as m; sys.exit(0 if m.hold_run_lock(%r) is None else 1)" % (
            os.path.dirname(os.path.abspath(__file__)), lock)
        assert subprocess.run([sys.executable, "-I", "-c", probe]).returncode == 0, "a second run was let in"
        msvcrt.locking(first, msvcrt.LK_UNLCK, 1)
        os.close(first)
        assert subprocess.run([sys.executable, "-I", "-c", probe]).returncode == 1, "the lock stayed after the first run"

    # A run is kept under its own name, and again without a fault: the validation is added to it later.
    with tempfile.TemporaryDirectory() as folder:
        global KEPT_RUNS
        kept_before = KEPT_RUNS
        KEPT_RUNS = os.path.join(folder, "runs-%s")
        run = os.path.join(folder, "20261102-000000-x")
        os.mkdir(run)
        with open(os.path.join(run, "a.txt"), "w", encoding="ascii") as file:
            file.write("a")
        kept = keep_run(run)
        keep_run(run)
        assert os.path.basename(kept) == os.path.basename(run) and os.path.isfile(os.path.join(kept, "a.txt"))
        KEPT_RUNS = kept_before

    # The picture rule, with a black and a real picture.
    import picture_diff  # noqa: F401
    from PIL import Image

    with tempfile.TemporaryDirectory() as folder:
        black = os.path.join(folder, "black.png")
        Image.new("RGB", (8, 8), (1, 1, 1)).save(black)
        real = os.path.join(folder, "real.png")
        Image.new("RGB", (8, 8), (90, 80, 70)).save(real)
        assert picture_is_black(black) and not picture_is_black(real)
    print("make_play_set self-test: ok")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--force", action="store_true", help="make a set when local.bat's set has these tips already")
    parser.add_argument("--no-wait", action="store_true", help="stop with exit code 3 where this would wait for a slot")
    parser.add_argument("--keep-slot", action="store_true", help="after a pass, keep the game slot for this session's next work, and release it yourself")
    parser.add_argument("--build-only", action="store_true", help="only the build in the trees: no set, no game")
    parser.add_argument("--bats-only", action="store_true", help="the step of the owner's start alone, on the set that local.bat names")
    parser.add_argument("--self-test", action="store_true", help="the tests of the pure parts: no build, no game")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0

    owner_local_bat = os.path.join(PLUGIN_REPO, "local.bat")
    plugin_commit, mge_commit = current_tips()
    say("the tips: nirender %s %s, render-link %s %s" % (PLUGIN_BRANCH, plugin_commit, MGE_BRANCH, mge_commit))
    if not args.force and not args.bats_only and current_set_commits(owner_local_bat) == (plugin_commit, mge_commit):
        say("local.bat names a set of these tips, and nothing is made")
        return 0

    slots = load_slots()
    os.makedirs(TREES, exist_ok=True)
    if hold_run_lock(os.path.join(TREES, "make_play_set.lock")) is None:
        say("another run of this tool is at work, and this one stops")
        return 3
    plugin_tree = ready_tree("nirender-plugin", PLUGIN_REPO, plugin_commit)
    mge_tree = ready_tree("MGE-XE-G7-render-link", MGE_REPO, mge_commit)
    os.makedirs(os.path.join(plugin_tree, "results"), exist_ok=True)
    # The runner of the tree finds the harness by this key: its siblings hold no moreFPS.
    with open(os.path.join(plugin_tree, "local.json"), "w", encoding="ascii") as file:
        json.dump({"harness": HARNESS}, file)

    if args.bats_only:
        values = local_bat_values(read_text(owner_local_bat))
        take_game(slots, "the step of the owner's start", args.no_wait)
        notes = []
        try:
            tested = smoke_owner_bats(plugin_tree, values["MGE_BUILD"], values["NIRENDER_PLUGIN_DLL"], owner_local_bat, notes)
        except SmokeFailed as failure:
            say("FAILED: %s" % failure)
            return 1
        finally:
            say(slots.release("game").text)
        say("%d of %d bats reached the game's start%s" % (tested, len(OWNER_BATS), "; " + "; ".join(notes) if notes else ""))
        return 0

    # A game by the bat takes the Lua of mod/ from the owner's checkout: a plugin and a Lua of different commits
    # are a wrong play set.
    head = git(PLUGIN_REPO, "rev-parse", "--short=7", "HEAD")
    checkout_note = ""
    if head != plugin_commit:
        if not args.force:
            say("the owner's checkout is at %s, not %s, and nothing is built: use --force to make this set anyway" % (head, plugin_commit))
            return 1
        checkout_note = "; the owner's checkout was at %s, so a game by the bat takes another Lua" % head

    wait_for_build(slots, args.no_wait)

    def build_trees():
        build_log = os.path.join(TREES, "build-%s-%s.log" % (plugin_commit, mge_commit))
        slots.note_active("build", time.time())
        build_step(["cargo", "build", "--release"], plugin_tree, build_log)
        slots.note_active("build", time.time())
        build_step(["cargo", "xtask", "dist"], mge_tree, build_log)
        return build_log

    build_log = build_trees()
    if args.build_only:
        say("built, with no set: %s" % build_log)
        return 0

    # The slot first: a stop in the queue leaves no set folder behind.
    take_game(slots, "the smoke of the next set", args.no_wait)
    name = None
    notes = []
    passed = False

    def write_note(result):
        with open(os.path.join(SETS, name, "WHAT.txt"), "w", encoding="ascii", errors="backslashreplace") as file:
            file.write(set_note(name, plugin_commit, mge_commit, result, SMOKE_MWSE))

    try:
        # The wait for the slot can be long, and the set must be of the tips when the smoke starts (f187 was
        # an hour old). A build here holds the slot, and it is a plain hold: builds may run beside it anyway.
        moved = tips_moved((plugin_commit, mge_commit), current_tips())
        if moved:
            say("%s during the wait for the slot, and I build again" % moved)
            plugin_commit, mge_commit = current_tips()
            plugin_tree = ready_tree("nirender-plugin", PLUGIN_REPO, plugin_commit)
            mge_tree = ready_tree("MGE-XE-G7-render-link", MGE_REPO, mge_commit)
            build_trees()
            head = git(PLUGIN_REPO, "rev-parse", "--short=7", "HEAD")
            if head != plugin_commit:
                checkout_note = "; the owner's checkout was at %s, so a game by the bat takes another Lua" % head
                say(checkout_note[2:])

        name = new_set_folder()
        folder = os.path.join(SETS, name).replace("\\", "/")
        with open(os.path.join(folder, "WHAT.txt"), "x", encoding="ascii") as file:
            file.write(set_note(name, plugin_commit, mge_commit, "pending"))
        plugin_dll = copy_stage(os.path.join(mge_tree, "target", "dist-staging"), plugin_tree, folder, plugin_commit).replace("\\", "/")
        say("the set: %s" % folder)
        result = run_smoke(plugin_tree, folder, plugin_dll, owner_local_bat, notes)
        passed = True
    except SmokeFailed as failure:
        reason = "FAILED: %s" % failure
        write_note(reason)
        say("%s: %s. local.bat stays on the last good set." % (name, reason))
        return 1
    except BaseException as failure:
        # A note must not stay on "pending" (a stop of a tool, an interrupt).
        if name:
            write_note("FAILED: stopped by %r" % failure)
        raise
    finally:
        if passed and args.keep_slot:
            say("game: kept for the next work of this session")
        else:
            say(slots.release("game").text)

    # A set that passed stays good, but it must not read as the tips.
    moved = tips_moved((plugin_commit, mge_commit), current_tips())
    if moved:
        notes.append("NOT THE TIPS: %s during the smoke" % moved)

    # local.bat first: a note that says passed then always has a set that local.bat names.
    write_local_bat(owner_local_bat, folder, plugin_dll)
    write_note(result + checkout_note + ("; " + "; ".join(notes) if notes else ""))
    say(read_text(os.path.join(folder, "WHAT.txt")).strip())
    say("local.bat now names %s" % name)
    return 0


if __name__ == "__main__":
    sys.exit(main())
