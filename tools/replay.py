"""Draws a frame of a capture file with the MGE XE helper, offline: the same picture from the
same file each time. For a check of a change of the renderer against the picture before it.

    python tools/replay.py <capture file> <picture.png> [--frame N] [--mge-build DIR]
    python tools/replay.py <capture file> <prefix> --frames-at 1,3,5 [--mge-build DIR]

With --frames-at, one run of the helper writes <prefix>-<frame as 5 digits>.png for each
frame named (the frames count from 0, as --frame): one start of the helper for a capture of
many stops, in place of one for each. With --marks-every N, the helper prints a mark (its
time for each frame by part) after every N frames: the bench of a long capture.
        [--config-root DIR] [--distant-land DIR] [--backend vulkan|dx12]
        [--debug-view NAME [TERM]] [--debug-lines NAME,NAME]

--debug-view and --debug-lines set the debug view of the replay (the helper's names: lights,
shadow-layers, shadow-map, water TERM, post TERM; the line sources lights, shadow-boxes,
bounds, lua). A replay takes its view only from these arguments: a view or a line source that
a record of the capture sets is not drawn.

The helper reads mgeXE.toml from --config-root (the Root folder of the Configuration mod of
the testing instance as the default) and the distant land from --distant-land (the overwrite
folder of the testing instance as the default). The archives of the game come from --game-dir;
a texture that is only in a mod folder is white: the same in each picture.
"""
import argparse
import os
import subprocess
import sys

import session
from common import SIBLINGS, build_missing, capture_format_line, local

MGE_BUILD = local("mge_build", os.path.join(SIBLINGS, "MGE-XE-G7-render-link", "bin", "MSVC-Release"))
CONFIG_ROOT = local("config_root", os.path.join(session.harness.MO2_ROOT, "mods", "Configuration", "Root"))
DISTANT_LAND = local("distant_land", os.path.join(session.harness.MO2_ROOT, "overwrite", "distantland"))
GAME_DIR = local("game_dir", session.harness.GAME_DIR)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("capture")
    ap.add_argument("picture")
    ap.add_argument("--frame", type=int)
    ap.add_argument("--frames-at", help="frames to write, with commas; the picture argument is then the prefix of the files")
    ap.add_argument("--mge-build", default=MGE_BUILD)
    ap.add_argument("--config-root", default=CONFIG_ROOT)
    ap.add_argument("--distant-land", default=DISTANT_LAND)
    ap.add_argument("--game-dir", default=GAME_DIR, help="the game folder with Morrowind.ini and the archives")
    ap.add_argument("--backend", default="vulkan")
    ap.add_argument("--anisotropy", type=int, default=None, help="the anisotropic filter of the textures the game filters (1 without this)")
    ap.add_argument("--marks-every", type=int, default=None, help="a mark after every N frames: the helper's time for each frame by part")
    ap.add_argument("--debug-view", nargs="+", metavar=("NAME", "TERM"), help="the shader view of the replay, and its term for water and post")
    ap.add_argument("--debug-lines", metavar="NAME,NAME", help="the line sources of the replay, with commas")
    args = ap.parse_args()
    if args.debug_view and len(args.debug_view) > 2:
        ap.error("--debug-view takes a name and at most one term")
    # A replay is build work. The slot hook reads only a command line, so a replay that a script or
    # another tool starts is checked here. A harness from before the check has no such function.
    check_build_slot = getattr(session.harness, "check_build_slot", None)
    if check_build_slot:
        check_build_slot()
    missing = build_missing(args.mge_build)
    if missing:
        sys.stderr.write("replay: the build folder %s has no %s: a replay without them draws another picture\n" % (args.mge_build, ", ".join(missing)))
        return 2
    exe = os.path.join(args.mge_build, "numidium.exe")
    command = [exe, "--replay", os.path.abspath(args.capture), os.path.abspath(args.picture), "--backend", args.backend]
    if args.frames_at:
        command += ["--frames-at", args.frames_at]
    elif args.frame is not None:
        command += ["--frame", str(args.frame)]
    if args.anisotropy is not None:
        command += ["--anisotropy", str(args.anisotropy)]
    if args.marks_every:
        command += ["--marks-every", str(args.marks_every)]
    # The helper checks the names and the term.
    if args.debug_view:
        command += ["--debug-view"] + args.debug_view
    if args.debug_lines is not None:
        command += ["--debug-lines", args.debug_lines]
    env = dict(os.environ, MGE_XE_DISTANT_LAND_DIR=args.distant_land, MGE_XE_GAME_DIR=args.game_dir)
    result = subprocess.run(command, cwd=args.config_root, env=env, capture_output=True, text=True)
    sys.stdout.write(result.stdout)
    sys.stderr.write(result.stderr)
    # The helper's refusal names the parts of a game; in a replay the capture is the other part.
    line = capture_format_line(os.path.basename(args.capture), result.returncode, result.stdout + result.stderr)
    if line:
        sys.stderr.write(line + "\n")
    return result.returncode


if __name__ == "__main__":
    sys.exit(main())
