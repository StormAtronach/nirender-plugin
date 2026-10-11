"""Builds nirender, installs it into the testing instance and runs the in-game test.

The game starts through the Mod Organizer of D:\\Modlists\\Morrowind75testing and loads a save.
Then every port runs in shadow mode, and then the frame comparisons run. The result is the
port table and the frame test lines. The exit code is 0 only when nothing differed.

    python tools/run_test.py --label first [--save quiksave.ess] [--shadow 20] [--live 5]
"""
import argparse
import os
import sys

import session
from common import read_report

TAG = "[nirender.test] "


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--label", required=True)
    ap.add_argument("--save", default="quiksave.ess", help="save to load, with extension")
    ap.add_argument("--shadow", type=int, default=20, help="seconds in shadow mode")
    ap.add_argument("--live", type=int, default=5, help="seconds with the Rust functions live at the end")
    ap.add_argument("--ports", default="*", help="part of the engine name of the ports to test")
    ap.add_argument("--cells", default="", help="cells to visit after the place of the save, separated by ;")
    ap.add_argument("--tour", default="", help="in each cell, put the player in front of each object whose id starts "
                    "with this text, for example @nif_ for the NIF test level")
    ap.add_argument("--tour-seconds", type=float, default=1.5, help="time at each object of the tour")
    ap.add_argument("--extras", action="store_true", help="in the first place, with the comparison on: open the "
                    "inventory, take a screenshot with the renderer, and make a save")
    ap.add_argument("--timeout", type=int, default=0, help="budget in seconds; 0 = from the scenario")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--start", choices=("off", "shadow", "live"),
                    help="install before the game creates its renderer and run its start in this mode")
    ap.add_argument("--dump", action="store_true",
                    help="write the renderer objects and one frame after the load, for tools/compare_runs.py")
    ap.add_argument("--fault", type=int, default=0,
                    help="put a fault into the Rust code (1: a device call, 2: a memory write); the run passes when the tests find it")
    ap.add_argument("--no-buffers", action="store_true", help="do not record the contents of locked buffers")
    args = ap.parse_args()

    session.prepare(build=not args.no_build)
    run = session.Run(args.label)
    cells = [c for c in args.cells.split(";") if c]
    spec = run.spec(
        "test",
        save=args.save or None,
        shadowSeconds=args.shadow,
        liveSeconds=args.live,
        ports=args.ports,
        startMode=args.start,
        dumpDir=run.dir.replace("\\", "/") if args.dump else None,
        fault=args.fault,
        recordBuffers=not args.no_buffers,
        cells=cells,
        tour=args.tour or None,
        extras=args.extras,
        tourSeconds=args.tour_seconds,
    )
    # A tour has 60 objects at most in a cell of the test level.
    place = max(args.shadow, 60 * args.tour_seconds if args.tour else 0)
    timeout = args.timeout or int((20 if args.extras else 0) + 90 + (1 + len(cells)) * (place + 12) + args.live)
    done = run.launch(spec, timeout, TAG)

    failures, notes = [], []
    if not done:
        failures.append("the scenario did not finish")
    rows = read_report(run.report)
    if rows:
        print("\n%-52s %-7s %12s %10s %8s %10s %10s" % ("port", "mode", "compared", "different", "skipped", "live", "rust runs"))
        for row in rows:
            hook = "LOCKED" if row["locked"] == "true" else "ok" if row["installed"] == "true" else "NO HOOK"
            print("%-52s %-7s %12s %10s %8s %10s %10s" % (row["port"], hook, row["shadow_calls"], row["mismatches"],
                                                         row["skipped"], row["live_calls"], row["rust_runs"]))
            if row["installed"] != "true":
                failures.append("%s: no detour" % row["port"])
            if int(row["mismatches"]):
                failures.append("%s: %s of %s calls differ" % (row["port"], row["mismatches"], row["shadow_calls"]))
    else:
        failures.append("no report")
    for line in run.progress_lines(TAG):
        if "frame test menu" in line and "RETRY" in line:
            # After eight tries the scene still changed between two engine frames. Particles are
            # one cause. The frame test gives no result here. The shadow test still gives one.
            notes.append("frame test without a result, the scene moves: " + line[:60])
        elif "frame test menu" in line and "FAIL" in line:
            failures.append(line[:300])
        if "ERROR" in line:
            failures.append(line[:300])
    failures += run.mge_problems()
    if os.path.isfile(run.log):
        mismatches = [l for l in open(run.log, encoding="utf-8", errors="replace").read().splitlines() if l.startswith("MISMATCH")]
        for line in mismatches[:20]:
            print("  " + line[:400])
    if args.fault:
        shadow_found = any("calls differ" in f for f in failures)
        frame_found = any("frame test menu live: FAIL" in f for f in failures)
        print("\nfault %d: shadow test %s, frame test %s" % (
            args.fault, "FOUND it" if shadow_found else "did NOT find it", "FOUND it" if frame_found else "did NOT find it"))
        for failure in failures:
            print("  " + failure)
        return 0 if shadow_found else 1
    for note in notes:
        print("note: " + note)
    print("\n" + ("PASS" if not failures else "FAIL"))
    for failure in failures:
        print("  " + failure)
    return 0 if not failures else 1


if __name__ == "__main__":
    sys.exit(main())
