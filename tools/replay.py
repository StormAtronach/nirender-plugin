"""Draws a frame of a capture file with the MGE XE helper, offline: the same picture from the
same file each time. For a check of a change of the renderer against the picture before it.

    python tools/replay.py <capture file> <picture.png> [--frame N] [--mge-build DIR]
    python tools/replay.py <capture file> <prefix> --frames-at 1,3,5 [--mge-build DIR]

With --frames-at, one run of the helper writes <prefix>-<frame as 5 digits>.png for each
frame named (the frames count from 0, as --frame): one start of the helper for a capture of
many stops, in place of one for each.
        [--config-root DIR] [--distant-land DIR] [--backend vulkan|dx12]

The helper reads mgeXE.toml from --config-root (the Root folder of the Configuration mod of
the testing instance as the default) and the distant land from --distant-land (the overwrite
folder of the testing instance as the default). The archives of the game come from --game-dir;
a texture that is only in a mod folder is white: the same in each picture.
"""
import argparse
import os
import subprocess
import sys

MGE_BUILD = "C:/Users/ivan_/source/repos/MGE-XE-G7-render-link/bin/MSVC-Release"
CONFIG_ROOT = "D:/Modlists/Morrowind75testing/mods/Configuration/Root"
DISTANT_LAND = "D:/Modlists/Morrowind75testing/overwrite/distantland"
GAME_DIR = "D:/GOG/Morrowind/Morrowind"


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
    args = ap.parse_args()
    exe = os.path.join(args.mge_build, "mgeHost64.exe")
    command = [exe, "--replay", os.path.abspath(args.capture), os.path.abspath(args.picture), "--backend", args.backend]
    if args.frames_at:
        command += ["--frames-at", args.frames_at]
    elif args.frame is not None:
        command += ["--frame", str(args.frame)]
    if args.anisotropy is not None:
        command += ["--anisotropy", str(args.anisotropy)]
    env = dict(os.environ, MGE_XE_DISTANT_LAND_DIR=args.distant_land, MGE_XE_GAME_DIR=args.game_dir)
    result = subprocess.run(command, cwd=args.config_root, env=env, capture_output=True, text=True)
    sys.stdout.write(result.stdout)
    sys.stderr.write(result.stderr)
    return result.returncode


if __name__ == "__main__":
    sys.exit(main())
