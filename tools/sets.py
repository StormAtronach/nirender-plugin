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

A set has five parts, and each part comes from its own commits (its key). A key is the part's own repo's commit and, for
each thing of another repo that the part's build reads, the smallest name that holds it: the git tree of a folder, or
the commit for a whole crate. Each path of a set is in one part.

- plugin: the plugin DLL and its copy in the root; key: the nirender commit (its build reads nothing outside nirender).
- plugin-lua: the nirender Lua of Data Files/MWSE/mods/nirender; key: the nirender commit and the git tree of
  render-link's xtask/, which picks the files that the dist copies (collect_plugin). It has no build.
- render-link: d3d8.dll, dinput8.dll, MGEXEgui.exe, the readmes, mge3 and the assets; key: the render-link commit and
  the git tree of nirender's contract/ (d3d8.dll compiles the contract header of the nirender tree beside it).
- numidium: numidium.exe; key: the render-link and the nirender commits (it holds the host crate).
- mwse: MWSE.dll and the core Lua; key: the MWSE commit.

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

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from new_run_folder import SETS  # noqa: E402,F401  (the root of the sets: the root of the f<N> folders)

MANIFEST = "MANIFEST.json"
# The form of the manifest. A reader refuses another form (D126: no code for an old form).
FORM = 1
REPOS = ("nirender", "render-link", "mwse")
KIND_USUAL = "usual"
PARTS = ("plugin", "plugin-lua", "render-link", "numidium", "mwse")
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


class SetError(Exception):
    """A folder is not a sealed set; the message names the fault."""


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
    if PLUGIN_COPY_RE.fullmatch(low) or low.startswith(PLUGIN_PATHS):
        return "plugin"
    if low.startswith(PLUGIN_LUA_PATHS):
        return "plugin-lua"
    if low.startswith(NUMIDIUM_PATHS):
        return "numidium"
    if low.startswith(MWSE_PATHS):
        return "mwse"
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
    copies = [path for path in wanted_files if PLUGIN_COPY_RE.fullmatch(path)]
    staged = staged_plugin(manifest)
    if len(copies) != 1:
        faults.append("the manifest has %d plugin copies akulakhan-<commit>.dll in the root, not 1" % len(copies))
    elif staged is None or wanted_files[copies[0]].get("md5") != wanted_files[staged].get("md5"):
        faults.append("the plugin copy %s and %s have two md5s" % (copies[0], PLUGIN_STAGED))
    if faults:
        raise SetError("%s is not a sealed set: %s" % (folder, "; ".join(faults)))
    return manifest


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


def self_test():
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
