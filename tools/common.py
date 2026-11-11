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


# The exit code of a tool whose stage cannot judge: merge_step.py prints "NOT RUN" with the reason of the tool's last line
# that starts with "NOT RUN".
NOT_RUN_EXIT = 5

# The build id that each part of the renderer carries as ASCII, "<program> v<version> <commit>[-dirty]"
# (build_support/build_id.rs): the plugin (`BUILD_ID` of src/lib.rs), the host crate in numidium.exe (`BUILD_ID` of
# host/src/lib.rs), and the two programs of render-link. By part: the file of a dist build and the programs it holds.
PART_PROGRAMS = (
    ("akulakhan.dll", os.path.join("Data Files", "mwse", "lib", "akulakhan.dll"), ("nirender",)),
    ("numidium.exe", "numidium.exe", ("nirender-host", "numidium")),
    ("d3d8.dll", "d3d8.dll", ("MGE-XE G7 Fork",)),
)
BUILD_ID_RE = re.compile(rb"(nirender-host|nirender|numidium|MGE-XE G7 Fork) v[0-9.]+ ([0-9a-f]{7,40}|unknown)(-dirty)?")


def part_ids(build):
    """The commit of each program in the parts of the build folder `build`, as {program: commit with its "-dirty"}, and
    the problems as a list of texts: a missing part, a program with no build id, a program with two different ids."""
    commits, problems = {}, []
    for label, path, programs in PART_PROGRAMS:
        full = os.path.join(build, path)
        if not os.path.isfile(full):
            problems.append("%s is not in the build" % label)
            continue
        with open(full, "rb") as file:
            found = {(m.group(1).decode(), m.group(2).decode() + (m.group(3) or b"").decode()) for m in BUILD_ID_RE.finditer(file.read())}
        for program in programs:
            ids = sorted(commit for name, commit in found if name == program)
            if len(ids) != 1:
                problems.append("%s holds %s build id of %s%s" % (label, "no" if not ids else "%d different" % len(ids), program, ": " + ", ".join(ids) if ids else ""))
            else:
                commits[program] = ids[0]
    return commits, problems


def same_commit(one, other):
    """True when two short hashes name one commit: the longer starts with the shorter (git may give more than 7 digits)."""
    one, other = one.lower(), other.lower()
    return bool(one) and bool(other) and (one.startswith(other) or other.startswith(one))


def parts_problems(build, plugin_commit=None, mge_commit=None):
    """The commits of nirender-plugin and render-link that the parts of `build` are one build of, as (plugin, mge), and why
    they are not, as texts (an empty list when they are). The plugin and the host crate in numidium.exe must name one
    commit of nirender-plugin, numidium and d3d8.dll one of render-link. A "-dirty" part names no commit. With
    `plugin_commit` or `mge_commit` (the commits that a merge step checks), the parts must name those; not a HEAD, which
    a merge can move while a build waits."""
    commits, problems = part_ids(build)
    found = []
    for programs, wanted, repo in ((("nirender", "nirender-host"), plugin_commit, "nirender-plugin"), (("numidium", "MGE-XE G7 Fork"), mge_commit, "render-link")):
        named = [(program, commits[program]) for program in programs if program in commits]
        for program, commit in named:
            if commit.endswith("-dirty"):
                problems.append("%s is %s: a build of a work tree with changes names no commit" % (program, commit))
        if len(named) == 2 and not same_commit(named[0][1], named[1][1]):
            problems.append("%s is %s, but %s is %s: two commits of %s" % (named[0] + named[1] + (repo,)))
        elif wanted and named and not same_commit(named[0][1], wanted):
            problems.append("%s is %s, but the commit of %s under test is %s" % (named[0] + (repo, wanted)))
        found.append(named[0][1] if named else None)
    return tuple(found), problems


# The exit codes of a helper that refuses a stream: 10 the stream format of the plugin, 11 the extension format of
# d3d8.dll (EXIT_STREAM_FORMAT and EXIT_EXTENSION_FORMAT of src/stream_format.rs), and the last line of such a refusal
# in a replay (host/src/cli.rs, renderer.rs of render-link).
REFUSED_FORMAT_EXITS = (10, 11)
CAPTURE_AGAIN = "make the capture again with this build"
# The start of the line that replay.py adds to such a refusal.
CAPTURE_FORMAT = "CAPTURE FORMAT"
# The end of such a line when the capture has a newer format than the replaying build reads.
OLDER_THAN_CAPTURE = "the build under test is older than the capture"
WRITER_RE = re.compile(r"^((?:akulakhan|d3d8)\.dll \([^)]*\)) writes (?:the (stream|extension) format (\d+)|.*)", re.M)
READER_RE = re.compile(r"reads the (stream|extension) format (\d+)")


def capture_format_line(capture, code, output):
    """For a replay that the helper refused, the line that names the capture as the other part, or "". In a replay the
    capture is the writer: the plugin and the d3d8.dll that made it wrote the stream, and the helper of the build under
    test reads it. The helper's own text says to install the parts from one archive, which is the advice for a game;
    in a replay the parts of the build may match, and the capture is from another build."""
    if code not in REFUSED_FORMAT_EXITS or CAPTURE_AGAIN not in output:
        return ""
    writer, reader = WRITER_RE.search(output), READER_RE.search(output)
    if not writer or not reader:
        return "%s: %s is refused by the helper of the build under test (exit code %d); its parts may match: the capture is from another build" % (CAPTURE_FORMAT, capture, code)
    wrote = "the %s format %s" % (writer.group(2), writer.group(3)) if writer.group(3) else "no format number"
    newer = writer.group(3) is not None and int(writer.group(3)) > int(reader.group(2))
    advice = OLDER_THAN_CAPTURE if newer else "make the capture again with this build, if the format change of this build is intended"
    return "%s: %s was made by %s, which writes %s; the build under test reads the %s format %s: %s" % (
        CAPTURE_FORMAT, capture, writer.group(1), wrote, reader.group(1), reader.group(2), advice)


def capture_format_in(output):
    """The CAPTURE FORMAT line of replay.py in `output`, or ""."""
    return next((line.strip() for line in output.splitlines() if line.strip().startswith(CAPTURE_FORMAT)), "")


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


# The docs repository: the harness of the runs, the state-page scripts and the batches that the tools run. A tool takes
# it beside the main checkout of this repository, never beside a worktree: a group of worktrees in C:/tmp can hold a
# docs worktree of an old branch, whose harness.py lacked an argument of the runner's call (W355). The key "docs" of
# local.json names another docs tree on purpose (a branch under test).
DOCS_REPO = "moreFPS"
DOCS_KEY = "docs"
# The main branch of the docs repository: the docs main checkout on another branch is named on stderr.
DOCS_MAIN = "main"


def find_docs(main, named):
    """The folder of the docs repository: `named` (the value of the key "docs") when it is set, else the moreFPS beside
    the main checkout `main`, else the one beside this tree (a tree outside git has no main checkout)."""
    if named:
        return os.path.normpath(named)
    return os.path.join(os.path.dirname(main) if main else SIBLINGS, DOCS_REPO)


def branch_of(folder):
    """The branch of the git tree at `folder`, "a detached HEAD" without one, or "no git tree"."""
    run = subprocess.run(["git", "-C", folder, "branch", "--show-current"], capture_output=True, text=True, env=git_env())
    if run.returncode != 0:
        return "no git tree"
    return run.stdout.strip() or "a detached HEAD"


def docs_line(root, beside_main, branch, key_file):
    """The stderr line for the docs tree `root` on `branch`, or None. A tree other than `beside_main` (the one beside the
    main checkout) came from the key "docs" of `key_file`; the tree beside the main checkout gets a line only when it is
    not on DOCS_MAIN (a missing tree stops later, with its own message)."""
    if os.path.normcase(root) != os.path.normcase(beside_main):
        return "docs: %s (branch %s), from the key \"%s\" of %s; the main checkout's is %s" % (
            root, branch, DOCS_KEY, key_file, beside_main)
    if branch not in (DOCS_MAIN, "no git tree"):
        return "docs: %s (branch %s), the docs main checkout, not on %s" % (root, branch, DOCS_MAIN)
    return None


@functools.lru_cache(maxsize=None)
def docs_root():
    """The docs repository of the tools (`find_docs`). A tree other than the one beside the main checkout, or that one
    on another branch than DOCS_MAIN, is named once on stderr (`docs_line`)."""
    main = main_checkout()
    root, beside_main = find_docs(main, local(DOCS_KEY, None)), find_docs(main, None)
    line = docs_line(root, beside_main, branch_of(root), local_file_hint())
    if line:
        print(line, file=sys.stderr)
    return root


def missing_docs_text(root, parts):
    """The text of a tool that stops for want of the place `parts` in the docs repository `root`."""
    return "no %s in the docs checkout %s. Check out %s there, or set the key \"%s\" in %s to another docs tree." % (
        os.path.join(*parts), root, DOCS_REPO, DOCS_KEY, local_file_hint())


def docs_place(*parts, required=False):
    """The place `parts` in the docs repository (`docs_root`), for example ("docs", "scripts", "test-harness-2026-10-03").
    With `required`, a missing place stops the tool (`missing_docs_text`); without it, the tool decides."""
    place = os.path.join(docs_root(), *parts)
    if required and not os.path.exists(place):
        raise SystemExit(missing_docs_text(docs_root(), parts))
    return place


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
    """The tests of `checkout_of`, `find_local`, `find_sibling` and `find_docs` in a temporary folder. A commit
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

    # A fault of git_env must reach only the temp repositories: the hook's names leave this
    # process, and only names() sets GIT_ names again, on purpose.
    for name in [name for name in os.environ if name.startswith("GIT_")]:
        del os.environ[name]
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
        # The docs repository: the one beside the main checkout. The null: a stale docs worktree named moreFPS beside
        # the worktree (the group folder of a worker in C:/tmp) is not taken.
        beside_main = os.path.join(root, "repos", DOCS_REPO)
        stale = os.path.join(root, "away", DOCS_REPO)
        os.makedirs(beside_main)
        os.makedirs(stale)
        assert same(find_docs(main, None)) == same(beside_main), find_docs(main, None)
        assert same(find_docs(main, "")) == same(beside_main), "an empty key names no tree"
        assert same(find_docs(checkout_of(tree), None)) == same(beside_main), "a worktree finds its main checkout's"
        # A branch under test, named on purpose by the key: that one, even the stale one.
        assert same(find_docs(main, stale)) == same(stale)
        assert same(find_docs(None, None)) == same(os.path.join(SIBLINGS, DOCS_REPO)), "outside git: beside this tree"
        text = missing_docs_text(beside_main, ("docs", "scripts", "test-harness-2026-10-03"))
        assert beside_main in text and '"docs"' in text and "test-harness-2026-10-03" in text, text
        assert branch_of(root) == "no git tree", branch_of(root)
        # The line on stderr: a named tree, and the tree beside the main checkout on another branch; none on main.
        named_line = docs_line(stale, beside_main, "old", "the file")
        assert stale in named_line and "branch old" in named_line and "the file" in named_line, named_line
        assert docs_line(beside_main, beside_main, DOCS_MAIN, "the file") is None
        assert docs_line(beside_main, beside_main, "no git tree", "the file") is None, "a missing tree stops later"
        other_line = docs_line(beside_main, beside_main, "scripts/x", "the file")
        assert beside_main in other_line and "branch scripts/x" in other_line, other_line
    if real:
        assert bare(real) == real_bare, "core.bare of %s changed from %r to %r" % (real, real_bare, bare(real))
    parts_test()
    print("common self-test: passed")


def fake_parts(build, plugin="79c752d", host="79c752d", mge="c610f3b", d3d8=None):
    """A build folder whose three parts carry these build ids, with other bytes around them."""
    contents = {
        os.path.join("Data Files", "mwse", "lib", "akulakhan.dll"): "MZ..nirender v0.3.6 %s\0.." % plugin,
        "numidium.exe": "MZ..nirender-host v0.1.0 %s\0..numidium v0.21.0 %s\0.." % (host, mge),
        "d3d8.dll": "MZ..MGE-XE G7 Fork v0.21.0 %s\0.." % (d3d8 or mge),
    }
    for path, text in contents.items():
        os.makedirs(os.path.dirname(os.path.join(build, path)) or build, exist_ok=True)
        with open(os.path.join(build, path), "wb") as file:
            file.write(text.encode("ascii"))


def parts_test():
    """The tests of part_ids, parts_problems and capture_format_line."""
    import tempfile
    with tempfile.TemporaryDirectory() as root:
        # One build of both repositories: the parts of f243.
        fake_parts(root)
        assert part_ids(root) == ({"nirender": "79c752d", "nirender-host": "79c752d", "numidium": "c610f3b", "MGE-XE G7 Fork": "c610f3b"}, []), part_ids(root)
        assert parts_problems(root) == (("79c752d", "c610f3b"), [])
        assert parts_problems(root, "79c752d", "c610f3b") == (("79c752d", "c610f3b"), [])
        assert parts_problems(root, "79c752d1234", "c610f3b")[1] == [], "a longer hash of the same commit"
        # The commits under test, not a HEAD: master moved on to 319e3c6 while this build of 79c752d waited.
        assert parts_problems(root, "319e3c6", "c610f3b")[1] == ["nirender is 79c752d, but the commit of nirender-plugin under test is 319e3c6"]
        # A plugin of an older commit than its helper, as the brief of 79c752d feared: refused also with no commits given.
        fake_parts(root, plugin="87f7049")
        assert parts_problems(root)[1] == ["nirender is 87f7049, but nirender-host is 79c752d: two commits of nirender-plugin"], parts_problems(root)
        # A d3d8.dll of another commit than numidium, and a dirty one.
        fake_parts(root, d3d8="99c8b62")
        assert parts_problems(root)[1] == ["numidium is c610f3b, but MGE-XE G7 Fork is 99c8b62: two commits of render-link"], parts_problems(root)
        fake_parts(root, mge="c610f3b-dirty")
        assert parts_problems(root)[1] == ["numidium is c610f3b-dirty: a build of a work tree with changes names no commit",
                                           "MGE-XE G7 Fork is c610f3b-dirty: a build of a work tree with changes names no commit"], parts_problems(root)
        # A part with no build id, and a missing part.
        fake_parts(root)
        with open(os.path.join(root, "d3d8.dll"), "wb") as file:
            file.write(b"MZ no id")
        os.remove(os.path.join(root, "numidium.exe"))
        problems = parts_problems(root, "79c752d", "c610f3b")[1]
        assert problems == ["numidium.exe is not in the build", "d3d8.dll holds no build id of MGE-XE G7 Fork"], problems
    # The refusal of a replay of narsis.nircap (format 8) by the helper of 79c752d (format 9): merge step of 79c752d.
    output = ("The parts of the wgpu renderer of MGE XE do not match.\n"
              "akulakhan.dll (nirender v0.3.6 87f7049) writes the stream format 8.\n"
              "numidium.exe (nirender-host v0.1.0 79c752d, numidium v0.21.0 c610f3b) reads the stream format 9.\n"
              "Install akulakhan.dll, numidium.exe and d3d8.dll from one archive.\n" + CAPTURE_AGAIN + "\n")
    line = capture_format_line("narsis.nircap", 10, output)
    assert line.startswith(CAPTURE_FORMAT + ": narsis.nircap was made by akulakhan.dll (nirender v0.3.6 87f7049), which writes the stream format 8;"), line
    assert "reads the stream format 9: make the capture again with this build, if the format change of this build is intended" in line, line
    assert capture_format_in("x\n  " + line + "\ny") == line
    # The grass capture of instance_ab: d3d8.dll of extension format 1, the base helper reads 2.
    output = ("d3d8.dll (MGE-XE G7 Fork v0.21.0 99c8b62-dirty) writes the extension format 1.\n"
              "numidium.exe (nirender-host v0.1.0 a76fd14, numidium v0.21.0 535bef1) reads the extension format 2.\n" + CAPTURE_AGAIN)
    line = capture_format_line("grass.nircap", 11, output)
    assert "d3d8.dll (MGE-XE G7 Fork v0.21.0 99c8b62-dirty), which writes the extension format 1" in line and "format 2" in line, line
    # A capture newer than the build: the advice is the other way.
    newer = output.replace("format 1.", "format 3.")
    assert "older than the capture" in capture_format_line("grass.nircap", 11, newer), capture_format_line("grass.nircap", 11, newer)
    # Another exit, or no "make the capture again": no line.
    assert capture_format_line("x.nircap", 1, output) == "" and capture_format_line("x.nircap", 10, "panic") == ""
    assert capture_format_in("no such line") == ""

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
