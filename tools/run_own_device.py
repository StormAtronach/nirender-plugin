"""Runs the game without a Direct3D device: the d3d8.dll of this repository (or of a build of
MGE XE with --mge-build) takes the place of the one of MGE XE in the game folder for the run,
and the game gets the Direct3D 8 of the plugin. The host program draws the only picture, in
the window of the game.

    python tools/run_own_device.py --label owndevice [--backend vulkan]
        [--plugin-dll <akulakhan.dll>] [--log <file.txt>]
    python -I tools/run_own_device.py --self-test    # the tests of the argument handling

The command of a run takes no variable prefix, redirect or chained command, so that one
allow rule of a session covers every run: `--plugin-dll` names the plugin DLL of the run
(the variable `NIRENDER_PLUGIN_DLL` is the fallback, then the release build of this
repository), and `--log` names a file that gets the runner's own output, written to the
console too (the programs the runner starts write to the console only).

The files of the run are in mods of the harness that are on for the run only (session.py):
"Harness - MGE XE" (d3d8.dll, numidium.exe), "Harness - config" (mgeXE.toml of the run) and
"Harness - exe" (--exe). The result is a picture of the host program after each step of the
scenario (host-<step>.png), the text of the host program (host.txt) and the lines of the
plugin about its device.
"""
import argparse
import os
import subprocess
import sys

# The modules of this folder, also under `python -I` (checks.py runs the self-test so).
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import session  # noqa: E402
from common import REPO  # noqa: E402

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
BRIDGE_DIR = os.path.join(session.harness.MO2_ROOT, "mods", "mwseMCP")


class Tee:
    """A stream that writes to the console and to the log file of the run (`--log`)."""

    def __init__(self, console, log):
        self.console = console
        self.log = log

    def write(self, text):
        self.console.write(text)
        self.log.write(text)

    def flush(self):
        self.console.flush()
        self.log.flush()


def plugin_dll_of(argument, environment, default):
    """The plugin DLL of a run and where it came from: `--plugin-dll`, else the variable
    `NIRENDER_PLUGIN_DLL`, else the release build of this repository."""
    if argument:
        return argument, "--plugin-dll"
    if environment.get("NIRENDER_PLUGIN_DLL"):
        return environment["NIRENDER_PLUGIN_DLL"], "NIRENDER_PLUGIN_DLL"
    return default, "the release build of this repository"


def self_test():
    """The tests of the argument handling: no game, no build."""
    build = os.path.join("target", "akulakhan.dll")
    assert plugin_dll_of("E:/a.dll", {"NIRENDER_PLUGIN_DLL": "E:/b.dll"}, build) == ("E:/a.dll", "--plugin-dll")
    assert plugin_dll_of(None, {"NIRENDER_PLUGIN_DLL": "E:/b.dll"}, build) == ("E:/b.dll", "NIRENDER_PLUGIN_DLL")
    assert plugin_dll_of(None, {}, build) == (build, "the release build of this repository")
    assert plugin_dll_of("", {}, build)[0] == build
    import io

    console, log = io.StringIO(), io.StringIO()
    tee = Tee(console, log)
    print("a line", file=tee)
    tee.flush()
    assert console.getvalue() == "a line\n" and log.getvalue() == "a line\n"
    print("run_own_device self-test: ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--label", default="owndevice")
    ap.add_argument("--plugin-dll", help="the plugin DLL of the run (else the variable NIRENDER_PLUGIN_DLL, else the release build of this repository)")
    ap.add_argument("--log", help="a file that gets the runner's output, written to the console too; no shell redirect needed")
    ap.add_argument("--self-test", action="store_true", help="the tests of the argument handling: no game, no build")
    ap.add_argument("--save", default="quiksave.ess", help="the save to load; \"\" starts a new game instead (the films of the game play)")
    ap.add_argument("--interior", default="Balmora, Guild of Mages")
    ap.add_argument("--exterior", default="Balmora")
    ap.add_argument("--backend", default="vulkan")
    ap.add_argument("--start-frames", default="5,100,300,600,900,1040", help="the frames of the start to take a picture of")
    ap.add_argument("--mge-build", help="a folder with a build of MGE XE that has the switch runtime.renderer_wgpu (its bin/MSVC-Release): "
                    "the run uses its d3d8.dll and its numidium.exe with the switch on, and not the d3d8.dll and the host program of this repository")
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
    ap.add_argument("--keep-fade", action="store_true", help="keep the fade-in of the distant statics and the grass (the tunable statics.fade); "
                    "without this the mod sets it to 0 at the start, so that a picture does not depend on the moment of an arrival")
    ap.add_argument("--keep-ai", action="store_true", help="keep the AI of the actors on; without this the mod turns it off after the load, so "
                    "that two pictures of a place show the same thing (a measurement of the game's cost needs the AI on)")
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
    ap.add_argument("--add-file", action="append", default=[], metavar="SOURCE=PATH",
                    help="a file for the run's mod at PATH (from the Data Files folder, or Root/... for the game folder), for this run only: "
                    "a test texture or mesh; the run's mod wins over the instance's mods")
    ap.add_argument("--mwse-dll", help="another MWSE.dll for this run only (as --add-file <it>=Root/MWSE.dll): an A/B of an MWSE build "
                    "without a change to the instance")
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.log:
        # The runner's own lines go to the console and to the file; the file is the one a
        # shell redirect wrote before.
        log = open(args.log, "w", encoding="utf-8", buffering=1)
        sys.stdout = Tee(sys.stdout, log)
        sys.stderr = Tee(sys.stderr, log)
    session.DLL, source = plugin_dll_of(args.plugin_dll, os.environ, session.DLL)
    print("plugin DLL: %s (%s)" % (session.DLL, source), flush=True)
    extra_files = [entry.split("=", 1) for entry in args.add_file]
    if any(len(pair) != 2 for pair in extra_files):
        raise SystemExit("--add-file takes SOURCE=PATH")
    if args.mwse_dll:
        extra_files.append((args.mwse_dll, "Root/MWSE.dll"))

    if args.instance:
        session.use_instance(args.instance, args.game_dir, args.profile)
    if not args.no_build:
        cargo = os.path.join(os.path.expanduser("~"), ".cargo", "bin", "cargo.exe")
        for folder in (HOST_DIR, D3D8_DIR):
            subprocess.run([cargo, "build", "--release", "--quiet"], cwd=folder, check=True)
    session.prepare(build=not args.no_build, extra_files=extra_files)
    run = session.Run(args.label)
    # Without a save (--save ""), the scenario starts a new game: the films of the game play.
    spec = run.spec("owndevice", save=args.save or None, interior=args.interior, exterior=args.exterior, dir=run.dir.replace("\\", "/"), steps=args.steps,
                    position=args.position, yaw=args.yaw, findWater=args.find_water, findDry=args.find_dry, hour=args.hour, weather=args.weather,
                    lua=open(args.lua_file, encoding="utf-8").read() if args.lua_file else None,
                    shaderBatch=open(args.shader_batch, encoding="utf-8").read() if args.shader_batch else None,
                    tourCells=args.tour_cells, tour=args.tour, tourSeconds=args.tour_seconds, tourDual=args.tour_dual or None, tourExtra=args.tour_extra or None, captureMenu=args.capture_menu, settleSeconds=args.settle, portTimes=args.port_times or None,
                    overlay=args.overlay or None, earlyPicture=run.path("early.png").replace("\\", "/") if args.early_picture else None,
                    afterLoadPictures=args.after_load_pictures or None, keepFade=args.keep_fade or None, keepAI=args.keep_ai or None)
    # The plugin starts the host program. It reads these from the environment of the game.
    if not args.mge_build:
        os.environ["NIRENDER_HOST_EXE"] = HOST
    os.environ["NIRENDER_HOST_LOG"] = run.path("host.txt")
    os.environ["NIRENDER_BACKEND"] = args.backend
    # A message box of the plugin, the helper or MGE XE closes itself after 5 s: a run never
    # waits for a click, and the box's text is in the log (the failure-path design, 5.1).
    os.environ["MGE_XE_BOX_SECONDS"] = "5"
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
        mods[0] = (MGE_MOD, {"Root/d3d8.dll": os.path.join(args.mge_build, "d3d8.dll"), "Root/numidium.exe": os.path.join(args.mge_build, "numidium.exe")})
        # The symbol files of the build beside the binaries, when it has them: a profile
        # of the run resolves the helper's symbols without a search path (the owner,
        # 2026-10-16). The Harness mod is the runner's staging, not an install.
        for name in ("numidium.pdb", "d3d8.pdb"):
            symbols = os.path.join(args.mge_build, name)
            if os.path.isfile(symbols):
                mods[0][1]["Root/" + name] = symbols
        # The DXVK of the build, when it has one, over the DXVK of the instance: the
        # Direct3D 9 path of a feature that needs a DXVK change.
        dxvk = os.path.join(args.mge_build, "d3d9.dll")
        if os.path.isfile(dxvk):
            mods[0][1]["Root/d3d9.dll"] = dxvk
        # The Lua of the build (the MGE XE Options mod: its menus and the event
        # distantLandReady), over the copy of the release in the instance.
        options = os.path.join(args.mge_build, "Data Files", "mwse", "mods", "MGE XE Options")
        for folder, _, names in os.walk(options):
            for name in names:
                source = os.path.join(folder, name)
                mods[0][1]["MWSE/mods/MGE XE Options/" + os.path.relpath(source, options).replace(os.sep, "/")] = source
        # The core shaders of the build (Direct3D 9 reads them at run time; the build's
        # d3d8.dll sets the parameters they declare), over the copy of the release.
        core = os.path.join(args.mge_build, "Data Files", "shaders", "core")
        if os.path.isdir(core):
            for name in os.listdir(core):
                mods[0][1]["shaders/core/" + name] = os.path.join(core, name)
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
    # The size of the run and of every run kept (the owner, 2026-10-18: the results folder
    # had grown to hundreds of GB unnoticed). The runner deletes nothing; the line says when
    # it is time to.
    print("size: this run %s; all runs in %s %s" % (folder_size(run.dir), os.path.dirname(run.dir), folder_size(os.path.dirname(run.dir))))
    return 0 if done else 1


def folder_size(path):
    """The size of a folder as text, counted file by file."""
    total = 0
    for root, _, files in os.walk(path):
        for name in files:
            try:
                total += os.path.getsize(os.path.join(root, name))
            except OSError:
                pass
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if total < 1024 or unit == "TB":
            return "%.1f %s" % (total, unit)
        total /= 1024.0
    return "?"


if __name__ == "__main__":
    sys.exit(main())
