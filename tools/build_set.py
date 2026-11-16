"""Makes a sealed build set (W521, the owner's D490): one folder with every binary of a run, from three commits.

    python -I tools/build_set.py [--nirender C] [--render-link C] [--mwse C] [--patch REPO=FILE] [--no-wait]
    python -I tools/build_set.py --self-test    # scratch folders and fake builds: no build, no slot, no write to E:

The commits default to the tips: nirender `master`, render-link `feature/wgpu-renderer`, and the MWSE commit that the
test installs run (the records of deploy_rest.py, `mwse_tree.installed_commit`). A commit is any name that git resolves
in its repository. The form of a set, its name, its parts and its check are in sets.py.

Each of the five parts is made one time for its key. A part that a sealed set in E:/nirender-keep/mgebuilds has already,
with the same key and kind, is copied from that set (its check is the proof), and no new build of it is used: so two
sets that differ in one commit differ only in the parts of that commit, byte for byte. Else the tool builds: the plugin
(`cargo build --release`) and `cargo xtask dist` in clean detached trees in C:/tmp/buildset (siblings, as the dist
needs), and MWSE.dll with MSBuild (Release, x86) in a detached worktree of the MWSE repository there, its LuaJIT built
by build_luajit.bat of the docs and its MorrowindDir a folder of C:/tmp/buildset. The MWSE core Lua comes from that
tree. A tree with changes makes no set.

--patch REPO=FILE (REPO nirender, render-link or mwse) applies ONE patch file to the clean tree of that repo before the
build. The set holds the patch as patches/<repo>.patch, the manifest has its md5, and the name has the mark +p<8 hex>
after that commit; a part of a patched repo is never copied to or from a set without that patch. A second --patch for
one repo is refused: make a commit. After the build, `git apply -R` takes the patch out of the tree again.

Before it writes to E:, the tool checks the build ids (the plugin and the host crate name the nirender commit,
numidium.exe and d3d8.dll the render-link commit; "-dirty" only for a repo with a patch) and runs the probe that
deploy_rest.py runs: the plugin's needs against the set's MWSE.dll, tools/check_mwse_sites.py --features --mge of the
trees at the set's commits. Its exit code 4 makes no set. The pdbs of the binaries go to E:/nirender-keep/pdb/<name>,
each only when it is the pdb of its binary (symbols.pdb_matches); a copied part takes its pdbs from the pdb folder of the
set that it comes from.

The set is made in <name>.making and renamed at the end, so a folder with the name is whole or is not there. If the
name is there and its check passes, nothing is built. If the name is there and its check fails, or <name>.making is
there, the tool stops and names the folder: it never builds a second time into a name, and it deletes nothing (until
the delete right of D492 is in, an unfinished folder is the owner's to remove). The tool takes the build slot for its
builds and the probe and gives it back at its end, and each write to E: waits for `slot.py e-ok` (D297). One run of the
tool at a time on this machine.

The last line is the set's folder. Exit code 0; 1 for a fault; 3 when a slot or the E: gate refuses; 4 when the probe
finds a missing MWSE feature.
"""
import argparse
import contextlib
import datetime
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import sets  # noqa: E402
from common import BUILD_ID_RE, REPO, docs_place, local, main_checkout, same_commit  # noqa: E402
from check_mwse_sites import MISSING, MISSING_FEATURE_EXIT  # noqa: E402
from deploy_rest import MWSE_REPO  # noqa: E402
from make_play_set import MGE_BRANCH, PDBS, PDB_KEEP, PLUGIN_BRANCH, hold_run_lock, inside, say  # noqa: E402
from new_run_folder import e_gate, this_session  # noqa: E402
import symbols  # noqa: E402

TREES = "C:/tmp/buildset"
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
            "numidium": "numidium.exe", "mwse": "MWSE.dll"}
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


class SetFault(Exception):
    """A fault that makes no set; the message is its line. `code` is the exit code."""

    def __init__(self, text, code=EXIT_FAULT):
        super().__init__(text)
        self.code = code


def git(folder, *args):
    done = subprocess.run(["git", "-C", folder, *args], capture_output=True, text=True)
    if done.returncode != 0:
        raise SetFault("git %s in %s: %s" % (" ".join(args), folder, done.stderr.strip()))
    return done.stdout.strip()


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


def part_keys(commits, tree, patch_md5s):
    """The key of each part (the Architect's rule, W521): its own repo's commit and, for each thing of another repo that
    its build reads, the smallest name that holds it: the git tree of a folder, or the commit for a whole crate. Each
    with its patch mark (sets.py). `tree(repo, folder)` gives the git tree of a folder at the set's commit.

    - plugin (the DLL): the nirender commit; its build reads nothing outside nirender.
    - plugin-lua: the nirender commit, and render-link's xtask/, which picks the nirender Lua files of the dist
      (`collect_plugin`). So an xtask change makes new Lua files and keeps the plugin's bytes.
    - render-link: its commit, and nirender's contract/ (d3d8/build.rs includes mge_render_link.h from it).
    - numidium: the render-link and the nirender commits (the host crate is a path dependency).
    - mwse: the MWSE commit (its submodules are part of it)."""
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


def built_parts(stage, mwse):
    """{part: {path in the set: source file}} of a new build: the dist stage `stage`, and `mwse` (the MWSE.dll and the
    core folder) when MWSE was built."""
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


def write_manifest(folder, name, kind, commits, patches, parts_from, keys, now, session):
    folders, files, links = sets.walk(folder)
    if links:
        raise SetFault("the new set holds links or junctions: %s" % ", ".join(links))
    manifest = {
        "form": sets.FORM,
        "name": name,
        "kind": kind,
        "commits": {repo: {"commit": commits[repo], "clean": True} for repo in sets.REPOS},
        "patches": {repo: {"file": "%s/%s.patch" % (sets.PATCHES, repo), "md5": md5} for repo, (_, md5) in sorted(patches.items())},
        "parts": {part: {"key": keys[part], "from": parts_from[part]} for part in sets.PARTS},
        # The helper of this build cannot give its format yet (step 4 of the design): a format of its own (3.4).
        "distant_land_format": "own-%s" % commits["render-link"][:8],
        "tool": {"name": "build_set.py", "commit": tool_version()},
        "made": now.isoformat(timespec="seconds"),
        "session": session,
        "folders": folders,
        "files": {path: {"size": os.path.getsize(os.path.join(folder, *path.split("/"))),
                         "md5": sets.md5_of(os.path.join(folder, *path.split("/")))} for path in files if path != sets.MANIFEST},
    }
    with open(os.path.join(folder, sets.MANIFEST), "w", encoding="utf-8", newline="\n") as file:
        json.dump(manifest, file, indent=1, sort_keys=True)
        file.write("\n")
    return manifest


# --- the builds -------------------------------------------------------------------------------

class Trees:
    """The real builds, in clean detached trees in `root`."""

    def __init__(self, root=TREES, gate=e_gate, disk_usage=shutil.disk_usage):
        self.root = root
        self.log = os.path.join(root, "build_set.log")
        self.trees = {}
        self.applied = []
        self.gate = gate
        self.disk_usage = disk_usage

    def on_project_disk(self):
        """The trees are on the disk of the sets (E:): each build step there is a write to E: (D297)."""
        return os.path.splitdrive(os.path.abspath(self.root))[0].lower() == os.path.splitdrive(os.path.abspath(sets.SETS))[0].lower()

    def room(self, what):
        """Stops before `what` when the disk of the trees has less than LEAST_FREE_BYTES free, or when the trees are on
        E: and the E: gate says no."""
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
            changed = git(tree, "status", "--porcelain", "--untracked-files=no", "--ignore-submodules=untracked")
            if changed:
                raise SetFault("the tree %s has changes, and no set is made:\n%s" % (tree, changed))
            git(tree, "checkout", "--quiet", "--detach", commit)
        if repo == "mwse":
            # The submodules come from the modules of the MWSE checkout on this machine, not from the network.
            git(tree, "submodule", "init")
            for path in git(tree, "config", "--file", ".gitmodules", "--get-regexp", r"submodule\..*\.path").splitlines():
                module = path.split()[1]
                local = os.path.join(SOURCE_REPOS["mwse"], ".git", "modules", *module.split("/"))
                if os.path.isdir(local):
                    git(tree, "config", "submodule.%s.url" % module, local.replace("\\", "/"))
            git(tree, "-c", "protocol.file.allow=always", "submodule", "update")
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

    def step(self, command, folder, env=None):
        self.room(" ".join(os.path.basename(part) for part in command[:2]))
        env = dict(env or os.environ)
        env.pop("CARGO_TARGET_DIR", None)
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

    def build_mwse(self):
        """MWSE.dll and its core folder."""
        tree = self.trees["mwse"]
        stamp = os.path.join(self.root, "MWSE-luajit.txt")
        luajit = git(tree, "rev-parse", "HEAD:deps/rubic0n")
        built_for = open(stamp, encoding="ascii").read().strip() if os.path.isfile(stamp) else ""
        if built_for != luajit or not os.path.isfile(os.path.join(tree, "deps", "rubic0n", "src", "lua51.lib")):
            self.step(["cmd", "/c", LUAJIT_BAT, os.path.normpath(tree)], tree)
            with open(stamp, "w", encoding="ascii") as file:
                file.write(luajit + "\n")
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
        return found

    def probe(self, dll):
        """The probe of deploy_rest.py, in the tree at the set's nirender commit: (exit code, output)."""
        tree = self.trees["nirender"]
        self.room("the probe (it compiles the plugin's tests)")
        command = [sys.executable, "-I", os.path.join(tree, "tools", "check_mwse_sites.py"), "--features", "--mge", self.trees["render-link"], dll]
        done = subprocess.run(command, cwd=tree, capture_output=True, text=True, encoding="utf-8", errors="replace", stdin=subprocess.DEVNULL)
        return done.returncode, done.stdout + done.stderr


@contextlib.contextmanager
def build_hold(slots, label, no_wait):
    """The build slot for the builds and the probe, noted each minute (a child's compile is not seen by the slot's own
    notes), and given back at the end."""
    from merge_step import take_build

    if not take_build(slots, label, no_wait):
        raise SetFault("no build slot", EXIT_REFUSED)
    stop = threading.Event()

    def note():
        while not stop.wait(NOTE_SECONDS):
            slots.note_holder_build(slots.owner_of(slots.session_id(), label), time.time())

    thread = threading.Thread(target=note, daemon=True)
    thread.start()
    try:
        yield
    finally:
        stop.set()
        thread.join()
        say(slots.release("build", label).text.splitlines()[0])


# --- the set ------------------------------------------------------------------------------------

def make(commits, patches, kind, builder, root=sets.SETS, gate=e_gate, hold=None, keep=PDB_KEEP, session=this_session,
         now=None, reuse=True):
    """Makes the set of `commits` ({repo: full commit}) with `patches` ({repo: (file, md5)}). `builder` does the builds
    (Trees, or a fake in the tests); `hold` is a context manager for the build slot (None: no slot, the tests). Returns
    (exit code, lines); the last line is the set's folder. `reuse` False builds every part (only the null of a test)."""
    patch_md5s = {repo: md5 for repo, (_, md5) in patches.items()}
    name = sets.set_name(commits, patch_md5s, kind)
    folder = os.path.join(root, name)
    making = folder + ".making"
    lines = []
    if os.path.exists(folder):
        try:
            sets.check(folder)
        except sets.SetError as error:
            return EXIT_FAULT, ["%s is there and is not a sealed set, and nothing is built: %s" % (name, error)]
        return 0, ["%s is there and its check passes: nothing is built" % name, folder.replace("\\", "/")]
    if os.path.exists(making):
        return EXIT_FAULT, ["%s is there: an unfinished set or another run that makes it; nothing is built" % making.replace("\\", "/")]
    keys = part_keys(commits, lambda repo, folder: builder.tree(repo, commits[repo], folder), patch_md5s)
    found = {part: sets.find_part(root, part, keys[part], kind) if reuse else None for part in sets.PARTS}
    to_build = [part for part in sets.PARTS if found[part] is None]
    for part in sets.PARTS:
        say("part %s: %s" % (part, "copied from %s" % found[part][0] if found[part] else "built"))
    try:
        with hold() if hold else contextlib.nullcontext():
            for repo in ("nirender", "render-link"):
                builder.ready(repo, commits[repo])
            if "mwse" in to_build:
                builder.ready("mwse", commits["mwse"])
            for repo, (patch_file, _) in sorted(patches.items()):
                if repo in builder.trees:
                    builder.patch(repo, patch_file)
            stage = builder.build_cargo() if set(to_build) & CARGO_PARTS else None
            mwse = builder.build_mwse() if "mwse" in to_build else None
            built = built_parts(stage, mwse)
            files = {}
            for part in sets.PARTS:
                part_files = set_part_files(root, found[part][0], found[part][1], part) if found[part] else built[part]
                if not any(path.lower() == REQUIRED[part].lower() for path in part_files):
                    raise SetFault("the part %s has no %s" % (part, REQUIRED[part]))
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
            allowed, gate_line = gate()
            if not allowed:
                raise SetFault("no set made: the E: gate says no: %s" % gate_line, EXIT_REFUSED)
            try:
                os.mkdir(making)
            except FileExistsError:
                raise SetFault("%s appeared: another run makes this set" % making) from None
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
            lines += ["pdb: " + line for line in keep_pdbs(name, making, pdbs, keep, root)]
            lines += builder.size_lines()
    except SetFault as fault:
        return fault.code, lines + builder.size_lines() + [str(fault)]
    finally:
        builder.unpatch()
    parts_from = {part: found[part][0] if found[part] else "built" for part in sets.PARTS}
    write_manifest(making, name, kind, commits, patches, parts_from, keys, now or datetime.datetime.now().astimezone(), session())
    os.rename(making, folder)
    try:
        manifest = sets.check(folder)
    except sets.SetError as error:
        return EXIT_FAULT, lines + ["the new set fails its own check: %s" % error]
    lines.append("%s: %d files, manifest md5 %s" % (name, len(manifest["files"]), sets.manifest_md5(folder)))
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
        self.probe_code = probe_code
        self.probed = []
        self.wrong_render_link = wrong_render_link
        # The git trees of the folders that a key names, by (repo, folder): "c" * 40 unless a test sets one.
        self.trees_of = {}

    def tree(self, repo, commit, folder):
        return self.trees_of.get((repo, folder), "c" * 40)

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
    """Today's runner takes the set as a plain --mge-build folder: its plugin copy with no WARNING, also when
    --plugin-dll names that copy, and every file that it stages is there. Returns the faults."""
    import run_own_device
    import session

    faults = []
    copy = os.path.join(folder, sets.plugin_copy(sets.read_manifest(folder)))
    dll, source = run_own_device.plugin_dll_of(None, {}, "checkout.dll", folder)
    if (dll, source) != (copy, run_own_device.FOLDER_PLUGIN_SOURCE):
        faults.append("the runner takes %s (%s), not the set's copy" % (dll, source))
    if run_own_device.plugin_warning(folder, dll, source) is not None:
        faults.append("the runner warns: %s" % run_own_device.plugin_warning(folder, dll, source))
    if run_own_device.plugin_warning(folder, copy, "--plugin-dll") is not None:
        faults.append("the runner warns with --plugin-dll of the copy")
    faults += ["the runner stages %s, and it is not there" % source for source in session.mge_build_files(folder).values() if not os.path.isfile(source)]
    return faults


def self_test():
    commits = {"nirender": "6d1b214" + "a" * 33, "render-link": "d9741db" + "b" * 33, "mwse": "89198b53" + "c" * 32}
    yes = lambda: (True, "e-ok: yes")  # noqa: E731
    session = lambda: "test (id)"  # noqa: E731
    with tempfile.TemporaryDirectory(prefix="build-set-") as scratch:
        root, keep = os.path.join(scratch, "sets"), os.path.join(scratch, "pdb")
        os.mkdir(root)
        builder = FakeBuilder(scratch)

        def run(commits_of, builder_of=None, patches=None, gate=yes, reuse=True):
            return make(commits_of, patches or {}, sets.KIND_USUAL, builder_of or builder, root, gate, None, keep, session, reuse=reuse)

        # A set from nothing: every part built, the set passes its check, today's runner takes it.
        code, lines = run(commits)
        assert code == 0, lines
        first = lines[-1]
        assert os.path.basename(first) == "s-6d1b214a-d9741dbb-89198b53" and builder.cargo_builds == 1 and builder.mwse_builds == 1, lines
        manifest = sets.check(first)
        assert manifest["parts"]["plugin"]["from"] == "built" and manifest["distant_land_format"] == "own-d9741dbb"
        assert manifest["commits"]["mwse"] == {"commit": commits["mwse"], "clean": True}
        assert "akulakhan-6d1b214.dll" in manifest["files"] and "MWSE.dll" in manifest["files"]
        assert "Data Files/MWSE/core/initialize.lua" in manifest["files"] or "Data Files/mwse/core/initialize.lua" in manifest["files"]
        assert not [path for path in manifest["files"] if path.endswith(".pdb")], "a pdb went into the set"
        assert runner_compatible(first) == [], runner_compatible(first)
        # The null of that check: a folder without the plugin copy fails it (the runner takes the checkout's build).
        bare = os.path.join(scratch, "bare", os.path.basename(first))
        shutil.copytree(first, bare)
        os.remove(os.path.join(bare, sets.plugin_copy(manifest)))
        assert any("not the set's copy" in fault for fault in runner_compatible(bare)), "the runner check cannot fail"
        # Test 1: the same commits again build nothing.
        code, lines = run(commits)
        assert code == 0 and "nothing is built" in lines[0] and builder.cargo_builds == 1 and builder.mwse_builds == 1, lines
        # Test 8: another MWSE commit builds only MWSE; the other parts are the same bytes.
        other = dict(commits, mwse="1234abcd" + "d" * 32)
        code, lines = run(other)
        assert code == 0 and builder.cargo_builds == 1 and builder.mwse_builds == 2, lines
        second = sets.check(lines[-1])
        for path in ("d3d8.dll", "numidium.exe", "akulakhan-6d1b214.dll"):
            assert second["files"][path]["md5"] == manifest["files"][path]["md5"], path
        assert second["parts"]["plugin"]["from"] == os.path.basename(first)
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
        assert code == 0 and keyed["parts"]["plugin-lua"]["from"] == "built", lines
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
        assert code == 1 and ".making is there" in lines[-1], lines
        # Test 6: a tree with changes makes no set (a scratch repository and the real Trees.ready).
        repo = os.path.join(scratch, "repo")
        os.mkdir(repo)
        for args in (["init", "-q"], ["config", "user.email", "t@t"], ["config", "user.name", "t"]):
            git(repo, *args)
        with open(os.path.join(repo, "a.txt"), "w", encoding="ascii") as file:
            file.write("a\n")
        git(repo, "add", "a.txt")
        git(repo, "commit", "-q", "-m", "a")
        trees = Trees(os.path.join(scratch, "trees"))
        SOURCE_REPOS["nirender"], saved = repo, SOURCE_REPOS["nirender"]
        try:
            tree = trees.ready("nirender", git(repo, "rev-parse", "HEAD"))
            assert trees.ready("nirender", git(repo, "rev-parse", "HEAD")) == tree, "a clean tree is taken again"
            with open(os.path.join(tree, "a.txt"), "w", encoding="ascii") as file:
                file.write("changed\n")
            try:
                trees.ready("nirender", git(repo, "rev-parse", "HEAD"))
                raise AssertionError("a tree with changes was taken")
            except SetFault as fault:
                assert "has changes" in str(fault)
        finally:
            SOURCE_REPOS["nirender"] = saved
            git(repo, "worktree", "remove", "--force", tree)
        # The floor of free space, and the E: gate for trees on the disk of the sets.
        usage = lambda free: (lambda path: shutil._ntuple_diskusage(0, 0, free))  # noqa: E731
        low = Trees(os.path.join(scratch, "low"), gate=yes, disk_usage=usage(LEAST_FREE_BYTES - 1))
        try:
            low.room("a test")
            raise AssertionError("a disk under the floor was taken")
        except SetFault as fault:
            assert fault.code == EXIT_REFUSED and "the floor is 20 GB" in str(fault), fault
        Trees(os.path.join(scratch, "high"), gate=yes, disk_usage=usage(LEAST_FREE_BYTES)).room("a test")
        asked = []
        on_e = Trees(os.path.join(os.path.splitdrive(sets.SETS)[0] + os.sep, "no-such-folder-of-a-test"),
                     gate=lambda: asked.append(1) or (False, "a timing run holds the game"), disk_usage=usage(LEAST_FREE_BYTES))
        assert on_e.on_project_disk() and not Trees(os.path.join(scratch, "c")).on_project_disk()
        try:
            on_e.room("a test")
            raise AssertionError("a build on E: went on while the E: gate said no")
        except SetFault as fault:
            assert fault.code == EXIT_REFUSED and "no build on E: now" in str(fault) and asked == [1], fault
    sets.self_test()
    print("build_set self-test: ok")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--nirender", help="the nirender-plugin commit (default: master)")
    parser.add_argument("--render-link", help="the render-link commit (default: feature/wgpu-renderer)")
    parser.add_argument("--mwse", help="the MWSE commit (default: the commit that the test installs run)")
    parser.add_argument("--patch", action="append", default=[], help="REPO=FILE: one patch on that repo's commit")
    parser.add_argument("--trees", default=TREES, help="the folder of the build trees (default %s); on E: each build step waits for e-ok" % TREES)
    parser.add_argument("--no-wait", action="store_true", help="stop with exit code 3 when the build slot is not free")
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
    os.makedirs(TREES, exist_ok=True)
    if hold_run_lock(os.path.join(TREES, "build_set.lock")) is None:
        say("another run of build_set.py is at work, and this one stops")
        return EXIT_REFUSED
    from make_play_set import load_slots

    slots = load_slots()
    label = "build_set: %s" % sets.set_name(commits, {repo: md5 for repo, (_, md5) in patches.items()})
    code, lines = make(commits, patches, sets.KIND_USUAL, Trees(args.trees), hold=lambda: build_hold(slots, label, args.no_wait))
    for line in lines:
        say(line)
    return code


if __name__ == "__main__":
    sys.exit(main())
