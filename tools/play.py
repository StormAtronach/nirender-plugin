"""Starts a game of the testing instance to play by hand, with the wgpu renderer of a build of
MGE XE, and puts the files of the instance back when the game ends.

    python tools/play.py [--mge-build DIR] [--reference] [--set "[section] key = value"]

The testing instance at rest has the release of MGE XE and of the plugin. This tool does for a
game by hand what the runners do for a scenario: the mod "Harness - MGE XE" with the d3d8.dll
and the numidium.exe of the build, and "Harness - config" with mgeXE.toml with the switch
`renderer_wgpu`, both on for this game only, over the mods of the instance. With --reference:
MGE XE of the instance on Direct3D 9.

Close Mod Organizer of the testing instance first. Leave the game by its menu: the tool
waits for that, then stops its Mod Organizer and puts the mod list back.
"""
import argparse
import os
import subprocess
import sys
import tempfile
import time

import session  # noqa: F401  (puts the harness on the path)
import harness

MGE_BUILD = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "..", "MGE-XE-G7-render-link", "bin", "MSVC-Release")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mge-build", default=MGE_BUILD, help="the folder with d3d8.dll and numidium.exe of the build")
    ap.add_argument("--config-mod", default="Configuration", help="the mod of the testing instance with mgeXE.toml")
    ap.add_argument("--reference", action="store_true", help="MGE XE of the instance on Direct3D 9")
    ap.add_argument("--set", action="append", default=[], help="a line of mgeXE.toml for this game, in its section: \"[section] key = value\"")
    args = ap.parse_args()

    harness.refuse_if_busy()
    toml = os.path.join(harness.MO2_ROOT, "mods", args.config_mod, "Root", "mgeXE.toml")
    lines = [line for line in open(toml, encoding="utf-8").read().splitlines() if not line.strip().startswith("renderer_wgpu")]
    assert "[runtime]" in lines, toml
    for setting in args.set:
        section, setting = setting[1:].split("]", 1)
        setting = setting.strip()
        key = setting.split("=")[0].strip()
        start = lines.index("[%s]" % section) + 1
        end = next((i for i in range(start, len(lines)) if lines[i].startswith("[")), len(lines))
        at = next((i for i in range(start, end) if lines[i].split("=")[0].strip() == key), None)
        if at is None:
            lines.insert(start, setting)
        else:
            lines[at] = setting
    # The switch is written for each mode: the default of MGE XE is on.
    lines.insert(lines.index("[runtime]") + 1, "renderer_wgpu = %s" % ("false" if args.reference else "true"))
    switched = os.path.join(tempfile.mkdtemp(prefix="nirender-play-"), "mgeXE.toml")
    open(switched, "w", encoding="utf-8").write("".join(line + chr(10) for line in lines))

    mods = [(harness.RUN_MOD_PREFIX + "config", {"Root/mgeXE.toml": switched})]
    if not args.reference:
        mods.append((harness.RUN_MOD_PREFIX + "MGE XE", {"Root/d3d8.dll": os.path.join(args.mge_build, "d3d8.dll"),
                                                         "Root/numidium.exe": os.path.join(args.mge_build, "numidium.exe")}))

    deployment = harness.Deployment()
    try:
        for name, files in mods:
            deployment.use_mod(name, files)
        subprocess.Popen(harness.launch_command(), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        print("starting the game", flush=True)
        deadline = time.time() + 180
        while not harness.processes("Morrowind.exe"):
            if time.time() > deadline:
                print("no game after 180 s")
                return 1
            time.sleep(2)
        print("the game runs; leave it by its menu when you are done", flush=True)
        while harness.processes("Morrowind.exe"):
            time.sleep(2)
        print("the game ended")
    finally:
        harness.close_mo2()
        deployment.restore()
        if os.path.isfile(switched):
            os.remove(switched)
        os.rmdir(os.path.dirname(switched))
        print("the mod list of the testing instance is back")
    return 0


if __name__ == "__main__":
    sys.exit(main())
