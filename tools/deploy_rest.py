"""The resting build: puts a build of `cargo xtask dist` into both test installs, and keeps a copy of it.

    python -I tools/deploy_rest.py --build <dist-staging> [--plugin-commit SHA] [--mge-commit SHA] [--tag NAME] [--no-keep] [--dry-run]
    python -I tools/deploy_rest.py --build <dist-staging> --mwse-dll <MWSE.dll> [...]   # the build and its MWSE together
    python -I tools/deploy_rest.py --mwse-dll <MWSE.dll> [--tag NAME] [--dry-run]       # an MWSE.dll beside the installed plugin
    python -I tools/deploy_rest.py --self-test    # the tests: scratch installs, no game, nothing of the real installs

The merge step runs this as the stage "deploy rest" (the owner's D193: the test installs always hold the latest build).
<build> is the folder target/dist-staging of render-link. Each install of REST_INSTALLS gets:

- into its MGE mod: the root files (d3d8.dll, dinput8.dll, numidium.exe, MGEXEgui.exe, the readmes, the classifiers of
  mge3/) under Root/, and the meshes, the textures, the shaders and the "MGE XE Options" mod of Data Files/. Every file
  of those folders is added or replaced; a file that the build lacks stays (the install may hold shaders of its own, such
  as water/tw_lava.wgsl), and the output names such files for the shaders as "only in the mod" (the Coordinator, W273);
- into its nirender mod: akulakhan.dll and the Lua files of the nirender mod.

It never copies a pdb, an esp or esm, mgeXE.toml or d3d9.dll (WITHHELD: these belong to the user's install or to a
debugger). It writes only files of that plan, and nothing in the user's settings. A file that already equals its source is
left as it is. A file it replaces is first copied to <install>/mge-backup-<tag>/<mod>/<path>, once for each tag: a
second run with the same tag keeps the first backup. Each copy is compared with its source by MD5. Nothing of the build
is deployed that the plan does not name: the output lists the files left out by folder, so that a new folder of the build
shows up.

It refuses, and writes nothing, while Morrowind, numidium or Mod Organizer runs, and it does not wait: exit code 3 and a line
that starts with "REFUSED". Any other fault is exit code 1. Before the first write it also checks that every source
exists, that each install has both mod folders (it makes no mod folder), and that the build ids in akulakhan.dll,
numidium.exe and d3d8.dll are one build (the dry run too): the plugin and the host crate of one commit of nirender-plugin,
numidium and d3d8.dll of one commit of render-link, no "-dirty", and the commits of --plugin-commit and --mge-commit
when given (the merge step gives the commits it checks). WHAT.txt names the commits of the parts.

Unless --no-keep, it keeps the build as the next set folder f<N> of SETS (E:\\nirender-keep\\mgebuilds) with no pdb, the
plugin beside it as akulakhan-<commit>.dll, and a one-line WHAT.txt: the same shape as the play sets of make_play_set.py.
The tag of the backups is that name. The copy to E: is the stage's write on the evidence disk: the merge step holds the
game slot in a round with this stage (its GAME_STAGES), and the stage refuses while a game runs.
The output prints the MD5 of d3d8.dll, numidium.exe and akulakhan.dll. --dry-run reads the installs and writes nothing.

--mwse-dll (W406) puts that MWSE.dll into each install too, at its place in MWSE_DEPLOYS, after the build: a plugin that
needs a new MWSE and that MWSE go together, under one backup tag. Without --build it puts only the MWSE.dll, beside the
plugin that the installs hold. Before any write (and in the dry run) it probes the DLL for the MWSE features that the
plugin needs (tools/check_mwse_sites.py --features, which runs the plugin's tests through cargo: a compile, so take a
build place first when you run it by hand): a missing required feature refuses with exit code 4
(MISSING_FEATURE_EXIT), any other failure of the probe is exit code 1, and nothing is written. The probe is the code of
this tree, so the tree must be at the commit of the plugin that goes beside the DLL, with no change in src/: the build's
plugin, or without --build the plugin of both installs (one commit). The output and WHAT.txt name the MD5 of the
MWSE.dll. A fault of the MWSE copy after the build was written says so: the installs may then hold the new plugin
beside the old MWSE.dll, and the replaced files are in the backups.
"""
import argparse
import dataclasses
import datetime
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile

# The modules of this folder, also under `python -I`.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from check_mwse_sites import MISSING, MISSING_FEATURE_EXIT  # noqa: E402
from common import BUILD_ID_RE, REPO, git_env, local, parts_problems, same_commit  # noqa: E402
from make_play_set import say  # noqa: E402
from new_run_folder import SETS, new_set_folder  # noqa: E402  (one rule for the number, W380)
import symbols  # noqa: E402

# The test installs, by name: the folder of the install (MO2 instance) and the names of its two mods. The key
# "rest_installs" of local.json changes the places.
REST_INSTALLS = local("rest_installs", {
    "Morrowind75testing": {"root": "D:/Modlists/Morrowind75testing", "mge_mod": "MGE XE G7 Fork", "nirender_mod": "nirender"},
    "Outlander-test": {"root": "D:/Modlists/Outlander-test", "mge_mod": "MGE XE G7", "nirender_mod": "nirender"},
})
# The MWSE.dll that each test install has deployed (the line build), by install: the "mwse features" and "mwse sites"
# stages of merge_step.py check these, and --mwse-dll writes them. Never a `Harness - *` mod: a live run puts its own
# MWSE.dll there for its game hold. The key "mwse_deploys" of local.json changes the places.
MWSE_DEPLOYS = local("mwse_deploys", {
    "Morrowind75testing": "D:/Modlists/Morrowind75testing/mods/MWSE/Root/MWSE.dll",
    "Outlander-test": "D:/Modlists/Outlander-test/mods/MWSE-Update for Nightly Builds/Root/MWSE.dll",
})
CHECK_MWSE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "check_mwse_sites.py")
# The programs that stop a deploy, as a part of the name of the image (without the case).
BUSY_PROGRAMS = ("morrowind", "numidium", "modorganizer")
REFUSED_EXIT = 3
FAULT_EXIT = 1
# The root files of the build that go into the MGE mod by name; its readmes (every .md of the root) go too. A root file
# that is not here stays out of the plan. The classifiers are the .toml files of the folder mge3/.
ROOT_FILES = ("d3d8.dll", "dinput8.dll", "numidium.exe", "MGEXEgui.exe")
DATA_FILES = "Data Files/"
# The folders of Data Files/ that go into the MGE mod, as paths below Data Files/.
MGE_DATA_FOLDERS = ("meshes", "textures", "shaders", "mwse/mods/MGE XE Options")
NIRENDER_PLUGIN = DATA_FILES + "mwse/lib/akulakhan.dll"
# The binaries of a build whose pdbs go into the symbol store (W345), as paths in dist-staging.
SYMBOL_BINARIES = ("d3d8.dll", "dinput8.dll", "numidium.exe", NIRENDER_PLUGIN)
NIRENDER_LUA = DATA_FILES + "mwse/mods/nirender"
# What a build must hold: a deploy without these is no deploy.
REQUIRED = ("d3d8.dll", "numidium.exe", NIRENDER_PLUGIN, NIRENDER_LUA + "/main.lua")
# The files the output names with their MD5.
NAMED_DIGESTS = (("d3d8.dll", "d3d8.dll"), ("numidium.exe", "numidium.exe"), ("akulakhan.dll", NIRENDER_PLUGIN))


class DeployError(Exception):
    """A fault of the deploy; the message is the line that the stage prints."""


class Refused(DeployError):
    """A game or Mod Organizer runs: the deploy writes nothing and does not wait."""


class MissingFeature(DeployError):
    """The MWSE.dll lacks a feature that the plugin needs: the deploy writes nothing."""


@dataclasses.dataclass(frozen=True)
class Item:
    source: str  # the path below the build folder, with "/"
    mod: str  # "mge" or "nirender"
    dest: str  # the path below the mod folder, with "/"


def md5_of(path):
    digest = hashlib.md5()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def withheld(path):
    """True for a file that no deploy writes: a pdb, an esp or esm, the user's mgeXE.toml and the d3d9.dll of the install."""
    name = path.replace("\\", "/").rsplit("/", 1)[-1].lower()
    return name.endswith((".pdb", ".esp", ".esm")) or name in ("mgexe.toml", "d3d9.dll")


def files_below(folder):
    """The files below a folder, as paths with "/" relative to it, sorted."""
    found = []
    for base, _, names in os.walk(folder):
        for name in names:
            found.append(os.path.relpath(os.path.join(base, name), folder).replace("\\", "/"))
    return sorted(found)


def plan_of(staged):
    """What a deploy writes, as (items, left out): the items for the files of the build (the paths of `files_below`), and the
    files the plan does not name or withholds. The items are sorted by mod and path."""
    items = []
    for path in staged:
        folder, _, name = path.rpartition("/")
        if folder == "" and (name in ROOT_FILES or name.lower().endswith(".md")):
            items.append(Item(path, "mge", "Root/" + path))
        elif folder == "mge3" and name.lower().endswith(".toml"):
            items.append(Item(path, "mge", "Root/" + path))
        elif path.startswith(DATA_FILES):
            below = path[len(DATA_FILES):]
            if any(below.startswith(data_folder + "/") for data_folder in MGE_DATA_FOLDERS):
                items.append(Item(path, "mge", below))
            elif path == NIRENDER_PLUGIN:
                items.append(Item(path, "nirender", "MWSE/lib/akulakhan.dll"))
            elif folder == NIRENDER_LUA:
                items.append(Item(path, "nirender", "MWSE/mods/nirender/" + name))
    kept = [item for item in items if not withheld(item.dest) and not withheld(item.source)]
    named = {item.source for item in kept}
    return sorted(kept, key=lambda item: (item.mod, item.dest)), [path for path in staged if path not in named]


def left_out_summary(left_out):
    """The left-out files by their folder: 'Data Files/shaders (31), Data Files/XE Sky Variations.esp'. A folder is the first
    two path parts of a file in Data Files/, else the first part."""
    counts = {}
    for path in left_out:
        parts = path.split("/")
        key = "/".join(parts[:2]) if parts[0] == "Data Files" else parts[0]
        counts[key] = counts.get(key, 0) + 1
    return ", ".join(name if count == 1 else "%s (%d)" % (name, count) for name, count in sorted(counts.items()))


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


def mod_folder(install, mod):
    """The folder of one mod of an install, by 'mge' or 'nirender'."""
    return os.path.join(install["root"], "mods", install["mge_mod"] if mod == "mge" else install["nirender_mod"])


def check_installs(installs):
    for name, install in installs.items():
        for mod in ("mge", "nirender"):
            if not os.path.isdir(mod_folder(install, mod)):
                raise DeployError("%s has no mod folder %s: the deploy makes none" % (name, mod_folder(install, mod)))


def check_sources(build, items):
    missing = [path for path in REQUIRED if not os.path.isfile(os.path.join(build, path))]
    if missing:
        raise DeployError("%s lacks %s: this is no build of `cargo xtask dist`" % (build, ", ".join(missing)))
    names = {item.source for item in items}
    absent = [path for path in REQUIRED if path not in names]
    if absent:
        raise DeployError("the plan has no item for %s" % ", ".join(absent))


def put(build, item, install, tag, copy=shutil.copyfile):
    """Copies one item into one install: 'unchanged' when the file equals its source, else 'copied' after the backup of the
    file it replaces (once for each tag). Raises DeployError when the copy does not equal its source."""
    if withheld(item.dest):
        raise DeployError("refused to write %s: a withheld file" % item.dest)
    source = os.path.join(build, *item.source.split("/"))
    folder = mod_folder(install, item.mod)
    dest = os.path.join(folder, *item.dest.split("/"))
    backup = os.path.join(install["root"], "mge-backup-" + tag, os.path.basename(folder), *item.dest.split("/"))
    return replace_file(source, dest, backup, copy, item.source)


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


def plugin_commit_of(dll):
    """The commit in the build id of a plugin file, with its "-dirty"; DeployError when it holds no single id."""
    with open(dll, "rb") as file:
        ids = sorted({m.group(2).decode() + (m.group(3) or b"").decode() for m in BUILD_ID_RE.finditer(file.read()) if m.group(1) == b"nirender"})
    if len(ids) != 1:
        raise DeployError("%s holds %s build id of nirender%s" % (dll, "no" if not ids else "%d different" % len(ids), ": " + ", ".join(ids) if ids else ""))
    return ids[0]


def installed_plugin_commit(installs):
    """The commit of the plugin that every install holds; DeployError when the installs hold plugins of two commits."""
    commits = {name: plugin_commit_of(os.path.join(mod_folder(install, "nirender"), "MWSE", "lib", "akulakhan.dll")) for name, install in installs.items()}
    if len(set(commits.values())) != 1:
        raise DeployError("the installs hold plugins of different commits (%s): deploy a build with the MWSE.dll" % ", ".join("%s %s" % item for item in sorted(commits.items())))
    return next(iter(commits.values()))


def tree_head():
    """The commit of this tree, with "-dirty" when src/ has a tracked change: the probe of the features is its code."""
    head = subprocess.run(["git", "-C", REPO, "rev-parse", "HEAD"], capture_output=True, text=True, env=git_env()).stdout.strip()
    changed = subprocess.run(["git", "-C", REPO, "diff", "--quiet", "HEAD", "--", "src"], env=git_env()).returncode != 0
    return head + ("-dirty" if changed else "")


def probe_features(dll):
    """tools/check_mwse_sites.py --features on `dll`, in this tree: (exit code, output)."""
    done = subprocess.run([sys.executable, "-I", CHECK_MWSE, "--features", dll], cwd=REPO, capture_output=True, text=True,
                          encoding="utf-8", errors="replace", stdin=subprocess.DEVNULL)
    return done.returncode, done.stdout + done.stderr


def mwse_backup(install, dest, tag):
    """The backup of an install's MWSE.dll: below mge-backup-<tag> by its path below the install's mods folder."""
    mods = os.path.join(install["root"], "mods")
    below = os.path.relpath(dest, mods)
    return os.path.join(install["root"], "mge-backup-" + tag, below if not below.startswith("..") else os.path.basename(dest))


def mwse_targets(mwse_dll, installs, plugin_commit, deploys, probe, tree_commit):
    """The place of the MWSE.dll in each install, {install name: path}, after the checks before any write: the file, the
    tree at the plugin's commit, the probe of the features, a place for each install in an existing folder. Raises
    MissingFeature when the probe names a missing required feature, DeployError on any other fault."""
    if not os.path.isfile(mwse_dll):
        raise DeployError("no file %s" % mwse_dll)
    tree = tree_commit()
    # same_commit compares by the start, so a "-dirty" tree would pass it.
    if tree.endswith("-dirty") or not same_commit(tree, plugin_commit):
        raise DeployError("the probe of the features is the code of this tree, at %s, but the plugin that goes beside %s is %s: run this in a tree at %s with no change in src/" % (
            tree, mwse_dll, plugin_commit, plugin_commit))
    code, output = probe(mwse_dll)
    for line in output.splitlines():
        if line.startswith(("feature: ", MISSING, "absent: ", "md5: ")):
            say("  " + line)
    if code == MISSING_FEATURE_EXIT:
        missing = [line[len(MISSING):] for line in output.splitlines() if line.startswith(MISSING)]
        raise MissingFeature("MISSING FEATURE: %s lacks MWSE features that the plugin %s needs: %s" % (mwse_dll, plugin_commit, "; ".join(missing) or "see the probe"))
    if code != 0:
        raise DeployError("the probe of the features of %s failed with exit code %d: %s" % (mwse_dll, code, (output.strip().splitlines() or ["no output"])[-1]))
    targets = {}
    for name in installs:
        if name not in deploys:
            raise DeployError("no place of the MWSE.dll of %s in mwse_deploys" % name)
        if not os.path.isdir(os.path.dirname(deploys[name])):
            raise DeployError("%s has no folder %s: the deploy makes none" % (name, os.path.dirname(deploys[name])))
        targets[name] = deploys[name]
    return targets


def deploy(build, installs, tag, items, copy=shutil.copyfile):
    """Puts the items into each install. Returns {install name: [copied, unchanged]}."""
    counts = {}
    for name, install in installs.items():
        copied = unchanged = 0
        for item in items:
            if put(build, item, install, tag, copy) == "copied":
                copied += 1
            else:
                unchanged += 1
        counts[name] = [copied, unchanged]
    return counts


def only_in_mod(install, items):
    """The files of the MGE mod's shaders folder that no item names: the install keeps them, and the output names them."""
    folder = os.path.join(mod_folder(install, "mge"), "shaders")
    named = {item.dest for item in items if item.mod == "mge"}
    return ["shaders/" + path for path in files_below(folder) if "shaders/" + path not in named] if os.path.isdir(folder) else []


def would_copy(build, installs, items):
    """The dry run: for each install, the items that a deploy would copy, by dest."""
    changes = {}
    for name, install in installs.items():
        changes[name] = []
        for item in items:
            dest = os.path.join(mod_folder(install, item.mod), *item.dest.split("/"))
            if not os.path.isfile(dest) or md5_of(dest) != md5_of(os.path.join(build, *item.source.split("/"))):
                changes[name].append(item.dest)
    return changes


def what_line(name, plugin_commit, mge_commit, digests, installs, stamp=None):
    """The one line of WHAT.txt."""
    stamp = stamp or datetime.datetime.now().strftime("%Y-%m-%d %H:%M")
    sums = ", ".join("%s %s" % (label, digest) for label, digest in digests)
    return "%s %s: the resting build of the merge step, nirender %s, render-link %s, no pdb, deployed to %s (backups mge-backup-%s); md5 %s\n" % (
        name, stamp, plugin_commit, mge_commit, ", ".join(installs), name, sums)


def keep_build(build, folder, plugin_commit):
    """Copies the build to the set folder with no pdb, and the plugin beside it. The copy of each file is compared by MD5."""
    shutil.copytree(build, folder, dirs_exist_ok=True, ignore=shutil.ignore_patterns("*.pdb"))
    plugin = os.path.join(folder, "akulakhan-%s.dll" % plugin_commit)
    shutil.copyfile(os.path.join(build, *NIRENDER_PLUGIN.split("/")), plugin)
    for path in files_below(build):
        if path.lower().endswith(".pdb"):
            continue
        if md5_of(os.path.join(build, *path.split("/"))) != md5_of(os.path.join(folder, *path.split("/"))):
            raise DeployError("the kept copy of %s differs from the build" % path)
    pdbs = [path for path in files_below(folder) if path.lower().endswith(".pdb")]
    if pdbs:
        raise DeployError("the kept copy holds a pdb: %s" % pdbs[0])


def check_parts(build, plugin_commit=None, mge_commit=None):
    """The plugin, the helper and d3d8.dll must be one build: a mismatched pair in an install shows only as the helper's
    refusal at the next game start. Returns the commits of the parts, (plugin, mge)."""
    commits, problems = parts_problems(build, plugin_commit, mge_commit)
    if problems:
        raise DeployError("the parts are not one build: %s" % "; ".join(problems))
    return commits


def run(build, installs, plugin_commit=None, mge_commit=None, tag=None, keep=True, dry_run=False, sets=SETS, tasklist=tasklist_text, copy=shutil.copyfile,
        add_symbols=None, mwse_dll=None, mwse_deploys=None, probe=probe_features, tree_commit=tree_head):
    """The whole deploy. Returns the exit code. `add_symbols` (symbols.add from the command line) puts the build's pdbs
    into the symbol store: the pdbs of render-link's tree around the dist-staging folder, and of this tree's plugin.
    `mwse_dll` goes into each install at its place of `mwse_deploys` (MWSE_DEPLOYS), after the build, or alone when
    `build` is None; `probe` and `tree_commit` are the probe of its features and the commit of the probe's code."""
    if build is None and mwse_dll is None:
        raise DeployError("nothing to deploy: give a build, an MWSE.dll or both")
    check_installs(installs)
    items, left_out, digests = [], [], []
    if build is not None:
        items, left_out = plan_of(files_below(build))
        check_sources(build, items)
        plugin_commit, mge_commit = check_parts(build, plugin_commit, mge_commit)
        say("parts: one build of nirender-plugin %s and render-link %s" % (plugin_commit, mge_commit))
        digests = [(label, md5_of(os.path.join(build, *path.split("/")))) for label, path in NAMED_DIGESTS]
    targets = {}
    if mwse_dll is not None:
        beside = plugin_commit if build is not None else installed_plugin_commit(installs)
        targets = mwse_targets(mwse_dll, installs, beside, MWSE_DEPLOYS if mwse_deploys is None else mwse_deploys, probe, tree_commit)
        say("MWSE.dll %s: has every MWSE feature that the plugin %s needs" % (mwse_dll, beside))
        digests.append(("MWSE.dll", md5_of(mwse_dll)))
    if dry_run:
        if build is not None:
            say("dry run: %d items for each of %d installs; nothing is written" % (len(items), len(installs)))
            for name, changed in would_copy(build, installs, items).items():
                first = ": " + ", ".join(changed[:5]) + (", ..." if len(changed) > 5 else "") if changed else ""
                say("  %s: would copy %d, unchanged %d%s" % (name, len(changed), len(items) - len(changed), first))
            say("left out of the plan: %s" % (left_out_summary(left_out) or "nothing"))
            for install_name, install in installs.items():
                say("  %s: shaders only in the mod: %s" % (install_name, ", ".join(only_in_mod(install, items)) or "none"))
        for install_name, dest in targets.items():
            same = os.path.isfile(dest) and md5_of(dest) == md5_of(mwse_dll)
            say("  %s: MWSE.dll %s: %s" % (install_name, dest, "unchanged" if same else "would copy"))
        for label, digest in digests:
            say("md5 %s %s" % (label, digest))
        return 0
    check_nothing_runs(tasklist)
    keep = keep and build is not None
    if keep:
        os.makedirs(sets, exist_ok=True)
    name = new_set_folder(sets) if keep else None
    tag = tag or name or datetime.datetime.now().strftime("rest-%Y%m%d-%H%M")
    if keep:
        keep_build(build, os.path.join(sets, name), plugin_commit)
        say("kept: %s" % os.path.join(sets, name))
    if build is not None:
        counts = deploy(build, installs, tag, items, copy)
        for install_name, (copied, unchanged) in counts.items():
            say("%s: copied %d, unchanged %d; backups in %s" % (install_name, copied, unchanged, os.path.join(installs[install_name]["root"], "mge-backup-" + tag)))
        say("left out of the plan: %s" % (left_out_summary(left_out) or "nothing"))
        # Not `name`: that is the set folder, and WHAT.txt goes into it below.
        for install_name, install in installs.items():
            say("%s: shaders only in the mod: %s" % (install_name, ", ".join(only_in_mod(install, items)) or "none"))
    for install_name, dest in targets.items():
        try:
            done = replace_file(mwse_dll, dest, mwse_backup(installs[install_name], dest, tag), copy, mwse_dll)
        except (OSError, DeployError) as error:
            raise DeployError("%s%s; the files that this run replaced are in mge-backup-%s of each install" % (
                error, ": the installs may now hold the new plugin beside the old MWSE.dll" if build is not None else "", tag))
        say("%s: MWSE.dll %s; backup in %s" % (install_name, done, os.path.join(installs[install_name]["root"], "mge-backup-" + tag)))
    for label, digest in digests:
        say("md5 %s %s" % (label, digest))
    if keep:
        with open(os.path.join(sets, name, "WHAT.txt"), "w", encoding="ascii", errors="backslashreplace") as file:
            file.write(what_line(name, plugin_commit, mge_commit, digests, list(installs)))
    pdbs = [path for install in installs.values() for mod in ("mge", "nirender") for path in files_below(mod_folder(install, mod)) if path.lower().endswith(".pdb")]
    say("pdb in the deployed mods: %d" % len(pdbs))
    if add_symbols and build is not None:
        mge_tree = os.path.dirname(os.path.dirname(build))
        say(add_symbols(name or tag, [os.path.join(build, *path.split("/")) for path in SYMBOL_BINARIES],
                        symbols.tree_pdb_dirs(mge_tree, REPO)))
    say("DONE: %d installs from %s with tag %s" % (len(installs), " and ".join(part for part in (build, mwse_dll) if part), tag))
    return 0


# --- the tests ---------------------------------------------------------------------------

def write(path, text="x"):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="ascii") as file:
        file.write(text)


def fake_build(folder, text="v1"):
    """A build in the layout of target/dist-staging, with the files that a deploy must leave out."""
    for path in ("d3d8.dll", "dinput8.dll", "numidium.exe", "MGEXEgui.exe", "MGE XE Readme.md", "MGE XE Renderer Readme.md",
                 "mge3/MGE XE Default Statics Classifiers.toml", "Data Files/meshes/f/a.nif", "Data Files/textures/MGE/b.dds",
                 "Data Files/mwse/mods/MGE XE Options/main.lua", "Data Files/mwse/lib/akulakhan.dll",
                 "Data Files/mwse/mods/nirender/main.lua", "Data Files/mwse/mods/nirender/failure.lua",
                 "Data Files/shaders/core/x.fx", "Data Files/shaders/core/y.fx"):
        write(os.path.join(folder, *path.split("/")), text + path)
    for path in ("numidium.pdb", "d3d8.pdb", "d3d9.dll", "mgeXE.toml", "mge3/mgeXE.toml", "Data Files/XE Sky Variations.esp",
                 "Data Files/meshes/f/b.pdb", "Data Files/docs/a.txt", "Data Files/docs/b.txt"):
        write(os.path.join(folder, *path.split("/")), "no " + path)
    # The build ids of the three parts, as one build of FAKE_HEADS.
    write(os.path.join(folder, "d3d8.dll"), text + " MGE-XE G7 Fork v0.21.0 %s" % FAKE_HEADS["mge"])
    write(os.path.join(folder, "numidium.exe"), text + " nirender-host v0.1.0 %s numidium v0.21.0 %s" % (FAKE_HEADS["plugin"], FAKE_HEADS["mge"]))
    write(os.path.join(folder, *NIRENDER_PLUGIN.split("/")), text + " nirender v0.3.6 %s" % FAKE_HEADS["plugin"])


# The commits under test in the tests: `run(build, installs, FAKE_HEADS["plugin"], FAKE_HEADS["mge"], ...)`.
FAKE_HEADS = {"plugin": "79c752d", "mge": "c610f3b"}


def fake_install(root, mge_mod="MGE X", old=None):
    for mod in (mge_mod, "nirender"):
        os.makedirs(os.path.join(root, "mods", mod), exist_ok=True)
    if old:
        write(os.path.join(root, "mods", mge_mod, "Root", "d3d8.dll"), old)
        write(os.path.join(root, "mods", mge_mod, "Root", "d3d9.dll"), "the install's own")
        write(os.path.join(root, "mods", mge_mod, "Root", "mgeXE.toml"), "the user's settings")
        write(os.path.join(root, "mods", "nirender", "MWSE", "lib", "akulakhan.dll"), old)
        write(os.path.join(root, "mods", mge_mod, "shaders", "core", "x.fx"), old)
        write(os.path.join(root, "mods", mge_mod, "shaders", "water", "tw_lava.wgsl"), "the mod's own")
    return {"root": root, "mge_mod": mge_mod, "nirender_mod": "nirender"}


def tree_of(folder):
    return {path: md5_of(os.path.join(folder, *path.split("/"))) for path in files_below(folder)} if os.path.isdir(folder) else {}


def self_test():
    quiet = lambda: ""  # noqa: E731 - a tasklist with no program
    with tempfile.TemporaryDirectory(prefix="deploy-rest-test-") as root:
        build = os.path.join(root, "build")
        fake_build(build)
        staged = files_below(build)

        # The plan: what goes where, and what never goes.
        items, left_out = plan_of(staged)
        dests = {(item.mod, item.dest) for item in items}
        assert ("mge", "Root/d3d8.dll") in dests and ("mge", "Root/MGE XE Readme.md") in dests, dests
        assert ("mge", "Root/mge3/MGE XE Default Statics Classifiers.toml") in dests, dests
        assert ("mge", "meshes/f/a.nif") in dests and ("mge", "textures/MGE/b.dds") in dests and ("mge", "mwse/mods/MGE XE Options/main.lua") in dests, dests
        assert ("nirender", "MWSE/lib/akulakhan.dll") in dests and ("nirender", "MWSE/mods/nirender/failure.lua") in dests, dests
        assert ("mge", "shaders/core/x.fx") in dests and ("mge", "shaders/core/y.fx") in dests, dests
        assert len(items) == 15 and all(not withheld(item.dest) for item in items), (len(items), dests)
        for never in ("numidium.pdb", "d3d9.dll", "mgeXE.toml", "mge3/mgeXE.toml", "Data Files/XE Sky Variations.esp", "Data Files/meshes/f/b.pdb"):
            assert never not in {item.source for item in items}, never
            assert never in left_out, never
        assert "Data Files/docs (2)" in left_out_summary(left_out) and "shaders" not in left_out_summary(left_out), left_out_summary(left_out)
        assert all(withheld(path) for path in ("a/B.PDB", "x.esp", "x.ESM", "mgeXE.toml", "C:\\x\\d3d9.dll")) and not withheld("d3d8.dll")

        # The programs: a game, numidium and Mod Organizer stop the deploy; others do not.
        listing = '"System","4","Services","0","100 K"\n"Morrowind.exe","12","Console","1","900 K"\n"ModOrganizer.exe","13","Console","1","90 K"\n"numidium.exe","14","Console","1","9 K"\n"cargo.exe","15","Console","1","9 K"\n'
        assert busy_programs(listing) == ["ModOrganizer.exe", "Morrowind.exe", "numidium.exe"], busy_programs(listing)
        assert busy_programs('"System","4","Services","0","100 K"\n"explorer.exe","5","Console","1","1 K"\n') == []

        # A deploy: new files, replaced files with one backup, the files that are not ours stay.
        installs = {"A": fake_install(os.path.join(root, "A"), "MGE A", old="old"), "B": fake_install(os.path.join(root, "B"), "MGE B")}
        counts = deploy(build, installs, "t1", items)
        assert counts == {"A": [15, 0], "B": [15, 0]}, counts
        for name, install in installs.items():
            for item in items:
                assert md5_of(os.path.join(mod_folder(install, item.mod), *item.dest.split("/"))) == md5_of(os.path.join(build, *item.source.split("/"))), (name, item)
        a_root = installs["A"]["root"]
        assert open(os.path.join(a_root, "mods", "MGE A", "Root", "d3d9.dll")).read() == "the install's own"
        assert open(os.path.join(a_root, "mods", "MGE A", "Root", "mgeXE.toml")).read() == "the user's settings"
        assert open(os.path.join(a_root, "mge-backup-t1", "MGE A", "Root", "d3d8.dll")).read() == "old"
        assert open(os.path.join(a_root, "mge-backup-t1", "nirender", "MWSE", "lib", "akulakhan.dll")).read() == "old"
        # The shaders: a file of the build replaces the mod's, with a backup; a file only in the mod stays and is named.
        assert open(os.path.join(a_root, "mge-backup-t1", "MGE A", "shaders", "core", "x.fx")).read() == "old"
        assert open(os.path.join(a_root, "mods", "MGE A", "shaders", "water", "tw_lava.wgsl")).read() == "the mod's own"
        assert only_in_mod(installs["A"], items) == ["shaders/water/tw_lava.wgsl"], only_in_mod(installs["A"], items)
        assert only_in_mod(installs["B"], items) == []
        b_root = installs["B"]["root"]
        assert not os.path.isdir(os.path.join(b_root, "mge-backup-t1")), "a new file has no backup"
        assert not [path for path in files_below(b_root) if withheld(path)], "a withheld file arrived in an install that had none"

        # A second run with the same tag and a changed build: the first backup stays; an unchanged file is not copied.
        fake_build(build, "v2")
        write(os.path.join(build, "d3d8.dll"), "v2d3d8")
        again = deploy(build, installs, "t1", items)
        assert again["A"][0] == 15, again
        assert open(os.path.join(a_root, "mge-backup-t1", "MGE A", "Root", "d3d8.dll")).read() == "old", "the backup of the first run stays"
        assert deploy(build, installs, "t1", items) == {"A": [0, 15], "B": [0, 15]}, "an equal file is left as it is"
        assert open(os.path.join(a_root, "mods", "MGE A", "Root", "d3d8.dll")).read() == "v2d3d8"
        # A new tag backs up what the last tag deployed.
        write(os.path.join(build, "d3d8.dll"), "v3d3d8")
        deploy(build, installs, "t2", items)
        assert open(os.path.join(a_root, "mge-backup-t2", "MGE A", "Root", "d3d8.dll")).read() == "v2d3d8"

        # A copy that does not equal its source is a fault.
        def bad_copy(source, dest):
            shutil.copyfile(source, dest)
            with open(dest, "ab") as file:
                file.write(b"!")
        write(os.path.join(build, "d3d8.dll"), "v4d3d8")
        try:
            put(build, [item for item in items if item.dest == "Root/d3d8.dll"][0], installs["B"], "t3", bad_copy)
            raise AssertionError("an unequal copy passed")
        except DeployError as error:
            assert "MD5 MISMATCH" in str(error), error

        # A refusal writes nothing, and a fault before the first write writes nothing.
        fake_build(build, "v5")
        fresh = {"C": fake_install(os.path.join(root, "C"), "MGE C", old="old")}
        before = tree_of(fresh["C"]["root"])
        try:
            run(build, fresh, FAKE_HEADS["plugin"], FAKE_HEADS["mge"], tag="r", keep=False, tasklist=lambda: listing)
            raise AssertionError("a running game did not stop the deploy")
        except Refused as error:
            assert str(error).startswith("REFUSED: ") and "Morrowind.exe" in str(error), error
        assert tree_of(fresh["C"]["root"]) == before, "a refused deploy wrote"
        os.remove(os.path.join(build, "numidium.exe"))
        try:
            run(build, fresh, FAKE_HEADS["plugin"], FAKE_HEADS["mge"], tag="r", keep=False, tasklist=quiet)
            raise AssertionError("a build without numidium.exe was deployed")
        except DeployError as error:
            assert "numidium.exe" in str(error), error
        assert tree_of(fresh["C"]["root"]) == before, "a fault wrote"
        fake_build(build, "v6")
        # Parts of two builds: the plugin of an older commit (the fear of the merge step of 79c752d). The deploy and its dry
        # run refuse before any write, and name the part.
        write(os.path.join(build, *NIRENDER_PLUGIN.split("/")), "v6 nirender v0.3.6 87f7049")
        for dry in (False, True):
            try:
                run(build, fresh, FAKE_HEADS["plugin"], FAKE_HEADS["mge"], tag="r", keep=False, dry_run=dry, tasklist=quiet)
                raise AssertionError("a build of mismatched parts was deployed (dry run %s)" % dry)
            except DeployError as error:
                assert "not one build" in str(error) and "nirender is 87f7049, but nirender-host is 79c752d" in str(error), error
        assert tree_of(fresh["C"]["root"]) == before, "a mismatched build wrote"
        fake_build(build, "v6")
        # Parts of one build, but not of the commit under test (a merge step of 319e3c6 with a build of 79c752d): refused.
        try:
            run(build, fresh, "319e3c6", FAKE_HEADS["mge"], tag="r", keep=False, dry_run=True, tasklist=quiet)
            raise AssertionError("a build of another commit than the one under test passed")
        except DeployError as error:
            assert "the commit of nirender-plugin under test is 319e3c6" in str(error), error
        no_mod ={"D": {"root": os.path.join(root, "D"), "mge_mod": "MGE D", "nirender_mod": "nirender"}}
        try:
            check_installs(no_mod)
            raise AssertionError("a missing mod folder passed")
        except DeployError as error:
            assert "makes none" in str(error), error
        assert not os.path.exists(os.path.join(root, "D")), "a mod folder was made"

        # The dry run writes nothing.
        before = tree_of(root)
        assert run(build, fresh, FAKE_HEADS["plugin"], FAKE_HEADS["mge"], tag="r", keep=False, dry_run=True, tasklist=quiet) == 0
        assert tree_of(root) == before, "the dry run wrote"

        # A whole deploy with a kept set, into two installs: WHAT.txt goes into the set folder, not into a folder named
        # after the last install (the merge step of 79c752d: 'mgebuilds\\Outlander-test\\WHAT.txt').
        whole_sets = os.path.join(root, "whole-sets")
        two = {"E": fake_install(os.path.join(root, "E"), "MGE E"), "F": fake_install(os.path.join(root, "F"), "MGE F")}
        # With no commits given, the parts' own commits name the build.
        # The symbol store gets the set's name, the 4 binaries of the build and the pdb folders of the two trees.
        added = []
        assert run(build, two, keep=True, sets=whole_sets, tasklist=quiet, add_symbols=lambda *call: added.append(call) or "symbols: +4") == 0
        assert len(added) == 1 and added[0][0] == "f1", added
        assert [os.path.relpath(path, build).replace(os.sep, "/") for path in added[0][1]] == ["d3d8.dll", "dinput8.dll", "numidium.exe", NIRENDER_PLUGIN], added
        assert os.path.join(os.path.dirname(os.path.dirname(build)), "target", "x86_64-pc-windows-msvc", "release") in added[0][2], added
        assert sorted(os.listdir(whole_sets)) == ["f1"], os.listdir(whole_sets)
        what = open(os.path.join(whole_sets, "f1", "WHAT.txt"), encoding="ascii").read()
        assert what.startswith("f1 ") and "nirender 79c752d, render-link c610f3b" in what and "deployed to E, F" in what, what
        assert os.path.isfile(os.path.join(whole_sets, "f1", "akulakhan-79c752d.dll"))

        # The kept copy: the next number as a number, no pdb, the plugin beside it, one ASCII line.
        sets = os.path.join(root, "sets")
        for existing in ("f9", "f100", "notes"):
            os.makedirs(os.path.join(sets, existing))
        first = new_set_folder(sets)
        second = new_set_folder(sets)
        assert (first, second) == ("f101", "f102"), (first, second)
        write(os.path.join(build, "extra.pdb"), "debug")
        keep_build(build, os.path.join(sets, first), "abcdef0")
        kept = files_below(os.path.join(sets, first))
        assert "akulakhan-abcdef0.dll" in kept and "d3d8.dll" in kept and not any(path.endswith(".pdb") for path in kept), kept
        line = what_line(first, "abcdef0", "1234567", [("d3d8.dll", "00" * 16)], ["A", "B"], "2026-11-10 09:00")
        assert line.endswith("\n") and line.count("\n") == 1 and line.isascii() and "backups mge-backup-f101" in line and "no pdb" in line, line

        mwse_test(root, build, quiet)
    print("self-test: all passed")


def mwse_test(root, build, quiet):
    """--mwse-dll (W406): the probe and the commit of the tree refuse before any write, a good DLL goes in after the build
    under the same tag, and an MWSE.dll alone goes beside the plugin that the installs hold."""
    def good(dll):
        return 0, "feature: count locks: present (40 of 40 count lock sites in the data of MWSE.dll)\nmd5: x %s\n" % dll

    def missing(dll):
        return MISSING_FEATURE_EXIT, "MISSING: count locks: 0 of 40 count lock sites in the data of MWSE.dll, missing 0x6fe39f, ...: the plugin needs it\nmd5: y %s\n" % dll

    def broken(dll):
        return 1, "error[E0425]: cannot find value `x`\nmwse features: FAILED\n"

    def at_plugin():
        return FAKE_HEADS["plugin"] + "0" * 33

    def refused(expected, **options):
        """Runs a deploy that must raise `expected` (and not MissingFeature when `expected` is DeployError); its message."""
        try:
            run(**options)
        except DeployError as error:
            assert isinstance(error, expected) and (expected is MissingFeature or not isinstance(error, MissingFeature)), (expected, error)
            return str(error)
        raise AssertionError("the deploy passed: %s" % options)

    mwse = os.path.join(root, "new-MWSE.dll")
    write(mwse, "new mwse")
    pair = {"G": fake_install(os.path.join(root, "G"), "MGE G", old="old"), "H": fake_install(os.path.join(root, "H"), "MGE H", old="old")}
    deploys = {name: os.path.join(install["root"], "mods", "MWSE", "Root", "MWSE.dll") for name, install in pair.items()}
    for path in deploys.values():
        write(path, "old mwse")
    fake_build(build, "v7")
    sets = os.path.join(root, "mwse-sets")
    joint = dict(build=build, installs=pair, plugin_commit=FAKE_HEADS["plugin"], mge_commit=FAKE_HEADS["mge"], tag=None, keep=True, sets=sets,
                 tasklist=quiet, mwse_dll=mwse, mwse_deploys=deploys, tree_commit=at_plugin)
    state = lambda: {name: tree_of(install["root"]) for name, install in pair.items()}  # noqa: E731
    before = state()

    # A DLL that the probe refuses: nothing is written, the deploy and its dry run alike. A missing feature is
    # MissingFeature (exit code 4); a probe that could not judge is a plain fault (exit code 1).
    for dry in (False, True):
        text = refused(MissingFeature, **joint, dry_run=dry, probe=missing)
        assert text.startswith("MISSING FEATURE: ") and "count locks: 0 of 40" in text and "the plugin %s needs" % FAKE_HEADS["plugin"] in text, text
        assert "exit code 1" in refused(DeployError, **joint, dry_run=dry, probe=broken)
    # The probe is the code of the tree: a tree at another commit, or with a change in src/, refuses before the probe.
    asked = []
    for tree in ("1234567", at_plugin() + "-dirty"):
        text = refused(DeployError, **dict(joint, tree_commit=lambda tree=tree: tree), probe=lambda dll: asked.append(dll) or good(dll))
        assert "run this in a tree at %s" % FAKE_HEADS["plugin"] in text, text
    assert asked == [], "the probe ran in a tree of another commit"
    # An install with no place, or no folder, for its MWSE.dll.
    assert "no place of the MWSE.dll of H" in refused(DeployError, **dict(joint, mwse_deploys={"G": deploys["G"]}), probe=good)
    assert "has no folder" in refused(DeployError, **dict(joint, mwse_deploys=dict(deploys, H=os.path.join(root, "none", "MWSE.dll"))), probe=good)
    assert "no file" in refused(DeployError, **dict(joint, mwse_dll=os.path.join(root, "none.dll")), probe=good)
    assert state() == before and not os.path.exists(sets), "a refused deploy wrote"

    # The dry run of a good DLL writes nothing.
    assert run(**joint, dry_run=True, probe=good) == 0
    assert state() == before and not os.path.exists(sets), "the dry run wrote"

    # A good DLL: the build and the MWSE.dll in both installs, both backed up under one tag, WHAT.txt with the MWSE md5.
    assert run(**joint, probe=good) == 0
    for name, install in pair.items():
        assert md5_of(deploys[name]) == md5_of(mwse), name
        assert open(os.path.join(install["root"], "mge-backup-f1", "MWSE", "Root", "MWSE.dll")).read() == "old mwse", name
        assert open(os.path.join(install["root"], "mge-backup-f1", "MGE " + name, "Root", "d3d8.dll")).read() == "old", name
        assert md5_of(os.path.join(mod_folder(install, "nirender"), "MWSE", "lib", "akulakhan.dll")) == md5_of(os.path.join(build, *NIRENDER_PLUGIN.split("/")))
    what = open(os.path.join(sets, "f1", "WHAT.txt"), encoding="ascii").read()
    assert "MWSE.dll %s" % md5_of(mwse) in what, what
    assert not [path for path in files_below(sets) if path.lower().endswith(".pdb")]

    # An MWSE.dll alone: it goes beside the plugin that both installs hold (79c752d now), after the same checks.
    alone = dict(build=None, installs=pair, tag="m", keep=True, sets=sets, tasklist=quiet, mwse_dll=mwse, mwse_deploys=deploys, tree_commit=at_plugin)
    newer = os.path.join(root, "newer-MWSE.dll")
    write(newer, "newer mwse")
    before = state()
    assert "MISSING FEATURE: " in refused(MissingFeature, **dict(alone, mwse_dll=newer), probe=missing)
    assert "run this in a tree at %s" % FAKE_HEADS["plugin"] in refused(DeployError, **dict(alone, mwse_dll=newer, tree_commit=lambda: "87f7049"), probe=good)
    assert state() == before, "a refused MWSE deploy wrote"
    assert run(**dict(alone, mwse_dll=newer), probe=good) == 0
    for name, install in pair.items():
        assert md5_of(deploys[name]) == md5_of(newer)
        assert open(os.path.join(install["root"], "mge-backup-m", "MWSE", "Root", "MWSE.dll")).read() == "new mwse", name
    assert sorted(os.listdir(sets)) == ["f1"], "an MWSE.dll alone keeps no set"
    # Installs that hold plugins of two commits, or a plugin with no build id: refused.
    write(os.path.join(mod_folder(pair["H"], "nirender"), "MWSE", "lib", "akulakhan.dll"), "x nirender v0.3.6 87f7049")
    assert "plugins of different commits" in refused(DeployError, **alone, probe=good)
    write(os.path.join(mod_folder(pair["H"], "nirender"), "MWSE", "lib", "akulakhan.dll"), "no id")
    assert "no build id of nirender" in refused(DeployError, **alone, probe=good)
    assert "nothing to deploy" in refused(DeployError, **dict(alone, mwse_dll=None), probe=good)
    assert os.path.basename(mwse_backup({"root": root}, os.path.join(os.path.dirname(root), "x", "MWSE.dll"), "t")) == "MWSE.dll"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--build", help="the folder target/dist-staging of render-link")
    parser.add_argument("--plugin-commit", help="the commit of nirender-plugin that the plugin and the host crate must be (the merge step's)")
    parser.add_argument("--mge-commit", help="the commit of render-link that numidium and d3d8.dll must be (the merge step's)")
    parser.add_argument("--tag", help="the tag of the backups; by default the name of the kept set, else a time stamp")
    parser.add_argument("--no-keep", action="store_true", help="keep no copy in %s" % SETS)
    parser.add_argument("--dry-run", action="store_true", help="print what a deploy would copy; write nothing")
    parser.add_argument("--sets", default=SETS, help="the folder of the kept sets (a test changes it)")
    parser.add_argument("--mwse-dll", help="an MWSE.dll that goes into each install too, after its probe; without --build, beside the installed plugin")
    parser.add_argument("--self-test", action="store_true", help="the tests: scratch installs, no game")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.build and not args.mwse_dll:
        parser.error("it needs --build, --mwse-dll or both")
    try:
        return run(os.path.abspath(args.build) if args.build else None, REST_INSTALLS, args.plugin_commit, args.mge_commit, args.tag, not args.no_keep,
                   args.dry_run, args.sets, add_symbols=symbols.add, mwse_dll=os.path.abspath(args.mwse_dll) if args.mwse_dll else None)
    except Refused as error:
        say(str(error))
        return REFUSED_EXIT
    except MissingFeature as error:
        say(str(error))
        return MISSING_FEATURE_EXIT
    except DeployError as error:
        say("FAIL: %s" % error)
        return FAULT_EXIT


if __name__ == "__main__":
    sys.exit(main())
