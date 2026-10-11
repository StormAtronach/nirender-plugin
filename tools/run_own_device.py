"""Runs the game without a Direct3D device: the d3d8.dll of this repository takes the place of
the one of MGE XE in the game folder for the run, and the game gets the Direct3D 8 of the
plugin. The host program draws the only picture, in the window of the game.

    python tools/run_own_device.py --label owndevice [--backend vulkan]

The runner puts the d3d8.dll of MGE XE back after the run. The result is a picture of the
host program after each step of the scenario (host-<step>.png), the text of the host program
(host.txt) and the lines of the plugin about its device.
"""
import argparse
import os
import subprocess
import sys

import session
from common import REPO

TAG = "[nirender.owndevice] "
HOST_DIR = os.path.join(REPO, "host")
HOST = os.path.join(HOST_DIR, "target", "x86_64-pc-windows-msvc", "release", "nirender-host.exe")
D3D8_DIR = os.path.join(REPO, "d3d8")
D3D8 = os.path.join(D3D8_DIR, "target", "i686-pc-windows-msvc", "release", "d3d8.dll")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--label", default="owndevice")
    ap.add_argument("--save", default="quiksave.ess")
    ap.add_argument("--interior", default="Balmora, Guild of Mages")
    ap.add_argument("--exterior", default="Balmora")
    ap.add_argument("--backend", default="vulkan")
    ap.add_argument("--mge-mod", default="MGE XE G7 Fork", help="the mod of the testing instance with the d3d8.dll of MGE XE")
    ap.add_argument("--budget", type=int, default=200)
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()

    if not args.no_build:
        cargo = os.path.join(os.path.expanduser("~"), ".cargo", "bin", "cargo.exe")
        for folder in (HOST_DIR, D3D8_DIR):
            subprocess.run([cargo, "build", "--release", "--quiet"], cwd=folder, check=True)
    session.prepare(build=not args.no_build)
    run = session.Run(args.label)
    spec = run.spec("owndevice", save=args.save, interior=args.interior, exterior=args.exterior, dir=run.dir.replace("\\", "/"))
    # The plugin starts the host program. It reads these from the environment of the game.
    os.environ["NIRENDER_HOST_EXE"] = HOST
    os.environ["NIRENDER_HOST_LOG"] = run.path("host.txt")
    os.environ["NIRENDER_BACKEND"] = args.backend
    # Mod Organizer puts the Root folder of a mod into the game folder when the game starts.
    # The d3d8.dll of the game folder comes from the mod of MGE XE.
    swaps = [(D3D8, os.path.join(session.harness.MO2_ROOT, "mods", args.mge_mod, "Root", "d3d8.dll"), "the d3d8.dll of nirender")]
    done = run.launch(spec, args.budget, TAG, swaps=swaps)
    # Mod Organizer leaves its copy in the game folder when the game does not end by itself.
    theirs, ours = swaps[0][1], os.path.join(session.harness.GAME_DIR, "d3d8.dll")
    if os.path.isfile(ours) and os.path.getsize(ours) == os.path.getsize(D3D8) and not session.harness.same_file_content(theirs, ours):
        import shutil
        shutil.copy2(theirs, ours)
        print("the d3d8.dll of MGE XE is back in the game folder")

    print()
    for line in run.progress_lines(TAG):
        if line.startswith(("own device", "step", "after", "steps done", "ERROR", "save loaded")):
            print("game: " + line[:200])
    if os.path.isfile(run.path("host.txt")):
        print("\nhost:")
        print("  " + open(run.path("host.txt"), errors="replace").read().strip().replace("\n", "\n  ")[-3000:])
    else:
        print("\nhost: no text; the host program did not start")
    # The plugin writes into nirender.log of the game folder until Lua names the log of the run.
    for path in (os.path.join(session.harness.GAME_DIR, "nirender.log"), run.log):
        if os.path.isfile(path) and os.path.getmtime(path) > run.started:
            for line in open(path, encoding="utf-8", errors="replace"):
                if line.startswith(("own device", "stream:", "capture: end", "own host", "install:", "link:")):
                    print("plugin: " + line.strip()[:240])
    print("pictures in " + run.dir)
    return 0 if done else 1


if __name__ == "__main__":
    sys.exit(main())
