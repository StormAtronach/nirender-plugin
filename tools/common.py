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
# The exit code of an A/B stage whose base build refused the capture with a format exit code of the helper
# (REFUSED_FORMAT_EXITS) while the build under test played the same capture: the two builds read two formats
# (format_change). merge_step.py judges it by this code, never by a line's text (W527).
FORMAT_CHANGE_EXIT = 6
# The exit code of an A/B stage whose build under test differs from the base while the base equals itself: a FAIL. Its
# other FAILs (noise, an empty hash line, a replay that does not play) exit with 1. merge_step.py passes this code only
# with an evidence file, as a planned change that the Coordinator judged from the numbers (W574).
DIFFERS_EXIT = 7

# The summary line of one test binary in the output of cargo test, quiet or not. The one reader of it in the tools:
# merge_step.py --require-tests and check_mwse_sites.py (W527).
TEST_RESULT = re.compile(r"^test result: \w+\. (\d+) passed; (\d+) failed;", re.MULTILINE)


def tests_ran(output):
    """How many tests the output of `cargo test` says ran: passed and failed, over every test binary."""
    return sum(int(passed) + int(failed) for passed, failed in TEST_RESULT.findall(output))

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


# The exit codes of a helper that refuses a stream, with the kind of format of each: 10 the stream format of the plugin,
# 11 the extension format of d3d8.dll (EXIT_STREAM_FORMAT and EXIT_EXTENSION_FORMAT of src/stream_format.rs; the
# self-test holds the two against that file), and the last line of such a refusal in a replay (host/src/cli.rs,
# renderer.rs of render-link).
FORMAT_OF_EXIT = {10: "stream", 11: "extension"}
EXIT_CONSTANT_OF_FORMAT = {"stream": "EXIT_STREAM_FORMAT", "extension": "EXIT_EXTENSION_FORMAT"}
REFUSED_FORMAT_EXITS = tuple(FORMAT_OF_EXIT)
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


def format_exit_problems(source):
    """The problems of FORMAT_OF_EXIT against the text of src/stream_format.rs: each code must be the value of its
    constant there, so that a change of a code fails this check and not a merge round."""
    problems = []
    for code, kind in FORMAT_OF_EXIT.items():
        name = EXIT_CONSTANT_OF_FORMAT[kind]
        match = re.search(r"^pub const %s: u32 = (\d+);" % name, source, re.M)
        if not match:
            problems.append("src/stream_format.rs has no %s" % name)
        elif int(match.group(1)) != code:
            problems.append("src/stream_format.rs has %s = %s, and tools/common.py has %d" % (name, match.group(1), code))
    return problems


def format_change(base_code, tip_code):
    """The format change between the base and the tip that two replays of ONE capture show, as the kind of format
    ("stream" or "extension"), or None. The base refused the capture with a format code of the helper, so it does not
    read the capture's format; the tip played it (exit code 0), so it does. Their formats differ. A capture that the tip
    refuses too is of a third format: no change of the two builds, and a failure of the tip's replay."""
    if tip_code != 0:
        return None
    return FORMAT_OF_EXIT.get(base_code)


# The helper's exit code of a replay whose stage shaders failed (EXIT_REPLAY_STAGE_SHADERS of src/stream_format.rs;
# the self-test reads the value there), and the start of the line that the helper prints before it
# (host/src/stage_shaders.rs, `replay_fault`). The replay made its pictures and its report first, and the pictures are
# right: the uber-shader drew the draws. Only this code and this line show the fault (W523).
EXIT_REPLAY_STAGE_SHADERS = 40
STAGE_FAULT_START = "stage shaders: a fault of the replay: "
# The start of the line that replay.py adds to such a fault.
STAGE_SHADERS = "STAGE SHADERS"
STAGE_FAULT_COUNTS = re.compile(r"; (\d+) pipelines failed.*; the uber-shader drew (\d+) draws that have a stage key .*; (\d+) draws past the cap")
# The first failed rewrite of the helper's line: its key, the part that refused and the count of the part's matches.
STAGE_FIRST_REFUSAL = re.compile(r"rewrites failed \((stage key \d+: .*? wanted)")
# The helper's report line of the stage shaders at the end of a replay or of a live session (`report` there).
STAGE_REPORT = re.compile(r"^stage shaders: (?:on|off) \((made [^)]*)\), .*?; (\d+) rewrites failed.*?; (\d+) pipelines failed.*?; draws on the uber-shader with a stage key: "
                          r"(\d+) after a failed rewrite, (\d+) after a failed pipeline, (\d+) with a pipeline not made; (\d+) draws past the cap, \d+ draws with no key", re.M)


# The end of the name of the file that keeps a replay's output beside its picture.
REPLAY_OUTPUT = ".replay.txt"


def keep_replay_output(picture, code, output):
    """Writes the exit code and the full output of a replay beside its picture, as <picture without .png>.replay.txt,
    and returns the path. The gate judges a replay by lines of this output (the counts, the order lines, the report of
    the stage shaders); a verdict can then be checked against the text it read. A file that cannot be written does
    not stop the gate: the path is then None."""
    path = (picture[:-4] if picture.lower().endswith(".png") else picture) + REPLAY_OUTPUT
    try:
        with open(path, "w", encoding="utf-8", errors="replace", newline="\n") as file:
            file.write("exit code %d\n%s" % (code, output))
    except OSError:
        return None
    return path


def stage_fault_summary(output):
    """The short form of the helper's fault line of a replay in `output`, or "": each count, and the first failed
    rewrite. The helper's line has every failed key, so it can be thousands of characters long."""
    line = next((line for line in output.splitlines() if line.startswith(STAGE_FAULT_START)), "")
    rewrites, counts = re.match(re.escape(STAGE_FAULT_START) + r"(\d+) rewrites failed", line), STAGE_FAULT_COUNTS.search(line)
    if not rewrites or not counts:
        return ""
    first = STAGE_FIRST_REFUSAL.search(line)
    return "%s rewrites failed%s, %s pipelines failed, %s draws on the uber-shader with a stage key, %s draws past the cap of the keys" % (
        rewrites.group(1), " (the first: %s)" % first.group(1) if first else "", counts.group(1), counts.group(2), counts.group(3))


def stage_fault_line(capture, code, output):
    """For a replay that ended with the fault of the stage shaders, the line that names the capture and the fault, or
    "". A tool that keeps only the end of a replay's output still has this line: replay.py writes it last."""
    if code != EXIT_REPLAY_STAGE_SHADERS:
        return ""
    summary = stage_fault_summary(output) or "the helper's line of the fault is not in the output"
    return "%s: the replay of %s failed with exit code %d: %s. The pictures are right; the speed of the stage shaders is lost" % (STAGE_SHADERS, capture, code, summary)


def stage_fault_in(output):
    """The STAGE SHADERS line of replay.py in `output`, or ""."""
    return next((line.strip() for line in output.splitlines() if line.strip().startswith(STAGE_SHADERS + ": ")), "")


# The checks of the helper (W526; host/src/checks.rs): the facts that a tool judges. The helper prints one line for
# each at the end of its report, always: "check <name>: <judged> judged, <faults> faults". `judged` says whether the
# check could judge at all. This is the ONE reader of these lines; a tool takes no verdict from another sentence of
# the helper. The self-test holds the form and the names against the Rust file.
CHECK_LINE = re.compile(r"^check ([a-z]+(?: [a-z]+)*): (\d+) judged, (\d+) faults\r?$", re.M)
EARLY_WORLD_DRAWS, OWN_VIEWS, READS_PAST_END = "early world draws", "views of their own", "reads past the end"
UNKNOWN_RECORDS, GPU_ERRORS, STAGE_SHADERS_CHECK = "unknown records", "gpu errors", "stage shaders"
CHECK_NAMES = (EARLY_WORLD_DRAWS, OWN_VIEWS, READS_PAST_END, UNKNOWN_RECORDS, GPU_ERRORS, STAGE_SHADERS_CHECK)
# The helper's exit code of a replay with a fault of a check that can change a picture (EXIT_REPLAY_CHECKS of
# src/stream_format.rs), also when the stage shaders have a fault too; the start of the helper's line before it
# (host/src/checks.rs, `replay_fault`); and the start of the line that replay.py adds.
EXIT_REPLAY_CHECKS = 41
CHECKS_FAULT_START = "checks: a fault of the replay: "
CHECKS = "CHECKS"


def helper_checks(output):
    """The checks of a helper's output: {name: (judged, faults)}, from its last line of each name. A name that the
    output does not have is not in the result: the helper did not end in order, or is from before the checks."""
    return {name: (int(judged), int(faults)) for name, judged, faults in CHECK_LINE.findall(output)}


def check_problems(output, names=CHECK_NAMES, may_judge_nothing=()):
    """What the checks `names` of a helper's output say against it: (not judged, failed), two lists of texts. Not
    judged (NOT RUN, never a pass): the output has no line of the check, so nothing is known; or the check judged 0
    things, so it found no fault in nothing. Failed: the check found faults. A caller for which 0 things is right
    for a check names it in `may_judge_nothing` and judges that case itself: a capture with no view of its own
    has no place of such a view to judge."""
    found = helper_checks(output)
    not_judged = ["the helper's output has no line of the check \"%s\"" % name for name in names if name not in found]
    not_judged += ["the check \"%s\" judged nothing" % name for name in names if name in found and found[name][0] == 0 and name not in may_judge_nothing]
    failed = ["the check \"%s\": %d faults of %d judged" % (name, found[name][1], found[name][0]) for name in names if name in found and found[name][1]]
    return not_judged, failed


def checks_fault_line(capture, code, output):
    """For a replay that ended with the code of the checks, the line that names the capture and the checks with
    faults, or "". replay.py writes it last, as the line of the stage shaders."""
    if code != EXIT_REPLAY_CHECKS:
        return ""
    line = next((line for line in output.splitlines() if line.startswith(CHECKS_FAULT_START)), "")
    body = line[len(CHECKS_FAULT_START):] if line else "the helper's line of the fault is not in the output"
    stage = stage_fault_summary(output)
    return "%s: the replay of %s failed with exit code %d: %s%s" % (CHECKS, capture, code, body, "; and the stage shaders: " + stage if stage else "")


def replay_fault_line(capture, code, output):
    """The line that replay.py adds to a replay that ended with a fault code of its own (the stage shaders' 40, the
    checks' 41), or ""."""
    return stage_fault_line(capture, code, output) or checks_fault_line(capture, code, output)


def replay_fault_in(output):
    """The line of replay.py in `output` for a replay that ended with a fault code of its own (STAGE SHADERS or
    CHECKS), or ""."""
    starts = (STAGE_SHADERS + ": ", CHECKS + ": ")
    return next((line.strip() for line in output.splitlines() if line.strip().startswith(starts)), "")


def stage_session_problem(host_text):
    """The stage shaders of a live session, from the text of its helper (host.txt): (line, problem). The line is the
    summary of the helper's last report line, or the reason why there is none. The problem is the text of a failed
    rewrite or a failed pipeline, else None. A draw on the uber-shader with a pipeline not made is no problem in a
    live session: a thread makes the pipelines behind the draws, and the line names how they were made."""
    found = None
    for found in STAGE_REPORT.finditer(host_text):
        pass
    if not found:
        return "stage shaders: no report line of W523 in the helper's text (a helper from before it, or one that did not end in order): not checked", None
    made, rewrites, pipelines, after_rewrite, after_pipeline, not_made, past_cap = found.groups()
    line = "stage shaders: %s; %s rewrites failed, %s pipelines failed; draws on the uber-shader with a stage key: %s after a failed rewrite, %s after a failed pipeline, %s with a pipeline not made; %s past the cap" % (
        made, rewrites, pipelines, after_rewrite, after_pipeline, not_made, past_cap)
    if rewrites == "0" and pipelines == "0":
        return line, None
    first = STAGE_FIRST_REFUSAL.search(found.group(0))
    return line, "the stage shaders failed in the session: %s rewrites failed%s, %s pipelines failed. The pictures are right; the speed of the stage shaders is lost" % (
        rewrites, " (the first: %s)" % first.group(1) if first else "", pipelines)


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
    stage_test()
    deploys_test()
    print("common self-test: passed")


def checks_test(rust_codes):
    """The tests of the one reader of the helper's check lines (W526). The first part holds the form of the line, the
    names and the exit code against the Rust files: a change there fails here, so the reader cannot go blind."""
    rust = open(os.path.join(REPO, "host", "src", "checks.rs"), encoding="utf-8").read()
    assert 'format!("check {}: {} judged, {} faults", self.name, self.judged, self.faults)' in rust, "host/src/checks.rs has another form of the check line"
    assert 'format!("checks: a fault of the replay: {}", faults.join("; "))' in rust and '"{}: {} faults of {} judged"' in rust, "host/src/checks.rs has another fault line"
    in_rust = set(re.findall(r'^pub\(crate\) const [A-Z_]+: &str = "([a-z ]+)";$', rust, re.M))
    assert in_rust == set(CHECK_NAMES), "the names of host/src/checks.rs are %s, and common.py has %s" % (sorted(in_rust), sorted(CHECK_NAMES))
    report = open(os.path.join(REPO, "host", "src", "player", "report.rs"), encoding="utf-8").read()
    assert "for check in self.checks() {" in report and 'println!("{}", check.line());' in report, "the report no longer prints each check's line"
    assert "pub const EXIT_REPLAY_CHECKS: u32 = %d;" % EXIT_REPLAY_CHECKS in rust_codes, "EXIT_REPLAY_CHECKS of src/stream_format.rs is not %d" % EXIT_REPLAY_CHECKS

    def lines(**faults):
        judged = {EARLY_WORLD_DRAWS: 12, OWN_VIEWS: 97, READS_PAST_END: 5000, UNKNOWN_RECORDS: 5000, GPU_ERRORS: 30, STAGE_SHADERS_CHECK: 13}
        return "".join("check %s: %d judged, %d faults\n" % (name, judged[name], faults.get(name.replace(" ", "_"), 0)) for name in CHECK_NAMES)

    clean = "shapes: 1\n" + lines() + "statics report end: x\n"
    assert helper_checks(clean) == {EARLY_WORLD_DRAWS: (12, 0), OWN_VIEWS: (97, 0), READS_PAST_END: (5000, 0), UNKNOWN_RECORDS: (5000, 0), GPU_ERRORS: (30, 0), STAGE_SHADERS_CHECK: (13, 0)}
    assert check_problems(clean) == ([], [])
    # A text with Windows line ends has the same checks: a pattern that ends at "\n" alone found 0 of 6 there (the
    # Reviewer's plant, verdict 470), which is a NOT RUN for a helper that printed every line.
    assert helper_checks(clean.replace("\n", "\r\n")) == helper_checks(clean), helper_checks(clean.replace("\n", "\r\n"))
    # Rob's plant of F-G-1: a line with one word moved, and no line at all, are "not judged", never a pass.
    for text in (clean.replace("check early world draws: 12 judged", "check early draws of a world: 12 checked"), clean.replace("check early world draws: 12 judged, 0 faults\n", ""), ""):
        missing, failed = check_problems(text, (EARLY_WORLD_DRAWS,))
        assert missing == ['the helper\'s output has no line of the check "early world draws"'] and failed == [], (text[:60], missing, failed)
    # The helper's sentences of before are no check line: a tool cannot fall back on them.
    old = "early world draws: 12 starts of a world for the extension checked, 0 scenes with draws of their world already in the target, 0 draws\n"
    assert helper_checks(old) == {} and check_problems(old, (EARLY_WORLD_DRAWS,))[0] != []
    # Faults: named with their counts. 0 judged is in neither list.
    faulty = lines(early_world_draws=5, gpu_errors=2)
    assert check_problems(faulty) == ([], ['the check "early world draws": 5 faults of 12 judged', 'the check "gpu errors": 2 faults of 30 judged']), check_problems(faulty)
    # A check that judged nothing is not judged (W524: 0 things judged is no pass), but for a caller that names it.
    idle = "check views of their own: 0 judged, 0 faults\n"
    assert check_problems(idle, (OWN_VIEWS,)) == (['the check "views of their own" judged nothing'], [])
    assert check_problems(idle, (OWN_VIEWS,), may_judge_nothing=(OWN_VIEWS,)) == ([], [])
    assert check_problems("check gpu errors: 0 judged, 2 faults\n", (GPU_ERRORS,)) == (['the check "gpu errors" judged nothing'], ['the check "gpu errors": 2 faults of 0 judged'])
    # The last line of a name counts (a live helper's text can hold the report of an earlier start).
    assert helper_checks(faulty + clean)[GPU_ERRORS] == (30, 0) and helper_checks(clean + faulty)[GPU_ERRORS] == (30, 2)
    # replay.py's line for the code of the checks, alone and with a fault of the stage shaders; no line for another code.
    fault = CHECKS_FAULT_START + "early world draws: 5 faults of 12 judged; gpu errors: 2 faults of 30 judged"
    line = checks_fault_line("seyda.nircap", EXIT_REPLAY_CHECKS, faulty + fault + "\n")
    assert line == "CHECKS: the replay of seyda.nircap failed with exit code 41: early world draws: 5 faults of 12 judged; gpu errors: 2 faults of 30 judged", line
    both = STAGE_FAULT_START + ("1 rewrites failed (stage key 1: the end mark of the stage facts: 0 in the text, 1 wanted); 0 pipelines failed; the uber-shader drew 9 draws that have a stage key "
                               "(9 after a failed rewrite, 0 after a failed pipeline, 0 with a pipeline not made); 0 draws past the cap of 256 keys\n") + fault + "\n"
    line = replay_fault_line("seyda.nircap", EXIT_REPLAY_CHECKS, both)
    assert line.startswith("CHECKS: the replay of seyda.nircap failed with exit code 41: early world draws: 5 faults") and "; and the stage shaders: 1 rewrites failed (the first: stage key 1: " in line, line
    assert replay_fault_line("seyda.nircap", EXIT_REPLAY_STAGE_SHADERS, both).startswith(STAGE_SHADERS + ": the replay of seyda.nircap failed with exit code 40: ")
    assert replay_fault_line("seyda.nircap", 0, both) == "" and replay_fault_line("seyda.nircap", 20, both) == ""
    assert "the helper's line of the fault is not in the output" in checks_fault_line("x.nircap", EXIT_REPLAY_CHECKS, "panic")
    assert replay_fault_in("x\n  " + line + "\ny") == line and replay_fault_in(both) == "", "the helper's own lines are not replay.py's line"
    stage_line = stage_fault_line("a.nircap", EXIT_REPLAY_STAGE_SHADERS, both)
    assert replay_fault_in(stage_line + "\n") == stage_line


def stage_test():
    """The tests of the stage shaders' lines (W523): the fault of a replay, replay.py's line of it, and the report line
    of a live session. The texts are the helper's; the first checks fail when the helper's code changes their fixed
    parts or the exit code, so that these readers cannot go blind without a failed check."""
    rust = open(os.path.join(REPO, "host", "src", "stage_shaders.rs"), encoding="utf-8").read()
    for part in ('"' + STAGE_FAULT_START + "{}; {}; the uber-shader drew {uber_draws} draws that have a stage key ({} after a failed rewrite, {} after a failed pipeline, {} with a pipeline not made); {} draws past the cap of {MAX_KEYS} keys\"",
                 '"stage shaders: {} ({made}), ', "; {}; {}; draws on the uber-shader with a stage key: {} after a failed rewrite, {} after a failed pipeline, {} with a pipeline not made; {} draws past the cap, {} draws with no key\"",
                 '"{} rewrites failed"', '"0 pipelines failed"', '"{} pipelines failed (stage keys {})"', '"stage key {id}: {refusal}"', '"{}: {} in the text, {}"', '"1 wanted"', '"0 wanted"',
                 '"made at the first use"', '"made on a thread, with no list: a cold start"', '"made on a thread, with a list"'):
        assert part in rust, "host/src/stage_shaders.rs no longer has %s: change the readers of common.py with it" % part
    codes = open(os.path.join(REPO, "src", "stream_format.rs"), encoding="utf-8").read()
    assert "pub const EXIT_REPLAY_STAGE_SHADERS: u32 = %d;" % EXIT_REPLAY_STAGE_SHADERS in codes, "EXIT_REPLAY_STAGE_SHADERS of src/stream_format.rs is not %d" % EXIT_REPLAY_STAGE_SHADERS

    refusal = "the end mark of the stage facts: 0 in the text, 1 wanted"
    fault = (STAGE_FAULT_START + "2 rewrites failed (stage key 1: %s; stage key 2: %s); 0 pipelines failed; the uber-shader drew 5120 draws that have a stage key "
             "(5120 after a failed rewrite, 0 after a failed pipeline, 0 with a pipeline not made); 0 draws past the cap of 256 keys" % (refusal, refusal))
    output = "frames 60\n" + fault + "\n"
    summary = stage_fault_summary(output)
    assert summary == "2 rewrites failed (the first: stage key 1: %s), 0 pipelines failed, 5120 draws on the uber-shader with a stage key, 0 draws past the cap of the keys" % refusal, summary
    line = stage_fault_line("narsis.nircap", EXIT_REPLAY_STAGE_SHADERS, output)
    assert line.startswith(STAGE_SHADERS + ": the replay of narsis.nircap failed with exit code 40: 2 rewrites failed (the first: stage key 1: "), line
    assert stage_fault_in("x\n  " + line + "\ny") == line and stage_fault_in(output) == "", "the helper's own line is not replay.py's line"
    # Only failed pipelines: no first refusal. Only draws: the counts.
    pipelines = STAGE_FAULT_START + ("0 rewrites failed; 2 pipelines failed (stage keys 3, 9); the uber-shader drew 7 draws that have a stage key "
                                     "(0 after a failed rewrite, 7 after a failed pipeline, 0 with a pipeline not made); 1 draws past the cap of 256 keys")
    assert stage_fault_summary(pipelines) == "0 rewrites failed, 2 pipelines failed, 7 draws on the uber-shader with a stage key, 1 draws past the cap of the keys", stage_fault_summary(pipelines)
    # Another exit code: no line. The code with no line of the helper: a line that says so.
    assert stage_fault_line("x.nircap", 0, output) == "" and stage_fault_line("x.nircap", 20, output) == ""
    assert "the helper's line of the fault is not in the output" in stage_fault_line("x.nircap", EXIT_REPLAY_STAGE_SHADERS, "panic")

    def report(made, rewrites, pipelines, draws):
        return ("stage shaders: on (%s), 37 stage keys (cap 256), 52 pipelines made (40 sent from the list, 0 of the list left out past its share of the cap); %s; %s; "
                "draws on the uber-shader with a stage key: %d after a failed rewrite, %d after a failed pipeline, %d with a pipeline not made; 0 draws past the cap, 12 draws with no key" % ((made, rewrites, pipelines) + draws))

    clean = report("made on a thread, with a list", "0 rewrites failed", "0 pipelines failed", (0, 0, 31))
    line, problem = stage_session_problem("shapes: 1\n" + clean + "\nstage key facts: 3 with a plane\n")
    assert problem is None and line == ("stage shaders: made on a thread, with a list; 0 rewrites failed, 0 pipelines failed; draws on the uber-shader with a stage key: "
                                        "0 after a failed rewrite, 0 after a failed pipeline, 31 with a pipeline not made; 0 past the cap"), line
    # A cold start with draws on the uber-shader: reported, not failed.
    assert stage_session_problem(report("made on a thread, with no list: a cold start", "0 rewrites failed", "0 pipelines failed", (0, 0, 9000)))[1] is None
    # A failed rewrite and a failed pipeline each fail; the last report line of the text counts.
    refused = report("made on a thread, with a list", "37 rewrites failed (stage key 1: %s; stage key 2: %s)" % (refusal, refusal), "0 pipelines failed", (900, 0, 0))
    line, problem = stage_session_problem(clean + "\n" + refused + "\n")
    assert problem == "the stage shaders failed in the session: 37 rewrites failed (the first: stage key 1: %s), 0 pipelines failed. The pictures are right; the speed of the stage shaders is lost" % refusal, problem
    assert stage_session_problem(refused + "\n" + clean + "\n")[1] is None
    failed = report("made on a thread, with a list", "0 rewrites failed", "1 pipelines failed (stage keys 4)", (0, 3, 0))
    assert "0 rewrites failed, 1 pipelines failed." in stage_session_problem(failed)[1], stage_session_problem(failed)
    # The line of a helper from before W523, and a text with no line: not checked, and it says so.
    old = "stage shaders: on, 37 keys (cap 256), 52 pipelines made, 0 failed, 0 draws on the uber-shader while their pipeline was not made"
    for text in (old, "", "gpu error: lost"):
        line, problem = stage_session_problem(text)
        assert problem is None and line.endswith("not checked"), (text, line)
    checks_test(rust_codes=codes)
    # The kept output of a replay: beside the picture, with the exit code first; no folder, no file and no stop.
    import tempfile
    with tempfile.TemporaryDirectory() as folder:
        kept = keep_replay_output(os.path.join(folder, "seyda.png"), 40, output)
        assert kept == os.path.join(folder, "seyda" + REPLAY_OUTPUT) and open(kept, encoding="utf-8").read() == "exit code 40\n" + output, kept
        assert keep_replay_output(os.path.join(folder, "tour-frames-tour-1"), 0, "x").endswith("tour-frames-tour-1" + REPLAY_OUTPUT)
        assert keep_replay_output(os.path.join(folder, "no such folder", "a.png"), 0, "x") is None


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
    # The refusal codes against the plugin's source, and the plants: another value, no constant.
    with open(os.path.join(REPO, "src", "stream_format.rs"), encoding="utf-8") as source:
        problems = format_exit_problems(source.read())
    assert problems == [], problems
    planted = "pub const EXIT_STREAM_FORMAT: u32 = 10;\npub const EXIT_EXTENSION_FORMAT: u32 = 13;\n"
    assert format_exit_problems(planted) == ["src/stream_format.rs has EXIT_EXTENSION_FORMAT = 13, and tools/common.py has 11"]
    assert format_exit_problems("pub const EXIT_EXTENSION_FORMAT: u32 = 11;\n") == ["src/stream_format.rs has no EXIT_STREAM_FORMAT"]
    # A format change: the base refuses with a format code and the tip plays the same capture. A third format (the tip
    # refuses too), another code of the base, and a base that played are none.
    assert format_change(11, 0) == "extension" and format_change(10, 0) == "stream"
    assert format_change(11, 11) is None and format_change(11, 1) is None
    assert format_change(5, 0) is None and format_change(0, 0) is None

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


# The open deploys (W528): a merge round whose deploy is WITHHELD appends a WITHHELD line, and deploy_rest.py appends a
# DEPLOYED line after each deploy of a build. A WITHHELD line stays open until a DEPLOYED line names the same commits or
# newer ones of both repos. Tab-separated, CRLF, ASCII, with a header; written by merge_step.py and deploy_rest.py only.
DEPLOYS_FILE = "E:\\nirender-keep\\merge-step\\deploys.tsv"
DEPLOYS_HEADER = "time\tword\tplugin\tmge\twhat"
DEPLOY_WITHHELD, DEPLOY_DONE = "WITHHELD", "DEPLOYED"


def deploys_line(when, word, plugin, mge, what):
    """One line of the deploys file. A tab or a line end in `what` becomes a space."""
    return "\t".join((when, word, plugin or "unknown", mge or "unknown", " ".join(what.split())))


def append_deploys(path, line):
    """Appends `line` to the deploys file at `path`, with the header first when the file is new. An OSError goes to the
    caller: the writer says it and goes on, as a missed line must not fail a round or a deploy."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    new = not os.path.exists(path)
    with open(path, "a", encoding="ascii", errors="replace", newline="\r\n") as file:
        if new:
            file.write(DEPLOYS_HEADER + "\n")
        file.write(line + "\n")


def open_deploys(lines, is_newer):
    """The WITHHELD rows of `lines` (the deploys file) that no later DEPLOYED row closes, oldest first, as (time, plugin,
    mge, what). A DEPLOYED row closes a WITHHELD row when, for each repo, its commit is the same or newer:
    `is_newer(kind, older, newer)` with the kind "plugin" or "mge". An unknown commit is never newer."""
    rows = [line.rstrip("\r\n").split("\t") for line in lines]
    rows = [row for row in rows if len(row) == 5 and row[1] in (DEPLOY_WITHHELD, DEPLOY_DONE)]
    still = []
    for index, (when, word, plugin, mge, what) in enumerate(rows):
        if word != DEPLOY_WITHHELD:
            continue
        later = [row for row in rows[index + 1:] if row[1] == DEPLOY_DONE]
        if not any("unknown" not in (plugin, mge, done[2], done[3]) and is_newer("plugin", plugin, done[2]) and is_newer("mge", mge, done[3])
                   for done in later):
            still.append((when, plugin, mge, what))
    return still


def open_deploys_text(still):
    """The machine answer of the close line: "open deploys: K", with the oldest when K > 0."""
    if not still:
        return "open deploys: 0"
    when, plugin, mge, _ = still[0]
    return "open deploys: %d (the oldest %s: plugin %s, mge %s)" % (len(still), when, plugin[:10], mge[:10])


def deploys_test():
    """The tests of the deploys file (W528), on a scratch file."""
    import tempfile

    order = ["p1", "p2", "p3"]

    def is_newer(kind, older, newer):
        return order.index(older.replace("m", "p")) <= order.index(newer.replace("m", "p"))

    with tempfile.TemporaryDirectory() as root:
        path = os.path.join(root, "keep", "deploys.tsv")
        append_deploys(path, deploys_line("t1", DEPLOY_WITHHELD, "p1", "m1", "a game\truns"))
        with open(path, "rb") as file:
            assert file.read() == ("%s\r\nt1\tWITHHELD\tp1\tm1\ta game runs\r\n" % DEPLOYS_HEADER).encode("ascii")
        with open(path, encoding="ascii") as file:
            lines = file.readlines()
        # The plant: a WITHHELD row with no DEPLOYED row is open.
        assert open_deploys(lines, is_newer) == [("t1", "p1", "m1", "a game runs")]
        assert open_deploys_text(open_deploys(lines, is_newer)) == "open deploys: 1 (the oldest t1: plugin p1, mge m1)"
        # A DEPLOYED row of an older plugin, of an unknown commit, or before the WITHHELD row closes nothing.
        older = [deploys_line("t0", DEPLOY_DONE, "p3", "m3", "A, B")] + lines + [deploys_line("t2", DEPLOY_DONE, "p1", "unknown", "A")]
        assert len(open_deploys(older, is_newer)) == 1
        mixed = lines + [deploys_line("t2", DEPLOY_DONE, "p2", "m1", "A")]
        assert len(open_deploys(mixed, is_newer)) == 0, "the same mge and a newer plugin close it"
        # The null: a DEPLOYED row of the same or newer commits closes it.
        for done in (deploys_line("t2", DEPLOY_DONE, "p1", "m1", "A, B"), deploys_line("t2", DEPLOY_DONE, "p3", "m2", "A, B")):
            assert open_deploys(lines + [done], is_newer) == [] and open_deploys_text([]) == "open deploys: 0"
        assert len(open_deploys(lines + ["a short line\n"], is_newer)) == 1, "a line of another form is no row"


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
