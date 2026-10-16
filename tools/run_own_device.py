"""Runs the game without a Direct3D device: the d3d8.dll of this repository (or of a build of
MGE XE with --mge-build) takes the place of the one of MGE XE in the game folder for the run,
and the game gets the Direct3D 8 of the plugin. The host program draws the only picture, in
the window of the game.

    python tools/run_own_device.py --label owndevice [--backend vulkan]

The files of the run are in mods of the harness that are on for the run only (session.py):
"Harness - MGE XE" (d3d8.dll, mgeHost64.exe), "Harness - config" (mgeXE.toml of the run) and
"Harness - exe" (--exe). The result is a picture of the host program after each step of the
scenario (host-<step>.png), the text of the host program (host.txt) and the lines of the
plugin about its device.
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
# The mods of the harness for the files of a run.
MGE_MOD = session.harness.RUN_MOD_PREFIX + "MGE XE"
CONFIG_MOD = session.harness.RUN_MOD_PREFIX + "config"
EXE_MOD = session.harness.RUN_MOD_PREFIX + "exe"
# The Lua bridge of mwse-mcp, as the testing instance has it (`--bridge`).
BRIDGE_DIR = r"D:\Modlists\Morrowind75testing\mods\mwseMCP"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--label", default="owndevice")
    ap.add_argument("--save", default="quiksave.ess", help="the save to load; \"\" starts a new game instead (the films of the game play)")
    ap.add_argument("--interior", default="Balmora, Guild of Mages")
    ap.add_argument("--exterior", default="Balmora")
    ap.add_argument("--backend", default="vulkan")
    ap.add_argument("--start-frames", default="5,100,300,600,900,1040", help="the frames of the start to take a picture of")
    ap.add_argument("--mge-build", help="a folder with a build of MGE XE that has the switch runtime.renderer_wgpu (its bin/MSVC-Release): "
                    "the run uses its d3d8.dll and its mgeHost64.exe with the switch on, and not the d3d8.dll and the host program of this repository")
    ap.add_argument("--config-mod", default="Configuration", help="the mod of the testing instance with mgeXE.toml")
    ap.add_argument("--reference", action="store_true", help="run the same scenario with MGE XE on Direct3D 9, for its frame rate: the "
                    "MGE XE of the instance, or of --mge-build")
    ap.add_argument("--instance", help="the folder of another Mod Organizer instance to run in, and not the testing instance")
    ap.add_argument("--game-dir", help="with --instance: the game folder of that instance")
    ap.add_argument("--profile", default="Default", help="with --instance: its profile")
    ap.add_argument("--position", help="x,y,z in the exterior cell: the player goes there after the cell loads (the "
                    "coordinates of the console)")
    ap.add_argument("--hour", type=float, default=None, help="the hour of the day in the exterior (12 without this)")
    ap.add_argument("--weather", type=int, default=None, help="the weather in the exterior, 0 clear to 9 blizzard (the game's without this)")
    ap.add_argument("--disable-mod", action="append", default=[], help="a mod of the profile switched off in its modlist.txt for the run, and back after; again for more mods")
    ap.add_argument("--find-water", action="store_true", help="after the cell loads, put the player under the water at the "
                    "nearest spot where the land is deep enough (a test of the view from under the water)")
    ap.add_argument("--find-dry", action="store_true", help="after the interior cell loads, put the player on the nearest floor above "
                    "the water level of the cell, looking back at the entry (a test of the water of an interior)")
    ap.add_argument("--yaw", type=float, default=None, help="the heading of the player in degrees, 0 north, with --position")
    ap.add_argument("--lua-file", help="a file of Lua that the game runs a little after the player arrives at the interior and at the "
                    "exterior: for a state that a mod makes from Lua (the variables of a post shader)")
    ap.add_argument("--shader-batch", help="a file of Lua that gives a list of sets of post shaders, { name = \"lens\", { \"Lens Effects\", "
                    "variable = value } }: the step `shaders` takes one picture for each set (sh-<name>), with one start of the game")
    ap.add_argument("--set", action="append", default=[], help="a line of mgeXE.toml for the run, key = value: the line with "
                    "that key is replaced (for example \"expanded_light_limit = true\"); with a section first, "
                    "\"[distant_land] key = value\", a key the file does not have goes at the end of that section")
    ap.add_argument("--tour-cells", help="the cells of a test level for the step tour, with semicolons (the NIF test level: "
                    "\"@nifTest_1;@nifTest_2;@nifTest_3\")")
    ap.add_argument("--tour", default="@nif_", help="the start of the ids of the objects of the tour")
    ap.add_argument("--tour-extra", action="store_true", help="the test NIFs of the flag support (tools/make_test_nifs.py) placed "
                    "in the first cell of the tour, as stops")
    ap.add_argument("--tour-seconds", type=float, default=1.5, help="the time at each object of the tour")
    ap.add_argument("--tour-dual", action="store_true", help="at each stop of the tour, two frames in mode gpu and two in mode off, "
                    "with the time of the game held between them (tools/validate_tour.py compares the pair)")
    ap.add_argument("--steps", help="the steps of the scenario to do, with commas (for example exterior,rate,capture); "
                    "all of them without this")
    ap.add_argument("--capture-menu", type=int, default=0, help="frames of a capture file that starts at the main menu and holds the load of the "
                    "save (menu.nircap): the loading screen is a screen polygon")
    ap.add_argument("--budget", type=int, default=200)
    ap.add_argument("--settle", type=float, default=None, help="the seconds to wait at the interior and at the exterior before the picture (6 without this)")
    ap.add_argument("--port-times", action="store_true", help="the mod reports the port times at each step (it turns the timing of every live port call on: not for a profile)")
    ap.add_argument("--overlay", action="store_true", help="turn the debug overlay of the renderer on at the exterior step (it is on the picture of the "
                    "window only, not in the picture files)")
    ap.add_argument("--exe", help="a Morrowind.exe to put in the game folder for the run, and the one of the game back after: "
                    "for example the exe with other options of the Morrowind Code Patch")
    ap.add_argument("--early-picture", action="store_true", help="the test mod asks for a picture (mge.saveScreenshot) when its main.lua "
                    "runs, before the game has a renderer: the file early.png of the run must come with the first frames")
    ap.add_argument("--after-load-pictures", action="store_true", help="the test mod takes a picture at the event loaded of the save, then one every "
                    "0.1 s (after-load-NN.png): the first frames after a game load")
    ap.add_argument("--window-shots", action="store_true", help="each picture of the host program also as the window shows it, "
                    "<name>-window.png: with the status text of MGE XE and the debug overlay (wgpu only)")
    ap.add_argument("--bridge", action="store_true", help="the Lua bridge of mwse-mcp (the mod mwseMCP of the testing instance) for this run only, "
                    "in the mod of the run's mgeXE.toml: live work in another instance without a change to its mods")
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
        harness.ROOT_BUILDER_DATA = os.path.join(args.instance, "plugins", "data", "RootBuilder")
    if not args.no_build:
        cargo = os.path.join(os.path.expanduser("~"), ".cargo", "bin", "cargo.exe")
        for folder in (HOST_DIR, D3D8_DIR):
            subprocess.run([cargo, "build", "--release", "--quiet"], cwd=folder, check=True)
    session.prepare(build=not args.no_build)
    run = session.Run(args.label)
    # Without a save (--save ""), the scenario starts a new game: the films of the game play.
    spec = run.spec("owndevice", save=args.save or None, interior=args.interior, exterior=args.exterior, dir=run.dir.replace("\\", "/"), steps=args.steps,
                    position=args.position, yaw=args.yaw, findWater=args.find_water, findDry=args.find_dry, hour=args.hour, weather=args.weather,
                    lua=open(args.lua_file, encoding="utf-8").read() if args.lua_file else None,
                    shaderBatch=open(args.shader_batch, encoding="utf-8").read() if args.shader_batch else None,
                    tourCells=args.tour_cells, tour=args.tour, tourSeconds=args.tour_seconds, tourDual=args.tour_dual or None, tourExtra=args.tour_extra or None, captureMenu=args.capture_menu, settleSeconds=args.settle, portTimes=args.port_times or None,
                    overlay=args.overlay or None, earlyPicture=run.path("early.png").replace("\\", "/") if args.early_picture else None,
                    afterLoadPictures=args.after_load_pictures or None)
    # The plugin starts the host program. It reads these from the environment of the game.
    if not args.mge_build:
        os.environ["NIRENDER_HOST_EXE"] = HOST
    os.environ["NIRENDER_HOST_LOG"] = run.path("host.txt")
    os.environ["NIRENDER_BACKEND"] = args.backend
    # Pictures of the start of the game, before the scenario can ask for one.
    # Pictures of the start of the game, before the scenario can ask for one, and of a frame of
    # a film (the 120th: a film starts from black): a film holds the main loop, so the scenario
    # cannot ask for it.
    os.environ["NIRENDER_HOST_ARGS"] = '--shots-at %s --shots-prefix "%s" --shot-film "%s"' % (args.start_frames, run.path("start"), run.path("host-film-frame.png"))
    if args.window_shots:
        os.environ["NIRENDER_HOST_ARGS"] += " --window-shots"
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
        if section is not None:
            # In its section only: a key such as `enabled` is in several sections.
            assert "[%s]" % section in lines, "no section [%s] in %s" % (section, toml)
            start = lines.index("[%s]" % section) + 1
            end = next((i for i in range(start, len(lines)) if lines[i].startswith("[")), len(lines))
            at = next((i for i in range(start, end) if lines[i].split("=")[0].strip() == key), None)
            if at is not None:
                lines[at] = setting
            else:
                while end > start and not lines[end - 1].strip():
                    end -= 1
                lines.insert(end, setting)
        elif any(line.split("=")[0].strip() == key for line in lines):
            lines = [setting if line.split("=")[0].strip() == key else line for line in lines]
        else:
            raise SystemExit("no line with the key %s in %s: give its section, [section] %s" % (key, toml, setting))
    # The switch is written for each mode: the default of MGE XE is on, and a reference run is
    # MGE XE on Direct3D 9.
    if args.reference:
        lines.insert(lines.index("[runtime]") + 1, "renderer_wgpu = false")
    elif args.mge_build:
        lines.insert(lines.index("[runtime]") + 1, "renderer_wgpu = true")
    switched = run.path("mgeXE.toml")
    open(switched, "w", encoding="utf-8").write("".join(line + chr(10) for line in lines))
    # The files of the run are in mods of the harness, on for the run over the mods of the
    # instance; Root Builder copies their Root folders into the game folder.
    mods = [(MGE_MOD, {"Root/d3d8.dll": D3D8}), (CONFIG_MOD, {"Root/mgeXE.toml": switched})]
    if args.mge_build:
        # MGE XE with its switch for the wgpu renderer on: its own d3d8.dll gives Direct3D to
        # the plugin, and its helper is the renderer. With --reference the same build with the
        # switch off: Direct3D 9 with the d3d8.dll and the helper of that build (the helper as
        # the IPC host of distant land).
        mods[0] = (MGE_MOD, {"Root/d3d8.dll": os.path.join(args.mge_build, "d3d8.dll"), "Root/mgeHost64.exe": os.path.join(args.mge_build, "mgeHost64.exe")})
        # The symbol files of the build beside the binaries, when it has them: a profile
        # of the run resolves the helper's symbols without a search path (the owner,
        # 2026-10-16). The Harness mod is the runner's staging, not an install.
        for name in ("mgeHost64.pdb", "d3d8.pdb"):
            symbols = os.path.join(args.mge_build, name)
            if os.path.isfile(symbols):
                mods[0][1]["Root/" + name] = symbols
        # The Lua of the build (the MGE XE Options mod: its menus and the event
        # distantLandReady), over the copy of the release in the instance.
        options = os.path.join(args.mge_build, "Data Files", "mwse", "mods", "MGE XE Options")
        for folder, _, names in os.walk(options):
            for name in names:
                source = os.path.join(folder, name)
                mods[0][1]["MWSE/mods/MGE XE Options/" + os.path.relpath(source, options).replace(os.sep, "/")] = source
    elif args.reference:
        # MGE XE of the instance on Direct3D 9.
        mods.pop(0)
    if args.exe:
        mods.append((EXE_MOD, {"Root/Morrowind.exe": args.exe}))
    if args.bridge:
        bridge = os.path.join(BRIDGE_DIR, "MWSE", "mods", "mwseMCP")
        files = next(files for name, files in mods if name == CONFIG_MOD)
        for name in os.listdir(bridge):
            files["MWSE/mods/mwseMCP/" + name] = os.path.join(bridge, name)
    done = run.launch(spec, args.budget, TAG, mods=mods, disable=args.disable_mod)

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
