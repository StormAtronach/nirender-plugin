"""Starts a game of the testing instance to play by hand, with the wgpu renderer of a sealed set,
and puts the files of the instance back when the game ends.

    python tools/play.py [--mge-build SET] [--reference] [--set "[section] key = value"]
                         [--plugin] [--livecoding FILE] [--stage-only]
                         [--instance DIR --game-dir DIR --profile NAME --config-mod NAME]

The testing instance at rest has the release of MGE XE and of the plugin. This tool does for a
game by hand what the runner does for a scenario (W521): --mge-build names a sealed set
(tools/build_set.py makes one), and without it the game takes the current set (MGE_BUILD of
current.bat). sets.verify checks the set first; no set, or a set that fails the check, stops
the tool with one line and no game. The files come from the runner's map (sets.stage_files):
"Harness - MGE XE" with the set's MGE XE files, MWSE.dll, lua51.dll and the MWSE core, and
"Harness - config" with mgeXE.toml with the switch `renderer_wgpu`, both on for this game only,
over the mods of the instance. With --reference: the same set with the switch off, on
Direct3D 9.

--instance runs the game in another Mod Organizer instance, with --game-dir its game folder,
--profile its profile and --config-mod the mod that holds its mgeXE.toml. For Outlander-test
play-manual-check-outlander.bat has them (the instance's folder comes from local.bat).
--stage-only puts the mods in place, prints them, and takes them out again without a game: a
check of the staging in an instance.

--plugin puts the set's plugin, its Lua and its test files into "Harness - nirender" for the
game, over the release of the instance. Without it the game has the instance's release of the
plugin. --plugin-dll is gone: it stops the tool with a line that names build_set.py. The
variable NIRENDER_PLUGIN_DLL is read by nothing: a game prints one line and goes on (the runner
stops on it).
--livecoding FILE adds the Livecoding mod to "Harness - nirender": its main.lua, read from
the instance's own Livecoding mod, and FILE as its livecoding.lua (Alt+X in the game loads
it). The staged copy of FILE is in the Harness mod: an edit there during the game shows at
the next Alt+X.

Close Mod Organizer of the testing instance first. Leave the game by its menu: the tool
waits for that, then stops its Mod Organizer and puts the mod list back. Then it prints how the
game closed: its exit code, and whether the fresh MWSE.log has a crash line, with its first lines.
This report only reads, and a fault in it stops nothing.
"""
import argparse
import contextlib
import ctypes
import os
import re
import subprocess
import sys
import tempfile
import time

# The modules of this folder, also under `python -I` (the self-test runs so).
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import session  # noqa: E402, F401  (puts the harness on the path)
import harness  # noqa: E402
import run_own_device  # noqa: E402  (the set of a run and its staging, W521)
import sets  # noqa: E402

# The line of a game with no set (D 2.5): the tools that make one and that make it the current set.
NO_SET_HINT = ("make a set with tools/build_set.py and name it with --mge-build, or make it the current set with "
               "tools/deploy_rest.py (it writes current.bat)")
# The testing instance, for files that another instance lacks (the Livecoding mod).
TESTING_INSTANCE = harness.MO2_ROOT


PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
CRASH_LINE = "Morrowind has crashed"
CRASH_DETAIL = re.compile(r"Playtime|Thread:|Exception:|^0x")


def kernel32():
    library = ctypes.WinDLL("kernel32")
    library.OpenProcess.restype = ctypes.c_void_p
    library.GetExitCodeProcess.argtypes = (ctypes.c_void_p, ctypes.POINTER(ctypes.c_ulong))
    library.CloseHandle.argtypes = (ctypes.c_void_p,)
    return library


def game_handle(pid):
    """A handle that keeps the game's exit code after the game has gone, or None."""
    return kernel32().OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)


def exit_code_text(handle):
    """The exit code of an ended process as text, or a text that says why there is none."""
    if not handle:
        return "not read: the game gave no handle"
    library = kernel32()
    code = ctypes.c_ulong()
    read = library.GetExitCodeProcess(handle, ctypes.byref(code))
    library.CloseHandle(handle)
    return "0x%08X" % code.value if read else "not read"


def close_lines(logs, since):
    """What the fresh MWSE.log says about the close, as lines of text. A log is fresh when it was written
    after `since`. Mod Organizer moves a live log to overwrite/Root when it closes: call this after that."""
    fresh = [path for path in logs if os.path.isfile(path) and os.path.getmtime(path) >= since]
    if not fresh:
        return ["MWSE.log: no fresh log in %s" % ", ".join(logs)]
    path = max(fresh, key=os.path.getmtime)
    with open(path, encoding="utf-8", errors="replace") as file:
        lines = file.read().splitlines()
    at = next((index for index, line in enumerate(lines) if CRASH_LINE in line), None)
    if at is None:
        return ["MWSE.log: no crash line in %s" % path]
    details = [line[:110] for line in lines[at + 1:at + 40] if CRASH_DETAIL.search(line)]
    return ["MWSE.log: CRASH at line %d of %s" % (at + 1, path)] + ["  " + line for line in details[:10]]


def wait_for_start(seconds):
    """True when a Morrowind.exe runs within `seconds`. A read of the processes that fails is not "no game": the wait
    goes on, and its last line says that the list was not known (W524 F-T-13)."""
    deadline = time.time() + seconds
    not_known = None
    while True:
        try:
            if harness.processes("Morrowind.exe"):
                return True
            not_known = None
        except harness.ProcessesNotKnown as error:
            not_known = error
        if time.time() >= deadline:
            print("the start of the game is not known after %d s: %s" % (seconds, not_known) if not_known else "no game after %d s" % seconds)
            return False
        time.sleep(2)


def wait_for_end():
    """Returns when no Morrowind.exe runs, by a list that was read. A failed read counts as a game that runs
    (harness.any_running): the files of the instance must not go back under the owner's game (W524 F-T-13)."""
    while harness.any_running("Morrowind.exe"):
        time.sleep(2)


def report_close(handle, started):
    """Prints how the game closed. This only reads, and no fault here may stop the owner's close."""
    try:
        print("the game closed after %.0f s, exit code %s" % (time.time() - started, exit_code_text(handle)), flush=True)
        for line in close_lines(harness.MWSE_LOGS, started):
            print(line, flush=True)
    except Exception as error:  # noqa: BLE001 - the report must never stop a close
        print("the report of the close failed: %s" % error, flush=True)


def self_test():
    """The tests of the report of a close: no game."""
    child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(0.5); raise SystemExit(3)"])
    handle = game_handle(child.pid)
    child.wait()
    assert exit_code_text(handle) == "0x00000003"
    assert "no handle" in exit_code_text(None)

    with tempfile.TemporaryDirectory() as folder:
        live, overwrite = os.path.join(folder, "live.log"), os.path.join(folder, "overwrite.log")
        missing = close_lines((overwrite, live), time.time())
        assert "no fresh log" in missing[0], missing

        with open(live, "w", encoding="ascii") as file:
            file.write("old\n%s\n" % CRASH_LINE)
        os.utime(live, (1000, 1000))
        with open(overwrite, "w", encoding="ascii") as file:
            file.write("a line\nanother\n")
        now = os.path.getmtime(overwrite)
        lines = close_lines((overwrite, live), now - 1)
        assert "no crash line" in lines[0] and overwrite in lines[0], lines

        with open(overwrite, "w", encoding="ascii") as file:
            file.write("a line\n%s!\nPlaytime: 00:05:35\nnoise\nThread: GameMainThread\nException: X\n0x001 | d3d8 | a |\n" % CRASH_LINE)
        lines = close_lines((overwrite, live), now - 1)
        assert "CRASH at line 2" in lines[0] and len(lines) == 5, lines
        assert "noise" not in " ".join(lines), lines

    # A fault in the report stays in the report.
    harness_logs = harness.MWSE_LOGS
    harness.MWSE_LOGS = 7
    report_close(None, time.time())
    harness.MWSE_LOGS = harness_logs
    # The toml of a game: the vsync of the instance stays, a section that the file lacks is made, and a new key
    # goes to the end of its section.
    text = '[runtime]\nrenderer_wgpu = false\nvsync = "default"\n\n[distant_land]\nfar = 5\n\n[statics]\nenabled = false\n'
    lines = session.run_toml_lines(text, ["[views] full_tier_max = 4", "[distant_land] near = 1"], "test.toml", True, vsync_immediate=False)
    assert lines[:3] == ["[runtime]", "renderer_wgpu = true", 'vsync = "default"'], lines
    assert lines[lines.index("far = 5") + 1] == "near = 1", lines
    assert lines[-3:] == ["", "[views]", "full_tier_max = 4"], lines
    self_test_set()
    self_test_not_known()
    print("play self-test: ok")


def self_test_not_known():
    """W524 F-T-13: a failed read of the processes ends no wait of a game. The plant: the reads fail three times, then
    say that no game runs. The null: before F-T-13 a failed read was an empty list, so the wait for the end stopped at
    its first read, while the owner's game could run."""
    saved_processes, saved_sleep = harness.processes, time.sleep
    reads = []

    def failing_reads(failures):
        def read(name, tries=3):
            reads.append(name)
            if len(reads) <= failures:
                raise harness.ProcessesNotKnown("the list of the %s processes was not read (planted)" % name)
            return []
        return read

    try:
        time.sleep = lambda seconds: None
        harness.processes = failing_reads(3)
        wait_for_end()
        assert len(reads) == 4, reads
        # The null: the reads of the harness before F-T-13, an empty list at once.
        reads.clear()
        harness.processes = failing_reads(0)
        wait_for_end()
        assert len(reads) == 1, reads
        # The start: reads that fail are no game, and the wait ends at its limit with the "not known" line.
        reads.clear()
        harness.processes = failing_reads(10 ** 6)
        assert wait_for_start(0) is False and len(reads) == 1, len(reads)
        harness.processes = lambda name, tries=3: [(12, "Morrowind.exe")]
        assert wait_for_start(0) is True
    finally:
        harness.processes, time.sleep = saved_processes, saved_sleep


def game_files(folder, manifest, plugin):
    """The files of a game from its set, by the runner's map (F-T-8): ({path in Harness - MGE XE: file in the set},
    {path in Harness - nirender: file in the set}). The second is empty without `plugin`: the game then has the
    instance's release of the plugin, and none of the set's Lua over it."""
    mods = run_own_device.set_mods(folder, manifest, plugin)
    return mods.get("MGE XE", {}), (mods.get("nirender", {}) if plugin else {})


def open_game_set(args, current=None, verify=sets.verify):
    """The manifest of the game's set (run_own_device.open_set, with play's line and the tools of D 2.5)."""
    return run_own_device.open_set(args, current, verify, who="play", hint=NO_SET_HINT)


def gone_problems(args):
    """The runner's lines for the gone options. The variable NIRENDER_PLUGIN_DLL does not stop a game: the owner's
    local.bat may set it by hand (verdict 507 addendum 6, change 1), and `plugin_variable_line` names it."""
    return run_own_device.gone_option_problems(args, environment={})


def plugin_variable_line(environment):
    """The line for a set NIRENDER_PLUGIN_DLL, or None: the game goes on with the set's plugin."""
    if environment.get("NIRENDER_PLUGIN_DLL"):
        return "play: NIRENDER_PLUGIN_DLL is set and not read: --plugin loads the set's plugin (W521)"
    return None


def self_test_set():
    """W521: the game's set and its files. The null of the staging: the runner's own map for a run with the plugin."""
    # The plugin variable with a foreign value: a line, and no stop (the runner stops on it, Q8).
    namespace = argparse.Namespace(plugin_dll=None, mwse_dll=None, mwse_commit=None)
    foreign = {"NIRENDER_PLUGIN_DLL": "E:/nirender-keep/mgebuilds/f308/akulakhan-0f2afbd.dll"}
    assert gone_problems(namespace) == [] and run_own_device.gone_option_problems(namespace, foreign) != []
    assert plugin_variable_line(foreign).startswith("play: NIRENDER_PLUGIN_DLL is set and not read") and plugin_variable_line({}) is None
    with tempfile.TemporaryDirectory(prefix="w521-play-") as scratch:
        name = "s-6d1b2140-d9741db1-89198b53"
        folder = os.path.join(scratch, name)
        for path in ("d3d8.dll", "numidium.exe", "dinput8.dll", "MWSE.dll", "lua51.dll", "akulakhan-6d1b214.dll", sets.PLUGIN_STAGED,
                     "Data Files/mwse/mods/nirender/main.lua", "Data Files/mwse/core/initialize.lua",
                     "Data Files/mwse/mods/MGE XE Options/main.lua", "plugin-test/MWSE/mods/nirender/test.lua"):
            sets.write(os.path.join(folder, *path.split("/")), b"plugin" if "akulakhan" in path else path.encode())
        manifest = sets.scratch_manifest(folder, name)
        # The files of a game with --plugin are the runner's, path for path (F-T-8).
        mge, nirender = game_files(folder, manifest, True)
        runner = run_own_device.set_mods(folder, manifest, True)
        assert (mge, nirender) == (runner["MGE XE"], runner["nirender"])
        assert mge["Root/dinput8.dll"] == os.path.join(folder, "dinput8.dll") and "Root/lua51.dll" in mge and "MWSE/core/initialize.lua" in mge
        assert nirender[sets.PLUGIN_IN_MOD] == os.path.join(folder, *sets.PLUGIN_STAGED.split("/")) and "MWSE/mods/nirender/test.lua" in nirender
        # Without --plugin: the MGE XE half, and nothing of the set over the instance's release of the plugin.
        assert game_files(folder, manifest, False) == (runner["MGE XE"], {})
        # No set, and a set that fails its check: one line that names both tools, no game (D 2.5).
        for current, change in ((lambda: None, None), (lambda: name, b"other")):
            if change:
                sets.write(os.path.join(folder, "d3d8.dll"), change)
            try:
                open_game_set(argparse.Namespace(mge_build=folder if change else None), current)
                raise AssertionError("a game with no good set was taken")
            except SystemExit as stop:
                assert str(stop).startswith("play: ") and "build_set.py" in str(stop) and "deploy_rest.py" in str(stop), stop
        sets.write(os.path.join(folder, "d3d8.dll"), b"d3d8.dll")
        args = argparse.Namespace(mge_build=folder)
        assert open_game_set(args)["name"] == name and args.mge_build == folder


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mge-build", help="a sealed set (tools/build_set.py makes one); without this, the current set of current.bat")
    ap.add_argument("--config-mod", default="Configuration", help="the mod of the testing instance with mgeXE.toml")
    ap.add_argument("--reference", action="store_true", help="the set's MGE XE with the switch renderer_wgpu off, on Direct3D 9")
    ap.add_argument("--set", action="append", default=[], help="a line of mgeXE.toml for this game, in its section: \"[section] key = value\"; a section the file lacks is added at the end of this game's copy")
    ap.add_argument("--plugin", action="store_true", help="the set's plugin, its Lua and its test files, over the release of the instance")
    ap.add_argument("--plugin-dll", help="gone (W521): --plugin loads the set's plugin; the option stops the tool with a line")
    ap.add_argument("--livecoding", help="a Lua file to run as the livecoding.lua of the Livecoding mod (Alt+X)")
    ap.add_argument("--instance", help="the folder of another Mod Organizer instance to play in, and not the testing instance")
    ap.add_argument("--game-dir", help="with --instance: the game folder of that instance")
    ap.add_argument("--profile", default="Default", help="with --instance: its profile")
    ap.add_argument("--stage-only", action="store_true", help="stage the mods, print them, take them out again; no game")
    ap.add_argument("--self-test", action="store_true", help="the tests of the report of a close: no game")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0

    gone = gone_problems(args)
    if gone:
        raise SystemExit("play: " + "; ".join(gone))
    variable_line = plugin_variable_line(os.environ)
    if variable_line:
        print(variable_line, flush=True)
    manifest = open_game_set(args)
    print("play: " + run_own_device.set_line(args.mge_build, manifest), flush=True)
    mge_files, plugin_files = game_files(args.mge_build, manifest, args.plugin)
    print("plugin: %s" % ("the set's, %s" % plugin_files[sets.PLUGIN_IN_MOD] if args.plugin else "the instance's release"), flush=True)
    if args.instance:
        session.use_instance(args.instance, args.game_dir, args.profile)
    toml = os.path.join(harness.MO2_ROOT, "mods", args.config_mod, "Root", "mgeXE.toml")
    # The lines of this game's mgeXE.toml, as the runner makes them but with the instance's vsync:
    # a fault of --set stops here, before the busy check and before anything is staged.
    lines = session.run_toml_lines(open(toml, encoding="utf-8").read(), args.set, toml, not args.reference, vsync_immediate=False)
    harness.refuse_if_busy()
    switched = os.path.join(tempfile.mkdtemp(prefix="nirender-play-"), "mgeXE.toml")
    open(switched, "w", encoding="utf-8").write("".join(line + chr(10) for line in lines))

    # The set's files, as the runner stages them (F-T-8); with --reference the same files with the switch off.
    mods = [(harness.RUN_MOD_PREFIX + "config", {"Root/mgeXE.toml": switched}), (harness.RUN_MOD_PREFIX + "MGE XE", mge_files)]

    if plugin_files or args.livecoding:
        files = dict(plugin_files)
        if args.livecoding:
            # The main.lua of the Livecoding mod: the instance's own, or the testing
            # instance's copy for an instance without the mod (Outlander-test has none).
            main = os.path.join(harness.MO2_ROOT, "mods", "Livecoding", "MWSE", "mods", "livecoding", "main.lua")
            if not os.path.isfile(main):
                main = os.path.join(TESTING_INSTANCE, "mods", "Livecoding", "MWSE", "mods", "livecoding", "main.lua")
                print("the instance has no Livecoding mod: its main.lua comes from the testing instance")
            files["MWSE/mods/livecoding/main.lua"] = main
            files["MWSE/mods/livecoding/livecoding.lua"] = os.path.abspath(args.livecoding)
        mods.append((session.MOD_NAME, files))
    deployment = harness.Deployment()
    handle, started, game_ended = None, time.time(), False
    try:
        for name, files in mods:
            deployment.use_mod(name, files)
        if args.stage_only:
            print("staged in %s for the profile %s; no game (--stage-only)" % (harness.MO2_ROOT, os.path.basename(harness.PROFILE)))
            return 0
        subprocess.Popen(harness.launch_command(), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        print("starting the game", flush=True)
        if not wait_for_start(180):
            return 1
        print("the game runs; leave it by its menu when you are done", flush=True)
        started = time.time()
        # A fault here costs only the exit code of the report.
        with contextlib.suppress(Exception):
            handle = game_handle(harness.processes("Morrowind.exe")[0][0])
        wait_for_end()
        print("the game ended")
        game_ended = True
    finally:
        harness.close_mo2()
        restored = deployment.restore()
        if os.path.isfile(switched):
            os.remove(switched)
        os.rmdir(os.path.dirname(switched))
        if restored:
            print("the mod list of the instance is back")
    # After Mod Organizer has closed: it moves the game's log to overwrite/Root.
    if game_ended:
        report_close(handle, started)
    return 0


if __name__ == "__main__":
    sys.exit(main())
