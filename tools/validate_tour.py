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
"""
import argparse
import os
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
PYTHON = sys.executable


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
    result = subprocess.run(command, capture_output=True, text=True)
    return result.returncode == 0 and os.path.isfile(picture), (result.stdout + result.stderr)[-400:]


def replay_many(capture, items):
    """One run of the helper for all the frames of a capture: `items` are (frame, picture).
    Returns the names of the pictures that were not made."""
    prefix = os.path.join(os.path.dirname(items[0][1]), "tour-frames-%s" % os.path.basename(capture)[:-7])
    command = [PYTHON, os.path.join(HERE, "replay.py"), capture, prefix, "--frames-at", ",".join(str(f) for f, _ in items)]
    if MGE_BUILD:
        command += ["--mge-build", MGE_BUILD]
    result = subprocess.run(command, capture_output=True, text=True)
    missing = []
    for frame, picture in items:
        made = "%s-%05d.png" % (prefix, frame)
        if os.path.isfile(made):
            if os.path.isfile(picture):
                os.remove(picture)
            os.replace(made, picture)
        else:
            missing.append(os.path.basename(picture))
    if missing:
        print("replay of %s: %d pictures missing: %s" % (os.path.basename(capture), len(missing), (result.stdout + result.stderr)[-300:].strip()))
    return missing


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
    failures = []
    # One run of the helper for each capture, with every frame of it (2026-10-16: 206 runs
    # became 3). The captures run at once, up to --jobs.
    by_capture = {}
    for capture, frame, picture, name in work:
        by_capture.setdefault(capture, []).append((frame, picture))
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for missing in pool.map(lambda kv: replay_many(kv[0], kv[1]), by_capture.items()):
            failures.extend(missing)
    for capture, frame, picture, name in work:
        if name in failures:
            # Once more, alone, one frame.
            ok, text = replay(capture, frame, picture)
            if ok:
                failures.remove(name)
            else:
                print("FAILED %s: %s" % (name, text.strip()[-200:]))
    print("%d pictures, %d failed" % (len(work) - len(failures), len(failures)))
    sys.path.insert(0, HERE)
    import picture_diff

    if pairs:
        rows = []
        for gpu, off in pairs:
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
        print("pairs: %d of %d equal" % (equal, len(rows)))
    if not args.against:
        return 1 if failures else 0

    rows = []
    import glob
    for _, _, picture, name in work:
        if name.endswith("-off.png"):
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
    return 0


if __name__ == "__main__":
    sys.exit(main())
