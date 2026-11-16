"""Draws a frame of a capture file with the MGE XE helper, offline: the same picture from the
same file each time. For a check of a change of the renderer against the picture before it.

    python tools/replay.py <capture file> <picture.png> [--frame N] [--mge-build DIR]
    python tools/replay.py <capture file> <prefix> --frames-at 1,3,5 [--mge-build DIR]
    python tools/replay.py --run-folder-for <picture.png> [--config-root DIR]
    python -I tools/replay.py --self-test

With --frames-at, one run of the helper writes <prefix>-<frame as 5 digits>.png for each
frame named (the frames count from 0, as --frame): one start of the helper for a capture of
many stops, in place of one for each. With --marks-every N, the helper prints a mark (its
time for each frame by part) after every N frames: the bench of a long capture.
        [--config-root DIR] [--distant-land DIR] [--backend vulkan|dx12]
        [--debug-view NAME [TERM]] [--debug-lines NAME,NAME] [--keep-run-folder]

--debug-view and --debug-lines set the debug view of the replay (the helper's names: lights,
shadow-layers, shadow-map, water TERM, post TERM; the line sources lights, shadow-boxes,
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
game folder is refused, since its run folder would be there too.
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
import locale
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from common import SIBLINGS, build_missing, capture_format_line, local, replay_fault_line  # noqa: E402

CONFIG_FILE = "mgeXE.toml"
LOG_NAME = "helper.txt"
RUN_SUFFIX = ".run"
# The tool's own files in a run folder.
OWN_FILES = (CONFIG_FILE, LOG_NAME)
# A folder with this file is a Mod Organizer instance; a folder with this program is a game folder.
INSTANCE_MARK = "ModOrganizer.ini"
GAME_MARK = "Morrowind.exe"
EXIT_REFUSED = 2


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
        "mge_build": local("mge_build", os.path.join(SIBLINGS, "MGE-XE-G7-render-link", "bin", "MSVC-Release")),
        "config_root": local("config_root", os.path.join(root, "mods", "Configuration", "Root")),
        "distant_land": local("distant_land", os.path.join(root, "overwrite", "distantland")),
        "game_dir": local("game_dir", session.harness.GAME_DIR),
        # A replay is build work. The slot hook reads only a command line, so a replay that a script or
        # another tool starts is checked here. A harness from before the check has no such function.
        "check_build_slot": getattr(session.harness, "check_build_slot", None),
    }


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
    print("replay self-test: ok")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("capture", nargs="?")
    ap.add_argument("picture", nargs="?")
    ap.add_argument("--self-test", action="store_true", help="the tool's own checks: no helper starts, nothing but a temp folder is written")
    ap.add_argument("--run-folder-for", metavar="PICTURE", help="makes the run folder of a picture and prints it and the log path; no helper starts")
    ap.add_argument("--keep-run-folder", action="store_true", help="keeps the run folder after a clean run too")
    ap.add_argument("--frame", type=int)
    ap.add_argument("--frames-at", help="frames to write, with commas; the picture argument is then the prefix of the files")
    ap.add_argument("--mge-build")
    ap.add_argument("--config-root", help="the folder whose mgeXE.toml the replay uses; it is only read")
    ap.add_argument("--distant-land")
    ap.add_argument("--game-dir", help="the game folder with Morrowind.ini and the archives")
    ap.add_argument("--backend", default="vulkan")
    ap.add_argument("--anisotropy", type=int, default=None, help="the anisotropic filter of the textures the game filters (1 without this)")
    ap.add_argument("--marks-every", type=int, default=None, help="a mark after every N frames: the helper's time for each frame by part")
    ap.add_argument("--debug-view", nargs="+", metavar=("NAME", "TERM"), help="the shader view of the replay, and its term for water and post")
    ap.add_argument("--debug-lines", metavar="NAME,NAME", help="the line sources of the replay, with commas (no lines are drawn before piece 3 of the debug views)")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    if args.run_folder_for:
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
    default = defaults()
    if default["check_build_slot"]:
        default["check_build_slot"]()
    mge_build = args.mge_build or default["mge_build"]
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
    if args.marks_every:
        command += ["--marks-every", str(args.marks_every)]
    # The helper checks the names and the term.
    if args.debug_view:
        command += ["--debug-view"] + args.debug_view
    if args.debug_lines is not None:
        command += ["--debug-lines", args.debug_lines]
    env = dict(os.environ, MGE_XE_DISTANT_LAND_DIR=args.distant_land or default["distant_land"],
               MGE_XE_GAME_DIR=args.game_dir or default["game_dir"])
    try:
        code, out, err = run_replay(command, args.picture, args.config_root or default["config_root"], env, args.keep_run_folder)
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
