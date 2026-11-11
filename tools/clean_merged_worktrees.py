"""Removes the side worktrees of the nirender and MGE XE repositories whose work is merged, the old play
sets, and the build output of any tree that no build touched for 48 hours (the data rule, D71). A step
of each merge. Without `--apply` it only lists.

    python tools/clean_merged_worktrees.py                  # the list: each worktree, its size, why
    python tools/clean_merged_worktrees.py --apply          # remove the ones the list names
    python tools/clean_merged_worktrees.py --name <path>    # also a worktree that is not merged or
                                                            # has no branch: the owner's list
    python tools/clean_merged_worktrees.py --during-game    # the list while a game runs: the walk
                                                            # reads the whole disk, so never in a timing
    python -I tools/clean_merged_worktrees.py --self-test   # the pure parts and a real junction

The list opens with the trees at risk (D98): each worktree that git does not hold locked, is no main or
standing checkout and is not under E:\\nirender-keep, and has at least one of these:

- commits that no ref of the main branch, no remote and no tag holds (a local branch does not count);
- changed or untracked files beyond a build's output, or ignored files that no build makes;
- the records (moreFPS/docs/Numidium Project/notes/takeover/*records*.md) cite the tree's path, or a
  run folder in its `results`.

Each line gives the reasons and the command that locks the tree. A worker locks the worktree it makes
(`git worktree lock --reason "<who and why>" <path>`). A locked tree is skipped, as before, and is
listed after the trees at risk with the words of its lock.

Build output is disposable. In each worktree of the nirender, MGE XE and MWSE repositories, main and
standing checkouts too, a folder goes with its source left alone when all of these hold:

- It is `target` or `target-*` with a Cargo.toml beside it or a CACHEDIR.TAG in it, at most three levels
  down, or MWSE's objects: `Release` or `Debug` of MWSE, CSSE and MWSE-Tests, with only a compiler's files.
- MWSE's `build` folder stays: it holds lua51.dll, which MSBuild does not always make here, and files put
  there by hand.
- It is a real folder, git ignores it or it has a CACHEDIR.TAG (cargo's mark: `target-*` is not always in
  .gitignore), and git tracks no file in it.
- Nothing was written in it for 48 hours.
- No process uses the tree, no game and no helper runs: both are looked at again just before the removal.

A play set is the current one that local.bat names, or the one before, or old. An old set's trees in
C:\\tmp\\play<N> go by the steps below, with no branch and no merge needed. The rolling trees of C:\\tmp\\playset stay.

All of these must hold before a worktree goes:

- Its branch is merged into the target of its repository, `--name` names it, or it is an old play set.
  In each case a ref must hold its commit: a removal must not leave a commit on no ref.
- `git status` works and shows no change and no untracked file, and each ignored file is a build output or
  in `results`: `git worktree remove` deletes ignored files without a word. One more is allowed: the
  ignored `local.json` at the root, when it is byte for byte the main checkout's. One that differs stays a
  reason to keep the tree (it may hold the paths of a session's own), and the list of trees at risk says so.
- No process uses it: a command line or a program path in it, or a current folder in it. This is
  looked at again just before the removal. It does not see a session that has a worktree in mind
  and no command running there: a standing worktree needs `.keep-worktree` or a line in
  SHARED_CHECKOUTS. System processes give no current folder. It does not see a junction elsewhere
  that points into a worktree either.
- The tool does not run from it.
- No game and no helper runs: a run writes into its worktree's results, and the walk would disturb
  a timing. The tool stops before the walk. With --apply it also keeps the game slot free for its run
  (`clean_common.game_kept_free`, W363).
- Nothing was written in it for `--idle-hours` hours, and it has no `.keep-worktree` file, and
  git does not hold it locked.
- No other worktree's Cargo.toml names it by path.
- It is no main checkout, no standing worktree of the train and no current play set, and it is not
  under E:\\nirender-keep.

Then, in this order, and any failure leaves the worktree as it is, apart from what came before:

1. What a real `results` folder holds is copied to E:\\nirender-keep\\runs-<date>, unless a runs-*
   folder holds the same; the number and the size of the files must agree. By the expiry of
   `clean_results.py`: a run on the keep list or younger than 7 days whole; a cited run of 7 days or
   more and an uncited one of 7 to 30 days with no pictures; an older uncited one not at all. A folder that is no run and a loose file are copied whole.
2. A new walk finds each reparse point in the tree, and each is removed as a link, which leaves its
   target. `git worktree remove` goes through a junction: on 2026-10-29 one deleted
   E:\\nirender-results that way. A folder that the walk cannot read, or a link that is still there
   after this, refuses the removal.
3. `git worktree remove`, never --force, then `git worktree prune`. Branches stay.

What `git worktree remove` leaves goes on a list with its size, for the owner. The plain folders in
C:\\tmp are listed too, and nothing touches them. The removal of build output goes file by file through
`clean_common.remove_tree`, and it takes each link as a link.
`cargo sweep` is not here: it finds nothing in the layout of our targets (the board, 2026-10-29).

Each act goes to E:\\nirender-keep\\clean-worktrees.log, with the target of each link removed, so
that a link can be made again when the removal then fails.
"""
import _winapi
import argparse
import contextlib
import glob
import io
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
# `python -I` leaves the folder of the script off the path.
sys.path.insert(0, HERE)
import clean_common as common  # noqa: E402
from clean_common import DOCS, FILE_ATTRIBUTE_REPARSE_POINT, KEEP_ROOT, REPOS_ROOT, REPOSITORIES, SETS, Evidence, GameNotFree, current_folder, current_set, files_in, game_kept_free, games_in, git, gigabytes, is_heavy, is_link, is_link_entry, kept_set_numbers, log_line, old_play_set, own_line, processes, project_trees, read_keep_list, remove_link, remove_tree, run_age_days, run_treatment, same_or_under, same_path, users_of, walk, worktrees_of  # noqa: E402

# The shared checkouts and the test train's pair stay, whatever their branch has. Git's own main
# worktree is one too.
SHARED_CHECKOUTS = ["nirender-plugin", "MGE-XE-G7", "MGE-XE-G7-render-link", "nirender-plugin-train", "MGE-XE-G7-train"]
# What a build leaves, as `git status --ignored` names it. `results` is copied before a removal.
BUILD_OUTPUTS = re.compile(r"(target|host/target|bin|dist|results|tools/out)/|(.*/)?__pycache__/")
PLAIN_FOLDERS = "C:\\tmp"
KEEP_MARKER = ".keep-worktree"
OUTPUT_HOURS = 48.0
TARGET_NAME = re.compile(r"target(-.+)?", re.I)
# How deep the search for a target goes below a tree.
TARGET_DEPTH = 3
# MWSE's objects, and the kinds of file that a compiler leaves there.
MSBUILD_OBJECTS = [os.path.join(project, configuration) for project in ("MWSE", "CSSE", "MWSE-Tests") for configuration in ("Release", "Debug")]
OBJECT_FILES = (".obj", ".tlog", ".log", ".recipe", ".iobj", ".ipdb", ".pch", ".res", ".txt", ".pdb", ".lastbuildstate", ".ilk", ".idb")


def is_play_set(path):
    """True for a tree of a play set: C:\\tmp\\play185\\..., or a folder with a name of that kind."""
    return any(re.fullmatch(r"play[-_]?\d*.*", part, re.I) for part in os.path.abspath(path).split(os.sep))


def decide(tree):
    """What happens to a worktree, from its facts: a dict with `path`, `main`, `branch` (None when
    detached), `merged`, `named`, `old_set`, `changes`, `locked`, `marker`, `used_by`, `depended_by`,
    `other_ignored`, `status_error`, `on_a_ref`, `idle_hours`, `idle_limit` and `unread`. Returns
    (True, the reason it may go) or (False, the reason it stays). The first refusal in this order is the reason."""
    path = tree["path"]
    if tree["main"] or any(same_path(path, os.path.join(REPOS_ROOT, name)) for name in SHARED_CHECKOUTS):
        return False, "a main or standing checkout"
    if same_or_under(HERE, path):
        return False, "the tool runs from it"
    if same_or_under(path, KEEP_ROOT):
        return False, "under E:\\nirender-keep"
    if is_play_set(path) and not tree.get("old_set"):
        return False, "a play set"
    if tree["locked"]:
        return False, "git holds it locked"
    if tree["marker"]:
        return False, "it has " + KEEP_MARKER
    if tree.get("kept_runs"):
        # A copy of a kept run would not be on the keep list, and clean_results.py would remove the copy (D152).
        name, line = tree["kept_runs"][0]
        return False, "%d runs on the keep list, the first %s (%s)" % (len(tree["kept_runs"]), name, line)
    if tree["used_by"]:
        return False, "in use: " + ", ".join(tree["used_by"][:3])
    if tree["depended_by"]:
        return False, "a Cargo.toml names it by path: " + ", ".join(tree["depended_by"][:3])
    if tree["status_error"]:
        return False, "git status failed: " + tree["status_error"]
    if tree["changes"]:
        return False, "%d changed or untracked files" % tree["changes"]
    if tree["other_ignored"]:
        return False, "ignored files that no build makes: " + ", ".join(tree["other_ignored"][:3])
    if tree["unread"]:
        return False, "%d folders could not be read" % tree["unread"]
    if tree["idle_hours"] < tree["idle_limit"]:
        return False, "written %.1f hours ago" % tree["idle_hours"]
    if not tree["on_a_ref"]:
        return False, "no ref holds its commit: make a branch first"
    if tree["named"]:
        return True, "named on the command line"
    if tree.get("old_set"):
        return True, "an old play set"
    if tree["branch"] is None:
        return False, "no branch (detached): only by name"
    if not tree["merged"]:
        return False, "not merged"
    return True, "merged"


# ---- the trees at risk

# The records that cite the runs and the trees of the work (the data rule, D71).
RECORDS_FILES = os.path.join(DOCS, "Numidium Project", "notes", "takeover", "*records*.md")
# What a checkout or a run leaves in a tree and no one writes: the machine-local paths (git ignores them) and the
# plugin's log files. They are no work that a lock would save.
LEFTOVERS = re.compile(r"(.*/)?(nirender\.log(\.\d+)?)/?")
# The file of a checkout's machine paths. Git ignores it. A tree's own is no work when it is the main checkout's.
LOCAL_JSON = "local.json"
# The words that the records use for a cleanup of worktrees ("It removed the merged and idle worktrees",
# "may go, merged and uncited", "the worktree C:/tmp/x is gone", "left for the owner to clean"). Bare
# "clean" and "remove" are no cleanup words: "a clean worktree" is a build check.
CLEANUP_SENTENCE = re.compile(r"\b(removed|may go|(is|are) gone|left for the owner to (clean|remove)|git worktree remove)\b")
# Folders that name a checkout, and no task: the lock reason takes the folder above.
GENERIC_FOLDERS = {"nirender-plugin", "mge-xe-g7", "mge-xe-g7-render-link", "render-link", "g7"}


def load_records(pattern=RECORDS_FILES):
    """The text of the records files as Evidence. Its `problems` say why a citation may be missing."""
    names = sorted(glob.glob(pattern))
    parts, problems = [], []
    for name in names:
        try:
            with open(name, encoding="utf-8", errors="replace") as file:
                parts.append(file.read())
        except OSError as error:
            problems.append("a records file cannot be read: %s (%s)" % (name, error))
    if not names:
        problems.append("no records file at " + pattern)
    return Evidence("\n".join(parts), problems)


def path_forms(path):
    """The spellings of a path in the records, lower case: with slashes (C:/tmp/x), with backslashes
    (C:\\tmp\\x) and as Git Bash writes it (/c/tmp/x)."""
    forward = os.path.abspath(path).replace("\\", "/").lower()
    forms = [forward, forward.replace("/", "\\")]
    if len(forward) > 2 and forward[1] == ":" and forward[2] == "/":
        forms.append("/" + forward[0] + forward[2:])
    return forms


def sentence_around(text, start, stop):
    """The sentence of `text` that holds text[start:stop]: from the previous ". " or line break to the
    next one. A colon or a semicolon does not end it."""
    before = max(text.rfind(". ", 0, start) + 1, text.rfind("\n", 0, start) + 1)
    ends = [end for end in (text.find(". ", stop), text.find("\n", stop)) if end != -1]
    return text[before:min(ends) if ends else len(text)]


def cites_path(records, path):
    """True when the records cite a tree by its path, in any of the spellings of `path_forms`, in any case.
    A path in a cleanup sentence does not count (CLEANUP_SENTENCE): it says that the tree went or may go,
    not that the records rely on it. A path into the tree's results folder counts in any sentence: it
    names a run."""
    text = records.text.lower()
    for form in path_forms(path):
        start = text.find(form)
        while start != -1:
            stop = start + len(form)
            if text.startswith(("/results/", "\\results\\"), stop) or not CLEANUP_SENTENCE.search(sentence_around(text, start, stop)):
                return True
            start = text.find(form, start + 1)
    return False


def local_json_state(tree_path, repository, ignored):
    """The ignored local.json at the root of a tree against the main checkout's: "none" (git does not list
    one as ignored), "same" (byte for byte) or "differs" (also when the main checkout has none or either
    cannot be read: the file may hold the session's own paths)."""
    if LOCAL_JSON not in ignored:
        return "none"
    try:
        with open(os.path.join(tree_path, LOCAL_JSON), "rb") as own, open(os.path.join(repository, LOCAL_JSON), "rb") as main:
            return "same" if own.read() == main.read() else "differs"
    except OSError:
        return "differs"


def uncommitted_beyond_output(status_lines):
    """The paths of a `git status --porcelain` list that are changed or untracked, a build's output
    left out (an untracked `results/` is no loss that a lock would save), and the leftovers of a
    checkout (LEFTOVERS). Ignored entries are no part of the list."""
    found = []
    for row in status_lines:
        if row.startswith("!! "):
            continue
        path = row[3:].split(" -> ")[-1].strip().strip('"')
        if not BUILD_OUTPUTS.fullmatch(path) and not LEFTOVERS.fullmatch(path):
            found.append(path)
    return found


def risk_reasons(tree):
    """Why a worktree is at risk when it is not locked: the lines of the reasons, empty when it is
    not at risk. A removal, by this tool or by hand, would lose these. A locked tree, a main or
    standing checkout and a tree under E:\\nirender-keep are never listed. `tree` is the facts
    of `facts_of`: `unique_commits` (None when git could not tell), `target`, `uncommitted`,
    `risky_ignored`, `local_json_differs`, `cites_path` and `cited_runs`."""
    path = tree["path"]
    if tree["locked"] or tree["main"] or any(same_path(path, os.path.join(REPOS_ROOT, name)) for name in SHARED_CHECKOUTS) or same_or_under(path, KEEP_ROOT):
        return []
    reasons = []
    if tree["status_error"]:
        reasons.append("git status failed, so its changes are not known: " + tree["status_error"])
    if tree["unique_commits"] is None:
        reasons.append("git could not say whether its commits are on a ref of %s, a remote or a tag" % tree["target"])
    elif tree["unique_commits"]:
        reasons.append("%d commits on no ref of %s, no remote and no tag" % (tree["unique_commits"], tree["target"]))
    if tree["uncommitted"]:
        reasons.append("%d uncommitted or untracked files: %s" % (len(tree["uncommitted"]), ", ".join(tree["uncommitted"][:3])))
    if tree["local_json_differs"]:
        reasons.append("local.json differs from the main checkout's")
    if tree["risky_ignored"]:
        reasons.append("ignored files that no build makes: " + ", ".join(tree["risky_ignored"][:3]))
    if tree["cites_path"]:
        reasons.append("the records cite its path")
    if tree["cited_runs"] and not tree.get("goes"):
        reasons.append("%d run folders that the records cite: %s" % (len(tree["cited_runs"]), ", ".join(tree["cited_runs"][:3])))
    return reasons


def go_line(row):
    """The line of a tree under MAY GO. A tree whose runs the records cite goes anyway: --apply copies
    the cited runs to the keep folder first, so the line says so in place of a risk."""
    line = "%9s  %s  [%s] %s; %d links; results: %s; idle %.0f h" % (gigabytes(row["size"]), row["path"], row["branch"] or "detached", row["reason"], len(row["links"]), "%d entries to keep" % row["result_runs"] if row["results"] else "none", min(row["idle_hours"], 9999))
    if row["cited_runs"]:
        line += "; %d cited runs, copied by --apply" % len(row["cited_runs"])
    return "  " + line


def print_risks(rows):
    """The two lists before the rest: the unlocked trees at risk with the command that locks each, and
    the locked trees with their reasons. `rows` are the facts of every tree, with `repository`."""
    risky = [(row, risk_reasons(row)) for row in rows]
    risky = [(row, reasons) for row, reasons in risky if reasons]
    print("\nTREES AT RISK, NOT LOCKED (%d). A removal would lose what is named; lock a tree whose work is not kept elsewhere:" % len(risky))
    for row, reasons in sorted(risky, key=lambda pair: pair[0]["path"]):
        print("  %s  [%s] %s" % (row["path"], row["branch"] or "detached", "; ".join(reasons)))
        print("      lock: " + lock_command(row, row["repository"]))
    locked = sorted((row for row in rows if row["locked"] and not row["main"]), key=lambda row: row["path"])
    print("\nLOCKED (%d), skipped by the tool:" % len(locked))
    for row in locked:
        print("  %s  [%s] %s" % (row["path"], row["branch"] or "detached", row["lock_reason"] or "no reason given"))


def lock_command(tree, repository):
    """The command that locks a worktree, so that this tool and `git worktree remove` leave it."""
    folder = os.path.basename(tree["path"])
    if folder.lower() in GENERIC_FOLDERS:
        folder = os.path.basename(os.path.dirname(tree["path"]))
    name = tree["branch"] or folder or "detached"
    return 'git -C "%s" worktree lock --reason "%s: work not kept elsewhere" "%s"' % (repository, name, tree["path"])


# ---- build output


def target_folders(tree_path):
    """The folders of a tree that a cargo build left, a link among them too: never below `results` or below a target."""
    found = []

    def scan(folder, depth):
        try:
            with os.scandir(folder) as entries:
                rows = list(entries)
        except OSError:
            return
        has_manifest = any(row.name == "Cargo.toml" for row in rows)
        for row in rows:
            if not row.is_dir(follow_symlinks=False) and not is_link_entry(row):
                continue
            if TARGET_NAME.fullmatch(row.name):
                if has_manifest or os.path.exists(os.path.join(row.path, "CACHEDIR.TAG")):
                    found.append(row.path)
            elif depth < TARGET_DEPTH and row.name not in (".git", "results") and not is_link_entry(row):
                scan(row.path, depth + 1)

    scan(tree_path, 1)
    return found


def output_folders(tree_path, output_only):
    """The folders that the sweep looks at in a tree: each target, and in MWSE its objects."""
    found = target_folders(tree_path)
    if output_only:
        found += [path for path in (os.path.join(tree_path, name) for name in MSBUILD_OBJECTS) if os.path.lexists(path)]
    return found


def output_facts(tree_path, folder, output_only, now):
    """All facts of one output folder for `decide_output`."""
    relative = os.path.relpath(folder, tree_path).replace("\\", "/")
    if is_link(folder):
        return {"tree": tree_path, "path": folder, "is_link": True, "size": 0, "links": [], "unread": 0, "idle_hours": 0.0, "ignored": True, "tracked": 0, "strange": [], "top": []}
    size, _, newest, links, unread = walk(folder)
    strange = []
    if output_only and not TARGET_NAME.fullmatch(os.path.basename(folder)):
        strange = [path for path, _, _ in files_in(folder) if not path.lower().endswith(OBJECT_FILES)]
    with os.scandir(folder) as entries:
        top = sorted(entry.name for entry in entries)
    return {
        "tree": tree_path,
        "path": folder,
        "is_link": False,
        "size": size,
        "links": links,
        "unread": unread,
        "idle_hours": (now - newest) / 3600.0 if newest else 1e9,
        "ignored": os.path.exists(os.path.join(folder, "CACHEDIR.TAG")) or git(tree_path, "check-ignore", "-q", relative + "/").returncode == 0,
        "tracked": len(git(tree_path, "ls-files", "--", relative).stdout.splitlines()),
        "strange": strange,
        "top": top,
    }


def decide_output(row, used_by):
    """What happens to an output folder: (True, the reason it goes) or (False, the reason it stays)."""
    if row["is_link"]:
        return False, "a link, and it stays"
    if not row["ignored"]:
        return False, "git does not ignore it"
    if row["tracked"]:
        return False, "git tracks %d files in it" % row["tracked"]
    if row["strange"]:
        return False, "%d files that no compiler makes: %s" % (len(row["strange"]), os.path.basename(row["strange"][0]))
    if row["unread"]:
        return False, "%d folders could not be read" % row["unread"]
    if used_by:
        return False, "in use: " + ", ".join(used_by[:3])
    if row["idle_hours"] < OUTPUT_HOURS:
        return False, "written %.1f hours ago" % row["idle_hours"]
    return True, "no write for %.0f hours" % min(row["idle_hours"], 9999)


def plan_outputs(trees, running, now):
    """Each output folder of the given trees with its decision."""
    skip = own_line(running, os.getpid())
    rows = []
    for tree_path, output_only in trees:
        used_by = users_of(tree_path, running, skip)
        for folder in output_folders(tree_path, output_only):
            row = output_facts(tree_path, folder, output_only, now)
            row["goes"], row["reason"] = decide_output(row, used_by)
            rows.append(row)
    return rows


def remove_output(row, now, running):
    """Removes one output folder after a new look at the processes and the writes. Returns the failures
    of the removal, or raises OSError when it refuses."""
    busy = games_in(running) + users_of(row["tree"], running, own_line(running, os.getpid()))
    if busy:
        raise OSError("in use now: " + ", ".join(busy[:3]))
    _, _, newest, _, unread = walk(row["path"])
    if unread or (newest and (now - newest) / 3600.0 < OUTPUT_HOURS):
        raise OSError("written since the list, or not readable")
    links, failures = remove_tree(row["path"])
    for link, target in links:
        log_line("  link removed: %s (its target %s stays: %s)" % (link, target, os.path.exists(target)), True)
    return failures


def other_outputs(trees):
    """Target folders under the folder of the repos that are in no tree of the project: only a list."""
    rows = []
    known = {os.path.normcase(path) for path, _ in trees}
    for name in sorted(os.listdir(REPOS_ROOT)):
        path = os.path.join(REPOS_ROOT, name)
        if not os.path.isdir(path) or os.path.normcase(path) in known:
            continue
        candidates = [path] if "target" in name.lower() else [os.path.join(path, "target")]
        for candidate in candidates:
            if os.path.isdir(candidate) and not is_link(candidate):
                rows.append((walk(candidate)[0], candidate))
    return sorted(rows, reverse=True)


# ---- the results of a worktree


def fingerprint(root, pictures=True):
    """The number and the size of the files of a folder, for the check of a copy: with no pictures
    when `pictures` is false. Raises OSError when it cannot read a folder."""
    found = [file for file in files_in(root) if pictures or not is_heavy(file[0])]
    return len(found), sum(file[1] for file in found)


def kept_copy(name, wanted, pictures, keep_root):
    """The folder under `keep_root`\\runs-* that holds this run with the same number and size of
    files, or None."""
    if not os.path.isdir(keep_root):
        return None
    for folder in sorted(os.listdir(keep_root)):
        if folder.startswith("runs-"):
            candidate = os.path.join(keep_root, folder, name)
            if os.path.isdir(candidate) and fingerprint(candidate, pictures) == wanted:
                return candidate
    return None


def real_results(tree_path):
    """The folder `results` of a tree when it is a real folder, else None (none, or a link)."""
    path = os.path.join(tree_path, "results")
    if not os.path.isdir(path) or os.lstat(path).st_file_attributes & FILE_ATTRIBUTE_REPARSE_POINT:
        return None
    return path


def kept_runs(results, names, keep):
    """The entries of a results folder that the keep list keeps, as (name, the list's line)."""
    if not results or keep is None:
        return []
    found = ((name, keep.kept_line(os.path.join(results, name))) for name in sorted(names))
    return [(name, line) for name, line in found if line is not None]


def copy_results(results, keep_root, date, evidence, now):
    """Copies what the expiry leaves of each entry of a real results folder, when it is not kept yet,
    to runs-<date>. Returns what it copied. Raises OSError when a copy does not agree, and before any
    copy when the keep list keeps an entry: the caller then leaves the tree. A link in the folder is not
    followed and not copied."""
    held = kept_runs(results, os.listdir(results), evidence)
    if held:
        raise OSError("the tree holds runs on the keep list, and a copy would not be on it: %s" % "; ".join("%s (%s)" % entry for entry in held))
    copied = []
    for name in sorted(os.listdir(results)):
        source = os.path.join(results, name)
        if is_link(source):
            continue
        if not os.path.isdir(source):
            target = os.path.join(keep_root, "runs-" + date, "loose-" + os.path.basename(os.path.dirname(results)), name)
            os.makedirs(os.path.dirname(target), exist_ok=True)
            shutil.copy2(source, target)
            if os.path.getsize(source) != os.path.getsize(target):
                raise OSError("the copy of %s differs" % source)
            copied.append(target)
            continue
        age = run_age_days(name, now)
        treatment = None if age is None else run_treatment(age, evidence.cites_run_with(name, files_in(source)), evidence.keeps(source))
        if treatment == "expired":
            continue
        pictures = treatment != "trim"
        wanted = fingerprint(source, pictures)
        if kept_copy(name, wanted, pictures, keep_root):
            continue
        target = os.path.join(keep_root, "runs-" + date, name)
        if os.path.exists(target):
            raise OSError("%s exists with other files than %s" % (target, source))
        shutil.copytree(source, target, ignore=lambda folder, names: uncopied(folder, names, pictures))
        if fingerprint(target) != wanted:
            raise OSError("the copy of %s differs: %s against %s" % (source, wanted, fingerprint(target)))
        copied.append(target)
    return copied


def uncopied(folder, names, pictures):
    """The entries of a folder that a copy leaves out: each link, and the pictures of a trimmed run."""
    left_out = []
    for name in names:
        path = os.path.join(folder, name)
        if is_link(path) or (not pictures and not os.path.isdir(path) and is_heavy(name)):
            left_out.append(name)
    return left_out


def path_dependencies(tree_path):
    """The folders that the Cargo.toml files of a tree name by path, the file's own tree left out.
    Only the tree's first two levels: the workspace and its members."""
    found = set()
    candidates = [os.path.join(tree_path, "Cargo.toml")]
    try:
        candidates += [os.path.join(tree_path, name, "Cargo.toml") for name in os.listdir(tree_path)]
    except OSError:
        return found
    for path in candidates:
        try:
            text = open(path, encoding="utf-8", errors="replace").read()
        except OSError:
            continue
        for relative in re.findall(r"""path\s*=\s*["']([^"']+)["']""", text):
            target = os.path.abspath(os.path.join(os.path.dirname(path), relative))
            if not same_or_under(target, tree_path):
                found.add(target)
    return found


# ---- git


def dependents_of(path, old_set, dependents):
    """The other trees whose Cargo.toml names this tree by path. The sibling of an old play set goes with it."""
    found = []
    for other, folders in dependents.items():
        sibling = old_set and same_or_under(other, os.path.dirname(path))
        if not same_or_under(other, path) and not sibling and any(same_or_under(folder, path) for folder in folders):
            found.append(os.path.basename(other))
    return sorted(found)


def facts_of(tree, target, repository, running, dependents, named, idle_limit, now, kept_sets=frozenset(), records=None, keep=None):
    """All facts of a worktree for `decide` and `risk_reasons`. `records` is the Evidence of the
    records files, or None when the citations are not checked. `keep` is an Evidence with the keep list,
    or None."""
    path = tree["path"]
    old_set = old_play_set(path, kept_sets)
    size, files, newest, links, unread = walk(path)
    head = tree["head"]
    merged = bool(tree["branch"]) and git(repository, "merge-base", "--is-ancestor", head, target).returncode == 0
    status_run = git(path, "status", "--porcelain", "--ignored")
    status = status_run.stdout.splitlines()
    ignored = [row[3:] for row in status if row.startswith("!! ")]
    # The root local.json holds a checkout's machine paths: it goes with the tree when it is the main checkout's.
    local_json = local_json_state(path, repository, ignored)
    kept_ignored = [name for name in ignored if not (name == LOCAL_JSON and local_json == "same")]
    refs = git(repository, "for-each-ref", "--contains", head, "--format=%(refname)")
    # The commits that the tree alone holds: no ref of the main branch, no remote and no tag has them.
    unique = git(path, "rev-list", "--count", "HEAD", "--not", target, "--remotes", "--tags")
    results = real_results(path)
    runs = os.listdir(results) if results else []
    return {
        **tree,
        "target": target,
        "unique_commits": int(unique.stdout) if unique.returncode == 0 and unique.stdout.strip().isdigit() else None,
        "uncommitted": uncommitted_beyond_output(status),
        "risky_ignored": [name for name in kept_ignored if not BUILD_OUTPUTS.fullmatch(name) and not LEFTOVERS.fullmatch(name) and name != LOCAL_JSON],
        "local_json_differs": local_json == "differs",
        "cites_path": records is not None and cites_path(records, path),
        "cited_runs": [name for name in runs if records is not None and records.cites_run(name)],
        "kept_runs": kept_runs(results, runs, keep),
        "merged": merged,
        "named": any(same_path(path, name) for name in named),
        "old_set": old_set,
        "status_error": status_run.stderr.strip() or "code %d" % status_run.returncode if status_run.returncode != 0 else "",
        "on_a_ref": refs.returncode == 0 and bool(refs.stdout.strip()),
        "changes": len(status) - len(ignored),
        "other_ignored": [name for name in kept_ignored if not BUILD_OUTPUTS.fullmatch(name)],
        "marker": os.path.exists(os.path.join(path, KEEP_MARKER)),
        "used_by": users_of(path, running),
        "depended_by": dependents_of(path, old_set, dependents),
        "idle_hours": (now - newest) / 3600.0 if newest else 1e9,
        "idle_limit": idle_limit,
        "unread": unread,
        "size": size,
        "files": files,
        "links": links,
        "results": results,
        "result_runs": len(runs),
    }


def clear_links(path, removed):
    """Removes each reparse point of a tree as a link, from a walk made now: a list from before may
    lack one, and git goes through a link. Adds each to `removed` as link -> target."""
    _, _, _, links, unread = walk(path)
    if unread:
        raise OSError("%d folders could not be read" % unread)
    for link in links:
        target = os.path.realpath(link)
        remove_link(link)
        removed.append("%s -> %s" % (link, target))
        log_line("  link removed: %s (its target %s stays: %s)" % (link, target, os.path.exists(target)), True)
    _, _, _, links, unread = walk(path)
    if links or unread:
        raise OSError("after the links went: %d links, %d folders could not be read" % (len(links), unread))


def remove(tree, repository, date, removed, evidence):
    """The three steps of removal. Returns what is left there, as (path, bytes), or raises OSError.
    `removed` gets the links that went, also when a later step fails."""
    path = tree["path"]
    results = real_results(path)
    if results:
        for copied in copy_results(results, KEEP_ROOT, date, evidence, time.time()):
            log_line("  copied to %s" % copied, True)
    clear_links(path, removed)
    result = git(repository, "worktree", "remove", path)
    if result.returncode != 0:
        raise OSError("git worktree remove: %s (the tree stays; a removal that failed midway leaves a part of it)" % result.stderr.strip())
    git(repository, "worktree", "prune")
    if os.path.lexists(path):
        return path, walk(path)[0]
    return None


def failure_text(error, removed):
    """What a failed removal did. The copy of results does not change the tree."""
    text = "  NOT REMOVED: %s" % error
    if removed:
        text += "; links removed, make them again if the tree stays: " + ", ".join(removed)
    return text


def plain_folders():
    """The folders of C:\\tmp that are no worktree of ours, with their size: only a list."""
    rows = []
    if os.path.isdir(PLAIN_FOLDERS):
        for name in sorted(os.listdir(PLAIN_FOLDERS)):
            path = os.path.join(PLAIN_FOLDERS, name)
            if os.path.isdir(path) and not os.path.lexists(os.path.join(path, ".git")):
                rows.append((walk(path)[0], path))
    return sorted(rows, reverse=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--apply", action="store_true", help="remove what the list names; else only the list")
    parser.add_argument("--name", action="append", default=[], help="a worktree path that may go though it is not merged or has no branch")
    parser.add_argument("--idle-hours", type=float, default=6.0, help="the hours since the last write in a worktree, at least")
    parser.add_argument("--during-game", action="store_true", help="the list while a game runs; --apply still refuses")
    parser.add_argument("--self-test", action="store_true", help="the tests of the pure parts and of a real junction")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if not args.apply:
        return clean(args)
    # No game may start in the middle of the cleaning (W363).
    try:
        with game_kept_free("clean_merged_worktrees.py"):
            return clean(args)
    except GameNotFree as error:
        print("NOTHING DONE: %s" % error)
        return 1


def clean(args):
    """The list, and with --apply the acts (inside `game_kept_free`)."""
    now = time.time()
    date = time.strftime("%Y-%m-%d")
    named = [os.path.abspath(name) for name in args.name]
    running, unread_processes = processes()
    games = games_in(running)
    if games and (args.apply or not args.during_game):
        print("NOTHING DONE: a game or a helper runs: " + ", ".join(games))
        print("A run writes into its worktree's results, and the walk would disturb a timing. --during-game gives the list anyway.")
        return 1
    repositories = []
    for name, target in REPOSITORIES:
        repository = os.path.join(REPOS_ROOT, name)
        repositories.append((repository, target, worktrees_of(repository)))
    # The dependencies come from every tree of every repository, with what it has on disk now.
    all_trees = [tree for _, _, trees in repositories for tree in trees]
    dependents = {tree["path"]: path_dependencies(tree["path"]) for tree in all_trees}
    jobs = [(tree, target, repository) for repository, target, trees in repositories for tree in trees if os.path.isdir(tree["path"])]
    current = current_set()
    kept_sets = kept_set_numbers(current, os.listdir(SETS) if os.path.isdir(SETS) else [])
    print("local.bat names the set %s; the play sets kept by number: %s" % (current, sorted(kept_sets)))
    records = load_records()
    kept_paths, keep_problems = read_keep_list()
    keep = Evidence("", keep_problems, kept_paths)
    for problem in keep_problems:
        print("THE KEEP LIST: " + problem)
    with ThreadPoolExecutor(8) as pool:
        rows = list(pool.map(lambda job: facts_of(job[0], job[1], job[2], running, dependents, named, args.idle_hours, now, kept_sets, records, keep), jobs))
    go, stay = [], []
    for row, job in zip(rows, jobs):
        ok, reason = decide(row)
        row["reason"], row["repository"], row["goes"] = reason, job[2], ok
        (go if ok else stay).append(row)
    print("%d processes, the current folder of %d could not be read" % (len(running), unread_processes))
    if games:
        print("a game or a helper runs: " + ", ".join(games))
    for problem in records.problems:
        print("THE RECORDS MAY LACK A CITATION: " + problem)
    print_risks(go + stay)
    print("\nMAY GO (%d, %s):" % (len(go), gigabytes(sum(row["size"] for row in go))))
    for row in sorted(go, key=lambda r: -r["size"]):
        print(go_line(row))
        for link in row["links"]:
            print("             link: %s -> %s" % (link, os.path.realpath(link)))
    print("\nSTAYS (%d, %s):" % (len(stay), gigabytes(sum(row["size"] for row in stay))))
    for row in sorted(stay, key=lambda r: -r["size"]):
        print("  %9s  %s  [%s] %s" % (gigabytes(row["size"]), row["path"], row["branch"] or "detached", row["reason"]))
    folders = plain_folders()
    print("\nPLAIN FOLDERS in %s, not touched (%s), the largest 15:" % (PLAIN_FOLDERS, gigabytes(sum(size for size, _ in folders))))
    for size, path in folders[:15]:
        print("  %9s  %s" % (gigabytes(size), path))
    # A worktree that goes takes its output with it.
    going = {os.path.normcase(row["path"]) for row in go}
    trees = project_trees()
    outputs = plan_outputs([tree for tree in trees if os.path.normcase(tree[0]) not in going], running, now)
    print("\nBUILD OUTPUT THAT GOES (%s):" % gigabytes(sum(row["size"] for row in outputs if row["goes"])))
    for row in sorted((row for row in outputs if row["goes"]), key=lambda r: -r["size"]):
        print("  %9s  %s  %s; %d links; top: %s" % (gigabytes(row["size"]), row["path"], row["reason"], len(row["links"]), " ".join(row["top"])[:160]))
        for link in row["links"]:
            print("             link: %s -> %s" % (link, os.path.realpath(link)))
    print("\nBUILD OUTPUT THAT STAYS (%s):" % gigabytes(sum(row["size"] for row in outputs if not row["goes"])))
    for row in sorted((row for row in outputs if not row["goes"]), key=lambda r: -r["size"]):
        print("  %9s  %s  %s" % (gigabytes(row["size"]), row["path"], row["reason"]))
    others = other_outputs(trees)
    print("\nOTHER TARGET FOLDERS under %s, in no tree of the project, not touched (%s):" % (REPOS_ROOT, gigabytes(sum(size for size, _ in others))))
    for size, path in others:
        print("  %9s  %s" % (gigabytes(size), path))
    evidence = Evidence.load()
    for problem in evidence.problems:
        print("THE EVIDENCE IS NOT WHOLE: " + problem)
    if not args.apply:
        print("\nA dry run: nothing changed. --apply acts.")
        return 0
    if evidence.problems:
        print("NOTHING DONE: a removal copies a run by its citations and the keep list, and either may be incomplete.")
        return 1
    left = []
    failed = 0
    for row in go:
        log_line("removing %s [%s] %s, %s" % (row["path"], row["branch"] or "detached", row["reason"], gigabytes(row["size"])), True)
        removed = []
        try:
            now_running, _ = processes()
            busy = games_in(now_running) + users_of(row["path"], now_running)
            if busy:
                raise OSError("in use now: " + ", ".join(busy[:3]))
            remains = remove(row, row["repository"], date, removed, evidence)
        except OSError as error:
            failed += 1
            log_line(failure_text(error, removed), True)
            continue
        log_line("  removed" if remains is None else "  git left the folder: %s" % gigabytes(remains[1]), True)
        if remains:
            left.append(remains)
    for row in outputs:
        if not row["goes"]:
            continue
        log_line("removing output %s, %s: %s" % (row["path"], gigabytes(row["size"]), row["reason"]), True)
        try:
            failures = remove_output(row, time.time(), processes()[0])
        except OSError as error:
            failed += 1
            log_line("  NOT REMOVED: %s" % error, True)
            continue
        for path, error in failures:
            log_line("  NOT REMOVED: %s (%s)" % (path, error), True)
        failed += len(failures)
    if left:
        print("\nFOLDERS THAT REMAIN, for the owner:")
        for path, size in left:
            print("  %9s  %s" % (gigabytes(size), path))
    return 1 if failed else 0


def make_junction(link, target):
    result = subprocess.run(["cmd", "/c", "mklink", "/J", link, target], capture_output=True, text=True)
    assert result.returncode == 0, result.stderr


def self_test():
    global KEEP_ROOT
    base = {"path": "C:\\repos\\nirender-plugin-x", "main": False, "branch": "fix/x", "merged": True, "named": False, "changes": 0, "other_ignored": [], "status_error": "", "on_a_ref": True, "locked": False, "marker": False, "used_by": [], "depended_by": [], "idle_hours": 48.0, "idle_limit": 6.0, "unread": 0}
    assert decide(base) == (True, "merged")
    for change, word in [
        ({"merged": False}, "not merged"),
        ({"branch": None}, "only by name"),
        ({"changes": 3}, "3 changed"),
        ({"status_error": "not a git repository"}, "git status failed"),
        ({"on_a_ref": False}, "no ref"),
        ({"path": HERE}, "runs from it"),
        ({"other_ignored": ["captures/"]}, "captures/"),
        ({"path": os.path.join(REPOS_ROOT, "MGE-XE-G7-train")}, "standing checkout"),
        ({"locked": True}, "locked"),
        ({"marker": True}, KEEP_MARKER),
        ({"used_by": ["cargo.exe 7"]}, "cargo.exe 7"),
        ({"depended_by": ["MGE-XE-G7-x"]}, "by path"),
        ({"idle_hours": 1.0}, "hours ago"),
        ({"unread": 2}, "could not be read"),
        ({"main": True}, "standing checkout"),
        ({"path": os.path.join(REPOS_ROOT, "MGE-XE-G7-render-link")}, "standing checkout"),
        ({"path": "C:\\tmp\\play185\\nirender-plugin"}, "play set"),
        ({"path": "E:\\nirender-keep\\x"}, "nirender-keep"),
    ]:
        ok, reason = decide({**base, **change})
        assert not ok and word in reason, (change, reason)
    # A name passes the tests of the branch, and no other test.
    assert decide({**base, "merged": False, "named": True})[0]
    assert decide({**base, "branch": None, "named": True})[0]
    for change in ({"changes": 1}, {"used_by": ["x 1"]}, {"marker": True}, {"path": "C:\\tmp\\play185\\x"}, {"main": True}, {"idle_hours": 0.5}, {"depended_by": ["y"]}, {"on_a_ref": False}, {"status_error": "x"}):
        assert not decide({**base, "named": True, **change})[0], change
    # An old play set needs no branch and no merge, and passes no other test.
    old = {**base, "path": "C:\\tmp\\play185\\nirender-plugin", "branch": None, "merged": False, "old_set": True}
    assert decide(old) == (True, "an old play set"), decide(old)
    for change in ({"changes": 1}, {"used_by": ["x 1"]}, {"marker": True}, {"idle_hours": 0.5}, {"on_a_ref": False}, {"unread": 1}, {"other_ignored": ["x/"]}):
        assert not decide({**old, **change})[0], change
    assert "play set" in decide({**old, "old_set": False})[1]
    # The sibling of an old play set names it by path, and goes with it. Any other tree still stops it.
    plugin, sibling, stranger = "C:\\tmp\\play185\\nirender-plugin", "C:\\tmp\\play185\\MGE-XE-G7-render-link", "C:\\repos\\MGE-XE-G7-x"
    names_it = {sibling: {plugin + "\\host"}, stranger: {"C:\\repos\\other\\host"}, plugin: set()}
    assert dependents_of(plugin, True, names_it) == [] and dependents_of(plugin, False, names_it) == ["MGE-XE-G7-render-link"]
    assert dependents_of(plugin, True, {**names_it, stranger: {plugin + "\\host"}}) == ["MGE-XE-G7-x"]
    assert BUILD_OUTPUTS.fullmatch("dist/") and not BUILD_OUTPUTS.fullmatch("tools/dist/")
    # The trees at risk, from the facts of a tree: each cause alone, a lock or a standing checkout ends the list.
    quiet = {**base, "target": "master", "unique_commits": 0, "uncommitted": [], "risky_ignored": [], "local_json_differs": False, "cites_path": False, "cited_runs": []}
    assert risk_reasons(quiet) == []
    for change, word in [
        ({"unique_commits": 2}, "2 commits on no ref of master, no remote and no tag"),
        ({"unique_commits": None}, "could not say"),
        ({"uncommitted": ["a.py", "b.py"]}, "2 uncommitted or untracked files: a.py, b.py"),
        ({"risky_ignored": ["captures/"]}, "ignored files that no build makes: captures/"),
        ({"local_json_differs": True}, "local.json differs from the main checkout's"),
        ({"cites_path": True}, "the records cite its path"),
        ({"cited_runs": ["20261104-010101-x"]}, "1 run folders that the records cite: 20261104-010101-x"),
        ({"status_error": "not a git repository"}, "git status failed"),
    ]:
        reasons = risk_reasons({**quiet, **change})
        assert len(reasons) == 1 and word in reasons[0], (change, reasons)
    assert len(risk_reasons({**quiet, "unique_commits": 1, "uncommitted": ["a"], "cited_runs": ["r"], "cites_path": True})) == 4
    # A tree that goes copies its cited runs with --apply: no risk, a note on its MAY GO line.
    cited = {**quiet, "cited_runs": ["20261104-010101-x", "r2"]}
    assert risk_reasons({**cited, "goes": True}) == [] and len(risk_reasons({**cited, "goes": False})) == 1
    assert len(risk_reasons({**cited, "goes": True, "cites_path": True})) == 1
    line = go_line({"size": 1 << 30, "path": "C:\\tmp\\x", "branch": "b", "reason": "merged", "links": [], "result_runs": 0, "results": False, "idle_hours": 3.0, "cited_runs": cited["cited_runs"]})
    assert line.endswith("; 2 cited runs, copied by --apply") and "[b] merged; 0 links; results: none" in line, line
    assert "cited runs" not in go_line({"size": 0, "path": "C:\\tmp\\x", "branch": None, "reason": "merged", "links": [], "result_runs": 0, "results": False, "idle_hours": 3.0, "cited_runs": []})
    for change in ({"locked": True}, {"main": True}, {"path": os.path.join(REPOS_ROOT, "MGE-XE-G7-train")}, {"path": "E:\\nirender-keep\\x"}):
        assert risk_reasons({**quiet, "unique_commits": 5, "uncommitted": ["a"], **change}) == [], change
    assert uncommitted_beyond_output(["?? new.txt", " M a.py", "?? results/", "!! target/", "R  old.rs -> new.rs", '?? "a b.txt"', "?? tools/__pycache__/", "?? host/target/", "?? nirender.log.1", "?? sub/local.json"]) == ["new.txt", "a.py", "new.rs", "a b.txt", "sub/local.json"]
    assert LEFTOVERS.fullmatch("nirender.log") and LEFTOVERS.fullmatch("nirender.log.12") and not LEFTOVERS.fullmatch("local.json")
    assert not LEFTOVERS.fullmatch("nirender.log.old") and not LEFTOVERS.fullmatch("notes.json")
    assert 'worktree lock --reason "nirender-plugin-bc7: work' in lock_command({"branch": None, "path": "C:\\repos\\nirender-plugin-bc7"}, "C:\\repos\\r")
    assert lock_command({"branch": None, "path": "C:\\tmp\\shadowv\\nirender-plugin"}, "C:\\repos\\r") == 'git -C "C:\\repos\\r" worktree lock --reason "shadowv: work not kept elsewhere" "C:\\tmp\\shadowv\\nirender-plugin"'
    # The decision on an output folder.
    output = {"is_link": False, "ignored": True, "tracked": 0, "strange": [], "unread": 0, "idle_hours": 49.0}
    assert decide_output(output, [])[0]
    for change, used_by, word in [
        ({"is_link": True}, [], "a link"),
        ({"ignored": False}, [], "does not ignore"),
        ({"tracked": 2}, [], "tracks 2"),
        ({"strange": ["C:\\x\\MWSE.dll.FIXED"]}, [], "MWSE.dll.FIXED"),
        ({"unread": 1}, [], "could not be read"),
        ({}, ["cargo.exe 7"], "cargo.exe 7"),
        ({"idle_hours": 47.0}, [], "hours ago"),
    ]:
        ok, reason = decide_output({**output, **change}, used_by)
        assert not ok and word in reason, (change, reason)
    for name in ("target/", "host/target/", "bin/", "results/", "tools/out/", "tools/__pycache__/", "tools/ports/__pycache__/"):
        assert BUILD_OUTPUTS.fullmatch(name), name
    for name in ("captures/", "tools/target/", "bin/notes.txt", "results"):
        assert not BUILD_OUTPUTS.fullmatch(name), name
    assert games_in([{"pid": 5, "name": "Morrowind.exe"}, {"pid": 6, "name": "cargo.exe"}]) == ["Morrowind.exe 5"]
    assert same_or_under("C:\\a\\b\\c", "C:\\a\\b") and not same_or_under("C:\\a\\bc", "C:\\a\\b")
    assert users_of("C:\\repos\\tree", [{"pid": 1, "name": "cargo.exe", "exe": "", "command": 'cargo build --manifest-path "C:/repos/tree/Cargo.toml"', "cwd": ""}]) == ["cargo.exe 1"]
    assert users_of("C:\\repos\\tree", [{"pid": 3, "name": "bash", "exe": "", "command": "bash -c 'cd /c/repos/tree/src'", "cwd": ""}]) == ["bash 3"]
    assert users_of("C:\\repos\\tree", [{"pid": 1, "name": "x", "exe": "", "command": "x C:\\repos\\tree-2\\y", "cwd": ""}]) == []
    assert users_of("C:\\repos\\tree", [{"pid": 2, "name": "sh", "exe": "", "command": "sh", "cwd": "C:\\repos\\tree\\src\\"}]) == ["sh 2"]
    assert current_folder(os.getpid()) is not None and same_or_under(os.getcwd(), current_folder(os.getpid()))

    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp:
        # A real junction: the walk does not go through it, and its removal leaves its target.
        outside = os.path.join(temp, "outside")
        os.makedirs(outside)
        open(os.path.join(outside, "precious.txt"), "w").write("x" * 1000)
        tree = os.path.join(temp, "tree")
        os.makedirs(os.path.join(tree, "src"))
        open(os.path.join(tree, "src", "a.txt"), "w").write("abc")
        link = os.path.join(tree, "results")
        make_junction(link, outside)
        size, files, _, links, unread = walk(tree)
        assert (size, files, links, unread) == (3, 1, [link], 0), (size, files, links, unread)
        assert real_results(tree) is None
        remove_link(link)
        assert not os.path.lexists(link) and os.path.getsize(os.path.join(outside, "precious.txt")) == 1000

        # A path with an ampersand: no shell reads it as a second command.
        odd = os.path.join(tree, "a&b")
        _winapi.CreateJunction(outside, odd)
        remove_link(odd)
        assert not os.path.lexists(odd) and os.path.exists(os.path.join(outside, "precious.txt"))

        # A real results folder: a copy, the check of it, and a run that is kept already.
        results = os.path.join(tree, "results")
        os.makedirs(os.path.join(results, "run1"))
        open(os.path.join(results, "run1", "x.log"), "w").write("12345")
        os.makedirs(os.path.join(results, "run2"))
        open(os.path.join(results, "run2", "y.log"), "w").write("1")
        open(os.path.join(results, "loose.txt"), "w").write("22")
        keep = os.path.join(temp, "keep")
        os.makedirs(os.path.join(keep, "runs-old", "run2"))
        open(os.path.join(keep, "runs-old", "run2", "y.log"), "w").write("1")
        assert real_results(tree) == results
        nothing = Evidence("")
        now = time.time()
        copied = copy_results(results, keep, "today", nothing, now)
        assert sorted(os.path.basename(c) for c in copied) == ["loose.txt", "run1"], copied
        assert open(os.path.join(keep, "runs-today", "run1", "x.log")).read() == "12345"
        assert not os.path.exists(os.path.join(keep, "runs-today", "run2"))
        # A folder of that name with other files: a refusal, not a mix.
        os.makedirs(os.path.join(results, "run3"))
        open(os.path.join(results, "run3", "z.log"), "w").write("abc")
        os.makedirs(os.path.join(keep, "runs-today", "run3"))
        open(os.path.join(keep, "runs-today", "run3", "other.log"), "w").write("d")
        try:
            copy_results(results, keep, "today", nothing, now)
            raise AssertionError("no refusal")
        except OSError as error:
            assert "other files" in str(error)

        # The expiry decides what a removal copies, and a junction in a run stays behind.
        results2 = os.path.join(temp, "results2")

        def run_folder(days):
            name = time.strftime("%Y%m%d-%H%M%S", time.localtime(now - days * 86400)) + "-r"
            os.makedirs(os.path.join(results2, name))
            for file in ("a.png", "b.log", "c.dmp"):
                open(os.path.join(results2, name, file), "w").write("x" * 10)
            return name

        young, trimmed, expired, cited, kept = run_folder(2), run_folder(10), run_folder(40), run_folder(50), run_folder(60)
        make_junction(os.path.join(results2, trimmed, "link"), outside)
        make_junction(os.path.join(results2, "run-link"), outside)
        keep2 = os.path.join(temp, "keep2")
        os.makedirs(keep2)
        # A run on the keep list stops the copy before it starts (D152, the Reviewer's A7): its copy would not be on
        # the list, and clean_results.py would remove the copy. The error names the run and its line.
        listed = Evidence("a record cites " + cited, kept={os.path.join(results2, kept): "line 3: the kept run | owner | 2026-11-09"})
        try:
            copy_results(results2, keep2, "today", listed, now)
            raise AssertionError("no refusal")
        except OSError as error:
            assert "keep list" in str(error) and kept in str(error) and "line 3" in str(error), str(error)
        assert os.listdir(keep2) == [], "nothing copied"
        # The null: without the line, the same folder copies. A cited old run copies as text only, and the old
        # uncited run (the one that was kept) not at all.
        evidence2 = Evidence("a record cites " + cited)
        copied = copy_results(results2, keep2, "today", evidence2, now)
        assert sorted(os.path.basename(c) for c in copied) == sorted([young, trimmed, cited]), copied
        target = os.path.join(keep2, "runs-today")
        assert sorted(os.listdir(os.path.join(target, young))) == ["a.png", "b.log", "c.dmp"]
        assert sorted(os.listdir(os.path.join(target, cited))) == ["b.log", "c.dmp"]
        assert sorted(os.listdir(os.path.join(target, trimmed))) == ["b.log", "c.dmp"]
        assert not os.path.exists(os.path.join(target, expired)) and not os.path.lexists(os.path.join(target, "run-link"))
        # A second removal finds the same files kept, with or without the pictures.
        assert copy_results(results2, keep2, "later", evidence2, now) == []

        # The output folders of a tree: its target with a link in it, a member's, a target with no build's marks, a link.
        built = os.path.join(temp, "built")
        os.makedirs(os.path.join(built, "host", "target", "release"))
        os.makedirs(os.path.join(built, "target", "release"))
        os.makedirs(os.path.join(built, "target-mine"))
        os.makedirs(os.path.join(built, "docs", "target"))
        os.makedirs(os.path.join(built, "results", "x", "target"))
        os.makedirs(os.path.join(built, "src"))
        for manifest in (os.path.join(built, "Cargo.toml"), os.path.join(built, "host", "Cargo.toml")):
            open(manifest, "w").write("[package]\n")
        open(os.path.join(built, "src", "lib.rs"), "w").write("source")
        open(os.path.join(built, "target", "release", "a.dll"), "w").write("xx")
        open(os.path.join(built, "host", "target", "CACHEDIR.TAG"), "w").write("tag")
        make_junction(os.path.join(built, "target", "release", "out"), outside)
        make_junction(os.path.join(built, "host", "target-link"), outside)
        found = sorted(os.path.relpath(path, built) for path in target_folders(built))
        assert found == sorted(["target", "target-mine", os.path.join("host", "target"), os.path.join("host", "target-link")]), found
        links, failures = remove_tree(os.path.join(built, "target"))
        assert failures == [] and [link for link, _ in links] == [os.path.join(built, "target", "release", "out")], (links, failures)
        assert not os.path.exists(os.path.join(built, "target")) and os.path.exists(os.path.join(built, "src", "lib.rs"))
        assert os.path.getsize(os.path.join(outside, "precious.txt")) == 1000
        try:
            remove_tree(os.path.join(built, "host", "target-link"))
            raise AssertionError("a link went")
        except OSError as error:
            assert "is a link" in str(error)
        assert os.path.getsize(os.path.join(outside, "precious.txt")) == 1000
        # A new look just before the removal: a write since the list refuses it, and a later one passes.
        fresh = os.path.join(built, "host", "target")
        # Cargo's mark counts where no .gitignore names the folder, as this tree has none.
        assert output_facts(built, fresh, False, time.time())["ignored"] and not output_facts(built, os.path.join(built, "target-mine"), False, time.time())["ignored"]
        row = {"tree": built, "path": fresh}
        for running, word in (([], "written since"), ([{"pid": 5, "parent": 0, "name": "Morrowind.exe", "exe": "", "command": "", "cwd": ""}], "Morrowind.exe 5"), ([{"pid": 6, "parent": 0, "name": "cargo.exe", "exe": "", "command": "", "cwd": fresh}], "cargo.exe 6")):
            later = time.time() + (0 if word == "written since" else 3 * 86400)
            try:
                remove_output(row, later, running)
                raise AssertionError("no refusal: " + word)
            except OSError as error:
                assert word in str(error), (word, error)
        assert remove_output(row, time.time() + 3 * 86400, []) == [] and not os.path.exists(fresh)
        # MWSE's objects: only with a compiler's files, and the search finds them only in such a tree.
        objects = os.path.join(built, "MWSE", "Release")
        os.makedirs(objects)
        open(os.path.join(objects, "a.obj"), "w").write("o")
        assert output_folders(built, True)[-1] == objects and objects not in output_folders(built, False)
        assert output_facts(built, objects, True, time.time())["strange"] == []
        open(os.path.join(objects, "MWSE.dll.FIXED"), "w").write("o")
        assert len(output_facts(built, objects, True, time.time())["strange"]) == 1
        # The path of a Cargo.toml, here in a member of the workspace.
        member = os.path.join(temp, "other", "member")
        os.makedirs(member)
        open(os.path.join(member, "Cargo.toml"), "w").write('a = { path = "../../tree/host" }\nb = { path = "../inside" }\n')
        assert path_dependencies(os.path.join(temp, "other")) == {os.path.join(temp, "tree", "host")}

        # A real repository and real worktrees, with the log and the keep folder here.
        KEEP_ROOT = keep
        common.LOG = os.path.join(temp, "test.log")
        repository = os.path.join(temp, "repository")
        commit = ("-c", "user.name=test", "-c", "user.email=test@test", "-c", "commit.gpgsign=false", "commit", "-q", "--allow-empty", "-m", "x")
        assert git(temp, "init", "-q", "-b", "master", repository).returncode == 0
        open(os.path.join(repository, ".gitignore"), "w").write("target/\nlocal.json\n")
        git(repository, "add", ".gitignore")
        assert git(repository, *commit).returncode == 0

        def worktree(name, detached=False):
            path = os.path.join(temp, name)
            assert git(repository, "worktree", "add", "-q", "-b", name, path).returncode == 0
            if detached:
                assert git(path, "checkout", "-q", "--detach").returncode == 0
            return path

        def facts(path, branch, records=None, keep=None):
            tree = {"path": path, "main": False, "branch": branch, "head": git(path, "rev-parse", "HEAD").stdout.strip(), "locked": False, "lock_reason": "", "prunable": False}
            return facts_of(tree, "master", repository, [], {}, [], 0.0, time.time(), frozenset(), records, keep)

        # A junction that came after the facts: the removal finds it, and its target stays.
        late = worktree("late")
        row = facts(late, "late")
        assert row["links"] == [] and decide(row) == (True, "merged"), decide(row)
        os.makedirs(os.path.join(late, "target"))
        outside2 = os.path.join(temp, "outside2")
        os.makedirs(outside2)
        open(os.path.join(outside2, "precious.txt"), "w").write("y" * 10)
        make_junction(os.path.join(late, "target", "link"), outside2)
        removed = []
        assert remove(row, repository, "today", removed, nothing) is None
        assert not os.path.lexists(late) and len(removed) == 1
        assert os.path.getsize(os.path.join(outside2, "precious.txt")) == 10

        # A status that fails does not read as clean: the state of a half-failed removal.
        broken = worktree("broken")
        os.remove(os.path.join(broken, ".git"))
        row = facts(broken, "broken")
        assert not decide(row)[0] and "git status failed" in decide(row)[1], decide(row)

        # A name does not take a commit that no ref holds, and does take one that a ref holds.
        loose = worktree("loose", detached=True)
        row = {**facts(loose, None), "named": True}
        assert decide(row) == (True, "named on the command line"), decide(row)
        assert git(loose, *commit).returncode == 0
        row = {**facts(loose, None), "named": True}
        assert not decide(row)[0] and "no ref" in decide(row)[1], decide(row)

        # A removal that git refuses after the links went says so, with the target of each link.
        refused = worktree("refused")
        row = facts(refused, "refused")
        os.makedirs(os.path.join(refused, "target"))
        make_junction(os.path.join(refused, "target", "link"), outside2)
        open(os.path.join(refused, "new.txt"), "w").write("z")
        removed = []
        try:
            remove(row, repository, "today", removed, nothing)
            raise AssertionError("no refusal")
        except OSError as error:
            text = failure_text(error, removed)
        assert "NOT REMOVED" in text and "the tree stays" in text and "links removed" in text and outside2 in text, text
        assert "links removed" not in failure_text(OSError("x"), [])
        assert os.path.exists(os.path.join(refused, "new.txt"))

        # The trees at risk, from real worktrees. Work that only a local branch of the tree holds:
        ahead = worktree("ahead")
        assert git(ahead, *commit).returncode == 0
        row = facts(ahead, "ahead")
        assert row["unique_commits"] == 1 and "1 commits on no ref of master" in risk_reasons(row)[0], (row["unique_commits"], risk_reasons(row))
        # A remote ref or a tag that holds the commit ends the risk, and so does the main branch.
        head = git(ahead, "rev-parse", "HEAD").stdout.strip()
        assert git(repository, "update-ref", "refs/remotes/origin/ahead", head).returncode == 0
        assert facts(ahead, "ahead")["unique_commits"] == 0 and risk_reasons(facts(ahead, "ahead")) == []
        assert git(repository, "update-ref", "-d", "refs/remotes/origin/ahead").returncode == 0
        assert facts(ahead, "ahead")["unique_commits"] == 1
        assert git(repository, "tag", "kept-tag", head).returncode == 0
        assert facts(ahead, "ahead")["unique_commits"] == 0
        assert git(repository, "tag", "-d", "kept-tag").returncode == 0
        assert git(repository, "merge", "-q", "--ff-only", "ahead").returncode == 0
        assert facts(ahead, "ahead")["unique_commits"] == 0
        # Uncommitted files, with a build's output left out.
        dirty = worktree("dirty")
        assert facts(dirty, "dirty")["uncommitted"] == [] and risk_reasons(facts(dirty, "dirty")) == []
        os.makedirs(os.path.join(dirty, "results", "20261104-010101-x"))
        open(os.path.join(dirty, "results", "20261104-010101-x", "a.log"), "w").write("a")
        assert facts(dirty, "dirty")["uncommitted"] == [] and risk_reasons(facts(dirty, "dirty")) == []
        open(os.path.join(dirty, "notes.txt"), "w").write("n")
        row = facts(dirty, "dirty")
        assert row["uncommitted"] == ["notes.txt"] and "1 uncommitted or untracked files: notes.txt" in risk_reasons(row), risk_reasons(row)
        # A run folder that the records cite, and a tree that the records name by its path.
        records = Evidence("a record cites 20261104-010101 and the tree " + dirty.replace("\\", "/").upper())
        row = facts(dirty, "dirty", records)
        assert row["cited_runs"] == ["20261104-010101-x"] and row["cites_path"], (row["cited_runs"], row["cites_path"])
        assert any("the records cite its path" in reason for reason in risk_reasons(row)) and any("1 run folders that the records cite" in reason for reason in risk_reasons(row))
        assert not any("run folders" in reason for reason in risk_reasons({**row, "goes": True})) and any("run folders" in reason for reason in risk_reasons({**row, "goes": False}))
        assert facts(dirty, "dirty", Evidence("nothing"))["cited_runs"] == []
        # A merged tree with a run on the keep list stays, and the list names the run and its line (A7). The null:
        # without the line, the reason is the next one, the untracked results folder (this repository ignores no
        # results; a real one does, and the tree goes).
        held = worktree("held")
        os.makedirs(os.path.join(held, "results", "20261104-020202-k"))
        open(os.path.join(held, "results", "20261104-020202-k", "a.png"), "w").write("p")
        keep = Evidence("", kept={os.path.join(held, "results", "20261104-020202-k").upper(): "line 7: the held results | open: risk 76 | 2026-11-09"})
        row = facts(held, "held", None, keep)
        assert row["kept_runs"] == [("20261104-020202-k", "line 7: the held results | open: risk 76 | 2026-11-09")], row["kept_runs"]
        assert not decide(row)[0] and "1 runs on the keep list, the first 20261104-020202-k (line 7" in decide(row)[1], decide(row)
        assert facts(held, "held", None, Evidence(""))["kept_runs"] == [] and decide(facts(held, "held", None, Evidence(""))) == (False, "1 changed or untracked files")
        assert facts(held, "held")["kept_runs"] == []
        # The records files: read as one text, and a missing file is a problem, not a silence.
        records_folder = os.path.join(temp, "records")
        os.makedirs(records_folder)
        open(os.path.join(records_folder, "plan-forward-records-x.md"), "w").write("entry: " + dirty + "\n20261104-010101-x\n")
        open(os.path.join(records_folder, "other.md"), "w").write("not read")
        loaded = load_records(os.path.join(records_folder, "*records*.md"))
        assert loaded.problems == [] and cites_path(loaded, dirty.upper()) and loaded.cites_run("20261104-010101-x") and "not read" not in loaded.text
        # The spellings of a path: slashes, backslashes and the Git Bash form /c/...; another drive or folder is no match.
        tree_path = "C:\\tmp\\p15\\nirender-plugin"
        assert path_forms(tree_path) == ["c:/tmp/p15/nirender-plugin", "c:\\tmp\\p15\\nirender-plugin", "/c/tmp/p15/nirender-plugin"]
        for spelling in ("C:/tmp/p15/nirender-plugin/results", "C:\\tmp\\p15\\nirender-plugin\\results", "/c/tmp/p15/nirender-plugin/results", "/C/TMP/P15/nirender-plugin"):
            assert cites_path(Evidence("the run in " + spelling + " shows"), tree_path), spelling
        for other in ("/d/tmp/p15/nirender-plugin", "/c/tmp/p16/nirender-plugin", "/tmp/p15/nirender-plugin", "nothing"):
            assert not cites_path(Evidence("the run in " + other), tree_path), other
        # A path that the records name only in a cleanup sentence is no citation. The sentences are the records' own.
        for cleanup in (
            "It removed the merged and idle worktrees, the old play set `C:/tmp/p15/nirender-plugin`, and more.",
            "Cleanup: removed `C:/tmp/p15/nirender-plugin`, `C:/tmp/other`.",
            "The worktree `C:/tmp/p15/nirender-plugin` is gone and its branch was merged.",
            "Left for the owner to clean: the worktree C:/tmp/p15/nirender-plugin (`git worktree remove` deletes a folder).",
            "Left for the owner to remove: /c/tmp/p15/nirender-plugin.",
            "MAY GO, merged and uncited: cullrel, C:\\tmp\\p15\\nirender-plugin.",
            "Earlier text.\nremoved C:/tmp/p15/nirender-plugin\nLater text.",
        ):
            assert not cites_path(Evidence(cleanup), tree_path), cleanup
        for cited_text in (
            "The walk in C:/tmp/p15/nirender-plugin shows 3 draws. Nothing was removed.",
            "A clean worktree C:/tmp/p15/nirender-plugin was built for the check.",
            "removed C:/tmp/p15/nirender-plugin. Later the run C:/tmp/p15/nirender-plugin/results/x showed it.",
            "It removed the old trees.\nThe run in C:/tmp/p15/nirender-plugin/results/x shows it.",
            "removed C:/tmp/p15/nirender-plugin. The log is in /c/tmp/p15/nirender-plugin/results. The check ran.",
        ):
            assert cites_path(Evidence(cited_text), tree_path), cited_text
        assert cites_path(Evidence("may go: ryker-w7 (runs `C:/tmp/p15/nirender-plugin/results/20261103-200212-x` and more);"), tree_path)
        assert not cites_path(Evidence("removed build output (the largest `C:/tmp/p15/nirender-plugin/target` 3.9 GB)"), tree_path)
        assert "no records file" in load_records(os.path.join(temp, "none", "*records*.md")).problems[0]
        # A lock takes a tree off the list and puts it on the list of the locked, with its reason.
        assert git(repository, "worktree", "lock", "--reason", "Ryker: a task of D98", dirty).returncode == 0
        locked = next(tree for tree in worktrees_of(repository) if same_path(tree["path"], dirty))
        assert locked["locked"] and locked["lock_reason"] == "Ryker: a task of D98", locked
        locked_row = facts_of(locked, "master", repository, [], {}, [], 0.0, time.time(), frozenset(), records)
        assert risk_reasons(locked_row) == []
        at_risk = {**facts(ahead, "ahead"), "unique_commits": 3, "repository": repository}
        text = io.StringIO()
        with contextlib.redirect_stdout(text):
            print_risks([at_risk, {**locked_row, "repository": repository}])
        text = text.getvalue()
        assert "TREES AT RISK, NOT LOCKED (1)" in text and "3 commits on no ref of master" in text, text
        assert 'lock: git -C "%s" worktree lock --reason "ahead: work not kept elsewhere" "%s"' % (repository, ahead) in text, text
        assert "LOCKED (1), skipped by the tool" in text and "Ryker: a task of D98" in text and text.index("TREES AT RISK") < text.index("LOCKED (1)"), text
        # The tool skips a locked tree as before.
        assert decide({**base, **{"locked": True}}) == (False, "git holds it locked")

        # The ignored local.json at the root: the main checkout's own, byte for byte, goes with the tree;
        # another one keeps the tree and shows in the list of the trees at risk.
        open(os.path.join(repository, LOCAL_JSON), "w").write('{"paths": "main"}')
        same = worktree("same_paths")
        open(os.path.join(same, LOCAL_JSON), "w").write('{"paths": "main"}')
        row = facts(same, "same_paths")
        assert row["other_ignored"] == [] and row["local_json_differs"] is False and decide(row) == (True, "merged"), (row["other_ignored"], decide(row))
        assert risk_reasons(row) == []
        other = worktree("own_paths")
        open(os.path.join(other, LOCAL_JSON), "w").write('{"paths": "mine"}')
        row = facts(other, "own_paths")
        assert row["other_ignored"] == [LOCAL_JSON] and row["local_json_differs"] is True, row["other_ignored"]
        assert not decide(row)[0] and "local.json" in decide(row)[1], decide(row)
        assert risk_reasons(row) == ["local.json differs from the main checkout's"], risk_reasons(row)
        # A main checkout with no local.json: the tree's differs.
        os.remove(os.path.join(repository, LOCAL_JSON))
        row = facts(same, "same_paths")
        assert row["local_json_differs"] is True and not decide(row)[0]
        # A local.json below the root is another ignored file, and keeps the tree.
        open(os.path.join(repository, LOCAL_JSON), "w").write('{"paths": "main"}')
        os.makedirs(os.path.join(same, "sub"))
        open(os.path.join(same, "sub", LOCAL_JSON), "w").write('{"paths": "main"}')
        row = facts(same, "same_paths")
        assert row["other_ignored"] == ["sub/"] and not decide(row)[0], row["other_ignored"]
        assert risk_reasons(row) == ["ignored files that no build makes: sub/"], risk_reasons(row)
    print("clean_merged_worktrees self-test: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
