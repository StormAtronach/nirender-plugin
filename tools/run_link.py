"""Compares the packets of the render link with what MGE XE infers.

Starts the game through the testing instance and loads a save. In the place of the save and
in each cell of --cells, the game runs with the game code and then with the Rust code.
MGE XE counts, for each fact, how many times the packet of the plugin and its own inference
were equal or different.

    python tools/run_link.py --cells "Balmora;Seyda Neen" [--seconds 8]

The d3d8.dll of the testing instance must have the render link host. Prints one table and
writes results/<run>/link.json. The exit code is 0 when no fact was different.
"""
import argparse
import json
import re
import sys

import session

TAG = "[nirender.link] "
FACT = re.compile(r"fact (\w+): both_yes (\d+) both_no (\d+) host_only (\d+) packet_only (\d+)")
COLUMNS = ("both_yes", "both_no", "host_only", "packet_only")
TOTALS = re.compile(r"scenes (\d+) draws (\d+) draws_without_packet (\d+)")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cells", default="", help="cells to visit, separated by ';'")
    ap.add_argument("--save", default="quiksave.ess")
    ap.add_argument("--seconds", type=int, default=8, help="time in each mode in each place")
    ap.add_argument("--hour", type=float, help="set the game hour before the test, for example 0 for night")
    ap.add_argument("--label", default="link")
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()

    cells = [c.strip() for c in args.cells.split(";") if c.strip()]
    session.prepare(build=not args.no_build)
    run = session.Run(args.label)
    spec = run.spec("link", save=args.save, cells=cells, seconds=args.seconds)
    if args.hour is not None:
        spec["hour"] = args.hour
    done = run.launch(spec, 120 + (len(cells) + 1) * (2 * (args.seconds + 2) + 12), TAG)

    rows = []
    connected = False
    for line in run.progress_lines(TAG):
        if line.startswith("link: "):
            connected = line.startswith("link: true")
            print(line)
        if not line.startswith("counters|"):
            continue
        _, where, mode, text = line.split("|", 3)
        totals = TOTALS.search(text)
        if totals:
            rows.append({"where": where, "mode": mode, "scenes": int(totals.group(1)), "draws": int(totals.group(2)),
                         "draws_without_packet": int(totals.group(3)), "facts": {}})
        fact = FACT.search(text)
        if fact and rows:
            rows[-1]["facts"][fact.group(1)] = dict(zip(COLUMNS, (int(n) for n in fact.groups()[1:])))
    json.dump(rows, open(run.path("link.json"), "w", encoding="utf-8"), indent=1)

    different = 0
    for row in rows:
        print("\n%s, %s: %d scenes, %d draw packets, %d world draws without a packet" % (
            row["where"], "game code" if row["mode"] == "off" else "rust code", row["scenes"], row["draws"],
            row["draws_without_packet"]))
        for name, count in row["facts"].items():
            if any(count.values()):
                print("  %-14s both yes %8d  both no %8d  only MGE XE %8d  only packet %8d" % (
                    name, count["both_yes"], count["both_no"], count["host_only"], count["packet_only"]))
            different += count["host_only"] + count["packet_only"]

    for problem in run.mge_problems():
        print("FAIL: " + problem)
        different += 1
    if not done:
        print("FAIL: the scenario did not finish")
        return 1
    if not connected or not rows:
        print("FAIL: no connection to the render link, or no counters")
        return 1
    print("\n%s" % ("PASS: every compared fact was equal" if different == 0 else "DIFFERENT: %d comparisons" % different))
    return 0 if different == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
