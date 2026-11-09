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
           accepted pictures (`accepted/adapter.txt`) and printed with the result; a
           compare on another adapter says so, since the pictures can differ by the GPU.
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
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
sys.path.insert(0, HERE)
from common import BUILD_PARTS, build_missing, plugin_lines  # noqa: E402
from validate_tour import NOT_RUN, refusal_of  # noqa: E402
import stream_rule  # noqa: E402
PY = sys.executable
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


def run(args, env=None):
    """The output of a program."""
    return run_code(args, env)[1]


def newest(label, since=None):
    """The newest run folder of a label. With `since` (a time in RUN_TIME's form), only a folder of
    that time or later."""
    runs = sorted((d for d in os.listdir(os.path.join(REPO, "results")) if d.endswith("-" + label) and (since is None or d[:15] >= since)), reverse=True)
    return os.path.join(REPO, "results", runs[0]) if runs else None


GPU_TIME = {}
RUNNER_ARGS = []
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
    if not ran(run_code(args, e), "the runner of " + label):
        return None
    folder = newest(label, started)
    if not folder:
        check(False, "the runner of %s made no folder since %s" % (label, started))
    return folder


def host_counts(folder):
    text = open(os.path.join(folder, "host.txt"), errors="replace").read()
    lines = {k: "" for k in ("geometry", "material", "shapes")}
    for line in text.splitlines():
        for key in lines:
            if line.startswith(key + ":"):
                lines[key] = line
    return lines


def check(condition, text):
    print(("  ok   " if condition else "  FAIL ") + text, flush=True)
    return condition


def ran(result, tool, accepting=False):
    """True when a tool of this folder ended with the code 0. `result` comes from `run_code`. Else
    its last lines go to the console and the stage fails: a failed tool would only show later as
    pictures that are missing, or in accept mode as pictures accepted. An exception, a helper
    that died and a tour stop with no picture all end with another code. In accept mode the
    pictures made before the stop are in the accepted folder already. A replay that the slot
    check refused checked no picture: it is NOT RUN, not a failed picture."""
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
    return check(False, "%s ended with the code %d%s" % (tool, code, ": the accepted folder holds a mix now, accept again" if accepting else ""))


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
    ok &= ran((code, out), "replay.py")
    # Each GPU error prints a line "gpu error N (kind, adapter ...)" (host/src/gpu.rs).
    errors = len(re.findall(r"^gpu error \d+ \(", out, re.M))
    ok &= check(os.path.isfile(a) and errors == 0, "replay of the capture: %s, %d gpu errors" % ("a picture" if os.path.isfile(a) else "no picture", errors))
    early = early_world_scenes(out)
    ok &= check(early in (None, 0), "replay of the capture: %s scenes with draws of their world in the target before the extension's parts" % ("no" if early is None else early))
    return ok


ADAPTER_FILE = "adapter.txt"


EARLY_LINE = re.compile(r"^early world draws: (\d+) starts of a world for the extension checked, (\d+) scenes with draws of their world already in the target, (\d+) draws", re.M)


def early_world_scenes(output):
    """The scenes with draws of their world already in the target when the extension's parts
    began, from the report line `early world draws: ...` of the helper (`Player::report`); None
    when the output has no such line (a replay with no start of a world to check). Each is a
    fault of the order of the draws: the gate wants 0."""
    match = EARLY_LINE.search(output)
    return int(match.group(2)) if match else None


def adapter_of(output):
    """The adapter line of the helper's output ("adapter: <GPU> (<backend>, driver ...)"),
    without the prefix; None when the output has none."""
    match = re.search(r"^adapter: (.+?)\s*$", output, re.MULTILINE)
    return match.group(1) if match else None


def adapter_note(recorded, now):
    """What to say about the adapter of a compare: the one of the run, and whether the
    accepted pictures were made on it."""
    if now is None:
        return "adapter: not printed by the helper"
    if recorded is None:
        return "adapter: %s; the accepted pictures name none (accept again to record it)" % now
    if recorded != now:
        return "adapter: %s; the accepted pictures were made on %s, so a difference can be the GPU's" % (now, recorded)
    return "adapter: %s, as the accepted pictures" % now


def self_test():
    """The tests of the pure parts: no game, no replay."""
    global REPO, run_code
    output = "distant land: on\nadapter: NVIDIA GeForce RTX 5070 (Vulkan, driver NVIDIA 617.14)\nframe 1\n"
    assert adapter_of(output) == "NVIDIA GeForce RTX 5070 (Vulkan, driver NVIDIA 617.14)"
    assert adapter_of("no such line\n") is None
    # The count of scenes with early world draws; no line means no start of a world was checked.
    line = "early world draws: 12 starts of a world for the extension checked, %d scenes with draws of their world already in the target, 7 draws\n"
    assert early_world_scenes("frame 1\n" + line % 0 + "retained: 3\n") == 0
    assert early_world_scenes(line % 2) == 2
    assert early_world_scenes("frame 1\n") is None
    same = "NVIDIA GeForce RTX 5070 (Vulkan, driver NVIDIA 617.14)"
    assert adapter_note(same, same) == "adapter: %s, as the accepted pictures" % same
    assert adapter_note(None, same).endswith("(accept again to record it)")
    assert "can be the GPU's" in adapter_note("AMD Radeon (Vulkan, driver 1)", same)
    assert adapter_note(same, None) == "adapter: not printed by the helper"
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
            # The clusters stage reads the capture of an earlier stage, so without a time the newest
            # run counts.
            assert newest("v-test") == new and newest("v-test", "99991231-000000") is None
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
    print("validate_retained self-test: ok")


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


def stage_baseline(baseline, accept, mge_build, now=None):
    """The captures of the baseline folder against their accepted pictures, in the order of
    the draws of the game, as a player has them."""
    print("baseline %s" % baseline, flush=True)
    missing = build_missing(mge_build)
    if missing:
        return check(False, "the build folder %s has no %s: its replays would draw other pictures" % (mge_build, ", ".join(missing)))
    captures = sorted(f for f in os.listdir(baseline) if f.endswith(".nircap")) if os.path.isdir(baseline) else []
    if not captures:
        return check(False, "no capture in the baseline folder")
    if not check_stream_rule(baseline):
        return False
    env = {}
    accepted = os.path.join(baseline, "accepted")
    out, tour_out = picture_folders(baseline, accept, now)
    os.makedirs(out, exist_ok=True)
    adapter = None
    early_captures = []
    for capture in captures:
        code, output = run_code([PY, os.path.join(HERE, "replay.py"), os.path.join(baseline, capture), os.path.join(out, capture[:-7] + ".png"), "--frame", "1", "--mge-build", mge_build], env)
        if not ran((code, output), "replay.py", accept):
            return False
        adapter = adapter or adapter_of(output)
        if early_world_scenes(output):
            early_captures.append(capture)
    tour = os.path.join(baseline, "tour")
    if os.path.isdir(tour) and not ran(run_code([PY, os.path.join(HERE, "validate_tour.py"), tour, "--out", tour_out, "--jobs", "4", "--mge-build", mge_build], env), "validate_tour.py", accept):
        return False
    adapter_path = os.path.join(accepted, ADAPTER_FILE)
    if accept:
        if adapter:
            with open(adapter_path, "w", encoding="utf-8", newline="\n") as out_file:
                out_file.write(adapter + "\n")
        elif os.path.isfile(adapter_path):
            # No adapter printed this time: an old record would name the wrong one.
            os.remove(adapter_path)
        return check(True, "%d pictures accepted%s, on %s" % (len(captures), ", and the tour" if os.path.isdir(tour) else "", adapter or "an adapter the helper did not print"))
    recorded = open(adapter_path, encoding="utf-8").read().strip() if os.path.isfile(adapter_path) else None
    print("       " + adapter_note(recorded, adapter), flush=True)
    sys.path.insert(0, HERE)
    import picture_diff

    ok = check(not early_captures, "no replay had draws of its world in the target before the extension's parts" if not early_captures else "replays with draws of their world in the target before the extension's parts: %s" % ", ".join(early_captures))
    for a, b in [(accepted, out)] + ([(os.path.join(tour, "accepted"), tour_out)] if os.path.isdir(tour) else []):
        names = sorted(f for f in os.listdir(a) if f.endswith(".png")) if os.path.isdir(a) else []
        equal = 0
        for name in names:
            if not os.path.isfile(os.path.join(b, name)):
                print("       %-40s no picture now" % name, flush=True)
                continue
            numbers = picture_diff.compare(picture_diff.load(os.path.join(a, name)), picture_diff.load(os.path.join(b, name)))
            if "problem" not in numbers and numbers["different"] == 0:
                equal += 1
            else:
                print("       %-40s %s" % (name, numbers.get("problem") or "%.2f %% different, mean %.3f, largest %d" % (numbers["different"] * 100, numbers["mean"], numbers.get("largest", 0))), flush=True)
        ok &= check(names != [] and equal == len(names), "%d of %d pictures equal to the accepted ones in %s" % (equal, len(names), b))
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


def stage_clusters(place, mge_build):
    print("clusters at %s" % place, flush=True)
    folder = newest("v-capture-" + place)
    capture = folder and os.path.join(folder, "exterior.nircap")
    if not capture or not os.path.isfile(capture):
        return check(False, "no capture of the place: run the capture stage first")
    a, b = os.path.join(folder, "replay-clusters.png"), os.path.join(folder, "replay-draw-lights.png")
    # The replays read the mgeXE.toml of the run (the fade of the lights is on in it).
    # The replays use the build under test: the default of replay.py is the folder of
    # `cargo xtask build`, which can hold an older helper.
    out = run([PY, os.path.join(HERE, "replay.py"), capture, a, "--frame", "20", "--config-root", folder, "--mge-build", mge_build], {"MGE_XE_CLUSTERS": "1"})
    run([PY, os.path.join(HERE, "replay.py"), capture, b, "--frame", "20", "--config-root", folder, "--mge-build", mge_build], {"MGE_XE_CLUSTERS": "0"})
    sys.path.insert(0, HERE)
    import picture_diff

    used = re.search(r"clusters: ([\d.]+) lights and (\d+) entries for each frame", out)
    ok = check(used is not None and used.group(2) != "0", used.group(0) if used else "no clusters line in the replay")
    numbers = picture_diff.compare(picture_diff.load(a), picture_diff.load(b))
    text = "replay with the clusters against the lights of the draws: %s" % (numbers.get("problem") or "%.2f %% different, %.2f %% by more than 8, mean %.3f, largest %d" % (
        numbers["different"] * 100, numbers.get("over8", 0) * 100, numbers["mean"], numbers.get("largest", 0)))
    ok &= check("problem" not in numbers and numbers["mean"] < 0.5, text)
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
    if not ran((code, out), "validate_tour.py"):
        return False
    tail = out.strip().split("\n")
    for line in tail[-45:]:
        print("       " + line[:160], flush=True)
    pairs = re.search(r"pairs: (\d+) of (\d+) equal", out)
    return ok & check(pairs is not None and pairs.group(2) != "0", pairs.group(0) if pairs else "no pairs compared")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("stages", nargs="*", default=["capture", "baseline", "only"])
    ap.add_argument("--mge-build", default=os.path.join(REPO, "..", "MGE-XE-G7-render-link", "bin", "MSVC-Release"))
    ap.add_argument("--places", default="seyda,narsis")
    ap.add_argument("--baseline", default=os.path.join(REPO, "results", "baseline-records"), help="the folder of the baseline stage")
    ap.add_argument("--accept", action="store_true", help="the baseline stage writes the accepted pictures and compares nothing")
    ap.add_argument("--now", default=None, help="the folder of the baseline stage's pictures, outside the baseline folder; without it, a run folder of its own in results")
    ap.add_argument("--gpu-time", action="store_true", help="the helper writes GPU time stamps in the only stage")
    ap.add_argument("--disable-mod", default=None, help="a mod of the profile switched off for the runs")
    ap.add_argument("--weather", default="0", help="the weather of the runs, pinned and held: 0 clear (the default) to 9 blizzard; any: the game's, not pinned")
    ap.add_argument("--self-test", action="store_true", help="the tests of the pure parts: no game, no replay")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
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
    for stage in args.stages:
        if stage == "capture":
            for place in places:
                if not stage_capture(place, args.mge_build):
                    return 1
        elif stage == "baseline":
            if not stage_baseline(args.baseline, args.accept, args.mge_build, args.now):
                return 1
        elif stage == "only":
            for place in places:
                if not stage_only(place, args.mge_build):
                    return 1
        elif stage == "tour":
            if not stage_tour(args.mge_build):
                return 1
        elif stage == "clusters":
            for place in places:
                if not stage_clusters(place, args.mge_build):
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
