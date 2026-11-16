"""Replays the stops of a tour of a test level and compares the pictures with a baseline.

    python tools/validate_tour.py <run dir> [--against <baseline dir>] [--out <dir>] [--jobs N]
                                  [--mge-build DIR]

The run comes from `run_own_device.py --steps tour --tour-cells "@nifTest_1;@nifTest_2;@nifTest_3"`:
its folder has `tour-<cell>.nircap` with two frames for each stop, and `progress.log` names
the stops. This replays the second frame of each stop into `<out>/<cell>-<stop>-<id>.png`
(`<out>` is the run folder without `--out`). The full output of each replay stays there too:
`tour-frames-<capture>.replay.txt` for the one run of a capture, `<picture>.replay.txt` for a
second try of one picture, each with the exit code first. With `--against`, it compares each picture with
the one of the same name in the baseline folder and prints the mean difference, with the
largest ones first. Make a baseline by replaying a run and copying its folder.

A run made with `--tour-dual` has four frames for each stop: two in mode gpu, then two in
mode off, with the time of the game held. Then this replays both (the off picture is
`<cell>-<stop>-<id>-off.png`) and compares each pair: the two paths at one moment.

A replay that the slot check refuses (a replay needs the whole build side) made no picture, and
no picture of it is checked: it is "NOT RUN: slot refused", not a failed picture, and it gets
no second try. Its pairs are not compared. The exit code is 1 when a picture failed, else 3 when
a replay was not run, else 0.

A replay that ended with a fault code of its own (the stage shaders' 40, W523; the checks' 41, W526)
made its pictures. The pictures are compared, the fault has a "FAILED replays" line with replay.py's
line of each such replay, and the exit code is 1.

The tool gives the verdict itself (W526), in its last line "tour verdict: PASS", "FAIL: ..." or
"NOT RUN: ...", and in its exit code. It fails for a picture that failed, for a replay with a fault
code, for a pair of mode gpu against mode off with a mean over MEAN_LIMIT or with no numbers, and for
fewer stops than --least-stops. It is NOT RUN (exit code 3) for a replay that the slot check refused,
for a replay whose output lacks a check line of the helper, and for a dual run with no pair to
compare. A caller reads the exit code, not the table.

    python tools/validate_tour.py --self-test   # a planted refusal and a planted fault, with no replay
"""
import argparse
import os
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from common import EARLY_WORLD_DRAWS, OWN_VIEWS, check_problems, keep_replay_output, replay_fault_in  # noqa: E402

PYTHON = sys.executable
# The start of the refusal of slot.py's check of build work (`refuse_build_tool`), which replay.py calls.
REFUSED = "Refused by the slot check"
NOT_RUN = "NOT RUN: slot refused"
NOT_RUN_CODE = 3
# The start of the tool's last line, which has its verdict (W526): PASS, "FAIL: ..." or "NOT RUN: ...".
VERDICT = "tour verdict: "
# The runner of replay.py: the self-test replaces it.
run_tool = subprocess.run
# replay.py's line of each replay of this process that ended with a fault code of its own (the stage shaders, the
# checks of the helper).
STAGE_FAULTS = []
# The replays of this process whose output lacks a check line of the helper, as texts: not judged.
CHECK_GAPS = []
# The largest mean difference of a pair of mode gpu against mode off that passes. f185 had a largest mean of 0.001 at
# the tour's 103 stops; the tips of 2026-11-16 have 0.033 at one stop and under 0.006 at the others. A change of the
# picture between the two modes that is larger than this is no noise. [I] The value is a judgement, with room for
# small differences. The gate's accepted pictures are the exact test: this is a second net. Its one home is here
# (make_play_set.py had it before W526).
MEAN_LIMIT = 0.05


def stops_of(run):
    """{cell index: [(stop index, object id)]} from the progress log, and whether the run
    is dual (two modes at each stop)."""
    stops = {}
    dual = False
    pattern = re.compile(r"tour: cell (\d+) stop (\d+) (\S+)")
    for line in open(os.path.join(run, "progress.log"), encoding="utf-8", errors="replace"):
        if "tour: dual" in line:
            dual = True
        found = pattern.search(line)
        if found:
            stops.setdefault(int(found.group(1)), []).append((int(found.group(2)), found.group(3)))
    return stops, dual


# replay.py's arguments that name the build of the replays (W545): ["--mge-build", <set>], ["--archive", <folder>] of
# release.py's gate, or none (replay.py then takes the current set). main sets them.
BUILD_ARGS = []


def build_args(mge_build, archive):
    """The arguments of replay.py for --mge-build or --archive. Raises ValueError for both."""
    if mge_build and archive:
        raise ValueError("an archive is not a set: no --mge-build with --archive")
    if archive:
        return ["--archive", archive]
    return ["--mge-build", mge_build] if mge_build else []


def replay(capture, frame, picture):
    command = [PYTHON, os.path.join(HERE, "replay.py"), capture, picture, "--frame", str(frame)]
    # The helper of the build under test, not the default one of replay.py.
    command += BUILD_ARGS
    result = run_tool(command, capture_output=True, text=True)
    keep_replay_output(picture, result.returncode, result.stdout + result.stderr)
    return result.returncode == 0 and os.path.isfile(picture), (result.stdout + result.stderr)[-400:]


def refusal_of(output):
    """The line of the slot check's refusal in the output of replay.py, or ""."""
    return next((line.strip() for line in output.splitlines() if REFUSED in line), "")


def replay_many(capture, items):
    """One run of the helper for all the frames of a capture: `items` are (frame, picture).
    Returns the names of the pictures that were not made, and the refusal of the slot check
    ("" when the replay ran)."""
    prefix = os.path.join(os.path.dirname(items[0][1]), "tour-frames-%s" % os.path.basename(capture)[:-7])
    command = [PYTHON, os.path.join(HERE, "replay.py"), capture, prefix, "--frames-at", ",".join(str(f) for f, _ in items)]
    command += BUILD_ARGS
    result = run_tool(command, capture_output=True, text=True)
    output = result.stdout + result.stderr
    keep_replay_output(prefix, result.returncode, output)
    refused = refusal_of(output)
    fault = replay_fault_in(output)
    if fault:
        STAGE_FAULTS.append(fault)
    if not refused and result.returncode == 0:
        # A replay that ended in order with no line of a check, or with a check that judged nothing: not judged (a
        # fault of a check has its exit code, and a replay that failed has its own line). A cell of the test level
        # can have no start of a world for the extension (an interior) and no view of its own.
        gaps = check_problems(output, may_judge_nothing=(EARLY_WORLD_DRAWS, OWN_VIEWS))[0]
        if gaps:
            CHECK_GAPS.append("replay of %s: %s" % (os.path.basename(capture), "; ".join(gaps)))
    missing = []
    for frame, picture in items:
        made = "%s-%05d.png" % (prefix, frame)
        if os.path.isfile(made):
            if os.path.isfile(picture):
                os.remove(picture)
            os.replace(made, picture)
        else:
            missing.append(os.path.basename(picture))
    if refused:
        print("%s: replay of %s: %d pictures not made: %s" % (NOT_RUN, os.path.basename(capture), len(missing), refused))
    elif missing:
        print("replay of %s: %d pictures missing: %s" % (os.path.basename(capture), len(missing), output[-300:].strip()))
    return missing, refused


def replay_all(work, jobs):
    """Replays `work`, (capture, frame, picture, name), with one run of the helper for each capture, and a
    second try alone for each picture that the first did not make. Returns the names of the failed pictures
    and of those not run: a refused replay is not tried again, as the check would refuse it again."""
    by_capture = {}
    for capture, frame, picture, name in work:
        by_capture.setdefault(capture, []).append((frame, picture))
    failures, not_run = [], []
    # The captures run at once, up to `jobs` (2026-10-16: 206 runs of the helper became 3).
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        for missing, refused in pool.map(lambda kv: replay_many(kv[0], kv[1]), by_capture.items()):
            (not_run if refused else failures).extend(missing)
    for capture, frame, picture, name in work:
        if name not in failures:
            continue
        # Once more, alone, one frame.
        ok, text = replay(capture, frame, picture)
        failures.remove(name)
        if ok:
            continue
        if refusal_of(text):
            print("%s: %s: %s" % (NOT_RUN, name, refusal_of(text)))
            not_run.append(name)
        else:
            print("FAILED %s: %s" % (name, text.strip()[-200:]))
            failures.append(name)
    return failures, not_run


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("run")
    ap.add_argument("--against")
    ap.add_argument("--out")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--mge-build", default=None, help="the sealed set whose helper replays (W545; replay.py's default, the current set, without it)")
    ap.add_argument("--archive", default=None, help="the folder of an unpacked release archive whose helper replays: the release check")
    ap.add_argument("--least-stops", type=int, default=0, help="the tour fails with fewer stops than this (the play set: a much shorter tour did not run through)")
    args = ap.parse_args()
    try:
        BUILD_ARGS[:] = build_args(args.mge_build, args.archive)
    except ValueError as problem:
        ap.error(str(problem))
    del STAGE_FAULTS[:], CHECK_GAPS[:]
    out = args.out or args.run
    os.makedirs(out, exist_ok=True)
    stops, dual = stops_of(args.run)
    work = []
    pairs = []
    for cell, cell_stops in sorted(stops.items()):
        capture = os.path.join(args.run, "tour-%d.nircap" % cell)
        if not os.path.isfile(capture):
            print("no capture for cell %d" % cell)
            continue
        for stop, ident in cell_stops:
            name = "%d-%02d-%s.png" % (cell, stop, re.sub(r"[^A-Za-z0-9_]", "_", ident))
            if dual:
                # Four frames for each stop: the second of mode gpu and the second of mode
                # off. The replay counts the frames from 0.
                off = name[:-4] + "-off.png"
                work.append((capture, (stop - 1) * 4 + 1, os.path.join(out, name), name))
                work.append((capture, (stop - 1) * 4 + 3, os.path.join(out, off), off))
                pairs.append((name, off))
            else:
                # Two frames for each stop: the second one.
                work.append((capture, stop * 2 - 1, os.path.join(out, name), name))
    print("%d stops to replay%s" % (len(work) // (2 if dual else 1), ", gpu and off at each" if dual else ""))
    failures, not_run = replay_all(work, args.jobs)
    print("%d pictures, %d failed" % (len(work) - len(failures) - len(not_run), len(failures)))
    if not_run:
        print("%s: %d pictures not made, and not checked" % (NOT_RUN, len(not_run)))
    stage_faults = list(STAGE_FAULTS)
    if stage_faults:
        print("FAILED replays: %d replays ended with a fault code of their own:" % len(stage_faults))
        for fault in stage_faults:
            print("  " + fault)
    # What fails the tour, and what it could not judge, each as a text for the verdict.
    stops_count = len(work) // (2 if dual else 1)
    fails = (["%d pictures failed" % len(failures)] if failures else []) + (["%d replays ended with a fault code" % len(stage_faults)] if stage_faults else [])
    if stops_count < args.least_stops:
        fails.append("the tour has %d stops, fewer than the %d of --least-stops" % (stops_count, args.least_stops))
    unjudged = (["%d pictures of replays that the slot check refused" % len(not_run)] if not_run else []) + list(CHECK_GAPS)
    import picture_diff

    if pairs:
        rows = []
        unrun = 0
        for gpu, off in pairs:
            if gpu in not_run or off in not_run:
                unrun += 1
                continue
            a, b = os.path.join(out, gpu), os.path.join(out, off)
            if not (os.path.isfile(a) and os.path.isfile(b)):
                rows.append((float("inf"), gpu, "missing"))
                continue
            numbers = picture_diff.compare(picture_diff.load(a), picture_diff.load(b))
            if "problem" in numbers:
                rows.append((float("inf"), gpu, numbers["problem"]))
            else:
                rows.append((numbers["mean"], gpu, "%.3f mean, %.2f %% different, %.2f %% over 8, largest %d" % (numbers["mean"], numbers["different"] * 100, numbers["over8"] * 100, numbers.get("largest", 0))))
        rows.sort(reverse=True)
        print("mode gpu against mode off at each stop, the largest differences first:")
        for mean, name, text in rows[:40]:
            print("  %-48s %s" % (name, text))
        equal = sum(1 for mean, _, _ in rows if mean == 0)
        print("pairs: %d of %d equal%s" % (equal, len(rows), "; %d pairs %s, not compared" % (unrun, NOT_RUN) if unrun else ""))
        # The verdict on the pairs: before W526 the tool printed the table and ended with 0 whatever it held.
        unmeasured = [name for mean, name, _ in rows if mean == float("inf")]
        over = [(mean, name) for mean, name, _ in rows if MEAN_LIMIT < mean < float("inf")]
        if unmeasured:
            fails.append("%d pairs have no numbers, first %s" % (len(unmeasured), unmeasured[-1]))
        if over:
            fails.append("%d pairs of mode gpu against mode off differ by more than the limit %.3f, the largest %.3f at %s" % ((len(over), MEAN_LIMIT) + max(over)))
        if not rows and not unrun:
            unjudged.append("the dual run has no pair of mode gpu against mode off to compare")
    elif dual:
        unjudged.append("the dual run has no stop, so no pair of mode gpu against mode off to compare")
    if not args.against:
        return verdict(fails, unjudged)

    rows = []
    import glob
    for _, _, picture, name in work:
        if name.endswith("-off.png") or name in not_run:
            continue
        other = os.path.join(args.against, name)
        if not os.path.isfile(other):
            # The same stop with another spelling of the id (a reference run names it in Lua).
            candidates = glob.glob(os.path.join(args.against, name[:name.index("-", 2) + 1] + "*.png"))
            other = candidates[0] if len(candidates) == 1 else other
        if not (os.path.isfile(picture) and os.path.isfile(other)):
            rows.append((float("inf"), name, "missing"))
            continue
        numbers = picture_diff.compare(picture_diff.load(picture), picture_diff.load(other))
        if "problem" in numbers:
            rows.append((float("inf"), name, numbers["problem"]))
        else:
            rows.append((numbers["mean"], name, "%.2f mean, %.1f %% different, %.1f %% over 8, max %d" % (numbers["mean"], numbers["different"] * 100, numbers["over8"] * 100, numbers.get("max", 0))))
    rows.sort(reverse=True)
    print("against %s, the largest differences first:" % args.against)
    for mean, name, text in rows[:30]:
        print("  %-48s %s" % (name, text))
    equal = sum(1 for mean, _, _ in rows if mean == 0)
    print("%d of %d equal" % (equal, len(rows)))
    return verdict(fails, unjudged)


def verdict_in(output):
    """The verdict line of this tool in its output, or ""."""
    return next((line.strip() for line in reversed(output.splitlines()) if line.startswith(VERDICT)), "")


def verdict(fails, unjudged):
    """Prints the last line of the tool and returns its exit code: 1 for a thing that failed, else NOT_RUN_CODE for a
    thing that the tool could not judge, else 0. A failure comes first: a NOT RUN must not hide it."""
    if fails:
        print(VERDICT + "FAIL: " + "; ".join(fails + unjudged))
        return 1
    if unjudged:
        print(VERDICT + "NOT RUN: " + "; ".join(unjudged))
        return NOT_RUN_CODE
    print(VERDICT + "PASS")
    return 0


def self_test():
    """A planted refusal of the slot check: the replays of every capture are refused. No picture may count as
    failed, none may get a second try, and each must count as not run. Then a replay that ran and made no
    picture: a failure, after its second try."""
    import contextlib
    import io
    import tempfile

    refusal = "Refused by the slot check: replay.py: the build slot is held by rob: caps helper compile (place 1); a replay needs the whole build side."
    calls = []

    def refused(command, **_):
        calls.append(command)
        return subprocess.CompletedProcess(command, 1, "", refusal + "\nTo pass anyway: SLOT_OVERRIDE=...\n")

    def ran_without_picture(command, **_):
        calls.append(command)
        return subprocess.CompletedProcess(command, 1, "gpu error: lost the device\n", "")

    global run_tool
    with tempfile.TemporaryDirectory() as folder:
        work = [(os.path.join(folder, "tour-%d.nircap" % cell), frame, os.path.join(folder, "%d-%02d.png" % (cell, frame)), "%d-%02d.png" % (cell, frame))
                for cell in (1, 2) for frame in (1, 3)]
        printed = io.StringIO()
        try:
            run_tool = refused
            with contextlib.redirect_stdout(printed):
                failures, not_run = replay_all(work, 2)
            assert failures == [], failures
            assert sorted(not_run) == sorted(name for *_, name in work), not_run
            assert len(calls) == 2, "one run for each capture, and no second try: %d runs" % len(calls)
            assert printed.getvalue().count(NOT_RUN + ": replay of tour-") == 2 and "FAILED" not in printed.getvalue(), printed.getvalue()

            calls.clear()
            run_tool = ran_without_picture
            with contextlib.redirect_stdout(io.StringIO()):
                failures, not_run = replay_all(work[:2], 2)
            assert sorted(failures) == sorted(name for *_, name in work[:2]) and not_run == [], (failures, not_run)
            assert len(calls) == 3, "one run of the capture, and a second try of each picture: %d runs" % len(calls)

            # A first run with no picture, then a refused second try (the hold ended between them): not run.
            def refused_after_first(command, **kind):
                return (ran_without_picture if len(calls) == 0 else refused)(command, **kind)

            calls.clear()
            run_tool = refused_after_first
            with contextlib.redirect_stdout(io.StringIO()) as printed:
                failures, not_run = replay_all(work[:2], 2)
            assert failures == [] and sorted(not_run) == sorted(name for *_, name in work[:2]), (failures, not_run)
            assert "FAILED" not in printed.getvalue(), printed.getvalue()

            # main(): a failed picture wins over a refused replay (exit code 1), and with only refused
            # replays the exit code is NOT_RUN_CODE, with no failed picture.
            run = os.path.join(folder, "run")
            os.makedirs(run)
            with open(os.path.join(run, "progress.log"), "w", encoding="utf-8") as file:
                file.write("tour: dual\ntour: cell 1 stop 1 a\ntour: cell 2 stop 1 b\n")
            for cell in (1, 2):
                open(os.path.join(run, "tour-%d.nircap" % cell), "wb").close()

            def first_refused(command, **kind):
                return (refused if "tour-1" in os.path.basename(command[2]) else ran_without_picture)(command, **kind)

            real_argv = sys.argv
            try:
                sys.argv = ["validate_tour.py", run, "--jobs", "1"]
                for tool, code, text, said in ((first_refused, 1, "0 pictures, 2 failed", "tour verdict: FAIL: 2 pictures failed; 1 pairs have no numbers, first 2-01-b.png; 2 pictures of replays that the slot check refused"),
                                               (refused, NOT_RUN_CODE, "0 pictures, 0 failed", "tour verdict: NOT RUN: 4 pictures of replays that the slot check refused")):
                    run_tool = tool
                    with contextlib.redirect_stdout(io.StringIO()) as printed:
                        assert main() == code, (tool.__name__, printed.getvalue())
                    assert text in printed.getvalue() and NOT_RUN in printed.getvalue(), printed.getvalue()
                    assert printed.getvalue().splitlines()[-1] == said, printed.getvalue()
                    assert STAGE_FAULTS == [] and CHECK_GAPS == [] and "FAILED replays" not in printed.getvalue(), printed.getvalue()

                # W523: a replay that made each picture and ended with the fault of the stage shaders (exit code 40).
                # No picture failed, the pairs are compared, and the tour fails with the line of each replay.
                # W526: the tool gives its verdict by its last line and its exit code.
                import shutil

                import common
                import picture_diff

                fault = "STAGE SHADERS: the replay of %s failed with exit code 40: 2 rewrites failed, 0 pipelines failed, 9 draws on the uber-shader with a stage key, 0 draws past the cap of the keys."
                checks = "".join("check %s: 7 judged, 0 faults\n" % name for name in common.CHECK_NAMES)
                pictures = {}
                for colour, size in (((10, 20, 30), 4), ((250, 0, 250), 4), ((10, 20, 30), 6)):
                    pictures[colour, size] = os.path.join(folder, "plant-%d-%d.png" % (colour[0], size))
                    picture_diff.Image.new("RGB", (size, size), colour).save(pictures[colour, size])

                def made(code=0, output=checks, last="", off=((10, 20, 30), 4)):
                    """A replay that makes each picture: the frames of mode gpu grey, those of mode off as `off`."""
                    def tool(command, **_):
                        capture, prefix = command[2], command[3]
                        for frame in command[command.index("--frames-at") + 1].split(","):
                            shutil.copyfile(pictures[off if int(frame) % 4 == 3 else ((10, 20, 30), 4)], "%s-%05d.png" % (prefix, int(frame)))
                        return subprocess.CompletedProcess(command, code, output, last % os.path.basename(capture) + "\n" if last else "")
                    return tool

                with_fault = made(40, checks + "stage shaders: a fault of the replay: 2 rewrites failed\n", fault)
                no_lines = "; ".join("the helper's output has no line of the check \"%s\"" % name for name in common.CHECK_NAMES)
                gap = "tour verdict: NOT RUN: replay of tour-1.nircap: %s; replay of tour-2.nircap: %s" % (no_lines, no_lines)
                idle = checks.replace("check gpu errors: 7 judged", "check gpu errors: 0 judged").replace("check views of their own: 7 judged", "check views of their own: 0 judged")
                for name, tool, arguments, code, faults, pairs, said in (
                        ("a fault code", with_fault, [], 1, 2, "2 of 2", "tour verdict: FAIL: 2 replays ended with a fault code"),
                        ("clean", made(), [], 0, 0, "2 of 2", "tour verdict: PASS"),
                        # A replay that ended in order with no check line of the helper: not judged, never a pass.
                        ("no check lines", made(output=""), [], NOT_RUN_CODE, 0, "2 of 2", gap),
                        # A check that judged nothing is not judged; a cell with no view of its own may have 0 places.
                        ("a check judged nothing", made(output=idle), [], NOT_RUN_CODE, 0, "2 of 2",
                         'tour verdict: NOT RUN: replay of tour-1.nircap: the check "gpu errors" judged nothing; replay of tour-2.nircap: the check "gpu errors" judged nothing'),
                        # Rob's plant of F-G-3: mode off differs from mode gpu at every stop. Before W526 the tool ended with 0.
                        ("pairs differ", made(off=((250, 0, 250), 4)), [], 1, 0, "0 of 2", "tour verdict: FAIL: 2 pairs of mode gpu against mode off differ by more than the limit 0.050, the largest 160.000 at 2-01-b.png"),
                        ("no numbers", made(off=((10, 20, 30), 6)), [], 1, 0, "0 of 2", "tour verdict: FAIL: 2 pairs have no numbers, first 1-01-a.png"),
                        ("too few stops", made(), ["--least-stops", "100"], 1, 0, "2 of 2", "tour verdict: FAIL: the tour has 2 stops, fewer than the 100 of --least-stops"),
                        ("enough stops", made(), ["--least-stops", "2"], 0, 0, "2 of 2", "tour verdict: PASS")):
                    run_tool = tool
                    sys.argv = ["validate_tour.py", run, "--jobs", "1"] + arguments
                    with contextlib.redirect_stdout(io.StringIO()) as printed:
                        assert main() == code, (name, printed.getvalue())
                    text = printed.getvalue()
                    assert "4 pictures, 0 failed" in text and "pairs: %s equal" % pairs in text and text.splitlines()[-1] == said, (name, text)
                    kept = os.path.join(run, "tour-frames-tour-1.replay.txt")
                    assert open(kept, encoding="utf-8").read().startswith("exit code %d\n" % (40 if faults else 0)), "the output of the replay is kept beside the pictures"
                    assert text.count("  STAGE SHADERS: the replay of tour-") == faults and ("FAILED replays: 2 replays" in text) == bool(faults), (name, text)
                # A dual run with no stop has no pair to compare: not judged.
                with open(os.path.join(run, "progress.log"), "w", encoding="utf-8") as file:
                    file.write("tour: dual\n")
                sys.argv = ["validate_tour.py", run, "--jobs", "1"]
                with contextlib.redirect_stdout(io.StringIO()) as printed:
                    assert main() == NOT_RUN_CODE, printed.getvalue()
                assert printed.getvalue().splitlines()[-1] == "tour verdict: NOT RUN: the dual run has no stop, so no pair of mode gpu against mode off to compare", printed.getvalue()
                assert MEAN_LIMIT == 0.05
            finally:
                sys.argv = real_argv
                del STAGE_FAULTS[:], CHECK_GAPS[:]
        finally:
            run_tool = subprocess.run
    assert refusal_of("a\n" + refusal + "\nb") == refusal and refusal_of("replay of x: 3 pictures missing") == ""
    # W545: the build goes to replay.py as --archive for an archive, as --mge-build for a set, and not at all for the
    # current set; both together are refused. The null of the archive is the form of the set.
    assert build_args(None, "A") == ["--archive", "A"] and build_args("S", None) == ["--mge-build", "S"] and build_args(None, None) == []
    try:
        build_args("S", "A")
        raise AssertionError("--mge-build with --archive was taken")
    except ValueError as problem:
        assert str(problem) == "an archive is not a set: no --mge-build with --archive", problem
    try:
        BUILD_ARGS[:] = ["--archive", "A"]
        calls.clear()
        run_tool = refused
        with tempfile.TemporaryDirectory() as folder:
            replay(os.path.join(folder, "tour-1.nircap"), 1, os.path.join(folder, "1-01.png"))
        assert calls[-1][-2:] == ["--archive", "A"] and "--mge-build" not in calls[-1], calls[-1]
    finally:
        BUILD_ARGS[:] = []
        run_tool = subprocess.run
    print("validate_tour self-test: ok")
    return 0


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        sys.exit(self_test())
    sys.exit(main())
