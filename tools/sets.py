"""The sealed build sets (W521, the owner's D490): the name of a set, its parts, its manifest and its check.

    python -I tools/sets.py <set folder>    # the check of one set: exit code 0 and its name, or 1 with the faults
    python -I tools/sets.py --self-test     # scratch folders only

A set is a folder E:/nirender-keep/mgebuilds/<name> that build_set.py makes, and nothing writes into it after that. It
holds every binary that a run needs: the stage of `cargo xtask dist`, the plugin also as akulakhan-<commit>.dll in its
root (the home that W520 gave it), MWSE.dll in its root with the MWSE core Lua of its commit below Data Files/MWSE/core,
the patch of a repo when the set has one (patches/<repo>.patch), and MANIFEST.json.

The name is s-<nirender>-<render-link>-<mwse>, each the first 8 hex of the commit. A repo built from a commit and one
patch has "+p<the first 8 hex of the patch's md5>" after its commit. A build of another kind than the usual one has
"-<kind>" at the end. So two kinds or a patch never share a name with the plain build of the same commits.

A set has seven parts, and each part comes from its own commits (its key). A key is the part's own repo's commit and, for
each thing of another repo that the part's build reads, the smallest name that holds it: the git tree of a folder, or
the commit for a whole crate. Each path of a set is in one part.

- plugin: the plugin DLL and its copy in the root; key: the nirender commit (its build reads nothing outside nirender).
- plugin-lua: the nirender Lua of Data Files/MWSE/mods/nirender; key: the nirender commit and the git tree of
  render-link's xtask/, which picks the files that the dist copies (collect_plugin). It has no build.
- render-link: d3d8.dll, dinput8.dll, MGEXEgui.exe, the readmes, mge3 and the assets; key: the render-link commit and
  the git tree of nirender's contract/ (d3d8.dll compiles the contract header of the nirender tree beside it).
- numidium: numidium.exe; key: the render-link and the nirender commits (it holds the host crate).
- mwse: MWSE.dll and the core Lua; key: the MWSE commit.
- luajit: lua51.dll in the root, beside MWSE.dll; key: the commit of deps/rubic0n that the MWSE commit names, and the
  git blob of SharedSE/LuaJIT.targets (the build parameter) at the MWSE commit (the design's 11.2).
- plugin-test: the files of the nirender tree's mod/ folder that the release does not ship (the test Lua, meshes and
  textures that a run stages), below plugin-test/ as below mod/; key: plugin-lua's, since xtask's list of shipped Lua
  decides the split (the plan's addendum 3). No deploy places it.

`check` is the proof that a folder is a sealed set: its manifest reads, each folder and each file of the manifest is
there, each file with its size and md5, no other file or folder is there, nothing in the folder is a link or a junction,
the manifest's name is the folder's name, and the two copies of the plugin have one md5.
"""
import hashlib
import json
import os
import re
import stat
import sys
import tempfile
import time
import types

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import docs_place  # noqa: E402

# The root of the sets (and of the older f<N> folders).
SETS = "E:/nirender-keep/mgebuilds"
HARNESS = docs_place("docs", "scripts", "test-harness-2026-10-03")
MANIFEST = "MANIFEST.json"
# The form of the manifest. A reader refuses another form (D126: no code for an old form).
FORM = 1
REPOS = ("nirender", "render-link", "mwse")
KIND_USUAL = "usual"
PARTS = ("plugin", "plugin-lua", "render-link", "numidium", "mwse", "luajit", "plugin-test")
# The folder of the part plugin-test in a set: the files of the nirender tree's mod/ folder, below it as below mod/.
PLUGIN_TEST = "plugin-test"
PATCHES = "patches"
# The name of a set: three commits of 8 hex, each with an optional patch mark, and an optional kind.
NAME_RE = re.compile(r"s-([0-9a-f]{8})(\+p[0-9a-f]{8})?-([0-9a-f]{8})(\+p[0-9a-f]{8})?-([0-9a-f]{8})(\+p[0-9a-f]{8})?(?:-([a-z][a-z0-9]*))?")
KIND_RE = re.compile(r"[a-z][a-z0-9]*")
PLUGIN_COPY_RE = re.compile(r"akulakhan-([0-9a-f]{7,40})\.dll", re.IGNORECASE)
PLUGIN_STAGED = "Data Files/mwse/lib/akulakhan.dll"
# The paths of the parts below the set, in lower case: a path that starts with one of them is of that part.
PLUGIN_PATHS = ("data files/mwse/lib/akulakhan.dll",)
PLUGIN_LUA_PATHS = ("data files/mwse/mods/nirender/",)
NUMIDIUM_PATHS = ("numidium.exe",)
MWSE_PATHS = ("mwse.dll", "data files/mwse/core/")
LUAJIT_PATHS = ("lua51.dll",)


class SetError(Exception):
    """A folder is not a sealed set; the message names the fault."""


def slot_tool():
    sys.path.insert(0, HARNESS)
    import slot
    return slot


def e_gate(slot=None, now=None):
    """(True, line) when no timing run holds the game (`slot.py e-ok`, D297), else (False, line). `slot` replaces the
    slot tool in the tests."""
    try:
        slot = slot or slot_tool()
        # The order matters (the plan's addendum 2): e_ok counts a timing holder's game past its lease only through
        # game.seen, which in_force writes (timing_rule_stands) and e_ok does not. So the game holder is read here
        # first, and a timing holder in force refuses before e_ok is asked. The full status read is not used: it may
        # walk the E: disk.
        holder = slot.read_record(slot.holder_path("game"), slot.Holder)
        if holder is not None and holder.timing and slot.in_force(holder, time.time() if now is None else now):
            return False, "e-ok: no: a timing run holds the game: %s" % getattr(holder, "who", "?")
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


def md5_of(path):
    digest = hashlib.md5()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def patch_mark(patch_md5):
    return "+p" + patch_md5[:8] if patch_md5 else ""


def set_name(commits, patch_md5s=None, kind=KIND_USUAL):
    """The name of a set of `commits` ({repo: full commit}), with the md5 of each repo's patch ({repo: md5}) and the
    kind."""
    patch_md5s = patch_md5s or {}
    if kind != KIND_USUAL and not KIND_RE.fullmatch(kind):
        raise SetError("the kind `%s` is not a word of small letters and digits" % kind)
    parts = ["%s%s" % (commits[repo][:8].lower(), patch_mark(patch_md5s.get(repo))) for repo in REPOS]
    return "s-" + "-".join(parts) + ("" if kind == KIND_USUAL else "-" + kind)


def part_of(path):
    """The part of a set's file `path` (with "/"), or None for the manifest and the patches."""
    low = path.lower()
    if low == MANIFEST.lower() or low.startswith(PATCHES + "/"):
        return None
    if low.startswith(PLUGIN_TEST + "/"):
        return "plugin-test"
    if PLUGIN_COPY_RE.fullmatch(low) or low.startswith(PLUGIN_PATHS):
        return "plugin"
    if low.startswith(PLUGIN_LUA_PATHS):
        return "plugin-lua"
    if low.startswith(NUMIDIUM_PATHS):
        return "numidium"
    if low.startswith(MWSE_PATHS):
        return "mwse"
    if low in LUAJIT_PATHS:
        return "luajit"
    return "render-link"


def is_link(path):
    """True for a symbolic link or a junction (any reparse point), never followed."""
    try:
        info = os.lstat(path)
    except OSError:
        return False
    attributes = getattr(info, "st_file_attributes", 0)
    return stat.S_ISLNK(info.st_mode) or bool(attributes & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0))


def walk(folder):
    """The folders and the files below `folder`, as sorted paths with "/", and the links and junctions found (none of
    them is entered)."""
    folders, files, links = [], [], []
    pending = [""]
    while pending:
        below = pending.pop()
        with os.scandir(os.path.join(folder, below) if below else folder) as entries:
            for entry in entries:
                path = below + "/" + entry.name if below else entry.name
                if is_link(entry.path):
                    links.append(path)
                elif entry.is_dir(follow_symlinks=False):
                    folders.append(path)
                    pending.append(path)
                else:
                    files.append(path)
    return sorted(folders), sorted(files), sorted(links)


def manifest_md5(folder):
    return md5_of(os.path.join(folder, MANIFEST))


def read_manifest(folder):
    path = os.path.join(folder, MANIFEST)
    try:
        with open(path, encoding="utf-8") as file:
            manifest = json.load(file)
    except FileNotFoundError:
        raise SetError("%s has no %s: it is not a sealed set (build_set.py makes one)" % (folder, MANIFEST)) from None
    except (OSError, ValueError) as error:
        raise SetError("the %s of %s does not read: %s" % (MANIFEST, folder, error)) from None
    if not isinstance(manifest, dict) or manifest.get("form") != FORM:
        raise SetError("the %s of %s is not of form %d" % (MANIFEST, folder, FORM))
    return manifest


def check(folder):
    """The manifest of the sealed set `folder`. Raises SetError with every fault found: a missing, changed or added file
    or folder, a link or a junction, a wrong name, two plugin copies that differ."""
    if is_link(folder) or not os.path.isdir(folder):
        raise SetError("%s is not a real folder" % folder)
    manifest = read_manifest(folder)
    faults = []
    name = os.path.basename(os.path.normpath(folder))
    if manifest.get("name") != name:
        faults.append("the manifest names the set %s, and the folder is %s" % (manifest.get("name"), name))
    folders, files, links = walk(folder)
    faults += ["%s is a link or a junction" % path for path in links]
    wanted_folders = set(manifest.get("folders", []))
    wanted_files = manifest.get("files", {})
    faults += ["the folder %s is missing" % path for path in sorted(wanted_folders - set(folders))]
    faults += ["the folder %s is not in the manifest" % path for path in sorted(set(folders) - wanted_folders)]
    present = set(files) - {MANIFEST}
    faults += ["the file %s is missing" % path for path in sorted(set(wanted_files) - present)]
    faults += ["the file %s is not in the manifest" % path for path in sorted(present - set(wanted_files))]
    for path in sorted(present & set(wanted_files)):
        full = os.path.join(folder, *path.split("/"))
        expected = wanted_files[path]
        if os.path.getsize(full) != expected.get("size"):
            faults.append("the file %s has %d bytes, and the manifest says %s" % (path, os.path.getsize(full), expected.get("size")))
        elif md5_of(full) != expected.get("md5"):
            faults.append("the file %s has another md5 than the manifest's" % path)
    # No pdb is deployed (the build skill's rule 2): the pdbs of a set live in E:/nirender-keep/pdb/<name>, outside it.
    faults += ["the set holds the pdb %s" % path for path in sorted(set(files) | set(wanted_files)) if path.lower().endswith(".pdb")]
    copies = [path for path in wanted_files if PLUGIN_COPY_RE.fullmatch(path)]
    staged = staged_plugin(manifest)
    if len(copies) != 1:
        faults.append("the manifest has %d plugin copies akulakhan-<commit>.dll in the root, not 1" % len(copies))
    elif staged is None or wanted_files[copies[0]].get("md5") != wanted_files[staged].get("md5"):
        faults.append("the plugin copy %s and %s have two md5s" % (copies[0], PLUGIN_STAGED))
    if faults:
        raise SetError("%s is not a sealed set: %s" % (folder, "; ".join(faults)))
    return manifest


# --- the one map of placing (the plan's addendum 1 A4) ----------------------------------------------
#
# Where each file of a set goes, for a run's Harness mods and for the deploy's install mods alike: (kind, path in the
# mod). The kinds: "mge" (the MGE XE mod), "nirender" (the plugin and its Lua), "mwse" (MWSE.dll, lua51.dll and the MWSE
# core), and "test" (the part plugin-test, which only a run places). A file that the map cannot place is a fault, so a
# set with a new kind of file stops both a run and a deploy (W524 F-T-4), and the two never place different files
# (F-T-8).
DATA_FILES = "data files/"
MGE_ROOT_FILES = ("d3d8.dll", "dinput8.dll", "numidium.exe", "mgexegui.exe")
MGE_DATA_FOLDERS = ("meshes/", "textures/", "shaders/", "mwse/mods/mge xe options/")
MWSE_ROOT_FILES = ("mwse.dll", "lua51.dll")
KINDS = ("mge", "nirender", "mwse", "test")
# The kinds that a deploy places: no test files go into an install.
DEPLOY_KINDS = ("mge", "nirender", "mwse")


def never_placed(path):
    """Why the set's file `path` is placed by no run and no deploy, or None. The set's own files (the manifest, the
    patches, the plugin copy in the root), and what no deploy writes: a pdb, an esp or esm (it changes the load order),
    the user's mgeXE.toml, a d3d9.dll."""
    low = path.lower()
    name = low.rsplit("/", 1)[-1]
    if low == MANIFEST.lower() or low.startswith(PATCHES + "/") or PLUGIN_COPY_RE.fullmatch(low):
        return "the set's own file"
    if name.endswith((".pdb", ".esp", ".esm")) or name in ("mgexe.toml", "d3d9.dll"):
        return "no deploy writes it"
    return None


def place_of(path):
    """(kind, path in the mod) of the set's file `path`, or None when the map has no place for it. The case of the path
    below a known folder is kept."""
    low = path.lower()
    folder, _, name = low.rpartition("/")
    if low.startswith(PLUGIN_TEST + "/"):
        return "test", path[len(PLUGIN_TEST) + 1:]
    if folder == "":
        if name in MGE_ROOT_FILES or name.endswith(".md"):
            return "mge", "Root/" + path
        if name in MWSE_ROOT_FILES:
            return "mwse", "Root/" + path
        return None
    if folder == "mge3" and name.endswith(".toml"):
        return "mge", "Root/" + path
    if not low.startswith(DATA_FILES):
        return None
    below, low_below = path[len(DATA_FILES):], low[len(DATA_FILES):]
    if low_below == "mwse/lib/akulakhan.dll":
        return "nirender", "MWSE/lib/akulakhan.dll"
    if low_below.startswith("mwse/mods/nirender/") and "/" not in low_below[len("mwse/mods/nirender/"):]:
        return "nirender", "MWSE/mods/nirender/" + below[len("mwse/mods/nirender/"):]
    if low_below.startswith("mwse/core/"):
        return "mwse", "MWSE/core/" + below[len("mwse/core/"):]
    if low_below.startswith(MGE_DATA_FOLDERS):
        return "mge", below
    return None


def placing(manifest, kinds=KINDS):
    """The one map of a set: ({set path: (kind, path in the mod)} for the kinds `kinds`, {set path: why it is never
    placed}, [the set paths that the map cannot place]). A run takes every kind (KINDS, less "nirender" and "test" when
    it loads no plugin); a deploy takes DEPLOY_KINDS."""
    placed, never, unplaced = {}, {}, []
    for path in sorted(manifest.get("files", {})):
        why = never_placed(path)
        if why:
            never[path] = why
            continue
        place = place_of(path)
        if place is None:
            unplaced.append(path)
        elif place[0] in kinds:
            placed[path] = place
    return placed, never, unplaced


# The Harness mod of a run that takes each kind, by the name after the prefix "Harness - " (harness.RUN_MOD_PREFIX).
RUN_MOD = {"mge": "MGE XE", "mwse": "MGE XE", "nirender": "nirender", "test": "nirender"}
PLUGIN_IN_MOD = "MWSE/lib/akulakhan.dll"


def stage_files(manifest, plugin):
    """The staging of a run from the set's manifest: {(Harness mod, path in the mod): set path}. A run takes every
    kind. A run that loads no plugin (not `plugin`) leaves out the plugin's DLL only: its scenario Lua
    (noplugin.lua, test.lua) is in the nirender mod too."""
    placed, _, _ = placing(manifest)
    return {(RUN_MOD[kind], dest): path for path, (kind, dest) in placed.items()
            if plugin or (kind, dest) != ("nirender", PLUGIN_IN_MOD)}


def placing_faults(manifest):
    """The faults of the map for this set: a file with no place, and two files with one place in a mod (the case of a
    path does not count on Windows)."""
    placed, _, unplaced = placing(manifest)
    faults = ["the file %s has no place in a run or a deploy (sets.place_of)" % path for path in unplaced]
    targets = {}
    for path, (kind, dest) in placed.items():
        target = (RUN_MOD[kind], dest.lower())
        if target in targets:
            faults.append("the files %s and %s have one place, %s in Harness - %s" % (targets[target], path, dest, target[0]))
        else:
            targets[target] = path
    return faults


def verify(folder):
    """The manifest of the sealed set `folder`, after check(folder) and the faults of the map. Raises SetError with every
    fault found. A run and a deploy read a set only through this."""
    manifest = check(folder)
    faults = placing_faults(manifest)
    if faults:
        raise SetError("%s cannot be placed: %s" % (folder, "; ".join(faults)))
    return manifest


# The binaries of a set whose pdbs the symbol store takes, as paths in the set; the pdbs are in E:/nirender-keep/pdb/<the
# set's name> (a set holds none). The runner and the deploy add them under the set's name.
SYMBOL_FILES = ("d3d8.dll", "dinput8.dll", "numidium.exe", "MWSE.dll", "lua51.dll", PLUGIN_STAGED)


def plugin_copy(manifest):
    """The path of the plugin copy in the root of a set, from its manifest."""
    return next(path for path in manifest["files"] if PLUGIN_COPY_RE.fullmatch(path))


def staged_plugin(manifest):
    """The path of the plugin in Data Files of a set, from its manifest, in the case that the folder has (Windows keeps
    the case of the folder that came first), or None."""
    return next((path for path in manifest.get("files", {}) if path.lower() == PLUGIN_STAGED.lower()), None)


def sets_in(root):
    """The names of the folders in `root` with the name of a set, sorted."""
    try:
        names = os.listdir(root)
    except OSError:
        return []
    return sorted(name for name in names if NAME_RE.fullmatch(name))


def find_part(root, part, key, kind, skip=()):
    """A sealed set in `root` whose manifest has `part` with this key and this kind, and whose check passes, as (name,
    manifest), or None. The sets of `skip` and the sets that fail their check are passed over."""
    for name in sets_in(root):
        if name in skip:
            continue
        folder = os.path.join(root, name)
        try:
            manifest = read_manifest(folder)
        except SetError:
            continue
        if manifest.get("kind") != kind or manifest.get("parts", {}).get(part, {}).get("key") != key:
            continue
        try:
            return name, check(folder)
        except SetError:
            continue
    return None


# --- the tests ------------------------------------------------------------------------------------

def write(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as file:
        file.write(data)


def scratch_manifest(folder, name):
    """A manifest of what `folder` holds now, of the form that build_set.py writes, for the tests."""
    folders, files, _ = walk(folder)
    manifest = {"form": FORM, "name": name, "kind": KIND_USUAL, "parts": {},
                "folders": folders,
                "files": {path: {"size": os.path.getsize(os.path.join(folder, *path.split("/"))),
                                 "md5": md5_of(os.path.join(folder, *path.split("/")))} for path in files if path != MANIFEST}}
    with open(os.path.join(folder, MANIFEST), "w", encoding="utf-8") as file:
        json.dump(manifest, file, indent=1, sort_keys=True)
    return manifest


def faults_of(folder):
    try:
        check(folder)
    except SetError as error:
        return str(error)
    return ""


def fake_slot(holder, in_force_answer, e_ok_code="done"):
    """A slot tool for the test of e_gate: it records its calls in `calls`."""
    calls = []
    slot = types.SimpleNamespace(DONE="done", Holder=object, calls=calls)
    slot.holder_path = lambda name: calls.append("holder_path " + name) or "path-" + name
    slot.read_record = lambda path, kind: calls.append("read_record " + path) or holder
    slot.in_force = lambda record, now: calls.append("in_force") or in_force_answer
    slot.e_ok = lambda: calls.append("e_ok") or types.SimpleNamespace(code=e_ok_code, text="e-ok: %s" % e_ok_code)
    return slot


def self_test_e_gate():
    timing = types.SimpleNamespace(timing=True, who="Hob: a timing run")
    plain = types.SimpleNamespace(timing=False, who="Rob: a live check")
    # A timing holder in force: no, and e_ok is not asked; the game holder is read first (addendum 2).
    slot = fake_slot(timing, True)
    assert e_gate(slot, 1.0) == (False, "e-ok: no: a timing run holds the game: Hob: a timing run"), slot.calls
    assert slot.calls == ["holder_path game", "read_record path-game", "in_force"], slot.calls
    # A timing holder whose rule ended: e_ok decides, after the read.
    slot = fake_slot(timing, False)
    assert e_gate(slot, 1.0) == (True, "e-ok: done") and slot.calls[-2:] == ["in_force", "e_ok"], slot.calls
    # A holder that is not timing: in_force is not needed, e_ok decides.
    slot = fake_slot(plain, True, e_ok_code="refused")
    assert e_gate(slot, 1.0) == (False, "e-ok: refused") and slot.calls[-1] == "e_ok" and "in_force" not in slot.calls
    # The null: no holder, e_ok's yes.
    slot = fake_slot(None, False)
    assert e_gate(slot, 1.0) == (True, "e-ok: done") and slot.calls[-1] == "e_ok"
    # A slot tool that fails: no.
    broken = fake_slot(None, False)
    broken.read_record = lambda path, kind: (_ for _ in ()).throw(OSError("state busy"))
    assert e_gate(broken, 1.0)[0] is False


def self_test_placing():
    # The files of a set of each group that the real set s-a7b9d1c6-8fd46faf-89198b53 holds, and plugin-test.
    paths = ["MANIFEST.json", "akulakhan-a7b9d1c.dll", "d3d8.dll", "dinput8.dll", "numidium.exe", "MGEXEgui.exe",
             "README.md", "mge3/main.toml", "MWSE.dll", "lua51.dll", "patches/render-link.patch",
             "Data Files/XE Sky Variations.esp", "Data Files/meshes/f/sky.nif", "Data Files/mwse/core/initialize.lua",
             "Data Files/mwse/lib/akulakhan.dll", "Data Files/mwse/mods/nirender/main.lua",
             "Data Files/mwse/mods/MGE XE Options/main.lua", "Data Files/shaders/core/a.fx",
             "Data Files/shaders/XEshaders/b.fx", "Data Files/textures/MGE/c.dds",
             "plugin-test/MWSE/mods/nirender/test_views.lua", "plugin-test/meshes/@_nifx/a.nif"]
    manifest = {"files": {path: {} for path in paths}}
    placed, never, unplaced = placing(manifest)
    assert unplaced == [], unplaced
    assert set(never) == {"MANIFEST.json", "akulakhan-a7b9d1c.dll", "patches/render-link.patch",
                          "Data Files/XE Sky Variations.esp"}, never
    assert set(placed) | set(never) == set(paths)
    assert placed["d3d8.dll"] == ("mge", "Root/d3d8.dll") and placed["mge3/main.toml"] == ("mge", "Root/mge3/main.toml")
    assert placed["MWSE.dll"] == ("mwse", "Root/MWSE.dll") and placed["lua51.dll"] == ("mwse", "Root/lua51.dll")
    assert placed["Data Files/mwse/core/initialize.lua"] == ("mwse", "MWSE/core/initialize.lua")
    assert placed["Data Files/mwse/lib/akulakhan.dll"] == ("nirender", "MWSE/lib/akulakhan.dll")
    assert placed["Data Files/mwse/mods/nirender/main.lua"] == ("nirender", "MWSE/mods/nirender/main.lua")
    assert placed["Data Files/mwse/mods/MGE XE Options/main.lua"] == ("mge", "mwse/mods/MGE XE Options/main.lua")
    assert placed["plugin-test/meshes/@_nifx/a.nif"] == ("test", "meshes/@_nifx/a.nif")
    # The run's map less plugin-test is the deploy's map, path for path (addendum 1 A4).
    deploy, _, _ = placing(manifest, DEPLOY_KINDS)
    assert {path: place for path, place in placed.items() if place[0] != "test"} == deploy
    assert not any(path.lower().startswith(PLUGIN_TEST + "/") for path in deploy)
    # The staging of a run with the plugin and without it.
    with_plugin = stage_files(manifest, True)
    assert with_plugin[("MGE XE", "Root/MWSE.dll")] == "MWSE.dll"
    assert with_plugin[("nirender", "MWSE/mods/nirender/test_views.lua")] == "plugin-test/MWSE/mods/nirender/test_views.lua"
    without = stage_files(manifest, False)
    assert set(with_plugin) - set(without) == {("nirender", PLUGIN_IN_MOD)}, set(with_plugin) - set(without)
    assert ("nirender", "MWSE/mods/nirender/main.lua") in without and ("MGE XE", "Root/dinput8.dll") in without
    assert len(with_plugin) == len(placed) and placing_faults(manifest) == []
    # Faults: a file of a new kind, a Lua folder below the plugin's Lua, a plugin-test file in the plugin-lua's place.
    for odd in ("tool.exe", "Data Files/Icons/a.dds", "Data Files/mwse/mods/nirender/sub/a.lua", "other/a.txt"):
        assert placing_faults({"files": dict(manifest["files"], **{odd: {}})}) == [
            "the file %s has no place in a run or a deploy (sets.place_of)" % odd], odd
    twin = dict(manifest["files"], **{"plugin-test/MWSE/mods/nirender/Main.lua": {}})
    assert "have one place" in placing_faults({"files": twin})[0], placing_faults({"files": twin})
    # Never placed: a pdb, an esm, mgeXE.toml and d3d9.dll; a pdb in the plugin's folder too.
    for withheld in ("d3d8.pdb", "Data Files/a.esm", "mge3/mgeXE.toml", "d3d9.dll", "Data Files/mwse/lib/akulakhan.pdb"):
        assert never_placed(withheld) == "no deploy writes it", withheld


def self_test():
    self_test_e_gate()
    self_test_placing()
    commits = {"nirender": "6d1b214" + "0" * 33, "render-link": "d9741db" + "1" * 33, "mwse": "89198b53f0" + "2" * 30}
    assert set_name(commits) == "s-6d1b2140-d9741db1-89198b53"
    assert set_name(commits, {"render-link": "abcdef0123456789"}, "rig") == "s-6d1b2140-d9741db1+pabcdef01-89198b53-rig"
    assert NAME_RE.fullmatch(set_name(commits, {"render-link": "abcdef0123456789"}, "rig"))
    assert not NAME_RE.fullmatch("f319") and not NAME_RE.fullmatch("s-6d1b2140-d9741db1-89198b53.making")
    try:
        set_name(commits, kind="Rig!")
        raise AssertionError("a kind with other letters was taken")
    except SetError:
        pass
    assert part_of("akulakhan-45b645b.dll") == "plugin" and part_of("Data Files/mwse/mods/nirender/main.lua") == "plugin-lua"
    assert part_of("Data Files/MWSE/lib/akulakhan.dll") == "plugin" and part_of("numidium.exe") == "numidium"
    assert part_of("MWSE.dll") == "mwse" and part_of("Data Files/MWSE/core/initialize.lua") == "mwse"
    assert part_of("lua51.dll") == "luajit" and part_of("Data Files/lua51.dll") == "render-link"
    assert part_of("plugin-test/meshes/@_nifx/a.nif") == "plugin-test" and part_of("plugin-test/MWSE/mods/nirender/t.lua") == "plugin-test"
    assert part_of("d3d8.dll") == "render-link" and part_of("Data Files/mwse/mods/MGE XE Options/main.lua") == "render-link"
    assert part_of(MANIFEST) is None and part_of("patches/mwse.patch") is None

    with tempfile.TemporaryDirectory(prefix="sets-") as root:
        name = "s-6d1b2140-d9741db1-89198b53"
        folder = os.path.join(root, name)
        write(os.path.join(folder, "d3d8.dll"), b"d3d8")
        write(os.path.join(folder, "akulakhan-6d1b214.dll"), b"plugin")
        write(os.path.join(folder, *PLUGIN_STAGED.split("/")), b"plugin")
        write(os.path.join(folder, "Data Files", "shaders", "core", "a.fx"), b"fx")
        scratch_manifest(folder, name)
        # The null: the set as made passes, so each fault below is the fault named.
        assert faults_of(folder) == "", faults_of(folder)
        assert sets_in(root) == [name]
        d3d8 = os.path.join(folder, "d3d8.dll")
        write(d3d8, b"d3d9")
        assert "d3d8.dll has another md5" in faults_of(folder), faults_of(folder)
        write(d3d8, b"d3d8")
        write(os.path.join(folder, "extra.dll"), b"x")
        assert "the file extra.dll is not in the manifest" in faults_of(folder)
        os.remove(os.path.join(folder, "extra.dll"))
        os.mkdir(os.path.join(folder, "empty"))
        assert "the folder empty is not in the manifest" in faults_of(folder)
        os.rmdir(os.path.join(folder, "empty"))
        os.remove(d3d8)
        assert "the file d3d8.dll is missing" in faults_of(folder)
        write(d3d8, b"d3d8")
        write(d3d8, b"d3d8!")
        assert "d3d8.dll has 5 bytes" in faults_of(folder)
        write(d3d8, b"d3d8")
        assert faults_of(folder) == ""
        # A pdb, also when the manifest names it: no pdb may be in a set.
        write(os.path.join(folder, "d3d8.pdb"), b"pdb")
        scratch_manifest(folder, name)
        assert "the set holds the pdb d3d8.pdb" in faults_of(folder), faults_of(folder)
        os.remove(os.path.join(folder, "d3d8.pdb"))
        scratch_manifest(folder, name)
        assert faults_of(folder) == ""
        # verify: the null passes; a sealed file with no place stops it, and check alone does not see it.
        assert verify(folder)["name"] == name
        write(os.path.join(folder, "tool.exe"), b"x")
        scratch_manifest(folder, name)
        assert faults_of(folder) == ""
        try:
            verify(folder)
            raise AssertionError("verify took a file with no place")
        except SetError as error:
            assert "the file tool.exe has no place" in str(error), error
        os.remove(os.path.join(folder, "tool.exe"))
        scratch_manifest(folder, name)
        # A junction inside: named, and not entered.
        target = os.path.join(root, "elsewhere")
        os.mkdir(target)
        junction = os.path.join(folder, "Data Files", "linked")
        made = os.system('mklink /J "%s" "%s" >nul 2>nul' % (junction, target)) == 0 if os.name == "nt" else False
        if made:
            assert "Data Files/linked is a link or a junction" in faults_of(folder), faults_of(folder)
            os.rmdir(junction)
        else:
            print("sets self-test: no junction could be made here; that case is not tested")
        # Two plugin copies that differ.
        write(os.path.join(folder, *PLUGIN_STAGED.split("/")), b"other")
        scratch_manifest(folder, name)
        assert "two md5s" in faults_of(folder)
        write(os.path.join(folder, *PLUGIN_STAGED.split("/")), b"plugin")
        scratch_manifest(folder, name)
        # Another folder name than the manifest's.
        renamed = os.path.join(root, "s-00000000-d9741db1-89198b53")
        os.rename(folder, renamed)
        assert "the manifest names the set" in faults_of(renamed)
        os.rename(renamed, folder)
        os.remove(os.path.join(folder, MANIFEST))
        assert "has no MANIFEST.json" in faults_of(folder)
    print("sets self-test: ok")
    return 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    if len(argv) != 1:
        print(__doc__.strip().splitlines()[0])
        return 2
    try:
        manifest = check(argv[0])
    except SetError as error:
        print(error)
        return 1
    print("%s: a sealed set, %d files, manifest md5 %s" % (manifest["name"], len(manifest["files"]), manifest_md5(argv[0])))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
