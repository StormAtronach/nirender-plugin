"""Starts a game of the testing instance to play by hand, with the wgpu renderer of a build of
MGE XE, and puts the files of the instance back when the game ends.

    python tools/play.py [--mge-build DIR] [--reference] [--set "[section] key = value"]
                         [--plugin] [--livecoding FILE] [--stage-only]
                         [--instance DIR --game-dir DIR --profile NAME --config-mod NAME]

The testing instance at rest has the release of MGE XE and of the plugin. This tool does for a
game by hand what the runners do for a scenario: the mod "Harness - MGE XE" with the d3d8.dll
and the numidium.exe of the build, and "Harness - config" with mgeXE.toml with the switch
`renderer_wgpu`, both on for this game only, over the mods of the instance. With --reference:
MGE XE of the instance on Direct3D 9.

--instance runs the game in another Mod Organizer instance, with --game-dir its game folder,
--profile its profile and --config-mod the mod that holds its mgeXE.toml. For Outlander-test
play-manual-check-outlander.bat has them (the instance's folder comes from local.bat).
--stage-only puts the mods in place, prints them, and takes them out again without a game: a
check of the staging in an instance.

--plugin puts a plugin DLL and the Lua of mod/ of this repo into "Harness - nirender" for the
game, over the release of the instance. The DLL comes as in run_own_device.py: --plugin-dll
(which implies --plugin), else the variable NIRENDER_PLUGIN_DLL, else the plugin copy
akulakhan-<commit>.dll of the --mge-build folder, else the build of this repo
(target/.../release/akulakhan.dll). With --mge-build, a WARNING line names a plugin that is not
the folder's copy: the build of this repo, or a DLL with other bytes than the folder's one copy.
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
import run_own_device  # noqa: E402  (the plugin rule of a run, W520)

MGE_BUILD = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "..", "MGE-XE-G7-render-link", "bin", "MSVC-Release")
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
    self_test_plugin()
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


def play_plugin(args, environment, default, listdir=os.listdir):
    """The plugin DLL of a game, its source and the WARNING line or None, by the rule of run_own_device.py (W520,
    D489). Only an explicit --mge-build names a run folder: the default folder is the checkout's MGE XE build."""
    dll, source = run_own_device.plugin_dll_of(args.plugin_dll, environment, default, args.mge_build, listdir)
    return dll, source, run_own_device.plugin_warning(args.mge_build, dll, source, listdir=listdir)


def self_test_plugin():
    """W520 (D489): the plugin of a game with --mge-build. The null: before W520 play.py took the build of this repo
    for every game, so the first assert below failed."""
    with tempfile.TemporaryDirectory(prefix="w520-play-") as scratch:
        folder, empty = os.path.join(scratch, "f1"), os.path.join(scratch, "f2")
        os.mkdir(folder)
        os.mkdir(empty)
        copy, other = os.path.join(folder, "akulakhan-45b645b.dll"), os.path.join(scratch, "other.dll")
        for path, data in ((copy, b"copy"), (other, b"other")):
            with open(path, "wb") as file:
                file.write(data)
        checkout = os.path.join(scratch, "akulakhan.dll")

        def game(mge_build, plugin_dll=None):
            return argparse.Namespace(mge_build=mge_build, plugin_dll=plugin_dll)

        # The folder's copy, with no line.
        assert play_plugin(game(folder), {}, checkout) == (copy, run_own_device.FOLDER_PLUGIN_SOURCE, None)
        # The variable and --plugin-dll stay above the folder's copy, with the WARNING of verdict 444.
        dll, source, warning = play_plugin(game(folder), {"NIRENDER_PLUGIN_DLL": other}, checkout)
        assert (dll, source) == (other, "NIRENDER_PLUGIN_DLL") and warning and copy in warning, warning
        dll, source, warning = play_plugin(game(folder, other), {}, checkout)
        assert (dll, source) == (other, "--plugin-dll") and (warning or "").startswith("WARNING: "), warning
        # No copy: the build of this repo, with the WARNING; no --mge-build: no run folder, no line.
        dll, source, warning = play_plugin(game(empty), {}, checkout)
        assert (dll, source) == (checkout, run_own_device.CHECKOUT_PLUGIN_SOURCE) and "not the run folder's" in warning, warning
        assert play_plugin(game(None), {}, checkout) == (checkout, run_own_device.CHECKOUT_PLUGIN_SOURCE, None)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mge-build", help="the folder with d3d8.dll and numidium.exe of the build, and with --plugin its plugin copy "
                    "akulakhan-<commit>.dll (default: %s)" % MGE_BUILD)
    ap.add_argument("--config-mod", default="Configuration", help="the mod of the testing instance with mgeXE.toml")
    ap.add_argument("--reference", action="store_true", help="MGE XE of the instance on Direct3D 9")
    ap.add_argument("--set", action="append", default=[], help="a line of mgeXE.toml for this game, in its section: \"[section] key = value\"; a section the file lacks is added at the end of this game's copy")
    ap.add_argument("--plugin", action="store_true", help="a plugin DLL and the Lua of this repo, over the release of the instance")
    ap.add_argument("--plugin-dll", help="the plugin DLL of the game (implies --plugin; else the variable NIRENDER_PLUGIN_DLL, "
                    "else the plugin copy of the --mge-build folder, else the build of this repo)")
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

    plugin_dll = None
    if args.plugin or args.plugin_dll:
        plugin_dll, source, warning = play_plugin(args, os.environ, session.DLL)
        print("plugin DLL: %s (%s)" % (plugin_dll, source), flush=True)
        if warning:
            print(warning, flush=True)
    mge_build = args.mge_build or MGE_BUILD
    if args.instance:
        session.use_instance(args.instance, args.game_dir, args.profile)
    toml = os.path.join(harness.MO2_ROOT, "mods", args.config_mod, "Root", "mgeXE.toml")
    # The lines of this game's mgeXE.toml, as the runner makes them but with the instance's vsync:
    # a fault of --set stops here, before the busy check and before anything is staged.
    lines = session.run_toml_lines(open(toml, encoding="utf-8").read(), args.set, toml, not args.reference, vsync_immediate=False)
    harness.refuse_if_busy()
    switched = os.path.join(tempfile.mkdtemp(prefix="nirender-play-"), "mgeXE.toml")
    open(switched, "w", encoding="utf-8").write("".join(line + chr(10) for line in lines))

    mods = [(harness.RUN_MOD_PREFIX + "config", {"Root/mgeXE.toml": switched})]
    if not args.reference:
        mods.append((harness.RUN_MOD_PREFIX + "MGE XE", {"Root/d3d8.dll": os.path.join(mge_build, "d3d8.dll"),
                                                         "Root/numidium.exe": os.path.join(mge_build, "numidium.exe")}))
        # The Lua of the build (the MGE XE Options mod: its menus and the event
        # distantLandReady) over the release's copy in the instance, as the runner stages it.
        options = os.path.join(mge_build, "Data Files", "mwse", "mods", "MGE XE Options")
        for folder, _, names in os.walk(options):
            for name in names:
                source = os.path.join(folder, name)
                mods[-1][1]["MWSE/mods/MGE XE Options/" + os.path.relpath(source, options).replace(os.sep, "/")] = source

    if plugin_dll or args.livecoding:
        files = {}
        if plugin_dll:
            mod = os.path.join(session.REPO, "mod")
            for folder, _, names in os.walk(mod):
                for name in names:
                    source = os.path.join(folder, name)
                    files[os.path.relpath(source, mod).replace(os.sep, "/")] = source
            files["MWSE/lib/akulakhan.dll"] = plugin_dll
            # The symbol file of the build: the Harness mod is the staging of a game, never
            # an install or an archive.
            pdb = os.path.splitext(plugin_dll)[0] + ".pdb"
            if os.path.isfile(pdb):
                files["MWSE/lib/akulakhan.pdb"] = pdb
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
