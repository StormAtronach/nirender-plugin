"""Makes the next run build folder E:/nirender-keep/mgebuilds/f<N> and writes its WHAT.txt (W380, the owner's D329).

    python -I tools/new_run_folder.py --what "<who>: <item>, <tips>" [--plugin <akulakhan.dll>]
    python -I tools/new_run_folder.py --self-test    # scratch folders only: no write to E:

--plugin copies the plugin DLL of the build into the folder as akulakhan-<commit>.dll, with the commit of its build id,
and adds its md5 and its source to WHAT.txt (W520, the owner's D488). run_own_device.py with --mge-build takes that copy
when the run names no --plugin-dll. A plugin with no build id, two ids or a "-dirty" id names no commit: the tool then
makes no folder.

N is the highest number of the folders f<N> there, as a number, plus one. A name that is not "f" and digits (f29ref)
does not count. os.mkdir makes the folder and fails on a name that another session made just before; the tool then
takes the next N. So two sessions at the same moment get two folders. make_play_set.py makes its sets by the same
rule (`new_set_folder`).

The write to E: waits for no timing run: when `slot.py e-ok` says no (a timing run holds the game, or the slot cannot
be read, D297), the tool makes nothing. WHAT.txt holds the text of --what, the time and the session
(CLAUDE_CODE_SESSION_ID, with its name when slot.py finds one). The last line is the folder, with forward slashes.
Exit code 0, or 1 with one line.
"""
import argparse
import datetime
import hashlib
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import BUILD_ID_RE, docs_place  # noqa: E402

SETS = "E:/nirender-keep/mgebuilds"
# The batch file in SETS that names the current set (W500, the owner's D461): deploy_rest.py after a kept deploy and
# make_play_set.py after a set that passed write it (`write_current_bat`); clean_common.current_set reads it, and the
# owner's local.bat calls it. The tries of its rename while a reader holds it, and the wait between them.
CURRENT_BAT = "current.bat"
CURRENT_BAT_TRIES = 5
CURRENT_BAT_WAIT_SECONDS = 0.5
HARNESS = docs_place("docs", "scripts", "test-harness-2026-10-03")
NOTE = "WHAT.txt"
EXIT_REFUSED = 1


def next_set_number(names):
    """The next number of a set folder. The highest number as a number: a text sort puts f99 after f100."""
    numbers = [int(match.group(1)) for match in (re.fullmatch(r"f(\d+)", name) for name in names) if match]
    return max(numbers, default=0) + 1


def new_set_folder(root=SETS):
    """Makes the next folder f<N> in `root` and answers its name. os.mkdir fails on a name that another session has
    just made: then the next N."""
    while True:
        name = "f%d" % next_set_number(os.listdir(root))
        try:
            os.mkdir(os.path.join(root, name))
        except FileExistsError:
            continue
        return name


def current_bat_text(folder, plugin_commit):
    """The two lines of CURRENT_BAT (CRLF) for the set `folder`: the build and the plugin beside it."""
    folder = os.path.normpath(os.path.abspath(folder))
    plugin = os.path.join(folder, "akulakhan-%s.dll" % plugin_commit)
    return 'set "MGE_BUILD=%s"\r\nset "NIRENDER_PLUGIN_DLL=%s"\r\n' % (folder, plugin)


def write_current_bat(sets, name, plugin_commit):
    """Writes CURRENT_BAT in `sets` for the set `name`. The text goes to a temporary name in the same folder first and
    then takes the place of the old file in one rename, so a reader sees the old file or the new one, never a part. A
    reader that holds the old file makes the rename try again a few times. Returns the path; raises OSError when it
    cannot."""
    path = os.path.join(sets, CURRENT_BAT)
    temporary = path + ".new"
    with open(temporary, "w", encoding="ascii", newline="") as file:
        file.write(current_bat_text(os.path.join(sets, name), plugin_commit))
    for attempt in range(CURRENT_BAT_TRIES):
        try:
            os.replace(temporary, path)
            return path
        except PermissionError:
            if attempt + 1 == CURRENT_BAT_TRIES:
                raise
            time.sleep(CURRENT_BAT_WAIT_SECONDS)
    return path


def slot_tool():
    sys.path.insert(0, HARNESS)
    import slot
    return slot


def e_gate():
    """(True, line) when no timing run holds the game (`slot.py e-ok`, D297), else (False, line)."""
    try:
        slot = slot_tool()
        reply = slot.e_ok()
    except Exception as error:  # noqa: BLE001 - any fault of the slot tool means no write
        return False, "the slot tool did not answer: %s" % error
    return reply.code == slot.DONE, reply.text


def this_session():
    """The session of this process for the note: its id and, when slot.py finds it, its name."""
    session = os.environ.get("CLAUDE_CODE_SESSION_ID", "")
    if not session:
        return "no session (a terminal)"
    try:
        name = slot_tool().session_name(session)
    except Exception:  # noqa: BLE001 - the name is only for the note
        name = None
    return "%s (%s)" % (name, session) if name else session


def note_text(what, session, now, plugin_line=None):
    text = "%s\nmade: %s\nsession: %s\n" % (what, now.isoformat(timespec="seconds"), session)
    return text + plugin_line + "\n" if plugin_line else text


def md5_of(path):
    digest = hashlib.md5()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def plugin_commit(dll):
    """The commit of the plugin DLL `dll` from its build id, as (commit, None), or (None, fault): no file, no build id,
    two ids, or a "-dirty" id, which names no commit."""
    try:
        with open(dll, "rb") as file:
            data = file.read()
    except OSError as error:
        return None, "the plugin %s cannot be read: %s" % (dll, error)
    ids = sorted({m.group(2).decode() + (m.group(3) or b"").decode() for m in BUILD_ID_RE.finditer(data) if m.group(1) == b"nirender"})
    if len(ids) != 1:
        return None, "the plugin %s holds %s build id of nirender%s" % (dll, "no" if not ids else "%d different" % len(ids), ": " + ", ".join(ids) if ids else "")
    if ids[0].endswith("-dirty") or ids[0] == "unknown":
        return None, "the plugin %s is %s: a build of a work tree with changes names no commit" % (dll, ids[0])
    return ids[0], None


def copy_plugin(dll, commit, folder):
    """Copies `dll` into `folder` as akulakhan-<commit>.dll and checks the copy's md5. Returns the note's line."""
    name = "akulakhan-%s.dll" % commit
    target = os.path.join(folder, name)
    shutil.copyfile(dll, target)
    digest = md5_of(dll)
    if md5_of(target) != digest:
        raise OSError("the copy %s differs from %s" % (target, dll))
    return "plugin: %s md5 %s, a copy of %s" % (name, digest, os.path.abspath(dll).replace("\\", "/"))


def make(what, root=SETS, gate=e_gate, session=this_session, now=None, plugin=None):
    """Makes the folder and its note, and with `plugin` its plugin copy (`copy_plugin`). (exit code, lines); the last
    line is the folder, or the one line of a refusal. A plugin that names no commit makes no folder."""
    if not what.strip():
        return EXIT_REFUSED, ["new_run_folder: --what is empty: name who, the item and the tips"]
    commit = None
    if plugin:
        commit, fault = plugin_commit(plugin)
        if fault:
            return EXIT_REFUSED, ["new_run_folder: no folder made: %s" % fault]
    allowed, line = gate()
    if not allowed:
        return EXIT_REFUSED, ["new_run_folder: no folder made: the E: gate says no: %s" % line]
    try:
        name = new_set_folder(root)
    except OSError as error:
        return EXIT_REFUSED, ["new_run_folder: no folder made in %s: %s" % (root, error)]
    folder = os.path.join(root, name).replace("\\", "/")
    stamp = now or datetime.datetime.now().astimezone()
    plugin_line = None
    if plugin:
        try:
            plugin_line = copy_plugin(plugin, commit, folder)
        except OSError as error:
            return EXIT_REFUSED, ["new_run_folder: %s was made, but its plugin was not copied: %s" % (folder, error)]
    try:
        with open(os.path.join(folder, NOTE), "x", encoding="ascii", errors="backslashreplace", newline="\n") as note:
            note.write(note_text(what.strip(), session(), stamp, plugin_line))
    except OSError as error:
        return EXIT_REFUSED, ["new_run_folder: %s was made, but its %s was not written: %s" % (folder, NOTE, error)]
    return 0, [folder]


# The child of the race test: it reads the listing, waits at a barrier until every child has read it, and only then
# makes its folder. So every child starts from the same N, as two sessions at the same moment do.
RACE_CHILD = r"""
import os, sys
tools, root, barrier, index = sys.argv[1:5]
sys.path.insert(0, tools)
import new_run_folder
real = os.listdir
first = [True]
def listdir(path):
    names = real(path)
    if first[0]:
        first[0] = False
        open(os.path.join(barrier, "ready-" + index), "w").close()
        while not os.path.exists(os.path.join(barrier, "go")):
            pass
    return names
os.listdir = listdir
print(new_run_folder.new_set_folder(root))
"""
RACE_CHILDREN = 12


def race(root, barrier):
    """The names that RACE_CHILDREN processes made at the same moment."""
    tools = os.path.dirname(os.path.abspath(__file__))
    children = [subprocess.Popen([sys.executable, "-I", "-c", RACE_CHILD, tools, root, barrier, str(index)],
                                 stdout=subprocess.PIPE, text=True) for index in range(RACE_CHILDREN)]
    while len([name for name in os.listdir(barrier) if name.startswith("ready-")]) < RACE_CHILDREN:
        if any(child.poll() is not None for child in children):
            break
    open(os.path.join(barrier, "go"), "w").close()
    names = []
    for child in children:
        out, _ = child.communicate(timeout=60)
        assert child.returncode == 0, "a child of the race failed"
        names.append(out.strip())
    return names


def self_test_plugin(gate, session, stamp):
    """W520: --plugin copies the DLL as akulakhan-<commit>.dll and writes its md5 into the note; a plugin that names no
    commit makes no folder."""
    with tempfile.TemporaryDirectory(prefix="new-run-folder-w520-") as root:
        plugins = os.path.join(root, "plugins")
        os.mkdir(plugins)

        def plugin_file(name, data):
            path = os.path.join(plugins, name)
            with open(path, "wb") as file:
                file.write(data)
            return path

        good = plugin_file("good.dll", b"MZ..nirender v0.3.8 45b645b..nirender-host v0.3.8 45b645b..")
        code, lines = make("Ryker: W520", root, gate, session, stamp, plugin=good)
        folder = os.path.join(root, "f1").replace("\\", "/")
        assert (code, lines) == (0, [folder]), lines
        assert md5_of(os.path.join(folder, "akulakhan-45b645b.dll")) == md5_of(good)
        with open(os.path.join(folder, NOTE), encoding="ascii") as note:
            text = note.read()
        expected = "plugin: akulakhan-45b645b.dll md5 %s, a copy of %s\n" % (md5_of(good), os.path.abspath(good).replace("\\", "/"))
        assert text.endswith(expected), text
        # A "-dirty" id, no id, two ids, no file: no folder.
        for data, words in ((b"nirender v0.3.8 45b645b-dirty", "names no commit"), (b"no id", "holds no build id"),
                            (b"nirender v0.3.8 45b645b nirender v0.3.8 0011e58", "2 different")):
            code, lines = make("x", root, gate, session, stamp, plugin=plugin_file("bad.dll", data))
            assert code == EXIT_REFUSED and words in lines[0] and not os.path.exists(os.path.join(root, "f2")), lines
        code, lines = make("x", root, gate, session, stamp, plugin=os.path.join(plugins, "none.dll"))
        assert code == EXIT_REFUSED and "cannot be read" in lines[0] and not os.path.exists(os.path.join(root, "f2")), lines
        # Without --plugin: no copy and no plugin line (the null of the copy above).
        code, lines = make("y", root, gate, session, stamp)
        with open(os.path.join(lines[0], NOTE), encoding="ascii") as note:
            assert "plugin:" not in note.read()
        assert [name for name in os.listdir(lines[0]) if name.startswith("akulakhan")] == []


def self_test():
    assert next_set_number(["f99", "f100", "f7", "other", "f12x"]) == 101
    assert next_set_number(["f29ref", "f29", "F30"]) == 30, "only f and digits count"
    assert next_set_number([]) == 1
    yes = lambda: (True, "e-ok: yes")  # noqa: E731
    session = lambda: "s1 (id)"  # noqa: E731
    stamp = datetime.datetime(2026, 11, 12, 6, 0, 0, tzinfo=datetime.timezone(datetime.timedelta(hours=4)))
    with tempfile.TemporaryDirectory(prefix="new-run-folder-") as root:
        for name in ("f7", "f29ref", "notes"):
            os.mkdir(os.path.join(root, name))
        code, lines = make("Ryker: W380, a1b2c3d", root, yes, session, stamp)
        folder = os.path.join(root, "f8").replace("\\", "/")
        assert (code, lines) == (0, [folder]), lines
        with open(os.path.join(folder, NOTE), encoding="ascii") as note:
            assert note.read() == "Ryker: W380, a1b2c3d\nmade: 2026-11-12T06:00:00+04:00\nsession: s1 (id)\n"
        # The gate: a timing run makes no folder (the null of the gate is the make above).
        code, lines = make("x", root, lambda: (False, "e-ok: no: a timing run holds the game: Hob"), session, stamp)
        assert code == EXIT_REFUSED and len(lines) == 1 and "a timing run holds the game: Hob" in lines[0], lines
        assert not os.path.exists(os.path.join(root, "f9")), "no folder after a refusal"
        code, lines = make("  ", root, yes, session, stamp)
        assert code == EXIT_REFUSED and "--what is empty" in lines[0] and not os.path.exists(os.path.join(root, "f9"))
        code, lines = make("x", os.path.join(root, "no-such-folder"), yes, session, stamp)
        assert code == EXIT_REFUSED and len(lines) == 1 and "no folder made" in lines[0], lines
        self_test_plugin(yes, session, stamp)
        # A name another session made just before: the next N.
        real, made = os.listdir, []

        def listdir_then_other(path):
            names = real(path)
            if not made:
                other = "f%d" % next_set_number(names)
                os.mkdir(os.path.join(path, other))
                made.append(other)
            return names

        os.listdir = listdir_then_other
        try:
            name = new_set_folder(root)
        finally:
            os.listdir = real
        assert made == ["f9"] and name == "f10", (made, name)
        # The race: RACE_CHILDREN processes that read the listing at the same moment get as many folders. A mkdir
        # replaced by an exists check and makedirs gives two of them one name (the null).
        barrier = os.path.join(root, "barrier")
        os.mkdir(barrier)
        names = race(root, barrier)
        assert len(set(names)) == RACE_CHILDREN, "two processes got one folder: %s" % sorted(names)
        assert sorted(names, key=lambda name: int(name[1:])) == ["f%d" % n for n in range(11, 11 + RACE_CHILDREN)], names
    print("new_run_folder self-test: ok")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--what", help="who, the item and the tips of the build, for WHAT.txt")
    parser.add_argument("--plugin", help="the plugin DLL of the build: copied into the folder as akulakhan-<commit>.dll, its md5 in WHAT.txt")
    parser.add_argument("--self-test", action="store_true", help="the tests on scratch folders: no write to E:")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if args.what is None:
        print("new_run_folder: it needs --what \"<who>: <item>, <tips>\"")
        return EXIT_REFUSED
    code, lines = make(args.what, plugin=args.plugin)
    for line in lines:
        print(line)
    return code


if __name__ == "__main__":
    sys.exit(main())
