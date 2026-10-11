"""Compares the picture of the host program with the picture of the game, view by view.

For each view the game makes a reference picture and a capture of the same frame, held in
menu mode. The reference is the game drawn by the fixed-function pipeline of Direct3D: MGE XE
with its additions off. The host program then draws each capture, and this tool compares the
two pictures and writes a sheet with both and their difference.

    python tools/run_ab.py --label ab --places ";here" ";actor" "Balmora, Guild of Mages;actor"
    python tools/run_ab.py --label nif --places "@nifTest_1;tour" --tour "@nif_"

A place is "cell;views". The cell can be empty for the place of the save. The views are
"here", "actor" (in front of the nearest actor) or "tour" (in front of each object whose id
starts with --tour).

The result is results/<run>/ab.json and one line for each view, the worst first.
"""
import argparse
import json
import os
import subprocess
import sys

import picture_diff
import session
from common import REPO

TAG = "[nirender.ab] "
ALL_FACTS = 4261413630
ALL_KINDS = 15
HOST_DIR = os.path.join(REPO, "host")
HOST = os.path.join(HOST_DIR, "target", "x86_64-pc-windows-msvc", "release", "nirender-host.exe")
MGE_CONFIGS = (r"D:\GOG\Morrowind\Morrowind\mgeXE.toml", r"D:\Modlists\Morrowind75testing\mods\Configuration\Root\mgeXE.toml")


# The settings of MGE XE that a run changes, each with the value for a plain reference.
PLAIN = {
    # MGE XE puts its own fog mode on the device and drops the mode that the game asks for.
    # The game asks for linear fog on the depth for each pixel.
    "fog_mode": '"depth_pixel"',
    # The host does not smooth edges yet.
    "anti_aliasing": '"none"',
    "transparency_antialiasing": "false",
}


class GameFog:
    """Gives MGE XE the settings of `PLAIN` for the time of a run, so that the reference
    picture is the picture of the game."""

    def __enter__(self):
        self.saved = {}
        for path in MGE_CONFIGS:
            if os.path.isfile(path):
                text = open(path, encoding="utf-8", newline="").read()
                self.saved[path] = text
                lines = text.splitlines(keepends=False)
                for i, line in enumerate(lines):
                    key = line.split("=")[0].strip()
                    if key in PLAIN and "=" in line:
                        lines[i] = "%s = %s" % (key, PLAIN[key])
                ending = "\r\n" if "\r\n" in text else "\n"
                open(path, "w", encoding="utf-8", newline="").write(ending.join(lines) + ending)
        return self

    def __exit__(self, *_):
        for path, text in self.saved.items():
            open(path, "w", encoding="utf-8", newline="").write(text)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--label", default="ab")
    ap.add_argument("--save", default="quiksave.ess")
    ap.add_argument("--places", nargs="+", default=[";here"], help='each one "cell;views"')
    ap.add_argument("--tour", default="@nif_")
    ap.add_argument("--max-views", type=int, default=0, help="at most this number of views in a place; 0 for all")
    ap.add_argument("--backends", default="vulkan", help="the backends of the host, separated by ','; the first one is compared")
    ap.add_argument("--anisotropy", type=int, default=8, help="the anisotropic filter of MGE XE in the reference")
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()

    if not args.no_build:
        cargo = os.path.join(os.path.expanduser("~"), ".cargo", "bin", "cargo.exe")
        subprocess.run([cargo, "build", "--release", "--quiet"], cwd=HOST_DIR, check=True)
    session.prepare(build=not args.no_build)
    run = session.Run(args.label)
    places = []
    for text in args.places:
        cell, _, views = text.partition(";")
        places.append({"cell": cell, "views": views or "here"})
    tours = sum(1 for place in places if place["views"] == "tour")
    spec = run.spec("ab", save=args.save, places=places, tour=args.tour, maxViews=args.max_views or None, facts=ALL_FACTS,
                    kinds=ALL_KINDS, dir=run.dir.replace("\\", "/"))
    views_budget = (args.max_views or 60) * tours + len(places)
    with GameFog():
        done = run.launch(spec, 120 + len(places) * 12 + views_budget * 7, TAG)

    views = []
    for line in run.progress_lines(TAG):
        parts = line.split("|")
        if parts[0] == "view" and len(parts) >= 4:
            views.append((int(parts[1]), parts[2], parts[3]))
        elif "ERROR" in line:
            print("  " + line[:300])

    backends = [b for b in args.backends.split(",") if b]
    results = []
    for number, label, status in views:
        capture = run.path("view-%03d.nircap" % number)
        reference = run.path("ref-%03d.png" % number)
        entry = {"view": number, "label": label, "capture": status}
        for backend in backends:
            picture = run.path("host-%03d-%s.png" % (number, backend))
            # The reference has the anisotropic filter of MGE XE: 8 in the test install.
            host = subprocess.run([HOST, capture, picture, "--backend", backend, "--anisotropy", str(args.anisotropy)], capture_output=True, text=True)
            entry["host_" + backend] = [line for line in host.stdout.splitlines() if line.startswith(("draws", "not drawn", "not complete"))]
            if host.returncode != 0 or not os.path.isfile(picture):
                entry["problem"] = "the host failed on %s: %s" % (backend, (host.stderr or host.stdout).strip()[-300:])
        first = run.path("host-%03d-%s.png" % (number, backends[0]))
        if "problem" not in entry and os.path.isfile(reference):
            a, b = picture_diff.load(reference), picture_diff.load(first)
            entry["numbers"] = picture_diff.compare(a, b)
            if "problem" not in entry["numbers"]:
                picture_diff.sheet(a, b, run.path("sheet-%03d.png" % number))
        elif "problem" not in entry:
            entry["problem"] = "no reference picture"
        results.append(entry)

    json.dump(results, open(run.path("ab.json"), "w"), indent=1)
    # The worst view first: by the part of the pixels that are different by more than 8.
    results.sort(key=lambda e: -e.get("numbers", {}).get("over8", 1.0))
    print("\n%-4s %-52s %9s %9s %9s %7s" % ("view", "what", "over 2", "over 8", "over 32", "mean"))
    for entry in results:
        numbers = entry.get("numbers", {})
        if "over8" in numbers:
            print("%-4d %-52s %8.2f%% %8.2f%% %8.2f%% %7.2f" % (entry["view"], entry["label"][:52], numbers["over2"] * 100, numbers["over8"] * 100,
                                                            numbers["over32"] * 100, numbers["mean"]))
        else:
            print("%-4d %-52s %s" % (entry["view"], entry["label"][:52], entry.get("problem") or numbers.get("problem")))
    print("\n%d views; pictures and sheets in %s" % (len(results), run.dir))
    problems = run.mge_problems()
    for problem in problems:
        print("  " + problem)
    return 0 if done and results and not problems else 1


if __name__ == "__main__":
    sys.exit(main())
