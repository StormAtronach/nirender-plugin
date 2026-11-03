"""Runs the game without a Direct3D device: the d3d8.dll of this repository (or of a build of
MGE XE with --mge-build) takes the place of the one of MGE XE in the game folder for the run,
and the game gets the Direct3D 8 of the plugin. The host program draws the only picture, in
the window of the game.

    python tools/run_own_device.py --label owndevice [--backend vulkan]
        [--plugin-dll <akulakhan.dll>] [--log <file.txt>]
    python tools/run_own_device.py --label rel --archive <extracted release archive>  # the release check
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
plugin about its device. The first lines of progress.log, `[runner]`, and setup.txt say what the
run ran (`harness.write_setup`).
"""
import argparse
import os
import re
import subprocess
import sys
import tempfile

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


# The variables of the run itself, and the option or the place that sets each. `--env` does
# not take them: the runner is an allowed command (decision D6), so the DLL and the programs
# that a run loads come only from its own options, never from a free value.
RUNNER_VARIABLES = {
    "NIRENDER_PLUGIN_DLL": "--plugin-dll",
    "NIRENDER_HOST_EXE": "--mge-build (its numidium.exe), or the host build of this repository without it",
    "NIRENDER_HOST_LOG": "the runner (host.txt in the run folder)",
    "NIRENDER_BACKEND": "--backend",
    "NIRENDER_HOST_ARGS": "--start-frames and --window-shots",
}


def env_of(entries):
    """The variables of `--env NAME=VALUE`, in their order: a later value of a name wins. The
    value is all after the first `=`. A variable of the run itself is refused, with the option
    that sets it."""
    variables = {}
    for entry in entries:
        name, equals, value = entry.partition("=")
        if not equals or not name:
            raise SystemExit("--env takes NAME=VALUE, not %r" % entry)
        if name in RUNNER_VARIABLES:
            raise SystemExit("--env cannot set %s: the run sets it through %s" % (name, RUNNER_VARIABLES[name]))
        variables[name] = value
    return variables


def apply_env(environment, entries):
    """Puts the `--env` variables into `environment` (the runner's, which the game gets), then
    the default box time: a message box of the plugin, the helper or MGE XE closes itself after
    5 s, so that a run never waits for a click, and the box's text is in the log (the
    failure-path design, 5.1). A box time from `--env` or from the runner's environment is
    kept: a test of a box that stays up longer than a limit."""
    environment.update(env_of(entries))
    environment.setdefault("MGE_XE_BOX_SECONDS", "5")


def archive_arguments(args):
    """`--archive`: the run takes the build, the plugin and the MWSE.dll of the extracted release
    archive, and it builds nothing. The mod's Lua is the archive's too (`session.prepare`), and
    the instance's own mod `nirender` is off for the run: under the run's mod, Mod Organizer would
    serve a file that the archive lacks from it, and a faulty archive would pass."""
    if not args.archive:
        return
    if args.plugin_dll or args.mge_build:
        raise SystemExit("--archive names the build and the plugin: no --plugin-dll or --mge-build with it")
    args.mge_build = args.archive
    args.no_build = True
    if "nirender" not in args.disable_mod:
        args.disable_mod.append("nirender")
    args.plugin_dll = os.path.join(args.archive, "Data Files", "MWSE", "lib", "akulakhan.dll")
    mwse = os.path.join(args.archive, "MWSE.dll")
    if os.path.isfile(mwse) and not args.mwse_dll:
        args.mwse_dll = mwse


def build_commits(mge_build):
    """The line of the record of a run that names the commits of its build, from WHAT.txt of the build
    folder. A folder that does not say them is named as such: no commit is guessed."""
    if not mge_build:
        return "commits: no --mge-build"
    what = os.path.join(mge_build, "WHAT.txt")
    try:
        with open(what, encoding="utf-8", errors="replace") as file:
            found = re.search(r"nirender \S+ (\w+), render-link \S+ (\w+)", file.read())
    except OSError:
        found = None
    if not found:
        return "commits: %s does not say them" % mge_build
    return "commits: nirender %s, render-link %s, from %s" % (found.group(1), found.group(2), what)


def self_test_setup():
    """The record of what a run ran, with a fake instance: each file by its MD5 at the game's folder,
    the plugin from the mod that wins, the plugins in the order of their numbers."""
    harness = session.harness
    with tempfile.TemporaryDirectory() as root:
        game, profile = os.path.join(root, "game"), os.path.join(root, "profiles", "Default")
        plugin = os.path.join("mods", "High", harness.PLUGIN_FILE)
        for folder in (game, profile, os.path.join(root, os.path.dirname(plugin)), os.path.join(root, "mods", "Low", "MWSE", "lib"), os.path.join(root, "mods", "Off", "MWSE", "lib")):
            os.makedirs(folder)

        def write(path, text):
            with open(path, "w", encoding="ascii") as file:
                file.write(text)

        write(os.path.join(game, "MWSE.dll"), "abc")
        write(os.path.join(root, plugin), "abc")
        write(os.path.join(root, "mods", "Low", harness.PLUGIN_FILE), "low")
        write(os.path.join(root, "mods", "Off", harness.PLUGIN_FILE), "off")
        modlist = ["# generated", "-Off", "+High", "+Low"]
        abc = "900150983cd24fb0d6963f7d28e17f72"

        assert harness.winning_file(harness.PLUGIN_FILE, root, modlist, game) == ("mod High", os.path.join(root, plugin))
        assert harness.winning_file(harness.PLUGIN_FILE, root, ["-Off"], game) is None

        ini = "[Game Files]\nGameFile10=Ten.esp\nGameFile2=Two.esp\ngamefile0 = Morrowind.esm \n[Other]\nNotGameFile=x\n"
        assert harness.active_plugins(ini) == ["Morrowind.esm", "Two.esp", "Ten.esp"], harness.active_plugins(ini)
        assert harness.read_active_plugins(profile, game) == ([], None)
        write(os.path.join(game, "Morrowind.ini"), "GameFile0=Game.esm\n")
        assert harness.read_active_plugins(profile, game)[0] == ["Game.esm"]
        write(os.path.join(profile, "Morrowind.ini"), ini)
        plugins, source = harness.read_active_plugins(profile, game)
        assert plugins == ["Morrowind.esm", "Two.esp", "Ten.esp"] and source == os.path.join(profile, "Morrowind.ini")
        assert harness.load_order_md5(plugins) != harness.load_order_md5(list(reversed(plugins)))

        lines = harness.setup_lines(root, profile, game, modlist, plugins, source, ["commits: x"])
        assert lines[0] == "instance %s, profile Default" % root, lines[0]
        assert "MWSE.dll " + abc in lines and "d3d8.dll absent" in lines and "commits: x" in lines, lines
        assert "akulakhan.dll %s (mod High)" % abc in lines, lines
        assert not any(line.startswith(("dinput8.dll", "d3d9.dll")) for line in lines), lines
        write(os.path.join(game, "dinput8.dll"), "abc")
        write(os.path.join(game, "d3d9.dll"), "abc")
        with_optional = harness.setup_lines(root, profile, game, modlist, plugins, source)
        assert "dinput8.dll " + abc in with_optional and "d3d9.dll " + abc in with_optional, with_optional
        assert any(line.startswith("plugins 3, load order MD5 ") for line in lines), lines
        assert harness.setup_lines(root, profile, game, ["-Off"], [], None)[-2:] == ["akulakhan.dll absent", "plugins: no Morrowind.ini names any"]
        text = harness.setup_text(profile, modlist, plugins, source)
        assert "+High\n" in text and "Ten.esp\n" in text and text.index("+High") < text.index("[plugins]") < text.index("Morrowind.esm"), text

        # The commits: only what the folder of the build says.
        assert build_commits(None) == "commits: no --mge-build"
        assert "does not say" in build_commits(root)
        write(os.path.join(root, "WHAT.txt"), "f1 2026-11-02 15:00: the owner's play set, nirender master abc1, render-link feature/wgpu-renderer def2, no pdb; smoke: pending\n")
        assert build_commits(root).startswith("commits: nirender abc1, render-link def2, from "), build_commits(root)


def self_test_archive_lua():
    """A fake archive and a fake run mod in a temporary folder: the archive's file wins, a file
    that the archive's Lua loads and lacks leaves the run's mod (the fault of v0.3.3: main.lua
    without failure.lua), and the harness's scenario stays."""
    with tempfile.TemporaryDirectory() as folder:
        archive = os.path.join(folder, "archive", "Data Files", "MWSE", "mods", "nirender")
        dist = os.path.join(folder, "dist", "MWSE", "mods", "nirender")
        os.makedirs(archive)
        os.makedirs(dist)

        def write(where, name, text):
            with open(os.path.join(where, name), "w", encoding="utf-8") as file:
                file.write(text)

        write(archive, "main.lua", 'dofile("Data Files\\\\MWSE\\\\mods\\\\nirender\\\\failure.lua")\nrequire("nirender.notifier")\nrequire("nirender." .. spec.scenario)\n')
        write(dist, "main.lua", "the repo's")
        write(dist, "failure.lua", "the repo's")
        write(dist, "notifier.lua", "the repo's")
        write(dist, "owndevice.lua", "the scenario")
        shipped, missing = session.stage_archive_lua(os.path.join(folder, "archive"), os.path.join(folder, "dist"))
        assert shipped == ["main.lua"], shipped
        assert missing == ["failure.lua", "notifier.lua"], missing
        assert sorted(os.listdir(dist)) == ["main.lua", "owndevice.lua"], os.listdir(dist)
        assert "failure.lua" in open(os.path.join(dist, "main.lua"), encoding="utf-8").read()

        # With the files there, nothing is missing, and a file loaded through another is reached.
        write(archive, "failure.lua", 'dofile("Data Files/MWSE/mods/nirender/extra.lua")')
        write(archive, "notifier.lua", "")
        write(dist, "extra.lua", "the repo's")
        shipped, missing = session.stage_archive_lua(os.path.join(folder, "archive"), os.path.join(folder, "dist"))
        assert missing == ["extra.lua"], missing
        assert "extra.lua" not in os.listdir(dist)
        write(archive, "extra.lua", "")
        assert session.stage_archive_lua(os.path.join(folder, "archive"), os.path.join(folder, "dist"))[1] == []

        # No main.lua in the archive: refused.
        os.remove(os.path.join(archive, "main.lua"))
        try:
            session.stage_archive_lua(os.path.join(folder, "archive"), os.path.join(folder, "dist"))
        except SystemExit:
            pass
        else:
            raise AssertionError("an archive without main.lua was taken")


def saves_folder(profile, game_dir):
    """The folder of the game's saves in a profile: its own with Mod Organizer's `LocalSaves`,
    else the one of the game."""
    try:
        with open(os.path.join(profile, "settings.ini"), encoding="utf-8", errors="replace") as settings:
            local = any(line.strip().lower() == "localsaves=true" for line in settings)
    except OSError:
        local = False
    return os.path.join(profile, "saves") if local else os.path.join(game_dir, "Saves")


def save_problem(save, folder):
    """Why the run cannot load `save` from `folder`, with the newest saves that are there, or
    None. Without the check the game waits at the main menu for a load that never comes, until the
    budget ends. An empty name starts a new game: no save is needed."""
    if not save or os.path.isfile(os.path.join(folder, save)):
        return None
    try:
        saves = [name for name in os.listdir(folder) if name.lower().endswith(".ess")]
    except OSError:
        saves = []
    saves.sort(key=lambda name: os.path.getmtime(os.path.join(folder, name)), reverse=True)
    return "the save %s is not in %s; the newest there: %s. Name one with --save, or --save \"\" for a new game" % (save, folder, ", ".join(saves[:3]) or "none")


def pngs_from_tgas(folder):
    """Turns the TGA files of a folder into PNG files: `niPixelData:exportTGA` is what the game can write."""
    import picture_diff  # noqa: F401  (it finds the DLLs of Pillow under -I)
    from PIL import Image

    for root, _, names in os.walk(folder):
        for name in names:
            if name.endswith(".tga"):
                path = os.path.join(root, name)
                with Image.open(path) as picture:
                    picture.convert("RGB").save(path[:-4] + ".png")
                os.remove(path)


def self_test_pngs_from_tgas():
    import picture_diff  # noqa: F401  (it finds the DLLs of Pillow under -I)
    from PIL import Image

    with tempfile.TemporaryDirectory() as folder:
        os.mkdir(os.path.join(folder, "eng"))
        path = os.path.join(folder, "eng", "a.tga")
        Image.new("RGBA", (4, 4), (10, 20, 30, 255)).save(path)
        pngs_from_tgas(folder)
        assert not os.path.exists(path)
        with Image.open(path[:-4] + ".png") as picture:
            assert picture.getpixel((0, 0))[:3] == (10, 20, 30)


def self_test_save_problem():
    """A missing save is refused with its folder and the newest saves; a save that is there and a
    new game are not. The folder follows `LocalSaves`."""
    with tempfile.TemporaryDirectory() as root:
        profile, game = os.path.join(root, "profile"), os.path.join(root, "game")
        os.makedirs(os.path.join(profile, "saves"))
        os.makedirs(os.path.join(game, "Saves"))
        settings = os.path.join(profile, "settings.ini")
        assert saves_folder(profile, game) == os.path.join(game, "Saves")
        with open(settings, "w") as f:
            f.write("[General]\nLocalSaves=false\n")
        assert saves_folder(profile, game) == os.path.join(game, "Saves")
        with open(settings, "w") as f:
            f.write("[General]\nLocalSaves=true\n")
        folder = saves_folder(profile, game)
        assert folder == os.path.join(profile, "saves")
        assert save_problem("", folder) is None
        missing = save_problem("quiksave.ess", folder)
        assert "quiksave.ess" in missing and folder in missing and "none" in missing, missing
        for name, age in (("old.ess", 100), ("new.ess", 200), ("note.txt", 300)):
            with open(os.path.join(folder, name), "w"):
                pass
            os.utime(os.path.join(folder, name), (age, age))
        missing = save_problem("quiksave.ess", folder)
        assert "new.ess, old.ess" in missing and "note.txt" not in missing, missing
        assert save_problem("old.ess", folder) is None
        assert save_problem("old.ess", os.path.join(game, "Saves")) is not None
        assert save_problem("x.ess", os.path.join(root, "no such folder")) is not None


def self_test_archive_arguments():
    """`--archive` takes the archive's parts and turns the instance's own mod off, once."""
    args = argparse.Namespace(archive="E:/a", plugin_dll=None, mge_build=None, mwse_dll=None, no_build=False, disable_mod=[])
    archive_arguments(args)
    assert args.mge_build == "E:/a" and args.no_build
    assert args.plugin_dll == os.path.join("E:/a", "Data Files", "MWSE", "lib", "akulakhan.dll"), args.plugin_dll
    assert args.disable_mod == ["nirender"], args.disable_mod
    args = argparse.Namespace(archive="E:/a", plugin_dll=None, mge_build=None, mwse_dll=None, no_build=False, disable_mod=["nirender", "other"])
    archive_arguments(args)
    assert args.disable_mod == ["nirender", "other"], args.disable_mod
    args = argparse.Namespace(archive=None, plugin_dll=None, mge_build=None, mwse_dll=None, no_build=False, disable_mod=[])
    archive_arguments(args)
    assert args.disable_mod == [] and not args.no_build
    try:
        archive_arguments(argparse.Namespace(archive="E:/a", plugin_dll="E:/b.dll", mge_build=None, mwse_dll=None, no_build=False, disable_mod=[]))
    except SystemExit:
        pass
    else:
        raise AssertionError("--archive with --plugin-dll was taken")


def self_test():
    """The tests of the argument handling: no game, no build."""
    self_test_setup()
    self_test_save_problem()
    self_test_pngs_from_tgas()
    self_test_archive_lua()
    self_test_archive_arguments()
    build = os.path.join("target", "akulakhan.dll")
    assert plugin_dll_of("E:/a.dll", {"NIRENDER_PLUGIN_DLL": "E:/b.dll"}, build) == ("E:/a.dll", "--plugin-dll")
    assert plugin_dll_of(None, {"NIRENDER_PLUGIN_DLL": "E:/b.dll"}, build) == ("E:/b.dll", "NIRENDER_PLUGIN_DLL")
    assert plugin_dll_of(None, {}, build) == (build, "the release build of this repository")
    assert plugin_dll_of("", {}, build)[0] == build
    assert env_of([]) == {}
    assert env_of(["MGE_XE_BOX_SECONDS=90", "A=1=2", "B=", "A=3"]) == {"MGE_XE_BOX_SECONDS": "90", "A": "3", "B": ""}
    for bad in ["NOVALUE", "=1"] + ["%s=E:/x" % name for name in RUNNER_VARIABLES]:
        try:
            env_of([bad])
        except SystemExit as refusal:
            name = bad.partition("=")[0]
            if name in RUNNER_VARIABLES:
                assert RUNNER_VARIABLES[name] in str(refusal), refusal
        else:
            raise AssertionError("--env %s was taken" % bad)
    # The two names that load code: refused one by one, also after a good entry.
    for name in ("NIRENDER_PLUGIN_DLL", "NIRENDER_HOST_EXE"):
        environment = {}
        try:
            apply_env(environment, ["MGE_XE_BOX_SECONDS=90", "%s=E:/other.bin" % name])
        except SystemExit:
            pass
        else:
            raise AssertionError("--env %s was taken" % name)
        assert name not in environment
    # The box time through the function that main uses: --env wins over the default, a value
    # of the runner's environment is kept, and without either the default is 5 s.
    environment = {}
    apply_env(environment, ["MGE_XE_BOX_SECONDS=90"])
    assert environment["MGE_XE_BOX_SECONDS"] == "90"
    environment = {"MGE_XE_BOX_SECONDS": "20"}
    apply_env(environment, [])
    assert environment["MGE_XE_BOX_SECONDS"] == "20"
    environment = {"MGE_XE_BOX_SECONDS": "20"}
    apply_env(environment, ["MGE_XE_BOX_SECONDS=90"])
    assert environment["MGE_XE_BOX_SECONDS"] == "90"
    environment = {}
    apply_env(environment, ["MGE_XE_WATCHDOG=1"])
    assert environment == {"MGE_XE_WATCHDOG": "1", "MGE_XE_BOX_SECONDS": "5"}
    import io

    # The plugin's lines of a run: from mgeXE.log with the export, else from nirender.log.
    import shutil
    import tempfile

    folder = tempfile.mkdtemp(prefix="nirender-lines-")
    assert session.common.plugin_lines(folder) == []
    with open(os.path.join(folder, "nirender.log"), "w", encoding="utf-8") as own:
        own.write("own device: from the file\n")
    assert session.common.plugin_lines(folder) == ["own device: from the file"]
    with open(os.path.join(folder, "mgeXE.log"), "w", encoding="utf-8", newline="") as mge:
        mge.write("MGE-XE G7\r\nnirender: own device: made\r\nnirender: MISMATCH a call\r\n>> Using the wgpu renderer\r\n")
    assert session.common.plugin_lines(folder) == ["own device: made", "MISMATCH a call"], session.common.plugin_lines(folder)
    with open(os.path.join(folder, "mgeXE.log"), "w", encoding="utf-8") as mge:
        mge.write("MGE-XE G7 without the plugin's lines\n")
    assert session.common.plugin_lines(folder) == ["own device: from the file"]
    shutil.rmtree(folder)
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
    ap.add_argument("--engine-reference", action="store_true", help="run the same scenario on the engine's own renderer: `runtime.disabled` in "
                    "the run's mgeXE.toml, so the game has the system's Direct3D 8 and no MGE XE draws. The pictures are eng-<step>.png: the world with no interface, at 2048 x 1024. "
                    "MGE XE still applies its early patches, and MWSE and the Code Patch their own: not the game of 2002")
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
    ap.add_argument("--env", action="append", default=[], metavar="NAME=VALUE",
                    help="a variable for the game's process (the plugin, the helper and MGE XE read it), for this run only; "
                    "again for more. It wins over the runner's default box time (MGE_XE_BOX_SECONDS=5). The variables "
                    "of the run itself are refused, with the option that sets each: " + ", ".join(sorted(RUNNER_VARIABLES)))
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--archive", help="the release check: a folder with an extracted release archive. The run takes its build, its plugin, its "
                    "MWSE.dll and its Lua files; a Lua file that the archive loads and lacks is not in the run, as in a player's game")
    args = ap.parse_args()
    archive_arguments(args)
    if args.self_test:
        self_test()
        return 0
    if args.engine_reference:
        args.reference = True
        args.set.append("[runtime] disabled = true")
    # The game is started from this process: its variables go into the game's environment.
    apply_env(os.environ, args.env)
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
    problem = save_problem(args.save, saves_folder(session.harness.PROFILE, session.harness.GAME_DIR))
    if problem:
        raise SystemExit("run_own_device: " + problem)
    if not args.no_build:
        cargo = os.path.join(os.path.expanduser("~"), ".cargo", "bin", "cargo.exe")
        for folder in (HOST_DIR, D3D8_DIR):
            subprocess.run([cargo, "build", "--release", "--quiet"], cwd=folder, check=True)
    session.prepare(build=not args.no_build, extra_files=extra_files, archive=args.archive)
    run = session.Run(args.label)
    # Without a save (--save ""), the scenario starts a new game: the films of the game play.
    spec = run.spec("owndevice", save=args.save or None, interior=args.interior, exterior=args.exterior, dir=run.dir.replace("\\", "/"), steps=args.steps,
                    position=args.position, yaw=args.yaw, findWater=args.find_water, findDry=args.find_dry, hour=args.hour, weather=args.weather,
                    lua=open(args.lua_file, encoding="utf-8").read() if args.lua_file else None,
                    shaderBatch=open(args.shader_batch, encoding="utf-8").read() if args.shader_batch else None,
                    tourCells=args.tour_cells, tour=args.tour, tourSeconds=args.tour_seconds, tourDual=args.tour_dual or None, tourExtra=args.tour_extra or None, captureMenu=args.capture_menu, settleSeconds=args.settle, portTimes=args.port_times or None,
                    overlay=args.overlay or None, earlyPicture=run.path("early.png").replace("\\", "/") if args.early_picture else None,
                    afterLoadPictures=args.after_load_pictures or None, keepFade=args.keep_fade or None, keepAI=args.keep_ai or None,
                    engineReference=args.engine_reference or None)
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
    done = run.launch(spec, args.budget, TAG, mods=mods, disable=args.disable_mod, facts=[build_commits(args.mge_build)])

    print()
    for line in run.progress_lines(TAG):
        if line.startswith(("own device", "step", "after", "steps done", "ERROR", "save loaded", "rate", "zoom")):
            print("game: " + line[:200])
    if os.path.isfile(run.path("host.txt")):
        print("\nhost:")
        print("  " + open(run.path("host.txt"), errors="replace").read().strip().replace("\n", "\n  ")[-3000:])
    else:
        print("\nhost: no text; the host program did not start")
    # The plugin's lines: in mgeXE.log with MGE XE's d3d8.dll. Without its export the plugin
    # writes into nirender.log of the game folder until Lua names the log of the run.
    lines = run.plugin_lines()
    game_log = os.path.join(session.harness.GAME_DIR, "nirender.log")
    if os.path.isfile(game_log) and os.path.getmtime(game_log) > run.started:
        lines = open(game_log, encoding="utf-8", errors="replace").read().splitlines() + lines
    for line in lines:
        if line.startswith(("own device", "stream:", "capture: end", "own host", "install:", "link:")):
            print("plugin: " + line.strip()[:240])
    if args.engine_reference:
        pngs_from_tgas(run.dir)
    print("pictures in " + run.dir)
    # The size of the run and of every run kept (the owner, 2026-10-18: the results folder
    # had grown to hundreds of GB unnoticed). The runner deletes nothing; the line says when
    # it is time to.
    print("size: this run %s; all runs in %s %s" % (folder_size(run.dir), os.path.dirname(run.dir), folder_size(os.path.dirname(run.dir))))
    print_disk_limits()
    return 0 if done else 1


def print_disk_limits():
    """The limits of the data rule (D71), from the slot tool beside the harness. A fault here never stops a run."""
    try:
        import slot

        for line in slot.runner_disk_lines():
            print(line)
    except Exception as error:  # noqa: BLE001 - only a line of a report
        print("the limits of the disks are not known: %s" % error)


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
