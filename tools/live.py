"""A live game of the testing instance, driven through the mwseMCP bridge.

The bridge is the Lua mod of the mwse-mcp package. It listens on 127.0.0.1:31415 in the game
and runs Lua that a client sends. This tool is a client of the bridge. It does not need the
MCP server of the package, so it also works in a session that has no MCP tools.

    python tools/live.py install            # put the bridge mod into the testing instance
    python tools/live.py start [--save quiksave.ess]
    python tools/live.py eval "return tes3.player.cell.editorName"
    python tools/live.py eval --file probe.lua
    python tools/live.py state
    python tools/live.py stop

`eval` prints the result as JSON. Return numbers, strings, booleans and tables of them: a game
object comes back as a stub. The Lua runs in the game on the next frame, so the game must
draw frames: a request during a load screen ends in a timeout.

The bridge makes Lua tables and JSON for each request. Do not use it during a measurement of
frame times.
"""
import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import time

import session  # noqa: F401  (puts the harness on the path)
import harness

PACKAGE = r"C:\Users\ivan_\tools\mwse-mcp\mwse-mcp"
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


class Bridge:
    """One connection to the bridge. The bridge keeps only the newest client."""

    def __init__(self, timeout=10.0):
        self.sock = socket.create_connection((HOST, PORT), timeout=timeout)
        self.buffer = b""
        self.next_id = 1
        self.hello = self._line()

    def _line(self):
        while b"\n" not in self.buffer:
            data = self.sock.recv(65536)
            if not data:
                raise ConnectionError("the bridge closed the connection")
            self.buffer += data
        line, self.buffer = self.buffer.split(b"\n", 1)
        return json.loads(line.decode("utf-8"))

    def request(self, op, **params):
        request_id = self.next_id
        self.next_id += 1
        self.sock.sendall((json.dumps({"id": request_id, "op": op, "params": params}) + "\n").encode("utf-8"))
        while True:
            answer = self._line()
            if answer.get("id") == request_id:
                return answer

    def eval(self, source):
        """Runs Lua in the game. Returns the result. Raises RuntimeError with the Lua error."""
        answer = self.request("eval", source=source)
        if not answer.get("ok"):
            error = answer.get("error", {})
            raise RuntimeError("%s: %s\n%s" % (error.get("phase"), error.get("message"), error.get("traceback", "")))
        return answer.get("result")


def connect(wait=0.0):
    """A connection to the bridge. Tries for `wait` seconds while the game starts."""
    deadline = time.time() + wait
    while True:
        try:
            return Bridge()
        except OSError:
            if time.time() >= deadline:
                raise SystemExit("no bridge on %s:%d: the game does not run, or the mod is not installed" % (HOST, PORT))
            time.sleep(2)


def start(save):
    harness.refuse_if_busy()
    subprocess.Popen(harness.launch_command(), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    print("starting the game; the bridge comes up at the main menu", flush=True)
    bridge = connect(wait=180)
    print("bridge up: MWSE %s, protocol %s" % (bridge.hello.get("mwse"), bridge.hello.get("protocolVersion")))
    if save:
        # The load takes longer than one request. The answer can be late or missing.
        try:
            bridge.eval("tes3.loadGame(%s) return true" % json.dumps(save))
        except (OSError, RuntimeError, ConnectionError) as problem:
            print("load: %s" % str(problem).splitlines()[0])
        for _ in range(60):
            time.sleep(2)
            try:
                cell = connect().eval("return tes3.player and tes3.player.cell and tes3.player.cell.editorName")
            except (SystemExit, OSError, RuntimeError, ConnectionError):
                continue
            if cell:
                # A dead player ends the game.
                connect().eval("tes3.worldController.menuController.godModeEnabled = true")
                print("save loaded, cell '%s', god mode on" % cell)
                return
        print("the save did not load in 120 s")


def stop():
    game = [pid for pid, _ in harness.processes("Morrowind.exe")]
    harness.kill_pids(game)
    harness.wait_gone(game)
    ours = harness.testing_mo2()
    harness.kill_pids(ours)
    harness.wait_gone(ours)
    print("stopped: %d game, %d Mod Organizer" % (len(game), len(ours)))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="command", required=True)
    sub.add_parser("install")
    start_parser = sub.add_parser("start")
    start_parser.add_argument("--save", default="quiksave.ess", help="'' starts at the main menu")
    eval_parser = sub.add_parser("eval")
    eval_parser.add_argument("source", nargs="?")
    eval_parser.add_argument("--file")
    sub.add_parser("state")
    sub.add_parser("stop")
    args = ap.parse_args()

    if args.command == "install":
        install()
    elif args.command == "start":
        start(args.save)
    elif args.command == "stop":
        stop()
    elif args.command == "state":
        print(json.dumps(connect().request("getState").get("result"), indent=1))
    else:
        source = open(args.file, encoding="utf-8").read() if args.file else args.source
        try:
            print(json.dumps(connect().eval(source), indent=1))
        except RuntimeError as problem:
            print("LUA ERROR " + str(problem))
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
