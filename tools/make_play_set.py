"""Makes the owner's play set: a build of what is ready now, tested, and named in current.bat of the sets.

    python -I tools/make_play_set.py [--force] [--no-wait]
    python -I tools/make_play_set.py --bats-only    # the step of the owner's start alone, on the current set
    python -I tools/make_play_set.py --self-test    # the tests of the pure parts: no build, no game

It asks the maker (tools/build_set.py, `make`) for the sealed set of the tips (W521): nirender-plugin `master`,
render-link `feature/wgpu-renderer` at their local tips, and the MWSE commit that the test installs run. The maker
builds what no set has yet, under its own build hold and its run lock, keeps the pdbs outside the sets and seals the
set; a set of these commits that is there already is taken as it is. This tool adds the set to the symbol store. The
smoke runs the tools of a detached worktree C:/tmp/playset/nirender-plugin at the set's nirender commit, which builds
nothing (the maker's trees can move under another caller's make). Then it takes the game slot and runs the smoke on
the testing instance; each run takes the set's own plugin (no --plugin-dll):

- live: tools/run_own_device.py with batch-play-smoke.lua: teleports over 5 cell borders, a save, a move to
  another cell, a load, the map, the inventory and the character. The runner must end well, each answer of
  the game must be the asked one, no step picture may be black, and the helper's check "gpu errors" in
  host.txt must have 0 faults (its line `check gpu errors: N judged, M faults`; no line is a failure).
- gate: tools/validate_retained.py baseline, with no game: the replays against the accepted pictures. This
  sees a fault that both modes share. A planned new look stops the set until a baseline is accepted.
- tour: the 3 test cells with --tour-dual, and tools/validate_tour.py --least-stops: the tool gives the
  verdict by its exit code (no picture may fail, no pair may lack its numbers, no pair of mode gpu against
  mode off may differ by more than its MEAN_LIMIT, and the tour must have its stops).

A replay needs the whole build side (the owner's D139), and the game slot does not give it. No session waits for
one slot while it holds the other (the owner's D208). So the set takes the game and the whole build side together
(`take both`) before its first game stage, and holds both through every stage up to the tour's replays, with no
release between them (W405: before, the set gave the game back before each replay stage, and another session's
`take --wait` got the game in that gap). After the tour's replays it gives the build side back and holds the game
alone to its end. A thread notes the work of both slots each minute (merge_step's notes), as a hold lapses with no
work seen; before each stage the holds are checked, and one that lapsed is taken again only when it is free at once.
A hold that is gone, or a replay that the slot check refused, is "NOT RUN: slot refused" with exit code 3: no
picture was checked, so the set did not fail, and it does not become the play set.
- the owner's start: both play-manual-check*.bat with the real tools/play.py and --stage-only, so that
  they stop just before the game. The bat must pass the slot check, with this session's variable in its
  terminal, and each instance's mod list must be as before. A bat that cannot be tested because the instance's
  Mod Organizer is open makes the smoke NOT RUN (W524 F-T-1).

This is the smoke of f185 with two changes: a batch replaces the live bridge, as a script cannot call it,
and the gate is added. A crash when the game closes, after the last step, fails the set: the runner's exit
code 5 (W417) names it, and the set's note says FAILED with the run (since W431 closed, D462). A crash at the
close that only MWSE.log shows, with the runner's exit code 0, stays a note.

Only on a pass, it writes current.bat in E:/nirender-keep/mgebuilds with the same writer as deploy_rest.py
(`sets.write_current_bat`, W500, the owner's D461): one line, MGE_BUILD of the set (W521). The
owner's local.bat calls current.bat, and the tool writes no local.bat. A failed smoke leaves current.bat on the
last good set, says why, and exit code 1; a NOT RUN leaves it too, with exit code 3. A sealed set holds no file
outside its manifest, so the note of each smoke is a file outside the sets: E:/nirender-keep/playset-notes/
<set>-<time>.txt, one line, which names the three commits, the MD5 of the MWSE.dll that the smoke ran on (from the
lines of the runner in progress.log) and the result. A set can be smoked again, with a note for each smoke.
No set is deleted: older sets stay until the owner's cleanup, and a failed set stays with its note. The
only file it deletes is the smoke's own save. The smoke's runs are copied to E:/nirender-keep/runs-<date>.

It reads the tips again when it holds the slot, and asks the maker again if they moved, as the wait in the queue
can be long; that make runs in the set's own hold of the whole build side. If they move during the smoke, the note
says that the set is not at the tips.

The current set is the one that current.bat names, else the one that local.bat names (`clean_common.current_set`).
When it has the tips (the three commits of its manifest) and a note of a smoke that passed, the tool does nothing,
unless --force. The owner's checkout must be at the plugin's commit, as a game by the bat takes the Lua of mod/
from there: otherwise it stops, unless --force, and the note says so. The maker's build hold and the smoke wait in
their queues, one after the other: with --no-wait each stops with exit code 3. It needs the slot tool, and a
session when enforcement is on: nobody waits for this job, and a game with no slot can spoil a timing run.
--build-only is gone: the maker makes a set alone (tools/build_set.py).
"""
import argparse
import contextlib
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
import types

# The modules of this folder, also under `python -I`.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import GPU_ERRORS, SIBLINGS, check_problems, docs_place, local, main_checkout  # noqa: E402
from sets import CURRENT_BAT, KIND_USUAL, MANIFEST, SETS, SYMBOL_FILES, SetError, verify, write_current_bat  # noqa: E402  (W500, W521)
import clean_common  # noqa: E402  (the current set, W500)
from validate_tour import NOT_RUN, NOT_RUN_CODE, verdict_in as tour_verdict_in  # noqa: E402
import symbols  # noqa: E402
# build_set.py imports this module at its top, so this module imports build_set inside its functions.

REPOS = local("repos", SIBLINGS)
PLUGIN_REPO = os.path.join(REPOS, "nirender-plugin")
PLUGIN_BRANCH = "master"
MGE_BRANCH = "feature/wgpu-renderer"
# The tree of the smoke's tools (no build) and the run lock of this tool.
TREES = "C:/tmp/playset"
# The note of each smoke, outside the sets: a sealed set holds no file outside its manifest (W524 F-T-4).
PLAYSET_NOTES = os.path.join(os.path.dirname(SETS), "playset-notes")
# The pdbs of the sets, one folder for each set (build_set.py keeps them). It must not be inside SETS: no pdb is
# deployed.
PDB_KEEP = "E:/nirender-keep/pdb"
# Each binary of a set (its path in the set), the tree that built it, and its pdb in that tree (build_set.py's
# Trees.pdb_sources).
PDBS = (
    ("d3d8.dll", "mge", os.path.join("target", "i686-pc-windows-msvc", "release-dll", "d3d8.pdb")),
    ("dinput8.dll", "mge", os.path.join("target", "i686-pc-windows-msvc", "release-dll", "dinput8.pdb")),
    ("numidium.exe", "mge", os.path.join("target", "x86_64-pc-windows-msvc", "release", "numidium.pdb")),
    (os.path.join("Data Files", "mwse", "lib", "akulakhan.dll"), "plugin", os.path.join("target", "i686-pc-windows-msvc", "release", "akulakhan.pdb")),
)
KEPT_RUNS = "E:/nirender-keep/runs-%s"
# The docs checkout beside the main checkout, or the one of the key "docs" (common.docs_root).
HARNESS = docs_place("docs", "scripts", "test-harness-2026-10-03")
BATCH = docs_place("docs", "Numidium Project", "scripts", "wgpu-scene-audit", "batch-play-smoke.lua")
LIVECODING = docs_place("docs", "Numidium Project", "scripts", "wgpu-scene-audit", "livecoding-mark.lua")
BASELINE_PARTS = ("results", "baseline-records")


def has_captures(folder):
    """Whether a baseline folder holds the captures that the replay gate replays."""
    return os.path.isdir(folder) and any(name.endswith(".nircap") for name in os.listdir(folder))


def accepted_baseline(tree, main):
    """The folder of the gate's accepted captures: the tree's own when it has captures, else the main checkout's
    (`main`). results/ is not tracked, so a worktree in C:/tmp has none (2026-11-09: a merge_step of a worktree group
    failed its gate at 0 s, after its builds). The tree's own when neither has captures, so that a check names it."""
    near = os.path.join(tree, *BASELINE_PARTS)
    if has_captures(near) or not main:
        return near
    far = os.path.join(main, *BASELINE_PARTS)
    return far if has_captures(far) else near


BASELINE = accepted_baseline(PLUGIN_REPO, main_checkout())
TOUR_CELLS = "@nifTest_1;@nifTest_2;@nifTest_3"
TESTING_MODLIST = "D:/Modlists/Morrowind75testing/profiles/Default/modlist.txt"
OUTLANDER_PROFILE = "Outlander"
SMOKE_SAVE = "D:/Modlists/Morrowind75testing/profiles/Default/saves/playsmoke.ess"
WAIT_SECONDS = 60
SLOT_MINUTES = 45
# The `what` of the label of the set's game hold. Each take of the game during the smoke has this label: slot.py
# refuses a take of the holder's session with another label.
SMOKE_GAME = "the smoke of the next set"
# The `what` of the game hold of --bats-only.
OWNER_START_GAME = "the step of the owner's start"
# The `what` of the maker's build hold, before the set's hold.
SET_BUILD = "the build of the next set"
# Whether the set's hold (`take_set_hold`) still has the whole build side: it gives it back after the last replay.
SET_HOLD = {"build": False}
# The log of merge_step's keep_holds: its lines go to the console only here.
QUIET_LOG = types.SimpleNamespace(write=lambda text: None, flush=lambda: None)
# The tour has 103 stops: a much shorter tour did not run through. validate_tour.py judges it (--least-stops), with
# the limit of a pair of mode gpu against mode off, whose one home is that tool (MEAN_LIMIT, W526).
LEAST_STOPS = 100
# Below this mean level (0 to 255) a step picture counts as black.
BLACK_LEVEL = 3.0
OWNER_BATS = ("play-manual-check.bat", "play-manual-check-outlander.bat")
BAT_CLEARS = 'set "CLAUDE_CODE_SESSION_ID="'
# The MD5 of the MWSE.dll of each run of this smoke.
SMOKE_MWSE = set()


class SmokeFailed(Exception):
    """The reason why a set does not become the play set."""


class SmokeNotRun(Exception):
    """A stage that did not run, as the slots refused it: no picture of it was checked, so the set did not
    fail. It does not become the play set."""


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

def tip_commits():
    """The full commits of the tips now, {repo: commit}: the two branches and the MWSE commit of the test installs
    (build_set.tips). A tip that is not known raises build_set.SetFault."""
    import build_set

    return {repo: build_set.resolve(build_set.SOURCE_REPOS[repo], name) for repo, name in build_set.tips().items()}


def tips_moved(built, now):
    """A text that says which tips differ from the built commits ({repo: commit}), or an empty text."""
    moved = ["%s moved from %s to %s" % (repo, built[repo][:8], now[repo][:8]) for repo in built if built[repo] != now[repo]]
    return ", ".join(moved)


def local_bat_values(text):
    """The values of the set lines in the text of local.bat, as a dict."""
    return dict(re.findall(r'^set "(\w+)=([^"\r\n]*)"', text, re.MULTILINE))


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


def set_note(name, commits, result, digests=()):
    """The one line of a smoke's note: the set, its commits ({repo: commit}), the MWSE.dll that the smoke ran on, and
    the result."""
    stamp = datetime.datetime.now().strftime("%Y-%m-%d %H:%M")
    return "%s %s: the owner's play set, nirender %s %s, render-link %s %s, MWSE %s, %s; smoke: %s\n" % (
        name, stamp, PLUGIN_BRANCH, commits["nirender"][:8], MGE_BRANCH, commits["render-link"][:8], commits["mwse"][:8],
        mwse_text(digests), result)


def note_path(name, when, notes=PLAYSET_NOTES):
    """The note of the smoke of the set `name` that began at `when` (a time.struct_time)."""
    return os.path.join(notes, "%s-%s.txt" % (name, time.strftime("%Y%m%d-%H%M%S", when)))


def newest_note(name, notes=PLAYSET_NOTES):
    """The text of the newest note of the set `name`, or None. The newest note decides (verdict 507 addendum 7): an
    older "passed" with a later "FAILED" does not count."""
    pattern = re.compile(re.escape(name) + r"-\d{8}-\d{6}\.txt")
    try:
        found = sorted(entry for entry in os.listdir(notes) if pattern.fullmatch(entry))
    except FileNotFoundError:
        return None
    return read_text(os.path.join(notes, found[-1])) if found else None


def note_says_passed(text):
    return bool(text) and "; smoke: passed" in text


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
    """What validate_tour.py says is wrong, as a list of texts. The tool's exit code is its verdict (W526): it judges
    its pictures, its replays' fault codes, each pair of mode gpu against mode off against its limit, and its stops
    (--least-stops), and its last line says why. This tool reads the code, not the table. NOT_RUN_CODE is no fault
    here: the caller makes it a NOT RUN. An exit code 0 without the tool's PASS line is a fault: a tool from
    before the verdict judged no pair."""
    said = tour_verdict_in(output)
    if code == NOT_RUN_CODE:
        return []
    if code != 0:
        return ["validate_tour.py gave exit code %d: %s" % (code, said or "no verdict line")]
    return [] if said.endswith("PASS") else ["validate_tour.py ended with 0 and without its PASS line: %s" % (said or "no verdict line")]


def gate_not_run(output, code):
    """Whether the gate did not run rather than fail: each FAIL line of validate_retained.py is a replay that the
    slot check refused. A failure of any other kind, or a failed tool with no FAIL line, is a failure."""
    if code == 0:
        return False
    fails = re.findall(r"^\s*FAIL (.*)$", output, re.MULTILINE)
    return bool(fails) and all(text.startswith(NOT_RUN) for text in fails)


def closing_crash(mwse_log):
    return "Morrowind has crashed" in mwse_log


# The runner's exit code of a run whose game crashed in its close after DONE (run_own_device.EXIT_CRASHED_AFTER_DONE,
# W417), the copy of its log in the run folder (run_own_device.RUN_LOG), and the start of the ERROR line that fails the
# set for it (W513). The self-test checks the two values against the runner.
RUNNER_CRASHED_AFTER_DONE = 5
RUNNER_LOG = "runner-log.txt"
CRASH_AFTER_DONE_LINE = "ERROR: crash after DONE in"


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


def slot_label(what):
    """The label of a take of this tool, "<who: what>". A release names it too: from a terminal with no session,
    slot.py owns a hold only by its label (`owner_of`), and a release without one owns nothing."""
    return "make_play_set: %s" % what


@contextlib.contextmanager
def slot_asker(slots, slot):
    """The asks of a loop of takes of one slot, as a function (who, **kind) -> the reply. It asks through slot.py's
    LoopWaiter when the slot tool has it: a `leave` of the loop's place then ends the wait at its next ask (WAIT_ENDED),
    where a loop of plain takes asks again and takes a new place (2026-11-09 07:51, a merge_step after its leave). A
    slot.py from before LoopWaiter gets plain takes."""
    waiter_kind = getattr(slots, "LoopWaiter", None)
    if waiter_kind is None:
        yield lambda who, **kind: slots.take(slot, who, **kind)
        return
    with waiter_kind(slot) as waiter:
        yield waiter.take


def ask_slot(slots, slot, what, no_wait):
    """Takes a slot ("both": the game and the whole build side together), and waits in its queue. It asks again at
    each wait, so that it keeps its place. Returns "" when it holds the slot, else the first line of the reply that
    ends the wait: a refusal, the end of the wait after a `leave` of its place, or a place in the queue with
    `no_wait`."""
    os.makedirs(slots.STATE_DIR, exist_ok=True)
    with slot_asker(slots, slot) as ask:
        while True:
            try:
                reply = ask(slot_label(what), minutes=SLOT_MINUTES)
            except slots.StateBusy as error:
                say("the state of the slots is busy, and I ask again: %s" % error)
                time.sleep(WAIT_SECONDS)
                continue
            first = reply.text.splitlines()[0]
            say(first)
            if reply.code == slots.DONE:
                return ""
            # Only a place in the queue waits: a refusal and the end of a wait (WAIT_ENDED) stop it.
            if no_wait or reply.code != slots.QUEUED:
                return first
            time.sleep(WAIT_SECONDS)


def take_game(slots, what, no_wait):
    """Takes the game slot, and waits in its queue. A refusal, or the queue with `no_wait`, ends with exit code 3."""
    if ask_slot(slots, "game", what, no_wait):
        sys.exit(3)


def release_game(slots, what):
    """Releases the game hold of the take with `what`, by its label. Returns the text of the reply."""
    return slots.release("game", slot_label(what)).text


def take_set_hold(slots, no_wait):
    """The set's hold for the whole smoke (W405): the game and the whole build side in one take (D208), under the label
    of SMOKE_GAME. The replays need the whole build side (D139, f220), and a wait for it with the game held is a hold and
    wait. Before W405 the smoke gave the game back before each replay stage and took both again: a `take --wait` of
    another session got the game in that gap (10:40:16 on 2026-11-12, while f266's smoke went on). So the set holds both
    from its first game stage to its last replay, and then the game alone (`release_set_build`). A refusal, or the queue
    with `no_wait`, ends with exit code 3."""
    if ask_slot(slots, "both", SMOKE_GAME, no_wait):
        sys.exit(3)
    SET_HOLD["build"] = True


@contextlib.contextmanager
def set_hold_noted(slots):
    """merge_step's notes of the work of the set's slots, each minute: a live run compiles nothing, and a replay stage
    starts no game, so either hold would lapse after IDLE_MINUTES with no work seen (slot.py `lapse_idle_holds`)."""
    from merge_step import game_hold_noted  # here: merge_step imports this module

    with game_hold_noted(slots, build_who=slot_label(SMOKE_GAME)):
        yield


def keep_set_hold(slots, stage):
    """Before a stage: the set still holds its slots, the game and, until the last replay, the whole build side. A lapsed
    hold is taken again only when it is free at once (merge_step's `keep_holds`): a wait would give the game to another
    session between two stages, or be a hold and wait (D208). Else the stage is NOT RUN."""
    from merge_step import keep_holds  # here: merge_step imports this module

    if not keep_holds(slots, slot_label(SMOKE_GAME), "both" if SET_HOLD["build"] else "game", QUIET_LOG):
        raise SmokeNotRun("the set's hold for %s ended, and it was not free at once" % stage)


def end_set_hold(slots, holds_noted, keep_game):
    """The end of the set's hold, on every exit path: the notes stop, the build side goes back if a stop came before the
    last replay, and the game goes back unless `keep_game` (--keep-slot after a pass)."""
    holds_noted.close()
    release_set_build(slots)
    if keep_game:
        say("game: kept for the next work of this session")
    else:
        say(release_game(slots, SMOKE_GAME))


def release_set_build(slots):
    """Gives the build side of the set's hold back after its last replay, or at a stop before it. The game stays."""
    if SET_HOLD["build"]:
        say(slots.release("build", slot_label(SMOKE_GAME)).text.splitlines()[0])
        SET_HOLD["build"] = False


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


# --- the set --------------------------------------------------------------------------------

def ready_tree(name, repo, commit):
    """A detached worktree at this commit in TREES, for the smoke's tools. Nothing is built in it."""
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


def make_set(slots, commits, no_wait, in_set_hold, make=None, builder=None):
    """The maker's set of `commits` ({repo: full commit}), with no patch, of the usual kind: (exit code, folder or None).
    Before the set's hold the maker takes the build slot itself (build_set.build_hold, under SET_BUILD's label) and
    gives it back; in the set's hold (`in_set_hold`, the tips moved during the wait) it takes none, as the set's hold
    has the whole build side and its notes. The E: gate is asked at each phase of writes either way. make() takes the
    maker's run lock. `make` and `builder` replace build_set.make and build_set.Trees in the tests."""
    import build_set

    make = make or build_set.make
    hold = None if in_set_hold else (lambda: build_set.build_hold(slots, slot_label(SET_BUILD), no_wait))
    code, lines = make(commits, {}, KIND_USUAL, builder or build_set.Trees(), hold=hold)
    for line in lines:
        say(line)
    return code, (lines[-1] if code == 0 and lines else None)


def add_symbols(name, folder, add=None):
    """The set's binaries and the pdbs that the maker kept, into the symbol store (W345), as deploy_rest.py does."""
    say((add or symbols.add)(name, [os.path.join(folder, path) for path in SYMBOL_FILES], [os.path.join(PDB_KEEP, name)]))


def inside(path, folder):
    """True when `path` is `folder` or a path in it, also through a junction or a link."""
    path, folder = os.path.normcase(os.path.realpath(path)), os.path.normcase(os.path.realpath(folder))
    return path == folder or path.startswith(folder.rstrip(os.sep) + os.sep)


# --- the smoke ------------------------------------------------------------------------------

def wait_clean():
    """Waits until the instance is free of the run before: no game, no Mod Organizer of it, no Harness mod."""
    sys.path.insert(0, HARNESS)
    import harness

    not_known = None
    for _ in range(90):
        with open(TESTING_MODLIST, encoding="utf-8", errors="replace") as file:
            harness_on = any(line.startswith("+" + harness.RUN_MOD_PREFIX) for line in file)
        # A failed read of the processes is no free instance (W524 F-T-13): the wait goes on.
        try:
            free = not harness_on and not harness.processes("Morrowind.exe") and not harness.testing_mo2()
            not_known = None
        except harness.ProcessesNotKnown as error:
            free, not_known = False, error
        if free:
            time.sleep(2)
            return
        time.sleep(2)
    raise SmokeFailed("the instance did not become free in 180 s%s" % ("; %s" % not_known if not_known else ""))


def newest_run(tree, label, since):
    """The folder of the run that began after `since`. The tree's results stay between sets. Two new folders of
    the label mean that another run wrote there too, so neither is taken (W383)."""
    folders = [path for path in sorted(glob.glob(os.path.join(tree, "results", "*-" + label))) if os.path.getmtime(path) >= since]
    if not folders:
        raise SmokeFailed("no new run folder of %s" % label)
    if len(folders) > 1:
        raise SmokeFailed("%d new run folders of %s, not one: %s" % (len(folders), label, ", ".join(folders)))
    return folders[0]


def keep_run(run):
    """Copies a run to the kept runs: the results of a tree in TREES are a real folder that a cleanup may delete."""
    target = os.path.join(KEPT_RUNS % datetime.date.today().isoformat(), os.path.basename(run))
    shutil.copytree(run, target, dirs_exist_ok=True)
    return target


def runner_command(label, folder, budget, log, *args):
    """The runner's command of a smoke run: the set's own plugin, so no --plugin-dll (W521)."""
    return [sys.executable, "tools/run_own_device.py", "--label", label, "--mge-build", folder, "--budget", str(budget), *args, "--log", log]


def run_runner(tree, label, folder, budget, *args):
    """Runs the runner of the tree, whose tools are of the set's nirender commit. Returns the run folder, kept
    also when the run failed, as a failed set's reason names it. A run whose game crashed in its close after DONE
    (the runner's RUNNER_CRASHED_AFTER_DONE, W417) fails the set with an ERROR line that names the run (D462)."""
    wait_clean()
    log = os.path.join(tree, "results", label + ".txt")
    say("smoke: %s" % label)
    began = time.time()
    # As the owner's start has it: a start that the renderer refuses is a fault of the set.
    done = subprocess.run(runner_command(label, folder, budget, log, *args),
                          cwd=tree, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    run = newest_run(tree, label, began)
    say("smoke: the run is kept in %s" % keep_run(run))
    digest = run_mwse(read_text(os.path.join(run, "progress.log")))
    # A run with no such line is not named here: a smoke with no name says so in the note.
    if digest:
        SMOKE_MWSE.add(digest)
    if done.returncode == RUNNER_CRASHED_AFTER_DONE:
        line = crash_after_done_line(run)
        say(line)
        raise SmokeFailed(line)
    if done.returncode != 0:
        raise SmokeFailed("the runner of %s gave exit code %d: %s" % (label, done.returncode, run))
    return run


def crash_after_done_line(run):
    """The ERROR line of a smoke run whose game crashed in its close after DONE; the runner's log names the exit code
    and the dumps."""
    return "%s %s: the runner's exit code %d (see %s)" % (CRASH_AFTER_DONE_LINE, os.path.basename(run), RUNNER_CRASHED_AFTER_DONE, RUNNER_LOG)


def read_text(path):
    with open(path, encoding="utf-8", errors="replace") as file:
        return file.read()


def check_run_files(run, notes):
    """The checks of every run: no GPU error, and a note for a crash at the close that only MWSE.log shows. The GPU
    errors are the helper's check "gpu errors" (W526; common.py's one reader), not a count of a text: a helper
    that printed no line of the check (it did not end in order, or it is from before the checks) is a failure, and
    so is a check that judged no frame."""
    not_judged, failed = check_problems(read_text(os.path.join(run, "host.txt")), (GPU_ERRORS,))
    if not_judged or failed:
        raise SmokeFailed("%s in host.txt of %s" % ("; ".join(failed + not_judged), run))
    # The runner's exit code 0 says that the last step ended and that the runner saw no crash at the close (no exit
    # code that is not 0, no dump: its exit code 5 fails the set in run_runner), so a crash in this log came at the
    # close and left no dump.
    if closing_crash(read_text(os.path.join(run, "MWSE.log"))):
        notes.append("crash at the close of %s: MWSE.log says so, and the runner saw no exit code that is not 0 and no dump" % os.path.basename(run))


def picture_is_black(path):
    # Here and not at the top: only this needs Pillow. picture_diff finds its DLLs under -I.
    import picture_diff  # noqa: F401
    from PIL import Image, ImageStat

    with Image.open(path) as picture:
        return max(ImageStat.Stat(picture.convert("RGB")).mean) < BLACK_LEVEL


def smoke_live(tree, folder, notes):
    run = run_runner(tree, "playset-live", folder, 400, "--steps", "exterior,shaders", "--exterior", "Balmora",
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


def gate_now(tree, folder, when):
    """The set's own folder of the gate's pictures, a run folder of the tree: another gate at the
    same time (a merge step's, another set's) writes elsewhere, never in the baseline folder."""
    return os.path.join(tree, "results", time.strftime("%Y%m%d-%H%M%S", when) + "-gate-" + os.path.basename(folder))


def smoke_gate(tree, folder, slots):
    """The replays against the accepted pictures, with no game, under the set's hold of the whole build side. Its exit
    code is honest since 849c9b3."""
    say("smoke: the gate")
    keep_set_hold(slots, "the gate of the next set")
    now = gate_now(tree, folder, time.localtime())
    done = subprocess.run([sys.executable, "-I", "tools/validate_retained.py", "baseline", "--mge-build", folder, "--baseline", BASELINE, "--now", now],
                          cwd=tree, capture_output=True, text=True, errors="replace")
    keep = os.path.join(KEPT_RUNS % datetime.date.today().isoformat(), "gate-%s.txt" % os.path.basename(folder))
    os.makedirs(os.path.dirname(keep), exist_ok=True)
    with open(keep, "w", encoding="utf-8") as file:
        file.write(done.stdout + done.stderr)
    # A failure first: a NOT RUN must not hide it.
    if gate_not_run(done.stdout, done.returncode):
        raise SmokeNotRun("the gate's replays, see %s" % keep)
    if done.returncode != 0:
        raise SmokeFailed("the gate gave exit code %d, see %s: %s" % (done.returncode, keep, (done.stdout + done.stderr).strip()[-200:]))
    return os.path.basename(keep)


def smoke_tour(tree, folder, notes, slots):
    """The tour in a game, then its replays under the set's hold of the whole build side, which it gives back after
    them: they are the last replays of the smoke."""
    run = run_runner(tree, "playset-tour", folder, 900, "--steps", "tour", "--tour-cells", TOUR_CELLS, "--tour-dual")
    check_run_files(run, notes)
    keep_set_hold(slots, "the tour's replays of the next set")
    done = subprocess.run([sys.executable, "tools/validate_tour.py", run, "--mge-build", folder, "--jobs", "3", "--least-stops", str(LEAST_STOPS)],
                          cwd=tree, capture_output=True, text=True, errors="replace")
    release_set_build(slots)
    with open(os.path.join(run, "validate.txt"), "w", encoding="utf-8") as file:
        file.write(done.stdout + done.stderr)
    keep_run(run)
    # A failure first: a NOT RUN must not hide it.
    faults = tour_faults(done.stdout, done.returncode)
    if faults:
        raise SmokeFailed("%s: %s" % (run, "; ".join(faults)))
    if done.returncode == NOT_RUN_CODE:
        raise SmokeNotRun("the tour's replays, see %s" % os.path.join(run, "validate.txt"))
    return run


def smoke_owner_bats(tree, folder, owner_local_bat, notes):
    """Runs both real bats through the real tools/play.py up to the game's start. The tree's local.bat names a
    small command in place of Python: it adds --stage-only, and so the last lines of the bat do not run.
    Returns the number of bats that reached the game's start. A bat that could not be tested because the
    instance's Mod Organizer is open makes the smoke NOT RUN (W524 F-T-1), after the check of the mod lists."""
    wait_clean()
    tested = 0
    not_run = []
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
        file.write('@set "PYTHON=%s"\n@set "MGE_BUILD=%s"\n@set "OUTLANDER_INSTANCE=%s"\n@set "LIVECODING=%s"\n' % (
            stage_only, folder, outlander, livecoding))

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
            not_run.append("%s is not tested: the instance's Mod Organizer is open" % bat)
            continue
        if "no game (--stage-only)" not in output:
            raise SmokeFailed("%s did not reach the game's start: %s" % (bat, output[-400:]))
        say("smoke: %s reached the game's start" % bat)
        tested += 1

    for path, digest in before.items():
        if md5_of(path) != digest:
            raise SmokeFailed("the bats changed the mod list %s" % path)
    if not_run:
        raise SmokeNotRun("; ".join(not_run))
    return tested


def run_smoke(tree, folder, owner_local_bat, notes, slots):
    """The quick part first: a fault costs less time. The set's hold (`take_set_hold`) covers every stage, with no
    release between them (W405)."""
    keep_set_hold(slots, "the live smoke of the next set")
    live = smoke_live(tree, folder, notes)
    gate = smoke_gate(tree, folder, slots)
    keep_set_hold(slots, "the tour of the next set")
    tour = smoke_tour(tree, folder, notes, slots)
    keep_set_hold(slots, "the owner's start of the next set")
    tested = smoke_owner_bats(tree, folder, owner_local_bat, notes)
    return "passed (%s, %s, %s, %d of %d bats)" % (os.path.basename(live), gate, os.path.basename(tour), tested, len(OWNER_BATS))


def current_set_folder(checkout=None, sets=SETS):
    """The folder of the current set: the one that current.bat of `sets` names, else the one that local.bat of
    `checkout` names (`clean_common.current_set`), or None."""
    name = clean_common.current_set(checkout or PLUGIN_REPO, sets)
    return os.path.join(sets, name) if name else None


def manifest_commits(folder):
    """The full commits of the set `folder`, {repo: commit}, from its manifest, or None when it has none that reads."""
    try:
        with open(os.path.join(folder, MANIFEST), encoding="utf-8") as file:
            commits = json.load(file)["commits"]
        return {repo: commits[repo]["commit"] for repo in ("nirender", "render-link", "mwse")}
    except (OSError, ValueError, KeyError, TypeError):
        return None


def current_set_commits(checkout=None, sets=SETS, notes=PLAYSET_NOTES):
    """The commits of the current set ({repo: full commit}, from its manifest) when the newest note of its smokes says
    passed, else None. A set that deploy_rest.py named, with no smoke that passed, is smoked."""
    folder = current_set_folder(checkout, sets)
    if folder is None or not note_says_passed(newest_note(os.path.basename(folder), notes)):
        return None
    return manifest_commits(folder)


# --- the tests of the pure parts ------------------------------------------------------------

def write_test_set(sets, name, commits):
    """A scratch set folder with a manifest that names `commits` ({repo: commit}), for the tests."""
    os.makedirs(os.path.join(sets, name))
    with open(os.path.join(sets, name, MANIFEST), "w", encoding="utf-8") as file:
        json.dump({"name": name, "kind": KIND_USUAL, "commits": {repo: {"commit": commit} for repo, commit in commits.items()}}, file)


def current_set_test():
    """The current set with the two forms of the owner's local.bat (the Reviewer's verdict 416): the set lines and then
    the call of current.bat, and the call alone. Each form is read by `current_set_commits` and run by cmd, on a scratch
    sets folder and checkout. The null: with no current.bat, the first form gives the set of its lines and the second
    gives none. The commits come from the manifest, and the newest note of the set's smokes decides (addendum 9): no
    note, a FAILED one, or a passed one with a FAILED one after it give None."""
    import io
    with tempfile.TemporaryDirectory(prefix="make-play-set-") as root:
        sets = os.path.join(root, "mgebuilds")
        notes = os.path.join(root, "playset-notes")
        checkout = os.path.join(root, "nirender-plugin")
        os.makedirs(notes)
        old = {"nirender": "a" * 40, "render-link": "b" * 40, "mwse": "c" * 40}
        new = {"nirender": "1" * 40, "render-link": "2" * 40, "mwse": "3" * 40}

        def note(name, stamp, result):
            with open(os.path.join(notes, "%s-%s.txt" % (name, stamp)), "w", encoding="ascii") as file:
                file.write(set_note(name, new if name == "s-new" else old, result, {"aae6"}))

        for name, commits in (("s-old", old), ("s-new", new)):
            write_test_set(sets, name, commits)
        note("s-old", "20261117-010000", "passed (a, b, c, 2 of 2 bats)")
        os.makedirs(checkout)
        bat = os.path.join(sets, CURRENT_BAT)
        call = 'if exist "%s" call "%s"\r\n' % (bat, bat)
        old_lines = 'set "MGE_BUILD=%s"\r\nset "NIRENDER_PLUGIN_DLL=%s"\r\n' % (os.path.join(sets, "s-old"), os.path.join(sets, "s-old", "akulakhan-aaaaaaa.dll"))
        forms = {"lines and the call": "@echo off\r\n" + old_lines + call, "the call alone": "@echo off\r\n" + call}
        probe = os.path.join(root, "probe.cmd")
        with open(probe, "w", encoding="ascii", newline="") as file:
            file.write('@call "%s"\r\n@echo %%MGE_BUILD%%\r\n' % os.path.join(checkout, "local.bat"))

        def form_gives(form):
            """(the commits, the value of MGE_BUILD in cmd) for the local.bat `form`."""
            with open(os.path.join(checkout, "local.bat"), "w", encoding="ascii", newline="") as file:
                file.write(forms[form])
            clean_env = {key: value for key, value in os.environ.items() if key.upper() not in ("MGE_BUILD", "NIRENDER_PLUGIN_DLL")}
            shell = subprocess.run(["cmd", "/c", probe], stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=60, env=clean_env)
            return current_set_commits(checkout, sets, notes), shell.stdout.strip()

        with contextlib.redirect_stdout(io.StringIO()):
            assert form_gives("lines and the call") == (old, os.path.join(sets, "s-old"))
            assert form_gives("the call alone")[0] is None, "no current.bat and no set lines: no current set"
            # current.bat's one line (W521) wins over local.bat's MGE_BUILD in cmd too.
            write_current_bat(sets, "s-new")
            # A set with no note of a smoke (deploy_rest.py named it) has no commits: it is smoked.
            assert form_gives("the call alone")[0] is None, "a set with no smoke"
            note("s-new", "20261117-020000", "FAILED: a reason")
            assert form_gives("the call alone")[0] is None, "a failed smoke"
            # The null of the newest note: a later pass decides.
            note("s-new", "20261117-030000", "passed (a, b, c, 2 of 2 bats)")
            for form in forms:
                gives = form_gives(form)
                assert gives == (new, os.path.join(sets, "s-new")), (form, gives)
            # Passed, then FAILED: the newest note decides, and the set is no current set with the tips.
            note("s-new", "20261117-040000", "FAILED: a later reason")
            assert form_gives("the call alone")[0] is None, "an old pass counted over a later failure"
            # A set whose name begins with another's: the other's notes are not its own.
            write_test_set(sets, "s-new+p1", new)
            write_current_bat(sets, "s-new+p1")
            note("s-new", "20261117-050000", "passed (a, b, c, 2 of 2 bats)")
            assert newest_note("s-new+p1", notes) is None and form_gives("the call alone")[0] is None


def run_runner_test():
    """W513: the runner's exit code 5 (a crash after DONE, W417) fails the smoke with the ERROR line that names the run;
    0 passes; another code fails as before. The runner, the wait and the kept copy are stand-ins. Also: the runner's
    values, and the MWSE.log note of a crash at the close that the runner did not see (exit code 0)."""
    import io
    import run_own_device
    assert (RUNNER_CRASHED_AFTER_DONE, RUNNER_LOG) == (run_own_device.EXIT_CRASHED_AFTER_DONE, run_own_device.RUN_LOG)
    module = sys.modules[__name__]
    names = ("wait_clean", "newest_run", "keep_run")
    real = {name: getattr(module, name) for name in names}
    real_run = subprocess.run
    with tempfile.TemporaryDirectory() as folder:
        run = os.path.join(folder, "20261115-224947-playset-live")
        os.mkdir(run)
        with open(os.path.join(run, "progress.log"), "w", encoding="ascii") as file:
            file.write("[runner] MWSE.dll aae6\n")
        code = [0]
        commands = []
        try:
            module.wait_clean = lambda: None
            module.newest_run = lambda tree, label, since: run
            module.keep_run = lambda path: path
            subprocess.run = lambda command, **kwargs: commands.append(command) or types.SimpleNamespace(returncode=code[0])
            with contextlib.redirect_stdout(io.StringIO()) as said:
                assert run_runner("t", "playset-live", "f", 400) == run, "exit code 0 passes"
                # The set's own plugin (W521): the command names the set and no plugin.
                assert commands[0][commands[0].index("--mge-build") + 1] == "f" and not [part for part in commands[0] if "plugin" in part], commands
                code[0] = RUNNER_CRASHED_AFTER_DONE
                try:
                    run_runner("t", "playset-live", "f", 400)
                    raise AssertionError("a crash after DONE passed the smoke")
                except SmokeFailed as failure:
                    line = str(failure)
                assert line == "ERROR: crash after DONE in 20261115-224947-playset-live: the runner's exit code 5 (see runner-log.txt)", line
                assert line in said.getvalue(), "the ERROR line is said"
                code[0] = 1
                try:
                    run_runner("t", "playset-live", "f", 400)
                    raise AssertionError("exit code 1 passed the smoke")
                except SmokeFailed as failure:
                    assert "gave exit code 1" in str(failure), failure
        finally:
            subprocess.run = real_run
            for name, value in real.items():
                setattr(module, name, value)
        # The helper's check of the GPU errors: no line, a check that judged nothing, and faults fail the smoke.
        for text, said in (("frames 10\n", 'no line of the check "gpu errors"'), ("gpu error 1 (validation, adapter x): y\n", 'no line of the check "gpu errors"'),
                           ("check gpu errors: 0 judged, 0 faults\n", 'the check "gpu errors" judged nothing'), ("check gpu errors: 10 judged, 2 faults\n", 'the check "gpu errors": 2 faults of 10 judged')):
            with open(os.path.join(run, "host.txt"), "w", encoding="ascii") as file:
                file.write(text)
            try:
                check_run_files(run, [])
                raise AssertionError("the smoke passed with %r in host.txt" % text)
            except SmokeFailed as failure:
                assert said in str(failure), failure
        with open(os.path.join(run, "host.txt"), "w", encoding="ascii") as file:
            file.write("frames 10\ncheck gpu errors: 10 judged, 0 faults\n")
        with open(os.path.join(run, "MWSE.log"), "w", encoding="ascii") as file:
            file.write("Morrowind has crashed! To help improve game stability, send MWSE_Minidump.dmp\n")
        notes = []
        check_run_files(run, notes)
        assert len(notes) == 1 and notes[0].startswith("crash at the close of 20261115-224947-playset-live"), notes


def smoke_wiring_test(fake_slots):
    """The wiring of the smoke (the f220 fault was there), with a fake runner, fake tools and fake slots: one take of
    both for the whole smoke, no release between the stages, the build side given back after the tour's replays, and a
    failure that wins over a NOT RUN, in the gate and in the tour. The fake slots keep slot.py's rule of labels: a take
    of the holder's session with another label is refused (f221), and a release with a label frees only a hold of that
    label. Another session waits for the game with `take --wait` (W405): it gets the game the moment nobody holds it."""
    import io

    events = []
    other_wait = "other: a take --wait of the game"

    class Slots(fake_slots):
        # The holds: the slot and its label; the label `other_wait` is another session's.
        held = {}
        # Another session waits in the game queue.
        waiting = False

        @classmethod
        def take(cls, slot, who, **kind):
            events.append("take %s" % slot)
            # A take of both holds the game and the build side (slot.py, D208).
            parts = ("game", "build") if slot == "both" else (slot,)
            for part in parts:
                if cls.held.get(part) == other_wait:
                    events.append("refused %s" % slot)
                    return types.SimpleNamespace(code=cls.REFUSED, text="%s: refused: held by %s" % (part, other_wait))
                if cls.held.get(part, who) != who:
                    events.append("refused %s" % slot)
                    return types.SimpleNamespace(code=cls.REFUSED, text="%s: refused: your session already holds the %s slot as \"%s\"; a take with another label is a second run" % (
                        part, part, cls.held[part]))
            for part in parts:
                cls.held[part] = who
            return types.SimpleNamespace(code=cls.DONE, text="taken")

        @classmethod
        def release(cls, slot, who=None):
            events.append("release %s" % slot)
            # As from a terminal with no session: a release without a label owns nothing (slot.py `owns`).
            if who is None:
                return types.SimpleNamespace(code=cls.REFUSED, text="%s: refused: the %s slot is held by another" % (slot, slot))
            if cls.held.get(slot) != who:
                return types.SimpleNamespace(code=cls.REFUSED, text="%s: refused: no place is held as \"%s\"" % (slot, who))
            cls.held.pop(slot, None)
            if cls.waiting and "game" not in cls.held:
                cls.held["game"], cls.waiting = other_wait, False
                events.append("the other session takes the game")
            return types.SimpleNamespace(code=cls.DONE, text="released")

        # What merge_step's keep_holds reads of slot.py: the places held by this session's run.
        Holder, StateError = object, OSError
        units_of = staticmethod(lambda slot: ["game"] if slot == "game" else ["build-1", "build-2"])
        holder_path = staticmethod(lambda unit: unit)
        session_id = staticmethod(lambda: "this session")
        owner_of = staticmethod(lambda session, who: session)
        note_holder_build = staticmethod(lambda owner, now: None)

        @classmethod
        def read_record(cls, path, kind):
            who = cls.held.get("game" if path == "game" else "build")
            return who and types.SimpleNamespace(session="another session" if who == other_wait else "this session", who=who)

        @classmethod
        def leave(cls, slot, who=None):
            events.append("leave %s" % slot)

    refused = "Refused by the slot check: replay.py: the build slot is held by rob (place 1); a replay needs the whole build side."
    row = "  %s                                       0.000 mean, 0.00 %% different, 0.00 %% over 8, largest 0\n"
    head = "103 stops to replay, gpu and off at each\n"
    pairs = "mode gpu against mode off at each stop, the largest differences first:\n" + row % "2-01-b.png"
    tours = {
        "passed": (0, head + "206 pictures, 0 failed\n" + pairs + "pairs: 103 of 103 equal\ntour verdict: PASS\n"),
        "not run": (NOT_RUN_CODE, head + "%s: replay of tour-1.nircap: 84 pictures not made: %s\n122 pictures, 0 failed\n%s: 84 pictures not made, and not checked\n" % (NOT_RUN, refused, NOT_RUN)
                    + pairs + "pairs: 61 of 61 equal; 42 pairs %s, not compared\n" % NOT_RUN),
        # f220 with this fix: every replay refused, so no pair has numbers.
        "all not run": (NOT_RUN_CODE, head + "".join("%s: replay of tour-%d.nircap: 68 pictures not made: %s\n" % (NOT_RUN, cell, refused) for cell in (1, 2, 3))
                        + "0 pictures, 0 failed\n%s: 206 pictures not made, and not checked\n" % NOT_RUN
                        + "mode gpu against mode off at each stop, the largest differences first:\npairs: 0 of 0 equal; 103 pairs %s, not compared\n" % NOT_RUN),
        "failed": (1, head + "%s: replay of tour-1.nircap: 84 pictures not made: %s\nFAILED 2-02-c.png: gpu error\n120 pictures, 2 failed\n" % (NOT_RUN, refused)
                   + pairs + "pairs: 60 of 60 equal; 42 pairs %s, not compared\n" % NOT_RUN),
    }
    gates = {
        "passed": (0, "  ok   12 of 12 pictures equal to the accepted ones in x\n"),
        "not run": (1, "       %s\n  FAIL %s: replay.py: %s\n" % (refused, NOT_RUN, refused)),
        "failed": (1, "       %s: replay of tour-1.nircap: 84 pictures not made: %s\n       120 pictures, 2 failed\n  FAIL validate_tour.py ended with the code 1\n" % (NOT_RUN, refused)),
        # A NOT RUN FAIL line beside a FAIL line of another kind: a failure.
        "mixed": (1, "  FAIL 11 of 12 pictures equal to the accepted ones in x\n       %s\n  FAIL %s: validate_tour.py: %s\n" % (refused, NOT_RUN, refused)),
    }
    outcome = {}

    def tool_run(command, **_):
        tool = next(os.path.basename(part) for part in command if str(part).endswith(".py"))
        events.append(tool)
        code, output = (gates if tool == "validate_retained.py" else tours)[outcome[tool]]
        return subprocess.CompletedProcess(command, code, output, "")

    # With True, the set's game hold lapses during the live smoke, and the waiting session takes the game.
    lapse = [False]

    with tempfile.TemporaryDirectory() as tree:
        def a_run(label):
            events.append("game " + label)
            if lapse[0] and Slots.waiting:
                Slots.held["game"], Slots.waiting = other_wait, False
                events.append("the set's game hold lapses, and the other session takes the game")
            path = os.path.join(tree, label)
            os.makedirs(path, exist_ok=True)
            return path

        fakes = {
            "smoke_live": lambda *_: a_run("playset-live"),
            "run_runner": lambda _tree, label, *_: a_run(label),
            "check_run_files": lambda *_: None,
            "keep_run": lambda run: run,
            "smoke_owner_bats": lambda *_: events.append("bats") or 2,
            "KEPT_RUNS": os.path.join(tree, "runs-%s"),
        }
        real = {name: globals()[name] for name in fakes}
        real_run = subprocess.run
        globals().update(fakes)
        subprocess.run = tool_run

        def smoke(gate, tour, build_held_as=None, keep_game=False, lapsed=False):
            """The result of the smoke as main() runs it (the set's hold, run_smoke, the end of the hold), or the class
            of its stop. Another session waits for the game all the time. With `build_held_as`, this session holds the
            build side with that label; with `lapsed`, the set's game hold lapses in the live smoke."""
            events.clear()
            Slots.held = {"build": build_held_as} if build_held_as else {}
            Slots.waiting, lapse[0] = True, lapsed
            outcome.update({"validate_retained.py": gate, "validate_tour.py": tour})
            try:
                with contextlib.redirect_stdout(io.StringIO()):
                    take_set_hold(Slots, False)
                    try:
                        return run_smoke(tree, "s-999", "local.bat", [], Slots)
                    finally:
                        end_set_hold(Slots, contextlib.ExitStack(), keep_game)
            except (SmokeFailed, SmokeNotRun, SystemExit) as stop:
                return type(stop)

        label = slot_label(SMOKE_GAME)
        try:
            # The null: one take of both, no release between the stages, the build side back after the tour's
            # replays, and the waiting session gets the game only after the set's last stage (W405; before, it got the
            # game when the smoke gave it back for its gate).
            result = smoke("passed", "passed")
            assert isinstance(result, str) and result.startswith("passed") and events == [
                "take both", "game playset-live", "validate_retained.py", "game playset-tour", "validate_tour.py",
                "release build", "bats", "release game", "the other session takes the game"], events
            assert Slots.held == {"game": other_wait}, Slots.held
            # --keep-slot after a pass: the set keeps the game, and the other session still waits.
            assert smoke("passed", "passed", keep_game=True).startswith("passed") and events[-2:] == ["release build", "bats"], events
            assert Slots.held == {"game": label} and Slots.waiting, Slots.held
            # A stop gives back what the set holds: a gate NOT RUN before the build side went back.
            assert smoke("not run", "passed") is SmokeNotRun and events[-4:] == [
                "validate_retained.py", "release build", "release game", "the other session takes the game"], events
            # A hold that lapsed and that the other session took is not waited for: the next stage is NOT RUN.
            assert smoke("passed", "passed", lapsed=True) is SmokeNotRun and "validate_retained.py" not in events, events
            assert "take game" in events and "refused game" in events, "the lost game was not asked for once"
            # main() releases each game hold by the label of its take, also from a terminal with no session.
            Slots.held, Slots.waiting = {}, False
            with contextlib.redirect_stdout(io.StringIO()):
                take_game(Slots, OWNER_START_GAME, False)
                release_game(Slots, OWNER_START_GAME)
            assert Slots.held == {}, "the hold of --bats-only stays: %s" % Slots.held
            # A place of this session with another label: slot.py refuses the take (a second run), so the set stops
            # with exit code 3 before any stage, and that hold is not released.
            other = "make_play_set: other work"
            assert smoke("passed", "passed", build_held_as=other) is SystemExit and events == ["take both", "refused both"], events
            assert Slots.held["build"] == other, Slots.held
            assert smoke("failed", "passed") is SmokeFailed, "the gate failed, and a NOT RUN beside it does not hide it"
            assert smoke("mixed", "passed") is SmokeFailed, "a NOT RUN FAIL line does not hide a FAIL line of another kind"
            assert smoke("passed", "not run") is SmokeNotRun, "the tour's replays refused: NOT RUN"
            assert smoke("passed", "all not run") is SmokeNotRun, "all the tour's replays refused, no pair: NOT RUN"
            assert smoke("passed", "failed") is SmokeFailed, "the tour failed, and a NOT RUN beside it does not hide it"
        finally:
            globals().update(real)
            subprocess.run = real_run
    assert gate_not_run(gates["not run"][1], 1) and not gate_not_run(gates["failed"][1], 1) and not gate_not_run("", 1)
    assert not gate_not_run(gates["mixed"][1], 1)
    assert not gate_not_run(gates["not run"][1], 0)
    assert tour_faults(tours["not run"][1], NOT_RUN_CODE) == [] and tour_faults(tours["failed"][1], 1)


def self_test_wait_clean():
    """W524 F-T-13: a failed read of the processes is no free instance. The plant: every read fails, and the wait ends
    with SmokeFailed and the reason. The null: a read of no process frees the instance at once."""
    global TESTING_MODLIST
    sys.path.insert(0, HARNESS)
    import harness

    saved = TESTING_MODLIST, harness.processes, time.sleep
    reads = []

    def not_known(name, tries=3):
        reads.append(name)
        raise harness.ProcessesNotKnown("the list of the %s processes was not read (planted)" % name)

    with tempfile.TemporaryDirectory() as folder:
        TESTING_MODLIST = os.path.join(folder, "modlist.txt")
        with open(TESTING_MODLIST, "w", encoding="utf-8") as file:
            file.write("+Other\n")
        try:
            time.sleep = lambda seconds: None
            harness.processes = not_known
            try:
                wait_clean()
            except SmokeFailed as failure:
                assert "not read (planted)" in str(failure) and len(reads) == 90, (failure, len(reads))
            else:
                raise AssertionError("a failed read freed the instance")
            reads.clear()
            harness.processes = lambda name, tries=3: reads.append(name) or []
            wait_clean()
            assert reads == ["Morrowind.exe", "ModOrganizer.exe"], reads
        finally:
            TESTING_MODLIST, harness.processes, time.sleep = saved


def make_and_smoke_test():
    """The set step (addendum 8 tests 1, 2 and 4, with addendum 9's nulls), with a fake maker and a fake smoke; the
    holds have their own test (smoke_wiring_test). A make() that made no set: its exit code, no hold, no note file, no
    current.bat. The null: make() with code 0 writes the pending note before the smoke, and the pass writes current.bat.
    A NOT RUN (F-T-1) and a failure write no current.bat. Tips that moved during the wait: a second make() with no hold
    of its own."""
    import io

    tips = {"nirender": "a" * 40, "render-link": "b" * 40, "mwse": "c" * 40}
    moved = dict(tips, nirender="d" * 40)
    events = []
    fakes = {
        "take_set_hold": lambda slots, no_wait: events.append("hold"),
        "set_hold_noted": lambda slots: contextlib.nullcontext(),
        "end_set_hold": lambda slots, holds_noted, keep_game: events.append("end"),
    }
    real = {name: globals()[name] for name in ("run_smoke", *fakes)}
    with tempfile.TemporaryDirectory(prefix="make-play-set-") as root:
        sets, notes = os.path.join(root, "mgebuilds"), os.path.join(root, "playset-notes")
        os.makedirs(sets)
        smoke = {"stop": None}

        def fake_make(commits, patches, kind, builder, hold=None):
            events.append(("make", commits["nirender"][:1], hold is not None))
            if smoke.get("make_code"):
                return smoke["make_code"], ["no build slot"]
            name = "s-%s" % commits["nirender"][:8]
            os.makedirs(os.path.join(sets, name), exist_ok=True)
            return 0, ["part plugin: built", os.path.join(sets, name).replace("\\", "/")]

        def fake_smoke(tree, folder, owner_local_bat, notes_list, slots):
            said = newest_note(os.path.basename(folder), notes)
            assert said and "; smoke: pending" in said, said
            events.append(("smoke", os.path.basename(folder), tree))
            if smoke["stop"]:
                raise smoke["stop"]
            return "passed (live, gate, tour, 2 of 2 bats)"

        def run(tips_now=tips, **case):
            events.clear()
            smoke.clear()
            smoke.update(dict(stop=None), **case)
            with contextlib.redirect_stdout(io.StringIO()):
                return make_and_smoke("slots", tips, "local.bat", make=fake_make, tips=lambda: tips_now, tree_of=lambda commit: "tree-" + commit[:1],
                                      checkout_of=lambda commit: "", add=lambda *_: "symbols: added", notes_root=notes, sets_root=sets)

        globals().update(fakes, run_smoke=fake_smoke)
        try:
            # make() made no set: its code, no hold, no note file, no current.bat (test 1; test 2's null of the note).
            assert run(make_code=3) == 3 and events == [("make", "a", True)], events
            assert not os.path.exists(notes) and not os.path.exists(os.path.join(sets, CURRENT_BAT))
            assert run(make_code=4) == 4
            # The null: a set, its pending note before the smoke, then the pass and current.bat.
            assert run() == 0 and events == [("make", "a", True), "hold", ("smoke", "s-aaaaaaaa", "tree-a"), "end"], events
            assert clean_common.mge_build_of(os.path.join(sets, CURRENT_BAT)).endswith("s-aaaaaaaa")
            assert note_says_passed(newest_note("s-aaaaaaaa", notes))
            os.remove(os.path.join(sets, CURRENT_BAT))
            # A NOT RUN (F-T-1) and a failure: exit 3 and 1, a note that says so, no current.bat.
            time.sleep(1.1)
            assert run(stop=SmokeNotRun("play-manual-check.bat is not tested: the instance's Mod Organizer is open")) == 3
            assert NOT_RUN in newest_note("s-aaaaaaaa", notes) and not os.path.exists(os.path.join(sets, CURRENT_BAT))
            time.sleep(1.1)
            assert run(stop=SmokeFailed("a black picture")) == 1 and "smoke: FAILED: a black picture" in newest_note("s-aaaaaaaa", notes)
            assert not os.path.exists(os.path.join(sets, CURRENT_BAT)) and len(os.listdir(notes)) == 3, os.listdir(notes)
            # The tips moved during the wait: the second make() takes no hold of its own, and the set of the new tips
            # is smoked.
            assert run(tips_now=moved) == 0 and events[:3] == [("make", "a", True), "hold", ("make", "d", False)], events
            assert events[3] == ("smoke", "s-dddddddd", "tree-d"), events
            assert clean_common.mge_build_of(os.path.join(sets, CURRENT_BAT)).endswith("s-dddddddd")
        finally:
            globals().update(real)


def owner_bats_test():
    """W524 F-T-1: a bat that could not be tested because Mod Organizer is open makes the bats' step NOT RUN. The null:
    both bats reach the game's start. The scratch local.bat names no NIRENDER_PLUGIN_DLL (3.2: nothing writes it)."""
    global TESTING_MODLIST
    outputs = {}
    real_run, real_wait, real_modlist = subprocess.run, globals()["wait_clean"], TESTING_MODLIST

    def fake_run(command, **_):
        return subprocess.CompletedProcess(command, 0, outputs[os.path.basename(command[-1])], "")

    with tempfile.TemporaryDirectory(prefix="make-play-set-") as tree:
        for bat in OWNER_BATS:
            with open(os.path.join(tree, bat), "w", encoding="ascii") as file:
                file.write("@echo off\r\n%s\r\n" % BAT_CLEARS)
        owner = os.path.join(tree, "owner-local.bat")
        with open(owner, "w", encoding="ascii") as file:
            file.write('set "OUTLANDER_INSTANCE=%s"\r\n' % os.path.join(tree, "no-instance"))
        TESTING_MODLIST = os.path.join(tree, "modlist.txt")
        with open(TESTING_MODLIST, "w", encoding="utf-8") as file:
            file.write("+Other\n")
        try:
            globals()["wait_clean"] = lambda: None
            subprocess.run = fake_run
            outputs.update({bat: "play: no game (--stage-only)" for bat in OWNER_BATS})
            assert smoke_owner_bats(tree, "E:/sets/s-1", owner, []) == 2
            assert "NIRENDER_PLUGIN_DLL" not in read_text(os.path.join(tree, "local.bat")) and "MGE_BUILD=E:/sets/s-1" in read_text(os.path.join(tree, "local.bat"))
            outputs[OWNER_BATS[1]] = "play: Mod Organizer is open in D:/o; close it first"
            try:
                smoke_owner_bats(tree, "E:/sets/s-1", owner, [])
                raise AssertionError("a bat not tested passed the bats' step")
            except SmokeNotRun as stop:
                assert OWNER_BATS[1] in str(stop) and "Mod Organizer is open" in str(stop), stop
        finally:
            subprocess.run, TESTING_MODLIST = real_run, real_modlist
            globals()["wait_clean"] = real_wait


def self_test():
    global WAIT_SECONDS
    import io

    self_test_wait_clean()

    # The accepted baseline: the tree's when it has captures, else the main checkout's; the tree's when neither has
    # (a worktree in C:/tmp has no results/: 2026-11-09, Rob's merge_step).
    with tempfile.TemporaryDirectory() as folder:
        tree, main = os.path.join(folder, "tree"), os.path.join(folder, "main")
        tree_baseline, main_baseline = os.path.join(tree, *BASELINE_PARTS), os.path.join(main, *BASELINE_PARTS)
        assert accepted_baseline(tree, main) == tree_baseline, "neither has captures: the tree's, for the check to name"
        os.makedirs(main_baseline)
        open(os.path.join(main_baseline, "seyda.nircap"), "wb").close()
        assert accepted_baseline(tree, main) == main_baseline, "the tree has no results/"
        os.makedirs(tree_baseline)
        assert accepted_baseline(tree, main) == main_baseline, "the tree's folder has no capture"
        open(os.path.join(tree_baseline, "narsis.nircap"), "wb").close()
        assert accepted_baseline(tree, main) == tree_baseline, "the tree's own captures first"
        assert accepted_baseline(tree, None) == tree_baseline
        assert has_captures(main_baseline) and not has_captures(os.path.join(folder, "none"))

    # The run of a smoke is the one new folder of its label. An older folder is not taken, and two new ones stop
    # the set with both names (W383). The null: the last of the two is what a pick of the newest took.
    with tempfile.TemporaryDirectory() as tree:
        since = time.time()
        older, mine, other = (os.path.join(tree, "results", name + "-smoke-x") for name in ("20200101-000000", "20991231-000000", "20991231-000001"))
        os.makedirs(older)
        os.utime(older, (since - 3600, since - 3600))
        try:
            newest_run(tree, "smoke-x", since)
            raise AssertionError("an older folder alone was taken")
        except SmokeFailed as failed:
            assert str(failed) == "no new run folder of smoke-x", failed
        # The times are set: the file system's clock is coarser than time.time(), so a folder made just now can
        # carry a time before `since`.
        os.makedirs(mine)
        os.utime(mine, (since + 1, since + 1))
        assert newest_run(tree, "smoke-x", since) == mine
        os.makedirs(other)
        os.utime(other, (since + 2, since + 2))
        assert sorted(glob.glob(os.path.join(tree, "results", "*-smoke-x")))[-1] == other
        try:
            newest_run(tree, "smoke-x", since)
            raise AssertionError("one of two new folders was taken")
        except SmokeFailed as failed:
            assert str(failed) == "2 new run folders of smoke-x, not one: %s, %s" % (mine, other), failed

    # Two sets' gates at the same second write into two folders, outside the baseline folder.
    when = time.localtime()
    first, second = gate_now("tree", "E:/sets/f202", when), gate_now("tree", "E:/sets/f203", when)
    assert first != second and first.endswith("-gate-f202") and os.path.dirname(first) == os.path.join("tree", "results"), (first, second)
    assert not os.path.normcase(os.path.abspath(first)).startswith(os.path.normcase(os.path.abspath(BASELINE)) + os.sep)
    # A refusal for lack of room ends each wait with a code. A wait for another holder goes on.
    class Slots:
        DONE, REFUSED, QUEUED = 0, 1, 3
        STATE_DIR = tempfile.gettempdir()
        StateBusy = OSError
        replies, refusals = [], []

        @classmethod
        def take(cls, *_, **__):
            return types.SimpleNamespace(code=cls.replies.pop(0), text="a reply")

        @classmethod
        def refusal(cls, *_):
            return cls.refusals.pop(0)

        @staticmethod
        def enforcing():
            return True

        @staticmethod
        def session_id():
            return "s"

    def ends_with_three(action, *args):
        try:
            action(*args)
        except SystemExit as stop:
            return stop.code == 3
        return False

    Slots.replies = [Slots.REFUSED]
    assert ends_with_three(take_game, Slots, "x", False)
    Slots.replies = [Slots.DONE]
    assert not ends_with_three(take_game, Slots, "x", False)

    # The set's hold (W405): one take of both, which waits in both queues while it holds nothing (D208). A refusal, or the
    # queue with --no-wait, ends the set with exit code 3.
    class Side(Slots):
        asked, released = [], []

        @classmethod
        def take(cls, slot, who, **kind):
            cls.asked.append(slot)
            return types.SimpleNamespace(code=cls.replies.pop(0), text="a reply\nmore")

        @classmethod
        def release(cls, slot, who=None):
            cls.released.append((slot, who))
            return types.SimpleNamespace(code=Slots.DONE, text="build: released")

    def set_hold(no_wait=False):
        """Takes the set's hold. Returns True, or the exit code of its stop."""
        Side.asked, Side.released = [], []
        SET_HOLD["build"] = False
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                take_set_hold(Side, no_wait)
        except SystemExit as stop:
            return stop.code
        return True

    wait_seconds, WAIT_SECONDS = WAIT_SECONDS, 0
    try:
        Side.replies = [Slots.QUEUED, Slots.QUEUED, Slots.DONE]
        assert set_hold() is True and Side.asked == ["both"] * 3 and Side.released == [] and SET_HOLD["build"], Side.asked
        Side.replies = [Slots.REFUSED]
        assert set_hold() == 3 and Side.asked == ["both"] and not SET_HOLD["build"], Side.asked
        Side.replies = [Slots.QUEUED]
        assert set_hold(no_wait=True) == 3 and Side.asked == ["both"], "--no-wait: one ask, then exit code 3"

        # With slot.py's LoopWaiter, a leave of the place ends the wait: ask_slot stops at WAIT_ENDED, and the set stops
        # with exit code 3 (2026-11-09 07:51). The plain takes of Side are not used. A slot.py without LoopWaiter gets
        # plain takes: the tests above.
        class Waiting(Side):
            WAIT_ENDED = 5
            asked, waiter_asks = [], []

            class LoopWaiter:
                def __init__(self, slot):
                    self.slot = slot

                def __enter__(self):
                    return self

                def __exit__(self, *_):
                    pass

                def take(self, who, **kind):
                    Waiting.waiter_asks.append((self.slot, who))
                    return types.SimpleNamespace(code=Waiting.replies.pop(0), text="both: this wait ended without the slot: the place was left\nmore")

        Waiting.replies = [Slots.QUEUED, Waiting.WAIT_ENDED]
        with contextlib.redirect_stdout(io.StringIO()):
            stop = ask_slot(Waiting, "both", "x", False)
        assert stop.startswith("both: this wait ended") and Waiting.replies == [], stop
        assert Waiting.asked == [] and Waiting.waiter_asks == [("both", "make_play_set: x")] * 2, (Waiting.asked, Waiting.waiter_asks)
        Waiting.replies = [Waiting.WAIT_ENDED]
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                take_set_hold(Waiting, False)
            raise AssertionError("the set went on after the end of its wait")
        except SystemExit as stop:
            assert stop.code == 3 and not SET_HOLD["build"], stop
    finally:
        WAIT_SECONDS = wait_seconds
    # A replay that the slot check refused is NOT RUN in validate_tour.py's output, and no failed picture.
    import validate_tour
    with contextlib.redirect_stdout(io.StringIO()):
        validate_tour.self_test()
    smoke_wiring_test(Side)

    tips = {"nirender": "a" * 40, "render-link": "b" * 40, "mwse": "c" * 40}
    assert tips_moved(tips, dict(tips)) == ""
    assert tips_moved(tips, dict(tips, mwse="d" * 40)) == "mwse moved from cccccccc to dddddddd"
    assert tips_moved(tips, dict(tips, nirender="e" * 40)).startswith("nirender moved")

    text = '@echo off\r\nset "PYTHON=C:\\p.exe"\r\nset "MGE_BUILD=E:\\a\\f185"\r\nset "NIRENDER_PLUGIN_DLL=E:\\a\\f185\\x.dll"\r\nset "OUTLANDER_INSTANCE=D:\\o"\r\n'
    assert local_bat_values(text) == {"PYTHON": "C:\\p.exe", "MGE_BUILD": "E:\\a\\f185", "NIRENDER_PLUGIN_DLL": "E:\\a\\f185\\x.dll", "OUTLANDER_INSTANCE": "D:\\o"}
    current_set_test()

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

    # The tour: the tool's exit code is the verdict, and its last line says why (validate_tour.py's own self-test has
    # the cases of the pictures, the pairs and the stops).
    tour = "103 stops to replay, gpu and off at each\n206 pictures, 0 failed\n  2-13.png    0.001 mean, 0.08 % different\npairs: 18 of 103 equal\n"
    assert tour_faults(tour + "tour verdict: PASS\n", 0) == []
    assert tour_faults(tour + "tour verdict: FAIL: 3 pictures failed\n", 1) == ["validate_tour.py gave exit code 1: tour verdict: FAIL: 3 pictures failed"]
    assert tour_faults(tour + "tour verdict: NOT RUN: 4 pictures of replays that the slot check refused\n", NOT_RUN_CODE) == []
    assert tour_faults("Traceback (most recent call last):\n", 1) == ["validate_tour.py gave exit code 1: no verdict line"]
    # A tool that ends with 0 and has no PASS line (one from before its verdict) judged no pair: a fault.
    assert tour_faults(tour, 0) == ["validate_tour.py ended with 0 and without its PASS line: no verdict line"] and tour_faults("", 0) != []

    note = set_note("s-1", tips, "passed (a, b, 2 of 2 bats)", {"aae6"})
    assert note.count("\n") == 1 and note.endswith("2 of 2 bats)\n") and ", MWSE.dll aae6; smoke:" in note, note
    assert "nirender master aaaaaaaa, render-link feature/wgpu-renderer bbbbbbbb, MWSE cccccccc," in note and note_says_passed(note), note
    assert "no run of the smoke" in set_note("s-1", tips, "pending") and not note_says_passed(set_note("s-1", tips, "pending"))
    assert not note_says_passed(None)
    assert "changed during the smoke: a1, b2" in mwse_text({"b2", "a1"})
    assert run_mwse("[runner] instance D:\\m, profile Default\n[runner] MWSE.dll aae6\n[t] MWSE.dll zz\n") == "aae6"
    assert run_mwse("[t] MWSE.dll zz\n") is None

    run_runner_test()

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

    # The pdbs stay outside the sets (build_set.py keeps them in PDB_KEEP).
    assert not inside(PDB_KEEP, SETS)
    assert inside(SETS + "/f9", SETS) and inside(SETS, SETS) and not inside(SETS + "x/f9", SETS)
    make_and_smoke_test()
    owner_bats_test()
    # --build-only is gone: one line that names the maker, and nothing is made.
    done = subprocess.run([sys.executable, "-I", os.path.abspath(__file__), "--build-only"], capture_output=True, text=True, timeout=120)
    assert done.returncode == 1 and done.stdout.strip() == BUILD_ONLY_GONE, (done.returncode, done.stdout, done.stderr)

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


BUILD_ONLY_GONE = ("make_play_set: --build-only is gone (W521): the maker makes and keeps a set alone, "
                   "python -I tools/build_set.py; this tool smokes the set of the tips")


def commits_text(commits):
    return ", ".join("%s %s" % (repo, commit[:8]) for repo, commit in commits.items())


def smoke_tree(commit):
    """The tree of the smoke's tools at the set's nirender commit (B2 of the plan's addendum 8), with its results
    folder and the key of the harness."""
    tree = ready_tree("nirender-plugin", PLUGIN_REPO, commit)
    os.makedirs(os.path.join(tree, "results"), exist_ok=True)
    # The runner of the tree finds the harness by this key: its siblings hold no moreFPS.
    with open(os.path.join(tree, "local.json"), "w", encoding="ascii") as file:
        json.dump({"harness": HARNESS}, file)
    return tree


def checkout_note_of(commit):
    """The note's words when the owner's checkout is not at the set's nirender commit, else an empty text: a game by
    the bat takes the Lua of mod/ from there."""
    head = git(PLUGIN_REPO, "rev-parse", "HEAD")
    return "" if head == commit else "; the owner's checkout was at %s, so a game by the bat takes another Lua" % head[:8]


def bats_only(slots, owner_local_bat, no_wait):
    """The step of the owner's start alone, on the current set after its check. Returns the exit code."""
    folder = current_set_folder()
    if folder is None:
        say("no current set: current.bat and local.bat name none")
        return 1
    try:
        verify(folder)
    except SetError as error:
        say("the current set fails its check: %s" % error)
        return 1
    commits = manifest_commits(folder)
    if commits is None:
        say("the current set %s has no commits in its manifest" % folder)
        return 1
    tree = smoke_tree(commits["nirender"])
    take_game(slots, OWNER_START_GAME, no_wait)
    notes = []
    try:
        tested = smoke_owner_bats(tree, folder, owner_local_bat, notes)
    except SmokeFailed as failure:
        say("FAILED: %s" % failure)
        return 1
    except SmokeNotRun as stop:
        say("%s: %s" % (NOT_RUN, stop))
        return 3
    finally:
        say(release_game(slots, OWNER_START_GAME))
    say("%d of %d bats reached the game's start%s" % (tested, len(OWNER_BATS), "; " + "; ".join(notes) if notes else ""))
    return 0


def make_and_smoke(slots, commits, owner_local_bat, no_wait=False, keep_slot=False, make=None, tips=None, tree_of=None,
                   checkout_of=None, add=None, notes_root=PLAYSET_NOTES, sets_root=SETS):
    """The set of `commits` from the maker, then its smoke and, on a pass, current.bat. Returns the exit code: make()'s
    when it made no set (0 means a set), 1 for a failed smoke, 3 for a NOT RUN or a slot. The keyword arguments after
    `keep_slot` replace the maker, the tips, the smoke's tree, the read of the owner's checkout and the symbol store in
    the tests."""
    tips = tips or tip_commits
    tree_of = tree_of or smoke_tree
    checkout_of = checkout_of or checkout_note_of
    code, folder = make_set(slots, commits, no_wait, False, make=make)
    if code != 0:
        return code
    # The slot first: a stop in the queue leaves no note behind.
    take_set_hold(slots, no_wait)
    # The notes start in the try: a fault at their start still ends the hold (the Reviewer's verdict 289).
    holds_noted = contextlib.ExitStack()
    name = os.path.basename(folder)
    note = None
    notes = []
    passed = False
    SMOKE_MWSE.clear()

    def write_note(result):
        with open(note, "w", encoding="ascii", errors="backslashreplace") as file:
            file.write(set_note(name, commits, result, SMOKE_MWSE))

    try:
        holds_noted.enter_context(set_hold_noted(slots))
        # The wait for the slot can be long, and the set must be of the tips when the smoke starts (f187 was an hour
        # old). This make runs in the set's own hold of the whole build side.
        now = tips()
        moved = tips_moved(commits, now)
        if moved:
            say("%s during the wait for the slot, and I ask the maker again" % moved)
            commits = now
            code, folder = make_set(slots, commits, no_wait, True, make=make)
            if code != 0:
                return code
            name = os.path.basename(folder)
        checkout_note = checkout_of(commits["nirender"])
        os.makedirs(notes_root, exist_ok=True)
        # "x": a note of another smoke is never written over; `note` names this smoke's note only once it is there.
        path = note_path(name, time.localtime(), notes_root)
        with open(path, "x", encoding="ascii", errors="backslashreplace") as file:
            file.write(set_note(name, commits, "pending"))
        note = path
        tree = tree_of(commits["nirender"])
        say("the set: %s" % folder)
        add_symbols(name, folder, add)
        result = run_smoke(tree, folder, owner_local_bat, notes, slots)
        passed = True
    except SmokeFailed as failure:
        reason = "FAILED: %s" % failure
        write_note(reason)
        say("%s: %s. %s stays on the last good set." % (name, reason, CURRENT_BAT))
        return 1
    except SmokeNotRun as stop:
        reason = "%s: %s" % (NOT_RUN, stop)
        write_note(reason)
        say("%s: %s. No picture of it was checked; %s stays on the last good set." % (name, reason, CURRENT_BAT))
        return 3
    except BaseException as failure:
        # A note must not stay on "pending" (a stop of a tool, an interrupt).
        if note:
            write_note("FAILED: stopped by %r" % failure)
        raise
    finally:
        end_set_hold(slots, holds_noted, passed and keep_slot)

    # A set that passed stays good, but it must not read as the tips.
    try:
        moved = tips_moved(commits, tips())
    except Exception as error:  # noqa: BLE001 - a tip that is not known now is a note, not a fault of the set
        moved = "the tips are not known now (%s)" % error
    if moved:
        notes.append("NOT THE TIPS: %s during the smoke" % moved)

    # current.bat first: a note that says passed then always has a set that current.bat names.
    try:
        bat = write_current_bat(sets_root, name)
    except (OSError, UnicodeEncodeError) as error:
        write_note("FAILED: the smoke passed, but %s was not written: %s" % (CURRENT_BAT, error))
        say("%s: the smoke passed, but %s was not written: %s" % (name, os.path.join(sets_root, CURRENT_BAT), error))
        return 1
    write_note(result + checkout_note + ("; " + "; ".join(notes) if notes else ""))
    say(read_text(note).strip())
    say("current build: %s names %s" % (bat, name))
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--force", action="store_true", help="smoke the set of the tips when the current set has them already")
    parser.add_argument("--no-wait", action="store_true", help="stop with exit code 3 where this would wait for a slot")
    parser.add_argument("--keep-slot", action="store_true", help="after a pass, keep the game slot for this session's next work, and release it yourself")
    parser.add_argument("--build-only", action="store_true", help="gone (W521): make a set alone with tools/build_set.py")
    parser.add_argument("--bats-only", action="store_true", help="the step of the owner's start alone, on the current set")
    parser.add_argument("--self-test", action="store_true", help="the tests of the pure parts: no build, no game")
    args = parser.parse_args()
    if inside(PDB_KEEP, SETS):
        raise SystemExit("PDB_KEEP %s is in SETS %s: no pdb may go into a set" % (PDB_KEEP, SETS))
    if args.self_test:
        self_test()
        return 0
    if args.build_only:
        say(BUILD_ONLY_GONE)
        return 1

    owner_local_bat = os.path.join(PLUGIN_REPO, "local.bat")
    if args.bats_only:
        slots = load_slots()
        os.makedirs(TREES, exist_ok=True)
        if hold_run_lock(os.path.join(TREES, "make_play_set.lock")) is None:
            say("another run of this tool is at work, and this one stops")
            return 3
        return bats_only(slots, owner_local_bat, args.no_wait)

    import build_set

    try:
        commits = tip_commits()
    except build_set.SetFault as fault:
        say(str(fault))
        return fault.code
    say("the tips: %s" % commits_text(commits))
    if not args.force and current_set_commits() == commits:
        say("the current set has these tips and a smoke that passed, and nothing is made")
        return 0

    slots = load_slots()
    os.makedirs(TREES, exist_ok=True)
    if hold_run_lock(os.path.join(TREES, "make_play_set.lock")) is None:
        say("another run of this tool is at work, and this one stops")
        return 3
    head = git(PLUGIN_REPO, "rev-parse", "HEAD")
    if head != commits["nirender"] and not args.force:
        say("the owner's checkout is at %s, not %s, and nothing is made: use --force to make this set anyway" % (head[:8], commits["nirender"][:8]))
        return 1
    return make_and_smoke(slots, commits, owner_local_bat, args.no_wait, args.keep_slot)


if __name__ == "__main__":
    sys.exit(main())
