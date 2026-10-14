"""Puts the testing instance into the mode without a Direct3D device, for a game by hand, and
takes it out again.

    python tools/own_device.py on       the d3d8.dll of this repository takes the place of the
                                        one of MGE XE; start the game from Mod Organizer
    python tools/own_device.py off      the instance has its own MGE XE and plugin again
    python tools/own_device.py status

"on" puts the plugin as it is built now, with the host program beside it, into the mod
"Harness - nirender", and the d3d8.dll of this repository into "Harness - MGE XE", and switches
both on at the top of the mod list. The mods of the instance are not written. "off" switches
the two off. A run of the runners also switches them off before it starts.
"""
import os
import shutil
import sys

import session
from common import REPO

MGE_MOD = session.harness.RUN_MOD_PREFIX + "MGE XE"
HOST = os.path.join(REPO, "host", "target", "x86_64-pc-windows-msvc", "release", "nirender-host.exe")
D3D8 = os.path.join(REPO, "d3d8", "target", "i686-pc-windows-msvc", "release", "d3d8.dll")


def is_on():
    lines, _ = session.harness.read_modlist()
    return "+" + session.MOD_NAME in lines and "+" + MGE_MOD in lines


def main():
    action = sys.argv[1] if len(sys.argv) > 1 else "status"
    harness = session.harness
    if action == "on":
        for built in (D3D8, HOST, session.DLL):
            if not os.path.isfile(built):
                raise SystemExit("not built: %s" % built)
        session.prepare(build=False)
        shutil.copy2(HOST, os.path.join(harness.MO2_ROOT, "mods", session.MOD_NAME, "MWSE", "lib", "nirender-host.exe"))
        harness.fill_mod(MGE_MOD, {"Root/d3d8.dll": D3D8})
        harness.set_mod_state(session.MOD_NAME, True, first=True)
        harness.set_mod_state(MGE_MOD, True, first=True)
        print("on: the game of the testing instance has no Direct3D device now. Start it from Mod Organizer.")
    elif action == "off":
        if harness.processes("Morrowind.exe"):
            raise SystemExit("Morrowind is running; close it first")
        for name in (session.MOD_NAME, MGE_MOD):
            lines, _ = harness.read_modlist()
            if "+" + name in lines:
                harness.set_mod_state(name, False)
        print("off: the instance has its own MGE XE and plugin.")
    else:
        print("on" if is_on() else "off")
    return 0


if __name__ == "__main__":
    sys.exit(main())
