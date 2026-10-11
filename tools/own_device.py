"""Puts the testing instance into the mode without a Direct3D device, for a game by hand, and
takes it out again.

    python tools/own_device.py on       the d3d8.dll of this repository takes the place of the
                                        one of MGE XE; start the game from Mod Organizer
    python tools/own_device.py off      the d3d8.dll of MGE XE comes back
    python tools/own_device.py status

"on" installs the nirender mod as it is built now, with the host program beside the plugin.
The d3d8.dll of MGE XE stays beside its place with the name d3d8.dll.mge-xe.
"""
import os
import shutil
import sys

import session
from common import REPO

MGE_MOD = "MGE XE G7 Fork"
HOST = os.path.join(REPO, "host", "target", "x86_64-pc-windows-msvc", "release", "nirender-host.exe")
D3D8 = os.path.join(REPO, "d3d8", "target", "i686-pc-windows-msvc", "release", "d3d8.dll")


def paths():
    root = session.harness.MO2_ROOT
    theirs = os.path.join(root, "mods", MGE_MOD, "Root", "d3d8.dll")
    return theirs, theirs + ".mge-xe", os.path.join(root, "mods", session.MOD_NAME, "MWSE", "lib", "nirender-host.exe")


def is_on(theirs):
    return os.path.isfile(theirs) and os.path.isfile(D3D8) and session.harness.same_file_content(theirs, D3D8)


def main():
    action = sys.argv[1] if len(sys.argv) > 1 else "status"
    theirs, kept, host = paths()
    game = os.path.join(session.harness.GAME_DIR, "d3d8.dll")
    if action == "on":
        for built in (D3D8, HOST, session.DLL):
            if not os.path.isfile(built):
                raise SystemExit("not built: %s" % built)
        if session.harness.processes("Morrowind.exe"):
            raise SystemExit("Morrowind is running; close it first")
        session.prepare(build=False)
        if not is_on(theirs):
            if os.path.exists(kept):
                raise SystemExit("%s is there already: look at it first" % kept)
            shutil.copy2(theirs, kept)
            shutil.copy2(D3D8, theirs)
        shutil.copy2(HOST, host)
        print("on: the game of the testing instance has no Direct3D device now. Start it from Mod Organizer.")
    elif action == "off":
        if os.path.isfile(kept):
            shutil.copy2(kept, theirs)
            os.remove(kept)
            # Mod Organizer leaves its copy in the game folder when the game does not end by itself.
            if os.path.isfile(game) and not session.harness.same_file_content(theirs, game):
                shutil.copy2(theirs, game)
        if os.path.isfile(host):
            os.remove(host)
        print("off: the d3d8.dll of MGE XE is in its place.")
    else:
        print("on" if is_on(theirs) else "off")
    return 0


if __name__ == "__main__":
    sys.exit(main())
