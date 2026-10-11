"""Runs the game and the host program together: the plugin sends the render link stream into
shared memory, and the host draws it in its own window while the game runs.

    python tools/run_stream.py --label live [--cell "Balmora, Guild of Mages"] [--seconds 15]

The game loads a save, puts the player in front of the nearest actor, and makes one stream for
each backend. The result is the frame rate of the host, the time that the stream costs the
game thread, a picture of the game (game.png) and a picture of the host for each backend.
"""
import argparse
import os
import subprocess
import sys
import threading

import session
from common import REPO

TAG = "[nirender.stream] "
ALL_FACTS = 4261413630
ALL_KINDS = 15
HOST_DIR = os.path.join(REPO, "host")
HOST = os.path.join(HOST_DIR, "target", "x86_64-pc-windows-msvc", "release", "nirender-host.exe")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--label", default="stream")
    ap.add_argument("--save", default="quiksave.ess")
    ap.add_argument("--cell", default="")
    ap.add_argument("--seconds", type=int, default=15, help="the time of each stream")
    ap.add_argument("--megabytes", type=int, default=64, help="the size of the ring")
    ap.add_argument("--backends", default="vulkan,dx12")
    ap.add_argument("--embed", action="store_true", help="the host shows its picture in the window of the game")
    ap.add_argument("--no-actor", action="store_true", help="stay at the place of the save or of the cell")
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()

    if not args.no_build:
        cargo = os.path.join(os.path.expanduser("~"), ".cargo", "bin", "cargo.exe")
        subprocess.run([cargo, "build", "--release", "--quiet"], cwd=HOST_DIR, check=True)
    session.prepare(build=not args.no_build)
    run = session.Run(args.label)
    backends = [b for b in args.backends.split(",") if b]
    spec = run.spec("stream", save=args.save, cell=args.cell, seconds=args.seconds, megabytes=args.megabytes, warmupFrames=5,
                    backends=backends, streamName="nirender-%d" % os.getpid(), actor=not args.no_actor, facts=ALL_FACTS,
                    kinds=ALL_KINDS, shot=run.path("game.png").replace("\\", "/"))

    hosts = []
    screens = []
    stop = threading.Event()

    def start_hosts():
        """Starts the host for each stream that the scenario reports."""
        seen = 0
        while not stop.wait(0.2):
            lines = run.progress_lines(TAG)
            for line in lines[seen:]:
                parts = line.split()
                if line.startswith("stream started") and len(parts) > 4 and parts[4] == "true":
                    name, backend = parts[2], parts[3]
                    log = open(run.path("host-%s.txt" % backend), "w")
                    command = [HOST, "--live", name, "--backend", backend, "--shot", run.path("host-%s.png" % backend), "--shot-frame", "120"]
                    process = subprocess.Popen(command + (["--embed"] if args.embed else []), stdout=log, stderr=subprocess.STDOUT)
                    hosts.append((backend, process, log))
                    # What the screen shows some seconds into the stream.
                    threading.Timer(6.0, lambda b=backend: screens.append((b, session.harness.screenshot("stream-" + b)))).start()
            seen = len(lines)

    watcher = threading.Thread(target=start_hosts, daemon=True)
    watcher.start()
    done = run.launch(spec, 120 + len(backends) * (args.seconds + 25), TAG)
    stop.set()
    for backend, process, log in hosts:
        try:
            process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            process.kill()
        log.close()

    print()
    for line in run.progress_lines(TAG):
        if line.startswith(("stream ended", "actor", "ERROR")):
            print("game: " + line[:240])
    for backend, process, _ in hosts:
        print("\nhost on %s (exit code %s):" % (backend, process.returncode))
        print("  " + open(run.path("host-%s.txt" % backend)).read().strip().replace("\n", "\n  "))
    for backend, path in screens:
        print("screen during the stream on %s: %s" % (backend, path))
    if os.path.isfile(run.log):
        for line in open(run.log, encoding="utf-8", errors="replace"):
            if line.startswith(("stream:", "capture: end", "capture: the renderer")):
                print("plugin: " + line.strip())
    problems = run.mge_problems()
    for problem in problems:
        print("  " + problem)
    return 0 if done and hosts and all(p.returncode == 0 for _, p, _ in hosts) and not problems else 1


if __name__ == "__main__":
    sys.exit(main())
