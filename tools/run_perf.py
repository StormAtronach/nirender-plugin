"""Frame times with the game code against frame times with the Rust code.

Starts the game through the testing instance, loads a save and goes to a cell. Then it
measures rounds that alternate: off, live, off, live. Every port has its detour in the two
modes. In "off" a call goes through the detour to the game function. In "live" a call goes
through the detour to the Rust function.

    python tools/run_perf.py --cell "Old Ebonheart" [--rounds 6] [--seconds 10]

Prints the frame time statistics of each mode and writes results/<run>/perf.json.
"""
import argparse
import json
import os
import statistics
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import run_test  # noqa: E402
from run_test import harness  # noqa: E402

TAG = "[nirender.perf] "


def percentile(ordered, fraction):
    return ordered[min(len(ordered) - 1, int(len(ordered) * fraction))]


def describe(times):
    ordered = sorted(times)
    mean = statistics.fmean(times)
    return {
        "frames": len(times),
        "mean_ms": mean / 1000,
        "fps": 1_000_000 / mean,
        "median_ms": percentile(ordered, 0.5) / 1000,
        "p95_ms": percentile(ordered, 0.95) / 1000,
        "p99_ms": percentile(ordered, 0.99) / 1000,
        "max_ms": ordered[-1] / 1000,
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cell", default="Old Ebonheart")
    ap.add_argument("--save", default="quiksave.ess")
    ap.add_argument("--rounds", type=int, default=6, help="rounds for each mode")
    ap.add_argument("--seconds", type=int, default=10, help="length of a round")
    ap.add_argument("--label", default="perf")
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()

    if not args.no_build:
        run_test.build()
    run_test.stage()
    run_test.use_our_mod()
    harness.refuse_if_busy()

    run_dir = os.path.join(run_test.RESULTS, time.strftime("%Y%m%d-%H%M%S") + "-" + args.label)
    os.makedirs(run_dir)
    progress_file = os.path.join(run_dir, "progress.log")
    result_file = os.path.join(run_dir, "frames.json")
    spec = {
        "perf": True,
        "progressFile": progress_file.replace("\\", "/"),
        "resultFile": result_file.replace("\\", "/"),
        "logFile": os.path.join(run_dir, "nirender.log").replace("\\", "/"),
        "save": args.save,
        "cell": args.cell,
        "rounds": args.rounds,
        "roundSeconds": args.seconds,
    }
    for change in harness.install_mod():
        print("  nirender mod: %s" % change)
    deployment = harness.Deployment()
    started = time.time()
    timeout = 90 + args.rounds * 2 * (args.seconds + 2)
    try:
        deployment.write_spec(run_test.SPEC_NAME, spec)
        print("launching (budget %ds, results in %s)" % (timeout, run_dir), flush=True)
        done = harness.run_game(progress_file, timeout, TAG, deployment, started)
    finally:
        deployment.restore()
    harness.copy_logs(run_dir, started)
    if not done or not os.path.isfile(result_file):
        print("FAIL: the measurement did not finish")
        return 1

    data = json.load(open(result_file, encoding="utf-8"))
    by_mode = {"off": [], "live": []}
    rounds = []
    for entry in data["rounds"]:
        times = [float(t) for t in entry["microseconds"].split(",") if t]
        by_mode[entry["mode"]].extend(times)
        rounds.append((entry["mode"], describe(times)))

    print("\ncell: %s, %d rounds of %d s for each mode" % (data["cell"], args.rounds, args.seconds))
    print("\n%-6s %8s %9s %8s %10s %8s %8s %8s" % ("mode", "frames", "mean ms", "fps", "median ms", "p95 ms", "p99 ms", "max ms"))
    summary = {}
    for mode, label in (("off", "game"), ("live", "rust")):
        d = describe(by_mode[mode])
        summary[label] = d
        print("%-6s %8d %9.3f %8.1f %10.3f %8.3f %8.3f %8.1f" % (
            label, d["frames"], d["mean_ms"], d["fps"], d["median_ms"], d["p95_ms"], d["p99_ms"], d["max_ms"]))
    change = (summary["rust"]["mean_ms"] / summary["game"]["mean_ms"] - 1) * 100
    print("\nmean frame time, rust against game: %+.2f %%" % change)
    print("\nrounds (mean ms): " + "  ".join("%s %.3f" % ("game" if m == "off" else "rust", d["mean_ms"]) for m, d in rounds))
    # The spread between rounds of the same mode tells how much a difference between the modes
    # means.
    for mode, label in (("off", "game"), ("live", "rust")):
        means = [d["mean_ms"] for m, d in rounds if m == mode]
        print("%s rounds: min %.3f max %.3f ms" % (label, min(means), max(means)))
    json.dump({"cell": data["cell"], "summary": summary, "rounds": rounds, "change_percent": change},
              open(os.path.join(run_dir, "perf.json"), "w", encoding="utf-8"), indent=1)
    return 0


if __name__ == "__main__":
    sys.exit(main())
