"""Records two clips of the same view: one with the game code, one with the Rust code.

    python tools/run_video.py [--cell "Old Ebonheart"] [--seconds 15] [--folder <OneDrive folder>]

OBS must run, with the scene "Harness" that captures the game window. See the recorder in
moreFPS/docs/scripts/obs-record. The game starts one time for each clip.
"""
import argparse
import os
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, r"C:\Users\ivan_\source\repos\moreFPS\docs\scripts\obs-record")
import obs_record  # noqa: E402
import run_test  # noqa: E402
from run_test import harness  # noqa: E402

TAG = "[nirender.video] "
FOLDER = r"C:\Users\ivan_\OneDrive\Morrowind modding\nirender videos"


def record_when_ready(progress_file, seconds, name, result):
    """Waits for the view, then records. Runs in parallel with the harness loop."""
    deadline = time.time() + 240
    while time.time() < deadline:
        if os.path.isfile(progress_file) and "view ready" in open(progress_file, encoding="utf-8", errors="replace").read():
            break
        time.sleep(0.5)
    else:
        result["error"] = "the view never came"
        return
    time.sleep(1)
    obs = obs_record.Obs()
    try:
        result["path"] = obs_record.record(obs, seconds, name)
    except BaseException as error:  # The harness loop must still end the game.
        result["error"] = str(error)
    finally:
        obs.close()


def clip(mode, label, args):
    run_dir = os.path.join(run_test.RESULTS, time.strftime("%Y%m%d-%H%M%S") + "-video-" + label)
    os.makedirs(run_dir)
    progress_file = os.path.join(run_dir, "progress.log")
    spec = {
        "video": True,
        "progressFile": progress_file.replace("\\", "/"),
        "reportFile": os.path.join(run_dir, "report.tsv").replace("\\", "/"),
        "logFile": os.path.join(run_dir, "nirender.log").replace("\\", "/"),
        "save": args.save,
        "cell": args.cell,
        "mode": mode,
        # The recorder waits 3 s for the capture to become stable, and a margin after the clip.
        "holdSeconds": args.seconds + 10,
    }
    deployment = harness.Deployment()
    started = time.time()
    result = {}
    name = "nirender %s - %s" % (args.cell.lower(), label)
    recorder = threading.Thread(target=record_when_ready, args=(progress_file, args.seconds, name, result))
    try:
        deployment.write_spec(run_test.SPEC_NAME, spec)
        recorder.start()
        harness.run_game(progress_file, 120 + args.seconds, TAG, deployment, started)
    finally:
        deployment.restore()
    recorder.join(timeout=30)
    harness.copy_logs(run_dir, started)
    # The number of calls that ran as Rust during the clip. It is 0 for the game code clip.
    rows = run_test.read_report(os.path.join(run_dir, "report.tsv"))
    live = sum(int(row["live_calls"]) for row in rows)
    print("%s: %s, %d calls ran as Rust" % (label, result.get("path") or "NO CLIP: " + result.get("error", "?"), live))
    return result.get("path"), live


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
    if not args.no_build:
        run_test.build()
    run_test.stage()
    run_test.use_our_mod()
    harness.refuse_if_busy()
    for change in harness.install_mod():
        print("  nirender mod: %s" % change)

    clips = [clip("off", "game code", args), clip("live", "rust plugin", args)]
    ok = all(path for path, _ in clips) and clips[0][1] == 0 and clips[1][1] > 0
    print("OK" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
