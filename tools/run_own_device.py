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
    ap.add_argument("--start-frames", default="5,100,300,600,900,1040", help="the frames of the start to take a picture of")
    ap.add_argument("--mge-build", help="a folder with a build of MGE XE that has the switch runtime.renderer_wgpu (its bin/MSVC-Release): "
                    "the run uses its d3d8.dll and its mgeHost64.exe with the switch on, and not the d3d8.dll and the host program of this repository")
    ap.add_argument("--config-mod", default="Configuration", help="the mod of the testing instance with mgeXE.toml")
    ap.add_argument("--reference", action="store_true", help="run the same scenario with MGE XE on Direct3D 9, for its frame rate: the "
                    "d3d8.dll of MGE XE that tools/own_device.py kept, or the one that is in place")
    ap.add_argument("--instance", help="the folder of another Mod Organizer instance to run in, and not the testing instance")
    ap.add_argument("--game-dir", help="with --instance: the game folder of that instance")
    ap.add_argument("--profile", default="Default", help="with --instance: its profile")
    ap.add_argument("--position", help="x,y,z in the exterior cell: the player goes there after the cell loads (the "
                    "coordinates of the console)")
    ap.add_argument("--hour", type=float, default=None, help="the hour of the day in the exterior (12 without this)")
    ap.add_argument("--find-water", action="store_true", help="after the cell loads, put the player under the water at the "
                    "nearest spot where the land is deep enough (a test of the view from under the water)")
    ap.add_argument("--yaw", type=float, default=None, help="the heading of the player in degrees, 0 north, with --position")
    ap.add_argument("--set", action="append", default=[], help="a line of mgeXE.toml for the run, key = value: the line with "
                    "that key is replaced (for example \"expanded_light_limit = true\"); with a section first, "
                    "\"[distant_land] key = value\", a key the file does not have goes at the end of that section")
    ap.add_argument("--shapes", type=int, default=None, help="the mode of the shape records: 0 off, 1 dual (the records and the "
                    "draws of the device), 2 only the records")
    ap.add_argument("--tour-cells", help="the cells of a test level for the step tour, with semicolons (the NIF test level: "
                    "\"@nifTest_1;@nifTest_2;@nifTest_3\")")
    ap.add_argument("--tour", default="@nif_", help="the start of the ids of the objects of the tour")
    ap.add_argument("--tour-seconds", type=float, default=1.5, help="the time at each object of the tour")
    ap.add_argument("--steps", help="the steps of the scenario to do, with commas (for example exterior,rate,capture); "
                    "all of them without this")
    ap.add_argument("--budget", type=int, default=200)
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()

    if args.instance:
        # The harness has the places of the testing instance. They become those of this one.
        harness = session.harness
        harness.MO2_ROOT = args.instance
        harness.MO2_EXE = os.path.join(args.instance, "ModOrganizer.exe")
        harness.PROFILE = os.path.join(args.instance, "profiles", args.profile)
        harness.GAME_DIR = args.game_dir or harness.GAME_DIR
        harness.MWSE_LOGS = (os.path.join(args.instance, "overwrite", "Root", "MWSE.log"), os.path.join(harness.GAME_DIR, "MWSE.log"))
        harness.WARNINGS = (os.path.join(args.instance, "overwrite", "Root", "Warnings.txt"), os.path.join(harness.GAME_DIR, "Warnings.txt"))
        session.MGE_LOGS = (os.path.join(harness.GAME_DIR, "mgeXE.log"), os.path.join(args.instance, "overwrite", "Root", "mgeXE.log"))
    if not args.no_build:
        cargo = os.path.join(os.path.expanduser("~"), ".cargo", "bin", "cargo.exe")
        for folder in (HOST_DIR, D3D8_DIR):
            subprocess.run([cargo, "build", "--release", "--quiet"], cwd=folder, check=True)
    session.prepare(build=not args.no_build)
    run = session.Run(args.label)
    spec = run.spec("owndevice", save=args.save, interior=args.interior, exterior=args.exterior, dir=run.dir.replace("\\", "/"), steps=args.steps,
                    position=args.position, yaw=args.yaw, findWater=args.find_water, hour=args.hour,
                    tourCells=args.tour_cells, tour=args.tour, tourSeconds=args.tour_seconds, shapes=args.shapes)
    # The plugin starts the host program. It reads these from the environment of the game.
    if not args.mge_build:
        os.environ["NIRENDER_HOST_EXE"] = HOST
    os.environ["NIRENDER_HOST_LOG"] = run.path("host.txt")
    os.environ["NIRENDER_BACKEND"] = args.backend
    # Pictures of the start of the game, before the scenario can ask for one.
    os.environ["NIRENDER_HOST_ARGS"] = '--shots-at %s --shots-prefix "%s"' % (args.start_frames, run.path("start"))
    # Mod Organizer puts the Root folder of a mod into the game folder when the game starts.
    # The d3d8.dll of the game folder comes from the mod of MGE XE.
    root = os.path.join(session.harness.MO2_ROOT, "mods", args.mge_mod, "Root")
    # The mgeXE.toml of the run: no frame limit (vsync off), so that the frame rates of the two
    # modes compare; with the switch for the wgpu renderer when the run is with it.
    toml = os.path.join(session.harness.MO2_ROOT, "mods", args.config_mod, "Root", "mgeXE.toml")
    text = open(toml, encoding="utf-8").read()
    assert "[runtime]" in text, toml
    lines = [line for line in text.splitlines() if not line.strip().startswith("renderer_wgpu")]
    lines = ['vsync = "immediate"' if line.strip().startswith("vsync") else line for line in lines]
    for setting in args.set:
        section = None
        if setting.startswith("["):
            section, setting = setting[1:].split("]", 1)
            setting = setting.strip()
        key = setting.split("=")[0].strip()
        if any(line.split("=")[0].strip() == key for line in lines):
            lines = [setting if line.split("=")[0].strip() == key else line for line in lines]
        elif section is not None:
            assert "[%s]" % section in lines, "no section [%s] in %s" % (section, toml)
            start = lines.index("[%s]" % section) + 1
            end = next((i for i in range(start, len(lines)) if lines[i].startswith("[")), len(lines))
            while end > start and not lines[end - 1].strip():
                end -= 1
            lines.insert(end, setting)
        else:
            raise SystemExit("no line with the key %s in %s: give its section, [section] %s" % (key, toml, setting))
    if args.mge_build and not args.reference:
        lines.insert(lines.index("[runtime]") + 1, "renderer_wgpu = true")
    switched = run.path("mgeXE.toml")
    open(switched, "w", encoding="utf-8").write("".join(line + chr(10) for line in lines))
    toml_swap = (switched, toml, "mgeXE.toml of the run")
    swaps = [(D3D8, os.path.join(root, "d3d8.dll"), "the d3d8.dll of nirender"), toml_swap]
    if args.reference:
        kept = os.path.join(root, "d3d8.dll.mge-xe")
        swaps = ([(kept, os.path.join(root, "d3d8.dll"), "the d3d8.dll of MGE XE")] if os.path.isfile(kept) else []) + [toml_swap]
    elif args.mge_build:
        # MGE XE with its switch for the wgpu renderer on: its own d3d8.dll gives Direct3D to
        # the plugin, and its helper is the renderer.
        swaps = [(os.path.join(args.mge_build, "d3d8.dll"), os.path.join(root, "d3d8.dll"), "the d3d8.dll of the MGE XE build"),
                 (os.path.join(args.mge_build, "mgeHost64.exe"), os.path.join(root, "mgeHost64.exe"), "the mgeHost64.exe of the MGE XE build"),
                 toml_swap]
    done = run.launch(spec, args.budget, TAG, swaps=swaps)
    # Mod Organizer leaves its copy in the game folder when the game does not end by itself.
    theirs, ours = os.path.join(root, "d3d8.dll"), os.path.join(session.harness.GAME_DIR, "d3d8.dll")
    if os.path.isfile(ours) and not session.harness.same_file_content(theirs, ours):
        import shutil
        shutil.copy2(theirs, ours)
        print("the d3d8.dll of the mod is back in the game folder")

    print()
    for line in run.progress_lines(TAG):
        if line.startswith(("own device", "step", "after", "steps done", "ERROR", "save loaded", "rate", "zoom")):
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
