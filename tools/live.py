"""Starts and stops a live game of the testing instance, for work through the mwse-mcp tools.

The mwse-mcp server talks to a game that runs. It does not start one. This tool does the
parts that the server does not do:

    python tools/live.py install            # put the bridge mod into the testing instance
    python tools/live.py start              # start the game; it stops at the main menu
    python tools/live.py stop

After `start`, use the MCP tools (mw_eval_lua, mw_get_state, ...) for everything in the game:
to load a save, to set god mode, to read state.

The bridge is the Lua mod `mwseMCP` of the package. It listens on 127.0.0.1:31415 in the
game. It makes Lua tables and JSON for each request, so do not use it during a measurement
of frame times.
"""
import argparse
import os
import shutil
import socket
import subprocess
import sys
import time

import session  # noqa: F401  (puts the harness on the path)
import harness
from common import local

PACKAGE = local("mwse_mcp", os.path.join(os.path.expanduser("~"), "tools", "mwse-mcp", "mwse-mcp"))
MOD_NAME = "mwseMCP"
HOST, PORT = "127.0.0.1", 31415


def install():
    """Copies the bridge mod into the testing instance and switches it on in the mod list."""
    source = os.path.join(PACKAGE, "mwse", "mwseMCP")
    target = os.path.join(harness.MO2_ROOT, "mods", MOD_NAME, "MWSE", "mods", "mwseMCP")
    if os.path.isdir(target):
        shutil.rmtree(target)
    shutil.copytree(source, target)
    modlist = os.path.join(harness.PROFILE, "modlist.txt")
    with open(modlist, encoding="utf-8", newline="") as fh:
        lines = fh.read().splitlines(keepends=True)
    ending = "\r\n" if lines and lines[0].endswith("\r\n") else "\n"
    on, off = "+" + MOD_NAME, "-" + MOD_NAME
    if any(line.strip() == on for line in lines):
        print("the mod is on in modlist.txt")
        return
    if any(line.strip() == off for line in lines):
        lines = [on + ending if line.strip() == off else line for line in lines]
    else:
        lines.insert(1 if lines and lines[0].startswith("#") else 0, on + ending)
    with open(modlist, "w", encoding="utf-8", newline="") as fh:
        fh.write("".join(lines))
    print("bridge mod installed and switched on")


def bridge_is_up():
    """True when the bridge accepts a connection. The bridge keeps only its newest client, so
    this test takes the bridge from the MCP server for a moment. The server connects again."""
    try:
        socket.create_connection((HOST, PORT), timeout=2).close()
        return True
    except OSError:
        return False


def start():
    harness.refuse_if_busy()
    subprocess.Popen(harness.launch_command(), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    print("starting the game", flush=True)
    deadline = time.time() + 180
    while time.time() < deadline:
        if bridge_is_up():
            print("the bridge is up; the game is at the main menu")
            return 0
        time.sleep(2)
    print("no bridge on %s:%d after 180 s: the game did not start, or the mod is not installed" % (HOST, PORT))
    return 1


def stop():
    game = [pid for pid, _ in harness.processes("Morrowind.exe")]
    harness.kill_pids(game)
    harness.wait_gone(game)
    # Mod Organizer ends by itself after Root Builder has cleared the game folder.
    harness.close_mo2()
    print("stopped: %d game" % len(game))
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("command", choices=("install", "start", "stop"))
    args = ap.parse_args()
    if args.command == "install":
        install()
        return 0
    return start() if args.command == "start" else stop()


if __name__ == "__main__":
    sys.exit(main())
