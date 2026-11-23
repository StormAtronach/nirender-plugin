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
`shaders` and `Textures`, and `shaders/XEshaders` is real and holds the pinned post files of gate-packs.txt. Each
pinned post file is read from the render-link commit of the first --set's manifest (`git show`) in the --render-link
tree, and its md5 must be the list's. `Textures` is real too: it links each entry of the game's `Textures` (one
level: a file or a folder is one link) and holds the pinned textures of the list, each read from its keep copy on E:
with the list's md5. A pinned texture with the name of a texture of the game is refused, as it would hide the game's
file. Each texture that a pinned post file names (its `//! texture:` line) must be a pinned texture or a texture of
the game: else the maker stops, because the helper reads a texture that it does not find as black. A failed
symbolic link stops the maker with the reason: it never copies the game's files in place of a link. A folder of
that name from an earlier round (a round that crashed) is removed first.

check: `Data Files`, `shaders`, `XEshaders` and `Textures` are real folders; each pinned file is a real file there
with its md5; each texture that a pinned post file names is in the folder's `Textures`; no real file of the folder
has a path under the game folder; and no pinned name has the stem of a post file (.fx or .wgsl) in
`Data Files/shaders/XEshaders` of a --set (rule 4.2: the game folder comes before the set in `data_places`, so a
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
TEXTURES = "Textures"
TEXTURE_LINE = "//! texture:"
CHANGE_HINT = "a new line in gate-packs.txt and a planned change in the gate list"
MISSING_HINT = ("the helper reads a texture that it does not find as black. Pin it: a keep copy on E: and a line in "
                "gate-packs.txt with the chain name, the keep path and the md5")


class GateGameError(Exception):
    """A fault of the gate game folder, with the line that says what to do."""


def md5_of(data):
    return hashlib.md5(data).hexdigest()


def is_post(entry):
    """A line of a post file (a .wgsl path in render-link); the other lines are textures (a keep path on E:)."""
    return entry[1].lower().endswith(".wgsl")


def posts_of(entries):
    return [entry for entry in entries if is_post(entry)]


def textures_of(entries):
    return [entry for entry in entries if not is_post(entry)]


def texture_name(entry):
    """The file name of a pinned texture in `Textures`: the name of its keep copy."""
    return os.path.basename(entry[1].replace("\\", "/"))


def read_list(path=LIST):
    """The pinned files: [(chain name, path, md5)]. A post file's path is in render-link and ends with
    /shaders/XEshaders/<chain name>.wgsl; a texture's path is its keep copy, a full path. Raises ValueError with the
    line of a fault."""
    entries = []
    with open(path, encoding="utf-8") as file:
        for number, line in enumerate(file, 1):
            line = line.rstrip("\r\n")
            if not line.strip() or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) != 3 or not parts[0] or not parts[1]:
                raise ValueError("%s:%d: a line is the chain name, the path and the md5, split by tabs: %r" % (path, number, line))
            name, file_path, md5 = parts
            if len(md5) != 32 or any(c not in "0123456789abcdef" for c in md5):
                raise ValueError("%s:%d: %r is not an md5 (32 small hex digits)" % (path, number, md5))
            entry = (name, file_path, md5)
            if is_post(entry):
                if not file_path.endswith("/shaders/XEshaders/%s.wgsl" % name):
                    raise ValueError("%s:%d: the pack path %s is not the file of the chain name %s" % (path, number, file_path, name))
                if any(name.lower() == other[0].lower() for other in posts_of(entries)):
                    raise ValueError("%s:%d: %s is in the list two times" % (path, number, name))
            else:
                if not os.path.isabs(file_path):
                    raise ValueError("%s:%d: the texture %s is not a full path: a texture line names its keep copy on E:"
                                     % (path, number, file_path))
                if any(texture_name(entry).lower() == texture_name(other).lower() for other in textures_of(entries)):
                    raise ValueError("%s:%d: the texture %s is in the list two times" % (path, number, texture_name(entry)))
            entries.append(entry)
    names = [entry[0].lower() for entry in posts_of(entries)]
    for name, file_path, _md5 in textures_of(entries):
        if name.lower() not in names:
            raise ValueError("%s: the texture %s is pinned for %s, which has no post file in the list" % (path, file_path, name))
    return entries


def header_textures(data):
    """The textures that a WGSL post file names in its `//! texture:` lines (a list split by commas)."""
    names = []
    for line in data.decode("utf-8", errors="replace").splitlines():
        if line.startswith(TEXTURE_LINE):
            names += [item.strip() for item in line[len(TEXTURE_LINE):].split(",") if item.strip()]
    return names


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
    """[(chain name, bytes)] of the list's post files, read from the render-link commit. Raises GateGameError for
    another md5."""
    files = []
    for name, pack_path, md5 in posts_of(entries):
        data = show(tree, commit, pack_path)
        found = md5_of(data)
        if found != md5:
            raise GateGameError("the pack file %s at render-link %s has md5 %s, and gate-packs.txt pins %s: the gate replays a "
                                "pinned file only. A changed pack file is %s" % (pack_path, commit[:10], found, md5, CHANGE_HINT))
        files.append((name, data))
    return files


def read_keep(path):
    try:
        with open(path, "rb") as file:
            return file.read()
    except OSError as error:
        raise GateGameError("the keep copy %s of a pinned texture is not readable: %s" % (path, error)) from None


def pinned_textures(entries, read=read_keep):
    """[(file name, bytes)] of the list's textures, read from their keep copies. Raises GateGameError for another
    md5."""
    textures = []
    for name, keep_path, md5 in textures_of(entries):
        data = read(keep_path)
        found = md5_of(data)
        if found != md5:
            raise GateGameError("the keep copy %s of a texture of %s has md5 %s, and gate-packs.txt pins %s: the gate replays "
                                "a pinned file only. A changed texture is %s" % (keep_path, name, found, md5, CHANGE_HINT))
        textures.append((texture_name((name, keep_path, md5)), data))
    return textures


def entry_names(folder):
    """The names of the entries of a folder in small letters; none when it is not there."""
    return {name.lower() for name in os.listdir(folder)} if os.path.isdir(folder) else set()


def game_textures(game):
    """The game's `Data Files/Textures`, by its name in the game folder (any case); None when the game has none."""
    data = os.path.join(game, DATA)
    for name in os.listdir(data):
        if name.lower() == TEXTURES.lower() and os.path.isdir(os.path.join(data, name)):
            return os.path.join(data, name)
    return None


def texture_problems(files, textures, game):
    """The faults of the textures before a make, as lines: a pinned texture with the name of a texture of the game,
    and a texture that a pinned post file names and that is neither pinned nor a texture of the game."""
    problems = []
    source = game_textures(game)
    present = entry_names(source) if source else set()
    pinned = {name.lower() for name, _ in textures}
    for name in sorted(pinned & present):
        problems.append("the pinned texture %s has the name of a texture of the game in %s: it would hide the game's file"
                        % (name, source))
    for post, data in files:
        for texture in header_textures(data):
            if texture.lower() in pinned:
                continue
            if source and os.path.isfile(os.path.join(source, *texture.split("/"))):
                continue
            problems.append("the post file %s.wgsl names the texture %s, and the gate folder would not hold it: %s"
                            % (post, texture, MISSING_HINT))
    return problems


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


def make(folder, game, files, textures=()):
    """The gate game folder of `game` with the pinned post `files` [(chain name, bytes)] and the pinned `textures`
    [(file name, bytes)]. A folder of that name from an earlier round is removed first. On a fault the made part is
    removed, and GateGameError says why."""
    checked_name(folder)
    if os.path.lexists(folder):
        print("gate game folder: %s is there from an earlier round: removed first" % folder, flush=True)
        remove(folder)
    if any(name.lower() == "shaders" for name in os.listdir(os.path.join(game, DATA))):
        raise GateGameError("%s has a shaders folder: the gate folder links no shaders of the game, and its files would be "
                            "missing from the replays; tell the Architect" % os.path.join(game, DATA))
    problems = texture_problems(files, textures, game)
    if problems:
        raise GateGameError("; ".join(problems))
    try:
        link_entries(game, folder, skip=(DATA.lower(),))
        link_entries(os.path.join(game, DATA), os.path.join(folder, DATA), skip=("shaders", TEXTURES.lower()))
        posts = os.path.join(folder, DATA, *POST_FOLDER)
        os.makedirs(posts)
        for name, data in files:
            path = os.path.join(posts, *name.split("/")) + ".wgsl"
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "wb") as file:
                file.write(data)
        source = game_textures(game)
        target = os.path.join(folder, DATA, TEXTURES)
        if source:
            link_entries(source, target, skip=())
        else:
            os.makedirs(target)
        for name, data in textures:
            with open(os.path.join(target, name), "wb") as file:
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
    textures = os.path.join(data, TEXTURES)
    for path in (folder, data, shaders, posts, textures):
        if not os.path.isdir(path) or is_link(path):
            problems.append("%s is not a real folder of the gate folder" % path)
    if problems:
        return problems
    pinned = [(os.path.join(posts, *name.split("/")) + ".wgsl", md5) for name, _path, md5 in posts_of(entries)]
    pinned += [(os.path.join(textures, texture_name(entry)), entry[2]) for entry in textures_of(entries)]
    for path, md5 in pinned:
        if not os.path.isfile(path) or is_link(path):
            problems.append("the pinned file %s is missing (or is a link)" % path)
            continue
        with open(path, "rb") as file:
            found = md5_of(file.read())
        if found != md5:
            problems.append("the pinned file %s has md5 %s, not the list's %s" % (path, found, md5))
    for name, _path, _md5 in posts_of(entries):
        path = os.path.join(posts, *name.split("/")) + ".wgsl"
        if not os.path.isfile(path):
            continue
        with open(path, "rb") as file:
            named = header_textures(file.read())
        for texture in named:
            if not os.path.isfile(os.path.join(textures, *texture.split("/"))):
                problems.append("the post file %s.wgsl names the texture %s, and the gate folder does not hold it: %s"
                                % (name, texture, MISSING_HINT))
    for path in real_files(folder):
        if under(path, game):
            problems.append("%s is a file of the game folder %s" % (path, game))
    problems += stem_problems([name for name, _, _ in posts_of(entries)], sets)
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
        # The list: two post files, one of which names a texture to pin and a texture of the game in a subfolder.
        bokeh = b"//! interface: mge-xe-wgsl 1\n//! texture: pinned.tga, sub/deep.dds\n"
        assert header_textures(bokeh) == ["pinned.tga", "sub/deep.dds"], header_textures(bokeh)
        assert header_textures(b"a") == []
        keep = os.path.join(scratch, "keep", "pinned.tga")
        os.makedirs(os.path.dirname(keep))
        with open(keep, "wb") as file:
            file.write(b"tga")
        listed = os.path.join(scratch, "list.txt")
        with open(listed, "w", encoding="utf-8") as file:
            file.write("# a comment\nashfall\tports/A - wgpu shaders/shaders/XEshaders/ashfall.wgsl\t%s\n"
                       "InspectIt/Bokeh\tports/I - wgpu shaders/shaders/XEshaders/InspectIt/Bokeh.wgsl\t%s\n"
                       "InspectIt/Bokeh\t%s\t%s\n" % (md5_of(b"a"), md5_of(bokeh), keep, md5_of(b"tga")))
        entries = read_list(listed)
        assert [name for name, _, _ in posts_of(entries)] == ["ashfall", "InspectIt/Bokeh"], entries
        assert [texture_name(entry) for entry in textures_of(entries)] == ["pinned.tga"], entries
        post = "ashfall\tp/shaders/XEshaders/ashfall.wgsl\t%s\n" % md5_of(b"a")
        for bad in ("ashfall\tports/x.wgsl\t%s\n" % md5_of(b"a"), "ashfall\tports/A/shaders/XEshaders/ashfall.wgsl\tnot-an-md5\n",
                    "ashfall ports/A/shaders/XEshaders/ashfall.wgsl %s\n" % md5_of(b"a"),
                    "a\tp/shaders/XEshaders/a.wgsl\t%s\nA\tq/shaders/XEshaders/A.wgsl\t%s\n" % (md5_of(b"a"), md5_of(b"a")),
                    # A texture by a relative path, a texture of a chain without a post file, a texture two times.
                    post + "ashfall\tTextures/x.tga\t%s\n" % md5_of(b"x"),
                    "ashfall\t%s\t%s\n" % (keep, md5_of(b"tga")),
                    post + "ashfall\t%s\t%s\nashfall\t%s\t%s\n" % (keep, md5_of(b"tga"), keep.upper(), md5_of(b"tga"))):
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
            assert CHANGE_HINT in str(error) and md5_of(bokeh) in str(error) and md5_of(b"changed") in str(error), error
        files = pinned_files(entries, "tree", "0123456789abcdef", lambda _t, _c, path: b"a" if path.endswith("ashfall.wgsl") else bokeh)
        assert [name for name, _ in files] == ["ashfall", "InspectIt/Bokeh"], files

        # The textures come from their keep copies; another md5 or no copy is refused.
        textures = pinned_textures(entries)
        assert textures == [("pinned.tga", b"tga")], textures
        try:
            pinned_textures(entries, lambda _path: b"other")
            raise AssertionError("a changed texture passed")
        except GateGameError as error:
            assert CHANGE_HINT in str(error) and md5_of(b"other") in str(error), error
        try:
            pinned_textures([("InspectIt/Bokeh", os.path.join(scratch, "keep", "absent.tga"), md5_of(b"tga"))])
            raise AssertionError("a texture with no keep copy passed")
        except GateGameError as error:
            assert "not readable" in str(error), error

        # A game folder with a sentinel behind a folder link, and a set.
        game = os.path.join(scratch, "game")
        os.makedirs(os.path.join(game, DATA, "Textures", "sub"))
        for path, text in ((os.path.join(game, "Morrowind.ini"), "ini"), (os.path.join(game, DATA, "Morrowind.esm"), "esm"),
                           (os.path.join(game, DATA, "Textures", "sentinel.txt"), "keep me"),
                           (os.path.join(game, DATA, "Textures", "sub", "deep.dds"), "dds"),
                           (os.path.join(game, DATA, "Textures", "sub", "sentinel.txt"), "keep me")):
            with open(path, "w", encoding="utf-8") as file:
                file.write(text)
        the_set = os.path.join(scratch, "set")
        os.makedirs(os.path.join(the_set, DATA, *POST_FOLDER, "Sub"))
        for name in ("Bloom Fine.wgsl", "Bloom Fine.fx", os.path.join("Sub", "Deep.fx"), "notes.txt"):
            open(os.path.join(the_set, DATA, *POST_FOLDER, name), "w").close()
        assert post_stems(the_set) == {"bloom fine", "sub/deep"}, post_stems(the_set)

        folder = os.path.join(scratch, NAME_START + "test")
        make(folder, game, files, textures)
        made_textures = os.path.join(folder, DATA, TEXTURES)
        assert is_link(os.path.join(folder, "Morrowind.ini")) and is_link(os.path.join(folder, DATA, "Morrowind.esm")), os.listdir(folder)
        assert not is_link(os.path.join(folder, DATA)) and os.path.isdir(os.path.join(folder, DATA))
        assert not is_link(made_textures) and os.path.isdir(made_textures), "Textures is a real folder"
        assert is_link(os.path.join(made_textures, "sub")) and is_link(os.path.join(made_textures, "sentinel.txt")), os.listdir(made_textures)
        assert not is_link(os.path.join(made_textures, "pinned.tga")), "the pinned texture is a real file"
        with open(os.path.join(made_textures, "pinned.tga"), "rb") as file:
            assert file.read() == b"tga"
        with open(os.path.join(made_textures, "sentinel.txt"), encoding="utf-8") as file:
            assert file.read() == "keep me", "the link reaches the game's file"
        with open(os.path.join(folder, DATA, *POST_FOLDER, "InspectIt", "Bokeh.wgsl"), "rb") as file:
            assert file.read() == bokeh
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

        # A pinned texture that the folder does not hold is a fault, not a silent black: the check names the file
        # and the post file that reads it.
        os.remove(os.path.join(made_textures, "pinned.tga"))
        found = check(folder, game, entries, [the_set])
        assert any("pinned.tga" in line and "is missing" in line for line in found), found
        assert any("Bokeh.wgsl names the texture pinned.tga" in line and "black" in line for line in found), found

        # The null of the remover: a sentinel behind the links survives (also under a linked folder of Textures),
        # and the folder is gone.
        remove(folder)
        assert not os.path.lexists(folder)
        for path in (os.path.join(game, DATA, "Textures", "sentinel.txt"), os.path.join(game, DATA, "Textures", "sub", "sentinel.txt"),
                     os.path.join(game, DATA, "Textures", "sub", "deep.dds"), os.path.join(game, DATA, "Morrowind.esm"),
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
        make(folder, game, files, textures)
        make(folder, game, files, textures)
        assert check(folder, game, entries, [the_set]) == []
        remove(folder)
        for wrong in (os.path.join(scratch, "game"), os.path.join(scratch, "results")):
            try:
                remove(wrong)
                raise AssertionError("remove took %s" % wrong)
            except GateGameError:
                pass
        assert os.path.isfile(os.path.join(game, "Morrowind.ini"))

        # The maker refuses, and makes nothing: a texture that a post file names and that is neither pinned nor a
        # texture of the game (the null: the same files with the texture pinned passed above), and a pinned texture
        # with the name of a texture of the game, in any case.
        for refused_files, refused_textures, words in (
                (files, [], "names the texture pinned.tga"),
                (files + [("absent", b"//! texture: absent.tga\n")], textures, "names the texture absent.tga"),
                (files, textures + [("SENTINEL.TXT", b"x")], "would hide the game's file")):
            try:
                make(folder, game, refused_files, refused_textures)
                raise AssertionError("the maker passed: %s" % words)
            except GateGameError as error:
                assert words in str(error), error
            assert not os.path.lexists(folder), "the maker left a part"

        # A game folder with no Textures folder: the folder's Textures is real and holds the pinned textures.
        bare = os.path.join(scratch, "bare")
        os.makedirs(os.path.join(bare, DATA))
        open(os.path.join(bare, DATA, "Morrowind.esm"), "w").close()
        make(folder, bare, [("ashfall", b"//! texture: pinned.tga\n")], textures)
        assert sorted(os.listdir(os.path.join(folder, DATA, TEXTURES))) == ["pinned.tga"]
        remove(folder)

        # A failed symbolic link stops the maker, which copies nothing and leaves no part.
        real_symlink = os.symlink

        def refused(*_args, **_kw):
            raise OSError(1314, "A required privilege is not held by the client")
        os.symlink = refused
        try:
            make(folder, game, files, textures)
            raise AssertionError("the maker passed without links")
        except GateGameError as error:
            assert "copies nothing" in str(error), error
        finally:
            os.symlink = real_symlink
        assert not os.path.lexists(folder), "the maker left a part"

        # A game folder with its own shaders folder is refused.
        os.makedirs(os.path.join(game, DATA, "shaders"))
        try:
            make(folder, game, files, textures)
            raise AssertionError("a game folder with shaders passed")
        except GateGameError as error:
            assert "shaders folder" in str(error), error
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
            make(args.folder, game, pinned_files(entries, args.render_link, commit), pinned_textures(entries))
        problems = check(args.folder, game, entries, args.set)
        if problems:
            for line in problems:
                print("  FAIL " + line, flush=True)
            return 1
        print("gate game folder: %s, links to %s; of %s, the pinned post files: %s; the pinned textures: %s" % (
            args.folder, game, os.path.basename(args.list), ", ".join(name for name, _, _ in posts_of(entries)),
            ", ".join(texture_name(entry) for entry in textures_of(entries)) or "none"), flush=True)
        return 0
    except (GateGameError, ValueError) as error:
        print("  FAIL %s" % error, flush=True)
        return 1


if __name__ == "__main__":
    sys.exit(main())
