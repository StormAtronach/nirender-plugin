"""The A/B stages of the merge step that need the data of a game instance, such as the grass of Outlander-test.

    python -I tools/instance_ab.py base-build --plugin-repo DIR --mge-repo DIR --plugin-tip SHA --mge-tip SHA --out DIR --work DIR
    python -I tools/instance_ab.py run --stage grass --baseline DIR --base-build DIR --now-build DIR --out DIR
    python -I tools/instance_ab.py --self-test    # the tests of the pure parts: no build, no game

The replay gate (validate_retained.py baseline) replays captures of the testing instance, which holds one grass place,
so no grass code runs in it. A stage of this tool replays a capture of an instance that does hold grass, with the data
of that instance (the distant land, the game folder) and a COPY of its mgeXE.toml. It holds no accepted pictures: the
data of the instance changes at each full regeneration (D128, D129), and a picture of the day before would fail every
merge. The stage compares two builds on the same data, on the same day:

- the BASE build, `cargo xtask dist` at the tip before the merge of the repo under test and at the HEAD of the sibling
  tree, which the build under test uses too (numidium.exe comes from both repos, so the A/B shows the change of one repo).
  `base-build` keeps it in a folder with a key file: the commit of each repo and the profile. It builds again when a
  commit or the profile differs from the key, and uses the kept build when both commits equal it;
- the build UNDER TEST (the dist-staging folder of the gate's build).

`run` replays the capture with the base, with the build under test and with the base again, in that order, each with
`--frames-at`. The second replay of the base is the null test: the base must equal itself, or the stage says "noise" and
fails. The pictures of the build under test and the hash lines of its grass calls (`MGE_XE_GRASS_HASH=1`: the records in
the order of the draw and as a set) must equal those of the base. A picture does not show the order of the draws; the
ordered hash does. When the base prints no hash line, the stage judges the pictures and prints "hash NOT RUN (base has no
hash line)": never a PASS of the hash.

A stage prints NOT RUN, and exits with code 5, when it cannot judge:

- an input is missing (the capture, the config copy, the distant land, the game folder, a build);
- the game slot is held or reserved for a run that names the instance (the game start regenerates the distant land), its
  status is not "game: free" or "game: held ... by", or a Morrowind.exe from the instance's folder runs;
- a file that the generator writes (`generation_report.toml`, `generation_state.bin`) changed its time or size between the
  first replay and the last;
- the base prints hash lines and none of them holds a grass record (the capture does not reach the code).

Exit code 0 is a PASS, 1 a FAIL, 5 NOT RUN. The inputs of a stage are in `<baseline>/instance/<stage>/`: the capture and
`mgeXE.toml`, a copy of the instance's file; the accepted baseline folder serves when the named one holds none. A planned
change of the grass fails the A/B, and the Coordinator judges it from the numbers, as for any planned new look.
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from common import BUILD_PARTS, NOT_RUN_EXIT, build_missing, capture_format_in, docs_place, docs_root, local, missing_docs_text  # noqa: E402

PASS, FAIL, NOT_RUN = 0, 1, NOT_RUN_EXIT
KEEP_FOLDER = "E:\\nirender-keep\\merge-step"
BASE_WORK = os.path.join(KEEP_FOLDER, "base-work")
PLUGIN_NAME = "nirender-plugin"
MGE_NAME = "MGE-XE-G7-render-link"
PROFILE = "dist"
HASH_PREFIX = "grass hash frame "
KEY_FILE = "key.txt"
BUILD_FOLDER = "build"


@dataclass(frozen=True)
class InstanceStage:
    """One capture of a game instance, and what its replay reads."""

    name: str
    capture: str
    config: str
    distant_land: str
    game_dir: str
    frames: str
    # Files of the distant land that the generator writes, checked before the first replay and after the last.
    guard: tuple
    # The variables of the replays.
    env: dict = field(default_factory=dict)
    # The instance's name, as the label of a game run names it, and its folder.
    instance: str = ""
    instance_dir: str = ""


OUTLANDER = "D:/Modlists/Outlander-test"
GRASS = InstanceStage(name="grass", capture="outlander-pelagiad.nircap", config="mgeXE.toml",
                      distant_land=local("outlander_distant_land", OUTLANDER + "/overwrite/distantland"),
                      game_dir=local("outlander_game_dir", OUTLANDER + "/Stock Game Folder"),
                      frames="1,15,29", guard=("generation_report.toml", "generation_state.bin"),
                      env={"MGE_XE_GRASS_HASH": "1"}, instance="Outlander-test", instance_dir=local("outlander_dir", OUTLANDER))
STAGES = {stage.name: stage for stage in (GRASS,)}


# --- the pure parts ------------------------------------------------------------------------

def stage_folder(baseline, stage):
    """The folder of the inputs of a stage in a baseline folder."""
    return os.path.join(baseline, "instance", stage.name)


def inputs_missing(stage, folder):
    """The inputs of a stage that are not there, as paths."""
    wanted = [os.path.join(folder, stage.capture), os.path.join(folder, stage.config), stage.distant_land, stage.game_dir]
    wanted += [os.path.join(stage.distant_land, name) for name in stage.guard]
    return [path for path in wanted if not os.path.exists(path)]


def file_state(paths):
    """The time and size of each file, or None for a file that is not there."""
    state = {}
    for path in paths:
        try:
            info = os.stat(path)
            state[path] = (info.st_mtime_ns, info.st_size)
        except OSError:
            state[path] = None
    return state


def guard_problem(before, after):
    """Why the data changed between two reads of `file_state`, as a text, or None."""
    changed = [os.path.basename(path) for path in before if before[path] != after.get(path)]
    if changed:
        return "%s changed between the first replay and the last: a game start regenerates the distant land" % ", ".join(changed)
    return None


def game_blocker(lines, instance):
    """Why the game slot stops a stage, as a text, or None. `lines` are the lines of the slot's status for the game: the
    first one says "game: free" or "game: held (timing) by <the holder>" ("held (not timing)", D297), and a reservation
    follows on a line of its own. Any other
    first line (a fault, a lease that ended) stops the stage, as the tool cannot tell who runs. A run or a reservation on
    another instance does not touch this one's data; a holder whose label names no instance is seen by `process_blocker`."""
    if not instance:
        return None
    first = lines[0] if lines else ""
    if re.match(r"game: held (\((not )?timing\) )?by ", first):
        if instance.lower() in first.lower():
            return "the game slot is held by a run on %s: %s" % (instance, first)
    elif not first.startswith("game: free"):
        return "the game slot is neither free nor held: %s" % (first or "no status line")
    for line in lines[1:]:
        if line.strip().startswith("reserved by") and instance.lower() in line.lower():
            return "the game slot is reserved for a run on %s: %s" % (instance, line.strip())
    return None


def is_inside(path, folder):
    """The path is the folder or in it, as Windows reads names."""
    path, folder = os.path.normcase(os.path.abspath(path)), os.path.normcase(os.path.abspath(folder))
    return path == folder or path.startswith(folder + os.sep)


def process_blocker(paths, folders):
    """Why a running Morrowind.exe stops a stage, as a text, or None. `paths` are the exe paths of the running
    Morrowind.exe processes; a game from the instance's folder rewrites its distant land, whatever its slot label says."""
    for path in paths:
        if any(folder and is_inside(path, folder) for folder in folders):
            return "a Morrowind.exe of the instance runs: %s" % path
    return None


def process_listing():
    """The exe paths of the running Morrowind.exe processes. It raises when the list cannot be read."""
    command = ["powershell", "-NoProfile", "-NonInteractive", "-Command",
               "Get-CimInstance Win32_Process -Filter \"Name='Morrowind.exe'\" | ForEach-Object { $_.ExecutablePath }"]
    done = subprocess.run(command, capture_output=True, text=True, stdin=subprocess.DEVNULL, timeout=60)
    if done.returncode != 0:
        raise RuntimeError("the process list failed: %s" % done.stderr.strip()[:200])
    return [line.strip() for line in done.stdout.splitlines() if line.strip()]


def base_key(plugin_tip, mge_tip, profile=PROFILE):
    """The text of the key file of a base build."""
    return "%s %s\n%s %s\nprofile %s\n" % (PLUGIN_NAME, plugin_tip, MGE_NAME, mge_tip, profile)


def key_problem(kept, wanted):
    """Why a kept base build cannot serve, as a text, or None. `kept` is the text of the key file, or None."""
    if kept is None:
        return "no kept base build"
    if kept == wanted:
        return None
    old, new = dict(), dict()
    for target, text in ((old, kept), (new, wanted)):
        for line in text.splitlines():
            name, _, value = line.partition(" ")
            target[name] = value
    differs = [name for name in new if old.get(name) != new[name]]
    return "the kept base build differs in %s" % ", ".join(differs or ["its key"])


def hash_lines(output):
    """The hash lines of a replay's output, in order, each from its prefix to the end of the line."""
    lines = []
    for line in output.splitlines():
        at = line.find(HASH_PREFIX)
        if at >= 0:
            lines.append(line[at:].strip())
    return lines


def hash_records(lines):
    """The records that the hash lines count, all calls together."""
    total = 0
    for line in lines:
        _, _, rest = line.partition(": ")
        count, _, _ = rest.partition(" records")
        total += int(count) if count.isdigit() else 0
    return total


# The ordered hash of a list with no record: the start value of the hash.
EMPTY_ORDERED = "cbf29ce484222325"


def empty_hash_lines(lines):
    """The hash lines that count records and show the ordered hash of an empty list: a hash that hashes nothing."""
    found = []
    for line in lines:
        _, _, rest = line.partition(": ")
        if hash_records([line]) > 0 and ("ordered " + EMPTY_ORDERED) in rest:
            found.append(line)
    return found


def hash_summary(lines):
    """What the hash lines hold, as a text: the calls, the records, and the different ordered hashes."""
    ordered = {line.partition("ordered ")[2].partition(",")[0] for line in lines if "ordered " in line}
    return "%d calls, %d records, %d different ordered hashes" % (len(lines), hash_records(lines), len(ordered))


def line_differences(a, b, what="hash line", limit=3):
    """The differences between two lists of lines, as texts: the length, and the first lines that differ."""
    found = []
    if len(a) != len(b):
        found.append("%d %ss against %d" % (len(a), what, len(b)))
    for index, (one, other) in enumerate(zip(a, b)):
        if one != other:
            found.append("%s %d: %s | %s" % (what, index, one, other))
            if len(found) > limit:
                break
    return found


def picture_differences(folder_a, folder_b, compare):
    """The pictures of the two folders that differ or are in one folder only, as texts. `compare(a_path, b_path)`
    returns None for equal pictures, else a text of the numbers."""
    found = []
    names_a, names_b = [sorted(name for name in os.listdir(folder) if name.endswith(".png")) if os.path.isdir(folder) else [] for folder in (folder_a, folder_b)]
    if not names_a:
        return ["no picture in %s" % folder_a]
    for name in sorted(set(names_a) | set(names_b)):
        if name not in names_a:
            found.append("%s: no picture in %s" % (name, folder_a))
            continue
        other = os.path.join(folder_b, name)
        if not os.path.isfile(other):
            found.append("%s: no picture in %s" % (name, folder_b))
            continue
        text = compare(os.path.join(folder_a, name), other)
        if text:
            found.append("%s: %s" % (name, text))
    return found


def judge(null_pictures, null_hash, pictures, hashes, base_hash, now_hash):
    """The verdict of a stage: (code, lines). The first four arguments are the lists of differences of the null test
    (base against base) and of the A/B (base against the build under test); `base_hash` and `now_hash` are the hash lines
    of the base and of the build under test."""
    if null_pictures or null_hash:
        return FAIL, ["FAIL noise: the base differs from itself, so the stage cannot judge the build under test"] + ["       " + line for line in null_pictures + null_hash]
    for who, found in (("base", empty_hash_lines(base_hash)), ("build under test", empty_hash_lines(now_hash))):
        if found:
            return FAIL, ["FAIL a hash line of the %s counts records and hashes none: %s" % (who, found[0])]
    if base_hash and hash_records(base_hash) == 0:
        return NOT_RUN, ["NOT RUN: the base prints hash lines and none holds a grass record: the capture does not reach the grass code"]
    lines = []
    failures = list(pictures)
    if not base_hash:
        lines.append("hash NOT RUN (base has no hash line)")
    elif not now_hash:
        failures.append("the build under test prints no hash line, and the base prints %d" % len(base_hash))
    else:
        failures += hashes
        if not hashes:
            lines.append("hash lines equal: %d calls, %d records" % (len(base_hash), hash_records(base_hash)))
    if failures:
        return FAIL, ["FAIL the build under test differs from the base"] + ["       " + line for line in failures] + lines
    return PASS, ["PASS pictures equal to the base, and the base equal to itself"] + lines


def named(line, name):
    """A verdict line with the stage's name after its verdict word."""
    for word in ("PASS", "FAIL", "NOT RUN"):
        if line.startswith(word):
            return "%s %s%s" % (word, name, line[len(word):])
    return line


# --- the base build ------------------------------------------------------------------------

def sync_folder(source, target):
    """Makes `target` hold the files of `source`: copies the new and the changed ones, and removes the files that are not in
    `source`, one by one. It removes no folder."""
    wanted = set()
    for folder, _, names in os.walk(source):
        relative = os.path.relpath(folder, source)
        os.makedirs(os.path.join(target, relative), exist_ok=True)
        for name in names:
            path = os.path.normpath(os.path.join(relative, name))
            wanted.add(path)
            one, other = os.path.join(source, path), os.path.join(target, path)
            if not os.path.isfile(other) or os.stat(one).st_size != os.stat(other).st_size or os.stat(one).st_mtime_ns != os.stat(other).st_mtime_ns:
                shutil.copy2(one, other)
    for folder, _, names in os.walk(target):
        for name in names:
            path = os.path.normpath(os.path.join(os.path.relpath(folder, target), name))
            if path not in wanted:
                os.remove(os.path.join(target, path))


def git_out(tree, *args):
    done = subprocess.run(["git", "-C", tree, *args], capture_output=True, text=True)
    if done.returncode != 0:
        raise SystemExit("git %s in %s: %s" % (" ".join(args), tree, done.stderr.strip()))
    return done.stdout.strip()


def put_worktree(tree, path, sha):
    """A worktree of the repository of `tree` at `path`, at the commit `sha`, detached."""
    if os.path.isdir(os.path.join(path, ".git")) or os.path.isfile(os.path.join(path, ".git")):
        if git_out(path, "status", "--porcelain", "--untracked-files=no"):
            raise SystemExit("%s has tracked changes: the base build does not touch a tree that is not clean" % path)
        git_out(path, "checkout", "--detach", sha)
    else:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        git_out(tree, "worktree", "add", "--detach", path, sha)


def base_build(args, out=print):
    """Keeps the base build of the current tips: returns the exit code."""
    wanted = base_key(args.plugin_tip, args.mge_tip)
    key_path = os.path.join(args.out, KEY_FILE)
    kept = open(key_path, encoding="ascii").read() if os.path.isfile(key_path) else None
    problem = key_problem(kept, wanted)
    if problem is None:
        out("the kept base build holds both tips: %s" % args.out)
        return PASS
    out("building the base: %s" % problem)
    plugin_work, mge_work = os.path.join(args.work, PLUGIN_NAME), os.path.join(args.work, MGE_NAME)
    put_worktree(args.plugin_repo, plugin_work, args.plugin_tip)
    put_worktree(args.mge_repo, mge_work, args.mge_tip)
    # The key file goes first: a build that fails leaves no key that a later run could trust.
    if os.path.isfile(key_path):
        os.remove(key_path)
    began = time.time()
    code = subprocess.run(["cargo", "xtask", "dist"], cwd=mge_work, stdin=subprocess.DEVNULL).returncode
    if code != 0:
        out("FAIL the base build: cargo xtask dist exit code %d" % code)
        return FAIL
    staging = os.path.join(mge_work, "target", "dist-staging")
    missing = build_missing(staging)
    if missing:
        out("FAIL the base build has no %s" % ", ".join(missing))
        return FAIL
    sync_folder(staging, os.path.join(args.out, BUILD_FOLDER))
    with open(key_path, "w", encoding="ascii", newline="\n") as key:
        key.write(wanted)
    out("the base build is kept in %s (%d s)" % (args.out, time.time() - began))
    return PASS


# --- the replays ---------------------------------------------------------------------------

def clear_files(folder):
    """Removes the files of `folder`, one by one."""
    if os.path.isdir(folder):
        for entry in os.scandir(folder):
            if entry.is_file(follow_symlinks=False):
                os.remove(entry.path)


def picture_compare(a, b):
    """None for two equal pictures, else the numbers as a text."""
    import picture_diff

    numbers = picture_diff.compare(picture_diff.load(a), picture_diff.load(b))
    if "problem" in numbers:
        return numbers["problem"]
    if numbers["different"] == 0:
        return None
    return "%.2f %% different, mean %.3f, largest %d" % (numbers["different"] * 100, numbers["mean"], numbers.get("largest", 0))


def replay(stage, folder, build, out, tag):
    """One replay of the capture with `build`: (exit code, output, folder of the pictures). Its mgeXE.toml is a copy that
    the helper may rewrite."""
    pictures = os.path.join(out, tag)
    config = os.path.join(out, tag + "-config")
    os.makedirs(pictures, exist_ok=True)
    os.makedirs(config, exist_ok=True)
    clear_files(pictures)
    shutil.copyfile(os.path.join(folder, stage.config), os.path.join(config, stage.config))
    command = [sys.executable, os.path.join(HERE, "replay.py"), os.path.join(folder, stage.capture), os.path.join(pictures, "p"),
               "--frames-at", stage.frames, "--mge-build", build, "--config-root", config, "--distant-land", stage.distant_land,
               "--game-dir", stage.game_dir]
    done = subprocess.run(command, capture_output=True, text=True, stdin=subprocess.DEVNULL, env=dict(os.environ, **stage.env))
    return done.returncode, done.stdout + done.stderr, pictures


HARNESS_PATH = ("docs", "scripts", "test-harness-2026-10-03")


def harness_folder():
    """The folder of slot.py, in the docs checkout of the tools (common.docs_place)."""
    folder = docs_place(*HARNESS_PATH)
    if not os.path.isfile(os.path.join(folder, "slot.py")):
        raise RuntimeError(missing_docs_text(docs_root(), HARNESS_PATH + ("slot.py",)))
    return folder


def slot_lines():
    """The lines of the slot status of the game."""
    sys.path.insert(0, harness_folder())
    import slot

    return slot.slot_status("game", time.time())


def run_stage(args, out=print, lines_of_game=slot_lines, compare=picture_compare, processes=process_listing):
    stage = STAGES[args.stage]
    folder = stage_folder(args.baseline, stage)
    if inputs_missing(stage, folder):
        # The accepted baseline folder serves when a named one holds no inputs.
        from make_play_set import BASELINE

        out("the named baseline %s holds no inputs of %s: the accepted folder serves" % (args.baseline, stage.name))
        folder = stage_folder(BASELINE, stage)
    out("inputs of %s from %s" % (stage.name, folder))
    missing = inputs_missing(stage, folder)
    for build, what in ((args.base_build, "base build"), (args.now_build, "build under test")):
        if build_missing(build):
            missing.append("%s %s" % (what, build))
    if missing:
        out("NOT RUN %s: missing: %s" % (stage.name, ", ".join(missing)))
        return NOT_RUN
    try:
        blocker = game_blocker(lines_of_game(), stage.instance) or process_blocker(processes(), (stage.game_dir, stage.instance_dir))
    except Exception as error:  # noqa: BLE001 - any fault of the slot tool or the process list means the same: the stage cannot tell
        out("NOT RUN %s: the game slot or the game processes cannot be read: %s" % (stage.name, error))
        return NOT_RUN
    if blocker:
        out("NOT RUN %s: %s" % (stage.name, blocker))
        return NOT_RUN

    guarded = [os.path.join(stage.distant_land, name) for name in stage.guard]
    before = file_state(guarded)
    runs = {}
    for tag, build in (("base", args.base_build), ("now", args.now_build), ("base-again", args.base_build)):
        began = time.time()
        code, output, pictures = replay(stage, folder, build, args.out, tag)
        out("replay %s: exit code %d, %d s, %d hash lines" % (tag, code, time.time() - began, len(hash_lines(output))))
        other_format = capture_format_in(output)
        if code != 0 and other_format:
            # The capture is from a build of another format: this stage cannot judge the two builds with it.
            for line in output.splitlines()[-6:]:
                out("       " + line)
            out("NOT RUN %s: the replay of the %s: %s" % (stage.name, tag, other_format))
            return NOT_RUN
        if code != 0:
            out("FAIL %s: the replay of the %s exited with code %d" % (stage.name, tag, code))
            for line in output.splitlines()[-6:]:
                out("       " + line)
            return FAIL
        runs[tag] = (pictures, hash_lines(output))
        out("       %s: %s" % (tag, hash_summary(runs[tag][1])))
    problem = guard_problem(before, file_state(guarded))
    if problem:
        out("NOT RUN %s: %s" % (stage.name, problem))
        return NOT_RUN

    code, lines = judge(picture_differences(runs["base"][0], runs["base-again"][0], compare),
                        line_differences(runs["base"][1], runs["base-again"][1]),
                        picture_differences(runs["base"][0], runs["now"][0], compare),
                        line_differences(runs["base"][1], runs["now"][1]),
                        runs["base"][1], runs["now"][1])
    out(named(lines[0], stage.name))
    for line in lines[1:]:
        out(line)
    return code


# --- the tests -----------------------------------------------------------------------------

def self_test():
    with tempfile.TemporaryDirectory() as root:
        # The folder of the inputs, and what is missing.
        stage = InstanceStage("t", "c.nircap", "m.toml", os.path.join(root, "dl"), os.path.join(root, "game"), "1", ("a.toml", "b.bin"))
        folder = stage_folder(os.path.join(root, "base"), stage)
        assert folder == os.path.join(root, "base", "instance", "t")
        assert len(inputs_missing(stage, folder)) == 6
        os.makedirs(folder)
        os.makedirs(stage.distant_land)
        os.makedirs(stage.game_dir)
        for path in (os.path.join(folder, "c.nircap"), os.path.join(folder, "m.toml"), os.path.join(stage.distant_land, "a.toml"), os.path.join(stage.distant_land, "b.bin")):
            open(path, "w").write("x")
        assert inputs_missing(stage, folder) == []
        os.remove(os.path.join(folder, "m.toml"))
        assert inputs_missing(stage, folder) == [os.path.join(folder, "m.toml")]

        # The race guard: a change of time or size of a generated file, and a file that appears or goes.
        guarded = [os.path.join(stage.distant_land, name) for name in stage.guard]
        before = file_state(guarded)
        assert guard_problem(before, file_state(guarded)) is None
        open(guarded[0], "w").write("longer")
        assert "a.toml changed" in guard_problem(before, file_state(guarded))
        before = file_state(guarded)
        os.utime(guarded[1], ns=(1, 1))
        assert "b.bin changed" in guard_problem(before, file_state(guarded))
        before = file_state(guarded)
        os.remove(guarded[1])
        assert "b.bin changed" in guard_problem(before, file_state(guarded))

        # The game slot: a run on the instance stops the stage, a run elsewhere and a free slot do not.
        assert game_blocker(["game: free"], "Outlander-test") is None
        assert game_blocker(["game: held (timing) by Ryker: grass capture on Outlander-test, since t"], "Outlander-test").startswith("the game slot is held by a run on Outlander-test")
        assert game_blocker(["game: held (not timing) by Rob: tour on Morrowind75testing, since t"], "Outlander-test") is None
        assert game_blocker(["game: held by Rob: tour on Morrowind75testing, since t"], "Outlander-test") is None
        # Any other first line stops the stage; a reservation is read; a Morrowind.exe from the instance's folder stops it.
        assert "neither free nor held" in game_blocker(["game: CORRUPT: x. The checks pass"], "Outlander-test")
        assert "neither free nor held" in game_blocker(["game: LEASE ENDED 3 min ago, holder x."], "Outlander-test")
        assert "no status line" in game_blocker([], "Outlander-test")
        assert game_blocker(["game: free", "  reserved by Ryker: grass on Outlander-test until t"], "Outlander-test").startswith("the game slot is reserved for a run on Outlander-test")
        assert game_blocker(["game: free", "  reserved by Rob: tour on Morrowind75testing until t"], "Outlander-test") is None
        assert game_blocker(["game: CORRUPT"], "") is None
        inside = os.path.join(root, "inst", "Stock Game Folder", "Morrowind.exe")
        assert process_blocker(["D:/other/Morrowind.exe", inside], (os.path.join(root, "inst", "Stock Game Folder"), "")) == "a Morrowind.exe of the instance runs: %s" % inside
        assert process_blocker(["D:/other/Morrowind.exe"], (os.path.join(root, "inst"), "")) is None and process_blocker([], (root,)) is None
        assert not is_inside(os.path.join(root, "inst2", "x"), os.path.join(root, "inst")) and is_inside(os.path.join(root, "INST", "x"), os.path.join(root, "inst"))

        # The key of the base build: both tips and the profile.
        key = base_key("p1", "m1")
        assert key_problem(key, key) is None and key_problem(None, key) == "no kept base build"
        assert key_problem(base_key("p2", "m1"), key) == "the kept base build differs in %s" % PLUGIN_NAME
        assert key_problem(base_key("p1", "m2"), key) == "the kept base build differs in %s" % MGE_NAME
        assert "profile" in key_problem(base_key("p1", "m1", "other"), key)
        both = key_problem(base_key("p2", "m2"), key)
        assert PLUGIN_NAME in both and MGE_NAME in both

        # The hash lines, and the records they count.
        one = "grass hash frame 1 main: 3 records, cut false, ordered 00000000000000aa, unordered 00000000000000bb"
        two = "grass hash frame 2 view: 0 records, cut false, ordered 00000000000000cc, unordered 0000000000000000"
        output = "other line\n[host] " + one + "\nmore\n" + two + "\n"
        assert hash_lines(output) == [one, two] and hash_lines("nothing") == []
        assert hash_summary([one, two]) == "2 calls, 3 records, 2 different ordered hashes" and hash_summary([]) == "0 calls, 0 records, 0 different ordered hashes"
        assert hash_records([one, two]) == 3 and hash_records([two]) == 0 and hash_records([]) == 0
        assert line_differences([one], [one]) == [] and "2 hash lines against 1" in line_differences([one, two], [one])[0]
        assert "hash line 0" in line_differences([one], [two])[0]

        # The pictures: equal, different, missing; and no picture at all.
        a, b = os.path.join(root, "a"), os.path.join(root, "b")
        os.makedirs(a)
        os.makedirs(b)
        for name, text in (("p-1.png", "x"), ("p-2.png", "y")):
            open(os.path.join(a, name), "w").write(text)
            open(os.path.join(b, name), "w").write(text)
        same = lambda x, y: None if open(x).read() == open(y).read() else "differs"  # noqa: E731
        assert picture_differences(a, b, same) == []
        open(os.path.join(b, "p-2.png"), "w").write("z")
        assert picture_differences(a, b, same) == ["p-2.png: differs"]
        os.remove(os.path.join(b, "p-1.png"))
        assert picture_differences(a, b, same)[0].startswith("p-1.png: no picture")
        assert picture_differences(os.path.join(root, "none"), b, same) == ["no picture in %s" % os.path.join(root, "none")]
        # A picture that only the second folder holds is a difference too: the base with one frame and the build with two.
        open(os.path.join(b, "p-1.png"), "w").write("x")
        open(os.path.join(b, "p-29.png"), "w").write("x")
        assert picture_differences(a, b, same) == ["p-2.png: differs", "p-29.png: no picture in %s" % a]

        # The verdict. The null test comes first; a base with no hash line gives "hash NOT RUN", never a PASS of the hash.
        code, lines = judge([], [], [], [], [], [])
        assert code == PASS and lines[-1] == "hash NOT RUN (base has no hash line)"
        code, lines = judge([], [], [], [], [one], [one])
        assert code == PASS and "hash lines equal: 1 calls, 3 records" in lines[-1] and "NOT RUN" not in " ".join(lines)
        code, lines = judge(["p-1.png: differs"], [], [], [], [one], [one])
        assert code == FAIL and "noise" in lines[0]
        code, lines = judge([], ["hash line 0: x"], [], [], [one], [one])
        assert code == FAIL and "noise" in lines[0]
        code, lines = judge([], [], ["p-1.png: differs"], [], [], [])
        assert code == FAIL and lines[-1] == "hash NOT RUN (base has no hash line)"
        code, lines = judge([], [], [], ["hash line 0: a | b"], [one], [two])
        assert code == FAIL and any("hash line 0" in line for line in lines)
        code, lines = judge([], [], [], [], [one], [])
        assert code == FAIL and "prints no hash line" in " ".join(lines)
        empty = "grass hash frame 3 main: 4 records, cut false, ordered cbf29ce484222325, unordered 0000000000000000"
        assert empty_hash_lines([one, two, empty]) == [empty]
        code, lines = judge([], [], [], [], [one], [empty])
        assert code == FAIL and "build under test counts records and hashes none" in lines[0]
        code, lines = judge([], [], [], [], [empty], [empty])
        assert code == FAIL and "base counts records" in lines[0]
        code, lines = judge([], [], [], [], [two], [two])
        assert code == NOT_RUN and "none holds a grass record" in lines[0]

        # The kept base folder follows the build: new and changed files copied, stale files removed one by one.
        source, target = os.path.join(root, "src"), os.path.join(root, "dst")
        os.makedirs(os.path.join(source, "Data Files"))
        os.makedirs(os.path.join(target, "old"))
        open(os.path.join(source, "numidium.exe"), "w").write("new")
        open(os.path.join(source, "Data Files", "s.fx"), "w").write("shader")
        open(os.path.join(target, "numidium.exe"), "w").write("older!")
        open(os.path.join(target, "old", "gone.dll"), "w").write("gone")
        sync_folder(source, target)
        assert open(os.path.join(target, "numidium.exe")).read() == "new" and open(os.path.join(target, "Data Files", "s.fx")).read() == "shader"
        assert not os.path.exists(os.path.join(target, "old", "gone.dll")) and os.path.isdir(os.path.join(target, "old"))

        # The verdict word comes first, then the stage.
        assert named("PASS pictures equal", "grass") == "PASS grass pictures equal" and named("NOT RUN: x", "grass") == "NOT RUN grass: x"
        assert named("FAIL noise: x", "grass") == "FAIL grass noise: x" and named("other", "grass") == "other"

        # The stage with its replays stubbed: the order of the runs is base, build under test, base again.
        stage_args = argparse.Namespace(stage="grass", baseline=root, base_build=os.path.join(root, "bb"), now_build=os.path.join(root, "nb"), out=os.path.join(root, "o"))
        said = []
        assert run_stage(stage_args, said.append, lambda: ["game: free"], same) == NOT_RUN and any("missing" in line for line in said) and any(line.startswith("inputs of grass from") for line in said) and said[0].startswith("the named baseline")

        # A game slot that cannot be read is a NOT RUN, with the reason (a stage with its inputs in place).
        full = InstanceStage("g", "c.nircap", "m.toml", stage.distant_land, stage.game_dir, "1", ())
        STAGES["g"] = full
        try:
            os.makedirs(os.path.join(root, "base", "instance", "g"), exist_ok=True)
            for name in ("c.nircap", "m.toml"):
                open(os.path.join(root, "base", "instance", "g", name), "w").write("x")
            for build in ("bb", "nb"):
                for part in BUILD_PARTS:
                    os.makedirs(os.path.dirname(os.path.join(root, build, part)), exist_ok=True)
                    open(os.path.join(root, build, part), "w").write("x")

            def unreadable():
                raise RuntimeError("no slot.py")

            said = []
            gargs = argparse.Namespace(stage="g", baseline=os.path.join(root, "base"), base_build=os.path.join(root, "bb"), now_build=os.path.join(root, "nb"), out=os.path.join(root, "o"))
            assert run_stage(gargs, said.append, unreadable, same) == NOT_RUN and any("cannot be read: no slot.py" in line for line in said), said

            # A free slot and a Morrowind.exe in the game folder: the process list stops the stage before any replay.
            said = []
            running = os.path.join(full.game_dir, "Morrowind.exe")
            assert run_stage(gargs, said.append, lambda: ["game: free"], same, lambda: [running]) == NOT_RUN and any("a Morrowind.exe of the instance runs: %s" % running in line for line in said), said
            assert not any(line.startswith("replay ") for line in said), said

            # A capture of another extension format (the grass capture at the merge step of 79c752d): NOT RUN, with the
            # line of replay.py; another exit of the replay stays a FAIL.
            global replay
            real_replay = replay
            stale = "CAPTURE FORMAT: c.nircap was made by d3d8.dll (MGE-XE G7 Fork v0.21.0 99c8b62-dirty), which writes the extension format 1; x"
            try:
                replay = lambda *_: (11, "make the capture again with this build\n" + stale + "\n", root)  # noqa: E731
                said = []
                assert run_stage(gargs, said.append, lambda: ["game: free"], same, lambda: []) == NOT_RUN, said
                assert "NOT RUN g: the replay of the base: " + stale in said and not any(line.startswith("FAIL") for line in said), said
                replay = lambda *_: (11, "a panic\n", root)  # noqa: E731
                said = []
                assert run_stage(gargs, said.append, lambda: ["game: free"], same, lambda: []) == FAIL, said
                assert "FAIL g: the replay of the base exited with code 11" in said, said
            finally:
                replay = real_replay
        finally:
            del STAGES["g"]
    print("instance_ab self-test: ok")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", nargs="?", choices=("base-build", "run"))
    parser.add_argument("--plugin-repo")
    parser.add_argument("--mge-repo")
    parser.add_argument("--plugin-tip")
    parser.add_argument("--mge-tip")
    parser.add_argument("--out")
    parser.add_argument("--work", default=BASE_WORK)
    parser.add_argument("--stage", choices=sorted(STAGES))
    parser.add_argument("--baseline")
    parser.add_argument("--base-build")
    parser.add_argument("--now-build")
    parser.add_argument("--self-test", action="store_true", help="the tests of the pure parts: no build, no game")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.command == "base-build":
        if not all((args.plugin_repo, args.mge_repo, args.plugin_tip, args.mge_tip, args.out)):
            parser.error("base-build needs --plugin-repo, --mge-repo, --plugin-tip, --mge-tip and --out")
        return base_build(args)
    if args.command == "run":
        if not all((args.stage, args.baseline, args.base_build, args.now_build, args.out)):
            parser.error("run needs --stage, --baseline, --base-build, --now-build and --out")
        return run_stage(args)
    parser.error("it needs a command, or --self-test")


if __name__ == "__main__":
    sys.exit(main())
