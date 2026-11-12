"""Makes the next run build folder E:/nirender-keep/mgebuilds/f<N> and writes its WHAT.txt (W380, the owner's D329).

    python -I tools/new_run_folder.py --what "<who>: <item>, <tips>"
    python -I tools/new_run_folder.py --self-test    # scratch folders only: no write to E:

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
import os
import re
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import docs_place  # noqa: E402

SETS = "E:/nirender-keep/mgebuilds"
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


def note_text(what, session, now):
    return "%s\nmade: %s\nsession: %s\n" % (what, now.isoformat(timespec="seconds"), session)


def make(what, root=SETS, gate=e_gate, session=this_session, now=None):
    """Makes the folder and its note. (exit code, lines); the last line is the folder, or the one line of a refusal."""
    if not what.strip():
        return EXIT_REFUSED, ["new_run_folder: --what is empty: name who, the item and the tips"]
    allowed, line = gate()
    if not allowed:
        return EXIT_REFUSED, ["new_run_folder: no folder made: the E: gate says no: %s" % line]
    try:
        name = new_set_folder(root)
    except OSError as error:
        return EXIT_REFUSED, ["new_run_folder: no folder made in %s: %s" % (root, error)]
    folder = os.path.join(root, name).replace("\\", "/")
    stamp = now or datetime.datetime.now().astimezone()
    try:
        with open(os.path.join(folder, NOTE), "x", encoding="ascii", errors="backslashreplace", newline="\n") as note:
            note.write(note_text(what.strip(), session(), stamp))
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
    parser.add_argument("--self-test", action="store_true", help="the tests on scratch folders: no write to E:")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if args.what is None:
        print("new_run_folder: it needs --what \"<who>: <item>, <tips>\"")
        return EXIT_REFUSED
    code, lines = make(args.what)
    for line in lines:
        print(line)
    return code


if __name__ == "__main__":
    sys.exit(main())
