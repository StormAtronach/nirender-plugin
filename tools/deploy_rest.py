"""The resting build: puts a sealed set into both test installs (W521).

    python -I tools/deploy_rest.py --set <set folder> [--plugin-commit SHA] [--mge-commit SHA] [--tag NAME] [--dry-run]
                                   [--lua51-change MD5]
    python -I tools/deploy_rest.py --check [--set <set folder>]   # each install's deployed files against the set; writes nothing
    python -I tools/deploy_rest.py --check-mwse-core   # each install's MWSE core against the commit of its MWSE.dll; writes nothing
    python -I tools/deploy_rest.py --self-test    # the tests: scratch sets and installs, no game

The merge step runs this as the stage "deploy rest" (the owner's D193: the test installs always hold the latest build).
<set folder> is a sealed set of tools/build_set.py. The deploy first runs sets.verify on it: a missing, changed or added
file, or a file that the one map of sets.py cannot place, stops it before anything is written (W524 F-T-4). The map is
the one that the runner stages from (sets.placing): a deploy places the kinds mge, nirender and mwse, and never the
set's own files (the manifest, the patches, the plugin copy in the root), plugin-test/, a pdb, an esp or esm, mgeXE.toml
or d3d9.dll. Each install of REST_INSTALLS gets:

- into its MGE mod: the root files (d3d8.dll, dinput8.dll, numidium.exe, MGEXEgui.exe, the readmes, the classifiers of
  mge3/) under Root/, and the meshes, the textures, the shaders and the "MGE XE Options" mod of Data Files/;
- into its nirender mod: akulakhan.dll and the Lua files of the nirender mod;
- into its MWSE mod (the folder of MWSE_DEPLOYS two levels up: <mod>/Root/MWSE.dll): MWSE.dll and lua51.dll under Root/,
  and the MWSE core under MWSE/core (D 2.3). The record MWSE_NOTE (mwse-deployed.json in the install's folder) names the
  set's MWSE commit, the MWSE.dll and its MD5.

Every file of those folders is added or replaced; a file that the set lacks stays (the install may hold shaders of its
own, such as water/tw_lava.wgsl), and the output names such files as "only in the mod" (W273, W480: nothing is deleted).
A file that already equals its source is left as it is. A file it replaces is first copied to
<install>/mge-backup-<tag>/<mod>/<path>, once for each tag: a second run with the same tag keeps the first backup. The tag
is the set's name unless --tag names one. Each copy is compared with its source by MD5.

A set with a patch or a kind is not a resting build (the commits name its code only without them): it is refused.
--plugin-commit and --mge-commit (the merge step's) must be the set's nirender and render-link commits.

The change-over of lua51.dll (D 11.2): before a deploy, the MD5 of each install's lua51.dll is read against the set's.
A difference stops the deploy, the dry run too, with the three MD5s, for the Coordinator: nothing is written. On the
Coordinator's word, --lua51-change <the set's lua51.dll MD5> lets that one lua51.dll in; another MD5 is refused. After
the deploy the installs hold the set's lua51.dll, so the next set with the same one passes without the option.

It refuses, and writes nothing, while Morrowind, numidium or Mod Organizer runs, and it does not wait: exit code 3 and a
line that starts with "REFUSED". Any other fault is exit code 1. It checks too that each install has its three mod
folders (it makes none). The output prints the MD5 of d3d8.dll, numidium.exe, akulakhan.dll, MWSE.dll and lua51.dll.
--dry-run reads the installs and writes nothing.

After a deploy, as the last step before DONE, it writes SETS\\current.bat (W500, the owner's D461), which names the set:
the owner's local.bat calls it, and clean_common.current_set reads it first. A dry run leaves it as it is. A failed write
fails the run after the deploy, with the set's name. The set's pdbs (E:/nirender-keep/pdb/<name>) go into the symbol
store under the set's name.

--check reads each install's files of the map against the set's manifest (the current set of current.bat without
--set): exit code 1 and each file that differs or is missing. It writes nothing.

--check-mwse-core (D442, a stage of the merge step) writes nothing. For each install it reads the record, and fails
when there is none, when the MWSE.dll is not the one of the record (by MD5), or when a file of the core of the record's
commit (CORE_PATH of the MWSE repository at that commit) is missing or differs in the install's core folder. A file that
only the install has is no fault.
"""
import argparse
import datetime
import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile

# The modules of this folder, also under `python -I`.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import sets  # noqa: E402
import symbols  # noqa: E402
from common import DEPLOY_DONE, DEPLOYS_FILE, REPO, append_deploys, deploys_line, local, main_checkout  # noqa: E402
from make_play_set import PDB_KEEP, say  # noqa: E402
from sets import CURRENT_BAT, write_current_bat  # noqa: E402  (W500)

# The test installs, by name: the folder of the install (MO2 instance) and the names of its two mods. The key
# "rest_installs" of local.json changes the places.
REST_INSTALLS = local("rest_installs", {
    "Morrowind75testing": {"root": "D:/Modlists/Morrowind75testing", "mge_mod": "MGE XE G7 Fork", "nirender_mod": "nirender",
                           "profile": "Default", "config_mod": "Configuration"},
    "Outlander-test": {"root": "D:/Modlists/Outlander-test", "mge_mod": "MGE XE G7", "nirender_mod": "nirender",
                       "profile": "Outlander", "config_mod": "MGE XE G7"},
})
# The MWSE.dll that each test install has deployed (the line build), by install: the "mwse features" and "mwse sites"
# stages of merge_step.py check these, and the deploy of a set writes them. The MWSE mod of an install is the folder two
# levels up (<mod>/Root/MWSE.dll). Never a `Harness - *` mod: a live run puts its own MWSE.dll there for its game hold.
# The key "mwse_deploys" of local.json changes the places.
MWSE_DEPLOYS = local("mwse_deploys", {
    "Morrowind75testing": "D:/Modlists/Morrowind75testing/mods/MWSE/Root/MWSE.dll",
    "Outlander-test": "D:/Modlists/Outlander-test/mods/MWSE-Update for Nightly Builds/Root/MWSE.dll",
})
# The MWSE repository and the core Lua in it: --check-mwse-core reads the core of the record's commit there. The key
# "mwse_repo" of local.json changes the repository.
MWSE_REPO = local("mwse_repo", os.path.join(os.path.dirname(main_checkout() or REPO), "MWSE", "MWSE"))
CORE_PATH = "misc/package/Data Files/MWSE/core"
# The record of the MWSE commit of an install's MWSE.dll and core (D442), in the folder of the install: the deploy writes
# it, and --check-mwse-core (a stage of the merge step) and mwse_tree.installed_commit read it.
MWSE_NOTE = "mwse-deployed.json"
# The programs that stop a deploy, as a part of the name of the image (without the case).
BUSY_PROGRAMS = ("morrowind", "numidium", "modorganizer")
REFUSED_EXIT = 3
FAULT_EXIT = 1
LUA51 = "lua51.dll"
# The files the output names with their MD5, by their path in the set.
NAMED_DIGESTS = (("d3d8.dll", "d3d8.dll"), ("numidium.exe", "numidium.exe"), ("akulakhan.dll", sets.PLUGIN_STAGED),
                 ("MWSE.dll", "MWSE.dll"), (LUA51, LUA51))


class DeployError(Exception):
    """A fault of the deploy; the message is the line that the stage prints."""


class Refused(DeployError):
    """A game or Mod Organizer runs: the deploy writes nothing and does not wait."""


def md5_of(path):
    digest = hashlib.md5()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def files_below(folder):
    """The files below a folder, as paths with "/" relative to it, sorted."""
    found = []
    for base, _, names in os.walk(folder):
        for name in names:
            found.append(os.path.relpath(os.path.join(base, name), folder).replace("\\", "/"))
    return sorted(found)


def busy_programs(tasklist):
    """The programs of BUSY_PROGRAMS in the text of `tasklist /FO CSV /NH`, as image names."""
    found = []
    for line in tasklist.splitlines():
        image = line.strip().split('","')[0].lstrip('"')
        if any(part in image.lower() for part in BUSY_PROGRAMS):
            found.append(image)
    return sorted(set(found))


def tasklist_text():
    done = subprocess.run(["tasklist", "/FO", "CSV", "/NH"], capture_output=True, text=True, stdin=subprocess.DEVNULL)
    if done.returncode != 0:
        raise DeployError("tasklist failed with exit code %d: the deploy cannot see whether a game runs" % done.returncode)
    return done.stdout


def check_nothing_runs(tasklist=tasklist_text):
    busy = busy_programs(tasklist())
    if busy:
        raise Refused("REFUSED: %s runs, and the deploy does not wait: end it, or run the stage again later" % ", ".join(busy))


def with_mwse_mods(installs, deploys):
    """The installs with the folder of each one's MWSE mod ("mwse_mod_folder": MWSE_DEPLOYS two levels up). An install
    with no place in `deploys` is a fault."""
    found = {}
    for name, install in installs.items():
        if name not in deploys:
            raise DeployError("no place of the MWSE.dll of %s in mwse_deploys" % name)
        found[name] = dict(install, mwse_mod_folder=os.path.dirname(os.path.dirname(deploys[name])))
    return found


def mod_folder(install, kind):
    """The folder of one mod of an install, by the kind of the map: "mge", "nirender" or "mwse"."""
    if kind == "mwse":
        return install["mwse_mod_folder"]
    return os.path.join(install["root"], "mods", install["mge_mod"] if kind == "mge" else install["nirender_mod"])


def check_installs(installs):
    for name, install in installs.items():
        for kind in sets.DEPLOY_KINDS:
            if not os.path.isdir(mod_folder(install, kind)):
                raise DeployError("%s has no mod folder %s: the deploy makes none" % (name, mod_folder(install, kind)))


def replace_file(source, dest, backup, copy=shutil.copyfile, label=None):
    """Copies `source` to `dest`: 'unchanged' when they are equal, else 'copied' after the backup of the file it replaces
    to `backup` (only when there is no backup yet). Raises DeployError when the copy does not equal its source."""
    digest = md5_of(source)
    if os.path.isfile(dest):
        if md5_of(dest) == digest:
            return "unchanged"
        if not os.path.exists(backup):
            os.makedirs(os.path.dirname(backup), exist_ok=True)
            shutil.copy2(dest, backup)
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    copy(source, dest)
    if md5_of(dest) != digest:
        raise DeployError("MD5 MISMATCH after the copy of %s to %s" % (label or source, dest))
    return "copied"


def set_plan(folder, plugin_commit=None, mge_commit=None, verify=sets.verify):
    """The manifest of the set `folder` after sets.verify, and its plan: {set path: (kind, path in the mod)} of the
    deploy's kinds. Raises DeployError for a set that fails its check, a set with a patch or a kind, and a commit of
    --plugin-commit or --mge-commit that is not the set's."""
    try:
        manifest = verify(folder)
    except sets.SetError as error:
        raise DeployError(str(error)) from None
    name = manifest.get("name", "")
    match = sets.NAME_RE.fullmatch(name)
    if not match or any(match.group(index) for index in (2, 4, 6, 7)):
        raise DeployError("%s has a patch or a kind: a resting build is a set of commits, and only a usual set names its code" % name)
    commits = manifest.get("commits", {})
    for repo, given, option in (("nirender", plugin_commit, "--plugin-commit"), ("render-link", mge_commit, "--mge-commit")):
        have = commits.get(repo, {}).get("commit", "")
        if given and not have.lower().startswith(given.lower()):
            raise DeployError("%s %s is not the set's %s commit %s" % (option, given, repo, have or "unknown"))
    placed, _, _ = sets.placing(manifest, sets.DEPLOY_KINDS)
    return manifest, placed


def lua51_line(folder, manifest, installs, change=None):
    """The change-over of lua51.dll (D 11.2), as (line, fault). A set without lua51.dll changes nothing. An install's
    lua51.dll that is not the set's is a fault unless `change` is the MD5 of the set's lua51.dll."""
    entry = manifest.get("files", {}).get(LUA51)
    if entry is None:
        return "lua51.dll: the set has none; the installs keep theirs", False
    ours = entry["md5"]
    theirs = {}
    for name, install in installs.items():
        path = os.path.join(mod_folder(install, "mwse"), "Root", LUA51)
        theirs[name] = md5_of(path) if os.path.isfile(path) else "no file"
    sums = ", ".join("%s md5 %s" % item for item in sorted(theirs.items()))
    if all(digest == ours for digest in theirs.values()):
        return "lua51.dll: md5 %s in the set and in each install" % ours, False
    if change and change.lower() == ours:
        return "lua51.dll: the change-over of D 11.2 by --lua51-change: the set's md5 %s replaces %s" % (ours, sums), False
    if change:
        return "LUA51 CHANGE: --lua51-change %s is not the set's lua51.dll (md5 %s); %s" % (change, ours, sums), True
    return ("LUA51 CHANGE: the set's lua51.dll (md5 %s) is not each install's (%s): the change-over of D 11.2 goes to the "
            "Coordinator; nothing is written. On the Coordinator's word: --lua51-change %s" % (ours, sums, ours)), True


def distant_land_keys(folder, manifest, installs):
    """{install name: the key of the set's distant land with the install's own mgeXE.toml, or None for an install whose
    entry names no profile} (W521 step 4). The key can start the set's helper: run() asks it before the first copy, so
    a failing key leaves every install as it was (verdict 553 C3). A folder of the key's mod name with no marker of the
    tool, which distant_land.rest refuses, is refused here too, before any write (C3b). A distant_land.Problem is a
    DeployError."""
    import distant_land
    keys = {}
    for install_name, install in installs.items():
        if not install.get("profile"):
            keys[install_name] = None
            continue
        toml = os.path.join(install["root"], "mods", install.get("config_mod", "Configuration"), "Root", "mgeXE.toml")
        try:
            key = distant_land.key_of(folder, manifest, toml, [])
        except distant_land.Problem as problem:
            raise DeployError("%s: the key of the distant land: %s; nothing is deployed" % (install_name, problem))
        mod = os.path.join(distant_land.mods_folder(install["root"]), key.mod_name)
        if os.path.isdir(mod) and key.mod_name not in distant_land.marked_mods(install["root"]):
            raise DeployError("%s: %s is there, but it is not a mod of the distant land tool (no marker for this key and "
                              "instance); nothing is deployed" % (install_name, mod))
        keys[install_name] = key
    return keys


def distant_land_lines(keys, installs, apply):
    """The resting distant land of each install after a deploy (`distant_land.rest`), with the keys of
    distant_land_keys: the key's mod goes on, every other marked mod off. An install with no key is passed over with a
    line. A distant_land.Problem is a DeployError."""
    import distant_land
    lines = []
    for install_name, install in installs.items():
        key = keys[install_name]
        if key is None:
            lines.append("%s: no profile named; the distant land mod is not set" % install_name)
            continue
        try:
            lines += distant_land.rest(install["root"], os.path.join(install["root"], "profiles", install["profile"]), key,
                                       distant_land.tool_commit(), apply)
        except distant_land.Problem as problem:
            raise DeployError("%s: the distant land mod: %s" % (install_name, problem))
    return lines


def backup_of(install, kind, dest, tag):
    """The backup place of a file of the plan: <install>/mge-backup-<tag>/<mod folder name>/<path in the mod>."""
    return os.path.join(install["root"], "mge-backup-" + tag, os.path.basename(mod_folder(install, kind)), *dest.split("/"))


def deploy(folder, installs, tag, placed, copy=shutil.copyfile):
    """Puts the files of the plan into each install. Returns {install name: {kind: [copied, unchanged]}}."""
    counts = {}
    for name, install in installs.items():
        counts[name] = {kind: [0, 0] for kind in sets.DEPLOY_KINDS}
        for path, (kind, dest) in sorted(placed.items()):
            source = os.path.join(folder, *path.split("/"))
            target = os.path.join(mod_folder(install, kind), *dest.split("/"))
            done = replace_file(source, target, backup_of(install, kind, dest, tag), copy, path)
            counts[name][kind][0 if done == "copied" else 1] += 1
    return counts


def would_copy(folder, installs, placed):
    """The dry run: for each install, the paths in the mods that a deploy would copy."""
    changes = {}
    for name, install in installs.items():
        changes[name] = []
        for path, (kind, dest) in sorted(placed.items()):
            target = os.path.join(mod_folder(install, kind), *dest.split("/"))
            if not os.path.isfile(target) or md5_of(target) != md5_of(os.path.join(folder, *path.split("/"))):
                changes[name].append("%s %s" % (kind, dest))
    return changes


# The folders of each mod whose files the output names when the set lacks them: they stay (W273, W480).
REPORTED_FOLDERS = (("mge", ("meshes", "textures", "shaders", "mwse/mods/MGE XE Options")), ("nirender", ("MWSE/mods/nirender",)),
                    ("mwse", ("MWSE/core",)))


def only_in_mod(install, placed):
    """The files of the deployed folders (REPORTED_FOLDERS) that the plan does not name: the install keeps them."""
    found = []
    for kind, folders in REPORTED_FOLDERS:
        named = {dest.lower() for path, (place_kind, dest) in placed.items() if place_kind == kind}
        for below in folders:
            root = os.path.join(mod_folder(install, kind), *below.split("/"))
            if os.path.isdir(root):
                found += ["%s %s/%s" % (kind, below, path) for path in files_below(root) if ("%s/%s" % (below, path)).lower() not in named]
    return found


def only_in_mod_line(install_name, paths):
    """The output line of `only_in_mod`: the count and the first 10 paths."""
    shown = ", ".join(paths[:10]) + (", ..." if len(paths) > 10 else "")
    return "%s: files only in the deployed mods (kept, not deleted): %d%s" % (install_name, len(paths), ": " + shown if paths else "")


def core_folder(dll_dest):
    """The MWSE core folder of an install, beside its MWSE.dll of MWSE_DEPLOYS: <mod>/Root/MWSE.dll gives <mod>/MWSE/core."""
    return os.path.join(os.path.dirname(os.path.dirname(dll_dest)), "MWSE", "core")


def core_of(commit, into, repo=None):
    """The MWSE core Lua of `commit` (CORE_PATH of MWSE_REPO, by git archive: the line endings of a checkout), extracted
    below `into`. Returns the folder of the core. Raises DeployError when git cannot give it."""
    repo = repo or MWSE_REPO
    done = subprocess.run(["git", "-C", repo, "archive", "--format=tar", commit, CORE_PATH], capture_output=True)
    if done.returncode != 0:
        raise DeployError("git archive of %s at %s in %s failed: %s" % (CORE_PATH, commit, repo, done.stderr.decode(errors="replace").strip()))
    with tarfile.open(fileobj=io.BytesIO(done.stdout)) as tar:
        tar.extractall(into, filter="data")
    folder = os.path.join(into, *CORE_PATH.split("/"))
    if not files_below(folder):
        raise DeployError("the MWSE commit %s has no files in %s" % (commit, CORE_PATH))
    return folder


def core_changes(core, dest):
    """The files of `core` that differ from `dest` or are missing there, and the files that only `dest` has."""
    ours = files_below(core)
    differ = [path for path in ours if not os.path.isfile(os.path.join(dest, *path.split("/")))
              or md5_of(os.path.join(dest, *path.split("/"))) != md5_of(os.path.join(core, *path.split("/")))]
    only = [path for path in files_below(dest) if path not in set(ours)] if os.path.isdir(dest) else []
    return ours, differ, only


def write_mwse_note(install, commit, dll, core_files, tag, stamp=None):
    """Writes MWSE_NOTE into the folder of the install: the MWSE commit of its MWSE.dll and core, and the DLL's MD5 (D442)."""
    note = {"commit": commit, "dll": dll, "md5": md5_of(dll), "core": core_folder(dll), "core_files": core_files, "tag": tag,
            "time": stamp or datetime.datetime.now().strftime("%Y-%m-%d %H:%M")}
    path = os.path.join(install["root"], MWSE_NOTE)
    with open(path, "w", encoding="ascii") as file:
        json.dump(note, file, indent=1)
        file.write("\n")
    return path


def read_mwse_note(install):
    """MWSE_NOTE of an install as a dict with "commit" and "md5", or None when there is none or it cannot be read."""
    try:
        with open(os.path.join(install["root"], MWSE_NOTE), encoding="ascii") as file:
            note = json.load(file)
    except (OSError, ValueError):
        return None
    if not isinstance(note, dict) or not all(isinstance(note.get(key), str) for key in ("commit", "md5")):
        return None
    return note


def noted_commit(install, dll):
    """The MWSE commit of the install's MWSE.dll by its MWSE_NOTE, or None: no note, no DLL, or a DLL that the note does
    not describe (another DLL came in by hand)."""
    note = read_mwse_note(install)
    if note is None or not os.path.isfile(dll) or md5_of(dll) != note["md5"]:
        return None
    return note["commit"]


def mwse_core_lines(installs, deploys, core_source=core_of):
    """The check of the merge step (D442): each install's MWSE core must be the core of the MWSE commit of its MWSE.dll,
    by MWSE_NOTE. Returns [(line, good)], one for each install. A file that only the install has is no fault."""
    rows = []
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as scratch:
        cores = {}
        for name, install in installs.items():
            dll = deploys.get(name)
            if dll is None or not os.path.isfile(dll):
                rows.append(("%s: no MWSE.dll at its place of mwse_deploys (%s)" % (name, dll), False))
                continue
            note = read_mwse_note(install)
            commit = noted_commit(install, dll)
            if commit is None:
                why = "no record %s" % os.path.join(install["root"], MWSE_NOTE) if note is None else "the record names another MWSE.dll (md5 %s, commit %s)" % (note["md5"], note["commit"])
                rows.append(("%s: the MWSE commit of %s is not known: %s. Deploy a set with deploy_rest.py --set <set>" % (name, dll, why), False))
                continue
            try:
                if commit not in cores:
                    cores[commit] = core_source(commit, os.path.join(scratch, str(len(cores))))
            except DeployError as error:
                rows.append(("%s: %s" % (name, error), False))
                continue
            ours, differ, only = core_changes(cores[commit], core_folder(dll))
            if differ:
                rows.append(("%s: %d of %d files of the MWSE core %s are not those of MWSE %s, the commit of its MWSE.dll: %s" % (
                    name, len(differ), len(ours), core_folder(dll), commit, ", ".join(differ[:10]) + (", ..." if len(differ) > 10 else "")), False))
            else:
                rows.append(("%s: the MWSE core %s is the core of MWSE %s, the commit of its MWSE.dll (%d files; %d only in the install, kept)" % (
                    name, core_folder(dll), commit, len(ours), len(only)), True))
    return rows


def check_mwse_core(installs, deploys, core_source=core_of):
    """--check-mwse-core: the lines of `mwse_core_lines`, a fault with "FAIL: "; exit code 1 when any install fails."""
    rows = mwse_core_lines(installs, deploys, core_source)
    for line, good in rows:
        say(line if good else "FAIL: " + line)
    return 0 if all(good for _, good in rows) else FAULT_EXIT


def check_deployed(folder, installs, deploys=None, verify=sets.verify):
    """--check: each install's files of the map against the set's manifest. Returns the exit code: 1 and a line for each
    file that differs or is missing."""
    installs = with_mwse_mods(installs, MWSE_DEPLOYS if deploys is None else deploys)
    manifest, placed = set_plan(folder, verify=verify)
    faults = 0
    for name, install in installs.items():
        differ = []
        for path, (kind, dest) in sorted(placed.items()):
            target = os.path.join(mod_folder(install, kind), *dest.split("/"))
            if not os.path.isfile(target):
                differ.append("%s %s is missing" % (kind, dest))
            elif md5_of(target) != manifest["files"][path]["md5"]:
                differ.append("%s %s differs" % (kind, dest))
        faults += len(differ)
        say("%s: %d of %d files of %s as sealed%s" % (name, len(placed) - len(differ), len(placed), manifest["name"],
                                                     "" if not differ else "; FAIL: " + ", ".join(differ[:10]) + (", ..." if len(differ) > 10 else "")))
    return FAULT_EXIT if faults else 0


def run(folder, installs, plugin_commit=None, mge_commit=None, tag=None, dry_run=False, sets_root=sets.SETS, tasklist=tasklist_text,
        copy=shutil.copyfile, add_symbols=None, mwse_deploys=None, deploys_file=None, lua51_change=None, verify=sets.verify,
        pdb_keep=PDB_KEEP):
    """The whole deploy of the set `folder`. Returns the exit code. `add_symbols` (symbols.add from the command line)
    puts the set's pdbs (`pdb_keep`/<name>) into the symbol store. Each install gets the record MWSE_NOTE of the set's MWSE
    commit and the distant land mod of the set's key on in its profile (distant_land_lines). A deploy appends a DEPLOYED line to `deploys_file` (common.DEPLOYS_FILE from the command line; None writes
    none): it closes the WITHHELD deploys of a merge round of the same or older commits (W528)."""
    installs = with_mwse_mods(installs, MWSE_DEPLOYS if mwse_deploys is None else mwse_deploys)
    check_installs(installs)
    manifest, placed = set_plan(folder, plugin_commit, mge_commit, verify)
    name = manifest["name"]
    commits = {repo: manifest["commits"][repo]["commit"] for repo in sets.REPOS}
    say("set: %s, nirender %s, render-link %s, MWSE %s" % (name, commits["nirender"], commits["render-link"], commits["mwse"]))
    line, fault = lua51_line(folder, manifest, installs, lua51_change)
    say(line)
    if fault:
        raise DeployError(line)
    land_keys = distant_land_keys(folder, manifest, installs)
    digests = [(label, manifest["files"][path]["md5"]) for label, path in NAMED_DIGESTS if path in manifest["files"]]
    kinds = sorted({kind for kind, _ in placed.values()})
    say("plan: %d files for each of %d installs (%s)" % (len(placed), len(installs), ", ".join(
        "%s %d" % (kind, sum(1 for place_kind, _ in placed.values() if place_kind == kind)) for kind in kinds)))
    if dry_run:
        for install_name, changed in would_copy(folder, installs, placed).items():
            first = ": " + ", ".join(changed[:5]) + (", ..." if len(changed) > 5 else "") if changed else ""
            say("  %s: would copy %d, unchanged %d%s" % (install_name, len(changed), len(placed) - len(changed), first))
            say("  " + only_in_mod_line(install_name, only_in_mod(installs[install_name], placed)))
            say("  %s: would write the record %s: MWSE commit %s" % (install_name, os.path.join(installs[install_name]["root"], MWSE_NOTE), commits["mwse"]))
        for line in distant_land_lines(land_keys, installs, apply=False):
            say("  " + line)
        for label, digest in digests:
            say("md5 %s %s" % (label, digest))
        say("dry run: nothing is written")
        return 0
    check_nothing_runs(tasklist)
    tag = tag or name
    counts = deploy(folder, installs, tag, placed, copy)
    for install_name, by_kind in counts.items():
        say("%s: %s; backups in %s" % (install_name, ", ".join("%s copied %d, unchanged %d" % (kind, *by_kind[kind]) for kind in kinds),
                                       os.path.join(installs[install_name]["root"], "mge-backup-" + tag)))
        say(only_in_mod_line(install_name, only_in_mod(installs[install_name], placed)))
    deploys = MWSE_DEPLOYS if mwse_deploys is None else mwse_deploys
    core_files = sum(1 for kind, dest in placed.values() if kind == "mwse" and dest.lower().startswith("mwse/core/"))
    if "MWSE.dll" in manifest["files"]:
        for install_name, install in installs.items():
            note = write_mwse_note(install, commits["mwse"], deploys[install_name], core_files, tag)
            say("%s: the record %s: MWSE commit %s" % (install_name, note, commits["mwse"]))
    for line in distant_land_lines(land_keys, installs, apply=True):
        say(line)
    for label, digest in digests:
        say("md5 %s %s" % (label, digest))
    pdbs = [path for install in installs.values() for kind in sets.DEPLOY_KINDS for path in files_below(mod_folder(install, kind))
            if path.lower().endswith(".pdb") and kind != "mwse"]
    say("pdb in the deployed mods: %d" % len(pdbs))
    if add_symbols:
        say(add_symbols(name, [os.path.join(folder, *path.split("/")) for path in sets.SYMBOL_FILES], [os.path.join(pdb_keep, name)]))
    try:
        say("current build: %s names %s" % (write_current_bat(sets_root, name), name))
    except (OSError, UnicodeEncodeError) as error:
        raise DeployError("the set %s is deployed, but %s was not written: %s" % (name, os.path.join(sets_root, CURRENT_BAT), error))
    if deploys_file:
        when = datetime.datetime.now().astimezone().isoformat(timespec="seconds")
        try:
            append_deploys(deploys_file, deploys_line(when, DEPLOY_DONE, commits["nirender"], commits["render-link"], ", ".join(installs)))
            say("deploys: DEPLOYED plugin %s, mge %s in %s" % (commits["nirender"], commits["render-link"], deploys_file))
        except OSError as error:
            say("deploys: NOT WRITTEN to %s (%s); the deploy stands, write the DEPLOYED line by hand" % (deploys_file, error))
    say("DONE: %d installs from %s with tag %s" % (len(installs), folder, tag))
    return 0


# --- the tests ---------------------------------------------------------------------------

def write(path, text="x"):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="ascii") as file:
        file.write(text)


FAKE_COMMITS = {"nirender": "79c752d" + "1" * 33, "render-link": "c610f3b" + "2" * 33, "mwse": "89198b53" + "3" * 32}


def fake_set(root, text="v1", lua51="lua v1", name=None):
    """A sealed set with a file of each group of the map, the files that no deploy writes, and plugin-test/."""
    name = name or sets.set_name(FAKE_COMMITS)
    folder = os.path.join(root, name)
    for path in ("d3d8.dll", "dinput8.dll", "numidium.exe", "MGEXEgui.exe", "MGE XE Readme.md", "mge3/MGE XE Default Statics Classifiers.toml",
                 "Data Files/meshes/f/a.nif", "Data Files/textures/MGE/b.dds", "Data Files/mwse/mods/MGE XE Options/main.lua",
                 "Data Files/mwse/mods/nirender/main.lua", "Data Files/mwse/mods/nirender/failure.lua", "Data Files/shaders/core/x.fx",
                 "MWSE.dll", "Data Files/mwse/core/initialize.lua", "Data Files/mwse/core/lib/a.lua"):
            write(os.path.join(folder, *path.split("/")), text + path)
    write(os.path.join(folder, *sets.PLUGIN_STAGED.split("/")), text + "plugin")
    write(os.path.join(folder, "akulakhan-%s.dll" % FAKE_COMMITS["nirender"][:7]), text + "plugin")
    if lua51:
        write(os.path.join(folder, LUA51), lua51)
    for path in ("Data Files/XE Sky Variations.esp", "plugin-test/MWSE/mods/nirender/test.lua", "plugin-test/meshes/t.nif"):
        write(os.path.join(folder, *path.split("/")), "no " + path)
    manifest = sets.scratch_manifest(folder, name)
    manifest["commits"] = {repo: {"commit": commit, "clean": True} for repo, commit in FAKE_COMMITS.items()}
    with open(os.path.join(folder, sets.MANIFEST), "w", encoding="utf-8") as file:
        json.dump(manifest, file, indent=1, sort_keys=True)
    return folder


def fake_install(root, mge_mod="MGE X", old=None, lua51="lua old"):
    """An install with its three mods; with `old`, files of an older deploy and files that are the install's own."""
    for mod in (mge_mod, "nirender", "MWSE"):
        os.makedirs(os.path.join(root, "mods", mod, "Root"), exist_ok=True)
    if old:
        write(os.path.join(root, "mods", mge_mod, "Root", "d3d8.dll"), old)
        write(os.path.join(root, "mods", mge_mod, "Root", "d3d9.dll"), "the install's own")
        write(os.path.join(root, "mods", mge_mod, "Root", "mgeXE.toml"), "the user's settings")
        write(os.path.join(root, "mods", "nirender", "MWSE", "lib", "akulakhan.dll"), old)
        write(os.path.join(root, "mods", mge_mod, "shaders", "water", "tw_lava.wgsl"), "the mod's own")
        write(os.path.join(root, "mods", "MWSE", "MWSE", "core", "lpeg.dll"), "the install's own")
    if lua51:
        write(os.path.join(root, "mods", "MWSE", "Root", LUA51), lua51)
    return {"root": root, "mge_mod": mge_mod, "nirender_mod": "nirender"}


def tree_of(folder):
    return {path: md5_of(os.path.join(folder, *path.split("/"))) for path in files_below(folder)} if os.path.isdir(folder) else {}


def self_test():
    quiet = lambda: ""  # noqa: E731 - a tasklist with no program
    with tempfile.TemporaryDirectory(prefix="deploy-rest-test-") as root:
        sets_root = os.path.join(root, "sets")
        folder = fake_set(sets_root)
        name = os.path.basename(folder)
        installs = {"A": fake_install(os.path.join(root, "A"), "MGE A", old="old", lua51="lua v1"),
                    "B": fake_install(os.path.join(root, "B"), "MGE B", lua51="lua v1")}
        deploys = {name_: os.path.join(install["root"], "mods", "MWSE", "Root", "MWSE.dll") for name_, install in installs.items()}

        def go(**options):
            values = dict(sets_root=sets_root, tasklist=quiet, mwse_deploys=deploys)
            values.update(options)
            return run(folder, installs, **values)

        # The plan: the run's map less plugin-test, path for path (addendum 1 A4), and nothing that a deploy never writes.
        manifest, placed = set_plan(folder)
        runner = {path: place for path, place in sets.placing(manifest)[0].items() if place[0] != "test"}
        assert placed == runner, (placed, runner)
        assert placed["MWSE.dll"] == ("mwse", "Root/MWSE.dll") and placed[LUA51] == ("mwse", "Root/lua51.dll")
        assert placed["Data Files/mwse/core/lib/a.lua"] == ("mwse", "MWSE/core/lib/a.lua")
        assert placed[sets.PLUGIN_STAGED] == ("nirender", "MWSE/lib/akulakhan.dll")
        assert not [path for path in placed if path.startswith(("plugin-test/", "akulakhan-")) or path.endswith(".esp")], placed

        # The programs: a game, numidium and Mod Organizer stop the deploy; others do not.
        listing = '"System","4","Services","0","100 K"\n"Morrowind.exe","12","Console","1","900 K"\n"ModOrganizer.exe","13","Console","1","90 K"\n"numidium.exe","14","Console","1","9 K"\n"cargo.exe","15","Console","1","9 K"\n'
        assert busy_programs(listing) == ["ModOrganizer.exe", "Morrowind.exe", "numidium.exe"], busy_programs(listing)
        before = {name_: tree_of(install["root"]) for name_, install in installs.items()}
        try:
            go(tasklist=lambda: listing)
            raise AssertionError("a deploy while a game runs")
        except Refused as error:
            assert str(error).startswith("REFUSED: ") and "Morrowind.exe" in str(error)
        assert {name_: tree_of(install["root"]) for name_, install in installs.items()} == before, "a refused deploy wrote"

        # The dry run writes nothing.
        assert go(dry_run=True) == 0
        assert {name_: tree_of(install["root"]) for name_, install in installs.items()} == before, "the dry run wrote"
        assert not os.path.exists(os.path.join(sets_root, CURRENT_BAT))

        # The deploy: every file of the plan in each install, the install's own files stay, one backup, the record, current.bat.
        deploys_file = os.path.join(root, "deploys.tsv")
        assert go(deploys_file=deploys_file) == 0
        for name_, install in installs.items():
            installed = with_mwse_mods({name_: install}, deploys)[name_]
            for path, (kind, dest) in placed.items():
                target = os.path.join(mod_folder(installed, kind), *dest.split("/"))
                assert md5_of(target) == manifest["files"][path]["md5"], (name_, path)
            note = read_mwse_note(install)
            assert note["commit"] == FAKE_COMMITS["mwse"] and note["md5"] == manifest["files"]["MWSE.dll"]["md5"] and note["core_files"] == 2, note
        a_mods = os.path.join(root, "A", "mods")
        assert open(os.path.join(a_mods, "MGE A", "Root", "mgeXE.toml")).read() == "the user's settings"
        assert open(os.path.join(a_mods, "MGE A", "Root", "d3d9.dll")).read() == "the install's own"
        assert open(os.path.join(a_mods, "MGE A", "shaders", "water", "tw_lava.wgsl")).read() == "the mod's own"
        assert open(os.path.join(a_mods, "MWSE", "MWSE", "core", "lpeg.dll")).read() == "the install's own"
        assert not os.path.exists(os.path.join(a_mods, "nirender", "MWSE", "mods", "nirender", "test.lua")), "plugin-test reached an install"
        backup = os.path.join(root, "A", "mge-backup-" + name, "MGE A", "Root", "d3d8.dll")
        assert open(backup).read() == "old", "the backup of the replaced file"
        with open(os.path.join(sets_root, CURRENT_BAT), "rb") as file:
            assert file.read() == sets.current_bat_text(folder).encode("ascii"), "current.bat is the one line of the set (3.2)"
        assert FAKE_COMMITS["nirender"] in open(deploys_file, encoding="ascii").read()
        # --check: as sealed; a changed file in one install fails, with its name; the null passes again.
        assert check_deployed(folder, installs, deploys) == 0
        lua = os.path.join(a_mods, "MWSE", "MWSE", "core", "lib", "a.lua")
        write(lua, "changed by hand")
        assert check_deployed(folder, installs, deploys) == FAULT_EXIT
        write(lua, "v1Data Files/mwse/core/lib/a.lua")
        assert check_deployed(folder, installs, deploys) == 0
        # A second deploy of the same set copies nothing and keeps the first backup.
        assert go() == 0 and open(backup).read() == "old"

        # The change-over of lua51.dll (D 11.2): a new lua51.dll stops the deploy and the dry run, with the three md5s;
        # --lua51-change with the set's md5 lets it in; another md5 is refused.
        other = fake_set(os.path.join(root, "other-sets"), text="v2", lua51="lua v2")
        other_manifest = sets.read_manifest(other)
        new_md5 = other_manifest["files"][LUA51]["md5"]
        for dry in (True, False):
            try:
                run(other, installs, sets_root=sets_root, tasklist=quiet, mwse_deploys=deploys, dry_run=dry)
                raise AssertionError("a new lua51.dll passed")
            except DeployError as error:
                assert str(error).startswith("LUA51 CHANGE: ") and new_md5 in str(error) and "A md5 " in str(error) and "B md5 " in str(error), error
        assert check_deployed(folder, installs, deploys) == 0, "the refused deploy wrote"
        try:
            run(other, installs, sets_root=sets_root, tasklist=quiet, mwse_deploys=deploys, lua51_change="0" * 32)
            raise AssertionError("a wrong --lua51-change passed")
        except DeployError as error:
            assert "is not the set's lua51.dll" in str(error), error
        assert run(other, installs, sets_root=sets_root, tasklist=quiet, mwse_deploys=deploys, lua51_change=new_md5) == 0
        assert md5_of(os.path.join(a_mods, "MWSE", "Root", LUA51)) == new_md5
        # A set without lua51.dll leaves the installs' own.
        old_set = fake_set(os.path.join(root, "old-sets"), text="v3", lua51=None)
        assert run(old_set, installs, sets_root=sets_root, tasklist=quiet, mwse_deploys=deploys) == 0
        assert md5_of(os.path.join(a_mods, "MWSE", "Root", LUA51)) == new_md5

        # Refused before any write: a set that fails sets.verify (a changed file; a file with no place), a set with a
        # patch, a commit of the merge step that is not the set's, an install without one of its mods.
        before = {name_: tree_of(install["root"]) for name_, install in installs.items()}
        write(os.path.join(folder, "numidium.exe"), "changed")
        for options, expected in ((dict(), "numidium.exe has"), (dict(plugin_commit="abcdef0"), "numidium.exe has")):
            try:
                go(**options)
                raise AssertionError("a changed set passed")
            except DeployError as error:
                assert expected in str(error), error
        write(os.path.join(folder, "numidium.exe"), "v1numidium.exe")
        try:
            go(plugin_commit="abcdef0")
            raise AssertionError("another plugin commit passed")
        except DeployError as error:
            assert "--plugin-commit abcdef0 is not the set's nirender commit" in str(error), error
        # The null of the commits (the installs hold the second set's lua51.dll by now: the change-over is given).
        assert go(plugin_commit=FAKE_COMMITS["nirender"][:7], mge_commit=FAKE_COMMITS["render-link"][:7], dry_run=True,
                  lua51_change=manifest["files"][LUA51]["md5"]) == 0
        write(os.path.join(folder, "tool.exe"), "new")
        sets.scratch_manifest(folder, name)
        with open(os.path.join(folder, sets.MANIFEST), encoding="utf-8") as file:
            resealed = json.load(file)
        resealed["commits"] = manifest["commits"]
        with open(os.path.join(folder, sets.MANIFEST), "w", encoding="utf-8") as file:
            json.dump(resealed, file)
        try:
            go()
            raise AssertionError("a file with no place passed (F-T-4)")
        except DeployError as error:
            assert "the file tool.exe has no place" in str(error), error
        patched = fake_set(os.path.join(root, "patched"), name=sets.set_name(FAKE_COMMITS, {"render-link": "ab" * 16}))
        try:
            run(patched, installs, sets_root=sets_root, tasklist=quiet, mwse_deploys=deploys)
            raise AssertionError("a patched set passed")
        except DeployError as error:
            assert "has a patch or a kind" in str(error), error
        shutil.rmtree(os.path.join(root, "B", "mods", "MWSE"))
        try:
            run(old_set, installs, sets_root=sets_root, tasklist=quiet, mwse_deploys=deploys)
            raise AssertionError("an install without its MWSE mod passed")
        except DeployError as error:
            assert "has no mod folder" in str(error) and "MWSE" in str(error), error
        installs.pop("B")
        assert {name_: tree_of(install["root"]) for name_, install in installs.items()} == {"A": before["A"]}, "a refused deploy wrote"

        core_check_test(root)
        distant_land_test(root)
        key_first_test(root)
    print("deploy_rest self-test: ok")


def distant_land_test(root):
    """W521 step 4: after a deploy, the set's mod is on in the install's mod list and a marked mod of another key is off;
    the dry run changes nothing. The null: an install entry with no profile is passed over with a line."""
    import distant_land
    instance = os.path.join(root, "dl-instance")
    profile = os.path.join(instance, "profiles", "Default")
    os.makedirs(profile)
    old = distant_land.Key("own-00000000", "own-00000000", "base")
    distant_land.ensure_mod(instance, old, "c0ffee")
    with open(os.path.join(profile, "modlist.txt"), "w", encoding="utf-8") as file:
        file.write("# MO2\n+%s\n" % old.mod_name)
    manifest = {"distant_land_format": "own-5296d863", "commits": {"render-link": {"commit": "5296d863" * 5, "clean": True}}}
    installs = {"T": {"root": instance, "profile": "Default", "config_mod": "Configuration"}}
    keys = distant_land_keys("S", manifest, installs)
    assert "would put" in distant_land_lines(keys, installs, apply=False)[0]
    assert open(os.path.join(profile, "modlist.txt"), encoding="utf-8").read() == "# MO2\n+%s\n" % old.mod_name
    line = distant_land_lines(keys, installs, apply=True)[0]
    assert "Distant land own-5296d863-base on" in line, line
    assert open(os.path.join(profile, "modlist.txt"), encoding="utf-8").read() == "# MO2\n+Distant land own-5296d863-base\n-%s\n" % old.mod_name
    bare = {"U": {"root": instance}}
    assert distant_land_lines(distant_land_keys("S", manifest, bare), bare, apply=True) == ["U: no profile named; the distant land mod is not set"]


def key_first_test(root, deploy=None):
    """Verdict 553 C3: the distant land key of every install comes before the first copy. A failing key stops the
    deploy, and the install does not change. C3b: so does a folder of the key's mod name with no marker. The null: the
    same deploy with a working key and no such folder writes the files and puts the key's mod on. `deploy` is run, or
    the run of an older deploy_rest for the null of the order."""
    import distant_land
    deploy = deploy or run
    sets_root = os.path.join(root, "c3-sets")
    folder = fake_set(sets_root)
    instance = os.path.join(root, "C3")
    installs = {"C": dict(fake_install(instance, "MGE C", lua51="lua v1"), profile="Default", config_mod="Configuration")}
    modlist = os.path.join(instance, "profiles", "Default", "modlist.txt")
    write(modlist, "# MO2\n")
    deploys = {"C": os.path.join(instance, "mods", "MWSE", "Root", "MWSE.dll")}
    before = tree_of(instance)

    def no_key(*args, **kwargs):
        raise distant_land.Problem("planted: the set's helper gave no key")

    real_key_of = distant_land.key_of
    distant_land.key_of = no_key
    try:
        deploy(folder, installs, sets_root=sets_root, tasklist=lambda: "", mwse_deploys=deploys)
        passed = True
    except Exception as error:  # noqa: BLE001 - the null runs an older module, with its own DeployError
        passed = False
        assert "planted" in str(error), error
    finally:
        distant_land.key_of = real_key_of
    assert not passed, "a deploy with no key passed"
    assert tree_of(instance) == before, "a deploy whose key failed wrote into the install"

    # C3b: a folder of the key's mod name that the tool did not make (no marker).
    planted = os.path.join(instance, "mods", "Distant land own-%s-base" % FAKE_COMMITS["render-link"][:8], "by hand.txt")
    write(planted, "the owner's")
    with_plant = tree_of(instance)
    try:
        deploy(folder, installs, sets_root=sets_root, tasklist=lambda: "", mwse_deploys=deploys)
        passed = True
    except Exception as error:  # noqa: BLE001 - the null runs an older module, with its own DeployError
        passed = False
        assert "not a mod of" in str(error), error
    assert not passed, "a deploy over an unmarked mod folder passed"
    assert tree_of(instance) == with_plant, "a deploy over an unmarked mod folder wrote into the install"
    os.remove(planted)
    os.rmdir(os.path.dirname(planted))

    assert deploy(folder, installs, sets_root=sets_root, tasklist=lambda: "", mwse_deploys=deploys) == 0
    assert tree_of(instance) != before
    assert "+Distant land own-%s-base" % FAKE_COMMITS["render-link"][:8] in open(modlist, encoding="utf-8").read().split("\n")


def core_check_test(root):
    """D442: --check-mwse-core on the record that a deploy writes. Each fault fails its install only, and the check passes
    again when the fault is gone (the null)."""
    pair = {"G": fake_install(os.path.join(root, "G"), "MGE G"), "H": fake_install(os.path.join(root, "H"), "MGE H")}
    deploys = {name: os.path.join(install["root"], "mods", "MWSE", "Root", "MWSE.dll") for name, install in pair.items()}
    sets_root = os.path.join(root, "core-sets")
    folder = fake_set(sets_root, lua51="lua old")
    assert run(folder, pair, sets_root=sets_root, tasklist=lambda: "", mwse_deploys=deploys) == 0
    commit = FAKE_COMMITS["mwse"]

    def fake_core(wanted, into):
        core = os.path.join(into, "core")
        assert wanted == commit, wanted
        write(os.path.join(core, "initialize.lua"), "v1Data Files/mwse/core/initialize.lua")
        write(os.path.join(core, "lib", "a.lua"), "v1Data Files/mwse/core/lib/a.lua")
        return core

    def verdicts(places=deploys, source=fake_core):
        return [(good, line) for line, good in mwse_core_lines(pair, places, source)]

    assert check_mwse_core(pair, deploys, fake_core) == 0
    (good_g, line_g), (good_h, _) = verdicts()
    assert good_g and good_h and "is the core of MWSE %s" % commit in line_g, line_g
    # A core file changed by hand, then a missing one.
    a_lua = os.path.join(core_folder(deploys["G"]), "lib", "a.lua")
    for change in (lambda: write(a_lua, "a changed"), lambda: os.remove(a_lua)):
        change()
        (good_g, line_g), (good_h, _) = verdicts()
        assert not good_g and good_h and "1 of 2 files of the MWSE core" in line_g and "lib/a.lua" in line_g, line_g
    write(a_lua, "v1Data Files/mwse/core/lib/a.lua")
    # Another MWSE.dll than the record's, then no record: the commit is not known.
    mwse = deploys["H"]
    kept = open(mwse).read()
    write(mwse, "a hand-made mwse")
    assert "the record names another MWSE.dll" in verdicts()[1][1]
    write(mwse, kept)
    os.remove(os.path.join(pair["H"]["root"], MWSE_NOTE))
    assert "no record" in verdicts()[1][1] and "deploy_rest.py --set" in verdicts()[1][1]
    write_mwse_note(pair["H"], commit, deploys["H"], 2, "f1")
    # No MWSE.dll at its place, and a commit whose core git cannot give.
    assert verdicts(places={"G": deploys["G"]})[1] == (False, "H: no MWSE.dll at its place of mwse_deploys (None)")

    def no_core(wanted, into):
        raise DeployError("git archive of the core at %s failed" % wanted)

    assert all(not good and "git archive of the core at %s failed" % commit in line for good, line in verdicts(source=no_core))
    assert check_mwse_core(pair, deploys, no_core) == FAULT_EXIT
    # The null: with every fault gone, the check passes.
    assert check_mwse_core(pair, deploys, fake_core) == 0


def current_set_folder(sets_root=sets.SETS):
    """The folder of the current set (MGE_BUILD of current.bat), or None."""
    import clean_common

    name = clean_common.current_set(sets=sets_root)
    return os.path.join(sets_root, name) if name else None


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--set", help="the sealed set to deploy (tools/build_set.py makes one)")
    parser.add_argument("--plugin-commit", help="the commit of nirender-plugin that the set must be of (the merge step's)")
    parser.add_argument("--mge-commit", help="the commit of render-link that the set must be of (the merge step's)")
    parser.add_argument("--tag", help="the tag of the backups; by default the set's name")
    parser.add_argument("--dry-run", action="store_true", help="print what a deploy would copy; write nothing")
    parser.add_argument("--lua51-change", metavar="MD5", help="on the Coordinator's word: let the set's lua51.dll of this MD5 replace "
                        "the installs' (the change-over of D 11.2)")
    parser.add_argument("--check", action="store_true", help="only check each install's deployed files against the set (--set, else "
                        "the current set); writes nothing")
    parser.add_argument("--sets", default=sets.SETS, help="the folder of the sets and of current.bat (a test changes it)")
    parser.add_argument("--check-mwse-core", action="store_true", help="only check that each install's MWSE core is the core of the MWSE commit "
                        "of its MWSE.dll, by its record %s (the merge step's stage); writes nothing" % MWSE_NOTE)
    parser.add_argument("--self-test", action="store_true", help="the tests: scratch sets and installs, no game")
    # The options of the deploy of a build folder (before W521): each stops the tool with a line.
    for gone in ("--build", "--mwse-dll", "--mwse-commit", "--mge"):
        parser.add_argument(gone, help="gone (W521): a deploy takes a sealed set (--set)")
    parser.add_argument("--no-keep", action="store_true", help="gone (W521): a deploy keeps no copy; the set is kept")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    gone = [option for option, value in (("--build", args.build), ("--mwse-dll", args.mwse_dll), ("--mwse-commit", args.mwse_commit),
                                         ("--mge", args.mge), ("--no-keep", args.no_keep)) if value]
    if gone:
        say("FAIL: %s %s gone: a deploy takes a sealed set; make one with tools/build_set.py and give it with --set (W521)" % (
            ", ".join(gone), "is" if len(gone) == 1 else "are"))
        return FAULT_EXIT
    if args.check_mwse_core:
        return check_mwse_core(REST_INSTALLS, MWSE_DEPLOYS)
    try:
        if args.check:
            folder = os.path.abspath(args.set) if args.set else current_set_folder(args.sets)
            if not folder:
                raise DeployError("no --set and no current set in %s" % os.path.join(args.sets, CURRENT_BAT))
            return check_deployed(folder, REST_INSTALLS)
        if not args.set:
            parser.error("it needs --set, --check, --check-mwse-core or --self-test")
        return run(os.path.abspath(args.set), REST_INSTALLS, args.plugin_commit, args.mge_commit, args.tag, args.dry_run, args.sets,
                   add_symbols=symbols.add, deploys_file=DEPLOYS_FILE, lua51_change=args.lua51_change)
    except Refused as error:
        say(str(error))
        return REFUSED_EXIT
    except DeployError as error:
        say("FAIL: %s" % error)
        return FAULT_EXIT


if __name__ == "__main__":
    sys.exit(main())
