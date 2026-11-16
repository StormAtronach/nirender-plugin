"""The checks of the retained pipeline, one stage at a time: each stage prints its result at
once and a failed stage stops the run, so a problem shows after one game run (about two
minutes) and not after all of them.

    python tools/validate_retained.py [stage ...] [--mge-build DIR] [--places seyda,narsis]
    python -I tools/validate_retained.py --self-test    # the tests of the pure parts, no game

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

--disable-mod NAME switches a mod of the profile off for the runs (an A/B of a mod).

--weather N pins the weather of the runs and holds it (the runner's --weather): 0, clear, by
default, so that a capture or a rate is made in a known weather. Each capture and rate stage
prints the run's weather line and fails when it does not show the pin held: "held" only when
the mod read the pinned weather back after its switch (run_own_device.py, weather_summary). The
tour step pins no weather, and the tour stage names its weather as not pinned. --weather any
leaves the game's weather, not pinned, and the line names it.
"""
import argparse
import contextlib
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
                    capture_format_in, check_problems, helper_checks, keep_replay_output, plugin_lines, replay_fault_in)
from validate_tour import NOT_RUN, NOT_RUN_CODE as TOUR_NOT_RUN, refusal_of, verdict_in as tour_verdict_in  # noqa: E402
import stream_rule  # noqa: E402
PY = sys.executable
# The variables of the baseline replays: no core mod (`Data Files/shaders/core-mods/*.wgsl`) of
# the replay's folder joins the shaders, so the accepted pictures do not depend on what is
# installed.
GATE_ENV = {"MGE_XE_CORE_MODS": "0"}
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


def baseline_failure_code():
    """The exit code of a baseline stage that did not pass: NOT RUN when it has no FAIL line and could not judge a
    thing (a capture of another format, which stops the stage at once; a check with no line in a replay's output;
    a check that no capture judges), with a "NOT RUN" line for merge_step.py; else 1. A FAIL comes first."""
    reasons = CAPTURE_FORMATS + NOT_RUN_LINES
    if FAILS[0] or not reasons:
        return 1
    print("NOT RUN baseline: %s" % reasons[-1], flush=True)
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
    code, out = run_code([PY, os.path.join(HERE, "replay.py"), capture, a, "--frame", "20", "--mge-build", mge_build], {})
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
                    assert not stage_baseline(folder, accept, "no build", os.path.join(folder, "now"))
            assert len(calls) == (0 if name == "wrong" else 1), (name, accept, calls)
        assert "replay.py" in calls[0][0][1]
        # The replays of the gate run with the core mods off.
        assert calls[0][1].get("MGE_XE_CORE_MODS") == "0", calls[0][1]
    finally:
        build_missing, run_code = real_build_missing, real_run_code
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
    self_test_baseline()
    self_test_tour_stage()
    print("validate_retained self-test: ok")


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
            name = os.path.basename(command[3])
            frames = pictures.get(name, pictures["*"])
            for frame, colour in (frames() if callable(frames) else frames).items():
                picture_diff.Image.new("RGB", (8, 8), colour).save("%s-%05d.png" % (command[3], frame))
            return code, reports.get(name, reports["*"])

        return run

    def stage(folder, tool, accept=False, add=False):
        """(passed, printed, exit code of a stage that did not pass) of the stage on `folder` with the plant `tool`."""
        global run_code
        run_code = tool
        FAILS[0] = 0
        del NOT_RUN_LINES[:]
        with contextlib.redirect_stdout(io.StringIO()) as printed:
            passed = stage_baseline(folder, accept, "no build", None if accept else os.path.join(folder, "now"), add)
            code = 0 if passed else baseline_failure_code()
        return passed, printed.getvalue(), code

    build_missing, check_stream_rule, stage_objects = (lambda _build: []), (lambda _folder: True), (lambda _baseline, _build, keep=None: True)
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
            passed, said, code = stage(folder, plant({"new": {1: other, 29: other}}))
            assert passed and "ok   6 of 6 pictures equal" in said and "3 captures: frame 1 of each, and the last frame of 3" in said, said
            passed, said, code = stage(folder, plant({"new": {1: other, 29: other}}), add=True)
            assert passed and "accepted new: nothing: each capture has its accepted state" in said, said
            # The other way: an accepted picture with no capture, and one of a later frame that the capture no longer has.
            picture_diff.Image.new("RGB", (8, 8), grey).save(os.path.join(kept, "gone.png"))
            passed, said, code = stage(folder, plant({"new": {1: other, 29: other}, "b": {1: grey}}))
            assert not passed and "FAIL the accepted picture gone.png has no capture" in said and "FAIL the accepted picture b-last.png has no capture" in said, said
            assert "ok   b.nircap: frame 1 (the capture has no later frame)" in said, said
    finally:
        run_code, build_missing, check_stream_rule, stage_objects = real
        FAILS[0] = 0
        del NOT_RUN_LINES[:]


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


def replay_frames(capture, prefix, mge_build, env):
    """One replay of a capture for the gate: frame 1 into <prefix>.png and the last frame into <prefix>-last.png.
    Returns (code, output, last frame or None). A capture whose last frame is frame 1 or before has no later
    picture. Pictures of an earlier replay with this prefix are removed first: no stale picture is judged."""
    folder, name = os.path.split(prefix)
    numbered = re.compile(re.escape(name) + r"-(\d{5})\.png")
    for stale in [prefix + ".png", prefix + LAST + ".png"] + [os.path.join(folder, entry) for entry in os.listdir(folder) if numbered.fullmatch(entry)]:
        if os.path.isfile(stale):
            os.remove(stale)
    code, output = run_code([PY, os.path.join(HERE, "replay.py"), capture, prefix, "--frames-at", GATE_FRAMES, "--mge-build", mge_build], env)
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


def steady_problems(capture, name, pictures, mge_build, env):
    """Why the pictures of a capture cannot be accepted, as texts: a second replay must give each of them again,
    byte for byte. A frame that follows the clock (an arrival of distant statics) differs from run to run, and an
    accepted picture of it would fail the gate by chance."""
    again = tempfile.mkdtemp(prefix="gate-steady-")
    code, output, _last = replay_frames(capture, os.path.join(again, name), mge_build, env)
    if code != 0:
        return ["the second replay ended with the code %d" % code]
    problems = []
    for suffix in pictures:
        second = os.path.join(again, name + suffix + ".png")
        equal, said = equal_pictures(pictures[suffix], second) if os.path.isfile(second) else (False, "no picture in the second replay")
        if not equal:
            problems.append("%s is not steady: two replays differ (%s)" % ("frame 1" if suffix == "" else "the last frame", said))
    return problems


def stage_baseline(baseline, accept, mge_build, now=None, add=False):
    """The captures of the baseline folder against their accepted state, in the order of the draws of the game, as
    a player has them. Each capture is judged, and what cannot be judged is said (W526): its checks (a missing line
    is NOT RUN, a fault is a FAIL, a check that judged at the accept and judges nothing now is a FAIL); its frame 1
    and its last frame against their accepted pictures (a capture with no accepted picture is a FAIL by name, and
    so is an accepted picture with no capture). `accept` writes the accepted state of every capture again; `add`
    (--accept-new) writes only the state that a capture lacks, after the compare of what it has, and replaces
    nothing. Both accept a picture only when a second replay gives it again."""
    print("baseline %s" % baseline, flush=True)
    missing = build_missing(mge_build)
    if missing:
        return check(False, "the build folder %s has no %s: its replays would draw other pictures" % (mge_build, ", ".join(missing)))
    captures = sorted(f for f in os.listdir(baseline) if f.endswith(".nircap")) if os.path.isdir(baseline) else []
    if not captures:
        return check(False, "no capture in the baseline folder")
    if not check_stream_rule(baseline):
        return False
    env = dict(GATE_ENV)
    accepted = os.path.join(baseline, "accepted")
    out, tour_out = picture_folders(baseline, accept, now)
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
        ok &= judge_checks(capture, output, may_judge_nothing=FLOOR_CHECKS)
        pictures = {suffix: os.path.join(out, name + suffix + ".png") for suffix in ("", LAST)}
        pictures = {suffix: picture for suffix, picture in pictures.items() if os.path.isfile(picture)}
        ok &= check("" in pictures, "%s: %s" % (capture, "frame 1 and frame %d (the last)" % last if last else "frame 1 (the capture has no later frame)") if "" in pictures else "%s: the replay made no picture of frame 1" % capture)
        now_state[name] = (helper_checks(output), pictures)
    for check_name in FLOOR_CHECKS:
        if not any(checks.get(check_name, (0, 0))[0] for checks, _pictures in now_state.values()):
            ok &= not_run("no capture of the baseline judges the check \"%s\": the stage cannot show it" % check_name)
    tour = os.path.join(baseline, "tour")
    if os.path.isdir(tour) and not ran(run_code([PY, os.path.join(HERE, "validate_tour.py"), tour, "--out", tour_out, "--jobs", "4", "--mge-build", mge_build], env), "validate_tour.py", accept):
        return False
    adapter_path = os.path.join(accepted, ADAPTER_FILE)
    if accept:
        for name, (checks, pictures) in sorted(now_state.items()):
            for problem in steady_problems(os.path.join(baseline, name + ".nircap"), name, pictures, mge_build, env):
                ok &= check(False, "%s.nircap: %s: the accepted folder holds a picture that the gate cannot repeat" % (name, problem))
            write_judged(os.path.join(accepted, name + JUDGED), checks)
        if adapter:
            with open(adapter_path, "w", encoding="utf-8", newline="\n") as out_file:
                out_file.write(adapter + "\n")
        elif os.path.isfile(adapter_path):
            # No adapter printed this time: an old record would name the wrong one.
            os.remove(adapter_path)
        return check(ok, "%d captures accepted (%d pictures)%s, on %s" % (len(captures), sum(len(pictures) for _checks, pictures in now_state.values()), ", and the tour" if os.path.isdir(tour) else "", adapter or "an adapter the helper did not print"))
    recorded = open(adapter_path, encoding="utf-8").read().strip() if os.path.isfile(adapter_path) else None
    print("       " + adapter_note(recorded, adapter), flush=True)
    sys.path.insert(0, HERE)

    equal, wanted, added = 0, 0, []
    for name, (checks, pictures) in sorted(now_state.items()):
        capture = name + ".nircap"
        lacks = []
        fine = True
        for suffix, picture in sorted(pictures.items()):
            wanted += 1
            kept = os.path.join(accepted, name + suffix + ".png")
            if not os.path.isfile(kept):
                lacks.append(suffix)
                continue
            same, said = equal_pictures(kept, picture)
            equal += same
            fine &= same
            if not same:
                print("       %-40s %s" % (os.path.basename(kept), said), flush=True)
        judged = read_judged(os.path.join(accepted, name + JUDGED))
        for problem in stopped_judging(judged or {}, checks):
            fine &= check(False, "%s: %s" % (capture, problem))
        if not add:
            for suffix in lacks:
                fine &= check(False, "%s has no accepted picture of %s: the gate cannot judge it (accept it with --accept-new)" % (capture, "frame 1" if suffix == "" else "its last frame"))
            if judged is None:
                fine &= check(False, "%s has no accepted counts of its checks (%s): the gate cannot tell a check that stopped judging (accept it with --accept-new)" % (capture, name + JUDGED))
        elif fine and (lacks or judged is None):
            # Only what the capture lacks, and only when what it has is equal: nothing is replaced.
            problems = steady_problems(os.path.join(baseline, capture), name, {suffix: pictures[suffix] for suffix in lacks}, mge_build, env) if lacks else []
            for problem in problems:
                fine &= check(False, "%s: %s: not accepted" % (capture, problem))
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
        ok &= check(False, "the accepted picture %s has no capture and no frame of this run: the gate judged nothing with it" % stray)
    if add:
        print("       accepted new: %s" % (", ".join(added) or "nothing: each capture has its accepted state"), flush=True)
    ok &= check(wanted != 0 and equal == wanted and not strays, "%d of %d pictures equal to the accepted ones in %s" % (equal, wanted, out))
    print("       %d captures: frame 1 of each, and the last frame of %d" % (len(now_state), sum(LAST in pictures for _checks, pictures in now_state.values())), flush=True)
    if os.path.isdir(tour):
        kept_tour = os.path.join(tour, "accepted")
        names = sorted(f for f in os.listdir(kept_tour) if f.endswith(".png")) if os.path.isdir(kept_tour) else []
        equal = 0
        for name in names:
            if not os.path.isfile(os.path.join(tour_out, name)):
                print("       %-40s no picture now" % name, flush=True)
                continue
            same, said = equal_pictures(os.path.join(kept_tour, name), os.path.join(tour_out, name))
            equal += same
            if not same:
                print("       %-40s %s" % (name, said), flush=True)
        ok &= check(names != [] and equal == len(names), "%d of %d pictures equal to the accepted ones in %s" % (equal, len(names), tour_out))
    return stage_objects(baseline, mge_build, out) and ok


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


def stage_objects(baseline, mge_build, keep=None):
    """Each capture of the baseline folder to its last frame, plain, on each backend of
    OBJECT_BACKENDS: no steady frame (no new content, no change of the extension) may make a GPU
    object, and no device may be lost (D288). A steady frame that made objects warns while
    OBJECTS_STRICT is False. A capture with no steady frame warns; a backend with no capture that
    has a steady frame fails. The stage compares no picture, so its pictures go to a temporary
    folder; the output of each replay stays in `keep` (the folder of the gate's pictures) as
    objects-<backend>-<capture>.replay.txt."""
    captures = sorted(f for f in os.listdir(baseline) if f.endswith(".nircap")) if os.path.isdir(baseline) else []
    if not captures:
        return check(False, "objects: no capture in the baseline folder")
    out = tempfile.mkdtemp(prefix="gate-objects-")
    ok = True
    for backend in OBJECT_BACKENDS:
        steady_seen = False
        for capture in captures:
            code, output = run_code([PY, os.path.join(HERE, "replay.py"), os.path.join(baseline, capture), os.path.join(out, "%s-%s.png" % (capture[:-7], backend)), "--backend", backend, "--mge-build", mge_build], dict(GATE_ENV))
            if keep:
                keep_replay_output(os.path.join(keep, "objects-%s-%s" % (backend, capture[:-7])), code, output)
            verdict, text = objects_verdict(output)
            line = "objects, %s, %s: %s%s" % (backend, capture, text, replay_end(code, output))
            steady_seen |= verdict in ("ok", "sites")
            if code == 0 and (verdict == "idle" or (verdict == "sites" and not OBJECTS_STRICT)):
                ok &= warn(line)
            else:
                ok &= check(verdict == "ok" and code == 0, line)
        if not steady_seen:
            ok &= check(False, "objects, %s: no capture has a steady frame: the stage cannot show the rule" % backend)
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
    replay = [PY, os.path.join(HERE, "replay.py"), capture, a, "--frame", "20", "--config-root", folder, "--mge-build", mge_build]
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
    code, out = run_code([PY, os.path.join(HERE, "validate_tour.py"), folder, "--jobs", "4", "--mge-build", mge_build], {})
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
    ap.add_argument("--mge-build", default=os.path.join(REPO, "..", "MGE-XE-G7-render-link", "bin", "MSVC-Release"))
    ap.add_argument("--places", default="seyda,narsis")
    ap.add_argument("--baseline", default=os.path.join(REPO, "results", "baseline-records"), help="the folder of the baseline stage")
    ap.add_argument("--accept", action="store_true", help="the baseline stage writes the accepted pictures and compares nothing")
    ap.add_argument("--accept-new", action="store_true", help="the baseline stage compares, and then accepts only what a capture lacks (a picture of frame 1 or of its last frame, "
                    "the counts of its checks); it replaces nothing")
    ap.add_argument("--now", default=None, help="the folder of the baseline stage's pictures, outside the baseline folder; without it, a run folder of its own in results")
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
    problem = now_problem(args.baseline, args.accept, args.now)
    if problem:
        print("  FAIL " + problem, flush=True)
        return 1
    if "baseline" in args.stages and not args.accept and not args.now:
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
                if not stage_capture(place, args.mge_build):
                    return 1
        elif stage == "baseline":
            if not stage_baseline(args.baseline, args.accept, args.mge_build, args.now, args.accept_new):
                return baseline_failure_code()
        elif stage == "only":
            for place in places:
                if not stage_only(place, args.mge_build):
                    return 1
        elif stage == "tour":
            if not stage_tour(args.mge_build):
                return 1
        elif stage == "clusters":
            for place in places:
                if not stage_clusters(place, args.mge_build, started):
                    return 1
        elif stage == "weathers":
            for place in places:
                if not stage_weathers(place, args.mge_build):
                    return 1
        else:
            print("unknown stage " + stage)
            return 2
    print("all stages passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
