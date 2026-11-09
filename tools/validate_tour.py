"""Replays the stops of a tour of a test level and compares the pictures with a baseline.

    python tools/validate_tour.py <run dir> [--against <baseline dir>] [--out <dir>] [--jobs N]
                                  [--mge-build DIR]

The run comes from `run_own_device.py --steps tour --tour-cells "@nifTest_1;@nifTest_2;@nifTest_3"`:
its folder has `tour-<cell>.nircap` with two frames for each stop, and `progress.log` names
the stops. This replays the second frame of each stop into `<out>/<cell>-<stop>-<id>.png`
(`<out>` is the run folder without `--out`). With `--against`, it compares each picture with
the one of the same name in the baseline folder and prints the mean difference, with the
largest ones first. Make a baseline by replaying a run and copying its folder.

A run made with `--tour-dual` has four frames for each stop: two in mode gpu, then two in
mode off, with the time of the game held. Then this replays both (the off picture is
`<cell>-<stop>-<id>-off.png`) and compares each pair: the two paths at one moment.

A replay that the slot check refuses (a replay needs the whole build side) made no picture, and
no picture of it is checked: it is "NOT RUN: slot refused", not a failed picture, and it gets
no second try. Its pairs are not compared. The exit code is 1 when a picture failed, else 3 when
a replay was not run, else 0.

    python tools/validate_tour.py --self-test   # a planted refusal, with no replay
"""
import argparse
import os
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
PYTHON = sys.executable
# The start of the refusal of slot.py's check of build work (`refuse_build_tool`), which replay.py calls.
REFUSED = "Refused by the slot check"
NOT_RUN = "NOT RUN: slot refused"
NOT_RUN_CODE = 3
# The runner of replay.py: the self-test replaces it.
run_tool = subprocess.run


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


MGE_BUILD = None


def replay(capture, frame, picture):
    command = [PYTHON, os.path.join(HERE, "replay.py"), capture, picture, "--frame", str(frame)]
    # The helper of the build under test, not the default one of replay.py.
    if MGE_BUILD:
        command += ["--mge-build", MGE_BUILD]
    result = run_tool(command, capture_output=True, text=True)
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
    if MGE_BUILD:
        command += ["--mge-build", MGE_BUILD]
    result = run_tool(command, capture_output=True, text=True)
    output = result.stdout + result.stderr
    refused = refusal_of(output)
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
    ap.add_argument("--mge-build", default=None, help="the MGE XE build whose helper replays (replay.py's default without it)")
    args = ap.parse_args()
    global MGE_BUILD
    MGE_BUILD = args.mge_build
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
    sys.path.insert(0, HERE)
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
    if not args.against:
        return 1 if failures else NOT_RUN_CODE if not_run else 0

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
    return NOT_RUN_CODE if not_run else 0


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
        finally:
            run_tool = subprocess.run
    assert refusal_of("a\n" + refusal + "\nb") == refusal and refusal_of("replay of x: 3 pictures missing") == ""
    print("validate_tour self-test: ok")
    return 0


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        sys.exit(self_test())
    sys.exit(main())
