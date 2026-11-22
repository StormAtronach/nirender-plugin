"""The checks of the retained pipeline, one stage at a time: each stage prints its result at
once and a failed stage stops the run, so a problem shows after one game run (about two
minutes) and not after all of them.

    python tools/validate_retained.py [stage ...] [--mge-build SET] [--places seyda,narsis]
    python tools/validate_retained.py baseline --archive DIR    # the release check of release.py
    python -I tools/validate_retained.py --self-test    # the tests of the pure parts, no game

The build (W545): --mge-build names a sealed set, and without it the stages take the current set.
The runs and the replays check the set themselves (the runner's rule of W521). --archive names an
unpacked release archive, which has no manifest; it goes to replay.py and validate_tour.py as
--archive, and only the stages that only replay (baseline, order) take it.

The helper draws from the records only, so a picture has no second path to compare with on
the same capture. The reference of a change of the helper is a baseline: captures that are
kept, each with the picture that was accepted for it.

Stages (capture, baseline and only by default, in this order):
  capture  a run at each place with a capture; the counts of the helper must show no shape
           without a geometry or a material and no light without a record, and the plugin
           no draw without a record; the capture replayed must give a picture and no GPU
           error.
  baseline first the stream rule on every kept capture (`stream_rule.py`: the player's views of a
           frame have the projection of the world scene; no capture with a scene of kind 5 is
           NOT RUN and fails the stage). Then it
           replays each capture of the baseline folder (--baseline, results/baseline-records by
           default: `<name>.nircap` files, frame 1) as a player has them and compares the
           picture
           with `accepted/<name>.png` there. --accept
           writes the pictures instead: for a new baseline, or after a change that is
           meant to change them. A tour run in `tour/` of the baseline folder is replayed
           and compared the same way (`tour/accepted`). The adapter of the replays (the
           GPU, the backend, the driver, as the helper prints it) is written with the
           accepted pictures (`accepted/adapter.txt`) and printed with the result, after
           a fixed word: SAME, OTHER (another adapter: the pictures can differ by the
           GPU) or UNKNOWN (no adapter printed or recorded). release.py reads the word.
           The replays run with MGE_XE_CORE_MODS=0: no core mod joins the shaders.
           Then a second pass, the hook fixtures (W528 item 5): the same captures with our hook
           files, one for each kind of hook, read only from --hook-fixtures (tools/hook-fixtures;
           MGE_XE_CORE_MODS_DIR), against `accepted-hooks/` of the baseline folder, with the
           objects check and no tour. Each replay must show the route: the helper names the
           folder, reads each fixture and no other file, uses each with all of its hooks, and
           prints no other line of the core mods (a dropped file, a failed home, a marker).
           Last, the objects check (D288): each capture again to its last frame, plain (no GPU
           time, no marks: nothing waits for the GPU), on vulkan and on dx12. A steady frame (no
           record that can need a new GPU object, no change of the extension) that made a GPU
           object fails, with the kinds and the lines of code of the objects
           (`host/src/objects.rs`; a warning while OBJECTS_STRICT is False). The view of the
           window's surface is allowed (D310), at most 1 in a steady frame. A capture with no
           steady frame cannot show the rule: it gives a warning with the first changes of its
           frames. The stage fails when no capture of a backend has a steady frame. A lost
           device and a build without the count fail.
  only     a run with the rate step: prints the frame rate, the time of the ports for each
           frame and the busy time of the helper. With --gpu-time the helper writes GPU
           time stamps (they cost it about 0.4 ms a frame).
  tour     a run over the NIF test level with a capture for each cell; its counts. Copy
           the run folder to `tour/` of the baseline folder to make it the baseline.
  clusters the newest capture of each place replayed with the clusters of lights
           (MGE_XE_CLUSTERS=1; `distant_land.clustered_lights` otherwise) and without, compared; then the
           rate step with and without.
  weathers a run that measures the frame rate of each of the ten weathers of the game in
           turn, at each place.
  order    (W535, verdict 595) each capture of the baseline folder and of its tour replayed to frame 1
           and its last frame with MGE_XE_TABLE_ORDER=grouped and =engine, the two pictures of each
           frame compared: 0 pixels may differ. Each replay must print "table order: <its value>"
           (else NOT RUN) and "table order digest: frame <N> <hash> <count> draws" for each pictured
           frame (else NOT RUN); the counts must be equal. At least one capture must have a frame
           whose digests differ, else the two values drew in one order and the pixels cannot show
           it: NOT SHOWN, which fails the stage. The merge step runs it after the replay gate when a
           file of the order changes (ORDER_PATHS of merge_step.py). It accepts nothing.
  accept-list  (W567 part D) with --gate-list FILE and --now <the gate's pictures>: the listed pictures of
           that run become the accepted ones, and nothing else. The merge step runs it as its last stage,
           so only after every stage passed. Each listed picture must differ from its accepted one; the
           pictures of a capture need a second replay that gives them again (as --accept-new); then the
           replay logs of those folders are rewritten too, not counted as changes (D557).

--gate-list FILE (W567 part D) names the planned changes of the baseline stage: one picture for each line,
relative to the baseline folder ("narsis.png", "narsis-last.png", "tour/3-03-_nif_052.png",
"hooks/narsis.png"; "#" starts a comment). The pictures that differ from the accepted ones must equal the
list exactly: a change that the list does not name fails, a listed picture that came out equal fails, and a
name with no accepted picture fails before the replays. Every other check of the stage stays (the counts, a
stray accepted picture, the objects, the tour's own pairs and their KNOWN_STOPS). The verdict names the file
and its md5. With --accept or --accept-new it is refused.

--disable-mod NAME switches a mod of the profile off for the runs (an A/B of a mod). Both arms have the
instance's distant land: the runner keeps MGE XE from a rebuild of it in a run that changes what the game loads
(run_own_device.py, the guard of W535), so the arm without the mod draws the distant land that was made with it.

--weather N pins the weather of the runs and holds it (the runner's --weather): 0, clear, by
default, so that a capture or a rate is made in a known weather. Each capture and rate stage
prints the run's weather line and fails when it does not show the pin held: "held" only when
the mod read the pinned weather back after its switch (run_own_device.py, weather_summary). The
tour step pins no weather, and the tour stage names its weather as not pinned. --weather any
leaves the game's weather, not pinned, and the line names it.
"""
import argparse
import contextlib
import hashlib
import io
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
sys.path.insert(0, HERE)
from common import (BUILD_PARTS, EARLY_WORLD_DRAWS, EXIT_REPLAY_CHECKS, EXIT_REPLAY_STAGE_SHADERS, NOT_RUN_EXIT, OWN_VIEWS, build_missing,  # noqa: E402
                    REPLAY_OUTPUT, capture_format_in, check_problems, helper_checks, keep_replay_output, plugin_lines, replay_fault_in)
from validate_tour import NOT_RUN, NOT_RUN_CODE as TOUR_NOT_RUN, refusal_of, verdict_in as tour_verdict_in  # noqa: E402
import stream_rule  # noqa: E402
PY = sys.executable
# The variables of the baseline replays: no core mod (`Data Files/shaders/core-mods/*.wgsl`) of
# the replay's folder joins the shaders, so the accepted pictures do not depend on what is
# installed.
GATE_ENV = {"MGE_XE_CORE_MODS": "0"}
# W528 item 5 (F-G-12): our hook files, one for each kind of hook, are fixtures of a second pass of the baseline stage
# (`hooks_pass`), read by the helper from this folder only (MGE_XE_CORE_MODS_DIR, W317). Their accepted state is in the
# folder HOOKS_ACCEPTED of the baseline folder.
HOOK_FIXTURES = os.path.join(HERE, "hook-fixtures")
HOOKS_ACCEPTED = "accepted-hooks"
# W567 part D: the planned changes of a gate (--gate-list FILE), the names of the pictures that must differ from their
# accepted ones, relative to the baseline folder: "<capture>.png" and "<capture>-last.png" of accepted/,
# "tour/<stop>.png" of tour/accepted/, "hooks/<capture>.png" of HOOKS_ACCEPTED. GATE_LIST holds the names and
# GATE_LIST_NOTE the file and its md5; CHANGED collects the names of the pictures that differed in this run.
TOUR_PREFIX, HOOKS_PREFIX = "tour/", "hooks/"
GATE_LIST = set()
GATE_LIST_NOTE = []
CHANGED = []
# A fixture is a file "XE Mod <name>.wgsl" of the folder; the README of the folder is not one.
HOOK_FIXTURE_NAME = re.compile(r"XE Mod .+\.wgsl")
# A hook of a fixture: a function at the start of a line whose name does not start with `mod_` (a helper of the file).
HOOK_DEFINITION = re.compile(r"^fn (?!mod_)(\w+)\(", re.M)
# The lines of the core mods in a replay's output that show the route (the helper's core_mods.rs and renderer.rs).
ROUTE_FOLDER = re.compile(r"core mods: only the folder (.+) \(MGE_XE_CORE_MODS_DIR\)")
ROUTE_READ = re.compile(r"core mods: read (.+)")
ROUTE_USED = re.compile(r"core mods: (XE Mod .+) \(interface mge-xe-core 1\)")
ROUTE_USED_FILE = re.compile(r"(XE Mod [^(]+?\.wgsl) \(([^)]*)\)")
# The other lines of the core mods that a fixture in use prints. Any other line of the core mods fails the route: a
# dropped file, a failed home or pipeline, a marker of a start before, a plant, a switch of the world text
# (verdict 480: a list of the lines that are fine, so that a new fault line of the helper fails too).
ROUTE_FINE = ("core mod: XE Mod Water.wgsl: the plane of the water takes its hooks; ",
              "core mod: XE Mod Water.wgsl: the debug view water shows the terms of the built-in colour",
              "core mod: XE Mod Sky.wgsl: the clouds take its clouds_color where the sky of MGE XE is drawn")
PLACES = {
    "seyda": ["--exterior", "Seyda Neen", "--position=-12288,-69632,74", "--yaw", "0"],
    "narsis": ["--exterior", "Narsis"],
    "council": ["--exterior", "Narsis, Council Quarter", "--position=53000,-412000", "--yaw", "270"],
    # Narsis at night, with the light of each pixel and its fade: the lights matter.
    "narsisnight": ["--exterior", "Narsis", "--hour", "0", "--set", "[distant_land] per_pixel_lighting = true", "--set", "[distant_land] per_pixel_light_fade = true"],
}


def run_code(args, env=None):
    """The exit code of a program and its output."""
    e = dict(os.environ)
    # The folder of the interpreter: an isolated interpreter (-I) finds the DLL of Pillow there.
    e["PATH"] = os.path.dirname(PY) + os.pathsep + e.get("PATH", "")
    e.update(env or {})
    result = subprocess.run(args, capture_output=True, text=True, env=e, cwd=REPO)
    return result.returncode, result.stdout + result.stderr


def newest(label, since=None):
    """The newest run folder of a label. With `since` (a time in RUN_TIME's form), only a folder of
    that time or later."""
    runs = sorted((d for d in os.listdir(os.path.join(REPO, "results")) if d.endswith("-" + label) and (since is None or d[:15] >= since)), reverse=True)
    return os.path.join(REPO, "results", runs[0]) if runs else None


GPU_TIME = {}
RUNNER_ARGS = []
# The runner's exit code of a run whose steps were done and whose game then crashed in its close
# (run_own_device.EXIT_CRASHED_AFTER_DONE, W417; the self-test checks the value). The gate judges the pictures of the
# steps, so such a run counts as ran, with a line.
RUNNER_CRASHED_AFTER_DONE = 5
CRASH_AFTER_DONE_LINE = "ERROR: the game crashed after DONE"
# The weather that the runs pin and hold (the runner's --weather), or None for --weather any.
WEATHER = {"pin": 0}
# The option that names the build to replay.py and validate_tour.py (W545): "--mge-build" for a sealed set, "--archive"
# for the unpacked release archive of release.py's gate. main sets it.
BUILD = {"option": "--mge-build"}
# The stages that only replay. The others start the game, and a run of the game does not take an archive here.
REPLAY_ONLY_STAGES = ("baseline", "order", "accept-list")


def build_option(mge_build):
    """The arguments of replay.py and validate_tour.py that name the build of the replays."""
    return [BUILD["option"], mge_build]


def build_problem(stages, mge_build, archive):
    """The line of a build that the stages cannot take, or None: --archive with --mge-build, or --archive with a stage
    that starts the game."""
    if archive and mge_build:
        return "an archive is not a set: no --mge-build with --archive"
    games = [stage for stage in stages if stage not in REPLAY_ONLY_STAGES]
    if archive and games:
        return "--archive is the release check, which only the stages %s replay; the stages %s start the game" % (", ".join(REPLAY_ONLY_STAGES), ", ".join(games))
    return None


def gate_build(mge_build, archive, current=None):
    """The build of the stages, and BUILD's option for it: the archive, else --mge-build, else the folder of the current
    set (`clean_common.current_set`; `current` is the self-test's stand-in). None when there is none. replay.py and the
    runner check the set itself."""
    import clean_common
    BUILD["option"] = "--archive" if archive else "--mge-build"
    if archive or mge_build:
        return archive or mge_build
    name = (current or clean_common.current_set)()
    if not name:
        return None
    folder = os.path.join(clean_common.SETS, name)
    print("the build: the current set %s" % folder, flush=True)
    return folder


def weather_pin(text):
    """The weather of --weather and a problem text: (0 to 9, None), (None, None) for `any`, or
    (None, the problem)."""
    if text == "any":
        return None, None
    if text.isdigit() and 0 <= int(text) <= 9:
        return int(text), None
    return None, "--weather %s: 0 to 9, or any" % text


def run_weather_line(folder):
    """The first line of the run's weather-of-the-run.txt (the runner's summary), or ""."""
    try:
        with open(os.path.join(folder, "weather-of-the-run.txt"), errors="replace") as f:
            return f.readline().strip()
    except OSError:
        return ""


def weather_held(line, pin):
    """Whether the runner's weather line shows the pin of the runs held: `weather: pinned N, held
    (...)`. With no pin (--weather any) any line does; the summary names it."""
    if pin is None:
        return True
    return line.startswith("weather: pinned %d, held" % pin)


def check_weather(folder):
    """The weather of a run in the summary; a run whose weather was not pinned and held fails,
    unless --weather any: a capture or a rate in the game's weather is no reference."""
    line = run_weather_line(folder)
    pin = WEATHER["pin"]
    text = (line or "no weather-of-the-run.txt") + ("" if pin is not None else " (--weather any)")
    return check(weather_held(line, pin), text)
# The start of a run folder's name.
RUN_TIME = "%Y%m%d-%H%M%S"


def game(label, extra, mge_build, env=None):
    """A game run. Returns its folder, or None when the runner failed or made no folder: the run of
    an earlier stage must not stand in for it. The runner refuses a session that does not hold the
    game slot."""
    args = [PY, os.path.join(HERE, "run_own_device.py"), "--mge-build", mge_build, "--no-build", "--label", label, "--set", "expanded_light_limit = true"] + RUNNER_ARGS + extra
    e = dict(GPU_TIME)
    e.update(env or {})
    started = time.strftime(RUN_TIME)
    if not ran(after_done(run_code(args, e), label), "the runner of " + label):
        return None
    folder = newest(label, started)
    if not folder:
        check(False, "the runner of %s made no folder since %s" % (label, started))
    return folder


def after_done(result, label):
    """The runner's `result` of `run_code`, with a crash after DONE (RUNNER_CRASHED_AFTER_DONE) as the code 0 and a line
    with the runner's ERROR line: the steps and their pictures are there."""
    code, output = result
    if code != RUNNER_CRASHED_AFTER_DONE:
        return result
    crash = next((line for line in output.splitlines() if line.startswith(CRASH_AFTER_DONE_LINE)), CRASH_AFTER_DONE_LINE)
    print("  the runner of %s: the steps were done; %s" % (label, crash), flush=True)
    return 0, output


def host_counts(folder):
    text = open(os.path.join(folder, "host.txt"), errors="replace").read()
    lines = {k: "" for k in ("geometry", "material", "shapes")}
    for line in text.splitlines():
        for key in lines:
            if line.startswith(key + ":"):
                lines[key] = line
    return lines


# The FAIL lines and the NOT RUN lines of this process: the exit code of the baseline stage reads them.
FAILS = [0]
NOT_RUN_LINES = []


def check(condition, text):
    print(("  ok   " if condition else "  FAIL ") + text, flush=True)
    if not condition:
        FAILS[0] += 1
    return condition


def not_run(text):
    """A thing that the gate could not judge: its line, and False. It is never a pass (W524: PASS, FAIL, NOT RUN,
    WITHHELD): a stage with such a line and no FAIL ends with NOT_RUN_EXIT."""
    print("  NOT RUN " + text, flush=True)
    NOT_RUN_LINES.append(text)
    return False


def warn(text):
    """A finding that does not fail the gate yet: its line, and True."""
    print("  warn " + text, flush=True)
    return True


# The CAPTURE FORMAT lines (common.py) of the replays of this process.
CAPTURE_FORMATS = []


def baseline_failure_code(stage="baseline"):
    """The exit code of a baseline stage (or the stage `order`) that did not pass: NOT RUN when it has no FAIL line and
    could not judge a thing (a capture of another format, which stops the stage at once; a check with no line in a
    replay's output; a check that no capture judges), with a "NOT RUN" line for merge_step.py; else 1. A FAIL comes
    first."""
    reasons = CAPTURE_FORMATS + NOT_RUN_LINES
    if FAILS[0] or not reasons:
        return 1
    print("NOT RUN %s: %s" % (stage, reasons[-1]), flush=True)
    return NOT_RUN_EXIT


def ran(result, tool, accepting=False):
    """True when a tool of this folder ended with the code 0. `result` comes from `run_code`. Else
    its last lines go to the console and the stage fails: a failed tool would only show later as
    pictures that are missing, or in accept mode as pictures accepted. An exception, a helper
    that died and a tour stop with no picture all end with another code. In accept mode the
    pictures made before the stop are in the accepted folder already. A replay that the slot
    check refused checked no picture: it is NOT RUN, not a failed picture. A replay that ended
    with the fault of the stage shaders (the helper's exit code 40, W523) made its pictures, and
    they are right; the stage fails with replay.py's line of the fault, which no picture shows."""
    code, output = result
    if code == 0:
        return True
    for line in output.strip().split("\n")[-6:]:
        print("       " + line[:200], flush=True)
    refused = refusal_of(output)
    # A failed picture first: validate_tour.py prints its count, and a NOT RUN must not hide it.
    failed = re.search(r"\d+ pictures, [1-9]\d* failed", output)
    if refused and not failed:
        return check(False, "%s: %s: %s" % (NOT_RUN, tool, refused[refused.index("Refused"):]))
    # A capture of another format than the build under test: the gate cannot judge this build with it.
    other_format = capture_format_in(output)
    if other_format and not failed:
        CAPTURE_FORMATS.append(other_format)
        print("  NOT RUN %s: %s" % (tool, other_format), flush=True)
        return False
    mix = ": the accepted folder holds a mix now, accept again" if accepting else ""
    # The tour tool could not judge a thing (a check line of the helper that is not there, no pair to compare): its
    # verdict line says what.
    if code == TOUR_NOT_RUN and tour_verdict_in(output) and not failed:
        return not_run("%s: %s%s" % (tool, tour_verdict_in(output), mix))
    fault = replay_fault_in(output)
    if fault:
        return check(False, "%s: %s%s" % (tool, fault, mix))
    return check(False, "%s ended with the code %d%s" % (tool, code, mix))


def plugin_report(folder):
    """The line of the shape records in the log of the plugin, or ""."""
    lines = [line for line in plugin_lines(folder) if "shapes: " in line and "draws without a record" in line]
    return lines[-1] if lines else ""


def check_counts(folder):
    """The counts of a run: each shape drew, each light had its record, each draw of the game
    had a record."""
    counts = host_counts(folder)
    host = open(os.path.join(folder, "host.txt"), errors="replace").read()
    ok = check(" 0 without a geometry or a material" in counts["shapes"], counts["shapes"][:160] or "no shapes line of the helper")
    ok &= check(" 0 draws of more index entries" in counts["geometry"], counts["geometry"][:160] or "no geometry line of the helper")
    # The line has more fields between the two since the light events (`0 destroyed, 71 known
    # now`, records entry 85); the pattern takes any.
    lights = re.search(r"lights: (\d+) records, .*?(\d+) lights of a shape without a record", host)
    ok &= check(lights is not None and lights.group(2) == "0", lights.group(0) if lights else "no lights line")
    report = plugin_report(folder)
    # Since the skin weights of NiSkinData (1f01765) the line also counts the skinned draws
    # with no palette and those skinned in software; each count must be 0.
    without = re.search(r"draws without a record: (\d+) with no geometry, (\d+) with no material(?:, (\d+) skinned with no palette, (\d+) skinned in software)?; (\d+) draws of an empty geometry|draws without a record: (\d+) with no geometry, (\d+) with no material;", report)
    ok &= check(without is not None and {g for g in without.groups() if g is not None} == {"0"}, report[-170:] or "no shapes line of the plugin")
    return ok


def stage_capture(place, mge_build):
    print("capture at %s" % place, flush=True)
    folder = game("v-capture-" + place, PLACES[place] + ["--steps", "exterior,capture"], mge_build)
    if not folder or not os.path.isfile(os.path.join(folder, "exterior.nircap")):
        return check(False, "no capture from the run")
    ok = check_weather(folder)
    ok &= check_counts(folder)
    capture = os.path.join(folder, "exterior.nircap")
    a = os.path.join(folder, "replay.png")
    code, out = run_code([PY, os.path.join(HERE, "replay.py"), capture, a, "--frame", "20"] + build_option(mge_build), {})
    # A replay with a fault of a check ends with a code of its own (41), so `ran` fails it with its line.
    ok &= ran((code, out), "replay.py")
    ok &= check(os.path.isfile(a), "replay of the capture: %s" % ("a picture" if os.path.isfile(a) else "no picture"))
    # A capture of a place has a start of a world for the extension; it need not have a view of its own.
    ok &= judge_checks("replay of the capture", out, may_judge_nothing=(OWN_VIEWS,))
    return ok


ADAPTER_FILE = "adapter.txt"
# The fixed word of the gate's adapter line, which release.py reads: SAME when the run's
# adapter is the one of the accepted pictures, OTHER when it is another one, UNKNOWN when the
# helper printed none or the accepted pictures name none.
ADAPTER_SAME, ADAPTER_OTHER, ADAPTER_UNKNOWN = "SAME", "OTHER", "UNKNOWN"
ADAPTER_LINE = re.compile(r"^\s*adapter: (%s|%s|%s): (.*?)\s*$" % (ADAPTER_SAME, ADAPTER_OTHER, ADAPTER_UNKNOWN), re.M)


def judge_checks(what, output, may_judge_nothing=()):
    """The checks of the helper in a replay's output (common.py, the one reader): a check with no line, or that
    judged nothing, is NOT RUN; a check with faults is a FAIL. Returns False for either. `may_judge_nothing` names
    the checks for which the caller judges the case of 0 things itself."""
    missing, failed = check_problems(output, may_judge_nothing=may_judge_nothing)
    ok = True
    for problem in missing:
        ok &= not_run("%s: %s" % (what, problem))
    for problem in failed:
        ok &= check(False, "%s: %s" % (what, problem))
    return ok


def adapter_of(output):
    """The adapter line of the helper's output ("adapter: <GPU> (<backend>, driver ...)"),
    without the prefix; None when the output has none."""
    match = re.search(r"^adapter: (.+?)\s*$", output, re.MULTILINE)
    return match.group(1) if match else None


def adapter_note(recorded, now):
    """What to say about the adapter of a compare: the fixed word (`ADAPTER_LINE`), the
    adapter of the run, and whether the accepted pictures were made on it."""
    if now is None:
        return "adapter: %s: not printed by the helper" % ADAPTER_UNKNOWN
    if recorded is None:
        return "adapter: %s: %s; the accepted pictures name none (accept again to record it)" % (ADAPTER_UNKNOWN, now)
    if recorded != now:
        return "adapter: %s: %s; the accepted pictures were made on %s, so a difference can be the GPU's" % (ADAPTER_OTHER, now, recorded)
    return "adapter: %s: %s, as the accepted pictures" % (ADAPTER_SAME, now)


def self_test():
    """The tests of the pure parts: no game, no replay."""
    global REPO, run_code, build_missing, newest
    output = "distant land: on\nadapter: NVIDIA GeForce RTX 5070 (Vulkan, driver NVIDIA 617.14)\nframe 1\n"
    assert adapter_of(output) == "NVIDIA GeForce RTX 5070 (Vulkan, driver NVIDIA 617.14)"
    assert adapter_of("no such line\n") is None
    same = "NVIDIA GeForce RTX 5070 (Vulkan, driver NVIDIA 617.14)"
    assert adapter_note(same, same) == "adapter: SAME: %s, as the accepted pictures" % same
    assert adapter_note(None, same).endswith("(accept again to record it)")
    assert "can be the GPU's" in adapter_note("AMD Radeon (Vulkan, driver 1)", same)
    assert adapter_note(same, None) == "adapter: UNKNOWN: not printed by the helper"
    # The word of each case, as the gate prints the line (indented).
    for recorded, now, word in ((same, same, "SAME"), ("AMD Radeon (Vulkan, driver 1)", same, "OTHER"), (None, same, "UNKNOWN"), (same, None, "UNKNOWN")):
        assert ADAPTER_LINE.findall("       " + adapter_note(recorded, now) + "\n")[0][0] == word
    # The objects stage (D288): a steady frame that made objects fails with its sites; a lost
    # device, a build without the count and two surface views in a steady frame fail too; a
    # capture with no steady frame warns.
    clean = "frame 119\ngpu objects: 840 made in 120 frames; 117 steady frames, 0 of them made objects (0 objects)\n"
    assert objects_verdict(clean) == ("ok", "117 steady frames of 120 made no object (840 made in the others)")
    dirty = ("gpu objects: 900 made in 120 frames; 117 steady frames, 96 of them made objects (96 objects)\n"
             "gpu objects of the steady frames: 96 bind group at host/src/renderer.rs:1155\n")
    verdict, text = objects_verdict(dirty)
    assert verdict == "sites" and text == "96 of 117 steady frames made 96 objects: 96 bind group at host/src/renderer.rs:1155", text
    # Every site, with no cut: more sites than the helper's warning line of a frame holds (12).
    many = ", ".join("1 texture view at host/src/site%02d.rs:%d" % (n, n) for n in range(20))
    verdict, text = objects_verdict("gpu objects: 120 made in 120 frames; 117 steady frames, 1 of them made objects (20 objects)\n"
                                    "gpu objects of the steady frames: " + many + "\n")
    assert verdict == "sites" and text.endswith(": " + many), text
    assert objects_verdict("gpu: the device is lost (Unknown): Out of memory\n" + clean) == ("fail", "the device is lost")
    assert objects_verdict("frame 119\n")[0] == "fail" and "without the count" in objects_verdict("frame 119\n")[1]
    restless = ("gpu objects: 5 made in 2 frames; 0 steady frames, 0 of them made objects (0 objects)\n"
                "gpu objects: the frames that were not steady, by their first change: new instance 2\n")
    assert objects_verdict(restless) == ("idle", "2 frames, none steady: the capture cannot show the rule; "
                                               "the frames that were not steady, by their first change: new instance 2")
    one_view = clean + "gpu objects allowed (D310): the surface view, 117 in the steady frames, at most 1 a frame\n"
    assert objects_verdict(one_view)[0] == "ok"
    two_views = clean + "gpu objects allowed (D310): the surface view, 118 in the steady frames, at most 2 a frame\n"
    assert objects_verdict(two_views) == ("fail", "a steady frame made 2 views of the window's surface; D310 allows 1")
    # The frames that an arrival of distant statics changed first are named, not checked (W340);
    # with none, the line stays as it was.
    walk = clean + ("gpu objects: the frames that were not steady, by their first change: new geometry 3, "
                    "an arrival of distant statics 9, texture 1\n")
    assert objects_verdict(walk) == ("ok", "117 steady frames of 120 made no object (840 made in the others; "
                                           "9 frames changed first by an arrival of distant statics, not checked)"), objects_verdict(walk)
    assert arrival_frames(restless) == 0
    # A tool that ends with another code than 0 fails the stage, and its last lines show: an
    # exception, or a helper that died and printed no traceback.
    assert ran((0, "pictures in a folder"), "tool")
    with contextlib.redirect_stdout(io.StringIO()) as printed:
        assert not ran((1, "Traceback (most recent call last):\n  a frame\nFileNotFoundError: progress.log"), "tool")
    assert "FileNotFoundError: progress.log" in printed.getvalue() and "FAIL tool ended with the code 1" in printed.getvalue()
    with contextlib.redirect_stdout(io.StringIO()) as printed:
        assert not ran((-1073741819, "a line of the helper"), "tool", True)
    assert "a line of the helper" in printed.getvalue() and "accept again" in printed.getvalue()
    # A replay that the slot check refused (a planted refusal) is NOT RUN, and no picture failed.
    with contextlib.redirect_stdout(io.StringIO()) as printed:
        assert not ran((1, "Refused by the slot check: replay.py: the build slot is held by rob (place 1); a replay needs the whole build side.\nTo pass anyway: ..."), "replay.py")
    assert "FAIL %s: replay.py: Refused by the slot check" % NOT_RUN in printed.getvalue() and "ended with the code" not in printed.getvalue(), printed.getvalue()
    # A tour with failed pictures and one refused capture failed: the NOT RUN does not hide it.
    with contextlib.redirect_stdout(io.StringIO()) as printed:
        assert not ran((1, "NOT RUN: slot refused: replay of tour-3.nircap: 54 pictures not made: Refused by the slot check: x\n"
                           "FAILED 1-01-a.png: gpu error\n150 pictures, 2 failed\n"), "validate_tour.py")
    assert "FAIL validate_tour.py ended with the code 1" in printed.getvalue() and "FAIL " + NOT_RUN not in printed.getvalue(), printed.getvalue()
    assert baseline_failure_code() == 1, "a failure with no capture of another format"
    # A capture of another stream format (narsis.nircap at the merge step of 79c752d): the stage is NOT RUN, and its line
    # names the capture, not the parts of the build.
    stale = "CAPTURE FORMAT: narsis.nircap was made by akulakhan.dll (nirender v0.3.6 87f7049), which writes the stream format 8; x"
    FAILS[0] = 0
    with contextlib.redirect_stdout(io.StringIO()) as printed:
        assert not ran((10, "The parts of the wgpu renderer of MGE XE do not match.\nmake the capture again with this build\n" + stale + "\n"), "replay.py")
        assert baseline_failure_code() == NOT_RUN_EXIT
    assert "NOT RUN replay.py: " + stale in printed.getvalue() and "FAIL" not in printed.getvalue(), printed.getvalue()
    assert ("\nNOT RUN baseline: " + stale) in printed.getvalue(), printed.getvalue()
    del CAPTURE_FORMATS[:]
    # W526: a thing that the stage could not judge is NOT RUN, with its line; a FAIL beside it comes first.
    FAILS[0] = 0
    with contextlib.redirect_stdout(io.StringIO()) as printed:
        assert not_run("a.nircap: the helper's output has no line of the check \"gpu errors\"") is False
        assert baseline_failure_code() == NOT_RUN_EXIT
        assert check(False, "a.nircap: a picture differs") is False and baseline_failure_code() == 1
    assert "  NOT RUN a.nircap: the helper's output has no line" in printed.getvalue() and "\nNOT RUN baseline: a.nircap: the helper's output" in printed.getvalue(), printed.getvalue()
    FAILS[0] = 0
    del NOT_RUN_LINES[:]
    # A build folder with the helper only (a cargo target folder) lacks the Data Files.
    with tempfile.TemporaryDirectory() as build:
        open(os.path.join(build, "numidium.exe"), "wb").close()
        assert build_missing(build) == list(BUILD_PARTS[1:])
        for part in BUILD_PARTS[1:]:
            os.makedirs(os.path.join(build, part))
        assert build_missing(build) == []
    # A game stage reads a run of its own. An old run of the label is there each time, and it must
    # not stand in: not after a refusal of the runner, and not when the runner made no folder.
    real_repo, real_run_code = REPO, run_code
    try:
        with tempfile.TemporaryDirectory() as repo:
            REPO = repo
            old = os.path.join(repo, "results", "20200101-000000-v-test")
            os.makedirs(old)
            run_code = lambda args, env=None: (1, "the game slot is held by another session")
            with contextlib.redirect_stdout(io.StringIO()) as printed:
                assert game("v-test", [], "build") is None
            assert "the game slot is held" in printed.getvalue() and "FAIL the runner of v-test ended with the code 1" in printed.getvalue()
            run_code = lambda args, env=None: (0, "")
            with contextlib.redirect_stdout(io.StringIO()) as printed:
                assert game("v-test", [], "build") is None
            assert "made no folder" in printed.getvalue()

            def a_new_run(args, env=None):
                os.makedirs(new)
                return 0, ""

            new = os.path.join(repo, "results", time.strftime(RUN_TIME) + "-v-test")
            run_code = a_new_run
            assert game("v-test", [], "build") == new
            assert newest("v-test") == new and newest("v-test", "99991231-000000") is None
            # The clusters stage takes only a capture of this validate run (W383). The null: an
            # older capture of the place alone is the newest of its label, and it is refused.
            older = os.path.join(repo, "results", "20200101-000000-v-capture-seyda")
            os.makedirs(older)
            open(os.path.join(older, "exterior.nircap"), "wb").close()
            assert newest("v-capture-seyda") == older
            run_code = lambda args, env=None: (0, "")
            with contextlib.redirect_stdout(io.StringIO()) as printed:
                assert not stage_clusters("seyda", "build", time.strftime(RUN_TIME))
            assert "FAIL no capture of this run at seyda: run the capture stage in this run" in printed.getvalue(), printed.getvalue()
    finally:
        REPO, run_code = real_repo, real_run_code
    # The weather: pinned clear by default, `any` for the game's; a run whose weather line does not
    # show the pin held fails, so a capture in the game's weather is not taken as a reference.
    assert weather_pin("0") == (0, None) and weather_pin("9") == (9, None) and weather_pin("any") == (None, None)
    assert weather_pin("10")[1] and weather_pin("clear")[1] and weather_pin("-1")[1]
    assert weather_held("weather: pinned 0, held (0 re-pins)", 0)
    assert not weather_held("weather: pinned 2, held (0 re-pins)", 0)
    assert not weather_held("weather: the game's: Ashstorm (6), Cloudy (1)", 0)
    assert not weather_held("", 0)
    assert weather_held("weather: the game's: Ashstorm (6)", None)
    # The planted fault: the runner's line when the mod did not read the pin back.
    assert not weather_held("weather: pinned 0, NOT applied (0 re-pins)", 0)
    with tempfile.TemporaryDirectory() as run:
        assert run_weather_line(run) == ""
        with open(os.path.join(run, "weather-of-the-run.txt"), "w") as f:
            f.write("weather: pinned 0, held (0 re-pins)\nweather start: current 6 \"Ashstorm\"\n")
        assert run_weather_line(run) == "weather: pinned 0, held (0 re-pins)"
        with contextlib.redirect_stdout(io.StringIO()) as printed:
            assert check_weather(run)
        assert "ok   weather: pinned 0, held" in printed.getvalue()
    # The folders of the baseline stage. With --now, it writes nothing in the baseline folder.
    base, other = os.path.join("x", "base"), os.path.join("x", "other")
    assert picture_folders(base, True, None) == (os.path.join(base, "accepted"), os.path.join(base, "tour", "accepted"))
    assert picture_folders(base, False, other) == (other, os.path.join(other, "tour"))
    # Two gates at once with no --now: two folders of their own, outside the baseline folder.
    when = time.localtime()
    results = os.path.join("x", "results")
    first, second = own_now(results, when, 4100), own_now(results, when, 4200)
    assert first != second and os.path.dirname(first) == results, (first, second)
    assert now_problem(os.path.join(results, "baseline-records"), False, first) is None, first
    assert now_problem(base, False, None) is None and now_problem(base, True, None) is None and now_problem(base, False, other) is None
    assert now_problem(base, False, os.path.join("x", "base2")) is None, "a name that only starts the same is outside"
    assert "--accept" in now_problem(base, True, other)
    assert "in the baseline folder" in now_problem(base, False, base)
    assert "in the baseline folder" in now_problem(base, False, os.path.join(base, "accepted"))
    # The stream rule on the kept captures: its own, the tour's and the instance A/B's.
    world, shadow = (0.9774, 1.7376), (1.3032, 2.3168)
    right = [[(1,) + world, (5,) + world, (7,) + shadow]]
    wrong = [[(1,) + world, (5,) + shadow, (7,) + shadow]]
    none = [[(1,) + world, (7,) + shadow]]

    def stream_rule_of(files):
        """The verdict (ok, printed) of the rule on a baseline folder made of `files`: a dict from a
        relative path to the frames of its planted capture."""
        with tempfile.TemporaryDirectory() as folder:
            for name, frames in files.items():
                path = os.path.join(folder, *name.split("/"))
                os.makedirs(os.path.dirname(path), exist_ok=True)
                with open(path, "wb") as file:
                    file.write(stream_rule.plant(frames))
            with contextlib.redirect_stdout(io.StringIO()) as printed:
                ok = check_stream_rule(folder)
        return ok, printed.getvalue()

    ok, printed = stream_rule_of({"a.nircap": right, "b.nircap": none})
    assert ok and "1 captures pass, 1 have no scene of kind 5" in printed, printed
    ok, printed = stream_rule_of({"a.nircap": right, "tour/t.nircap": wrong})
    assert not ok and "FAIL stream rule" in printed and "t.nircap" in printed, printed
    ok, printed = stream_rule_of({"a.nircap": right, "instance/grass/g.nircap": wrong})
    assert not ok and "g.nircap" in printed, printed
    ok, printed = stream_rule_of({"a.nircap": none, "tour/t.nircap": none})
    assert not ok and "FAIL %s: stream rule: no capture" % NOT_RUN in printed, printed
    assert not stream_rule_of({})[0]
    # The baseline stage runs the rule before any replay: a wrong projection in a kept capture ends it with no replay
    # started (a stage that left the rule out would start the replays), and a right one goes on to the first replay.
    calls = []
    real_build_missing, real_run_code = build_missing, run_code
    build_missing = lambda _build: []  # noqa: E731
    run_code = lambda command, env: calls.append((command, env)) or (1, "a planted failure of the replay")  # noqa: E731
    try:
        # Also when the stage accepts: an accept of pictures drawn from a wrong stream would keep the fault.
        for name, frames, accept in (("wrong", wrong, False), ("wrong", wrong, True), ("right", right, False), ("right", right, True)):
            with tempfile.TemporaryDirectory() as folder:
                with open(os.path.join(folder, "a.nircap"), "wb") as file:
                    file.write(stream_rule.plant(frames))
                calls.clear()
                with contextlib.redirect_stdout(io.StringIO()):
                    assert not stage_baseline(folder, accept, "no build", os.path.join(folder, "now"), hooks=None)
            assert len(calls) == (0 if name == "wrong" else 1), (name, accept, calls)
        assert "replay.py" in calls[0][0][1]
        # The replays of the gate run with the core mods off.
        assert calls[0][1].get("MGE_XE_CORE_MODS") == "0", calls[0][1]
        # W545: a set goes to replay.py as --mge-build; the archive of release.py's gate as --archive, never as
        # --mge-build, which replay.py would refuse for a folder with no manifest.
        assert calls[0][0][-2:] == ["--mge-build", "no build"], calls[0][0]
        assert gate_build(None, "the archive") == "the archive" and BUILD["option"] == "--archive"
        with tempfile.TemporaryDirectory() as folder:
            with open(os.path.join(folder, "a.nircap"), "wb") as file:
                file.write(stream_rule.plant(right))
            calls.clear()
            with contextlib.redirect_stdout(io.StringIO()):
                assert not stage_baseline(folder, False, "the archive", os.path.join(folder, "now"), hooks=None)
        assert calls[0][0][-2:] == ["--archive", "the archive"] and "--mge-build" not in calls[0][0], calls[0][0]
    finally:
        build_missing, run_code = real_build_missing, real_run_code
        BUILD["option"] = "--mge-build"
    # W545: the build of the stages. --archive only with the baseline stage and without --mge-build; the nulls: the
    # baseline stage with --archive, and any stage with a set.
    assert build_problem(["baseline"], None, "a") is None and build_problem(["capture", "baseline"], "s", None) is None
    assert build_problem(["baseline"], "s", "a") == "an archive is not a set: no --mge-build with --archive"
    assert build_problem(["baseline", "order"], None, "a") is None
    assert "the stages capture, only start the game" in build_problem(["capture", "baseline", "only"], None, "a")
    # Without --mge-build, the current set of the sets folder; with none, None. The null: --mge-build asks for no set.
    with contextlib.redirect_stdout(io.StringIO()):
        import clean_common
        assert gate_build(None, None, current=lambda: "s-x") == os.path.join(clean_common.SETS, "s-x")
        assert gate_build(None, None, current=lambda: None) is None
        assert gate_build("the set", None, current=lambda: 1 / 0) == "the set" and BUILD["option"] == "--mge-build"
    # W417: a crash after DONE counts as ran, with its line; the null: any other code stays as it was.
    import run_own_device
    assert RUNNER_CRASHED_AFTER_DONE == run_own_device.EXIT_CRASHED_AFTER_DONE
    assert run_own_device.crash_after_done_line(True, [0xC0000005], []).startswith(CRASH_AFTER_DONE_LINE[len("ERROR: "):])
    output = "pictures in x\nERROR: the game crashed after DONE: exit code 0xC0000005 at its close; dumps: none.\n"
    with contextlib.redirect_stdout(io.StringIO()) as said:
        assert after_done((RUNNER_CRASHED_AFTER_DONE, output), "a") == (0, output)
    assert said.getvalue() == "  the runner of a: the steps were done; " + output.splitlines()[1] + "\n", said.getvalue()
    for code in (0, 1, 3, 4):
        assert after_done((code, output), "a") == (code, output), code
    # W523: a replay that ended with the fault of the stage shaders fails with replay.py's line of it, in each
    # stage that replays; the null: the code 0 passes, and another code has the plain line.
    import common
    fault_output = ("frames 60\n" + common.STAGE_FAULT_START + "1 rewrites failed (stage key 1: the end mark of the stage loop: 0 in the text, 1 wanted); 0 pipelines failed; "
                    "the uber-shader drew 9 draws that have a stage key (9 after a failed rewrite, 0 after a failed pipeline, 0 with a pipeline not made); 0 draws past the cap of 256 keys\n")
    fault_line = common.stage_fault_line("seyda.nircap", EXIT_REPLAY_STAGE_SHADERS, fault_output)
    assert fault_line.startswith("STAGE SHADERS: the replay of seyda.nircap failed with exit code 40: 1 rewrites failed (the first: stage key 1: the end mark"), fault_line
    fault_output += fault_line + "\n"
    for accepting, end in ((False, ""), (True, ": the accepted folder holds a mix now, accept again")):
        with contextlib.redirect_stdout(io.StringIO()) as said:
            assert ran((EXIT_REPLAY_STAGE_SHADERS, fault_output), "replay.py", accepting) is False
        assert said.getvalue().endswith("  FAIL replay.py: " + fault_line + end + "\n"), said.getvalue()
    with contextlib.redirect_stdout(io.StringIO()) as said:
        assert ran((0, fault_output), "replay.py") is True and ran((20, "gpu error\n"), "replay.py") is False
    assert said.getvalue().endswith("  FAIL replay.py ended with the code 20\n") and "STAGE SHADERS" not in said.getvalue(), said.getvalue()
    # The tour tool's output with the line of a replay fails the same way.
    tour_output = "4 pictures, 0 failed\nFAILED stage shaders: 1 replays ended with the fault of the stage shaders:\n  " + fault_line + "\npairs: 2 of 2 equal\n"
    with contextlib.redirect_stdout(io.StringIO()) as said:
        assert ran((1, tour_output), "validate_tour.py") is False
    assert said.getvalue().endswith("  FAIL validate_tour.py: " + fault_line + "\n"), said.getvalue()
    assert replay_end(0, fault_output) == "" and replay_end(20, "gpu error") == " (replay.py ended with the code 20)"
    assert replay_end(EXIT_REPLAY_STAGE_SHADERS, fault_output) == " (replay.py ended with the code 40: " + fault_line + ")"
    # The clusters stage: a replay that ended with the fault fails the stage before any picture is compared.
    real_run_code, real_newest = run_code, newest
    try:
        with tempfile.TemporaryDirectory() as folder:
            open(os.path.join(folder, "exterior.nircap"), "wb").close()
            newest = lambda label, since=None: folder  # noqa: E731
            calls = []

            def a_replay(args, env=None):
                calls.append(env)
                return (EXIT_REPLAY_STAGE_SHADERS, fault_output) if len(calls) == fails_at[0] else (0, "clusters: 3.0 lights and 12 entries for each frame\n")

            run_code = a_replay
            for at, tool in ((1, "replay.py with the clusters"), (2, "replay.py with the lights of the draws")):
                fails_at = [at]
                calls.clear()
                with contextlib.redirect_stdout(io.StringIO()) as said:
                    assert stage_clusters("seyda", "build", "20261116-000000") is False
                assert len(calls) == at and said.getvalue().endswith("  FAIL %s: %s\n" % (tool, fault_line)), said.getvalue()
                kept = sorted(name for name in os.listdir(folder) if name.endswith(".replay.txt"))
                assert kept == ["replay-clusters.replay.txt", "replay-draw-lights.replay.txt"][:at], kept
            assert open(os.path.join(folder, "replay-draw-lights.replay.txt")).read() == "exit code 40\n" + fault_output
    finally:
        run_code, newest = real_run_code, real_newest
    # W526, F-G-19: the clusters stage's compare. Equal pictures pass. A wrong area of 3 % of the picture with a mean
    # of 0.2 passed before (the mean limit alone); one pixel over 8 levels fails now. The mean limit stays.
    equal = {"pixels": 2073600, "different": 0.0, "over8": 0.0, "mean": 0.0, "largest": 0}
    assert clusters_verdict(equal) == (True, "replay with the clusters against the lights of the draws: 0 pixels by more than 8 levels (0.0000 %; the limit is 0), "
                                             "the largest level 0, mean 0.000 (the limit is under 0.5), 0.00 % different")
    assert clusters_verdict(dict(equal, different=0.01, mean=0.01, largest=8))[0], "pixels that differ by 8 levels or less pass"
    for numbers, said in ((dict(equal, different=0.03, over8=0.03, mean=0.2, largest=40), "62208 pixels by more than 8 levels (3.0000 %; the limit is 0), the largest level 40"),
                          (dict(equal, different=0.000001, over8=1 / 2073600, mean=0.0001, largest=159), "1 pixels by more than 8 levels"),
                          (dict(equal, different=0.9, mean=0.6, largest=3), "mean 0.600 (the limit is under 0.5)")):
        passed, text = clusters_verdict(numbers)
        assert not passed and said in text and text.endswith("go to the Architect before a limit moves)"), (numbers, text)
    assert clusters_verdict({"problem": "the sizes differ"}) == (False, "replay with the clusters against the lights of the draws: the sizes differ")
    self_test_route()
    self_test_baseline()
    self_test_scratch_folders()
    self_test_tour_stage()
    self_test_order_stage()
    print("validate_retained self-test: ok")


def self_test_order_stage():
    """W535, verdict 595: the stage `order` on a planted baseline folder (two captures and one of the tour) with
    planted replays (no build, no replay, no GPU). Each plant must give its verdict: equal pictures with orders that
    differ in one capture pass; the planted null (one pixel of one capture's engine picture) fails by name; orders
    equal everywhere are NOT SHOWN; a missing route line, a route line of the other value and a missing digest are
    NOT RUN; a last frame in one value only, unequal counts and a replay that failed fail."""
    global run_code, build_missing
    import picture_diff

    real = run_code, build_missing
    grey = (10, 20, 30)
    digest = "0123456789abcdef"
    other_digest = "fedcba9876543210"

    def plant(change=None):
        """A run_code whose replay of <prefix>.nircap with MGE_XE_TABLE_ORDER=<value> writes frames 1 and 29 and prints
        the route line, a digest of each and the counts line of the report's end ("table order <value>: ...", which
        is not a route line). `change(name, value, frames, lines)` alters
        them, and may return (code, output) to replace the result."""

        def run(command, env=None):
            if not command[1].endswith("replay.py"):
                return 0, ""
            value = (env or {}).get(ORDER_SWITCH)
            name = os.path.basename(command[2])[:-len(".nircap")]
            frames = {1: grey, 29: grey}
            lines = ["table order: %s" % value] + ["table order digest: frame %d %s 120 draws" % (frame, digest) for frame in frames]
            lines.append("table order %s: 40 groups, 120 draws" % value)
            replaced = change(name, value, frames, lines) if change else None
            for frame, colour in frames.items():
                picture = picture_diff.Image.new("RGB", (8, 8), grey)
                if colour != grey:
                    picture.putpixel((3, 3), colour)
                picture.save("%s-%05d.png" % (command[3], frame))
            return replaced or (0, "".join(line + "\r\n" for line in lines))

        return run

    def differ_in(capture):
        """A change: the engine order's digests of `capture` differ from the grouped order's."""
        def change(name, value, _frames, lines):
            if name == capture and value == "engine":
                lines[:] = [line.replace(digest, other_digest) for line in lines]
        return change

    def stage(folder, tool):
        """(passed, printed, exit code of a stage that did not pass)."""
        global run_code
        run_code = tool
        FAILS[0] = 0
        del NOT_RUN_LINES[:]
        with contextlib.redirect_stdout(io.StringIO()) as printed:
            passed = stage_order(folder, "no build", os.path.join(folder, "now"))
            code = 0 if passed else baseline_failure_code("order")
        return passed, printed.getvalue(), code

    build_missing = lambda _build: []  # noqa: E731
    try:
        with tempfile.TemporaryDirectory() as folder:
            os.makedirs(os.path.join(folder, "tour"))
            for path in ("a.nircap", "b.nircap", os.path.join("tour", "t.nircap")):
                open(os.path.join(folder, path), "wb").close()
            assert [os.path.basename(c) for c in order_captures(folder)] == ["a.nircap", "b.nircap", "t.nircap"]
            # Equal pictures, the orders differ in the tour's capture: PASS, and the tour is in.
            passed, said, _ = stage(folder, plant(differ_in("t")))
            assert passed and "the orders differed in 1 of 3 captures: t.nircap" in said and "  ok   table order: 3 captures" in said, said
            assert os.path.isfile(os.path.join(folder, "now", "order", "tour-t-engine-last.png")), os.listdir(os.path.join(folder, "now", "order"))
            assert "tour-t, engine: frame 1 %s, frame 29 %s" % (other_digest, other_digest) in said, said

            # The planted null: one pixel of b's engine picture of frame 29 differs. The stage fails and names it.
            def one_pixel(name, value, frames, lines):
                differ_in("t")(name, value, frames, lines)
                if name == "b" and value == "engine":
                    frames[29] = (200, 0, 0)
            passed, said, code = stage(folder, plant(one_pixel))
            assert not passed and code == 1 and "  FAIL b frame 29: grouped against engine:" in said and "1 of 3 captures failed" in said, said

            # The orders are the same everywhere: the pixels cannot show the order.
            passed, said, code = stage(folder, plant())
            assert not passed and code == 1 and "  NOT SHOWN table order:" in said and "the orders differed in 0 of 3" in said, said

            # No route line, the route line of the other value, two route lines: NOT RUN (exit code NOT_RUN_EXIT).
            for routes in ([], ["table order: grouped", "table order: grouped"]):
                def route(name, value, frames, lines, routes=routes):
                    differ_in("t")(name, value, frames, lines)
                    if name == "a" and value == "engine":
                        lines[:] = [line for line in lines if not line.startswith("table order: ")] + routes
                passed, said, code = stage(folder, plant(route))
                assert not passed and code == NOT_RUN_EXIT and "  NOT RUN a: the replay with MGE_XE_TABLE_ORDER=engine printed the route lines" in said, (routes, said)
            passed, said, code = stage(folder, plant(lambda name, value, frames, lines: lines.__setitem__(0, "table order: grouped") if value == "engine" else None))
            assert not passed and code == NOT_RUN_EXIT and "printed the route lines ['grouped']" in said, said

            # A compared frame with no digest line: NOT RUN.
            def no_digest(name, value, frames, lines):
                differ_in("t")(name, value, frames, lines)
                if name == "a" and value == "grouped":
                    lines[:] = [line for line in lines if "frame 29 " not in line]
            passed, said, code = stage(folder, plant(no_digest))
            assert not passed and code == NOT_RUN_EXIT and "NOT RUN a frame 29: no \"table order digest: frame 29\" line in the replay of grouped" in said, said

            # A last frame in one value only, and unequal counts: FAIL.
            def one_frame(name, value, frames, lines):
                differ_in("t")(name, value, frames, lines)
                if name == "b" and value == "engine":
                    del frames[29]
            passed, said, code = stage(folder, plant(one_frame))
            assert not passed and code == 1 and "b: the pictured frames differ between the values" in said, said

            def counts(name, value, frames, lines):
                differ_in("t")(name, value, frames, lines)
                if name == "a" and value == "engine":
                    lines[:] = [line.replace(" 120 draws", " 119 draws") for line in lines]
            passed, said, code = stage(folder, plant(counts))
            assert not passed and code == 1 and "a frame 1: 120 draws with grouped and 119 with engine" in said, said

            # A replay that failed fails the stage.
            passed, said, code = stage(folder, plant(lambda name, value, frames, lines: (20, "gpu error\r\n") if name == "a" else None))
            assert not passed and code == 1 and "FAIL" in said, said

        # No capture: a FAIL with its line.
        with tempfile.TemporaryDirectory() as empty:
            passed, said, _ = stage(empty, plant())
            assert not passed and "no capture in the baseline folder or its tour" in said, said
    finally:
        run_code, build_missing = real
        FAILS[0] = 0
        del NOT_RUN_LINES[:]


def self_test_scratch_folders():
    """Verdicts 504 and 512: the temporary folders of the objects stage and of the second replays. A clean stage
    removes its folder (before, one stayed in %TEMP% at each gate run). A replay that fails, or whose run folder
    replay.py keeps, keeps the folder with its dump, named in a line. A folder that cannot be removed is named, and
    the stage's result stands."""
    global run_code
    real, fails, real_temp = run_code, FAILS[0], tempfile.tempdir
    replays = []

    def planted(code=0, kept=False):
        """A run_code whose replay writes its picture and, when it fails or `kept`, a run folder with a dump, as
        replay.py keeps one."""
        def run(command, env=None):
            if not command[1].endswith("replay.py"):
                return 0, ""
            picture = command[3]
            replays.append(picture)
            open(picture if picture.endswith(".png") else picture + "-00001.png", "wb").close()
            if code == 0 and not kept:
                return 0, ""
            run_folder = (picture[:-4] if picture.endswith(".png") else picture) + ".run"
            os.makedirs(run_folder)
            open(os.path.join(run_folder, "numidium.dmp"), "wb").close()
            return code, "replay: the run folder %s is kept: numidium.dmp\n" % run_folder
        return run

    def objects(tool, baseline):
        """(the stage's temporary folder, what the stage printed) of the objects stage with the replays of `tool`."""
        global run_code
        run_code = tool
        del replays[:]
        with contextlib.redirect_stdout(io.StringIO()) as printed:
            stage_objects(baseline, "no build")
        folders = {os.path.dirname(picture) for picture in replays}
        assert len(replays) == len(OBJECT_BACKENDS) and len(folders) == 1, replays
        return folders.pop(), printed.getvalue()

    try:
        with tempfile.TemporaryDirectory() as root:
            # Every temporary folder of this test, a kept one too, is made inside `root`, which is removed at the end.
            tempfile.tempdir = root
            baseline = os.path.join(root, "baseline")
            os.makedirs(baseline)
            open(os.path.join(baseline, "a.nircap"), "wb").close()

            folder, said = objects(planted(), baseline)
            assert os.path.basename(folder).startswith("gate-objects-") and not os.path.exists(folder), folder
            assert " is kept: " not in said and " is left: " not in said, said

            folder, said = objects(planted(code=41), baseline)
            first = OBJECT_BACKENDS[0]
            assert os.path.isfile(os.path.join(folder, "a-%s.run" % first, "numidium.dmp")), os.listdir(folder)
            assert "the folder %s is kept: %s, a.nircap: replay.py ended with the code 41" % (folder, first) in said, said

            # A clean end with a run folder that replay.py keeps (a file that the helper left) keeps the folder too.
            folder, said = objects(planted(kept=True), baseline)
            assert os.path.isdir(folder) and "%s, a.nircap: replay.py kept its run folder" % first in said, said

            # The second replay of an accept, in the same way.
            run_code = planted(code=41)
            with contextlib.redirect_stdout(io.StringIO()) as printed:
                problems = steady_problems(os.path.join(baseline, "a.nircap"), "a", {"": "unused"}, "no build", GATE_ENV)
            folder = os.path.dirname(replays[-1])
            assert problems == ["the second replay ended with the code 41"], problems
            assert os.path.basename(folder).startswith("gate-steady-") and os.path.isfile(os.path.join(folder, "a.run", "numidium.dmp")), folder
            assert "the folder %s is kept: a: replay.py ended with the code 41" % folder in printed.getvalue(), printed.getvalue()

            # A file in use at the end: the folder is left and named, and no exception reaches the stage.
            with contextlib.redirect_stdout(io.StringIO()) as printed:
                with scratch_folder("gate-objects-") as (folder, _keep):
                    held = open(os.path.join(folder, "held.bin"), "wb")
            held.close()
            assert os.path.isdir(folder) and "the folder %s is left: it could not be removed" % folder in printed.getvalue(), printed.getvalue()
    finally:
        run_code, FAILS[0], tempfile.tempdir = real, fails, real_temp


def self_test_tour_stage():
    """W526, F-G-3: the stage `tour` takes the tour tool's verdict by its exit code. Rob's plant: an output with
    "pairs: 0 of 20 equal" passed the stage before; now the tool ends with 1 for it, and the stage fails."""
    global run_code, game, check_counts
    real = run_code, game, check_counts
    table = "20 stops to replay, gpu and off at each\n40 pictures, 0 failed\npairs: %d of 20 equal\n"
    try:
        with tempfile.TemporaryDirectory() as folder:
            for cell in (1, 2, 3):
                open(os.path.join(folder, "tour-%d.nircap" % cell), "wb").close()
            game, check_counts = (lambda *_args, **_kind: folder), (lambda _folder: True)
            for code, output, passed, said in (
                    (0, table % 20 + "tour verdict: PASS\n", True, "  ok   tour verdict: PASS"),
                    (0, table % 3 + "tour verdict: PASS\n", True, "  ok   tour verdict: PASS"),
                    (1, table % 0 + "tour verdict: FAIL: 20 pairs of mode gpu against mode off differ by more than the limit 0.050, the largest 3.000 at 1-01-a.png\n", False, "  FAIL validate_tour.py ended with the code 1"),
                    (TOUR_NOT_RUN, table % 20 + "tour verdict: NOT RUN: replay of tour-1.nircap: the helper's output has no line of the checks: gpu errors\n", False,
                     "  NOT RUN validate_tour.py: tour verdict: NOT RUN: replay of tour-1.nircap: the helper's output has no line of the checks: gpu errors"),
                    # A tool that ends with 0 and no verdict line (a tool from before W526, or a broken one) is no pass.
                    (0, table % 0, False, "  FAIL validate_tour.py printed no verdict")):
                run_code = lambda command, env=None, result=(code, output): result  # noqa: E731
                with contextlib.redirect_stdout(io.StringIO()) as printed:
                    assert stage_tour("no build") is passed, (code, printed.getvalue())
                assert printed.getvalue().rstrip("\n").endswith(said), (said, printed.getvalue())
    finally:
        run_code, game, check_counts = real
        FAILS[0] = 0
        del NOT_RUN_LINES[:]


def route_plant(root):
    """A planted folder of hook fixtures in `root` (two files of code with a helper of their own, a file of values and
    a README), and the lines of a replay that shows their route: (folder, lines)."""
    folder = os.path.join(root, "fixtures")
    os.makedirs(folder)
    texts = {"XE Mod Shadow.wgsl": "//! interface: mge-xe-core 1\nfn shadow_delta(s0: vec3f, s1: vec3f) -> f32 {\n    return mod_far();\n}\n"
                                   "fn shadow_esm(dz: f32) -> f32 {\n    return 0.5;\n}\nfn mod_far() -> f32 {\n    return 1e-6;\n}\n",
             "XE Mod Shadow Data.wgsl": "//! interface: mge-xe-core 1\n//! values: shade = 0.6\n",
             "XE Mod Water.wgsl": "//! interface: mge-xe-core 1\nfn water_normal(texcoords: vec4f, distance: f32, world_xy: vec2f) -> vec3f {\n    return vec3f(0.0, 0.0, 1.0);\n}\n",
             "README.md": "fn not_a_hook() {}\n"}
    for name, text in texts.items():
        with open(os.path.join(folder, name), "w", encoding="utf-8", newline="\n") as file:
            file.write(text)
    read = ["core mods: read " + os.path.join(folder, name) for name in ("XE Mod Shadow.wgsl", "XE Mod Shadow Data.wgsl", "XE Mod Water.wgsl")]
    lines = ["core mods: only the folder %s (MGE_XE_CORE_MODS_DIR)" % folder] + read + read[:2] + [
        "core mod: XE Mod Water.wgsl: the plane of the water takes its hooks; the water volumes and the dry spaces keep the built-in water",
        "core mods: XE Mod Shadow.wgsl (shadow_delta, shadow_esm), XE Mod Shadow Data.wgsl (shade = 0.6), XE Mod Water.wgsl (water_normal) (interface mge-xe-core 1)"]
    return folder, lines


def self_test_route():
    """W528 item 5: the route check of the hook fixtures on planted outputs. The right output passes, with Windows line
    ends and the files read two times; each plant fails with its own text (verdict 480, points 1, 3 and 4)."""
    assert GATE_ENV == {"MGE_XE_CORE_MODS": "0"}, "the plain pass keeps the core mods off"
    with tempfile.TemporaryDirectory() as root:
        folder, good = route_plant(root)
        fixtures = hook_fixtures(folder)
        assert fixtures == {"XE Mod Shadow.wgsl": {"shadow_delta", "shadow_esm"}, "XE Mod Shadow Data.wgsl": set(), "XE Mod Water.wgsl": {"water_normal"}}, fixtures
        assert hook_fixtures(os.path.join(root, "missing")) == {}
        env = hooks_env(folder, os.path.join(root, "markers"))
        assert env == {"MGE_XE_CORE_MODS_DIR": folder, "MGE_XE_CORE_MOD_MARKERS": os.path.join(root, "markers")}, env
        assert route_problems("".join(line + "\r\n" for line in good), folder, fixtures) == []
        used = good[-1]
        water_read = "core mods: read " + os.path.join(folder, "XE Mod Water.wgsl")
        plants = {
            "no folder line": ([line for line in good if "only the folder" not in line], "does not read the core mods from the folder"),
            "another folder": ([line.replace(folder, folder + "2") if "only the folder" in line else line for line in good], "does not read the core mods from the folder"),
            "a missing read": ([line for line in good if line != water_read], "XE Mod Water.wgsl is not read"),
            "the README read": (good + ["core mods: read " + os.path.join(folder, "README.md")], "a file that is not a fixture is read"),
            "a dropped file": ([line for line in good if line != used] + [used.replace(", XE Mod Water.wgsl (water_normal)", ""),
                                "core mod: XE Mod Water.wgsl: no hook of interface 1; the built-in water is used"], "XE Mod Water.wgsl is not in use"),
            "a used line short of a hook": ([line for line in good if line != used] + [used.replace("shadow_delta, shadow_esm", "shadow_delta")],
                                            "XE Mod Shadow.wgsl is in use with the hooks shadow_delta, not shadow_delta, shadow_esm"),
            "none used": ([line for line in good if line != used] + ["core mods: none used (Data Files/shaders/core-mods, interface mge-xe-core 1)"], "XE Mod Shadow.wgsl is not in use"),
            "a device failure": (good + ["core mod: XE Mod Shadow.wgsl: the world shader failed with it on the device (x); the built-in hooks are used"], "a line of the core mods that a fixture in use does not print"),
            "the built-in text too": (good + ["core mods: the world shader's pipeline failed with the built-in text too (x); the key stays"], "a line of the core mods that a fixture in use does not print"),
            "a marker": (good + ["core mod: XE Mod Water.wgsl: a start before ended in the build of the water plane with it; it is not used until it changes"], "a line of the core mods that a fixture in use does not print"),
            "the core mods off": (["core mods: off (MGE_XE_CORE_MODS=0)"], "does not read the core mods from the folder"),
        }
        for plant, (lines, said) in plants.items():
            problems = route_problems("\n".join(lines) + "\n", folder, fixtures)
            assert any(said in problem for problem in problems), (plant, problems)
        # The dropped file also fails by its line, and a fault line also by itself when all else is right.
        assert any(problem.startswith("a line of the core mods") for problem in route_problems("\n".join(plants["a dropped file"][0]), folder, fixtures))
        with contextlib.redirect_stdout(io.StringIO()) as said:
            assert check_route("a.nircap", "\n".join(good), folder, fixtures)
            assert not check_route("a.nircap", "\n".join(plants["a marker"][0]), folder, fixtures)
        assert "ok   a.nircap: the route of the hook fixtures: 3 files read and in use" in said.getvalue(), said.getvalue()
        FAILS[0] = 0
    # replay.py --core-mods: the route of W525's replays with a core-mod file and with none.
    import replay
    assert replay.core_mods_env(None) == {}
    assert replay.core_mods_env("off") == {"MGE_XE_CORE_MODS": "0"}
    assert replay.core_mods_env(HOOK_FIXTURES) == {"MGE_XE_CORE_MODS_DIR": HOOK_FIXTURES}


def self_test_baseline():
    """W526: the baseline stage on planted baseline folders with planted replays (no build, no replay, no GPU). Each
    plant of the audit must fail the stage: a check line that is not there (F-G-1), a capture with no accepted picture
    (F-G-2), a fault that shows only after frame 1 (F-G-13); and the nulls pass."""
    global run_code, build_missing, check_stream_rule, stage_objects
    import common
    import picture_diff

    real = run_code, build_missing, check_stream_rule, stage_objects
    grey, other = (10, 20, 30), (250, 0, 250)
    judged = {common.EARLY_WORLD_DRAWS: 12, common.OWN_VIEWS: 97, common.READS_PAST_END: 5000, common.UNKNOWN_RECORDS: 5000, common.GPU_ERRORS: 30, common.STAGE_SHADERS_CHECK: 13}

    def report(counts=None, faults=None, without=()):
        counts, faults = dict(judged, **(counts or {})), faults or {}
        return "adapter: plant\n" + "".join("check %s: %d judged, %d faults\n" % (name, counts[name], faults.get(name, 0)) for name in common.CHECK_NAMES if name not in without)

    def plant(pictures=None, reports=None, code=0):
        """A run_code whose replay of <name>.nircap writes the frames of `pictures[name]` ({frame: colour}; the key
        "*" for the other captures) and prints `reports[name]`."""
        pictures, reports = dict({"*": {1: grey, 29: grey}}, **(pictures or {})), dict({"*": report()}, **(reports or {}))

        def run(command, env=None):
            if not command[1].endswith("replay.py"):
                return 0, ""
            prefixes.append(command[3])
            name = os.path.basename(command[3])
            frames = pictures.get(name, pictures["*"])
            for frame, colour in (frames() if callable(frames) else frames).items():
                picture_diff.Image.new("RGB", (8, 8), colour).save("%s-%05d.png" % (command[3], frame))
            return code, reports.get(name, reports["*"])

        return run

    def stage(folder, tool, accept=False, add=False, hooks=None):
        """(passed, printed, exit code of a stage that did not pass) of the stage on `folder` with the plant `tool`."""
        global run_code
        run_code = tool
        FAILS[0] = 0
        del NOT_RUN_LINES[:]
        with contextlib.redirect_stdout(io.StringIO()) as printed:
            passed = stage_baseline(folder, accept, "no build", None if accept else os.path.join(folder, "now"), add, hooks)
            code = 0 if passed else baseline_failure_code()
        return passed, printed.getvalue(), code

    objects_calls, prefixes = [], []

    def objects(_baseline, _build, keep=None, env=None, route=None):
        objects_calls.append((env, route))
        return True

    build_missing, check_stream_rule, stage_objects = (lambda _build: []), (lambda _folder: True), objects
    try:
        with tempfile.TemporaryDirectory() as folder:
            kept = os.path.join(folder, "accepted")
            for name in ("a", "b"):
                open(os.path.join(folder, name + ".nircap"), "wb").close()
            # The accept writes frame 1, the last frame and the judged counts of each capture; the same replays pass.
            passed, said, _code = stage(folder, plant(), accept=True)
            assert passed and "2 captures accepted (4 pictures)" in said, said
            assert sorted(f for f in os.listdir(kept) if not f.endswith(".replay.txt")) == ["a-last.png", "a.checks.txt", "a.png", "adapter.txt", "b-last.png", "b.checks.txt", "b.png"], os.listdir(kept)
            assert read_judged(os.path.join(kept, "a.checks.txt")) == judged
            passed, said, _code = stage(folder, plant())
            assert passed and "ok   a.nircap: frame 1 and frame 29 (the last)" in said and "ok   4 of 4 pictures equal to the accepted ones in" in said, said
            assert "2 captures: frame 1 of each, and the last frame of 2" in said, said
            # F-G-13: a fault that shows only in the last frame fails; before W526 only frame 1 was compared.
            passed, said, code = stage(folder, plant({"a": {1: grey, 29: other}}))
            assert not passed and code == 1 and "a-last.png" in said and "FAIL 3 of 4 pictures equal" in said, said
            # F-G-1 (Rob's plant): a check with no line in one replay's output is NOT RUN, never a pass.
            for lost in (common.EARLY_WORLD_DRAWS, common.OWN_VIEWS, common.GPU_ERRORS):
                passed, said, code = stage(folder, plant(reports={"a": report(without=(lost,))}))
                assert not passed and code == NOT_RUN_EXIT and '  NOT RUN a.nircap: the helper\'s output has no line of the check "%s"' % lost in said, (lost, said)
                assert "NOT RUN baseline: a.nircap: the helper's output has no line" in said, said
            # A check that judged nothing (no frame drawn, no record read, no stage key) is NOT RUN: no fault in nothing
            # is no pass. The two checks of the floor are judged by the accepted counts instead (the next case).
            passed, said, code = stage(folder, plant(reports={"a": report({common.GPU_ERRORS: 0})}))
            assert not passed and '  NOT RUN a.nircap: the check "gpu errors" judged nothing' in said, said
            # A check that judged at the accept and judges nothing now fails its capture, though another capture judges it.
            passed, said, code = stage(folder, plant(reports={"a": report({common.OWN_VIEWS: 0})}))
            assert not passed and code == 1 and 'FAIL a.nircap: the check "views of their own" judged 97 things at the accept and judges 0 now' in said, said
            # The floor: no capture of the stage judges the check. The accepted counts say the same here, and they fail first.
            passed, said, code = stage(folder, plant(reports={"*": report({common.EARLY_WORLD_DRAWS: 0})}))
            assert not passed and 'NOT RUN no capture of the baseline judges the check "early world draws"' in said and code == 1, said
            # A check with faults: the helper ends the replay with its code, and replay.py's line names it.
            fault = report(faults={common.GPU_ERRORS: 2}) + "CHECKS: the replay of a.nircap failed with exit code 41: gpu errors: 2 faults of 30 judged\n"
            passed, said, code = stage(folder, plant(reports={"*": fault}, code=EXIT_REPLAY_CHECKS))
            assert not passed and code == 1 and "FAIL replay.py: CHECKS: the replay of a.nircap failed with exit code 41: gpu errors: 2 faults of 30 judged" in said, said
            # Faults with the exit code 0 (a helper that did not end with its code) still fail.
            passed, said, code = stage(folder, plant(reports={"a": report(faults={common.EARLY_WORLD_DRAWS: 5})}))
            assert not passed and code == 1 and 'FAIL a.nircap: the check "early world draws": 5 faults of 12 judged' in said, said
            # F-G-2 (Rob's plant): a capture with no accepted state fails by name; before W526 the stage said "ok".
            open(os.path.join(folder, "new.nircap"), "wb").close()
            passed, said, code = stage(folder, plant({"new": {1: other, 29: other}}))
            assert not passed and code == 1 and "FAIL new.nircap has no accepted picture of frame 1" in said and "FAIL new.nircap has no accepted picture of its last frame" in said, said
            assert "FAIL new.nircap has no accepted counts of its checks (new.checks.txt)" in said and "FAIL 4 of 6 pictures equal" in said, said
            # --accept-new adds only what a capture lacks, when two replays give its pictures again; nothing is replaced.
            turns = iter([{1: other, 29: other}, {1: other, 29: grey}])
            passed, said, code = stage(folder, plant({"new": lambda: next(turns)}), add=True)
            assert not passed and "FAIL new.nircap: the last frame is not steady: two replays differ" in said and not os.path.isfile(os.path.join(kept, "new.png")), said
            passed, said, code = stage(folder, plant({"new": {1: other, 29: other}, "a": {1: other, 29: grey}}), add=True)
            assert not passed and "accepted new: new.png, new-last.png, new.checks.txt" in said and "a.png" in said, said
            assert picture_diff.Image.open(os.path.join(kept, "a.png")).getpixel((0, 0)) == grey, "an accepted picture was replaced"
            # The second replays of the accept go to a temporary folder each, removed after the judgement (verdict 504).
            steady = {os.path.dirname(prefix) for prefix in prefixes if os.path.basename(os.path.dirname(prefix)).startswith("gate-steady-")}
            assert steady and not any(os.path.exists(folder) for folder in steady), steady
            passed, said, code = stage(folder, plant({"new": {1: other, 29: other}}))
            assert passed and "ok   6 of 6 pictures equal" in said and "3 captures: frame 1 of each, and the last frame of 3" in said, said
            passed, said, code = stage(folder, plant({"new": {1: other, 29: other}}), add=True)
            assert passed and "accepted new: nothing: each capture has its accepted state" in said, said
            # The other way: an accepted picture with no capture, and one of a later frame that the capture no longer has.
            picture_diff.Image.new("RGB", (8, 8), grey).save(os.path.join(kept, "gone.png"))
            passed, said, code = stage(folder, plant({"new": {1: other, 29: other}, "b": {1: grey}}))
            assert not passed and "FAIL the accepted picture gone.png has no capture" in said and "FAIL the accepted picture b-last.png has no capture" in said, said
            assert "ok   b.nircap: frame 1 (the capture has no later frame)" in said, said
        # W528 item 5: the pass of the hook fixtures, after the plain pass, with its own variables and accepted state.
        with tempfile.TemporaryDirectory() as folder:
            fixtures, good = route_plant(folder)
            envs, markers_there = [], []

            def with_route(tool, lines=good):
                def run(command, env=None):
                    code, output = tool(command, env)
                    if command[1].endswith("replay.py"):
                        envs.append(env)
                        if "MGE_XE_CORE_MOD_MARKERS" in env:
                            markers_there.append(os.path.isdir(env["MGE_XE_CORE_MOD_MARKERS"]))
                    return code, output + ("".join(line + "\r\n" for line in lines) if "MGE_XE_CORE_MODS_DIR" in (env or {}) else "")
                return run

            open(os.path.join(folder, "a.nircap"), "wb").close()
            passed, said, _code = stage(folder, with_route(plant()), accept=True, hooks=fixtures)
            assert passed and "ok   hook fixtures: 1 captures accepted (2 pictures) in " in said, said
            assert "ok   hook fixtures: a.nircap: the route of the hook fixtures: 3 files read and in use" in said, said
            assert sorted(f for f in os.listdir(os.path.join(folder, HOOKS_ACCEPTED)) if not f.endswith(".replay.txt")) == ["a-last.png", "a.checks.txt", "a.png"]
            plain, hooked = [env for env in envs if "MGE_XE_CORE_MODS_DIR" not in env], [env for env in envs if "MGE_XE_CORE_MODS_DIR" in env]
            assert plain and all(env == GATE_ENV for env in plain), plain
            assert hooked and all(env["MGE_XE_CORE_MODS_DIR"] == fixtures and "MGE_XE_CORE_MODS" not in env for env in hooked), hooked
            # The markers folder is there while the pass replays, and gone when the pass ends (verdict 504).
            assert markers_there and all(markers_there), markers_there
            assert not any(os.path.exists(env["MGE_XE_CORE_MOD_MARKERS"]) for env in hooked), [env["MGE_XE_CORE_MOD_MARKERS"] for env in hooked]
            del objects_calls[:]
            passed, said, _code = stage(folder, with_route(plant()), hooks=fixtures)
            assert passed and "ok   2 of 2 pictures equal" in said and "ok   hook fixtures: 2 of 2 pictures equal" in said, said
            # The objects stage runs in both passes, the second with the variables and the route of the fixtures (verdict 480).
            assert [route for _env, route in objects_calls] == [None, (fixtures, hook_fixtures(fixtures))], objects_calls
            assert objects_calls[1][0]["MGE_XE_CORE_MODS_DIR"] == fixtures, objects_calls
            # A hook path that draws another picture fails the second pass only.
            turns = iter([{1: grey, 29: grey}, {1: other, 29: grey}])
            passed, said, code = stage(folder, with_route(plant({"a": lambda: next(turns)})), hooks=fixtures)
            assert not passed and code == 1 and "ok   2 of 2 pictures equal" in said and "FAIL hook fixtures: 1 of 2 pictures equal" in said, said
            # A replay that does not show the route fails, whatever its pictures.
            passed, said, code = stage(folder, with_route(plant(), [line for line in good if not line.endswith("XE Mod Water.wgsl")]), hooks=fixtures)
            assert not passed and code == 1 and "FAIL hook fixtures: a.nircap: the route of the hook fixtures: XE Mod Water.wgsl is not read" in said, said
            # A folder with no fixture fails by name: the pass is never left out without a line.
            empty = os.path.join(folder, "empty")
            os.makedirs(empty)
            passed, said, code = stage(folder, with_route(plant()), hooks=empty)
            assert not passed and "FAIL hook fixtures: %s has no XE Mod *.wgsl file" % empty in said, said
            # W567 part D: a gate list that names the hook pass's picture passes that change, and only it.
            list_file = os.path.join(folder, "gate-list.txt")
            with open(list_file, "w", encoding="utf-8") as file:
                file.write("hooks/a.png  # the hook path's new look\n")
            GATE_LIST.update(read_gate_list(list_file)[0])
            GATE_LIST_NOTE.append((list_file, read_gate_list(list_file)[1]))
            turns = iter([{1: grey, 29: grey}, {1: other, 29: grey}])
            passed, said, code = stage(folder, with_route(plant({"a": lambda: next(turns)})), hooks=fixtures)
            assert passed and "listed: a planned change" in said and "ok   the gate list %s (md5 " % list_file in said and "1 of 1 listed pictures changed" in said, said
            GATE_LIST.clear()
            del GATE_LIST_NOTE[:]
        self_test_gate_list(stage, plant, grey, other, prefixes)
    finally:
        run_code, build_missing, check_stream_rule, stage_objects = real
        FAILS[0] = 0
        del NOT_RUN_LINES[:]
        GATE_LIST.clear()
        del GATE_LIST_NOTE[:], CHANGED[:]


def self_test_gate_list(stage, plant, grey, other, prefixes):
    """W567 part D on a planted baseline folder (the plants of self_test_baseline): the gate list's rule, its refusals,
    and the accept after a pass, after which the next gate with no list is equal."""
    global run_code
    import picture_diff

    def with_list(folder, *names):
        path = os.path.join(folder, "gate-list.txt")
        with open(path, "w", encoding="utf-8") as file:
            file.write("".join(name + "\n" for name in names))
        GATE_LIST.clear()
        del GATE_LIST_NOTE[:]
        found, md5 = read_gate_list(path)
        GATE_LIST.update(found)
        GATE_LIST_NOTE.append((path, md5))
        return path, md5

    def accept_list(folder, tool):
        global run_code
        run_code = tool
        FAILS[0] = 0
        with contextlib.redirect_stdout(io.StringIO()) as printed:
            done = stage_accept_list(folder, "no build", os.path.join(folder, "now"), None)
        return done, printed.getvalue()

    def colour(path):
        return picture_diff.Image.open(path).getpixel((0, 0))

    with tempfile.TemporaryDirectory() as folder:
        kept = os.path.join(folder, "accepted")
        for name in ("a", "b"):
            open(os.path.join(folder, name + ".nircap"), "wb").close()
        assert stage(folder, plant(), accept=True)[0]
        change = plant({"a": {1: grey, 29: other}})
        # The null: the change with no list fails.
        passed, said, _code = stage(folder, change)
        assert not passed and "FAIL 3 of 4 pictures equal" in said, said
        # The list that names it: the stage passes, and names the file and its md5.
        path, md5 = with_list(folder, "a-last.png")
        passed, said, _code = stage(folder, change)
        assert passed and "a-last.png" in said and "listed: a planned change" in said, said
        assert "ok   4 of 4 pictures equal to the accepted ones in %s, 1 of them not equal but changes that the gate list plans" % os.path.join(folder, "now") in said, said
        assert "ok   the gate list %s (md5 %s): 1 of 1 listed pictures changed" % (path, md5) in said, said
        # A change that the list does not name fails; the listed one still passes.
        passed, said, _code = stage(folder, plant({"a": {1: other, 29: other}}))
        assert not passed and "FAIL 3 of 4 pictures equal" in said, said
        # A listed picture that came out equal fails (the Coordinator's answer to part D's question).
        passed, said, _code = stage(folder, plant())
        assert not passed and "FAIL the gate list names a-last.png, and it is equal to its accepted picture" in said, said
        # A name with no accepted picture fails before any replay.
        with_list(folder, "c.png", "a-last.png")
        del prefixes[:]
        passed, said, _code = stage(folder, change)
        assert not passed and "FAIL the gate list names c.png, which has no accepted picture" in said and prefixes == [], said
        # The accept: a gate with the list, then accept-list. A picture that a second replay does not give again copies
        # nothing.
        with_list(folder, "a-last.png")
        assert stage(folder, change)[0]
        turns = iter([{1: grey, 29: grey}])
        done, said = accept_list(folder, plant({"a": lambda: next(turns)}))
        assert not done and "a.nircap: the last frame is not steady" in said and colour(os.path.join(kept, "a-last.png")) == grey, said
        done, said = accept_list(folder, change)
        assert done and "accepted 1 listed pictures (gate list %s md5 %s): a-last.png" % (path, md5) in said, said
        assert colour(os.path.join(kept, "a-last.png")) == other and colour(os.path.join(kept, "a.png")) == grey, "accept-list took another picture"
        # D557: the replay logs of the folder are rewritten with the pictures, and are not counted.
        assert "the replay logs rewritten, not counted: 2" in said, said
        with open(os.path.join(folder, "now", "a" + REPLAY_OUTPUT), encoding="utf-8") as now_log, open(os.path.join(kept, "a" + REPLAY_OUTPUT), encoding="utf-8") as kept_log:
            assert now_log.read() == kept_log.read()
        # Then the next gate with no list is equal: no second round of the same pictures.
        GATE_LIST.clear()
        del GATE_LIST_NOTE[:]
        passed, said, _code = stage(folder, change)
        assert passed and "ok   4 of 4 pictures equal to the accepted ones in" in said and "gate list" not in said, said
        # accept-list on a run whose listed picture is equal: nothing to accept, and nothing is copied.
        with_list(folder, "a-last.png")
        stage(folder, change)
        done, said = accept_list(folder, change)
        assert not done and "a-last.png is equal to its accepted picture: nothing to accept, and the list is wrong" in said, said
        # accept-list needs the list.
        GATE_LIST.clear()
        del GATE_LIST_NOTE[:]
        done, said = accept_list(folder, change)
        assert not done and "the stage accept-list needs --gate-list" in said, said
        # --gate-list with --accept is refused.
        with_list(folder, "a-last.png")
        passed, said, _code = stage(folder, change, accept=True)
        assert not passed and "--gate-list with --accept or --accept-new" in said, said
        GATE_LIST.clear()
        del GATE_LIST_NOTE[:]
        # The file: comments, the three kinds of name, and its refusals.
        listed = os.path.join(folder, "list.txt")
        for text, wanted in (("# planned\nnarsis.png\n\ntour/3-03-x.png # the fog\nhooks/narsis-last.png\n", {"narsis.png", "tour/3-03-x.png", "hooks/narsis-last.png"}),
                             ("narsis.png\nnarsis.png\n", "named twice"), ("tour/a/b.png\n", "is no picture"), ("narsis.txt\n", "is no picture"),
                             ("../accepted/x.png\n", "is no picture"), ("# nothing\n", "names no picture")):
            with open(listed, "w", encoding="utf-8") as file:
                file.write(text)
            try:
                found = read_gate_list(listed)[0]
            except ValueError as error:
                found = str(error)
            assert found == wanted if isinstance(wanted, set) else wanted in found, (text, found)
        try:
            read_gate_list(os.path.join(folder, "none.txt"))
            raise AssertionError("a missing list was read")
        except ValueError as error:
            assert "is no file" in str(error)
        assert accepted_of("B", "tour/x.png") == os.path.join("B", "tour", "accepted", "x.png") and now_of("N", "hooks/x.png") == os.path.join("N", "hooks", "x.png")
        assert accepted_of("B", "x-last.png") == os.path.join("B", "accepted", "x-last.png") and now_of("N", "tour/x.png") == os.path.join("N", "tour", "x.png")


def picture_folders(baseline, accept, now):
    """Where the baseline stage writes: the folder of the pictures, and the folder of the tour's.
    A compare writes into `now`: [`own_now`] when the call names none."""
    if accept:
        return os.path.join(baseline, "accepted"), os.path.join(baseline, "tour", "accepted")
    assert now, "a compare writes into a folder of its own"
    return now, os.path.join(now, "tour")


def own_now(results, when, pid):
    """The folder of a gate's pictures when the call names none: a run folder of its own in
    `results`, by the time and the process, so that two gates at once never write the same
    files (the play set's gate failed with WinError 32 on the shared `now` folder)."""
    return os.path.join(results, time.strftime(RUN_TIME, when) + "-gate-%d" % pid)


def now_problem(baseline, accept, now):
    """Why --now cannot be used, as a text, or None. With --now, the stage writes nothing in the baseline folder."""
    if not now:
        return None
    if accept:
        return "--now and --accept: an acceptance writes the accepted pictures"
    folder, inside = os.path.normcase(os.path.abspath(baseline)), os.path.normcase(os.path.abspath(now))
    if inside == folder or inside.startswith(folder + os.sep):
        return "--now %s is in the baseline folder" % now
    return None


def kept_captures(baseline):
    """The paths of all the captures that the baseline folder keeps: its own, the tour's and the
    instance A/B's (`instance/<name>/`). The stream rule reads them all."""
    paths = []
    for folder in (baseline, os.path.join(baseline, "tour")):
        if os.path.isdir(folder):
            paths += [os.path.join(folder, f) for f in sorted(os.listdir(folder)) if f.endswith(".nircap")]
    instance = os.path.join(baseline, "instance")
    if os.path.isdir(instance):
        for name in sorted(os.listdir(instance)):
            inner = os.path.join(instance, name)
            if os.path.isdir(inner):
                paths += [os.path.join(inner, f) for f in sorted(os.listdir(inner)) if f.endswith(".nircap")]
    return paths


def check_stream_rule(baseline):
    """The stream rule (`stream_rule.py`) on each kept capture: the scenes of the player's views
    must have the projection of the world scene of their frame. The replays compare mode gpu with
    mode off, and both read the projection from the stream, so a wrong projection in the stream
    passes them. A capture with a fault fails the stage. A capture with no scene of kind 5 is not
    run; the stage fails when no capture was run at all, as it saw none of the shadow camera's draws."""
    results = [(path,) + stream_rule.check_capture(path) for path in kept_captures(baseline)]
    for path, status, report in results:
        if status != stream_rule.PASS:
            for line in stream_rule.describe(path, status, report):
                print("       " + line, flush=True)
    failed = [os.path.basename(path) for path, status, _ in results if status == stream_rule.FAIL]
    passed = [path for path, status, _ in results if status == stream_rule.PASS]
    not_run = len(results) - len(passed) - len(failed)
    if failed:
        return check(False, "stream rule: the scenes of the player's views differ in projection from the world scene in %s" % ", ".join(failed))
    if not passed:
        return check(False, "%s: stream rule: no capture of the baseline folder holds a scene of kind 5 (%d captures read)" % (NOT_RUN, len(results)))
    return check(True, "stream rule: %d captures pass, %d have no scene of kind 5 (not run)" % (len(passed), not_run))


# What a capture has in the accepted folder (W526): <name>.png, the picture of frame 1; <name>-last.png, the picture
# of its last frame (a fault that shows only after frame 1, such as a fade, a cache or a filled atlas, shows there);
# <name>.checks.txt, how many things each check of the helper judged in the accepted replay.
LAST = "-last"
JUDGED = ".checks.txt"
# The frames of each replay of the stage, for the helper's --frames-at.
GATE_FRAMES = "1,last"
# The checks that a capture can have nothing to judge for (no start of a world for the extension, no view of its
# own). For each capture the accepted count says whether it must judge; over the stage, one capture at least must.
FLOOR_CHECKS = (EARLY_WORLD_DRAWS, OWN_VIEWS)


def hook_fixtures(folder):
    """The fixtures of a folder, {file name: the set of its hooks}; a file of values only (XE Mod Shadow Data) has
    an empty set. {} when the folder is not there."""
    if not os.path.isdir(folder):
        return {}
    fixtures = {}
    for name in sorted(os.listdir(folder)):
        path = os.path.join(folder, name)
        if HOOK_FIXTURE_NAME.fullmatch(name) and os.path.isfile(path):
            with open(path, encoding="utf-8") as file:
                fixtures[name] = set(HOOK_DEFINITION.findall(file.read()))
    return fixtures


def hooks_env(folder, markers):
    """The variables of the hook fixtures' pass: the core mods only from `folder`, the build markers in `markers`.
    No MGE_XE_CORE_MODS=0."""
    return {"MGE_XE_CORE_MODS_DIR": folder, "MGE_XE_CORE_MOD_MARKERS": markers}


def same_path(a, b):
    return os.path.normcase(os.path.normpath(os.path.abspath(a))) == os.path.normcase(os.path.normpath(os.path.abspath(b)))


def route_problems(output, folder, fixtures):
    """Why a replay's output does not show the route of the hook fixtures of `folder` ({file: hooks},
    [`hook_fixtures`]), as texts (the plan of W528 item 5 with verdict 480):
    a. the helper names the folder of MGE_XE_CORE_MODS_DIR;
    b. the paths of its "read" lines are the fixtures of the folder, no more and no fewer (the helper reads some files
       two times: the set counts, not the lines);
    c. its line of the core mods in use names each fixture, with all the hooks of a file of code;
    d. no other line of the core mods than those of ROUTE_FINE."""
    lines = [line.rstrip("\r") for line in output.split("\n")]
    problems = []
    folders = [found.group(1) for found in map(ROUTE_FOLDER.fullmatch, lines) if found]
    if not any(same_path(shown, folder) for shown in folders):
        problems.append("the helper does not read the core mods from the folder %s only (no line \"core mods: only the folder\")" % folder)
    read = {os.path.normcase(os.path.normpath(found.group(1))) for found in map(ROUTE_READ.fullmatch, lines) if found}
    wanted = {os.path.normcase(os.path.normpath(os.path.join(folder, name))): name for name in fixtures}
    for path in sorted(set(wanted) - read):
        problems.append("%s is not read" % wanted[path])
    for path in sorted(read - set(wanted)):
        problems.append("a file that is not a fixture is read: %s" % path)
    used = [found.group(1) for found in map(ROUTE_USED.fullmatch, lines) if found]
    in_use = {name: {hook.strip() for hook in hooks.split(",")} for line in used for name, hooks in ROUTE_USED_FILE.findall(line)}
    for name, hooks in sorted(fixtures.items()):
        if name not in in_use:
            problems.append("%s is not in use" % name)
        elif hooks and in_use[name] != hooks:
            problems.append("%s is in use with the hooks %s, not %s" % (name, ", ".join(sorted(in_use[name])), ", ".join(sorted(hooks))))
    for line in lines:
        if not line.startswith(("core mod: ", "core mods: ")):
            continue
        if ROUTE_FOLDER.fullmatch(line) or ROUTE_READ.fullmatch(line) or ROUTE_USED.fullmatch(line) or line.startswith(ROUTE_FINE):
            continue
        problems.append("a line of the core mods that a fixture in use does not print: %s" % line)
    return problems


def check_route(what, output, folder, fixtures):
    """The route of the hook fixtures in one replay's output ([`route_problems`]): a FAIL line for each problem, or
    one ok line."""
    problems = route_problems(output, folder, fixtures)
    for problem in problems:
        check(False, "%s: the route of the hook fixtures: %s" % (what, problem))
    return problems == [] and check(True, "%s: the route of the hook fixtures: %d files read and in use" % (what, len(fixtures)))


def replay_frames(capture, prefix, mge_build, env):
    """One replay of a capture for the gate: frame 1 into <prefix>.png and the last frame into <prefix>-last.png.
    Returns (code, output, last frame or None). A capture whose last frame is frame 1 or before has no later
    picture. Pictures of an earlier replay with this prefix are removed first: no stale picture is judged."""
    folder, name = os.path.split(prefix)
    numbered = re.compile(re.escape(name) + r"-(\d{5})\.png")
    for stale in [prefix + ".png", prefix + LAST + ".png"] + [os.path.join(folder, entry) for entry in os.listdir(folder) if numbered.fullmatch(entry)]:
        if os.path.isfile(stale):
            os.remove(stale)
    code, output = run_code([PY, os.path.join(HERE, "replay.py"), capture, prefix, "--frames-at", GATE_FRAMES] + build_option(mge_build), env)
    frames = sorted(int(found.group(1)) for found in map(numbered.fullmatch, os.listdir(folder)) if found)
    last = None
    for frame in frames:
        made = "%s-%05d.png" % (prefix, frame)
        if frame == 1:
            os.replace(made, prefix + ".png")
        elif frame == frames[-1]:
            os.replace(made, prefix + LAST + ".png")
            last = frame
        else:
            os.remove(made)
    return code, output, last


def read_judged(path):
    """The judged counts of a capture's checks at its accept, {name: count}; None when the file is not there."""
    if not os.path.isfile(path):
        return None
    with open(path, encoding="utf-8") as file:
        return {name: int(count) for name, count in (line.rstrip("\n").split("\t") for line in file if "\t" in line)}


def write_judged(path, checks):
    with open(path, "w", encoding="utf-8", newline="\n") as file:
        file.write("".join("%s\t%d\n" % (name, judged) for name, (judged, _faults) in sorted(checks.items())))


def stopped_judging(accepted, now):
    """The checks that judged above 0 at the accept and judge 0 now, as texts. Only that step is compared, not the
    number: a capture that still judges its check judges it. A check with no line now is not here: it is NOT RUN
    (`judge_checks`), which is another thing than a check that ran and judged nothing."""
    return ["the check \"%s\" judged %d things at the accept and judges 0 now" % (name, count)
            for name, count in sorted(accepted.items()) if count > 0 and name in now and now[name][0] == 0]


def equal_pictures(a, b):
    """(equal, text) for two picture files."""
    import picture_diff

    numbers = picture_diff.compare(picture_diff.load(a), picture_diff.load(b))
    if "problem" not in numbers and numbers["different"] == 0:
        return True, "equal"
    return False, numbers.get("problem") or "%.2f %% different, mean %.3f, largest %d" % (numbers["different"] * 100, numbers["mean"], numbers.get("largest", 0))


# replay.py's line on standard error when it keeps a run folder: a failed replay, or a file that the helper left there.
RUN_FOLDER_KEPT = re.compile(r"replay: the run folder (.+) is kept: ")


@contextlib.contextmanager
def scratch_folder(prefix):
    """A temporary folder for files of the gate that it does not keep, as (folder, keep). `keep(code, output, what)`
    is called after each replay into the folder. The folder is removed at the end only when every replay in it ended
    with 0 and kept no run folder: a kept run folder holds the helper's log and its crash dump (verdict 512). A folder
    that stays, kept or not removable (a file in use), is named in a line, and the stage's result stands."""
    folder = tempfile.mkdtemp(prefix=prefix)
    reasons = []

    def keep(code, output, what):
        if code != 0:
            reasons.append("%s: replay.py ended with the code %d" % (what, code))
        elif RUN_FOLDER_KEPT.search(output):
            reasons.append("%s: replay.py kept its run folder" % what)

    try:
        yield folder, keep
    finally:
        if reasons:
            print("       the folder %s is kept: %s" % (folder, "; ".join(reasons)), flush=True)
        else:
            shutil.rmtree(folder, ignore_errors=True)
            if os.path.exists(folder):
                print("       the folder %s is left: it could not be removed (a file in it is in use)" % folder, flush=True)


def steady_problems(capture, name, pictures, mge_build, env):
    """Why the pictures of a capture cannot be accepted, as texts: a second replay must give each of them again,
    byte for byte. A frame that follows the clock (an arrival of distant statics) differs from run to run, and an
    accepted picture of it would fail the gate by chance. The second replay's pictures go to a temporary folder,
    removed when the judgement ends, unless the replay failed ([`scratch_folder`])."""
    with scratch_folder("gate-steady-") as (again, keep):
        code, output, _last = replay_frames(capture, os.path.join(again, name), mge_build, env)
        keep(code, output, name)
        if code != 0:
            return ["the second replay ended with the code %d" % code]
        problems = []
        for suffix in pictures:
            second = os.path.join(again, name + suffix + ".png")
            equal, said = equal_pictures(pictures[suffix], second) if os.path.isfile(second) else (False, "no picture in the second replay")
            if not equal:
                problems.append("%s is not steady: two replays differ (%s)" % ("frame 1" if suffix == "" else "the last frame", said))
        return problems


def read_gate_list(path):
    """The names of a gate list (W567 part D) and the file's md5, or raises ValueError with the reason. One name for
    each line; "#" starts a comment, and an empty line is no name. A name is a picture of the baseline folder:
    "<file>.png", "tour/<file>.png" or "hooks/<file>.png", with no other folder in it."""
    if not os.path.isfile(path):
        raise ValueError("the gate list %s is no file" % path)
    with open(path, "rb") as file:
        data = file.read()
    names = set()
    for number, line in enumerate(data.decode("utf-8").splitlines(), 1):
        name = line.split("#", 1)[0].strip()
        if not name:
            continue
        rest = name[len(TOUR_PREFIX):] if name.startswith(TOUR_PREFIX) else name[len(HOOKS_PREFIX):] if name.startswith(HOOKS_PREFIX) else name
        if not rest.endswith(".png") or "/" in rest or "\\" in rest or rest.startswith("."):
            raise ValueError("the gate list %s, line %d: %r is no picture of the baseline folder (<file>.png, tour/<file>.png or hooks/<file>.png)" % (path, number, name))
        if name in names:
            raise ValueError("the gate list %s, line %d: %s is named twice" % (path, number, name))
        names.add(name)
    if not names:
        raise ValueError("the gate list %s names no picture" % path)
    return names, hashlib.md5(data).hexdigest()


def accepted_of(baseline, name):
    """The accepted picture of a name of a gate list."""
    if name.startswith(TOUR_PREFIX):
        return os.path.join(baseline, "tour", "accepted", name[len(TOUR_PREFIX):])
    if name.startswith(HOOKS_PREFIX):
        return os.path.join(baseline, HOOKS_ACCEPTED, name[len(HOOKS_PREFIX):])
    return os.path.join(baseline, "accepted", name)


def now_of(now, name):
    """The picture of this gate's run for a name of a gate list, in the folders of `picture_folders` and `stage_baseline`."""
    if name.startswith(TOUR_PREFIX):
        return os.path.join(now, "tour", name[len(TOUR_PREFIX):])
    if name.startswith(HOOKS_PREFIX):
        return os.path.join(now, "hooks", name[len(HOOKS_PREFIX):])
    return os.path.join(now, name)


def gate_list_start(baseline):
    """Before the replays: each name of the gate list has an accepted picture, so that a wrong name fails at once."""
    ok = True
    for name in sorted(GATE_LIST):
        if not os.path.isfile(accepted_of(baseline, name)):
            ok &= check(False, "the gate list names %s, which has no accepted picture (%s): no such capture, frame or stop" % (name, accepted_of(baseline, name)))
    return ok


def gate_list_verdict():
    """After the compare: each name of the gate list differed. A listed picture that came out equal fails (the
    Coordinator, W567 part D): the planned change did not come, or the list is wrong."""
    if not GATE_LIST_NOTE:
        return True
    path, md5 = GATE_LIST_NOTE[0]
    changed = set(CHANGED)
    ok = True
    for name in sorted(GATE_LIST - changed):
        ok &= check(False, "the gate list names %s, and it is equal to its accepted picture or was not compared: the planned change did not come, or the list is wrong" % name)
    return check(ok, "the gate list %s (md5 %s): %d of %d listed pictures changed" % (path, md5, len(GATE_LIST & changed), len(GATE_LIST)))


def stage_accept_list(baseline, mge_build, now, hooks=HOOK_FIXTURES):
    """W567 part D: after a merge round that passed with a gate list, the listed pictures of the gate's run (`now`)
    become the accepted ones, and nothing else. Each must differ from its accepted picture now; the pictures of a
    capture are accepted only when a second replay gives them again (`steady_problems`, as --accept-new); a tour stop's
    picture is taken as the tour's accept takes it. Nothing is copied unless every listed picture can be. Then the
    tool's own files of each folder whose pictures were accepted (the replay logs `*.replay.txt` of the gate's run) are
    rewritten too, and are not counted as changes (D557). That is every replay log of the folder, also the logs of the
    captures that the list does not name (verdict 607): a log is no picture, and the gate's run made all of them from
    the same commits."""
    print("accept the gate list into %s" % baseline, flush=True)
    if not GATE_LIST_NOTE:
        return check(False, "the stage accept-list needs --gate-list: it accepts only the listed pictures")
    if not now or not os.path.isdir(now):
        return check(False, "the stage accept-list needs --now <the folder of the gate's pictures>, and %s is none" % now)
    path, md5 = GATE_LIST_NOTE[0]
    ok, pairs = True, []
    for name in sorted(GATE_LIST):
        source, target = now_of(now, name), accepted_of(baseline, name)
        if not os.path.isfile(source):
            ok &= check(False, "%s: the gate made no picture %s" % (name, source))
        elif not os.path.isfile(target):
            ok &= check(False, "%s: no accepted picture %s" % (name, target))
        elif equal_pictures(target, source)[0]:
            ok &= check(False, "%s is equal to its accepted picture: nothing to accept, and the list is wrong" % name)
        else:
            pairs.append((name, source, target))
    if not ok:
        return False
    by_capture = {}
    for name, source, _target in pairs:
        if name.startswith(TOUR_PREFIX):
            continue
        prefix = HOOKS_PREFIX if name.startswith(HOOKS_PREFIX) else ""
        stem = name[len(prefix):-len(".png")]
        last = stem.endswith(LAST)
        by_capture.setdefault((prefix, stem[:-len(LAST)] if last else stem), {})[LAST if last else ""] = source
    for (prefix, capture), pictures in sorted(by_capture.items()):
        nircap = os.path.join(baseline, capture + ".nircap")
        if not os.path.isfile(nircap):
            ok &= check(False, "%s%s: no capture %s" % (prefix, capture, nircap))
            continue
        if prefix:
            with scratch_folder("gate-hook-markers-") as (markers, _keep):
                problems = steady_problems(nircap, capture, pictures, mge_build, hooks_env(hooks, markers))
        else:
            problems = steady_problems(nircap, capture, pictures, mge_build, dict(GATE_ENV))
        for problem in problems:
            ok &= check(False, "%s%s.nircap: %s: not accepted" % (prefix, capture, problem))
    if not ok:
        return False
    for _name, source, target in pairs:
        shutil.copyfile(source, target)
    logs = []
    for folder in sorted({os.path.dirname(source) for _name, source, _target in pairs}):
        target_folder = os.path.dirname(next(target for _name, source, target in pairs if os.path.dirname(source) == folder))
        for entry in sorted(os.listdir(folder)):
            if entry.endswith(REPLAY_OUTPUT) and os.path.isfile(os.path.join(folder, entry)):
                shutil.copyfile(os.path.join(folder, entry), os.path.join(target_folder, entry))
                logs.append(entry)
    return check(True, "accepted %d listed pictures (gate list %s md5 %s): %s; the replay logs rewritten, not counted: %d" % (
        len(pairs), path, md5, ", ".join(name for name, _, _ in pairs), len(logs)))


def stage_baseline(baseline, accept, mge_build, now=None, add=False, hooks=HOOK_FIXTURES):
    """The captures of the baseline folder against their accepted state, in the order of the draws of the game, as
    a player has them. Each capture is judged, and what cannot be judged is said (W526): its checks (a missing line
    is NOT RUN, a fault is a FAIL, a check that judged at the accept and judges nothing now is a FAIL); its frame 1
    and its last frame against their accepted pictures (a capture with no accepted picture is a FAIL by name, and
    so is an accepted picture with no capture). `accept` writes the accepted state of every capture again; `add`
    (--accept-new) writes only the state that a capture lacks, after the compare of what it has, and replaces
    nothing. Both accept a picture only when a second replay gives it again. Then the same captures with the hook
    fixtures of the folder `hooks` ([`hooks_pass`]); None leaves that pass out."""
    print("baseline %s" % baseline, flush=True)
    missing = build_missing(mge_build)
    if missing:
        return check(False, "the build folder %s has no %s: its replays would draw other pictures" % (mge_build, ", ".join(missing)))
    captures = sorted(f for f in os.listdir(baseline) if f.endswith(".nircap")) if os.path.isdir(baseline) else []
    if not captures:
        return check(False, "no capture in the baseline folder")
    if not check_stream_rule(baseline):
        return False
    del CHANGED[:]
    if GATE_LIST_NOTE and (accept or add):
        return check(False, "--gate-list with --accept or --accept-new: a gate list compares, and the stage accept-list accepts it after a pass")
    if not gate_list_start(baseline):
        return False
    out, tour_out = picture_folders(baseline, accept, now)
    ok = baseline_pass(baseline, captures, accept, mge_build, out, add, tour_out=tour_out)
    if hooks is not None:
        hooks_out = os.path.join(baseline, HOOKS_ACCEPTED) if accept else os.path.join(now, "hooks")
        ok = hooks_pass(baseline, captures, accept, mge_build, hooks_out, add, hooks) and ok
    return gate_list_verdict() and ok


def hooks_pass(baseline, captures, accept, mge_build, out, add, hooks):
    """W528 item 5 (F-G-12): the captures again with our hook files, one for each kind of hook, from the folder
    `hooks` (MGE_XE_CORE_MODS_DIR), against the accepted state in HOOKS_ACCEPTED of the baseline folder. The plain
    pass replays with no core mod, so only this pass judges the helper's path that joins a hook file into its
    shaders. Each replay must show the route ([`route_problems`]). The build markers go to a new folder of this pass,
    never to the user's, and the folder is removed when the pass ends. A folder with no fixture is a FAIL: the pass is
    never left out without a line."""
    fixtures = hook_fixtures(hooks)
    if not fixtures:
        return check(False, "hook fixtures: %s has no XE Mod *.wgsl file: the gate cannot judge the path of a hook file" % hooks)
    print("hook fixtures %s: %s" % (hooks, ", ".join(sorted(fixtures))), flush=True)
    with scratch_folder("gate-hook-markers-") as (markers, _keep):
        env = hooks_env(hooks, markers)
        return baseline_pass(baseline, captures, accept, mge_build, out, add, env=env, accepted=os.path.join(baseline, HOOKS_ACCEPTED), route=(hooks, fixtures))


def baseline_pass(baseline, captures, accept, mge_build, out, add, env=None, accepted=None, route=None, tour_out=None):
    """One pass of the baseline stage over `captures` with the variables `env` (GATE_ENV by default), its pictures
    in `out`, against `accepted` (the folder `accepted` of the baseline by default). The plain pass (`route` None)
    also judges the tour and names the adapter. The hook fixtures' pass gives `route`, (folder, fixtures): each
    replay must show that route."""
    plain = route is None
    env = dict(GATE_ENV if env is None else env)
    accepted = accepted or os.path.join(baseline, "accepted")
    title = "" if plain else "hook fixtures: "
    os.makedirs(out, exist_ok=True)
    adapter = None
    ok = True
    now_state = {}
    for capture in captures:
        name = capture[:-7]
        code, output, last = replay_frames(os.path.join(baseline, capture), os.path.join(out, name), mge_build, env)
        keep_replay_output(os.path.join(out, name + ".png"), code, output)
        if not ran((code, output), "replay.py", accept):
            return False
        adapter = adapter or adapter_of(output)
        # The two checks of the floor can have nothing to judge in a capture: its accepted counts and the floor say
        # whether that is right. Each other check must judge (records, frames, stage keys).
        ok &= judge_checks(title + capture, output, may_judge_nothing=FLOOR_CHECKS)
        if not plain:
            ok &= check_route(title + capture, output, *route)
        pictures = {suffix: os.path.join(out, name + suffix + ".png") for suffix in ("", LAST)}
        pictures = {suffix: picture for suffix, picture in pictures.items() if os.path.isfile(picture)}
        ok &= check("" in pictures, "%s%s: %s" % (title, capture, "frame 1 and frame %d (the last)" % last if last else "frame 1 (the capture has no later frame)") if "" in pictures else "%s%s: the replay made no picture of frame 1" % (title, capture))
        now_state[name] = (helper_checks(output), pictures)
    for check_name in FLOOR_CHECKS:
        if not any(checks.get(check_name, (0, 0))[0] for checks, _pictures in now_state.values()):
            ok &= not_run("%sno capture of the baseline judges the check \"%s\": the stage cannot show it" % (title, check_name))
    tour = os.path.join(baseline, "tour") if plain else None
    if tour and os.path.isdir(tour) and not ran(run_code([PY, os.path.join(HERE, "validate_tour.py"), tour, "--out", tour_out, "--jobs", "4"] + build_option(mge_build), env), "validate_tour.py", accept):
        return False
    adapter_path = os.path.join(accepted, ADAPTER_FILE)
    if accept:
        for name, (checks, pictures) in sorted(now_state.items()):
            for problem in steady_problems(os.path.join(baseline, name + ".nircap"), name, pictures, mge_build, env):
                ok &= check(False, "%s.nircap: %s: the accepted folder holds a picture that the gate cannot repeat" % (name, problem))
            write_judged(os.path.join(accepted, name + JUDGED), checks)
        made = sum(len(pictures) for _checks, pictures in now_state.values())
        if not plain:
            return check(ok, "%s%d captures accepted (%d pictures) in %s" % (title, len(captures), made, accepted))
        if adapter:
            with open(adapter_path, "w", encoding="utf-8", newline="\n") as out_file:
                out_file.write(adapter + "\n")
        elif os.path.isfile(adapter_path):
            # No adapter printed this time: an old record would name the wrong one.
            os.remove(adapter_path)
        return check(ok, "%d captures accepted (%d pictures)%s, on %s" % (len(captures), made, ", and the tour" if os.path.isdir(tour) else "", adapter or "an adapter the helper did not print"))
    if plain:
        recorded = open(adapter_path, encoding="utf-8").read().strip() if os.path.isfile(adapter_path) else None
        print("       " + adapter_note(recorded, adapter), flush=True)
    sys.path.insert(0, HERE)

    # The names of this pass in a gate list (W567 part D): a listed picture that differs passes, as a planned change.
    prefix = "" if plain else HOOKS_PREFIX
    equal, wanted, added, as_listed = 0, 0, [], 0
    for name, (checks, pictures) in sorted(now_state.items()):
        capture = name + ".nircap"
        what = title + capture
        lacks = []
        fine = True
        for suffix, picture in sorted(pictures.items()):
            wanted += 1
            kept = os.path.join(accepted, name + suffix + ".png")
            if not os.path.isfile(kept):
                lacks.append(suffix)
                continue
            same, said = equal_pictures(kept, picture)
            listed = not same and prefix + name + suffix + ".png" in GATE_LIST
            if not same:
                CHANGED.append(prefix + name + suffix + ".png")
            equal += same or listed
            as_listed += listed
            fine &= same or listed
            if not same:
                print("       %-40s %s%s" % (os.path.basename(kept), said, "; listed: a planned change" if listed else ""), flush=True)
        judged = read_judged(os.path.join(accepted, name + JUDGED))
        for problem in stopped_judging(judged or {}, checks):
            fine &= check(False, "%s: %s" % (what, problem))
        if not add:
            for suffix in lacks:
                fine &= check(False, "%s has no accepted picture of %s: the gate cannot judge it (accept it with --accept-new)" % (what, "frame 1" if suffix == "" else "its last frame"))
            if judged is None:
                fine &= check(False, "%s has no accepted counts of its checks (%s): the gate cannot tell a check that stopped judging (accept it with --accept-new)" % (what, name + JUDGED))
        elif fine and (lacks or judged is None):
            # Only what the capture lacks, and only when what it has is equal: nothing is replaced.
            problems = steady_problems(os.path.join(baseline, capture), name, {suffix: pictures[suffix] for suffix in lacks}, mge_build, env) if lacks else []
            for problem in problems:
                fine &= check(False, "%s: %s: not accepted" % (what, problem))
            if not problems:
                os.makedirs(accepted, exist_ok=True)
                for suffix in lacks:
                    shutil.copyfile(pictures[suffix], os.path.join(accepted, name + suffix + ".png"))
                    added.append(name + suffix + ".png")
                    equal += 1
                if judged is None:
                    write_judged(os.path.join(accepted, name + JUDGED), checks)
                    added.append(name + JUDGED)
        ok &= fine
    # The other way: an accepted picture that no capture of this run has.
    known = {name + suffix + ".png" for name, (_checks, pictures) in now_state.items() for suffix in pictures}
    strays = sorted(f for f in os.listdir(accepted) if f.endswith(".png") and f not in known) if os.path.isdir(accepted) else []
    for stray in strays:
        ok &= check(False, "%sthe accepted picture %s has no capture and no frame of this run: the gate judged nothing with it" % (title, stray))
    if add:
        print("       %saccepted new: %s" % (title, ", ".join(added) or "nothing: each capture has its accepted state"), flush=True)
    listed_note = lambda count: ", %d of them not equal but changes that the gate list plans" % count if GATE_LIST_NOTE else ""  # noqa: E731
    ok &= check(wanted != 0 and equal == wanted and not strays, "%s%d of %d pictures equal to the accepted ones in %s%s" % (title, equal, wanted, out, listed_note(as_listed)))
    print("       %d captures: frame 1 of each, and the last frame of %d" % (len(now_state), sum(LAST in pictures for _checks, pictures in now_state.values())), flush=True)
    if tour and os.path.isdir(tour):
        kept_tour = os.path.join(tour, "accepted")
        names = sorted(f for f in os.listdir(kept_tour) if f.endswith(".png")) if os.path.isdir(kept_tour) else []
        equal, as_listed = 0, 0
        for name in names:
            if not os.path.isfile(os.path.join(tour_out, name)):
                print("       %-40s no picture now" % name, flush=True)
                continue
            same, said = equal_pictures(os.path.join(kept_tour, name), os.path.join(tour_out, name))
            listed = not same and TOUR_PREFIX + name in GATE_LIST
            if not same:
                CHANGED.append(TOUR_PREFIX + name)
            equal += same or listed
            as_listed += listed
            if not same:
                print("       %-40s %s%s" % (name, said, "; listed: a planned change" if listed else ""), flush=True)
        ok &= check(names != [] and equal == len(names), "%d of %d pictures equal to the accepted ones in %s%s" % (equal, len(names), tour_out, listed_note(as_listed)))
    return stage_objects(baseline, mge_build, out, env=env, route=route) and ok


OBJECTS_LINE = re.compile(r"^gpu objects: (\d+) made in (\d+) frames; (\d+) steady frames, (\d+) of them made objects \((\d+) objects\)", re.M)
OBJECTS_CHANGES = "gpu objects: the frames that were not steady, by their first change: "
# The name of the helper's change for the subsets of distant statics that came (W340).
OBJECTS_ARRIVAL = re.compile(r"an arrival of distant statics (\d+)")
# The allowed object (D310): the view of the window's surface, at most 1 in a steady frame.
OBJECTS_SURFACE = re.compile(r"^gpu objects allowed \(D310\): the surface view, (\d+) in the steady frames, at most (\d+) a frame", re.M)
# The backends of the objects stage: each replay runs plain, to its last frame, with no GPU time
# and no marks, so that nothing waits for the GPU (risk 79: a wait hid a lost dx12 device).
OBJECT_BACKENDS = ("vulkan", "dx12")
# True since the last site of W340 landed: a site found fails the gate. False only to look at the
# sites of a build without a stop.
OBJECTS_STRICT = True


def objects_verdict(output):
    """The verdict of one replay of the objects stage, from its output: (verdict, text). The verdict
    is "ok"; "sites" for steady frames that made GPU objects (with the lines of the objects);
    "idle" for a capture with no steady frame (with the first changes of its frames); "fail" for a
    lost device, for a build without the count (no report line) and for more than one surface view
    in a steady frame (D310)."""
    if "device is lost" in output:
        return "fail", "the device is lost"
    found = OBJECTS_LINE.search(output)
    if not found:
        return "fail", "no 'gpu objects' report line: a build without the count of the objects (D288)"
    made, frames, steady, steady_made, objects = (int(n) for n in found.groups())
    if steady == 0:
        changes = next((line for line in output.splitlines() if line.startswith(OBJECTS_CHANGES)), OBJECTS_CHANGES + "none")
        return "idle", "%d frames, none steady: the capture cannot show the rule; %s" % (frames, changes[len("gpu objects: "):])
    surface = OBJECTS_SURFACE.search(output)
    if surface and int(surface.group(2)) > 1:
        return "fail", "a steady frame made %s views of the window's surface; D310 allows 1" % surface.group(2)
    arrivals = arrival_frames(output)
    unchecked = "; %d frames changed first by an arrival of distant statics, not checked" % arrivals if arrivals else ""
    if steady_made:
        sites = next((line for line in output.splitlines() if line.startswith("gpu objects of the steady frames: ")), "")
        return "sites", "%d of %d steady frames made %d objects: %s%s" % (steady_made, steady, objects, sites[len("gpu objects of the steady frames: "):], unchecked)
    return "ok", "%d steady frames of %d made no object (%d made in the others%s)" % (steady, frames, made, unchecked)


def arrival_frames(output):
    """The frames whose first change is an arrival of distant statics (W340). The helper names
    it after the new content of the game, its records and its loads, so none of them changed
    these frames; a change that a part of the crate notes (a picture, the texture arrays) comes
    after it. The steady rule does not check these frames."""
    changes = next((line for line in output.splitlines() if line.startswith(OBJECTS_CHANGES)), "")
    found = OBJECTS_ARRIVAL.search(changes)
    return int(found.group(1)) if found else 0


def replay_end(code, output):
    """The end of a line about a replay: "" for the exit code 0, else the code, with replay.py's line of the fault of
    the stage shaders when the code is that fault's (W523)."""
    if code == 0:
        return ""
    fault = replay_fault_in(output) if code in (EXIT_REPLAY_STAGE_SHADERS, EXIT_REPLAY_CHECKS) else ""
    return " (replay.py ended with the code %d%s)" % (code, ": " + fault if fault else "")


def stage_objects(baseline, mge_build, keep=None, env=None, route=None):
    """Each capture of the baseline folder to its last frame, plain, on each backend of
    OBJECT_BACKENDS: no steady frame (no new content, no change of the extension) may make a GPU
    object, and no device may be lost (D288). A steady frame that made objects warns while
    OBJECTS_STRICT is False. A capture with no steady frame warns; a backend with no capture that
    has a steady frame fails. The stage compares no picture, so its pictures go to a temporary
    folder, removed when the stage ends unless a replay failed ([`scratch_folder`]); the output of
    each replay stays in `keep` (the folder of the gate's pictures) as
    objects-<backend>-<capture>.replay.txt. `env` is GATE_ENV by default; the hook fixtures' pass
    gives its own, and `route` (folder, fixtures): each replay must show that route too, as a
    hook that is dropped tests no object of its path (verdict 480)."""
    title = "" if route is None else "hook fixtures: "
    env = dict(GATE_ENV if env is None else env)
    captures = sorted(f for f in os.listdir(baseline) if f.endswith(".nircap")) if os.path.isdir(baseline) else []
    if not captures:
        return check(False, "objects: no capture in the baseline folder")
    ok = True
    with scratch_folder("gate-objects-") as (out, keep_folder):
        for backend in OBJECT_BACKENDS:
            steady_seen = False
            for capture in captures:
                code, output = run_code([PY, os.path.join(HERE, "replay.py"), os.path.join(baseline, capture), os.path.join(out, "%s-%s.png" % (capture[:-7], backend)), "--backend", backend] + build_option(mge_build), env)
                keep_folder(code, output, "%s, %s" % (backend, capture))
                if keep:
                    keep_replay_output(os.path.join(keep, "objects-%s-%s" % (backend, capture[:-7])), code, output)
                if route is not None:
                    ok &= check_route("%sobjects, %s, %s" % (title, backend, capture), output, *route)
                verdict, text = objects_verdict(output)
                line = "%sobjects, %s, %s: %s%s" % (title, backend, capture, text, replay_end(code, output))
                steady_seen |= verdict in ("ok", "sites")
                if code == 0 and (verdict == "idle" or (verdict == "sites" and not OBJECTS_STRICT)):
                    ok &= warn(line)
                else:
                    ok &= check(verdict == "ok" and code == 0, line)
            if not steady_seen:
                ok &= check(False, "%sobjects, %s: no capture has a steady frame: the stage cannot show the rule" % (title, backend))
    return ok


def stage_only(place, mge_build):
    print("only at %s" % place, flush=True)
    folder = game("v-only-" + place, PLACES[place] + ["--steps", "exterior,rate"], mge_build)
    if not folder:
        return check(False, "no run")
    if not check_weather(folder):
        return False
    log = open(os.path.join(folder, "MWSE.log"), errors="replace").read()
    rate = re.search(r"rate: ([\d.]+) frames", log)
    lines = [l for l in log.splitlines() if "[nirender.owndevice] after " in l]
    ports = []
    for line in lines:
        m = re.search(r"frames (\d+) ms_per_frame [\d.]+ ms stream ([\d.]+) ports ([\d.]+)", line)
        if m:
            ports.append((int(m.group(1)), float(m.group(3))))
    per_frame = (ports[-1][1] - ports[-2][1]) / max(ports[-1][0] - ports[-2][0], 1) if len(ports) >= 2 else float("nan")
    host = open(os.path.join(folder, "host.txt"), errors="replace").read()
    busy = re.search(r"mark after rate: .*busy ([\d.]+) ms", host)
    gpu = re.search(r"mark after rate:(?:.*\n)*?\s+gpu: ([\d.]+) ms in the passes, ([\d.]+) ms the frame", host)
    ok = check(rate is not None, "frame rate %s, ports %.3f ms for each frame, helper busy %s ms, GPU %s ms in the passes, %s ms the frame" % (
        rate.group(1) if rate else "?", per_frame, busy.group(1) if busy else "?", gpu.group(1) if gpu else "?", gpu.group(2) if gpu else "?"))
    ok &= check_counts(folder)
    skinned = re.search(r"skinned shapes (\d+), particle systems (\d+)", host)
    print("       skinned shapes %s, particle systems %s" % (skinned.groups() if skinned else ("?", "?")), flush=True)
    return ok


# The limits of the clusters stage's compare (the replay with the clusters against the replay with the lights of the
# draws). The mean over all pixels alone passed a wrong area of a few percent of the picture (the audit's F-G-19), so
# the stage also allows NO pixel that differs by more than 8 levels (the Architect's limit, W526).
# The expectation, written before the first run under it: the clusters and the lights of the draws are two forms of
# one light. Three kept captures gave 0 pixels different on 2026-11-16 (seyda, narsis, night-masser at frame 20;
# narsis with lights in its clusters), under the instance's settings. No capture of this stage's own place under its
# own settings (the expanded light limit, a night place with many lights) was compared then: the first run of the
# stage there is the test of this limit. A failure here means that the expectation was wrong or that the code is.
# It is not noise: bring the numbers and their cause to the Architect before the limit moves.
CLUSTERS_MEAN_LIMIT = 0.5
CLUSTERS_OVER_8_LIMIT = 0


def clusters_verdict(numbers):
    """(passed, text) for the numbers of picture_diff.compare of the two replays of the clusters stage: the share of
    pixels over 8 levels with its count and the largest level, and the mean, each against its limit."""
    if "problem" in numbers:
        return False, "replay with the clusters against the lights of the draws: %s" % numbers["problem"]
    over = round(numbers.get("over8", 0) * numbers.get("pixels", 0))
    passed = over <= CLUSTERS_OVER_8_LIMIT and numbers["mean"] < CLUSTERS_MEAN_LIMIT
    said = "replay with the clusters against the lights of the draws: %d pixels by more than 8 levels (%.4f %%; the limit is %d), the largest level %d, mean %.3f (the limit is under %.1f), %.2f %% different" % (
        over, numbers.get("over8", 0) * 100, CLUSTERS_OVER_8_LIMIT, numbers.get("largest", 0), numbers["mean"], CLUSTERS_MEAN_LIMIT, numbers["different"] * 100)
    return passed, said if passed else said + ": the two are two forms of one light, so the expectation was wrong or the code is; this is not noise (the numbers and their cause go to the Architect before a limit moves)"


def stage_clusters(place, mge_build, started):
    """The replays of the capture that this validate run made at the place (`started`, in RUN_TIME's
    form, is the start of this run). An older capture of the place is not used: it can be of another
    build or of another session."""
    print("clusters at %s" % place, flush=True)
    folder = newest("v-capture-" + place, started)
    capture = folder and os.path.join(folder, "exterior.nircap")
    if not capture or not os.path.isfile(capture):
        return check(False, "no capture of this run at %s: run the capture stage in this run" % place)
    a, b = os.path.join(folder, "replay-clusters.png"), os.path.join(folder, "replay-draw-lights.png")
    # The replays read the mgeXE.toml of the run (the fade of the lights is on in it).
    # The replays use the build under test: the default of replay.py is the folder of
    # `cargo xtask build`, which can hold an older helper.
    # A replay that failed fails the stage with its code: before W523 only a missing line or picture showed it.
    replay = [PY, os.path.join(HERE, "replay.py"), capture, a, "--frame", "20", "--config-root", folder] + build_option(mge_build)
    code, out = run_code(replay, {"MGE_XE_CLUSTERS": "1"})
    keep_replay_output(a, code, out)
    if not ran((code, out), "replay.py with the clusters"):
        return False
    replay[3] = b
    other = run_code(replay, {"MGE_XE_CLUSTERS": "0"})
    keep_replay_output(b, *other)
    if not ran(other, "replay.py with the lights of the draws"):
        return False
    sys.path.insert(0, HERE)
    import picture_diff

    used = re.search(r"clusters: ([\d.]+) lights and (\d+) entries for each frame", out)
    ok = check(used is not None and used.group(2) != "0", used.group(0) if used else "no clusters line in the replay")
    ok &= check(*clusters_verdict(picture_diff.compare(picture_diff.load(a), picture_diff.load(b))))
    for name, env in (("clusters", {"MGE_XE_CLUSTERS": "1"}), ("draw-lights", {"MGE_XE_CLUSTERS": "0"})):
        label = "v-%s-%s" % (name, place)
        env = dict(env)
        env["MGE_XE_GPU_TIME"] = "1"
        run_folder = game(label, PLACES[place] + ["--steps", "exterior,rate"], mge_build, env)
        host = open(os.path.join(run_folder, "host.txt"), errors="replace").read() if run_folder else ""
        rate = re.search(r"rate: ([\d.]+) frames", open(os.path.join(run_folder, "MWSE.log"), errors="replace").read()) if run_folder else None
        gpu = re.search(r"mark after rate:(?:.*\n)*?\s+gpu: ([\d.]+) ms in the passes, ([\d.]+) ms the frame", host)
        busy = re.search(r"mark after rate: .*busy ([\d.]+) ms", host)
        clusters = re.search(r"clusters: .*", host)
        ok &= check(gpu is not None, "%s: frame rate %s, helper busy %s ms, GPU %s ms in the passes, %s ms the frame" % (name, rate.group(1) if rate else "?", busy.group(1) if busy else "?", gpu.group(1) if gpu else "?", gpu.group(2) if gpu else "?"))
        if clusters:
            print("       " + clusters.group(0)[:150], flush=True)
    return ok


# The stage `order` (W535; the Architect's page w535-w538-order-and-origin-2026-11-16.md section 1; verdict 595): each
# capture of the gate replayed in the table's two orders, 0 pixels that differ between them. The helper's lines that it
# reads, agreed with Hob on 2026-11-17: the route line once a replay (an unknown value prints the value the helper uses),
# a digest of the main view's unsorted table draws in their order for each pictured frame (FNV-1a 64, with the count of
# the draws), and a digest of all frames (printed for the record, not judged).
ORDER_SWITCH = "MGE_XE_TABLE_ORDER"
ORDER_VALUES = ("grouped", "engine")
ORDER_ROUTE = re.compile(r"^table order: (\S+)[ \t\r]*$", re.MULTILINE)
ORDER_DIGEST = re.compile(r"^table order digest: frame (\d+) ([0-9a-f]{16}) (\d+) draws[ \t\r]*$", re.MULTILINE)


def order_captures(baseline):
    """The captures of the gate: those of the baseline folder, then those of its tour."""
    found = []
    for folder in (baseline, os.path.join(baseline, "tour")):
        if os.path.isdir(folder):
            found += [os.path.join(folder, name) for name in sorted(os.listdir(folder)) if name.endswith(".nircap")]
    return found


def order_lines(output):
    """The route values and the digests ({frame: (hash, count)}) of one replay's output."""
    return ORDER_ROUTE.findall(output), {int(frame): (digest, int(count)) for frame, digest, count in ORDER_DIGEST.findall(output)}


def order_capture(capture, out, mge_build):
    """The two replays of one capture and their judgement: (state, shown). The state is "pass", "fail" or "not run";
    `shown` is True when a compared frame's digest differs between the two values."""
    name = os.path.basename(capture)[:-len(".nircap")]
    if os.path.basename(os.path.dirname(capture)) == "tour":
        name = "tour-" + name
    replays = {}
    for value in ORDER_VALUES:
        prefix = os.path.join(out, "%s-%s" % (name, value))
        code, output, last = replay_frames(capture, prefix, mge_build, dict(GATE_ENV, **{ORDER_SWITCH: value}))
        keep_replay_output(prefix + ".png", code, output)
        if not ran((code, output), "replay.py of %s with %s=%s" % (name, ORDER_SWITCH, value)):
            return "fail", False
        routes, digests = order_lines(output)
        if routes != [value]:
            not_run("%s: the replay with %s=%s printed the route lines %s, not one \"table order: %s\": the build does "
                    "not take the switch, and the pictures cannot show the order" % (name, ORDER_SWITCH, value, routes or "none", value))
            return "not run", False
        print("       %s, %s: %s" % (name, value, ", ".join("frame %d %s" % (frame, digests[frame][0]) for frame in sorted(digests)) or "no digest"), flush=True)
        frames = {suffix: frame for suffix, frame in (("", 1), (LAST, last)) if frame is not None and os.path.isfile(prefix + suffix + ".png")}
        replays[value] = (prefix, frames, digests)
    (prefix_a, frames_a, digests_a), (prefix_b, frames_b, digests_b) = (replays[value] for value in ORDER_VALUES)
    if frames_a != frames_b or "" not in frames_a:
        check(False, "%s: the pictured frames differ between the values (%s against %s)" % (name, frames_a, frames_b))
        return "fail", False
    state, shown = "pass", False
    for suffix, frame in sorted(frames_a.items()):
        what = "%s frame %d" % (name, frame)
        if frame not in digests_a or frame not in digests_b:
            not_run("%s: no \"table order digest: frame %d\" line in the replay of %s" % (what, frame, ORDER_VALUES[0] if frame not in digests_a else ORDER_VALUES[1]))
            state = "not run" if state == "pass" else state
            continue
        (hash_a, count_a), (hash_b, count_b) = digests_a[frame], digests_b[frame]
        if count_a != count_b:
            check(False, "%s: %d draws with %s and %d with %s: the two orders must draw the same draws" % (what, count_a, ORDER_VALUES[0], count_b, ORDER_VALUES[1]))
            state = "fail"
        shown |= hash_a != hash_b
        same, said = equal_pictures(prefix_a + suffix + ".png", prefix_b + suffix + ".png")
        if not check(same, "%s: %s against %s: %s (%d draws, the orders %s)" % (what, ORDER_VALUES[0], ORDER_VALUES[1], said, count_a, "differ" if hash_a != hash_b else "are the same")):
            state = "fail"
    return state, shown


def stage_order(baseline, mge_build, now):
    """Each capture of the gate (the baseline folder's and its tour's) replayed to frame 1 and its last frame with
    MGE_XE_TABLE_ORDER=grouped and =engine, the pictures of each frame compared: 0 pixels may differ. Each replay must
    print its value's route line and a digest of each pictured frame. At least one capture must have a frame whose
    orders differ, else the two values drew in one order everywhere and the pixels cannot show the order: NOT SHOWN,
    which fails the stage."""
    print("table order (%s=%s against %s) on %s" % (ORDER_SWITCH, ORDER_VALUES[0], ORDER_VALUES[1], baseline), flush=True)
    missing = build_missing(mge_build)
    if missing:
        return check(False, "the build folder %s has no %s: its replays would draw other pictures" % (mge_build, ", ".join(missing)))
    captures = order_captures(baseline)
    if not captures:
        return check(False, "no capture in the baseline folder or its tour")
    out = os.path.join(now, "order")
    os.makedirs(out, exist_ok=True)
    states, shown = [], []
    for capture in captures:
        state, differs = order_capture(capture, out, mge_build)
        states.append(state)
        if differs:
            shown.append(os.path.basename(capture))
    print("       the orders differed in %d of %d captures%s" % (len(shown), len(captures), (": " + ", ".join(shown)) if shown else ""), flush=True)
    if "fail" in states:
        return check(False, "table order: %d of %d captures failed" % (states.count("fail"), len(captures)))
    if "not run" in states:
        return False
    if not shown:
        print("  NOT SHOWN table order: the two values drew every compared frame of %d captures in one order: the pixels cannot "
              "show the order" % len(captures), flush=True)
        FAILS[0] += 1
        return False
    return check(True, "table order: %d captures, every compared frame equal in both values; the orders differed in %d" % (len(captures), len(shown)))


WEATHERS = ["clear", "cloudy", "foggy", "overcast", "rain", "thunder", "ash", "blight", "snow", "blizzard"]


def stage_weathers(place, mge_build):
    print("weathers at %s" % place, flush=True)
    folder = game("v-weathers-" + place, PLACES[place] + ["--steps", "exterior,weathers", "--budget", "300"], mge_build)
    if not folder:
        return check(False, "no run")
    log = open(os.path.join(folder, "MWSE.log"), errors="replace").read()
    rates = weather_rates(log)
    print("       %-9s %6s %6s %6s" % ("", "pass 1", "pass 2", "mean"), flush=True)
    for index, name in enumerate(WEATHERS):
        print("       %-9s %s" % (name, weather_line(rates, index)), flush=True)
    return check(len(rates) == 20, "%d of 20 measurements (10 weathers, two passes)" % len(rates))


def weather_rates(log):
    """The frame rate of each (weather, pass) of the step "weathers" in a log: the second
    pass goes from 9 to 0, so a trend of the run shows between the passes."""
    return {(int(m.group(2)), int(m.group(3))): float(m.group(1)) for m in re.finditer(r"rate: ([\d.]+) frames in a second at '[^']*' weather (\d+) pass (\d+)", log)}


def weather_line(rates, index):
    passes = [rates.get((index, p)) for p in (1, 2)]
    mean = sum(p for p in passes if p is not None) / max(sum(p is not None for p in passes), 1)
    return "%6s %6s %6.1f" % tuple([("%.1f" % p) if p is not None else "?" for p in passes] + [mean])


def stage_tour(mge_build):
    """A run over the test level in one session: at each stop two frames in mode gpu and two
    in mode off, with the time of the game held. validate_tour.py replays and compares the
    pairs. The counts of the run must be clean."""
    print("tour of the test level", flush=True)
    folder = game("v-tour", ["--steps", "tour", "--tour-cells", "@nifTest_1;@nifTest_2;@nifTest_3", "--tour-dual", "--budget", "900"], mge_build)
    if not folder or len([f for f in os.listdir(folder) if f.endswith(".nircap")]) != 3:
        return check(False, "no three captures from the run")
    print("       " + folder, flush=True)
    # The tour step pins no weather (owndevice.lua, the step tour): its line says NOT applied, and
    # the gate names it as not pinned, not as held.
    print("       weather: not pinned: the tour step pins none (the run's line: %s)" % (run_weather_line(folder) or "none"), flush=True)
    ok = check_counts(folder)
    code, out = run_code([PY, os.path.join(HERE, "validate_tour.py"), folder, "--jobs", "4"] + build_option(mge_build), {})
    for line in out.strip().split("\n")[-45:]:
        print("       " + line[:160], flush=True)
    # The tool's exit code is its verdict (W526: its pictures, its replays' fault codes, each pair of mode gpu against
    # mode off against its limit). Before, this stage passed on any count of equal pairs.
    if not ran((code, out), "validate_tour.py"):
        return False
    return ok & check(tour_verdict_in(out).endswith("PASS"), tour_verdict_in(out) or "validate_tour.py printed no verdict")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("stages", nargs="*", default=["capture", "baseline", "only"])
    ap.add_argument("--mge-build", help="the sealed set of the runs and the replays (W545); without it, the current set")
    ap.add_argument("--archive", help="the folder of an unpacked release archive: the release check, the baseline stage only")
    ap.add_argument("--places", default="seyda,narsis")
    ap.add_argument("--baseline", default=os.path.join(REPO, "results", "baseline-records"), help="the folder of the baseline stage")
    ap.add_argument("--accept", action="store_true", help="the baseline stage writes the accepted pictures and compares nothing")
    ap.add_argument("--accept-new", action="store_true", help="the baseline stage compares, and then accepts only what a capture lacks (a picture of frame 1 or of its last frame, "
                    "the counts of its checks); it replaces nothing")
    ap.add_argument("--now", default=None, help="the folder of the baseline stage's pictures, outside the baseline folder; without it, a run folder of its own in results")
    ap.add_argument("--gate-list", default=None, help="the planned changes (W567 part D): a file of the pictures that must differ from the accepted ones; the baseline "
                    "stage passes them and fails on any other change and on a listed picture that stayed equal; the stage accept-list accepts them")
    ap.add_argument("--hook-fixtures", default=HOOK_FIXTURES, help="the folder of the hook fixtures of the baseline stage's second pass (tools/hook-fixtures by default)")
    ap.add_argument("--gpu-time", action="store_true", help="the helper writes GPU time stamps in the only stage")
    ap.add_argument("--disable-mod", default=None, help="a mod of the profile switched off for the runs")
    ap.add_argument("--weather", default="0", help="the weather of the runs, pinned and held: 0 clear (the default) to 9 blizzard; any: the game's, not pinned")
    ap.add_argument("--self-test", action="store_true", help="the tests of the pure parts: no game, no replay")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.accept and args.accept_new:
        print("  FAIL --accept writes every accepted picture again, and --accept-new replaces none: give one of them", flush=True)
        return 1
    del GATE_LIST_NOTE[:]
    GATE_LIST.clear()
    if args.gate_list:
        try:
            names, md5 = read_gate_list(args.gate_list)
        except ValueError as error:
            print("  FAIL %s" % error, flush=True)
            return 1
        GATE_LIST.update(names)
        GATE_LIST_NOTE.append((os.path.abspath(args.gate_list), md5))
        print("the gate list %s (md5 %s): %s" % (os.path.abspath(args.gate_list), md5, ", ".join(sorted(names))), flush=True)
    elif "accept-list" in args.stages:
        print("  FAIL the stage accept-list needs --gate-list: it accepts only the listed pictures", flush=True)
        return 1
    if "accept-list" in args.stages and (args.archive or args.accept or args.accept_new or not args.now):
        print("  FAIL the stage accept-list takes --now <the folder of the gate's pictures> and a set, and no --archive, --accept or --accept-new", flush=True)
        return 1
    problem = build_problem(args.stages, args.mge_build, args.archive)
    if problem:
        print("  FAIL " + problem, flush=True)
        return 1
    mge_build = gate_build(args.mge_build, args.archive)
    if mge_build is None:
        print("  FAIL no --mge-build and no current set (current.bat of the sets folder): make a set of the commits with "
              "tools/build_set.py and name it with --mge-build", flush=True)
        return 1
    problem = now_problem(args.baseline, args.accept, args.now)
    if problem:
        print("  FAIL " + problem, flush=True)
        return 1
    if ("baseline" in args.stages or "order" in args.stages) and not args.accept and not args.now:
        args.now = own_now(os.path.join(REPO, "results"), time.localtime(), os.getpid())
        print("the pictures of this gate: %s" % args.now, flush=True)
    if args.disable_mod:
        RUNNER_ARGS.extend(["--disable-mod", args.disable_mod])
    weather, problem = weather_pin(args.weather)
    if problem:
        print("  FAIL " + problem, flush=True)
        return 1
    WEATHER["pin"] = weather
    if weather is not None:
        RUNNER_ARGS.extend(["--weather", str(weather)])
    print("weather of the runs: %s" % ("pinned %d and held" % weather if weather is not None else "the game's, not pinned (--weather any)"), flush=True)
    if args.gpu_time:
        GPU_TIME["MGE_XE_GPU_TIME"] = "1"
    places = args.places.split(",")
    started = time.strftime(RUN_TIME)
    for stage in args.stages:
        if stage == "capture":
            for place in places:
                if not stage_capture(place, mge_build):
                    return 1
        elif stage == "baseline":
            if not stage_baseline(args.baseline, args.accept, mge_build, args.now, args.accept_new, os.path.abspath(args.hook_fixtures)):
                return baseline_failure_code()
        elif stage == "only":
            for place in places:
                if not stage_only(place, mge_build):
                    return 1
        elif stage == "tour":
            if not stage_tour(mge_build):
                return 1
        elif stage == "clusters":
            for place in places:
                if not stage_clusters(place, mge_build, started):
                    return 1
        elif stage == "order":
            if args.accept:
                print("  FAIL the stage order compares two replays of each capture with each other: it accepts nothing", flush=True)
                return 1
            if not stage_order(args.baseline, mge_build, args.now):
                return baseline_failure_code("order")
        elif stage == "weathers":
            for place in places:
                if not stage_weathers(place, mge_build):
                    return 1
        elif stage == "accept-list":
            if not stage_accept_list(args.baseline, mge_build, args.now, os.path.abspath(args.hook_fixtures)):
                return 1
        else:
            print("unknown stage " + stage)
            return 2
    print("all stages passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
