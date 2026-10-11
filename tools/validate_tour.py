"""Replays the stops of a tour of a test level and compares the pictures with a baseline.

    python tools/validate_tour.py <run dir> [--against <baseline dir>] [--out <dir>] [--jobs N]

The run comes from `run_own_device.py --steps tour --tour-cells "@nifTest_1;@nifTest_2;@nifTest_3"`:
its folder has `tour-<cell>.nircap` with two frames for each stop, and `progress.log` names
the stops. This replays the second frame of each stop into `<out>/<cell>-<stop>-<id>.png`
(`<out>` is the run folder without `--out`). With `--against`, it compares each picture with
the one of the same name in the baseline folder and prints the mean difference, with the
largest ones first. Make a baseline by replaying a run and copying its folder.
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
    """{cell index: [(stop index, object id)]} from the progress log."""
    stops = {}
    pattern = re.compile(r"tour: cell (\d+) stop (\d+) (\S+)")
    for line in open(os.path.join(run, "progress.log"), encoding="utf-8", errors="replace"):
        found = pattern.search(line)
        if found:
            stops.setdefault(int(found.group(1)), []).append((int(found.group(2)), found.group(3)))
    return stops


def replay(capture, frame, picture):
    command = [PYTHON, os.path.join(HERE, "replay.py"), capture, picture, "--frame", str(frame)]
    result = subprocess.run(command, capture_output=True, text=True)
    return result.returncode == 0 and os.path.isfile(picture), (result.stdout + result.stderr)[-400:]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("run")
    ap.add_argument("--against")
    ap.add_argument("--out")
    ap.add_argument("--jobs", type=int, default=4)
    args = ap.parse_args()
    out = args.out or args.run
    os.makedirs(out, exist_ok=True)
    stops = stops_of(args.run)
    work = []
    for cell, cell_stops in sorted(stops.items()):
        capture = os.path.join(args.run, "tour-%d.nircap" % cell)
        if not os.path.isfile(capture):
            print("no capture for cell %d" % cell)
            continue
        for stop, ident in cell_stops:
            name = "%d-%02d-%s.png" % (cell, stop, re.sub(r"[^A-Za-z0-9_]", "_", ident))
            # Two frames for each stop: the second one. The replay counts the frames from 0.
            work.append((capture, stop * 2 - 1, os.path.join(out, name), name))
    print("%d stops to replay" % len(work))
    failures = []
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for (capture, frame, picture, name), (ok, text) in zip(work, pool.map(lambda w: replay(w[0], w[1], w[2]), work)):
            if not ok:
                # A replay can fail when several run at once. Once more, alone.
                ok, text = replay(capture, frame, picture)
            if not ok:
                failures.append(name)
                print("FAILED %s: %s" % (name, text.strip()[-200:]))
    print("%d pictures, %d failed" % (len(work) - len(failures), len(failures)))
    if not args.against:
        return 1 if failures else 0
    sys.path.insert(0, HERE)
    import picture_diff

    rows = []
    import glob
    for _, _, picture, name in work:
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
