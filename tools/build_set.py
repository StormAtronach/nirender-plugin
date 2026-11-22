"""Makes a sealed build set (W521, the owner's D490): one folder with every binary of a run, from three commits.

    python -I tools/build_set.py [--nirender C] [--render-link C] [--mwse C] [--patch REPO=FILE] [--no-wait]
    python -I tools/build_set.py --nirender C --render-link C --mwse C --in-hold LABEL   # a stage of a merge round
    python -I tools/build_set.py --self-test    # scratch folders and fake builds: no build, no slot, no write to E:

The commits default to the tips: nirender `master`, render-link `feature/wgpu-renderer`, and the MWSE commit that the
test installs run (the records of deploy_rest.py, `mwse_tree.installed_commit`). A commit is any name that git resolves
in its repository. The form of a set, its name, its parts and its check are in sets.py.

Each of the seven parts is made one time for its key. A part that a sealed set in E:/nirender-keep/mgebuilds has already,
with the same key and kind, is copied from that set (its check is the proof), and no new build of it is used: so two
sets that differ in one commit differ only in the parts of that commit, byte for byte. Else the tool builds: the plugin
(`cargo build --release`) and `cargo xtask dist` in clean detached trees in C:/tmp/buildset (siblings, as the dist
needs), and MWSE.dll with MSBuild (Release, x86) in a detached worktree of the MWSE repository there, its LuaJIT built
by build_luajit.bat of the docs and its MorrowindDir a folder of C:/tmp/buildset. The MWSE core Lua comes from that
tree. lua51.dll, the luajit part, goes into the set's root beside MWSE.dll; the build parameter of the tree's
SharedSE/LuaJIT.targets must be the bat's, and the bat's md5 goes into the part's record. Each part's record also names
the tools of its build (rustc and cargo, MSBuild, cl's version line); a copied part carries its set's record. The
record is in no key and no name (the design's 11.7). The part plugin-test is the nirender tree's mod/ folder less the
Lua that the release ships, below plugin-test/ of the set: the test Lua, meshes and textures that a run stages beside
the plugin, of the plugin's commit. A tree with changes makes no set.

--patch REPO=FILE (REPO nirender, render-link or mwse) applies ONE patch file to the clean tree of that repo before the
build. The set holds the patch as patches/<repo>.patch, the manifest has its md5, and the name has the mark +p<8 hex>
after that commit; a part of a patched repo is never copied to or from a set without that patch. A second --patch for
one repo is refused: make a commit. After the build, `git apply -R` takes the patch out of the tree again.

Before any build, the tool checks that the commits' trees carry the proofs that the seal needs (PROOFS: the build ids
of both repos, the probe's --features and --mge), read from the trees' content; an older commit stops with one line
that names the first proof it lacks and the merge that brought it, and no flag passes a proof (the design's 11.3).
Before it writes to E:, the tool checks the build ids (the plugin and the host crate name the nirender commit,
numidium.exe and d3d8.dll the render-link commit; "-dirty" only for a repo with a patch) and runs the probe that
deploy_rest.py runs: the plugin's needs against the set's MWSE.dll, tools/check_mwse_sites.py --features --mge of the
trees at the set's commits. Its exit code 4 makes no set. The pdbs of the binaries go to E:/nirender-keep/pdb/<name>,
each only when it is the pdb of its binary (symbols.pdb_matches); a copied part takes its pdbs from the pdb folder of the
set that it comes from.

The set is made in <name>.making and renamed at the end, so a folder with the name is whole or is not there. If the
name is there, its check passes and its manifest has the request's full commits, patch md5s and kind, nothing is built;
with other full commits (the name holds only 8 hex of each), the tool stops with exit code 1 and names both. If the
name is there and its check fails, the tool stops and names the folder: it never builds a second time into a name. The
first file of <name>.making is the marker MAKING.json (the tool's commit, the session, the process id, the time); it is
removed before the manifest is written. A later call that finds <name>.making with a marker whose process is gone
renames it to <name>.failed-<time> and makes the set; with no marker, or with a process that lives, it stops. It
deletes nothing (an unfinished folder goes by the delete right of D492, or by the owner). The tool takes the build slot before its
first write to E: and keeps it to the rename of the set, noting its work and renewing its lease each minute. A hold can
lapse after 15 idle minutes, so the tool asks the E: gate (`slot.py e-ok`, D297; the game holder is read first, see
sets.e_gate) again before each phase of writes to E: (each build step of a tree on E:, the set's folder, the pdbs, the
manifest and the rename). A gate that says no, or a renew that the slot tool refuses, stops it with exit code 3 before
that phase writes. One run of the tool at a time on this machine:
make() itself takes the run lock C:/tmp/buildset/build_set.lock, also when another tool calls it, and stops with exit
code 3 while another run holds it.

--in-hold LABEL is for a stage of a merge round (merge_step.py), which runs under one take of the game and the whole
build side: a take of its own would wait for that round to end. With it the tool takes no slot. It checks that this
session holds every place of the build side as LABEL, else exit code 3 and nothing is built; then it notes the build
work and renews that hold each minute, as with its own hold, and a refused renew stops it before the next phase of
writes to E:.

The last line is the set's folder. Exit code 0; 1 for a fault; 3 when a slot or the E: gate refuses; 4 when the probe
finds a missing MWSE feature.
"""
import argparse
import contextlib
import datetime
import json
import msvcrt
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import types

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import sets  # noqa: E402
from common import BUILD_ID_RE, REPO, docs_place, git_env, local, main_checkout, same_commit  # noqa: E402
from check_mwse_sites import MISSING, MISSING_FEATURE_EXIT  # noqa: E402
from deploy_rest import MWSE_REPO  # noqa: E402
from make_play_set import MGE_BRANCH, PDBS, PDB_KEEP, PLUGIN_BRANCH, hold_run_lock, inside, say  # noqa: E402
from sets import e_gate, this_session  # noqa: E402
import symbols  # noqa: E402

TREES = "C:/tmp/buildset"
# The tool's run lock: one make() at a time on this machine, whoever calls it (main, make_play_set.py, merge_step.py).
RUN_LOCK = os.path.join(TREES, "build_set.lock")
TREE_NAMES = {"nirender": "nirender-plugin", "render-link": "MGE-XE-G7-render-link", "mwse": "MWSE"}
# The repositories that the commits come from: beside the main checkout (as deploy_rest.MWSE_REPO), so that a run from
# a worktree finds them; the key "repos" of local.json (make_play_set.REPOS) when it is set.
SOURCE_ROOT = local("repos", os.path.dirname(main_checkout() or REPO))
SOURCE_REPOS = {"nirender": os.path.join(SOURCE_ROOT, "nirender-plugin"), "render-link": os.path.join(SOURCE_ROOT, "MGE-XE-G7-render-link"), "mwse": MWSE_REPO}
LUAJIT_BAT = docs_place("docs", "scripts", "mwse-build", "build_luajit.bat")
MSBUILD = "C:/Program Files/Microsoft Visual Studio/2022/Community/MSBuild/Current/Bin/MSBuild.exe"
PLUGIN_TARGET = os.path.join("target", "i686-pc-windows-msvc", "release", "akulakhan.dll")
MWSE_BUILT = os.path.join("build", "Release")
MWSE_CORE = os.path.join("misc", "package", "Data Files", "MWSE", "core")
SET_CORE = "Data Files/MWSE/core"
# The binaries whose build ids name the commits of the set: (path in the set, program, repo).
BUILD_IDS = (
    (sets.PLUGIN_STAGED, "nirender", "nirender"),
    ("numidium.exe", "nirender-host", "nirender"),
    ("numidium.exe", "numidium", "render-link"),
    ("d3d8.dll", "MGE-XE G7 Fork", "render-link"),
)
# The file that each part must hold.
REQUIRED = {"plugin": sets.PLUGIN_STAGED, "plugin-lua": "Data Files/mwse/mods/nirender/main.lua", "render-link": "d3d8.dll",
            "numidium": "numidium.exe", "mwse": "MWSE.dll", "luajit": "lua51.dll",
            # plugin-test: at least one file, none named.
            "plugin-test": None}
# The folder of the nirender Lua below mod/ of the tree, and in a set's Data Files (the dist's place, plugin-lua).
MOD_LUA = "MWSE/mods/nirender/"
SET_LUA = "data files/mwse/mods/nirender/"
# LuaJIT in the MWSE tree: the build parameter's file, and the outputs of msvcbuild.bat.
LUAJIT_TARGETS = "SharedSE/LuaJIT.targets"
LUAJIT_OUT = os.path.join("deps", "rubic0n", "src")
# The parts that come out of the cargo builds and the dist.
CARGO_PARTS = {"plugin", "plugin-lua", "render-link", "numidium"}
# The pdbs of the MWSE part: (path in the set, the pdb in the tree).
MWSE_PDBS = (("MWSE.dll", os.path.join(MWSE_BUILT, "MWSE.pdb")),)
NOTE_SECONDS = 60
# The least free space on the disk of the trees before each build step (the Coordinator, 2026-11-16: three cold trees
# can take C: from 35 GB to under 20).
LEAST_FREE_BYTES = 20 * 10**9
EXIT_FAULT = 1
EXIT_REFUSED = 3
# The proofs that the seal needs from a commit's tree (the design's 11.3, the plan's addendum 1 A6): (repo, path, the
# exact text in it or None for the file alone, what it proves, the merge that brought it). They are read from the tree's
# content, not by ancestry, so a branch with the proofs picked onto an old commit passes. Each was checked: missing at
# the parent of its commit, there at the commit (824e51c, 95cd1f9, c0f6286, 4bc9a9c).
PROOFS = (
    ("nirender", "build_support/build_id.rs", None, "the build ids of the plugin and the host crate", "8c4a52f"),
    ("render-link", "build_support/build_id.rs", None, "the build ids of d3d8.dll and numidium.exe", "97b999d"),
    ("nirender", "tools/check_mwse_sites.py", 'add_argument("--features"', "the probe's --features", "bceceff"),
    ("nirender", "tools/check_mwse_sites.py", 'add_argument("--mge"', "the probe's --mge", "5171d71"),
)
# The first file of `<name>.making` (the design's 11.5): the tool's commit, the session, the process id and the time.
# It is removed before the manifest is written, so a set never holds it.
MARKER = "MAKING.json"
# The Reviewer's point B (verdict 452): no real run has yet built a plain and a patched set of one commit in both
# orders. A checkout of the same commit does not write HEAD, and the build ids are made again only on a change of HEAD,
# so [I] the build id of a binary can miss its patch, or show "-dirty" without one.
PATCH_UNVERIFIED = ("UNVERIFIED: --patch (verdict 452 B): no plain and patched pair of one commit is built in both orders "
                    "yet; the build ids of this set's binaries may not show its patch")
# Variables of the caller's environment that would make a build read or write outside its tree: another cargo target,
# and another plugin folder for the dist (d3d8/build.rs and xtask's plugin_dir read NIRENDER_PLUGIN_DIR).
OUTSIDE_TREE_VARIABLES = ("CARGO_TARGET_DIR", "NIRENDER_PLUGIN_DIR")


def build_environment(env=None):
    """A copy of `env` (default: this process's) without OUTSIDE_TREE_VARIABLES, for the builds and the probe."""
    env = dict(os.environ if env is None else env)
    for name in OUTSIDE_TREE_VARIABLES:
        env.pop(name, None)
    return env


class SetFault(Exception):
    """A fault that makes no set; the message is its line. `code` is the exit code."""

    def __init__(self, text, code=EXIT_FAULT):
        super().__init__(text)
        self.code = code


def git(folder, *args):
    # Without the GIT_ names of a hook (common.git_env): under a hook, "git init" and "git config" in the self-test's
    # scratch repo wrote core.bare = true and the scratch user into the config of the hook's repository (W527).
    done = subprocess.run(["git", "-C", folder, *args], capture_output=True, text=True, env=git_env())
    if done.returncode != 0:
        raise SetFault("git %s in %s: %s" % (" ".join(args), folder, done.stderr.strip()))
    return done.stdout.strip()


def git_code(folder, *args):
    """The exit code of a git command whose answer is its code (cat-file -e, grep -q)."""
    return subprocess.run(["git", "-C", folder, *args], capture_output=True, text=True, env=git_env()).returncode


def resolve(repo, name):
    """The full commit that `name` names in `repo`."""
    return git(repo, "rev-parse", "--verify", "--quiet", name + "^{commit}")


def tips():
    """The commits of the tips: the two branches, and the MWSE commit of the test installs."""
    import mwse_tree

    mwse, reason = mwse_tree.installed_commit()
    if mwse is None:
        raise SetFault("no MWSE commit for the tips (%s): name one with --mwse" % reason)
    return {"nirender": PLUGIN_BRANCH, "render-link": MGE_BRANCH, "mwse": mwse}


def read_patches(texts):
    """{repo: (file, md5)} of the --patch arguments REPO=FILE. One patch for each repo at most."""
    patches = {}
    for text in texts:
        repo, _, path = text.partition("=")
        if repo not in sets.REPOS or not path:
            raise SetFault("--patch %s: it is REPO=FILE, with REPO one of %s" % (text, ", ".join(sets.REPOS)))
        if repo in patches:
            raise SetFault("--patch: a second patch for %s; a set takes one patch for each repo: make a commit" % repo)
        if not os.path.isfile(path):
            raise SetFault("--patch %s: no file %s" % (text, path))
        patches[repo] = (os.path.abspath(path), sets.md5_of(path))
    return patches


# W567 part C: the paths of the nirender repo that any part's build reads (the one home of this list). Two nirender
# commits whose diff touches none of them build the same parts but for their build ids, so a set of one serves as the
# base set of a merge round at the other (merge_step.equal_parts_base). The cargo builds read src/, build.rs,
# build_support/ and host/ (their dep-info files, `input_drift`); the dist reads mod/ (plugin-lua, plugin-test) and
# render-link's d3d8 reads contract/; the Cargo files and the toolchain decide every cargo build; d3d8/ is listed so that
# no folder of a build is left out.
NIRENDER_PART_INPUTS = ("src/", "build.rs", "build_support/", "host/", "contract/", "mod/", "d3d8/", "Cargo.toml", "Cargo.lock",
                        ".cargo/", "rust-toolchain.toml", ".gitattributes", ".gitmodules")
# .gitattributes and .gitmodules: a change to them changes the bytes that git checks out (for example of src/gen/code.bin),
# and no dep-info file names them, so maker_input_drift cannot show them (verdict 608).


def is_part_input(path):
    """Whether a path of the nirender repo (with "/") is read by the build of a part (NIRENDER_PART_INPUTS)."""
    return any(path == name or path.startswith(name) if name.endswith("/") else path == name for name in NIRENDER_PART_INPUTS)


def dep_info_paths(text):
    """The files of the first rule of a dep-info file of cargo ("<target>: <file> <file> ..."). A space in a path is
    written "\\ "."""
    first = text.split("\n", 1)[0].rstrip("\r")
    _, colon, files = first.partition(": ")
    if not colon:
        return []
    return [part.replace("\\ ", " ") for part in re.split(r"(?<!\\) ", files) if part]


# The dep-info files of the maker's cargo builds that read nirender (the plugin DLL; numidium with the host crate),
# relative to the maker's trees (TREES, TREE_NAMES).
MAKER_DEP_INFO = (("nirender", os.path.join("target", "i686-pc-windows-msvc", "release", "akulakhan.d")),
                  ("render-link", os.path.join("target", "x86_64-pc-windows-msvc", "release", "numidium.d")))


def maker_input_drift(trees=None):
    """`input_drift` of the maker's own dep-info files (MAKER_DEP_INFO) against its nirender tree: (paths, None), or
    (None, why) when a file is not there, as then the list cannot be checked against the truth."""
    trees = trees or TREES
    texts = []
    for repo, relative in MAKER_DEP_INFO:
        path = os.path.join(trees, TREE_NAMES[repo], relative)
        if not os.path.isfile(path):
            return None, "no dep-info file %s: the maker built no set here yet" % path
        with open(path, encoding="utf-8", errors="replace") as file:
            texts.append(file.read())
    return input_drift(texts, os.path.join(trees, TREE_NAMES["nirender"])), None


def input_drift(dep_texts, tree):
    """The paths of the nirender tree `tree` that the cargo dep-info texts `dep_texts` name and NIRENDER_PART_INPUTS does
    not hold, as sorted paths relative to the tree: a non-empty answer means the list misses an input. Paths outside the
    tree (the registry, the toolchain, the other repo) and under its .git (the build id's commit) are not the list's."""
    root = os.path.normcase(os.path.abspath(tree))
    outside = set()
    for text in dep_texts:
        for path in dep_info_paths(text):
            full = os.path.normcase(os.path.abspath(path))
            if not full.startswith(root + os.sep):
                continue
            relative = os.path.relpath(full, root).replace("\\", "/")
            if relative.startswith(".git/") or relative == ".git":
                continue
            if not is_part_input(relative):
                outside.add(relative)
    return sorted(outside)


def part_keys(commits, tree, patch_md5s):
    """The key of each part (the Architect's rule, W521): its own repo's commit and, for each thing of another repo that
    its build reads, the smallest name that holds it: the git tree of a folder, or the commit for a whole crate. Each
    with its patch mark (sets.py). `tree(repo, folder)` gives the git tree of a folder at the set's commit.

    - plugin (the DLL): the nirender commit; its build reads nothing outside nirender.
    - plugin-lua: the nirender commit, and render-link's xtask/, which picks the nirender Lua files of the dist
      (`collect_plugin`). So an xtask change makes new Lua files and keeps the plugin's bytes.
    - render-link: its commit, and nirender's contract/ (d3d8/build.rs includes mge_render_link.h from it).
    - numidium: the render-link and the nirender commits (the host crate is a path dependency).
    - mwse: the MWSE commit (its submodules are part of it).
    - luajit (lua51.dll): the commit of deps/rubic0n that the MWSE commit names and the git blob of
      SharedSE/LuaJIT.targets, which holds the build parameter (the design's 11.2). Its build reads only those, so two
      MWSE commits with one rubic0n commit and one parameter share one lua51.dll, byte for byte."""
    def marked(repo):
        return commits[repo] + sets.patch_mark(patch_md5s.get(repo))

    def tree_of(repo, folder):
        return tree(repo, folder) + sets.patch_mark(patch_md5s.get(repo))

    return {
        "plugin": {"nirender": marked("nirender")},
        "plugin-lua": {"nirender": marked("nirender"), "render-link xtask": tree_of("render-link", "xtask")},
        "render-link": {"render-link": marked("render-link"), "nirender contract": tree_of("nirender", "contract")},
        "numidium": {"render-link": marked("render-link"), "nirender": marked("nirender")},
        "mwse": {"mwse": marked("mwse")},
        "luajit": {"mwse rubic0n": tree_of("mwse", "deps/rubic0n"), "mwse LuaJIT.targets": tree_of("mwse", LUAJIT_TARGETS)},
        # plugin-test: the rest of mod/ after the shipped Lua, so the same as plugin-lua (the plan's addendum 3).
        "plugin-test": {"nirender": marked("nirender"), "render-link xtask": tree_of("render-link", "xtask")},
    }


def ids_of(path):
    """The build ids in a binary, as {program: commit with its "-dirty"} (one each), and the programs with two."""
    with open(path, "rb") as file:
        found = {(m.group(1).decode(), m.group(2).decode() + (m.group(3) or b"").decode()) for m in BUILD_ID_RE.finditer(file.read())}
    ids = {}
    for program, commit in found:
        ids.setdefault(program, set()).add(commit)
    return ids


def build_id_faults(files, commits, patches):
    """Why the binaries of `files` ({path in the set: source file}) are not builds of `commits`: each must name its
    commit once, and "-dirty" only for a repo with a patch."""
    faults = []
    for path, program, repo in BUILD_IDS:
        source = next((files[name] for name in files if name.lower() == path.lower()), None)
        if source is None:
            faults.append("the set has no %s" % path)
            continue
        named = ids_of(source).get(program, set())
        if len(named) != 1:
            faults.append("%s holds %d build ids of %s" % (path, len(named), program))
            continue
        commit = next(iter(named))
        plain = commit[:-len("-dirty")] if commit.endswith("-dirty") else commit
        if not same_commit(plain, commits[repo]):
            faults.append("%s is a build of %s %s, not of %s" % (path, program, commit, commits[repo][:8]))
        elif commit.endswith("-dirty") and repo not in patches:
            faults.append("%s is a build of a %s tree with changes (%s), and the set has no patch for it" % (path, repo, commit))
    return faults


def plugin_copy_name(plugin_file):
    """akulakhan-<commit>.dll, with the commit of the plugin's build id (W520's name)."""
    commit = next(iter(ids_of(plugin_file).get("nirender", {"unknown"})))
    return "akulakhan-%s.dll" % commit.replace("-dirty", "")


def files_below(folder, skip_pdb=True):
    """{path with "/": file} of the files below `folder`."""
    _, files, links = sets.walk(folder)
    if links:
        raise SetFault("%s holds links or junctions: %s" % (folder, ", ".join(links[:5])))
    return {path: os.path.join(folder, *path.split("/")) for path in files if not (skip_pdb and path.lower().endswith(".pdb"))}


def test_part_files(mod, plugin_lua_paths):
    """{path in the set: source file} of the part plugin-test: the files below the tree's mod folder `mod`, as
    plugin-test/<path below mod/>, less the Lua files that plugin-lua ships (`plugin_lua_paths`, the part's paths in
    the set), so that no file is in both parts once staged."""
    shipped = {path.lower()[len(SET_LUA):] for path in plugin_lua_paths if path.lower().startswith(SET_LUA)}
    files = {}
    for path, source in files_below(mod, skip_pdb=False).items():
        low = path.lower()
        if low.startswith(MOD_LUA.lower()) and low[len(MOD_LUA):] in shipped:
            continue
        files["%s/%s" % (sets.PLUGIN_TEST, path)] = source
    return files


def luajit_parameter_fault(targets_text, bat_text):
    """Why the LuaJIT build parameter of LuaJIT.targets (`<LuaJitBuildParams>`) and of build_luajit.bat (the argument of
    its msvcbuild.bat call) do not agree, or None (the design's 11.2: the maker stops on two values)."""
    in_targets = re.search(r"<LuaJitBuildParams>([^<]*)</LuaJitBuildParams>", targets_text)
    in_bat = re.search(r"msvcbuild\.bat\"?[ \t]+([^\r\n]*)", bat_text, re.IGNORECASE)
    if not in_targets or not in_bat:
        return "the LuaJIT build parameter cannot be read in %s" % ("LuaJIT.targets" if not in_targets else "build_luajit.bat")
    if in_targets.group(1).split() != in_bat.group(1).split():
        return "the LuaJIT build parameter of LuaJIT.targets (%s) is not the one of build_luajit.bat (%s): no set is made" % (
            in_targets.group(1).strip(), in_bat.group(1).strip())
    return None


def built_parts(stage, mwse, luajit=None):
    """{part: {path in the set: source file}} of a new build: the dist stage `stage`, `mwse` (the MWSE.dll and the
    core folder) when MWSE was built, and `luajit` (lua51.dll) when LuaJIT was built."""
    parts = {part: {} for part in sets.PARTS}
    if stage:
        for path, source in files_below(stage).items():
            parts[sets.part_of(path)][path] = source
        staged = next((source for path, source in parts["plugin"].items() if path.lower() == sets.PLUGIN_STAGED.lower()), None)
        if staged:
            parts["plugin"][plugin_copy_name(staged)] = staged
    if mwse:
        dll, core = mwse
        parts["mwse"]["MWSE.dll"] = dll
        for path, source in files_below(core).items():
            parts["mwse"][SET_CORE + "/" + path] = source
    if luajit:
        parts["luajit"]["lua51.dll"] = luajit
    return parts


def set_part_files(root, name, manifest, part):
    """{path in the set: source file} of `part` of the set `name`."""
    return {path: os.path.join(root, name, *path.split("/")) for path in manifest["files"] if sets.part_of(path) == part}


def copy_part(files, folder, expected=None):
    """Copies the files of a part into the new set and compares each copy's md5 with its source (or with `expected`,
    the md5s of the source set's manifest)."""
    for path, source in sorted(files.items()):
        target = os.path.join(folder, *path.split("/"))
        os.makedirs(os.path.dirname(target), exist_ok=True)
        shutil.copyfile(source, target)
        want = expected[path]["md5"] if expected else sets.md5_of(source)
        if sets.md5_of(target) != want:
            raise SetFault("the copy of %s in %s differs from its source %s" % (path, folder, source))


def keep_pdbs(name, folder, sources, keep=PDB_KEEP, root=sets.SETS):
    """The pdbs of the set's binaries into keep/name: `sources` gives, for each binary (path in the set), the pdb file
    that may be its pdb. A pdb is kept only when it is the pdb of its binary. Returns the lines."""
    if inside(keep, root):
        raise SetFault("the folder of the pdbs %s is in the sets %s: no pdb may go into a set" % (keep, root))
    out = os.path.join(keep, name)
    os.makedirs(out, exist_ok=True)
    lines = []
    for binary_path, pdb in sorted(sources.items()):
        binary = os.path.join(folder, *binary_path.split("/"))
        pdb_name = os.path.basename(pdb)
        try:
            with open(binary, "rb") as file:
                record = symbols.codeview(file.read())
            if record is None or record[2].lower() != pdb_name.lower():
                lines.append("%s: not kept: %s has no CodeView record of it" % (pdb_name, binary_path))
            elif not os.path.isfile(pdb) or not symbols.pdb_matches(pdb, record[0], record[1]):
                lines.append("%s: not kept: no pdb of this build of %s at %s" % (pdb_name, binary_path, pdb))
            else:
                shutil.copy2(pdb, os.path.join(out, pdb_name))
                lines.append("%s: %s, GUID %s, age %d" % (pdb_name, binary_path, record[0].hex(), record[1]))
        except OSError as error:
            lines.append("%s: not kept: %s" % (pdb_name, error))
    with open(os.path.join(out, "WHAT.txt"), "w", encoding="ascii", errors="backslashreplace") as file:
        file.write("\n".join(["%s: the pdbs of the set %s/%s. Never deploy them." % (name, root, name)] + lines) + "\n")
    return lines


def tool_version():
    """The commit of this tool's tree, with "-dirty" when its tools/ has changes."""
    try:
        head = git(REPO, "rev-parse", "HEAD")
        dirty = git(REPO, "status", "--porcelain", "--", "tools")
    except SetFault:
        return "unknown"
    return head + ("-dirty" if dirty else "")


# The source of render-link's option --distant-land-key (W521 step 4). A render-link commit without it has a helper
# that cannot give its format, and the set gets a format of its own (design 3.4).
KEY_SOURCE = "mgeHost64/src/distant_land_key.rs"


def distant_land_format(builder, commit, making, run=subprocess.run):
    """(the set's distant land format, its text or None): the 8 hex and the text of the set's helper in `making`
    (`numidium.exe --distant-land-key`) when the render-link `commit` has the option, else own-<8 hex of the commit>
    and None. A helper with the option that gives no key is a SetFault: the set is not made."""
    if not builder.has_file("render-link", commit, KEY_SOURCE):
        return "own-%s" % commit[:8], None
    import distant_land
    with tempfile.TemporaryDirectory(prefix="build-set-key-") as temp:
        toml = os.path.join(temp, "mgeXE.toml")
        with open(toml, "w", encoding="utf-8") as file:
            file.write("")
        try:
            form, text, _ = distant_land.helper_key(os.path.join(making, "numidium.exe"), toml, run)
        except (distant_land.Problem, OSError, subprocess.SubprocessError) as problem:
            raise SetFault("the set's helper gave no distant land key: %s" % problem)
    return form, text


def write_manifest(folder, name, kind, commits, patches, parts_from, keys, now, session, records, form=None):
    folders, files, links = sets.walk(folder)
    if links:
        raise SetFault("the new set holds links or junctions: %s" % ", ".join(links))
    manifest = {
        "form": sets.FORM,
        "name": name,
        "kind": kind,
        "commits": {repo: {"commit": commits[repo], "clean": True} for repo in sets.REPOS},
        "patches": {repo: {"file": "%s/%s.patch" % (sets.PATCHES, repo), "md5": md5} for repo, (_, md5) in sorted(patches.items())},
        "parts": {part: {"key": keys[part], "from": parts_from[part], "record": records[part]} for part in sets.PARTS},
        # The format of the set's helper (distant_land_format), or own-<8 hex> for a helper without the option (3.4).
        "distant_land_format": (form or ("own-%s" % commits["render-link"][:8], None))[0],
        "tool": {"name": "build_set.py", "commit": tool_version()},
        "made": now.isoformat(timespec="seconds"),
        "session": session,
        "folders": folders,
        "files": {path: {"size": os.path.getsize(os.path.join(folder, *path.split("/"))),
                         "md5": sets.md5_of(os.path.join(folder, *path.split("/")))} for path in files if path != sets.MANIFEST},
    }
    if form and form[1]:
        manifest["distant_land_format_text"] = form[1]
    with open(os.path.join(folder, sets.MANIFEST), "w", encoding="utf-8", newline="\n") as file:
        json.dump(manifest, file, indent=1, sort_keys=True)
        file.write("\n")
    return manifest


# --- the builds -------------------------------------------------------------------------------

def update_submodules(tree, source):
    """Checks out the submodules of the worktree `tree` from the modules of the checkout `source` on this machine, not
    from the network. Their URL goes to the one command by `git -c`: a worktree's `git config` would write the config of
    the owner's repository (2026-11-16: an earlier form of this tool changed the URL of deps/rubic0n there)."""
    overrides = ["-c", "protocol.file.allow=always"]
    for line in git(tree, "config", "--file", ".gitmodules", "--get-regexp", r"submodule\..*\.path").splitlines():
        key = line.split(None, 1)[0]
        module = key[len("submodule."):-len(".path")]
        local = os.path.join(source, ".git", "modules", *module.split("/"))
        if not os.path.isdir(local):
            raise SetFault("the checkout %s has no modules of the submodule %s: no set is made" % (source, module))
        overrides += ["-c", "submodule.%s.url=%s" % (module, local.replace("\\", "/")), "-c", "submodule.%s.active=true" % module]
    git(tree, *overrides, "submodule", "update")

class Trees:
    """The real builds, in clean detached trees in `root`."""

    def __init__(self, root=TREES, gate=e_gate, disk_usage=shutil.disk_usage):
        self.root = root
        self.log = os.path.join(root, "build_set.log")
        self.trees = {}
        self.applied = []
        self.gate = gate
        self.disk_usage = disk_usage
        # The SlotHold of the make() that uses these trees (make sets it): a lost hold stops the next step.
        self.slot_hold = None

    def on_project_disk(self):
        """The trees are on the disk of the sets (E:): each build step there is a write to E: (D297)."""
        return os.path.splitdrive(os.path.abspath(self.root))[0].lower() == os.path.splitdrive(os.path.abspath(sets.SETS))[0].lower()

    def room(self, what):
        """Stops before `what` when the build hold was lost, when the disk of the trees has less than LEAST_FREE_BYTES
        free, or when the trees are on E: and the E: gate says no."""
        if self.slot_hold is not None and self.slot_hold.lost:
            raise SetFault("the build slot was lost before %s (%s): nothing more is built or written" % (what, self.slot_hold.lost), EXIT_REFUSED)
        free = self.disk_usage(os.path.splitdrive(os.path.abspath(self.root))[0] + os.sep).free
        if free < LEAST_FREE_BYTES:
            raise SetFault("no room for %s: the disk of %s has %.1f GB free, and the floor is %.0f GB" % (
                what, self.root, free / 1e9, LEAST_FREE_BYTES / 1e9), EXIT_REFUSED)
        if self.on_project_disk():
            allowed, line = self.gate()
            if not allowed:
                raise SetFault("no build on E: now: %s" % line, EXIT_REFUSED)

    def size_lines(self):
        """The bytes and the files of each tree, for the cost of a cold set."""
        lines = []
        for repo, tree in sorted(self.trees.items()):
            total = count = 0
            for base, _, names in os.walk(tree):
                for name in names:
                    with contextlib.suppress(OSError):
                        total += os.path.getsize(os.path.join(base, name))
                        count += 1
            lines.append("tree %s: %s, %d bytes in %d files" % (repo, tree.replace("\\", "/"), total, count))
        return lines

    def ready(self, repo, commit):
        """A detached tree of `repo` at `commit`; a tree with changes stops the set."""
        tree = os.path.join(self.root, TREE_NAMES[repo])
        if not os.path.exists(os.path.join(tree, ".git")):
            self.room("the tree %s" % tree)
            git(SOURCE_REPOS[repo], "worktree", "add", "--detach", tree, commit)
        else:
            # An untracked file is a change too: a build can read it (a stray Lua file of the mod folder goes into the
            # dist). The build outputs are ignored files.
            changed = git(tree, "status", "--porcelain", "--ignore-submodules=untracked")
            if changed:
                raise SetFault("the tree %s has changes, and no set is made:\n%s" % (tree, changed))
            git(tree, "checkout", "--quiet", "--detach", commit)
        if repo == "mwse":
            update_submodules(tree, SOURCE_REPOS["mwse"])
        self.trees[repo] = tree
        return tree

    def patch(self, repo, patch_file):
        git(self.trees[repo], "apply", "--whitespace=nowarn", patch_file)
        self.applied.append((repo, patch_file))

    def unpatch(self):
        while self.applied:
            repo, patch_file = self.applied.pop()
            git(self.trees[repo], "apply", "-R", "--whitespace=nowarn", patch_file)

    def tree(self, repo, commit, folder):
        """The git tree of `folder` of `repo` at `commit`."""
        return git(SOURCE_REPOS[repo], "rev-parse", "%s:%s" % (commit, folder))

    def mod_folder(self):
        """The mod folder of the nirender tree: the source of the part plugin-test."""
        return os.path.join(self.trees["nirender"], "mod")

    def has_file(self, repo, commit, path):
        """True when `path` is a file of `repo` at `commit`."""
        return git_code(SOURCE_REPOS[repo], "cat-file", "-e", "%s:%s" % (commit, path)) == 0

    def has_text(self, repo, commit, path, text):
        """True when the file `path` of `repo` at `commit` holds `text` (fixed, not a pattern)."""
        return git_code(SOURCE_REPOS[repo], "grep", "-q", "-F", text, commit, "--", path) == 0

    def distant_land_format(self, commit, making):
        """The set's distant land format ([`distant_land_format`])."""
        return distant_land_format(self, commit, making)

    def step(self, command, folder, env=None):
        self.room(" ".join(os.path.basename(part) for part in command[:2]))
        env = build_environment(env)
        say("building: %s in %s" % (" ".join(command), folder))
        with open(self.log, "a", encoding="utf-8") as log:
            log.write("\n$ %s\n" % " ".join(command))
            log.flush()
            done = subprocess.run(command, cwd=folder, env=env, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
        if done.returncode != 0:
            raise SetFault("the build failed with exit code %d: see %s" % (done.returncode, self.log))

    def build_cargo(self):
        """The plugin and the dist: returns the dist stage folder."""
        self.step(["cargo", "build", "--release"], self.trees["nirender"])
        self.step(["cargo", "xtask", "dist"], self.trees["render-link"])
        stage = os.path.join(self.trees["render-link"], "target", "dist-staging")
        built = os.path.join(self.trees["nirender"], PLUGIN_TARGET)
        staged = os.path.join(stage, *sets.PLUGIN_STAGED.split("/"))
        if sets.md5_of(built) != sets.md5_of(staged):
            raise SetFault("the plugin of the dist stage is not the plugin of the tree: %s" % staged)
        return stage

    def build_luajit(self):
        """lua51.dll of the MWSE tree, built one time for each rubic0n commit, with the build parameter of the tree's
        LuaJIT.targets, which must be the parameter of build_luajit.bat. Returns the DLL."""
        tree = self.trees["mwse"]
        with open(os.path.join(tree, *LUAJIT_TARGETS.split("/")), encoding="utf-8") as file:
            targets = file.read()
        with open(LUAJIT_BAT, encoding="utf-8", errors="replace") as file:
            bat = file.read()
        fault = luajit_parameter_fault(targets, bat)
        if fault:
            raise SetFault(fault)
        stamp = os.path.join(self.root, "MWSE-luajit.txt")
        luajit = git(tree, "rev-parse", "HEAD:deps/rubic0n")
        built_for = open(stamp, encoding="ascii").read().strip() if os.path.isfile(stamp) else ""
        if built_for != luajit or not os.path.isfile(os.path.join(tree, LUAJIT_OUT, "lua51.lib")):
            self.step(["cmd", "/c", LUAJIT_BAT, os.path.normpath(tree)], tree)
            with open(stamp, "w", encoding="ascii") as file:
                file.write(luajit + "\n")
        return os.path.join(tree, LUAJIT_OUT, "lua51.dll")

    def build_mwse(self):
        """MWSE.dll and its core folder."""
        tree = self.trees["mwse"]
        # MWSE.dll links lua51.lib: LuaJIT first (one time for its rubic0n commit).
        self.build_luajit()
        out = os.path.join(self.root, "MWSE-morrowind")
        os.makedirs(out, exist_ok=True)
        self.step([MSBUILD, "MWSE.sln", "-t:MWSE", "-p:Configuration=Release", "-p:Platform=x86",
                   "-p:MorrowindDir=%s\\" % os.path.normpath(out), "-m", "-nologo", "-verbosity:minimal"], tree)
        return os.path.join(tree, MWSE_BUILT, "MWSE.dll"), os.path.join(tree, MWSE_CORE)

    def pdb_sources(self, parts_built):
        """{binary path in the set: its pdb in a tree} for the binaries of the parts built here."""
        found = {}
        trees = {"mge": self.trees.get("render-link"), "plugin": self.trees.get("nirender")}
        for binary, tree, pdb in PDBS:
            path = binary.replace("\\", "/")
            if sets.part_of(path) in parts_built and trees[tree]:
                found[path] = os.path.join(trees[tree], pdb)
        if "mwse" in parts_built:
            for binary, pdb in MWSE_PDBS:
                found[binary] = os.path.join(self.trees["mwse"], pdb)
        if "luajit" in parts_built:
            found["lua51.dll"] = os.path.join(self.trees["mwse"], LUAJIT_OUT, "lua51.pdb")
        return found

    def tool_line(self, command, folder=None):
        """The first line of a tool's version answer, or "unknown: <why>". One run for each command and folder."""
        cache = self.__dict__.setdefault("tool_lines", {})
        key = (tuple(command), folder)
        if key not in cache:
            try:
                done = subprocess.run(command, cwd=folder, capture_output=True, text=True, errors="replace", timeout=60,
                                      stdin=subprocess.DEVNULL, env=build_environment())
                # stderr first: cl writes its version line there and a usage line on stdout.
                lines = (done.stderr + "\n" + done.stdout).strip().splitlines()
                cache[key] = lines[0].strip() if lines else "unknown: no answer, exit code %d" % done.returncode
            except (OSError, subprocess.SubprocessError) as error:
                cache[key] = "unknown: %s" % error
        return cache[key]

    def cl_line(self):
        """The version line of the cl.exe (x86) of the default MSVC toolset of the Visual Studio of MSBUILD: [I] the one
        that MSBuild and the cc crate take when nothing names another."""
        vs = os.path.normpath(os.path.join(os.path.dirname(MSBUILD), "..", "..", ".."))
        try:
            with open(os.path.join(vs, "VC", "Auxiliary", "Build", "Microsoft.VCToolsVersion.default.txt"), encoding="ascii") as file:
                version = file.read().strip()
        except OSError as error:
            return "unknown: %s" % error
        return self.tool_line([os.path.join(vs, "VC", "Tools", "MSVC", version, "bin", "Hostx64", "x86", "cl.exe")])

    def record(self, part):
        """The record of a part built here, for its manifest entry (the design's 11.7): the tools of its build, not in
        any key. For luajit also the md5 of build_luajit.bat, which is in no set's commits (the plan's addendum 1 A3)."""
        rust = lambda repo: {"rustc": self.tool_line(["rustc", "-V"], self.trees[repo]),  # noqa: E731
                             "cargo": self.tool_line(["cargo", "-V"], self.trees[repo])}
        msvc = lambda: {"msbuild": self.tool_line([MSBUILD, "-version", "-nologo"]), "cl": self.cl_line()}  # noqa: E731
        if part == "plugin":
            return rust("nirender")
        if part == "numidium":
            return rust("render-link")
        if part == "render-link":
            return dict(rust("render-link"), cl=self.cl_line())
        if part == "mwse":
            return msvc()
        if part == "luajit":
            return dict(msvc(), **{"build_luajit.bat md5": sets.md5_of(LUAJIT_BAT)})
        # plugin-lua: a copy of the Lua files, no build.
        return {}

    def probe(self, dll):
        """The probe of deploy_rest.py, in the tree at the set's nirender commit: (exit code, output)."""
        tree = self.trees["nirender"]
        self.room("the probe (it compiles the plugin's tests)")
        command = [sys.executable, "-I", os.path.join(tree, "tools", "check_mwse_sites.py"), "--features", "--mge", self.trees["render-link"], dll]
        done = subprocess.run(command, cwd=tree, env=build_environment(), capture_output=True, text=True, encoding="utf-8",
                              errors="replace", stdin=subprocess.DEVNULL)
        return done.returncode, done.stdout + done.stderr


class SlotHold:
    """The state of the maker's build hold: `lost` is the slot tool's line when a renew was refused, else None."""

    def __init__(self):
        self.lost = None


@contextlib.contextmanager
def hold_notes(slots, label, seconds=NOTE_SECONDS):
    """Notes the build work and renews the lease of the hold `label` every `seconds` (a child's compile is not seen by
    the slot's own notes). A renew that the slot tool refuses (the hold lapsed) sets `lost` of the SlotHold that this
    yields, and the notes stop: the maker then writes nothing more to E: (verdict 507, addition 1)."""
    hold = SlotHold()
    stop = threading.Event()

    def note():
        while not stop.wait(seconds):
            slots.note_holder_build(slots.owner_of(slots.session_id(), label), time.time())
            reply = slots.renew("build", who=label)
            if reply.code != slots.DONE:
                hold.lost = (reply.text or "").strip().splitlines()[0] if (reply.text or "").strip() else "the renew was refused"
                return

    thread = threading.Thread(target=note, daemon=True)
    thread.start()
    try:
        yield hold
    finally:
        stop.set()
        thread.join()


def in_hold_problem(slots, label):
    """Why this session does not hold every place of the build side under `label` (--in-hold), as a text, or None. A
    state that cannot be read is a problem too: the maker then builds nothing."""
    from merge_step import places_held

    needed = len(slots.units_of("build"))
    held = places_held(slots, "build", label)
    if held is None:
        return "the state of the slots cannot be read"
    if held < needed:
        return "this session holds %d of the %d places of the build side as \"%s\"" % (held, needed, label)
    return None


@contextlib.contextmanager
def in_hold(slots, label):
    """The maker inside a caller's take (--in-hold, the plan's addendum 10 C2): no take and no release; the notes and
    the renew of `label` each minute, as build_hold has them. Yields the SlotHold."""
    with hold_notes(slots, label) as hold:
        yield hold


def maker_hold(slots, round_label, label, no_wait):
    """The hold of one make(): (None, a function that gives its context), or (why not, None) when --in-hold names a
    hold that this session does not have. `round_label` is the label of --in-hold, or None for the maker's own take
    under `label`."""
    if round_label is None:
        return None, lambda: build_hold(slots, label, no_wait)
    problem = in_hold_problem(slots, round_label)
    if problem:
        return "--in-hold: %s: nothing is built" % problem, None
    return None, lambda: in_hold(slots, round_label)


@contextlib.contextmanager
def build_hold(slots, label, no_wait):
    """The build slot from before the first write to E: to the rename of the set, with hold_notes, and given back at
    the end. Yields the SlotHold."""
    from merge_step import take_build

    if not take_build(slots, label, no_wait):
        raise SetFault("no build slot", EXIT_REFUSED)
    try:
        with hold_notes(slots, label) as hold:
            yield hold
    finally:
        say(slots.release("build", label).text.splitlines()[0])


# --- the set ------------------------------------------------------------------------------------

@contextlib.contextmanager
def run_lock(path):
    """The tool's run lock for the time of one make(): yields True, or False when another holds it. It is given back at
    the end, so that a caller can call make() again (a lock is held per handle: a second handle of this process would be
    refused too)."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    descriptor = hold_run_lock(path)
    if descriptor is None:
        yield False
        return
    try:
        yield True
    finally:
        with contextlib.suppress(OSError):
            os.lseek(descriptor, 0, os.SEEK_SET)
            msvcrt.locking(descriptor, msvcrt.LK_UNLCK, 1)
        os.close(descriptor)


def process_lives(pid):
    """(lives, program) of the process `pid`, from the Windows process list. Any process with that id counts as living,
    whatever its program (another process can have taken the id again): the safe side. A list that cannot be read
    counts as living too."""
    try:
        done = subprocess.run(["tasklist", "/FI", "PID eq %d" % int(pid), "/FO", "CSV", "/NH"], capture_output=True, text=True,
                              timeout=30, stdin=subprocess.DEVNULL)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        return True, "the process list: %s" % error
    for line in done.stdout.splitlines():
        cells = [cell.strip('"') for cell in line.split('","')]
        if len(cells) > 1 and cells[1].strip('"') == str(int(pid)):
            return True, cells[0]
    return done.returncode != 0, "" if done.returncode == 0 else "the process list failed"


def write_marker(making, session, now):
    """The first file of `<name>.making`: who makes it (the design's 11.5)."""
    marker = {"tool": tool_version(), "session": session, "pid": os.getpid(), "time": now.isoformat(timespec="seconds")}
    with open(os.path.join(making, MARKER), "w", encoding="utf-8", newline="\n") as file:
        json.dump(marker, file, indent=1, sort_keys=True)
        file.write("\n")


def read_marker(making):
    """The marker of `<name>.making` as a dict, or None when it has none or it does not read."""
    try:
        with open(os.path.join(making, MARKER), encoding="utf-8") as file:
            marker = json.load(file)
    except (OSError, ValueError):
        return None
    return marker if isinstance(marker, dict) and isinstance(marker.get("pid"), int) else None


def missing_proof(commits, builder):
    """The line for the first proof of PROOFS that a commit's tree lacks, or None. No flag passes a proof."""
    for repo, path, text, what, brought_by in PROOFS:
        there = builder.has_file(repo, commits[repo], path) if text is None else builder.has_text(repo, commits[repo], path, text)
        if not there:
            return ("the %s commit %s lacks %s (%s%s, brought by %s): no set is made; for an older build, make a branch "
                    "with that proof picked onto it" % (repo, commits[repo][:8], what, path, "" if text is None else ": " + text, brought_by))
    return None


def request_differences(manifest, commits, patch_md5s, kind):
    """How the set of `manifest` differs from a request of `commits` ({repo: full commit}), `patch_md5s` ({repo: md5})
    and `kind`: one text for each repo whose commit or patch differs, and one for the kind. Empty when it is the set of
    the request."""
    differences = []
    for repo in sets.REPOS:
        there = manifest.get("commits", {}).get(repo, {}).get("commit")
        if there != commits[repo]:
            differences.append("%s: the set has %s, the request %s" % (repo, there, commits[repo]))
        there_patch = manifest.get("patches", {}).get(repo, {}).get("md5")
        if there_patch != patch_md5s.get(repo):
            differences.append("%s: the set's patch md5 is %s, the request's %s" % (repo, there_patch, patch_md5s.get(repo)))
    if manifest.get("kind") != kind:
        differences.append("the set's kind is %s, the request's %s" % (manifest.get("kind"), kind))
    return differences


def make(commits, patches, kind, builder, root=sets.SETS, gate=e_gate, hold=None, keep=PDB_KEEP, session=this_session,
         now=None, reuse=True, lock=RUN_LOCK, lives=process_lives):
    """Makes the set of `commits` ({repo: full commit}) with `patches` ({repo: (file, md5)}). `builder` does the builds
    (Trees, or a fake in the tests); `hold` is a context manager for the build slot (None: no slot, the tests). Returns
    (exit code, lines); the last line is the set's folder. `reuse` False builds every part (only the null of a test).
    make() takes the tool's run lock `lock` itself, so that no caller can skip it, and stops with exit code 3 when
    another run holds it (verdict 507, addition 2). `lives` replaces the process list in the tests."""
    with run_lock(lock) as held:
        if not held:
            return EXIT_REFUSED, ["another run of build_set.py holds its run lock %s: nothing is built" % lock.replace("\\", "/")]
        return make_locked(commits, patches, kind, builder, root, gate, hold, keep, session, now, reuse, lives)


def make_locked(commits, patches, kind, builder, root, gate, hold, keep, session, now, reuse, lives):
    """make() under the run lock."""
    now = now or datetime.datetime.now().astimezone()
    patch_md5s = {repo: md5 for repo, (_, md5) in patches.items()}
    name = sets.set_name(commits, patch_md5s, kind)
    folder = os.path.join(root, name)
    making = folder + ".making"
    lines = []
    if os.path.exists(folder):
        try:
            there_manifest = sets.check(folder)
        except sets.SetError as error:
            return EXIT_FAULT, ["%s is there and is not a sealed set, and nothing is built: %s" % (name, error)]
        # The name holds 8 hex of each commit only: the set there must be of the full commits, the patches and the kind
        # of the request (the design's 11.4).
        differences = request_differences(there_manifest, commits, patch_md5s, kind)
        if differences:
            return EXIT_FAULT, ["%s is there and is the set of other commits, and nothing is built: %s" % (name, "; ".join(differences))]
        there = ["%s is there and its check passes: nothing is built" % name]
        return 0, there + ([PATCH_UNVERIFIED] if patches else []) + [folder.replace("\\", "/")]
    # An unfinished folder (the design's 11.5). Under the run lock no other maker runs, so a folder with the tool's
    # marker is stale; the marker's process must also be gone (a second check). It is renamed aside, as the first
    # phase of writes below; nothing is deleted.
    stale = None
    if os.path.exists(making):
        marker = read_marker(making)
        if marker is None:
            return EXIT_FAULT, ["%s is there with no marker %s: not an unfinished set of this tool by proof; nothing is built"
                                % (making.replace("\\", "/"), MARKER)]
        alive, program = lives(marker["pid"])
        if alive:
            return EXIT_FAULT, ["%s is there, and the process %d of its marker lives (%s); nothing is built"
                                % (making.replace("\\", "/"), marker["pid"], program or "unknown")]
        stale = "%s.failed-%s" % (folder, now.strftime("%Y%m%d-%H%M%S"))
        if os.path.exists(stale):
            return EXIT_FAULT, ["%s is there already: nothing is built" % stale.replace("\\", "/")]
    lacking = missing_proof(commits, builder)
    if lacking:
        return EXIT_FAULT, [lacking]
    keys = part_keys(commits, lambda repo, folder: builder.tree(repo, commits[repo], folder), patch_md5s)
    found = {part: sets.find_part(root, part, keys[part], kind) if reuse else None for part in sets.PARTS}
    to_build = [part for part in sets.PARTS if found[part] is None]
    for part in sets.PARTS:
        say("part %s: %s" % (part, "copied from %s" % found[part][0] if found[part] else "built"))
    parts_from = {part: found[part][0] if found[part] else "built" for part in sets.PARTS}
    try:
        # The build slot is held from before the first write to E: (a tree on E:) to the rename of the set (the
        # design's 11.6). A hold can lapse (15 idle minutes, verdict 507 addition 1), so each phase of writes to E:
        # checks the hold and asks the E: gate again, and stops before it writes.
        with hold() if hold else contextlib.nullcontext() as slot_hold:
            builder.slot_hold = slot_hold

            def phase(what):
                if slot_hold is not None and slot_hold.lost:
                    raise SetFault("the build slot was lost before %s (%s): nothing more is written" % (what, slot_hold.lost), EXIT_REFUSED)
                allowed, gate_line = gate()
                if not allowed:
                    raise SetFault("no set made: the E: gate says no before %s: %s" % (what, gate_line), EXIT_REFUSED)

            if stale:
                phase("the rename of the unfinished folder")
                os.rename(making, stale)
                lines.append("%s was an unfinished set of a run that is gone: renamed to %s" % (
                    making.replace("\\", "/"), os.path.basename(stale)))
            for repo in ("nirender", "render-link"):
                builder.ready(repo, commits[repo])
            if {"mwse", "luajit"} & set(to_build):
                builder.ready("mwse", commits["mwse"])
            for repo, (patch_file, _) in sorted(patches.items()):
                if repo in builder.trees:
                    builder.patch(repo, patch_file)
            stage = builder.build_cargo() if set(to_build) & CARGO_PARTS else None
            mwse = builder.build_mwse() if "mwse" in to_build else None
            luajit = builder.build_luajit() if "luajit" in to_build else None
            built = built_parts(stage, mwse, luajit)
            files = {}
            for part in sets.PARTS:
                if found[part]:
                    part_files = set_part_files(root, found[part][0], found[part][1], part)
                elif part == "plugin-test":
                    # After plugin-lua in PARTS: the split takes the final list of the shipped Lua.
                    part_files = test_part_files(builder.mod_folder(), [path for path in files if sets.part_of(path) == "plugin-lua"])
                    built[part] = part_files
                else:
                    part_files = built[part]
                required = REQUIRED[part]
                if (not part_files) if required is None else not any(path.lower() == required.lower() for path in part_files):
                    raise SetFault("the part %s has no %s" % (part, required or "file"))
                files.update(part_files)
            faults = build_id_faults(files, commits, patches)
            if faults:
                raise SetFault("the binaries are not builds of the set's commits: %s" % "; ".join(faults))
            code, output = builder.probe(files["MWSE.dll"])
            for line in output.splitlines():
                if line.startswith(("feature: ", MISSING, "d3d8: ")):
                    lines.append("probe: " + line)
            if code == MISSING_FEATURE_EXIT:
                raise SetFault("MISSING FEATURE: the MWSE.dll of the set lacks what the plugin needs; no set is made", MISSING_FEATURE_EXIT)
            if code != 0:
                raise SetFault("the probe of the MWSE features failed with exit code %d: %s" % (code, (output.strip().splitlines() or ["no output"])[-1]))
            phase("the set's folder")
            try:
                os.mkdir(making)
            except FileExistsError:
                raise SetFault("%s appeared: another run makes this set" % making) from None
            write_marker(making, session(), now)
            for part in sets.PARTS:
                if found[part]:
                    source_name, source_manifest = found[part]
                    copy_part(set_part_files(root, source_name, source_manifest, part), making, source_manifest["files"])
                else:
                    copy_part(built[part], making)
            for repo, (patch_file, _) in sorted(patches.items()):
                copy_part({"%s/%s.patch" % (sets.PATCHES, repo): patch_file}, making)
            pdbs = builder.pdb_sources(to_build)
            for part in sets.PARTS:
                if found[part]:
                    source = os.path.join(keep, found[part][0])
                    for path in found[part][1]["files"]:
                        if sets.part_of(path) == part and path.lower().endswith((".dll", ".exe")):
                            pdbs[path] = os.path.join(source, os.path.splitext(os.path.basename(path))[0] + ".pdb")
            pdbs = {path: pdb for path, pdb in pdbs.items() if not sets.PLUGIN_COPY_RE.fullmatch(path)}
            phase("the pdbs")
            lines += ["pdb: " + line for line in keep_pdbs(name, making, pdbs, keep, root)]
            lines += builder.size_lines()
            phase("the manifest and the rename of the set")
            # The marker is the tool's own file: it goes before the manifest, which records the same facts.
            os.remove(os.path.join(making, MARKER))
            # The record of each part (the plan's addendum 1 A3): a built part's from the builder, a copied part's from
            # the manifest of its set.
            records = {part: found[part][1]["parts"][part].get("record", {}) if found[part] else builder.record(part) for part in sets.PARTS}
            form = builder.distant_land_format(commits["render-link"], making)
            write_manifest(making, name, kind, commits, patches, parts_from, keys, now, session(), records, form)
            os.rename(making, folder)
    except SetFault as fault:
        return fault.code, lines + builder.size_lines() + [str(fault)]
    finally:
        builder.unpatch()
    try:
        manifest = sets.check(folder)
    except sets.SetError as error:
        return EXIT_FAULT, lines + ["the new set fails its own check: %s" % error]
    lines.append("%s: %d files, manifest md5 %s" % (name, len(manifest["files"]), sets.manifest_md5(folder)))
    if patches:
        lines.append(PATCH_UNVERIFIED)
    return 0, lines + [folder.replace("\\", "/")]


# --- the tests ------------------------------------------------------------------------------------

class FakeBuilder:
    """Builds of fake files with the build ids of the commits and new random bytes at each build, so that two builds of
    one commit differ (as real builds do), and the reuse test can fail."""

    def __init__(self, scratch, probe_code=0, wrong_render_link=False):
        self.scratch = scratch
        self.trees = {}
        self.applied = []
        self.cargo_builds = 0
        self.mwse_builds = 0
        self.luajit_builds = 0
        self.probe_code = probe_code
        self.probed = []
        self.wrong_render_link = wrong_render_link
        # The git trees of the folders that a key names, by (repo, folder): "c" * 40 unless a test sets one.
        self.trees_of = {}
        # The proofs that the trees lack, by (repo, path, text): none unless a test sets one.
        self.lacks = set()

    def tree(self, repo, commit, folder):
        return self.trees_of.get((repo, folder), "c" * 40)

    def has_file(self, repo, commit, path):
        return (repo, path, None) not in self.lacks

    def has_text(self, repo, commit, path, text):
        return (repo, path, text) not in self.lacks

    def distant_land_format(self, commit, making):
        """The real function, with a planted helper that gives a key: the fake numidium.exe cannot run."""
        def helper(command, **_):
            return subprocess.CompletedProcess(command, 0, "format abcd1234 MGE_DL_VERSION=22\nsettings 00112233\n", "")
        return distant_land_format(self, commit, making, helper)

    def ready(self, repo, commit):
        self.trees[repo] = commit

    def patch(self, repo, patch_file):
        self.applied.append(repo)

    def unpatch(self):
        self.applied = []

    def build_cargo(self):
        self.cargo_builds += 1
        stage = tempfile.mkdtemp(dir=self.scratch)
        nirender = self.trees["nirender"][:7] + ("-dirty" if "nirender" in self.applied else "")
        render_link = ("0000000" if self.wrong_render_link else self.trees["render-link"][:7]) + ("-dirty" if "render-link" in self.applied else "")
        files = {
            sets.PLUGIN_STAGED: b"nirender v0.3.8 %s." % nirender.encode(),
            "numidium.exe": b"nirender-host v0.3.8 %s. numidium v1.0 %s." % (nirender.encode(), render_link.encode()),
            "d3d8.dll": b"MGE-XE G7 Fork v1.0 %s." % render_link.encode(),
            "dinput8.dll": b"dinput8",
            "Data Files/mwse/mods/nirender/main.lua": b"-- main",
            "Data Files/shaders/core/XE Main.fx": b"// fx",
            "d3d8.pdb": b"no pdb goes into a set",
        }
        for path, data in files.items():
            sets.write(os.path.join(stage, *path.split("/")), data + os.urandom(8))
        return stage

    def mod_folder(self):
        """A mod folder with the shipped main.lua, a test Lua file, a test mesh and a test texture."""
        mod = os.path.join(self.scratch, "fake-mod")
        for path in (MOD_LUA + "main.lua", MOD_LUA + "test_views.lua", "meshes/@_nifx/a.nif", "textures/@_nifx/a.dds"):
            if not os.path.isfile(os.path.join(mod, *path.split("/"))):
                sets.write(os.path.join(mod, *path.split("/")), path.encode())
        return mod

    def build_luajit(self):
        self.luajit_builds += 1
        out = tempfile.mkdtemp(dir=self.scratch)
        sets.write(os.path.join(out, "lua51.dll"), b"LuaJIT" + os.urandom(8))
        return os.path.join(out, "lua51.dll")

    def record(self, part):
        tools = {"plugin": {"rustc": "rustc t", "cargo": "cargo t"}, "mwse": {"msbuild": "msbuild t", "cl": "cl t"},
                 "luajit": {"build_luajit.bat md5": "b" * 32}}
        return dict(tools.get(part, {}), build=self.cargo_builds + self.mwse_builds + self.luajit_builds)

    def build_mwse(self):
        self.mwse_builds += 1
        out = tempfile.mkdtemp(dir=self.scratch)
        sets.write(os.path.join(out, "MWSE.dll"), b"MWSE" + os.urandom(8))
        sets.write(os.path.join(out, "core", "initialize.lua"), b"-- core")
        return os.path.join(out, "MWSE.dll"), os.path.join(out, "core")

    def pdb_sources(self, parts_built):
        return {}

    def size_lines(self):
        return []

    def probe(self, dll):
        self.probed.append(dll)
        return self.probe_code, "feature: x\n" + (MISSING + "y\n" if self.probe_code == MISSING_FEATURE_EXIT else "")


def runner_compatible(folder):
    """The runner takes the set (W521): sets.verify passes, and the runner's staging, with the plugin and without it,
    holds the set's plugin exactly when the run loads it, and every file that it stages is there. Returns the
    faults."""
    import run_own_device

    try:
        manifest = sets.verify(folder)
    except sets.SetError as error:
        return ["the runner refuses the set: %s" % error]
    faults = []
    for plugin in (True, False):
        mods = run_own_device.set_mods(folder, manifest, plugin)
        staged = mods.get("nirender", {}).get(sets.PLUGIN_IN_MOD)
        if plugin and staged != os.path.join(folder, *sets.staged_plugin(manifest).split("/")):
            faults.append("the runner stages %s as the plugin, not the set's" % staged)
        if not plugin and staged:
            faults.append("a run without the plugin stages %s" % staged)
        faults += ["the runner stages %s, and it is not there" % source for files in mods.values() for source in files.values()
                   if not os.path.isfile(source)]
    return faults


def in_hold_test(state):
    """--in-hold (addendum 10 C2, C8 test 2) on slot.py with a scratch state folder: a label that this session does not
    hold refuses, and nothing is built; the null is the round's take of both slots under the label. The renew of the
    label under that take works, as hold_notes renews it each minute."""
    from merge_step import scratch_slots

    environment = dict(os.environ)
    path = list(sys.path)
    try:
        slots = scratch_slots(state)
        label = "merge_step: plugin aaaaaaaaaa"
        places = len(slots.units_of("build"))
        problem, hold = maker_hold(slots, label, "build_set: unused", True)
        assert hold is None and problem == "--in-hold: this session holds 0 of the %d places of the build side as \"%s\": nothing is built" % (places, label), problem
        assert slots.take("both", label, minutes=5).code == slots.DONE
        try:
            problem, hold = maker_hold(slots, label, "build_set: unused", True)
            assert problem is None and hold is not None, problem
            with hold() as held:
                assert held.lost is None
            assert slots.renew("build", who=label).code == slots.DONE, "the renew of hold_notes was refused under the round's take"
            other, none = maker_hold(slots, "merge_step: another", "build_set: unused", True)
            assert none is None and "holds 0 of the %d places" % places in other, other
            # The maker gives nothing back: the round's take stands after it.
            assert in_hold_problem(slots, label) is None
        finally:
            assert slots.release("both", label).code == slots.DONE
        assert maker_hold(slots, None, "build_set: x", True)[0] is None, "without --in-hold the maker takes its own hold"
    finally:
        os.environ.clear()
        os.environ.update(environment)
        sys.path[:] = path


def part_inputs_test(scratch):
    """W567 part C: the list of the part inputs, and its check against the dep-info files of the builds."""
    assert is_part_input("src/lib.rs") and is_part_input("host/src/retained.rs") and is_part_input("Cargo.toml") and is_part_input("mod/MWSE/mods/x.lua")
    assert not is_part_input("tools/merge_step.py") and not is_part_input("README.md") and not is_part_input("Cargo.toml.bak") and not is_part_input("srcx/a.rs")
    # Verdict 608: git's own files change the checked-out bytes, and no dep-info file names them.
    assert is_part_input(".gitattributes") and is_part_input(".gitmodules") and not is_part_input("tools/.gitattributes")
    tree = os.path.join(scratch, "nirender-plugin")
    text = "%s: %s %s %s %s %s\n%s:\n" % (os.path.join(tree, "target", "x.dll"), os.path.join(tree, "src", "lib.rs"), os.path.join(tree, "host", "src", "a\\ b.rs"),
                                       os.path.join(tree, ".git", "HEAD"), os.path.join(scratch, "registry", "x.rs"), os.path.join(tree, "build.rs"),
                                       os.path.join(tree, "src", "lib.rs"))
    assert input_drift([text], tree) == [], input_drift([text], tree)
    # The null: a build that reads a file of the tree outside the list is a drift, by its path.
    planted = text.replace(os.path.join(tree, "build.rs"), os.path.join(tree, "tools", "gen.rs"))
    assert input_drift([planted], tree) == ["tools/gen.rs"], input_drift([planted], tree)
    # The maker's files: both are read; a missing one cannot be checked.
    trees = os.path.join(scratch, "maker")
    for repo, relative in MAKER_DEP_INFO:
        path = os.path.join(trees, TREE_NAMES[repo], relative)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as file:
            file.write("%s: %s\n" % (path, os.path.join(trees, TREE_NAMES["nirender"], "tools" if repo == "render-link" else "src", "a.rs")))
    assert maker_input_drift(trees) == (["tools/a.rs"], None), maker_input_drift(trees)
    os.remove(os.path.join(trees, TREE_NAMES["render-link"], MAKER_DEP_INFO[1][1]))
    outside, why = maker_input_drift(trees)
    assert outside is None and "no dep-info file" in why, why


def self_test():
    commits = {"nirender": "6d1b214" + "a" * 33, "render-link": "d9741db" + "b" * 33, "mwse": "89198b53" + "c" * 32}
    yes = lambda: (True, "e-ok: yes")  # noqa: E731
    session = lambda: "test (id)"  # noqa: E731
    with tempfile.TemporaryDirectory(prefix="build-set-") as scratch:
        root, keep = os.path.join(scratch, "sets"), os.path.join(scratch, "pdb")
        os.mkdir(root)
        builder = FakeBuilder(scratch)

        lock = os.path.join(scratch, "build_set.lock")

        def run(commits_of, builder_of=None, patches=None, gate=yes, reuse=True):
            return make(commits_of, patches or {}, sets.KIND_USUAL, builder_of or builder, root, gate, None, keep, session, reuse=reuse,
                        lock=lock)

        # The run lock (verdict 507, addition 2): make() takes it itself, and refuses while another holds it. The null:
        # the same call after the holder lets go makes the set (the first set below).
        with run_lock(lock) as held:
            assert held
            code, lines = run(commits)
            assert code == EXIT_REFUSED and "holds its run lock" in lines[-1] and builder.cargo_builds == 0, lines
            assert os.listdir(root) == [], os.listdir(root)

        # A set from nothing: every part built, the set passes its check, today's runner takes it.
        code, lines = run(commits)
        assert code == 0, lines
        first = lines[-1]
        assert os.path.basename(first) == "s-6d1b214a-d9741dbb-89198b53" and builder.cargo_builds == 1 and builder.mwse_builds == 1, lines
        manifest = sets.check(first)
        assert manifest["parts"]["plugin"]["from"] == "built" and manifest["distant_land_format"] == "abcd1234"
        assert manifest["distant_land_format_text"] == "MGE_DL_VERSION=22", manifest.get("distant_land_format_text")
        # W521 step 4: a render-link commit without the option gives own-<8 hex> and asks no helper (the null); one
        # with it takes the helper's format; a helper with the option that gives no key makes no set.
        class Tree:
            def __init__(self, has):
                self.has = has

            def has_file(self, repo, commit, path):
                return self.has and path == KEY_SOURCE

        def never(*_, **__):
            raise AssertionError("an old helper was started")

        def no_key(command, **_):
            return subprocess.CompletedProcess(command, 2, "", "usage")

        def gives_key(command, **_):
            return subprocess.CompletedProcess(command, 0, "format 0f0f0f0f A=1\nsettings 1\n", "")

        assert distant_land_format(Tree(False), "d9741dbb" * 5, scratch, never) == ("own-d9741dbb", None)
        assert distant_land_format(Tree(True), "d9741dbb" * 5, scratch, gives_key) == ("0f0f0f0f", "A=1")
        try:
            distant_land_format(Tree(True), "d9741dbb" * 5, scratch, no_key)
            raise AssertionError("a set was made with a helper that gives no key")
        except SetFault as fault:
            assert "gave no distant land key" in str(fault), fault
        assert manifest["commits"]["mwse"] == {"commit": commits["mwse"], "clean": True}
        assert "akulakhan-6d1b214.dll" in manifest["files"] and "MWSE.dll" in manifest["files"]
        assert "Data Files/MWSE/core/initialize.lua" in manifest["files"] or "Data Files/mwse/core/initialize.lua" in manifest["files"]
        assert not [path for path in manifest["files"] if path.endswith(".pdb")], "a pdb went into the set"
        assert runner_compatible(first) == [], runner_compatible(first)
        # The null of that check: a copy of the set without its plugin copy fails it (the runner's verify refuses it).
        bare = os.path.join(scratch, "bare", os.path.basename(first))
        shutil.copytree(first, bare)
        os.remove(os.path.join(bare, sets.plugin_copy(manifest)))
        assert any("the runner refuses the set" in fault for fault in runner_compatible(bare)), "the runner check cannot fail"
        # Test 1: the same commits again build nothing.
        code, lines = run(commits)
        assert code == 0 and "nothing is built" in lines[0] and builder.cargo_builds == 1 and builder.mwse_builds == 1, lines
        # A name that is there with other full commits (the design's 11.4): refused, nothing built, both commits named.
        # The null is test 1 above: the same full commits pass.
        same_name = dict(commits, nirender=commits["nirender"][:8] + "9" * 32)
        assert sets.set_name(same_name) == os.path.basename(first)
        code, lines = run(same_name)
        assert code == EXIT_FAULT and "the set of other commits" in lines[-1] and builder.cargo_builds == 1, lines
        assert commits["nirender"] in lines[-1] and same_name["nirender"] in lines[-1], lines
        assert request_differences(manifest, commits, {}, "rig") == ["the set's kind is usual, the request's rig"]
        assert request_differences(manifest, commits, {"mwse": "f" * 32}, sets.KIND_USUAL)[0].startswith("mwse: the set's patch md5 is None")
        # Test 8: another MWSE commit builds only MWSE; the other parts are the same bytes.
        other = dict(commits, mwse="1234abcd" + "d" * 32)
        code, lines = run(other)
        assert code == 0 and builder.cargo_builds == 1 and builder.mwse_builds == 2, lines
        second = sets.check(lines[-1])
        for path in ("d3d8.dll", "numidium.exe", "akulakhan-6d1b214.dll"):
            assert second["files"][path]["md5"] == manifest["files"][path]["md5"], path
        assert second["parts"]["plugin"]["from"] == os.path.basename(first)
        # The part plugin-test (the plan's addendum 3): the mod folder less the shipped Lua; with plugin-lua it is the
        # whole mod folder once staged, and no staged path is in both.
        tests = sorted(path for path in manifest["files"] if sets.part_of(path) == "plugin-test")
        assert tests == ["plugin-test/MWSE/mods/nirender/test_views.lua", "plugin-test/meshes/@_nifx/a.nif",
                         "plugin-test/textures/@_nifx/a.dds"], tests
        shipped = {path.lower()[len(SET_LUA):] for path in manifest["files"] if sets.part_of(path) == "plugin-lua"}
        staged_tests = {path[len(sets.PLUGIN_TEST) + 1:].lower() for path in tests}
        assert shipped == {"main.lua"} and not {MOD_LUA.lower() + name for name in shipped} & staged_tests
        mod_files = {path.lower() for path in files_below(builder.mod_folder())}
        assert {MOD_LUA.lower() + name for name in shipped} | staged_tests == mod_files, (shipped, staged_tests, mod_files)
        assert second["parts"]["plugin-test"]["from"] == os.path.basename(first)
        # The luajit part (the design's 11.2): built for the first set, with the bat's md5 in its record; the second
        # set's MWSE commit names the same rubic0n commit and LuaJIT.targets, so lua51.dll is copied, byte for byte,
        # with its record.
        assert manifest["parts"]["luajit"]["from"] == "built" and manifest["parts"]["luajit"]["record"]["build_luajit.bat md5"] == "b" * 32
        # The tools record (the design's 11.7): a copied part carries its set's record (the fake's build count shows
        # whose), and a part built again has its own.
        assert second["parts"]["plugin"]["record"] == manifest["parts"]["plugin"]["record"] and manifest["parts"]["plugin"]["record"]["rustc"] == "rustc t"
        assert second["parts"]["mwse"]["record"] != manifest["parts"]["mwse"]["record"] and second["parts"]["mwse"]["record"]["cl"] == "cl t"
        tools = Trees(os.path.join(scratch, "tool-trees"))
        assert tools.tool_line([sys.executable, "-I", "-c", "print('tool 1.2.3'); print('more')"]) == "tool 1.2.3"
        assert tools.tool_line([os.path.join(scratch, "no-such-tool.exe")]).startswith("unknown: ")
        # cl's form: the version on stderr, a usage line on stdout.
        cl_form = "import sys; print('usage: cl'); sys.stderr.write('Compiler Version 19\\n')"
        assert tools.tool_line([sys.executable, "-I", "-c", cl_form]) == "Compiler Version 19"
        assert second["parts"]["luajit"]["from"] == os.path.basename(first) and builder.luajit_builds == 1
        assert second["files"]["lua51.dll"]["md5"] == manifest["files"]["lua51.dll"]["md5"]
        assert second["parts"]["luajit"]["record"] == manifest["parts"]["luajit"]["record"]
        # Another rubic0n commit: lua51.dll is built again, and the MWSE part of that commit too.
        builder.trees_of[("mwse", "deps/rubic0n")] = "f" * 40
        code, lines = run(dict(commits, mwse="2468abcd" + "d" * 32))
        assert code == 0 and builder.luajit_builds == 2 and sets.check(lines[-1])["parts"]["luajit"]["key"]["mwse rubic0n"] == "f" * 40, lines
        builder.trees_of = {}
        # The build parameter: the real LuaJIT.targets of the MWSE checkout and the real bat agree (lua52compat, the
        # null); another value in either is refused; an unreadable one too.
        targets_path = os.path.join(SOURCE_REPOS["mwse"], *LUAJIT_TARGETS.split("/"))
        if os.path.isfile(targets_path) and os.path.isfile(LUAJIT_BAT):
            with open(targets_path, encoding="utf-8") as file, open(LUAJIT_BAT, encoding="utf-8", errors="replace") as bat:
                assert luajit_parameter_fault(file.read(), bat.read()) is None
        else:
            print("build_set self-test: the real LuaJIT.targets or build_luajit.bat is not here; only the planted texts are tested")
        planted_targets = "<LuaJitBuildParams>lua52compat</LuaJitBuildParams>"
        assert luajit_parameter_fault(planted_targets, 'call "%~1\\deps\\rubic0n\\src\\msvcbuild.bat" lua52compat\r\n') is None
        assert "is not the one of build_luajit.bat (static)" in luajit_parameter_fault(planted_targets, 'call "x\\msvcbuild.bat" static\r\n')
        assert "cannot be read in LuaJIT.targets" in luajit_parameter_fault("<Project/>", 'call "x\\msvcbuild.bat" lua52compat')
        # The keys of folders of another repo: another contract/ tree makes a new render-link part and keeps the plugin;
        # another xtask/ tree makes a new Lua part and keeps the plugin DLL and the render-link part, byte for byte.
        builder.trees_of[("nirender", "contract")] = "d" * 40
        code, lines = run(dict(commits, mwse="c0c0c0c0" + "0" * 32))
        keyed = sets.check(lines[-1])
        assert code == 0 and keyed["parts"]["render-link"]["from"] == "built" and keyed["parts"]["plugin"]["from"] != "built", lines
        assert keyed["parts"]["render-link"]["key"]["nirender contract"] == "d" * 40
        builder.trees_of = {("render-link", "xtask"): "e" * 40}
        code, lines = run(dict(commits, mwse="c1c1c1c1" + "0" * 32))
        keyed = sets.check(lines[-1])
        assert code == 0 and keyed["parts"]["plugin-lua"]["from"] == "built" and keyed["parts"]["plugin-test"]["from"] == "built", lines
        assert keyed["parts"]["plugin"]["from"] != "built" and keyed["parts"]["render-link"]["from"] != "built", lines
        for path in ("akulakhan-6d1b214.dll", "d3d8.dll"):
            assert keyed["files"][path]["md5"] == manifest["files"][path]["md5"], path
        builder.trees_of = {}
        # The null of test 8: without the reuse each part is built again, and the md5s differ.
        third = dict(commits, mwse="5678abcd" + "e" * 32)
        builds = builder.cargo_builds
        code, lines = run(third, reuse=False)
        assert code == 0 and builder.cargo_builds == builds + 1, lines
        assert sets.check(lines[-1])["files"]["d3d8.dll"]["md5"] != manifest["files"]["d3d8.dll"]["md5"], "the null of the reuse cannot fail"
        # The probe's exit code 4: no set, no folder.
        missing = dict(commits, mwse="9999abcd" + "f" * 32)
        code, lines = run(missing, FakeBuilder(scratch, probe_code=MISSING_FEATURE_EXIT))
        assert code == MISSING_FEATURE_EXIT and "MISSING FEATURE" in lines[-1], lines
        assert not [name for name in os.listdir(root) if name.endswith("9999abcd") or "9999abcd.making" in name], os.listdir(root)
        # A build of another commit than the set's: no set.
        code, lines = run(dict(commits, mwse="aaaa1111" + "0" * 32), FakeBuilder(scratch, wrong_render_link=True), reuse=False)
        assert code == 1 and "not builds of the set's commits" in lines[-1], lines
        # The E: gate: nothing written.
        before = sorted(os.listdir(root))
        code, lines = run(dict(commits, mwse="bbbb2222" + "0" * 32), gate=lambda: (False, "a timing run holds the game"))
        assert code == EXIT_REFUSED and sorted(os.listdir(root)) == before, lines
        # Verdict 507, addition 1: the gate is asked before each phase of writes to E:. A no at the pdb phase: exit 3,
        # no pdb, no manifest, no set (the unfinished folder stays, as after any fault).
        answers = iter([(True, "e-ok: yes"), (False, "a timing run holds the game")])
        late = dict(commits, mwse="bbbb3333" + "0" * 32)
        code, lines = run(late, gate=lambda: next(answers), reuse=False)
        late_name = sets.set_name(late)
        assert code == EXIT_REFUSED and "before the pdbs" in lines[-1], lines
        assert not os.path.exists(os.path.join(keep, late_name)) and not os.path.exists(os.path.join(root, late_name)), lines
        assert not os.path.exists(os.path.join(root, late_name + ".making", sets.MANIFEST)), lines
        # A hold that lapses between two phases: the next phase stops before it writes. The null: a hold that stays
        # (the first set of this test, made with no hold, and the notes test below).
        lapsing = SlotHold()

        def lapse_after_the_first_ask():
            lapsing.lost = "build: nobody holds it: take it"
            return True, "e-ok: yes"

        lost = dict(commits, mwse="bbbb4444" + "0" * 32)
        code, lines = make(lost, {}, sets.KIND_USUAL, builder, root, lapse_after_the_first_ask, lambda: contextlib.nullcontext(lapsing),
                           keep, session, reuse=False, lock=lock)
        assert code == EXIT_REFUSED and "the build slot was lost before the pdbs" in lines[-1], lines
        assert not os.path.exists(os.path.join(keep, sets.set_name(lost))) and not os.path.exists(os.path.join(root, sets.set_name(lost))), lines
        # The notes: a renew that the slot tool refuses after 2 calls sets `lost`; the null: renews that pass keep it None.
        for refuse_after, expect_lost in ((2, True), (None, False)):
            renews = []

            def renew(slot, who=None, refuse_after=refuse_after):
                renews.append(who)
                refused = refuse_after is not None and len(renews) > refuse_after
                return types.SimpleNamespace(code="refused" if refused else "done", text="build: nobody holds it: take it" if refused else "")

            fake = types.SimpleNamespace(DONE="done", renew=renew, note_holder_build=lambda owner, now: None,
                                         owner_of=lambda session, label: session, session_id=lambda: "s")
            with hold_notes(fake, "build_set: test", seconds=0.01) as noted:
                deadline = time.time() + 5
                while len(renews) < 4 and noted.lost is None and time.time() < deadline:
                    time.sleep(0.01)
            assert (noted.lost is not None) == expect_lost and renews[0] == "build_set: test", (noted.lost, renews)
            if expect_lost:
                assert noted.lost == "build: nobody holds it: take it" and len(renews) == 3, renews
        # Test 9: a commit and one patch: the mark, the patch in the set, a "-dirty" render-link taken; no part shared
        # with the plain set.
        patch_file = os.path.join(scratch, "change.patch")
        with open(patch_file, "w", encoding="ascii") as file:
            file.write("--- a/x\n+++ b/x\n")
        patches = read_patches(["render-link=" + patch_file])
        code, lines = run(commits, patches=patches)
        assert code == 0, lines
        patched = sets.check(lines[-1])
        assert "+p%s" % patches["render-link"][1][:8] in patched["name"] and "patches/render-link.patch" in patched["files"]
        assert patched["patches"]["render-link"]["md5"] == patches["render-link"][1]
        assert patched["parts"]["render-link"]["from"] == "built" and patched["parts"]["mwse"]["from"] == os.path.basename(first)
        # A patched set says that --patch is not verified (verdict 452 B), at its making and when it is there; the null:
        # the plain set does not.
        assert lines[-2] == PATCH_UNVERIFIED, lines
        assert run(commits, patches=patches)[1][-2] == PATCH_UNVERIFIED
        assert PATCH_UNVERIFIED not in run(commits)[1]
        try:
            read_patches(["render-link=" + patch_file, "render-link=" + patch_file])
            raise AssertionError("a second patch for one repo was taken")
        except SetFault as fault:
            assert "second patch" in str(fault)
        # A name that is there and fails its check: refused, not built again; a .making folder: refused.
        with open(os.path.join(first, "d3d8.dll"), "ab") as file:
            file.write(b"!")
        builds = builder.cargo_builds
        code, lines = run(commits)
        assert code == 1 and "is not a sealed set" in lines[-1] and builder.cargo_builds == builds, lines
        os.mkdir(os.path.join(root, "s-6d1b214a-d9741dbb-0f0f0f0f.making"))
        code, lines = run(dict(commits, mwse="0f0f0f0f" + "1" * 32))
        assert code == 1 and ".making is there with no marker" in lines[-1], lines
        # The design's 11.5: a .making with the tool's marker whose process is gone is renamed to .failed-<time> under
        # the lock, and the set is made; a marker whose process lives stops the call; the rename waits for the E: gate.
        stale_commits = dict(commits, mwse="0e0e0e0e" + "1" * 32)
        stale_making = os.path.join(root, sets.set_name(stale_commits) + ".making")
        os.mkdir(stale_making)
        write_marker(stale_making, "a gone session", datetime.datetime(2026, 11, 17, 1, 2, 3).astimezone())
        dead = lambda pid: (False, "")  # noqa: E731
        alive = lambda pid: (True, "python.exe")  # noqa: E731
        code, lines = make(stale_commits, {}, sets.KIND_USUAL, builder, root, yes, None, keep, session, lock=lock, lives=alive)
        assert code == EXIT_FAULT and "lives (python.exe)" in lines[-1] and os.path.isdir(stale_making), lines
        code, lines = make(stale_commits, {}, sets.KIND_USUAL, builder, root, lambda: (False, "a timing run holds the game"), None, keep,
                           session, lock=lock, lives=dead)
        assert code == EXIT_REFUSED and "before the rename of the unfinished folder" in lines[-1] and os.path.isdir(stale_making), lines
        assert not [name for name in os.listdir(root) if ".failed-" in name], os.listdir(root)
        when = datetime.datetime(2026, 11, 17, 4, 5, 6).astimezone()
        code, lines = make(stale_commits, {}, sets.KIND_USUAL, builder, root, yes, None, keep, session, now=when, lock=lock, lives=dead)
        failed = sets.set_name(stale_commits) + ".failed-20261117-040506"
        assert code == 0 and any("renamed to " + failed in line for line in lines), lines
        assert os.path.isfile(os.path.join(root, failed, MARKER)) and not os.path.exists(stale_making), os.listdir(root)
        made = sets.check(lines[-1])
        assert MARKER not in made["files"] and not os.path.exists(os.path.join(lines[-1], MARKER)), "the marker went into the set"
        assert read_marker(os.path.join(root, failed))["session"] == "a gone session"
        # The proofs of an old commit (the design's 11.3): each missing in turn stops the set before any build, and the
        # line names it and the merge that brought it. The null: every set above, with all proofs there.
        proof_commits = dict(commits, mwse="0d0d0d0d" + "1" * 32)
        for repo, path, text, what, brought_by in PROOFS:
            builds = builder.cargo_builds
            builder.lacks = {(repo, path, text)}
            code, lines = run(proof_commits)
            assert code == EXIT_FAULT and what in lines[-1] and brought_by in lines[-1] and builder.cargo_builds == builds, lines
            assert not os.path.exists(os.path.join(root, sets.set_name(proof_commits) + ".making")), lines
        builder.lacks = set()
        # The real reads of the proofs, on both sides of each proof's commit [V 2026-11-16], when the repos are here.
        real = Trees(os.path.join(scratch, "real-trees"))
        try:
            sides = {"nirender": ("824e51c", "c0f6286", "4bc9a9c"), "render-link": ("95cd1f9",)}
            known = {repo: [resolve(SOURCE_REPOS[repo], commit) for commit in names] for repo, names in sides.items()}
        except SetFault:
            known = None
            print("build_set self-test: the repos of the proofs are not here; their real reads are not tested")
        if known:
            for (repo, path, text, what, _), commit in zip(PROOFS, known["nirender"][:1] + known["render-link"] + known["nirender"][1:]):
                read = (lambda c: real.has_file(repo, c, path)) if text is None else (lambda c: real.has_text(repo, c, path, text))
                assert read(commit) and not read(commit + "~1"), (what, commit)
        assert process_lives(os.getpid())[0] is True
        assert process_lives(99999991) == (False, ""), process_lives(99999991)
        # A process list that fails (tasklist refuses this id) counts as living: the safe side.
        assert process_lives(4294967291)[0] is True
        # The builds and the probe run without the variables that point outside their trees (verdict 452, A2).
        outside = {"PATH": "p", "CARGO_TARGET_DIR": "t", "NIRENDER_PLUGIN_DIR": "n"}
        assert build_environment(outside) == {"PATH": "p"} and len(outside) == 3, outside
        # Test 6: a tree with changes makes no set (a scratch repository and the real Trees.ready).
        repo = os.path.join(scratch, "repo")
        os.mkdir(repo)
        # Under a hook, GIT_DIR names the hook's repository: a scratch repository of its own stands for it, and its config
        # must not change (W527: the hook's run of this self-test made the main checkout bare). The null: without
        # git_env, "git -C repo init" with this GIT_DIR wrote core.bare = true into that config.
        hooked = os.path.join(scratch, "hooked")
        os.mkdir(hooked)
        git(hooked, "init", "-q")
        with open(os.path.join(hooked, ".git", "config"), encoding="utf-8") as file:
            hooked_config = file.read()
        saved_git_dir = os.environ.get("GIT_DIR")
        os.environ["GIT_DIR"] = os.path.join(hooked, ".git")
        try:
            for args in (["init", "-q"], ["config", "user.email", "t@t"], ["config", "user.name", "t"]):
                git(repo, *args)
        finally:
            if saved_git_dir is None:
                del os.environ["GIT_DIR"]
            else:
                os.environ["GIT_DIR"] = saved_git_dir
        with open(os.path.join(hooked, ".git", "config"), encoding="utf-8") as file:
            assert file.read() == hooked_config, "a git call of the self-test changed the config of the hook's repository"
        with open(os.path.join(repo, "a.txt"), "w", encoding="ascii") as file:
            file.write("a\n")
        git(repo, "add", "a.txt")
        git(repo, "commit", "-q", "-m", "a")
        trees = Trees(os.path.join(scratch, "trees"))
        SOURCE_REPOS["nirender"], saved = repo, SOURCE_REPOS["nirender"]
        try:
            tree = trees.ready("nirender", git(repo, "rev-parse", "HEAD"))
            assert trees.ready("nirender", git(repo, "rev-parse", "HEAD")) == tree, "a clean tree is taken again"
            def refused(change):
                try:
                    trees.ready("nirender", git(repo, "rev-parse", "HEAD"))
                    raise AssertionError("a tree with %s was taken" % change)
                except SetFault as fault:
                    assert "has changes" in str(fault), fault
                    return str(fault)

            with open(os.path.join(tree, "a.txt"), "w", encoding="ascii") as file:
                file.write("changed\n")
            refused("a changed file")
            with open(os.path.join(tree, "a.txt"), "w", encoding="ascii") as file:
                file.write("a\n")
            # The Reviewer's plant (verdict 452): an untracked file is a change too. The null: without it the tree is
            # taken again.
            stray = os.path.join(tree, "mod", "stray.lua")
            os.mkdir(os.path.dirname(stray))
            with open(stray, "w", encoding="ascii") as file:
                file.write("-- stray\n")
            assert "mod/" in refused("an untracked file")
            os.remove(stray)
            os.rmdir(os.path.dirname(stray))
            assert trees.ready("nirender", git(repo, "rev-parse", "HEAD")) == tree, "the tree without the stray file"
        finally:
            SOURCE_REPOS["nirender"] = saved
            git(repo, "worktree", "remove", "--force", tree)
        # The submodules of a worktree come from the main checkout's modules, and the main checkout's config stays as it
        # is. The null: the old form (`git config` in the worktree) changes it.
        sub, main = os.path.join(scratch, "sub"), os.path.join(scratch, "main")
        for folder in (sub, main):
            os.mkdir(folder)
            for args in (["init", "-q"], ["config", "user.email", "t@t"], ["config", "user.name", "t"]):
                git(folder, *args)
        with open(os.path.join(sub, "s.txt"), "w", encoding="ascii") as file:
            file.write("s\n")
        git(sub, "add", "s.txt")
        git(sub, "commit", "-q", "-m", "s")
        git(main, "-c", "protocol.file.allow=always", "submodule", "add", "-q", sub.replace("\\", "/"), "deps/sub")
        git(main, "commit", "-q", "-m", "m")
        before = git(main, "config", "--get-regexp", r"^submodule\.")
        linked = os.path.join(scratch, "linked")
        git(main, "worktree", "add", "-q", "--detach", linked, "HEAD")
        update_submodules(linked, main)
        assert os.path.isfile(os.path.join(linked, "deps", "sub", "s.txt")), "the submodule was not checked out"
        assert git(main, "config", "--get-regexp", r"^submodule\.") == before, "the main checkout's config changed"
        git(linked, "config", "submodule.deps/sub.url", "elsewhere")
        assert git(main, "config", "--get-regexp", r"^submodule\.") != before, "the null cannot fail"
        # The floor of free space, and the E: gate for trees on the disk of the sets.
        usage = lambda free: (lambda path: shutil._ntuple_diskusage(0, 0, free))  # noqa: E731
        low = Trees(os.path.join(scratch, "low"), gate=yes, disk_usage=usage(LEAST_FREE_BYTES - 1))
        try:
            low.room("a test")
            raise AssertionError("a disk under the floor was taken")
        except SetFault as fault:
            assert fault.code == EXIT_REFUSED and "the floor is 20 GB" in str(fault), fault
        Trees(os.path.join(scratch, "high"), gate=yes, disk_usage=usage(LEAST_FREE_BYTES)).room("a test")
        # A lost build hold stops the next build step too (verdict 507, addition 1); the null is the line above.
        held_lost = Trees(os.path.join(scratch, "high"), gate=yes, disk_usage=usage(LEAST_FREE_BYTES))
        held_lost.slot_hold = SlotHold()
        held_lost.slot_hold.lost = "build: nobody holds it: take it"
        try:
            held_lost.room("a test")
            raise AssertionError("a build step went on after the build hold was lost")
        except SetFault as fault:
            assert fault.code == EXIT_REFUSED and "the build slot was lost before a test" in str(fault), fault
        asked = []
        on_e = Trees(os.path.join(os.path.splitdrive(sets.SETS)[0] + os.sep, "no-such-folder-of-a-test"),
                     gate=lambda: asked.append(1) or (False, "a timing run holds the game"), disk_usage=usage(LEAST_FREE_BYTES))
        assert on_e.on_project_disk() and not Trees(os.path.join(scratch, "c")).on_project_disk()
        try:
            on_e.room("a test")
            raise AssertionError("a build on E: went on while the E: gate said no")
        except SetFault as fault:
            assert fault.code == EXIT_REFUSED and "no build on E: now" in str(fault) and asked == [1], fault
        in_hold_test(os.path.join(scratch, "slot-state"))
        part_inputs_test(scratch)
    sets.self_test()
    print("build_set self-test: ok")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--nirender", help="the nirender-plugin commit (default: master)")
    parser.add_argument("--render-link", help="the render-link commit (default: feature/wgpu-renderer)")
    parser.add_argument("--mwse", help="the MWSE commit (default: the commit that the test installs run)")
    parser.add_argument("--patch", action="append", default=[], help="REPO=FILE: one patch on that repo's commit")
    parser.add_argument("--trees", default=TREES, help="the folder of the build trees (default %s); on E: each build step asks e-ok one time and stops on a no" % TREES)
    parser.add_argument("--no-wait", action="store_true", help="stop with exit code 3 when the build slot is not free")
    parser.add_argument("--in-hold", metavar="LABEL", help="take no slot: this session holds the whole build side under LABEL (a merge round's take); "
                        "the maker checks it, notes and renews that hold, and stops with exit code 3 without it")
    parser.add_argument("--self-test", action="store_true", help="the tests on scratch folders: no build, no slot, no write to E:")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    try:
        names = {"nirender": args.nirender, "render-link": args.render_link, "mwse": args.mwse}
        if not all(names.values()):
            defaults = tips()
            names = {repo: name or defaults[repo] for repo, name in names.items()}
        commits = {repo: resolve(SOURCE_REPOS[repo], name) for repo, name in names.items()}
        patches = read_patches(args.patch)
    except SetFault as fault:
        say(str(fault))
        return fault.code
    say("the commits: %s" % ", ".join("%s %s (%s)" % (repo, commits[repo][:8], names[repo]) for repo in sets.REPOS))
    from make_play_set import load_slots

    slots = load_slots()
    label = "build_set: %s" % sets.set_name(commits, {repo: md5 for repo, (_, md5) in patches.items()})
    problem, hold = maker_hold(slots, args.in_hold, label, args.no_wait)
    if problem:
        say(problem)
        return EXIT_REFUSED
    code, lines = make(commits, patches, sets.KIND_USUAL, Trees(args.trees), hold=hold)
    for line in lines:
        say(line)
    return code


if __name__ == "__main__":
    sys.exit(main())
