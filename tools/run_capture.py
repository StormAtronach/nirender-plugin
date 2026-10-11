"""Makes a capture of the render link stream in the game.

The game loads a save, goes to a cell, and the plugin writes results/<run>/frames.nircap:
each resource with its bytes and each command of some frames. The host program draws the
capture again without the game. The picture of the game for the same view is game.png.

    python tools/run_capture.py --label interior --cell "Balmora, Guild of Mages" [--frames 3]
"""
import argparse
import os
import sys

import session

TAG = "[nirender.capture] "
# Each fact and each kind of the link: the capture has only what goes through the link.
ALL_FACTS = 4261413630
ALL_KINDS = 15


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--label", default="capture")
    ap.add_argument("--save", default="quiksave.ess")
    ap.add_argument("--cell", default="", help="the cell to capture; empty for the place of the save")
    ap.add_argument("--yaw", type=float, help="turn the player to this direction, in degrees from north")
    ap.add_argument("--frames", type=int, default=3, help="frames of record")
    ap.add_argument("--warmup", type=int, default=3, help="frames before the record, in which the plugin notes the geometry")
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()

    session.prepare(build=not args.no_build)
    run = session.Run(args.label)
    capture = run.path("frames.nircap")
    spec = run.spec("capture", save=args.save, cell=args.cell, frames=args.frames, warmupFrames=args.warmup,
                    facts=ALL_FACTS, kinds=ALL_KINDS, captureFile=capture.replace("\\", "/"),
                    shot=run.path("game.png").replace("\\", "/"), yaw=args.yaw)
    done = run.launch(spec, 150, TAG)
    for line in run.progress_lines(TAG):
        if "capture" in line or "ERROR" in line:
            print("  " + line[:200])
    size = os.path.getsize(capture) if os.path.isfile(capture) else 0
    print("\n%s: %.1f MB" % (capture, size / 1e6))
    problems = run.mge_problems()
    for problem in problems:
        print("  " + problem)
    return 0 if done and size > 8 and not problems else 1


if __name__ == "__main__":
    sys.exit(main())
