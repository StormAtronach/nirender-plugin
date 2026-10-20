"""Records two clips of the same view: one with the game code, one with the Rust code.

    python tools/run_video.py [--cell "Old Ebonheart"] [--seconds 15] [--folder <OneDrive folder>]

OBS must run, with the scene "Harness" that captures the game window. See the recorder in
moreFPS/docs/scripts/obs-record. The game starts one time and holds the view for each clip.
"""
import argparse
import os
import re
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))  # the shared tools in tools/
import session

sys.path.insert(0, r"C:\Users\ivan_\source\repos\moreFPS\docs\scripts\obs-record")
import obs_record  # noqa: E402

TAG = "[nirender.video] "
FOLDER = r"C:\Users\ivan_\OneDrive\Morrowind modding\nirender videos"
# The mode of the game for each clip, and the name of the clip.
CLIPS = (("off", "game code"), ("live", "rust plugin"))


def record_clips(run, seconds, cell, results):
    """Records one clip at each "view ready" line of the scenario. Runs in parallel with the
    harness loop."""
    for index, (mode, label) in enumerate(CLIPS):
        deadline = time.time() + 240
        while time.time() < deadline and sum("view ready" in line for line in run.progress_lines(TAG)) <= index:
            time.sleep(0.5)
        if time.time() >= deadline:
            results[label] = "NO CLIP: the view never came"
            return
        time.sleep(1)
        obs = obs_record.Obs()
        try:
            results[label] = obs_record.record(obs, seconds, "nirender %s - %s" % (cell.lower(), label))
        except BaseException as error:  # The harness loop must still end the game.
            results[label] = "NO CLIP: %s" % error
        finally:
            obs.close()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cell", default="Old Ebonheart")
    ap.add_argument("--save", default="quiksave.ess")
    ap.add_argument("--seconds", type=float, default=15)
    ap.add_argument("--folder", default=FOLDER)
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()

    obs_record.LONGEST = args.seconds
    obs_record.DRIVE = args.folder
    session.prepare(build=not args.no_build, features=("rig",))
    run = session.Run("video")
    # The recorder waits 3 s for the capture to become stable, and needs a margin after the clip.
    spec = run.spec("video", save=args.save, cell=args.cell, modes=[mode for mode, _ in CLIPS],
                    holdSeconds=args.seconds + 10)
    results = {}
    recorder = threading.Thread(target=record_clips, args=(run, args.seconds, args.cell, results))
    recorder.start()
    run.launch(spec, 90 + len(CLIPS) * (args.seconds + 20), TAG)
    recorder.join(timeout=30)

    # The scenario reports the calls that ran as Rust up to the end of each clip. The count
    # must be 0 after the game code clip and more than 0 after the Rust clip.
    counts = [int(m.group(1)) for line in run.progress_lines(TAG) for m in [re.search(r"(\d+) calls ran as Rust", line)] if m]
    ok = len(counts) == len(CLIPS) and counts[0] == 0 and counts[1] > 0
    for (_, label), count in zip(CLIPS, counts):
        print("%s: %s, %d calls ran as Rust at its end" % (label, results.get(label, "NO CLIP"), count))
    ok = ok and all(os.path.isfile(str(results.get(label, ""))) for _, label in CLIPS)
    for problem in run.mge_problems():
        print("WARNING: " + problem)
    print("OK" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
