"""Builds nirender, installs it into the testing instance and runs the in-game test.

The game starts through the Mod Organizer of D:\\Modlists\\Morrowind75testing and loads a save.
Then it runs every port in shadow mode, and then it runs the frame comparisons. The result is
the port table and the frame test lines. The exit code is 0 only when nothing differed.

    python tools/run_test.py --label first [--save quiksave.ess] [--shadow 20] [--live 5]

The script puts the testing instance back to its initial state, but the mod "nirender" stays
installed. The mod does nothing without a run spec and with mode "off".
"""
import argparse
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
HARNESS_DIR = r"C:\Users\ivan_\source\repos\moreFPS\docs\scripts\test-harness-2026-10-03"
sys.path.insert(0, HARNESS_DIR)
import harness  # noqa: E402

MOD_NAME = "nirender"
DLL = os.path.join(REPO, "target", "i686-pc-windows-msvc", "release", "nirender.dll")
DIST = os.path.join(REPO, "target", "dist", MOD_NAME)
RESULTS = os.path.join(REPO, "results")
SPEC_NAME = "nirender_test"
TAG = "[nirender.test] "


def build():
    env = dict(os.environ)
    env["PATH"] = os.path.join(os.path.expanduser("~"), ".cargo", "bin") + os.pathsep + env["PATH"]
    for command in (["cargo", "test", "--quiet"], ["cargo", "build", "--release", "--quiet"]):
        print("> " + " ".join(command), flush=True)
        if subprocess.run(command, cwd=REPO, env=env).returncode != 0:
            raise SystemExit("build failed")


def stage():
    """The files of the installed mod: the Lua files and the DLL. It never has the pdb."""
    if os.path.isdir(DIST):
        shutil.rmtree(DIST)
    shutil.copytree(os.path.join(REPO, "mod"), DIST)
    os.makedirs(os.path.join(DIST, "MWSE", "lib"), exist_ok=True)
    shutil.copy2(DLL, os.path.join(DIST, "MWSE", "lib", "nirender.dll"))


def use_our_mod():
    """Sets the harness to this mod, so the install and spec code of the harness serve it."""
    harness.MOD_SRC = DIST
    harness.MOD_NAME = MOD_NAME
    harness.MOD_DIR = os.path.join(harness.MO2_ROOT, "mods", MOD_NAME)
    harness.SPEC_DIR = os.path.join(harness.MOD_DIR, "MWSE", "config")


def read_report(path):
    rows = []
    if not os.path.isfile(path):
        return rows
    lines = open(path, encoding="utf-8").read().splitlines()
    header = lines[0].split("\t")
    for line in lines[1:]:
        if line.strip():
            rows.append(dict(zip(header, line.split("\t"))))
    return rows


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--label", required=True)
    ap.add_argument("--save", default="quiksave.ess", help="save to load, with extension")
    ap.add_argument("--shadow", type=int, default=20, help="seconds in shadow mode")
    ap.add_argument("--live", type=int, default=5, help="seconds with the Rust functions live at the end")
    ap.add_argument("--ports", default="*", help="part of the engine name of the ports to test")
    ap.add_argument("--cells", default="", help="cells to visit after the place of the save, separated by ;")
    ap.add_argument("--timeout", type=int, default=0, help="budget in seconds; 0 = from the scenario")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--start", choices=("off", "shadow", "live"),
                    help="install before the game creates its renderer and run its start in this mode")
    ap.add_argument("--dump", action="store_true",
                    help="write the renderer objects and one frame after the load, for tools/compare_runs.py")
    ap.add_argument("--without", default="", help="mods to switch off for the run, separated by ; (for example \"MGE XE G7 Fork\")")
    ap.add_argument("--fault", type=int, default=0,
                    help="put a fault into the Rust code (1: a device call, 2: a memory write); the run passes when the tests find it")
    ap.add_argument("--no-buffers", action="store_true", help="do not record the contents of locked buffers")
    args = ap.parse_args()

    if not args.no_build:
        build()
    stage()
    use_our_mod()
    harness.refuse_if_busy()

    run_dir = os.path.join(RESULTS, time.strftime("%Y%m%d-%H%M%S") + "-" + args.label)
    os.makedirs(run_dir)
    progress_file = os.path.join(run_dir, "progress.log")
    report_file = os.path.join(run_dir, "report.tsv")
    log_file = os.path.join(run_dir, "nirender.log")
    spec = {
        "progressFile": progress_file.replace("\\", "/"),
        "reportFile": report_file.replace("\\", "/"),
        "logFile": log_file.replace("\\", "/"),
        "save": args.save or None,
        "shadowSeconds": args.shadow,
        "liveSeconds": args.live,
        "ports": args.ports,
        "startMode": args.start,
        "dumpDir": run_dir.replace("\\", "/") if args.dump else None,
        "fault": args.fault,
        "recordBuffers": not args.no_buffers,
        "cells": [c for c in args.cells.split(";") if c],
    }
    places = 1 + len(spec["cells"])
    timeout = args.timeout or 90 + places * (args.shadow + 12) + args.live
    for change in harness.install_mod():
        print("  nirender mod: %s" % change)
    deployment = harness.Deployment()
    started = time.time()
    try:
        deployment.write_spec(SPEC_NAME, spec)
        off = [m for m in args.without.split(";") if m]
        if off:
            modlist = os.path.join(harness.PROFILE, "modlist.txt")
            with open(modlist, encoding="utf-8", newline="") as fh:
                lines = fh.read().splitlines(keepends=True)
            missing = [m for m in off if not any(l.strip() == "+" + m for l in lines)]
            if missing:
                raise SystemExit("not an active mod of the profile: %s" % ", ".join(missing))
            edited = os.path.join(run_dir, "modlist.txt")
            with open(edited, "w", encoding="utf-8", newline="") as fh:
                fh.write("".join("-" + l[1:] if l.strip()[1:] in off and l.startswith("+") else l for l in lines))
            deployment.swap(edited, modlist, "mod list without " + ", ".join(off))
    except BaseException:
        deployment.restore()
        raise
    print("launching (budget %ds, results in %s)" % (timeout, run_dir), flush=True)
    try:
        done = harness.run_game(progress_file, timeout, TAG, deployment, started)
    finally:
        # run_game also restores. This restore is for a failure before run_game gets to that
        # point.
        deployment.restore()
    harness.copy_logs(run_dir, started)

    failures, notes = [], []
    if not done:
        failures.append("the scenario did not finish")
    rows = read_report(report_file)
    if rows:
        print("\n%-52s %-7s %12s %10s %8s %10s %10s" % ("port", "mode", "compared", "different", "skipped", "live", "rust runs"))
        for row in rows:
            hook = "LOCKED" if row.get("locked") == "true" else "ok" if row["installed"] == "true" else "NO HOOK"
            print("%-52s %-7s %12s %10s %8s %10s %10s" % (row["port"], hook, row["shadow_calls"], row["mismatches"],
                                                         row["skipped"], row["live_calls"], row.get("rust_runs", "")))
            if row["installed"] != "true":
                failures.append("%s: no detour" % row["port"])
            if int(row["mismatches"]):
                failures.append("%s: %s of %s calls differ" % (row["port"], row["mismatches"], row["shadow_calls"]))
    else:
        failures.append("no report")
    if os.path.isfile(progress_file):
        for line in open(progress_file, encoding="utf-8", errors="replace").read().splitlines():
            if "frame test menu" in line and "RETRY" in line:
                # After eight tries the scene still changed between two engine frames. Particles
                # are one cause. The frame test gives no result here. The shadow test still
                # gives a result.
                notes.append("frame test without a result, the scene moves: " + line.replace(TAG, "").strip()[:60])
            elif "frame test menu" in line and "FAIL" in line:
                failures.append(line.replace(TAG, "").strip()[:300])
            if "ERROR" in line:
                failures.append(line.replace(TAG, "").strip()[:300])
    if os.path.isfile(log_file):
        mismatches = [l for l in open(log_file, encoding="utf-8", errors="replace").read().splitlines() if l.startswith("MISMATCH")]
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
