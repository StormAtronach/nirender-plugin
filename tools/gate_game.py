"""The game folder of the replay gate, with the pinned post files of the packs (W609 step 5 part 0).

The design: D640 piece 2 addendum 2 (docs 0f77e947, section 2) and the Architect's answers of 2026-11-24. A replay
finds a post file in its run folder, then in --game-dir, then beside the set's program (renderer.rs `data_places`).
The GOG game folder has no `shaders` folder, so without this folder a gate replay runs no pack: the chain states of
the gate's captures that name a mod's shader (ashfall, skoomaesthesia) find no file and are skipped.

    python -I tools/gate_game.py make <folder> --set <set> [--set <base set>] --render-link <render-link tree>
    python -I tools/gate_game.py check <folder> --set <set> [--set <base set>]
    python -I tools/gate_game.py remove <folder>
    python -I tools/gate_game.py --self-test

make: the folder links each entry of the game folder (replay.py's default --game-dir) except `Data Files`, by a
symbolic link. `Data Files` is a real folder of its own: it links each entry of the game's `Data Files` except
`shaders`, and `shaders/XEshaders` is real and holds the pinned files of gate-packs.txt. Each pinned file is read
from the render-link commit of the first --set's manifest (`git show`) in the --render-link tree, and its md5 must be
the list's. A failed symbolic link stops the maker with the reason: it never copies the game's files in place of a
link. A folder of that name from an earlier round (a round that crashed) is removed first.

check: `Data Files`, `shaders` and `XEshaders` are real folders; each pinned file is there with its md5; no real
file of the folder has a path under the game folder; and no pinned name has the stem of a post file (.fx or .wgsl)
in `Data Files/shaders/XEshaders` of a --set (rule 4.2: the game folder comes before the set in `data_places`, so a
pinned file would hide that set's file). validate_retained.py runs it with the sets of its stage before the replays.

remove: only a folder whose name starts with "gate-game-". It never goes through a link: each link is removed as a
link, then the files and folders that the maker made. merge_step.py removes the round's folder at the round's end,
also when the round fails.

The links need the right to make symbolic links (Windows' developer mode, or an administrator).
"""
import argparse
import hashlib
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

LIST = os.path.join(HERE, "gate-packs.txt")
NAME_START = "gate-game-"
DATA = "Data Files"
POST_FOLDER = ("shaders", "XEshaders")
POST_SUFFIXES = (".fx", ".wgsl")
CHANGE_HINT = "a new line in gate-packs.txt and a planned change in the gate list"


class GateGameError(Exception):
    """A fault of the gate game folder, with the line that says what to do."""


def md5_of(data):
    return hashlib.md5(data).hexdigest()


def read_list(path=LIST):
    """The pinned files: [(chain name, path in render-link, md5)]. Raises ValueError with the line of a fault."""
    entries = []
    with open(path, encoding="utf-8") as file:
        for number, line in enumerate(file, 1):
            line = line.rstrip("\r\n")
            if not line.strip() or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) != 3 or not parts[0] or not parts[1]:
                raise ValueError("%s:%d: a line is the chain name, the pack path and the md5, split by tabs: %r" % (path, number, line))
            name, pack_path, md5 = parts
            if len(md5) != 32 or any(c not in "0123456789abcdef" for c in md5):
                raise ValueError("%s:%d: %r is not an md5 (32 small hex digits)" % (path, number, md5))
            if not pack_path.endswith("/shaders/XEshaders/%s.wgsl" % name):
                raise ValueError("%s:%d: the pack path %s is not the file of the chain name %s" % (path, number, pack_path, name))
            if any(name.lower() == other.lower() for other, _, _ in entries):
                raise ValueError("%s:%d: %s is in the list two times" % (path, number, name))
            entries.append((name, pack_path, md5))
    return entries


def post_stems(set_folder):
    """The stems of the post files of a set: the paths under its XEshaders without the suffix, with "/" and in small
    letters ("bloom fine", "inspectit/bokeh")."""
    root = os.path.join(set_folder, DATA, *POST_FOLDER)
    stems = set()
    for folder, _dirs, files in os.walk(root):
        for file in files:
            stem, suffix = os.path.splitext(file)
            if suffix.lower() in POST_SUFFIXES:
                relative = os.path.relpath(os.path.join(folder, stem), root)
                stems.add(relative.replace(os.sep, "/").lower())
    return stems


def stem_problems(names, sets):
    """Rule 4.2: a pinned name with the stem of a post file of a set hides that file. The lines of the faults."""
    problems = []
    for set_folder in sets:
        stems = post_stems(set_folder)
        for name in names:
            if name.lower() in stems:
                problems.append("the pinned file %s.wgsl has the name of a post file of the set %s: the game folder comes "
                                "before the set, so it would hide that file (addendum 2, rule 4.2)" % (name, set_folder))
    return problems


def render_link_commit(set_folder):
    """The render-link commit of a sealed set, from its manifest."""
    import sets
    return sets.read_manifest(set_folder)["commits"]["render-link"]["commit"]


def git_show(tree, commit, path):
    result = subprocess.run(["git", "-C", tree, "show", "%s:%s" % (commit, path)], capture_output=True)
    if result.returncode != 0:
        raise GateGameError("git show %s:%s in %s failed: %s" % (commit[:10], path, tree, result.stderr.decode(errors="replace").strip()))
    return result.stdout


def pinned_files(entries, tree, commit, show=git_show):
    """[(chain name, bytes)] of the list, read from the render-link commit. Raises GateGameError for another md5."""
    files = []
    for name, pack_path, md5 in entries:
        data = show(tree, commit, pack_path)
        found = md5_of(data)
        if found != md5:
            raise GateGameError("the pack file %s at render-link %s has md5 %s, and gate-packs.txt pins %s: the gate replays a "
                                "pinned file only. A changed pack file is %s" % (pack_path, commit[:10], found, md5, CHANGE_HINT))
        files.append((name, data))
    return files


def default_game():
    """replay.py's default --game-dir, the folder whose archives the replays read."""
    import replay
    return replay.defaults()["game_dir"]


def is_link(path):
    """A symbolic link or a junction: the remover never goes through one."""
    return os.path.islink(path) or bool(getattr(os.path, "isjunction", lambda _p: False)(path))


def checked_name(folder):
    name = os.path.basename(os.path.normpath(folder))
    if not name.startswith(NAME_START):
        raise GateGameError("%s: the name of a gate game folder starts with %r, so that no other folder is made or removed "
                            "by this tool" % (folder, NAME_START))


def link(source, target):
    try:
        os.symlink(source, target, target_is_directory=os.path.isdir(source))
    except OSError as error:
        raise GateGameError("the symbolic link %s -> %s failed: %s. The maker copies nothing in place of a link; give the "
                            "right to make symbolic links (Windows' developer mode)" % (target, source, error)) from None


def link_entries(source, target, skip):
    os.makedirs(target)
    for name in sorted(os.listdir(source)):
        if name.lower() not in skip:
            link(os.path.join(source, name), os.path.join(target, name))


def make(folder, game, files):
    """The gate game folder of `game` with the pinned `files` [(chain name, bytes)]. A folder of that name from an
    earlier round is removed first. On a fault the made part is removed, and GateGameError says why."""
    checked_name(folder)
    if os.path.lexists(folder):
        print("gate game folder: %s is there from an earlier round: removed first" % folder, flush=True)
        remove(folder)
    if any(name.lower() == "shaders" for name in os.listdir(os.path.join(game, DATA))):
        raise GateGameError("%s has a shaders folder: the gate folder links no shaders of the game, and its files would be "
                            "missing from the replays; tell the Architect" % os.path.join(game, DATA))
    try:
        link_entries(game, folder, skip=(DATA.lower(),))
        link_entries(os.path.join(game, DATA), os.path.join(folder, DATA), skip=("shaders",))
        posts = os.path.join(folder, DATA, *POST_FOLDER)
        os.makedirs(posts)
        for name, data in files:
            path = os.path.join(posts, *name.split("/")) + ".wgsl"
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "wb") as file:
                file.write(data)
    except BaseException:
        remove(folder)
        raise


def under(path, folder):
    path, folder = os.path.normcase(os.path.realpath(path)), os.path.normcase(os.path.realpath(folder))
    return path == folder or path.startswith(folder.rstrip(os.sep) + os.sep)


def check(folder, game, entries, sets):
    """The faults of a gate game folder, as lines; [] when it is right."""
    problems = []
    data = os.path.join(folder, DATA)
    shaders = os.path.join(data, POST_FOLDER[0])
    posts = os.path.join(shaders, POST_FOLDER[1])
    for path in (folder, data, shaders, posts):
        if not os.path.isdir(path) or is_link(path):
            problems.append("%s is not a real folder of the gate folder" % path)
    if problems:
        return problems
    for name, _pack_path, md5 in entries:
        path = os.path.join(posts, *name.split("/")) + ".wgsl"
        if not os.path.isfile(path):
            problems.append("the pinned file %s is missing" % path)
            continue
        with open(path, "rb") as file:
            found = md5_of(file.read())
        if found != md5:
            problems.append("the pinned file %s has md5 %s, not the list's %s" % (path, found, md5))
    for path in real_files(folder):
        if under(path, game):
            problems.append("%s is a file of the game folder %s" % (path, game))
    problems += stem_problems([name for name, _, _ in entries], sets)
    return problems


def real_files(folder):
    """The files and folders that the maker made: the walk never goes through a link."""
    found = []
    for entry in os.scandir(folder):
        if is_link(entry.path):
            continue
        found.append(entry.path)
        if entry.is_dir(follow_symlinks=False):
            found += real_files(entry.path)
    return found


def remove_link(path):
    """A link as a link: a folder link with rmdir (a junction too), a file link with unlink."""
    if os.path.isdir(path):
        os.rmdir(path)
    else:
        os.unlink(path)


def clear(folder):
    for entry in os.scandir(folder):
        if is_link(entry.path):
            remove_link(entry.path)
        elif entry.is_dir(follow_symlinks=False):
            clear(entry.path)
            os.rmdir(entry.path)
        else:
            os.remove(entry.path)


def remove(folder):
    """Removes a gate game folder: its links as links, then what the maker made. Nothing when it is not there."""
    checked_name(folder)
    if not os.path.lexists(folder):
        return
    if is_link(folder):
        raise GateGameError("%s is a link, not a gate game folder" % folder)
    clear(folder)
    os.rmdir(folder)


def self_test():
    with tempfile.TemporaryDirectory() as scratch:
        # The list.
        listed = os.path.join(scratch, "list.txt")
        with open(listed, "w", encoding="utf-8") as file:
            file.write("# a comment\nashfall\tports/A - wgpu shaders/shaders/XEshaders/ashfall.wgsl\t%s\n"
                       "InspectIt/Bokeh\tports/I - wgpu shaders/shaders/XEshaders/InspectIt/Bokeh.wgsl\t%s\n" % (md5_of(b"a"), md5_of(b"b")))
        entries = read_list(listed)
        assert [name for name, _, _ in entries] == ["ashfall", "InspectIt/Bokeh"], entries
        for bad in ("ashfall\tports/x.wgsl\t%s\n" % md5_of(b"a"), "ashfall\tports/A/shaders/XEshaders/ashfall.wgsl\tnot-an-md5\n",
                    "ashfall ports/A/shaders/XEshaders/ashfall.wgsl %s\n" % md5_of(b"a"),
                    "a\tp/shaders/XEshaders/a.wgsl\t%s\nA\tq/shaders/XEshaders/A.wgsl\t%s\n" % (md5_of(b"a"), md5_of(b"a"))):
            with open(listed, "w", encoding="utf-8") as file:
                file.write(bad)
            try:
                read_list(listed)
                raise AssertionError("a bad list line passed: %r" % bad)
            except ValueError:
                pass
        # The real list reads.
        assert read_list(), "gate-packs.txt is empty"

        # A changed pack file: the refusal says what to do.
        show = lambda _tree, _commit, path: b"a" if path.endswith("ashfall.wgsl") else b"changed"  # noqa: E731
        try:
            pinned_files(entries, "tree", "0123456789abcdef", show)
            raise AssertionError("a changed pack file passed")
        except GateGameError as error:
            assert CHANGE_HINT in str(error) and md5_of(b"b") in str(error) and md5_of(b"changed") in str(error), error
        files = pinned_files(entries, "tree", "0123456789abcdef", lambda _t, _c, path: b"a" if path.endswith("ashfall.wgsl") else b"b")

        # A game folder with a sentinel behind a folder link, and a set.
        game = os.path.join(scratch, "game")
        os.makedirs(os.path.join(game, DATA, "Textures"))
        for path, text in ((os.path.join(game, "Morrowind.ini"), "ini"), (os.path.join(game, DATA, "Morrowind.esm"), "esm"),
                           (os.path.join(game, DATA, "Textures", "sentinel.txt"), "keep me")):
            with open(path, "w", encoding="utf-8") as file:
                file.write(text)
        the_set = os.path.join(scratch, "set")
        os.makedirs(os.path.join(the_set, DATA, *POST_FOLDER, "Sub"))
        for name in ("Bloom Fine.wgsl", "Bloom Fine.fx", os.path.join("Sub", "Deep.fx"), "notes.txt"):
            open(os.path.join(the_set, DATA, *POST_FOLDER, name), "w").close()
        assert post_stems(the_set) == {"bloom fine", "sub/deep"}, post_stems(the_set)

        folder = os.path.join(scratch, NAME_START + "test")
        make(folder, game, files)
        assert is_link(os.path.join(folder, "Morrowind.ini")) and is_link(os.path.join(folder, DATA, "Textures")), os.listdir(folder)
        assert not is_link(os.path.join(folder, DATA)) and os.path.isdir(os.path.join(folder, DATA))
        with open(os.path.join(folder, DATA, "Textures", "sentinel.txt"), encoding="utf-8") as file:
            assert file.read() == "keep me", "the link reaches the game's file"
        with open(os.path.join(folder, DATA, *POST_FOLDER, "InspectIt", "Bokeh.wgsl"), "rb") as file:
            assert file.read() == b"b"
        assert check(folder, game, entries, [the_set]) == [], check(folder, game, entries, [the_set])

        # Rule 4.2, in either set.
        other = os.path.join(scratch, "base")
        os.makedirs(os.path.join(other, DATA, *POST_FOLDER, "InspectIt"))
        open(os.path.join(other, DATA, *POST_FOLDER, "InspectIt", "bokeh.fx"), "w").close()
        found = check(folder, game, entries, [the_set, other])
        assert len(found) == 1 and "InspectIt/Bokeh.wgsl" in found[0] and "rule 4.2" in found[0], found

        # A changed pinned file, a missing one, and Data Files as a link are faults.
        posts = os.path.join(folder, DATA, *POST_FOLDER)
        with open(os.path.join(posts, "ashfall.wgsl"), "wb") as file:
            file.write(b"other")
        assert any("not the list's" in line for line in check(folder, game, entries, [the_set]))
        os.remove(os.path.join(posts, "ashfall.wgsl"))
        assert any("is missing" in line for line in check(folder, game, entries, [the_set]))

        # The null of the remover: a sentinel behind the links survives, and the folder is gone.
        remove(folder)
        assert not os.path.lexists(folder)
        for path in (os.path.join(game, DATA, "Textures", "sentinel.txt"), os.path.join(game, DATA, "Morrowind.esm"),
                     os.path.join(game, "Morrowind.ini")):
            assert os.path.isfile(path), "the remover went through a link: %s is gone" % path
        remove(folder)

        linked = os.path.join(scratch, NAME_START + "linked")
        os.makedirs(linked)
        os.symlink(os.path.join(game, DATA), os.path.join(linked, DATA), target_is_directory=True)
        assert any("not a real folder" in line for line in check(linked, game, entries, []))
        remove(linked)
        assert os.path.isfile(os.path.join(game, DATA, "Morrowind.esm")), "the remover went through the Data Files link"

        # A stale folder of an earlier round is removed by the next make; another name is refused.
        make(folder, game, files)
        make(folder, game, files)
        assert check(folder, game, entries, [the_set]) == []
        remove(folder)
        for wrong in (os.path.join(scratch, "game"), os.path.join(scratch, "results")):
            try:
                remove(wrong)
                raise AssertionError("remove took %s" % wrong)
            except GateGameError:
                pass
        assert os.path.isfile(os.path.join(game, "Morrowind.ini"))

        # A failed symbolic link stops the maker, which copies nothing and leaves no part.
        real_symlink = os.symlink

        def refused(*_args, **_kw):
            raise OSError(1314, "A required privilege is not held by the client")
        os.symlink = refused
        try:
            make(folder, game, files)
            raise AssertionError("the maker passed without links")
        except GateGameError as error:
            assert "copies nothing" in str(error), error
        finally:
            os.symlink = real_symlink
        assert not os.path.lexists(folder), "the maker left a part"

        # A game folder with its own shaders folder is refused.
        os.makedirs(os.path.join(game, DATA, "shaders"))
        try:
            make(folder, game, files)
            raise AssertionError("a game folder with shaders passed")
        except GateGameError:
            pass
        assert not os.path.lexists(folder)
    print("gate_game self-test: passed")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("action", nargs="?", choices=("make", "check", "remove"))
    ap.add_argument("folder", nargs="?")
    ap.add_argument("--set", action="append", default=[], help="a sealed set of the round: the first gives the render-link commit (make)")
    ap.add_argument("--render-link", help="make: the render-link tree that has the commit of the set")
    ap.add_argument("--game-dir", help="the game folder to link (replay.py's default without it)")
    ap.add_argument("--list", default=LIST, help="the pinned files (tools/gate-packs.txt)")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.action or not args.folder:
        ap.error("give an action and a folder")
    try:
        if args.action == "remove":
            remove(args.folder)
            print("gate game folder: removed %s" % args.folder, flush=True)
            return 0
        entries = read_list(args.list)
        game = args.game_dir or default_game()
        if args.action == "make":
            if not args.set or not args.render_link:
                ap.error("make takes --set and --render-link")
            commit = render_link_commit(args.set[0])
            make(args.folder, game, pinned_files(entries, args.render_link, commit))
        problems = check(args.folder, game, entries, args.set)
        if problems:
            for line in problems:
                print("  FAIL " + line, flush=True)
            return 1
        print("gate game folder: %s, links to %s, %d pinned files (%s) of %s" % (
            args.folder, game, len(entries), ", ".join(name for name, _, _ in entries), os.path.basename(args.list)), flush=True)
        return 0
    except (GateGameError, ValueError) as error:
        print("  FAIL %s" % error, flush=True)
        return 1


if __name__ == "__main__":
    sys.exit(main())
