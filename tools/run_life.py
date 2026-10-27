"""Runs the game with the plugin as the host of its own render link, and does the things that
need more than draws: the character image, the screenshot of the game, a save with its
picture, changes of cell, and a load.

    python tools/run_life.py --label life [--backend vulkan] [--embed]

After the change of host no call of the game gets to a D3D device. The host program draws the
only picture, and it gives the pixels of a target to the game when the game reads them. The
result is a picture of the host after each step (host-<step>.png), the screenshot that the
game wrote (game-screenshot.png), the picture in the save (save-picture.png), and the lines of
the plugin about the pixel reads and the device calls that it took.
"""
import argparse
import glob
import os
import struct
import subprocess
import sys
import threading
import time

import session
from common import REPO

TAG = "[nirender.life] "
ALL_FACTS = 4261413630
ALL_KINDS = 15
HOST_DIR = os.path.join(REPO, "host")
HOST = os.path.join(HOST_DIR, "target", "x86_64-pc-windows-msvc", "release", "nirender-host.exe")


def save_picture(save, out):
    """Writes the picture of a save file as a PNG: the TES3 record has a SCRS or SCRD field
    with 128 x 128 pixels of 4 bytes."""
    from PIL import Image
    data = open(save, "rb").read()
    size = struct.unpack_from("<I", data, 4)[0]
    at, end = 16, 16 + size
    while at + 8 <= end:
        name, length = data[at:at + 4], struct.unpack_from("<I", data, at + 4)[0]
        if name in (b"SCRS", b"SCRD") and length >= 128 * 128 * 4:
            pixels = data[at + 8:at + 8 + 128 * 128 * 4]
            Image.frombytes("RGBA", (128, 128), pixels, "raw", "BGRA").convert("RGB").save(out)
            return True
        at += 8 + length
    return False


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--label", default="life")
    ap.add_argument("--save", default="quiksave.ess")
    ap.add_argument("--interior", default="Balmora, Guild of Mages")
    ap.add_argument("--exterior", default="Balmora")
    ap.add_argument("--megabytes", type=int, default=64)
    ap.add_argument("--backend", default="vulkan")
    ap.add_argument("--embed", action="store_true", help="the host shows its picture in the window of the game")
    ap.add_argument("--dump-only", action="store_true", help="write device.txt, what the Direct3D of the game says, and end")
    ap.add_argument("--budget", type=int, default=240)
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()

    if not args.no_build:
        cargo = os.path.join(os.path.expanduser("~"), ".cargo", "bin", "cargo.exe")
        subprocess.run([cargo, "build", "--release", "--quiet"], cwd=HOST_DIR, check=True)
    session.prepare(build=not args.no_build)
    run = session.Run(args.label)
    spec = run.spec("life", save=args.save, interior=args.interior, exterior=args.exterior, megabytes=args.megabytes, warmupFrames=5,
                    backend=args.backend, streamName="nirender-%d" % os.getpid(), facts=ALL_FACTS, kinds=ALL_KINDS,
                    dir=run.dir.replace("\\", "/"), dumpOnly=args.dump_only)

    hosts = []
    stop = threading.Event()

    def start_host():
        while not stop.wait(0.2):
            for line in run.progress_lines(TAG):
                parts = line.split()
                if line.startswith("stream started") and len(parts) > 4 and parts[4] == "true" and not hosts:
                    log = open(run.path("host.txt"), "w")
                    command = [HOST, "--live", parts[2], "--backend", parts[3]] + (["--embed"] if args.embed else [])
                    hosts.append((subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT), log))

    threading.Thread(target=start_host, daemon=True).start()
    done = run.launch(spec, args.budget, TAG)
    stop.set()
    for process, log in hosts:
        try:
            process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            process.kill()
        log.close()

    print()
    for line in run.progress_lines(TAG):
        if line.startswith(("step", "after", "steps done", "ERROR", "stream started")):
            print("game: " + line[:200])
    for process, _ in hosts:
        print("\nhost (exit code %s):" % process.returncode)
        print("  " + open(run.path("host.txt")).read().strip().replace("\n", "\n  "))
    for line in run.plugin_lines():
        if line.startswith(("stream:", "capture: end", "own host")):
            print("plugin: " + line.strip())

    # The screenshot that the game wrote in this run, and the picture of the save.
    from PIL import Image
    shots = [p for p in glob.glob(os.path.join(session.harness.GAME_DIR, "*.bmp")) if os.path.getmtime(p) > run.started]
    for path in shots:
        Image.open(path).save(run.path("game-screenshot.png"))
        print("screenshot of the game: %s -> game-screenshot.png" % os.path.basename(path))
    if not shots:
        print("screenshot of the game: no new BMP file in the game folder")
    saves = [p for root in (session.harness.GAME_DIR, session.harness.MO2_ROOT) for p in
             glob.glob(os.path.join(root, "**", "nirender_life.ess"), recursive=True) if os.path.getmtime(p) > run.started]
    for path in saves[:1]:
        print("picture of the save: %s" % ("save-picture.png" if save_picture(path, run.path("save-picture.png")) else "not in the file"))
    if not saves:
        print("picture of the save: no new save file")
    print("pictures in " + run.dir)
    return 0 if done and hosts else 1


if __name__ == "__main__":
    sys.exit(main())
