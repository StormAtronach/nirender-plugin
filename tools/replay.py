"""Draws a frame of a capture file with the MGE XE helper, offline: the same picture from the
same file each time. For a check of a change of the renderer against the picture before it.

    python tools/replay.py <capture file> <picture.png> [--frame N] [--mge-build SET | --archive DIR]
    python tools/replay.py <capture file> <prefix> --frames-at 1,3,5 [--mge-build SET | --archive DIR]
    python tools/replay.py --run-folder-for <picture.png> [--config-root DIR]
    python -I tools/replay.py --self-test

THE BUILD (W545, the runner's rule of W521). A replay takes its helper from a sealed set of
build_set.py: --mge-build names the set, and without it the replay takes the current set (the one
that current.bat names). The set must pass sets.verify; a folder that is not a set, such as a run
build folder f<N> or bin/MSVC-Release, is refused, and nothing starts. The tool writes the set's
line (its name, the three commits, the manifest md5) to standard error before the helper starts.
--archive names the folder of an unpacked release archive, which has no manifest: the release
check of release.py. A local.json with the old key "mge_build" is refused: remove the key.

With --frames-at, one run of the helper writes <prefix>-<frame as 5 digits>.png for each
frame named (the frames count from 0, as --frame): one start of the helper for a capture of
many stops, in place of one for each. With --marks-every N, the helper prints a mark (its
time for each frame by part) after every N frames: the bench of a long capture.
        [--config-root DIR] [--distant-land DIR] [--backend vulkan|dx12]
        [--debug-view NAME [TERM]] [--debug-lines NAME,NAME] [--core-mods FOLDER|off]
        [--keep-run-folder]

--core-mods FOLDER reads the core mods (`XE Mod <name>.wgsl`) only from that folder
(MGE_XE_CORE_MODS_DIR), never from the game folder; --core-mods off reads none
(MGE_XE_CORE_MODS=0). Without it, the helper searches as a player's game does. The hook
fixtures of the gate are in tools/hook-fixtures (W528 item 5).

--debug-view and --debug-lines set the debug view of the replay (the helper's names: lights,
shadow-layers, shadow-map, water TERM, post TERM, fog; the line sources lights, shadow-boxes,
bounds, lua). A replay takes its view only from these arguments: a view or a line source that
a record of the capture sets is not drawn. The helper draws no lines yet: --debug-lines sets
the sources, and the lines come with piece 3 of the debug views.

THE RUN FOLDER. The helper never runs in a folder of an instance or of the game (the owner's
D530). For each call the tool makes a run folder next to the picture, <picture without ".png">.run
(with --frames-at: <prefix>.run), copies mgeXE.toml from --config-root into it (the Root folder
of the Configuration mod of the testing instance as the default; that folder is only read),
and starts the helper there with --log <run folder>/helper.txt. So the helper's log, a crash
dump (numidium.dmp, beside the log) and a file of a tool that instruments the helper land in the
run folder. The helper's lines are in the log, not in its standard output: the tool prints the
log's text when the helper has ended, so a caller reads what it read before.
After a run with exit code 0 and no other file in the run folder, the tool removes its two files
and the folder. Else it keeps the folder and names its files on standard error;
--keep-run-folder keeps it always. A picture path inside a Mod Organizer instance or inside the
game folder is refused, since its run folder would be there too. An empty picture path is refused.
The helper looks for a file under "Data Files" in its current folder first, then in --game-dir,
then beside itself: a --config-root with a "Data Files" folder is refused, because the run folder
would not have it.
--run-folder-for makes the run folder for a tool that starts the helper itself (a profiler) and
prints the folder and the log path; no helper starts, and the folder stays.

The distant land comes from --distant-land (the overwrite folder of the testing instance as the
default). The archives of the game come from --game-dir; a texture that is only in a mod folder
is white: the same in each picture.
"""
import argparse
import contextlib
import io
import json
import locale
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from common import BUILD_PARTS, build_missing, capture_format_line, local, local_file, replay_fault_line  # noqa: E402
import sets  # noqa: E402

CONFIG_FILE = "mgeXE.toml"
LOG_NAME = "helper.txt"
RUN_SUFFIX = ".run"
# The tool's own files in a run folder.
OWN_FILES = (CONFIG_FILE, LOG_NAME)
# A folder with this file is a Mod Organizer instance; a folder with this program is a game folder.
INSTANCE_MARK = "ModOrganizer.ini"
GAME_MARK = "Morrowind.exe"
EXIT_REFUSED = 2
# The exit code of a build that is not a set, or of a local.json with the gone key: the runner's and play.py's code.
EXIT_NOT_A_SET = 1
# The key of local.json that named the build folder before W545.
# The keys of local.json that replay.py no longer reads, with what takes their place (W545, W521 step 4).
GONE_LOCAL_KEYS = {
    "mge_build": "a replay takes the current set or --mge-build <set>",
    "distant_land": "a replay takes the distant land mod of its key, or --distant-land <folder>",
}


class Refused(Exception):
    """A call that the tool does not run, with the reason as its text."""


def run_folder_of(picture):
    """The run folder of a picture argument: the path without a ".png" at its end, with ".run". A
    prefix of --frames-at has no ".png" and takes ".run" as it is (a dot inside a name stays)."""
    path = os.path.abspath(picture)
    if path.lower().endswith(".png"):
        path = path[:-4]
    return path + RUN_SUFFIX


def owner_of(folder):
    """What holds `folder`, if the helper must not run there: "the Mod Organizer instance <root>" or
    "the game folder <root>" for the nearest parent (or the folder itself) with the mark of one; else
    None. Found by the marks, so every instance and every game folder counts, not only the testing one."""
    place = os.path.abspath(folder)
    while True:
        if os.path.isfile(os.path.join(place, INSTANCE_MARK)):
            return "the Mod Organizer instance %s" % place
        if os.path.isfile(os.path.join(place, GAME_MARK)):
            return "the game folder %s" % place
        parent = os.path.dirname(place)
        if parent == place:
            return None
        place = parent


def prepare_run_folder(picture, config_root):
    """Makes the run folder of `picture` with the configuration of `config_root`: (the folder, the path
    of the helper's log). Raises [`Refused`] for a run folder inside an instance or the game folder, for
    a `config_root` without mgeXE.toml, and for one with a "Data Files" folder. Old files of the tool in
    a folder that exists are removed; other files there stay, and the end of the run names them."""
    # An empty path would give the run folder "<the current folder>.run", wherever the caller stands.
    if not str(picture).strip():
        raise Refused("the picture path is empty")
    folder = run_folder_of(picture)
    owner = owner_of(folder)
    if owner:
        raise Refused("the picture %s is inside %s: its run folder would be there too, and the helper does not run in an "
                      "instance or in the game folder" % (os.path.abspath(picture), owner))
    source = os.path.join(config_root, CONFIG_FILE)
    if not os.path.isfile(source):
        raise Refused("no %s in %s (--config-root)" % (CONFIG_FILE, config_root))
    if os.path.isdir(os.path.join(config_root, "Data Files")):
        raise Refused("%s (--config-root) has a \"Data Files\" folder: the helper reads such files from its current folder "
                      "first, and the run folder would not have them" % config_root)
    os.makedirs(folder, exist_ok=True)
    for name in OWN_FILES:
        old = os.path.join(folder, name)
        if os.path.isfile(old):
            os.remove(old)
    shutil.copyfile(source, os.path.join(folder, CONFIG_FILE))
    return folder, os.path.join(folder, LOG_NAME)


def finish_run_folder(folder, code, keep):
    """The end of a run. After exit code 0, with only the tool's own files in the folder and without
    `keep`: removes them and the folder, and returns None. Else the folder stays, and the line that
    names it and its files is returned."""
    names = sorted(os.listdir(folder))
    others = [name for name in names if name not in OWN_FILES]
    if code == 0 and not others and not keep:
        for name in names:
            os.remove(os.path.join(folder, name))
        os.rmdir(folder)
        return None
    return "replay: the run folder %s is kept: %s" % (folder, ", ".join(names) or "no file")


def log_text(path):
    """The text of the helper's log, read as a caller read the helper's output before: the encoding of
    a pipe in text mode, a bad byte replaced, the line ends as "\\n". Empty for no log."""
    if not os.path.isfile(path):
        return ""
    with open(path, encoding=locale.getpreferredencoding(False), errors="replace") as file:
        return file.read()


def start_helper(command, folder, env):
    """Runs the helper in `folder` to its end: (exit code, standard output, standard error)."""
    result = subprocess.run(command, cwd=folder, env=env, capture_output=True, text=True)
    return result.returncode, result.stdout, result.stderr


def run_replay(command, picture, config_root, env, keep, start=start_helper):
    """One replay in its run folder: (exit code, the text for standard output, the lines for standard
    error). `command` is the helper's command without --log; `start` is [`start_helper`] but in the
    self-test."""
    folder, log = prepare_run_folder(picture, config_root)
    code, out, err = start(command + ["--log", log], folder, env)
    # The helper's lines are in its log; what it still wrote to its standard output comes after them.
    out = log_text(log) + out
    kept = finish_run_folder(folder, code, keep)
    return code, out, err + (kept + "\n" if kept else "")


def defaults():
    """The default folders of a replay, from the harness of the docs (not needed by the self-test)."""
    import session
    root = session.harness.MO2_ROOT
    return {
        "config_root": local("config_root", os.path.join(root, "mods", "Configuration", "Root")),
        "distant_land": distant_land_of,
        "game_dir": local("game_dir", session.harness.GAME_DIR),
        # A replay is build work. The slot hook reads only a command line, so a replay that a script or
        # another tool starts is checked here. A harness from before the check has no such function.
        "check_build_slot": getattr(session.harness, "check_build_slot", None),
    }


def gone_local_key(path):
    """The refusal of a local.json at `path` that still has a key that replay.py no longer reads (GONE_LOCAL_KEYS), or
    None. Without the refusal, a worktree with the key would replay another build or another distant land than the
    one its local.json names."""
    if not path:
        return None
    with open(path, encoding="utf-8") as file:
        values = json.load(file)
    gone = [key for key in GONE_LOCAL_KEYS if key in values]
    if not gone:
        return None
    return ('%s has the key "%s", which replay.py no longer reads: %s; remove the key from that file; nothing started'
            % (path, gone[0], GONE_LOCAL_KEYS[gone[0]]))


def distant_land_of(mge_build, manifest, config_root):
    """(the distant land folder of a replay, its line): the mod of the key of the set (or the archive) with the
    mgeXE.toml of `config_root`, in the instance of the harness (W521 step 4). A mod that is not there, or has no
    version file, is a distant_land.Problem: no other folder is taken (the Architect's ruling Q4)."""
    import distant_land
    import session
    toml = os.path.join(config_root, CONFIG_FILE)
    if manifest:
        key = distant_land.key_of(mge_build, manifest, toml, [])
    else:
        key = distant_land.archive_key(mge_build, toml, [])
    mod = os.path.join(distant_land.mods_folder(session.harness.MO2_ROOT), key.mod_name)
    if not distant_land.has_data(mod):
        raise distant_land.Problem("the distant land of this replay, %s, is not there or has no version file: a plain run of "
                                   "that build makes it; no other folder is taken; nothing started" % mod)
    return os.path.join(mod, distant_land.DATA), "distant land: the mod %s (key %s)" % (key.mod_name, key.text)


def open_build(args, verify=None, current=None):
    """The build of a replay, its line for standard error and its manifest: (folder, line, manifest or None for an
    archive). --archive takes the unpacked release
    archive as it is. Else the set of --mge-build, or the current set, after sets.verify, as the runner and play.py
    take theirs (`run_own_device.open_set`, `set_line`). Raises Refused for --archive with --mge-build, and SystemExit
    with the runner's line for a folder that is not a set or a set that fails its check. `verify` and `current` are
    the stand-ins of the self-test."""
    if args.archive:
        if args.mge_build:
            raise Refused("an archive is not a set: no --mge-build with --archive")
        return args.archive, "build: the release archive %s" % args.archive, None
    # The runner's module is large; a call of --run-folder-for or --self-test does not need it.
    import run_own_device
    manifest = run_own_device.open_set(args, current, verify or sets.verify, who="replay")
    return args.mge_build, run_own_device.set_line(args.mge_build, manifest), manifest


def self_test():
    with tempfile.TemporaryDirectory() as temp:
        temp = os.path.realpath(temp)
        join = os.path.join

        def write(path, text=""):
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w", encoding="ascii", newline="\n") as file:
                file.write(text)

        # The run folder's name: a picture loses its ".png"; a prefix of --frames-at keeps a dot of its name.
        assert run_folder_of(join(temp, "out", "seyda.png")) == join(temp, "out", "seyda.run")
        assert run_folder_of(join(temp, "out", "SEYDA.PNG")) == join(temp, "out", "SEYDA.run")
        assert run_folder_of(join(temp, "out", "tour-1")) == join(temp, "out", "tour-1.run")
        assert run_folder_of(join(temp, "out", "v0.2-seyda")) == join(temp, "out", "v0.2-seyda.run")

        # How the tool knows an instance and the game folder: by their marks, at any parent.
        instance, game, config = join(temp, "Instance"), join(temp, "Game"), join(temp, "config")
        write(join(instance, INSTANCE_MARK))
        write(join(game, GAME_MARK))
        write(join(config, CONFIG_FILE), "[runtime]\nrenderer_wgpu = true\n")
        write(join(config, "csse.toml"), "other = 1\n")
        for inside in (join(instance, "mods", "Configuration", "Root"), join(instance, "overwrite", "distantland"),
                       join(instance, "profiles", "Default"), instance):
            assert owner_of(inside) == "the Mod Organizer instance %s" % instance, inside
        assert owner_of(join(game, "Data Files")) == "the game folder %s" % game
        assert owner_of(join(temp, "out")) is None and owner_of(config) is None

        # The refusals: a picture inside an instance or the game folder; no mgeXE.toml; a "Data Files" folder.
        for picture in (join(instance, "mods", "Configuration", "Root", "a.png"), join(instance, "overwrite", "a.png"),
                        join(game, "a.png")):
            try:
                prepare_run_folder(picture, config)
                raise AssertionError("not refused: %s" % picture)
            except Refused as refused:
                assert "the helper does not run in an instance or in the game folder" in str(refused), refused
            assert not os.path.exists(run_folder_of(picture)), "a refused call made its folder"
        try:
            prepare_run_folder(join(temp, "out", "a.png"), join(temp, "empty"))
            raise AssertionError("no mgeXE.toml and not refused")
        except Refused as refused:
            assert "no mgeXE.toml" in str(refused), refused
        data = join(temp, "config-with-data")
        write(join(data, CONFIG_FILE))
        os.makedirs(join(data, "Data Files"))
        try:
            prepare_run_folder(join(temp, "out", "a.png"), data)
            raise AssertionError("a Data Files folder and not refused")
        except Refused as refused:
            assert "Data Files" in str(refused), refused
        assert not os.path.exists(join(temp, "out")), "a refused call made a folder"

        # The source folder may be the instance's: it is only read. The run folder holds mgeXE.toml and nothing
        # else when the helper starts, and the copy is the source's bytes.
        source = join(instance, "mods", "Configuration", "Root")
        write(join(source, CONFIG_FILE), "[distant_land]\nenabled = true\n")
        before = sorted(os.listdir(source))
        seen = {}

        def helper(code, extra=None, printed="", fault=""):
            def start(command, folder, env):
                seen["command"], seen["folder"], seen["at start"] = command, folder, sorted(os.listdir(folder))
                seen["copy"] = open(join(folder, CONFIG_FILE), encoding="ascii").read()
                log = command[command.index("--log") + 1]
                write(log, "nirender-host v0 test\nmark replay frame 100: 100 frames\n")
                if extra:
                    write(join(folder, extra), "dump")
                return code, printed, fault
            return start

        picture = join(temp, "out", "seyda.png")
        folder = run_folder_of(picture)
        code, out, err = run_replay(["numidium.exe", "--replay", "c.nircap", picture], picture, source, {}, False, helper(0))
        assert (code, err) == (0, ""), (code, err)
        assert out == "nirender-host v0 test\nmark replay frame 100: 100 frames\n", out
        assert seen["folder"] == folder and seen["at start"] == [CONFIG_FILE], seen
        assert seen["copy"] == "[distant_land]\nenabled = true\n"
        assert seen["command"][-2:] == ["--log", join(folder, LOG_NAME)] and seen["command"][:2] == ["numidium.exe", "--replay"]
        assert not os.path.exists(folder), "the run folder of a clean run stays"
        assert sorted(os.listdir(source)) == before, "the source folder changed"

        # What the helper still writes to its standard output comes after the log's text.
        code, out, err = run_replay(["numidium.exe"], picture, source, {}, False, helper(0, printed="late line\n"))
        assert out.endswith("frames\nlate line\n") and err == "", (out, err)

        # A failed helper: the folder stays and is named, with the helper's own standard error first.
        code, out, err = run_replay(["numidium.exe"], picture, source, {}, False, helper(41, fault="the helper's fault\n"))
        assert code == 41 and err == "the helper's fault\nreplay: the run folder %s is kept: %s, %s\n" % (folder, LOG_NAME, CONFIG_FILE), err
        # The same call again uses the folder: the tool's old files are replaced, and a clean run removes it.
        code, out, err = run_replay(["numidium.exe"], picture, source, {}, False, helper(0))
        assert seen["at start"] == [CONFIG_FILE] and (code, err) == (0, "") and not os.path.exists(folder)

        # Another file in the folder (a dump, a file of a tool that instruments the helper): kept and named.
        code, out, err = run_replay(["numidium.exe"], picture, source, {}, False, helper(0, extra="numidium.dmp"))
        assert code == 0 and err == "replay: the run folder %s is kept: %s, %s, numidium.dmp\n" % (folder, LOG_NAME, CONFIG_FILE), err
        # It is still there at the next call, and so is named again; the tool removes no file that is not its own.
        code, out, err = run_replay(["numidium.exe"], picture, source, {}, False, helper(0))
        assert seen["at start"] == [CONFIG_FILE, "numidium.dmp"] and "numidium.dmp" in err and os.path.isfile(join(folder, "numidium.dmp"))

        # --keep-run-folder keeps a clean run's folder.
        other = join(temp, "out", "tour-1")
        code, out, err = run_replay(["numidium.exe"], other, source, {}, True, helper(0))
        assert err == "replay: the run folder %s is kept: %s, %s\n" % (run_folder_of(other), LOG_NAME, CONFIG_FILE), err

        # --run-folder-for: the folder and the log path, no helper; the folder stays with its mgeXE.toml.
        printed = io.StringIO()
        with contextlib.redirect_stdout(printed):
            code = main(["--run-folder-for", join(temp, "profile", "walk.png"), "--config-root", source])
        made = join(temp, "profile", "walk.run")
        assert code == 0 and printed.getvalue().splitlines()[:2] == [made, join(made, LOG_NAME)], printed.getvalue()
        assert os.listdir(made) == [CONFIG_FILE]
        errors = io.StringIO()
        with contextlib.redirect_stderr(errors), contextlib.redirect_stdout(io.StringIO()):
            code = main(["--run-folder-for", join(game, "walk.png"), "--config-root", source])
        assert code == EXIT_REFUSED and errors.getvalue().startswith("replay: refused: the picture "), errors.getvalue()

        # The variables of --core-mods (W528 item 5): none without the option, "off" reads no core mod, a folder
        # is given to the helper as an absolute path.
        assert core_mods_env(None) == {} and core_mods_env("off") == {"MGE_XE_CORE_MODS": "0"}
        assert core_mods_env(join(temp, "fixtures")) == {"MGE_XE_CORE_MODS_DIR": join(temp, "fixtures")}
        relative = core_mods_env("fixtures")["MGE_XE_CORE_MODS_DIR"]
        assert os.path.isabs(relative) and os.path.basename(relative) == "fixtures", relative

        # An empty picture path is refused before anything is made, in the mode and for a replay.
        for empty in ("", "   "):
            try:
                prepare_run_folder(empty, source)
                raise AssertionError("an empty picture path and not refused: %r" % empty)
            except Refused as refused:
                assert str(refused) == "the picture path is empty", refused
        assert not os.path.exists(os.path.abspath("") + RUN_SUFFIX), "an empty picture path made a folder"
        # --run-folder-for with an empty value takes the mode and is refused: it does not fall through to a replay of
        # the arguments after it. The helper's stand-in must not be called, and the exit code is the refusal's.
        # The build folder has every part that a replay asks for, so that a fall-through would reach the helper's
        # start and show: without the test for "is not None" in main, `started` has the command of a replay.
        started = []
        build = join(temp, "build")
        write(join(build, "numidium.exe"))
        for part in BUILD_PARTS:
            if not os.path.exists(join(build, part)):
                os.makedirs(join(build, part))
        assert not build_missing(build)

        def never(command, folder, env):
            started.append(command)
            return 0, "", ""

        # The build of a replay (W545) in these tests: `build` is a stand-in set. It has a MANIFEST.json, so that the
        # set line has an md5, and `good_set` stands for sets.verify. The local.json and the defaults are stand-ins too,
        # so that no test reads this machine's files or slots.
        manifest = {"name": "build", "commits": {repo: {"commit": repo[0] * 40} for repo in sets.REPOS}}
        write(join(build, sets.MANIFEST), json.dumps(manifest))

        def good_set(folder):
            return manifest

        clean_local = join(temp, "local-clean.json")
        write(clean_local, '{"video_folder": "C:/videos"}')
        slot_checks = []
        planted_land = join(temp, "distantland")
        settings = {"config_root": source, "distant_land": lambda *_: (planted_land, "distant land: planted"), "game_dir": join(temp, "game-dir"),
                    "check_build_slot": lambda: slot_checks.append("checked")}

        for flag in (["--run-folder-for", ""], ["--run-folder-for="]):
            errors = io.StringIO()
            with contextlib.redirect_stderr(errors), contextlib.redirect_stdout(io.StringIO()):
                code = main(flag + [join(temp, "c.nircap"), join(temp, "out", "fall.png"), "--config-root", source,
                                    "--mge-build", build], start=never, verify=good_set, local_path=clean_local, settings=settings)
            assert code == EXIT_REFUSED and errors.getvalue() == "replay: refused: the picture path is empty\n", (flag, code, errors.getvalue())
            assert not started and not os.path.exists(run_folder_of(join(temp, "out", "fall.png"))), (flag, started)
        build_tests(temp, write, build, manifest, good_set, clean_local, settings, slot_checks)
    print("replay self-test: ok")
    return 0


def build_tests(temp, write, build, manifest, good_set, clean_local, settings, slot_checks):
    """The tests of the build of a replay (W545), each with its null: a set, a folder that is not a set, a set that fails
    its check, the current set, --archive, the order against the slot check, a set without a part, and local.json's
    gone key. No helper starts: a stand-in records the command."""
    import clean_common
    join = os.path.join
    started = []
    picture = join(temp, "out", "build.png")

    def record(command, folder, env):
        started.append(command)
        write(command[command.index("--log") + 1], "nirender-host v0 test\n")
        return 0, "", ""

    def call(arguments, verify=good_set, current=None, local_path=clean_local, start=None):
        started.clear()
        slot_checks.clear()
        errors = io.StringIO()
        with contextlib.redirect_stderr(errors), contextlib.redirect_stdout(io.StringIO()):
            code = main([join(temp, "c.nircap"), picture] + arguments, start=start or record, verify=verify, current=current,
                        local_path=local_path, settings=settings)
        return code, errors.getvalue()

    def parts(folder):
        write(join(folder, "numidium.exe"))
        for part in BUILD_PARTS:
            if not os.path.exists(join(folder, part)):
                os.makedirs(join(folder, part))

    def nothing_started(what, code, err, wanted):
        assert code == wanted, (what, code, err)
        assert not started and not slot_checks, (what, started, slot_checks)
        assert not os.path.exists(run_folder_of(picture)), what

    # T1, a set: its helper starts, after the slot check, and the set line comes first on standard error.
    code, err = call(["--mge-build", build])
    line = "replay: set: build (nirender %s, render-link %s, mwse %s), manifest md5 %s" % (
        "n" * 40, "r" * 40, "m" * 40, sets.md5_of(join(build, sets.MANIFEST)))
    assert code == 0 and err.splitlines()[0] == line, (code, err)
    assert started[0][0] == join(build, "numidium.exe") and slot_checks == ["checked"], (started, slot_checks)
    # Its null: the same folder as --archive has the archive's line and no set line.
    code, err = call(["--archive", build])
    assert code == 0 and err.splitlines()[0] == "replay: build: the release archive %s" % build and "set:" not in err, err
    assert started[0][0] == join(build, "numidium.exe")

    # T2, a folder that is not a set, with the real sets.verify: refused before the slot check and the run folder.
    plain = join(temp, "plain")
    parts(plain)
    code, err = call(["--mge-build", plain], verify=None)
    nothing_started("a folder that is not a set", code, err, EXIT_NOT_A_SET)
    assert err.startswith("replay: ") and "is not a sealed set" in err and "nothing started" in err, err
    # Its null: the same folder with a manifest and a check that passes replays, so the refusal came from the manifest.
    write(join(plain, sets.MANIFEST), json.dumps(manifest))
    code, err = call(["--mge-build", plain])
    assert code == 0 and started[0][0] == join(plain, "numidium.exe"), (code, err)

    # T3, a set that fails its check, with the real sets.verify: a file of its manifest is not there.
    bad = join(temp, "bad-set")
    parts(bad)
    write(join(bad, sets.MANIFEST), json.dumps({"form": sets.FORM, "name": "bad-set", "folders": [],
                                                 "files": {"lost.dll": {"size": 1, "md5": "0" * 32}}}))
    code, err = call(["--mge-build", bad], verify=None)
    nothing_started("a set that fails its check", code, err, EXIT_NOT_A_SET)
    assert "the file lost.dll is missing" in err and "nothing started" in err, err

    # T4, the default: without --mge-build, the current set of the sets folder.
    sets_root = join(temp, "sets")
    parts(join(sets_root, "s-current"))
    write(join(sets_root, "s-current", sets.MANIFEST), json.dumps(manifest))
    before = clean_common.SETS
    clean_common.SETS = sets_root
    try:
        code, err = call([], current=lambda: "s-current")
        assert code == 0 and started[0][0] == join(sets_root, "s-current", "numidium.exe"), (code, err, started)
        code, err = call([], current=lambda: None)
        nothing_started("no current set", code, err, EXIT_NOT_A_SET)
        assert "no --mge-build and no current set" in err, err
    finally:
        clean_common.SETS = before

    # Its null: with --mge-build, the current set is not asked for.
    def no_current():
        raise AssertionError("the current set was asked for with --mge-build")

    code, err = call(["--mge-build", build], current=no_current)
    assert code == 0, (code, err)

    # T5, --archive with --mge-build: refused. Its null is T1's null.
    code, err = call(["--archive", build, "--mge-build", build])
    nothing_started("--archive with --mge-build", code, err, EXIT_REFUSED)
    assert err == "replay: refused: an archive is not a set: no --mge-build with --archive\n", err

    # T6, the order: T2, T3 and T5 show that a refused build reaches no slot check; T1 shows that a set does.

    # T7, a set without a part of the build: the old refusal, after the set line. Its null is T1.
    partial = join(temp, "set-without-textures")
    write(join(partial, "numidium.exe"))
    os.makedirs(join(partial, "Data Files", "shaders"))
    write(join(partial, sets.MANIFEST), json.dumps(manifest))
    code, err = call(["--mge-build", partial])
    assert code == 2 and not started and "has no %s" % join("Data Files", "textures", "MGE") in err, (code, err)
    assert err.startswith("replay: set: build "), err

    # W521 step 4. T-N4a: a replay takes the mod of its key and names it; --distant-land wins over it (the override).
    lands = []
    planted_land = settings["distant_land"]()[0]

    def land_env(command, folder, env):
        # By its end, not its quoted name: tools/check_readme.py counts a quoted name as a switch that this file reads.
        lands.append(next(value for name, value in env.items() if name.endswith("_DISTANT_LAND_DIR")))
        return 0, "", ""

    code, err = call(["--mge-build", build], start=land_env)
    assert code == 0 and lands[-1] == planted_land and "replay: distant land: planted\n" in err, (code, err, lands)
    code, err = call(["--mge-build", build, "--distant-land", join(temp, "other")], start=land_env)
    assert code == 0 and lands[-1] == join(temp, "other") and "distant land: --distant-land %s" % join(temp, "other") in err, (code, err)
    # T-N4b: local.json with the gone key distant_land is refused by name. Its null is T1.
    old_land = join(temp, "local-land.json")
    write(old_land, '{"distant_land": "D:/x/overwrite/distantland"}')
    code, err = call(["--mge-build", build], local_path=old_land)
    nothing_started("local.json with the gone key distant_land", code, err, EXIT_NOT_A_SET)
    assert 'has the key "distant_land", which replay.py no longer reads: a replay takes the distant land mod of its key' in err, err
    # T-N4c: the key's mod with no version file is refused, and no other folder is taken. The null: with a version file
    # the mod's distantland folder is the answer.
    import distant_land
    import session
    instance = join(temp, "Instance")
    own = {"distant_land_format": "own-5296d863", "commits": {"render-link": {"commit": "5296d863" * 5, "clean": True}}}
    saved_root = session.harness.MO2_ROOT
    session.harness.MO2_ROOT = instance
    try:
        mod = join(instance, "mods", "Distant land own-5296d863-base")
        os.makedirs(join(instance, "overwrite", "distantland"))
        write(join(instance, "overwrite", "distantland", "version"), "22")
        try:
            distant_land_of("S", own, settings["config_root"])
            raise AssertionError("a replay took a folder with no mod")
        except distant_land.Problem as problem:
            assert "Distant land own-5296d863-base" in str(problem) and "no other folder is taken" in str(problem), problem
        write(join(mod, "distantland", "version"), "22")
        folder, line = distant_land_of("S", own, settings["config_root"])
        assert folder == join(mod, "distantland") and "Distant land own-5296d863-base" in line, (folder, line)
    finally:
        session.harness.MO2_ROOT = saved_root

    # T8, local.json with the gone key: refused, the key and the file named. Its null is T1, with a local.json without it.
    old_local = join(temp, "local-old.json")
    write(old_local, '{"mge_build": "C:/old/bin/MSVC-Release"}')
    code, err = call(["--mge-build", build], local_path=old_local)
    nothing_started("local.json with the gone key", code, err, EXIT_NOT_A_SET)
    assert err == ('replay: %s has the key "mge_build", which replay.py no longer reads: a replay takes the current set '
                   "or --mge-build <set>; remove the key from that file; nothing started\n" % old_local), err


def core_mods_env(value):
    """The variables of --core-mods: off reads no core mod, a folder reads them only from it, None changes nothing."""
    if value is None:
        return {}
    if value == "off":
        return {"MGE_XE_CORE_MODS": "0"}
    return {"MGE_XE_CORE_MODS_DIR": os.path.abspath(value)}


def main(argv=None, start=start_helper, verify=None, current=None, local_path=None, settings=None):
    """The tool. The keywords are the stand-ins of the self-test: `start` of the helper, `verify` and `current` of the
    set (`open_build`), `local_path` of the local.json that is read (None: `local_file()`), `settings` of `defaults()`."""
    ap = argparse.ArgumentParser()
    ap.add_argument("capture", nargs="?")
    ap.add_argument("picture", nargs="?")
    ap.add_argument("--self-test", action="store_true", help="the tool's own checks: no helper starts, nothing but a temp folder is written")
    ap.add_argument("--run-folder-for", metavar="PICTURE", help="makes the run folder of a picture and prints it and the log path; no helper starts")
    ap.add_argument("--keep-run-folder", action="store_true", help="keeps the run folder after a clean run too")
    ap.add_argument("--frame", type=int)
    ap.add_argument("--frames-at", help="frames to write, with commas; the picture argument is then the prefix of the files")
    ap.add_argument("--mge-build", help="the sealed set whose helper replays (W545); without it, the current set")
    ap.add_argument("--archive", help="the folder of an unpacked release archive, which has no manifest: the release check")
    ap.add_argument("--config-root", help="the folder whose mgeXE.toml the replay uses; it is only read")
    ap.add_argument("--distant-land")
    ap.add_argument("--game-dir", help="the game folder with Morrowind.ini and the archives")
    ap.add_argument("--backend", default="vulkan")
    ap.add_argument("--core-mods", metavar="FOLDER|off", help="the core mods only from this folder, or off: none")
    ap.add_argument("--anisotropy", type=int, default=None, help="the anisotropic filter of the textures the game filters (1 without this)")
    ap.add_argument("--sky-map-filter", choices=("engine", "trilinear", "bias", "area", "area-tent"), default=None,
                    help="how the sky's draws read their BILERP maps (W632); without this, the setting of the mgeXE.toml")
    ap.add_argument("--sky-lod-bias", type=float, default=None,
                    help="the bias of the level for --sky-map-filter bias, a power of 2 (-1 without this)")
    ap.add_argument("--marks-every", type=int, default=None, help="a mark after every N frames: the helper's time for each frame by part")
    ap.add_argument("--debug-view", nargs="+", metavar=("NAME", "TERM"), help="the shader view of the replay, and its term for water and post")
    ap.add_argument("--debug-lines", metavar="NAME,NAME", help="the line sources of the replay, with commas (no lines are drawn before piece 3 of the debug views)")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    # Every given value takes the mode, an empty one too (it is refused below): with a test for a true value, an
    # empty value fell through to a real replay of the arguments after it.
    if args.run_folder_for is not None:
        try:
            folder, log = prepare_run_folder(args.run_folder_for, args.config_root or defaults()["config_root"])
        except Refused as refused:
            sys.stderr.write("replay: refused: %s\n" % refused)
            return EXIT_REFUSED
        print(folder)
        print(log)
        print("replay: the run folder stays; start the helper in it with --log and the log path above")
        return 0
    if args.capture is None or args.picture is None:
        ap.error("a capture file and a picture are needed")
    if args.debug_view and len(args.debug_view) > 2:
        ap.error("--debug-view takes a name and at most one term")
    # The build first: a call that names no set starts nothing, takes no look at the slots and makes no run folder.
    gone = gone_local_key(local_file() if local_path is None else local_path)
    if gone:
        sys.stderr.write("replay: %s\n" % gone)
        return EXIT_NOT_A_SET
    try:
        mge_build, build_line, manifest = open_build(args, verify, current)
    except Refused as refused:
        sys.stderr.write("replay: refused: %s\n" % refused)
        return EXIT_REFUSED
    except SystemExit as stop:
        sys.stderr.write("%s\n" % stop)
        return EXIT_NOT_A_SET
    sys.stderr.write("replay: %s\n" % build_line)
    default = settings or defaults()
    if args.distant_land:
        land, land_line = args.distant_land, "distant land: --distant-land %s" % args.distant_land
    else:
        import distant_land
        try:
            land, land_line = default["distant_land"](mge_build, manifest, args.config_root or default["config_root"])
        except distant_land.Problem as problem:
            sys.stderr.write("replay: %s\n" % problem)
            return EXIT_NOT_A_SET
    sys.stderr.write("replay: %s\n" % land_line)
    if default["check_build_slot"]:
        default["check_build_slot"]()
    missing = build_missing(mge_build)
    if missing:
        sys.stderr.write("replay: the build folder %s has no %s: a replay without them draws another picture\n" % (mge_build, ", ".join(missing)))
        return 2
    exe = os.path.join(mge_build, "numidium.exe")
    command = [exe, "--replay", os.path.abspath(args.capture), os.path.abspath(args.picture), "--backend", args.backend]
    if args.frames_at:
        command += ["--frames-at", args.frames_at]
    elif args.frame is not None:
        command += ["--frame", str(args.frame)]
    if args.anisotropy is not None:
        command += ["--anisotropy", str(args.anisotropy)]
    if args.sky_map_filter is not None:
        command += ["--sky-map-filter", args.sky_map_filter]
    if args.sky_lod_bias is not None:
        command += ["--sky-lod-bias", repr(args.sky_lod_bias)]
    if args.marks_every:
        command += ["--marks-every", str(args.marks_every)]
    # The helper checks the names and the term.
    if args.debug_view:
        command += ["--debug-view"] + args.debug_view
    if args.debug_lines is not None:
        command += ["--debug-lines", args.debug_lines]
    env = dict(os.environ, MGE_XE_DISTANT_LAND_DIR=land,
               MGE_XE_GAME_DIR=args.game_dir or default["game_dir"], **core_mods_env(args.core_mods))
    try:
        code, out, err = run_replay(command, args.picture, args.config_root or default["config_root"], env, args.keep_run_folder, start)
    except Refused as refused:
        sys.stderr.write("replay: refused: %s\n" % refused)
        return EXIT_REFUSED
    sys.stdout.write(out)
    sys.stderr.write(err)
    # The helper's refusal names the parts of a game; in a replay the capture is the other part.
    line = capture_format_line(os.path.basename(args.capture), code, out + err)
    if line:
        sys.stderr.write(line + "\n")
    # A replay that ended with a fault code of its own (the stage shaders' 40, W523; the checks' 41, W526): the helper's
    # line of the stage shaders has every failed key; this one is short, and the last.
    line = replay_fault_line(os.path.basename(args.capture), code, out)
    if line:
        sys.stderr.write(line + "\n")
    return code


if __name__ == "__main__":
    sys.exit(main())
