"""What the tools share: the names of the renderer functions, the port macros, the report.

The game side of a run is in `session.py`.
"""
import functools
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
# The folder of the sibling repos (MGE-XE-G7-render-link, moreFPS), next to this one.
SIBLINGS = os.path.dirname(REPO)


# The prefix of the plugin's lines in MGE XE's log (`MGE_LogLine` of d3d8.dll).
PLUGIN_PREFIX = "nirender: "


def plugin_lines(folder):
    """The plugin's lines of the run in `folder`, without their line ends. With the d3d8.dll of
    MGE XE they are the lines of the folder's `mgeXE.log` (which the session copies there after
    the game) that start with `nirender: `, without the prefix; without that export (an older
    d3d8.dll, the d3d8.dll of this repository) they are the plugin's own `nirender.log` of the
    folder (the run spec's logFile)."""
    mge = os.path.join(folder, "mgeXE.log")
    if os.path.isfile(mge):
        lines = [line[len(PLUGIN_PREFIX):] for line in open(mge, encoding="utf-8", errors="replace").read().splitlines() if line.startswith(PLUGIN_PREFIX)]
        if lines:
            return lines
    own = os.path.join(folder, "nirender.log")
    return open(own, encoding="utf-8", errors="replace").read().splitlines() if os.path.isfile(own) else []


# What a replay reads from an MGE XE build folder: the helper, MGE XE's shaders (the post
# chain, the water) and its textures (the waves, the noise of the ambient occlusion). Without
# the Data Files the helper draws on and only logs the files it did not find, so the picture
# is another one.
BUILD_PARTS = ("numidium.exe", os.path.join("Data Files", "shaders"), os.path.join("Data Files", "textures", "MGE"))


def build_missing(mge_build):
    """The parts of `BUILD_PARTS` that the build folder `mge_build` lacks: an empty list when
    a replay can use it."""
    return [part for part in BUILD_PARTS if not os.path.exists(os.path.join(mge_build, part))]


def git_env():
    """The environment for a git call about a folder of our choice: without the GIT_ names, with
    which a git hook points git at its own repository (GIT_DIR, GIT_INDEX_FILE and others). A hook
    of another repository (the render-link worktree runs these checks) must not make git answer
    for that repository, and a `git init` with a hook's GIT_DIR writes core.bare = true into the
    config of the hook's repository."""
    return {name: value for name, value in os.environ.items() if not name.startswith("GIT_")}


def checkout_of(folder):
    """The folder of the main checkout of the repository at `folder`: `folder` itself, or the
    checkout that a worktree belongs to. None when `folder` is not in a git repository."""
    run = subprocess.run(["git", "rev-parse", "--path-format=absolute", "--git-common-dir"], cwd=folder,
                         capture_output=True, text=True, env=git_env())
    if run.returncode != 0 or not run.stdout.strip():
        return None
    return os.path.dirname(os.path.normpath(run.stdout.strip()))


@functools.lru_cache(maxsize=None)
def main_checkout():
    """The folder of the main checkout of this repository (`checkout_of(REPO)`)."""
    return checkout_of(REPO)


def find_local(repo, main):
    """The `local.json` of the tree `repo`, else the one of its main checkout `main`: the path
    and whether it is the main checkout's. (None, False) when neither has one."""
    own = os.path.join(repo, "local.json")
    if os.path.isfile(own):
        return own, False
    if main and os.path.normcase(main) != os.path.normcase(repo) and os.path.isfile(os.path.join(main, "local.json")):
        return os.path.join(main, "local.json"), True
    return None, False


@functools.lru_cache(maxsize=None)
def local_file():
    """The `local.json` that `local` reads (`find_local`): the file is not tracked, so a new
    worktree has none, and then it reads the main checkout's. Every key of that file applies to
    the worktree. The main checkout's file and its keys are named once on stderr, so that the
    output shows where the places came from."""
    path, from_main = find_local(REPO, main_checkout())
    if from_main:
        keys = sorted(json.load(open(path, encoding="utf-8")))
        print("local.json: %s (the main checkout's; this tree has none), keys: %s" % (path, ", ".join(keys) or "none"),
              file=sys.stderr)
    return path


def local_file_hint():
    """Where to set a key of `local()` that is missing: the file it reads, else where to make one."""
    path = local_file()
    if path:
        return path
    own = os.path.join(REPO, "local.json")
    main = main_checkout()
    if main and os.path.normcase(main) != os.path.normcase(REPO):
        return "a new %s or %s" % (own, os.path.join(main, "local.json"))
    if main:
        return "a new " + own
    return "a new %s (this tree is not in a git repository, so there is no main checkout)" % own


def local(key, default):
    """A place of this machine that a tool needs and the repo does not hold: the key of
    `local_file()` (not tracked; `local.example.json` names the keys), else `default`, a place
    next to this repo or under the home folder."""
    path = local_file()
    if path:
        values = json.load(open(path, encoding="utf-8"))
        if key in values:
            return values[key]
    return default


def find_sibling(siblings, main, parts):
    """The place `parts` in the folder `siblings`, else next to the main checkout `main`. The
    place in `siblings` when neither exists."""
    near = os.path.join(siblings, *parts)
    if os.path.exists(near) or not main:
        return near
    far = os.path.join(os.path.dirname(main), *parts)
    return far if os.path.exists(far) else near


def sibling(*parts):
    """The place `parts` next to this repo, else next to the main checkout (`find_sibling`; a
    worktree away from the repos folder has no siblings but its own)."""
    return find_sibling(SIBLINGS, main_checkout(), parts)


def self_test():
    """The tests of `checkout_of`, `find_local` and `find_sibling` in a temporary folder. A commit
    hook runs this test with the GIT_ names of its own repository: every git call of the test
    goes without them (`git_env`), and the test fails when core.bare of this repository's main
    checkout changes."""
    import contextlib
    import tempfile

    def git(*args):
        return subprocess.run(["git", *args], check=True, env=git_env(), capture_output=True, text=True).stdout.strip()

    def bare(folder):
        run = subprocess.run(["git", "-C", folder, "config", "--get", "core.bare"], env=git_env(), capture_output=True, text=True)
        return run.stdout.strip()

    @contextlib.contextmanager
    def names(**values):
        saved = {name: os.environ.get(name) for name in values}
        os.environ.update(values)
        try:
            yield
        finally:
            for name, value in saved.items():
                if value is None:
                    os.environ.pop(name, None)
                else:
                    os.environ[name] = value

    real = main_checkout()
    real_bare = bare(real) if real else None
    with tempfile.TemporaryDirectory() as root:
        main = os.path.join(root, "repos", "plugin")
        tree = os.path.join(root, "away", "plugin")
        other = os.path.join(root, "other")
        os.makedirs(main)
        for folder in (main, other):
            git("init", "-q", folder)
            git("-C", folder, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "t")
        git("-C", main, "worktree", "add", "-q", "--detach", tree)
        same = os.path.normcase
        assert same(checkout_of(tree)) == same(main), checkout_of(tree)
        assert checkout_of(root) is None
        # A hook of another repository sets its own GIT_DIR: the answer stays the tree's.
        with names(GIT_DIR=os.path.join(other, ".git"), GIT_WORK_TREE=other):
            assert same(checkout_of(tree)) == same(main), checkout_of(tree)
        # The names of a commit hook in the worktree: a `git init` of the test makes its own
        # repository and leaves core.bare of the hook's repository as it was.
        hook_dir = git("-C", tree, "rev-parse", "--absolute-git-dir")
        assert bare(main) == "false", bare(main)
        with names(GIT_DIR=hook_dir, GIT_INDEX_FILE=os.path.join(hook_dir, "index")):
            git("init", "-q", os.path.join(root, "third"))
        assert bare(main) == "false", bare(main)
        assert os.path.isdir(os.path.join(root, "third", ".git"))
        assert find_local(tree, main) == (None, False)
        with open(os.path.join(main, "local.json"), "w", encoding="ascii") as file:
            file.write("{}")
        assert find_local(tree, main) == (os.path.join(main, "local.json"), True)
        assert find_local(main, main) == (os.path.join(main, "local.json"), False)
        with open(os.path.join(tree, "local.json"), "w", encoding="ascii") as file:
            file.write("{}")
        assert find_local(tree, main) == (os.path.join(tree, "local.json"), False)
        assert find_local(tree, None) == (os.path.join(tree, "local.json"), False)
        os.makedirs(os.path.join(root, "repos", "docs"))
        assert find_sibling(os.path.join(root, "away"), main, ["docs"]) == os.path.join(root, "repos", "docs")
        assert find_sibling(os.path.join(root, "away"), main, ["none"]) == os.path.join(root, "away", "none")
        assert find_sibling(os.path.join(root, "away"), None, ["docs"]) == os.path.join(root, "away", "docs")
        os.makedirs(os.path.join(root, "away", "docs"))
        assert find_sibling(os.path.join(root, "away"), main, ["docs"]) == os.path.join(root, "away", "docs")
    if real:
        assert bare(real) == real_bare, "core.bare of %s changed from %r to %r" % (real, real_bare, bare(real))
    print("common self-test: passed")

# The functions of the renderer subsystem, by their name in the IDA database.
FUNC_RE = re.compile(r"^(NiDX8|NiD3D|NiRenderState|NiRenderer|NiGetD3DFormat|NiPropertyState"
                     # The scene traversal (plan 0.5 entry T2, 2026-10-16).
                     r"|NiCamera::Click$|NiAVObject::CullShow$|NiNode::Display$|NiNode::IsVisualObject$|NiTriShape::Display$"
                     r"|NiParticles::Display$|NiLines::Display$|NiAutoNormalParticles::Display$|NiSwitchNode::Display$"
                     r"|NiFltAnimationNode::Display$|NiBSPNode::Display$|NiBillboardNode::Display$|NiBillboardNode::RotateToCamera$"
                     r"|NiSortAdjustNode::Display$|NiGeometry::pushStateToRenderer$|NiAccumulator::|NiClusterAccumulator::"
                     r"|NiAlphaAccumulator::RegisterObject$|NiBound::WhichSide$"
                     r"|BSAnimationNode::Display$|NiLODNode::Display$|NiTriStrips::Display$"
                     # The readers of the display flag (plan F8, part 1, 2026-10-17).
                     r"|BSAnimationManager::Update$|BSAnimationNode::UpdateControllers$"
                     r"|BSAnimationNode::UpdateDownwardPass$"
                     # The update of a subtree from its root (D6 step 2, 2026-10-19; MWSE hooks
                     # its entry, so it is in the table but never a port).
                     r"|NiAVObject::Update$"
                     r"|NiNode::UpdateDownwardPass$"
                     # The new property and effect states of a shape (plan 8b, 2026-10-17).
                     r"|NiGeometry::UpdatePropertiesDownward$|NiGeometry::UpdateEffectsDownward$"
                     # The events of the table walk (entry 13, 2026-10-17; the walk and its ports
                     # were removed on 2026-10-20, the functions stay in the table for the census).
                     r"|NiGeometry::UpdateWorldData$|NiAVObject::SetAppCulled$"
                     r"|NiNode::AttachChild$|NiNode::DetachChild$|NiNode::DetachChildAt$|NiNode::SetChildAt$"
                     r"|ActorAnimController::setOpacity$"
                     r"|NiFlipController::Update$|NiAlphaController::Update$|NiMaterialColorController::Update$"
                     r"|NiUVController::Update$|WorldController::updateEnchantedItemEffect$"
                     r"|NiGeometry::dtor$)")
FUNC_SKIP = re.compile(r"_SEH$|NiDX8Application|NiDX8SelectResolution|StdMap")

# A port declaration in src/ports/*.rs: the module name, the address, the Rust function.
PORT_RE = re.compile(r"port_(?:thiscall|cdecl)!\(\s*(\w+),\s*0x([0-9A-Fa-f]+),.*?rust\s*=\s*([\w:]+)\s*\)", re.S)


def is_renderer_function(name):
    return bool(FUNC_RE.search(name)) and not FUNC_SKIP.search(name)


def read_report(path):
    """The rows of a report.tsv, each as a dict from column name to text."""
    if not os.path.isfile(path):
        return []
    lines = open(path, encoding="utf-8").read().splitlines()
    if not lines:
        return []
    header = lines[0].split("\t")
    return [dict(zip(header, line.split("\t"))) for line in lines[1:] if line.strip()]


if __name__ == "__main__":
    if sys.argv[1:] != ["--self-test"]:
        raise SystemExit("usage: common.py --self-test")
    self_test()
