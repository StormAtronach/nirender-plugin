"""Removes the side worktrees of the nirender and MGE XE repositories whose work is merged, so that
builds do not pile up. A step of each merge. Without `--apply` it only lists.

    python tools/clean_merged_worktrees.py                  # the list: each worktree, its size, why
    python tools/clean_merged_worktrees.py --apply          # remove the ones the list names
    python tools/clean_merged_worktrees.py --name <path>    # also a worktree that is not merged or
                                                            # has no branch: the owner's list
    python tools/clean_merged_worktrees.py --during-game    # the list while a game runs: the walk
                                                            # reads the whole disk, so never in a timing
    python -I tools/clean_merged_worktrees.py --self-test   # the pure parts and a real junction

All of these must hold before a worktree goes:

- Its branch is merged into the target of its repository, or `--name` names it.
- `git status` shows no change and no untracked file, and each ignored file is a build output or
  in `results`: `git worktree remove` deletes ignored files without a word.
- No process uses it: a command line or a program path in it, or a current folder in it. This is
  looked at again just before the removal. It does not see a session that has a worktree in mind
  and no command running there: a standing worktree needs `.keep-worktree` or a line in
  SHARED_CHECKOUTS.
- No game and no helper runs: a run writes into its worktree's results, and the walk would disturb
  a timing. The tool stops before the walk.
- Nothing was written in it for `--idle-hours` hours, and it has no `.keep-worktree` file, and
  git does not hold it locked.
- No other worktree's Cargo.toml names it by path.
- It is no main checkout, no standing worktree of the train and no play set, and it is not under
  E:\\nirender-keep.

Then, in this order, and any failure leaves the worktree as it is, apart from what came before:

1. A real `results` folder is copied to E:\\nirender-keep\\runs-<date>, unless a runs-* folder holds
   the same run; the number and the size of the files must agree.
2. Each reparse point in the tree is removed as a link, which leaves its target. `git worktree
   remove` goes through a junction: on 2026-10-29 one deleted E:\\nirender-results that way.
3. `git worktree remove`, never --force, then `git worktree prune`. Branches stay.

What `git worktree remove` leaves goes on a list with its size, for the owner: this tool makes no
recursive delete of its own. The plain folders in C:\\tmp are listed too, and nothing touches them.
`cargo sweep` is not here: it finds nothing in the layout of our targets (the board, 2026-10-29).

Each act goes to E:\\nirender-keep\\clean-worktrees.log.
"""
import argparse
import ctypes
import ctypes.wintypes as wt
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
REPOS_ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
# The repositories: a checkout of each, and the branch that a worktree's work merges into.
REPOSITORIES = [("nirender-plugin", "master"), ("MGE-XE-G7-render-link", "feature/wgpu-renderer")]
# The shared checkouts and the test train's pair stay, whatever their branch has. Git's own main
# worktree is one too.
SHARED_CHECKOUTS = ["nirender-plugin", "MGE-XE-G7", "MGE-XE-G7-render-link", "nirender-plugin-train", "MGE-XE-G7-train"]
# What a build leaves, as `git status --ignored` names it. `results` is copied before a removal.
BUILD_OUTPUTS = re.compile(r"(target|bin|results|tools/out)/|(.*/)?__pycache__/")
KEEP_ROOT = "E:\\nirender-keep"
LOG = os.path.join(KEEP_ROOT, "clean-worktrees.log")
PLAIN_FOLDERS = "C:\\tmp"
KEEP_MARKER = ".keep-worktree"
GAME_PROGRAMS = {"morrowind.exe", "numidium.exe", "nirender-host.exe"}
FILE_ATTRIBUTE_REPARSE_POINT = 0x400


def same_path(path, other):
    return os.path.normcase(os.path.abspath(path)) == os.path.normcase(os.path.abspath(other))


def same_or_under(path, root):
    """True when `path` is `root` or a path below it, on Windows' terms."""
    path, root = os.path.normcase(os.path.abspath(path)), os.path.normcase(os.path.abspath(root))
    return path == root or path.startswith(root.rstrip("\\/") + os.sep)


def is_play_set(path):
    """True for a tree of a play set: C:\\tmp\\play185\\..., or a folder with a name of that kind."""
    return any(re.fullmatch(r"play[-_]?\d*.*", part, re.I) for part in os.path.abspath(path).split(os.sep))


def decide(tree):
    """What happens to a worktree, from its facts: a dict with `path`, `main`, `branch` (None when
    detached), `merged`, `named`, `changes`, `locked`, `marker`, `used_by`, `depended_by`,
    `other_ignored`, `idle_hours`, `idle_limit` and `unread`. Returns (True, the reason it may go) or (False, the
    reason it stays). The first refusal in this order is the reason."""
    path = tree["path"]
    if tree["main"] or any(same_path(path, os.path.join(REPOS_ROOT, name)) for name in SHARED_CHECKOUTS):
        return False, "a main checkout"
    if same_or_under(path, KEEP_ROOT):
        return False, "under E:\\nirender-keep"
    if is_play_set(path):
        return False, "a play set"
    if tree["locked"]:
        return False, "git holds it locked"
    if tree["marker"]:
        return False, "it has " + KEEP_MARKER
    if tree["used_by"]:
        return False, "in use: " + ", ".join(tree["used_by"][:3])
    if tree["depended_by"]:
        return False, "a Cargo.toml names it by path: " + ", ".join(tree["depended_by"][:3])
    if tree["changes"]:
        return False, "%d changed or untracked files" % tree["changes"]
    if tree["other_ignored"]:
        return False, "ignored files that no build makes: " + ", ".join(tree["other_ignored"][:3])
    if tree["unread"]:
        return False, "%d folders could not be read" % tree["unread"]
    if tree["idle_hours"] < tree["idle_limit"]:
        return False, "written %.1f hours ago" % tree["idle_hours"]
    if tree["named"]:
        return True, "named on the command line"
    if tree["branch"] is None:
        return False, "no branch (detached): only by name"
    if not tree["merged"]:
        return False, "not merged"
    return True, "merged"


def is_link_entry(entry):
    """True for a reparse point: a symbolic link or a junction. The walk does not go into it."""
    return bool(entry.stat(follow_symlinks=False).st_file_attributes & FILE_ATTRIBUTE_REPARSE_POINT)


def walk(root):
    """The facts of a tree by a walk that never goes through a link: (bytes, files, the newest
    time of a write, the reparse points, the folders that could not be read)."""
    size = files = 0
    newest = 0.0
    links = []
    unread = 0
    stack = [root]
    while stack:
        folder = stack.pop()
        try:
            with os.scandir(folder) as entries:
                for entry in entries:
                    try:
                        if is_link_entry(entry):
                            links.append(entry.path)
                        elif entry.is_dir(follow_symlinks=False):
                            stack.append(entry.path)
                        else:
                            info = entry.stat(follow_symlinks=False)
                            size += info.st_size
                            files += 1
                            newest = max(newest, info.st_mtime)
                    except OSError:
                        unread += 1
        except OSError:
            unread += 1
    return size, files, newest, links, unread


def remove_link(path):
    """Removes a reparse point as a link: its target stays. A folder link goes with `rmdir`, which
    with no switch only removes an empty folder or a link, and a file link with os.remove."""
    if os.lstat(path).st_file_attributes & stat.FILE_ATTRIBUTE_DIRECTORY:
        result = subprocess.run(["cmd", "/c", "rmdir", path], capture_output=True, text=True)
        if result.returncode != 0:
            raise OSError("rmdir %s: %s" % (path, result.stderr.strip()))
    else:
        os.remove(path)
    if os.path.lexists(path):
        raise OSError("the link %s is still there" % path)


def fingerprint(root):
    """The number and the size of the files of a folder, for the check of a copy."""
    size, files, _, _, unread = walk(root)
    return files, size, unread


def kept_copy(name, source, keep_root):
    """The folder under `keep_root`\\runs-* that holds this run with the same number and size of
    files, or None."""
    if not os.path.isdir(keep_root):
        return None
    wanted = fingerprint(source)
    for folder in sorted(os.listdir(keep_root)):
        if folder.startswith("runs-"):
            candidate = os.path.join(keep_root, folder, name)
            if os.path.isdir(candidate) and fingerprint(candidate) == wanted:
                return candidate
    return None


def real_results(tree_path):
    """The folder `results` of a tree when it is a real folder, else None (none, or a link)."""
    path = os.path.join(tree_path, "results")
    if not os.path.isdir(path) or os.lstat(path).st_file_attributes & FILE_ATTRIBUTE_REPARSE_POINT:
        return None
    return path


def copy_results(results, keep_root, date):
    """Copies each entry of a real results folder that is not kept yet to runs-<date>. Returns what
    it copied. Raises OSError when a copy does not agree: the caller then leaves the tree."""
    copied = []
    for name in sorted(os.listdir(results)):
        source = os.path.join(results, name)
        if not os.path.isdir(source):
            target = os.path.join(keep_root, "runs-" + date, "loose-" + os.path.basename(os.path.dirname(results)), name)
            os.makedirs(os.path.dirname(target), exist_ok=True)
            shutil.copy2(source, target)
            if os.path.getsize(source) != os.path.getsize(target):
                raise OSError("the copy of %s differs" % source)
            copied.append(target)
            continue
        if kept_copy(name, source, keep_root):
            continue
        target = os.path.join(keep_root, "runs-" + date, name)
        if os.path.exists(target):
            raise OSError("%s exists with other files than %s" % (target, source))
        shutil.copytree(source, target, symlinks=True)
        if fingerprint(source) != fingerprint(target):
            raise OSError("the copy of %s differs: %s against %s" % (source, fingerprint(source), fingerprint(target)))
        copied.append(target)
    return copied


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


# ---- processes


class PROCESS_BASIC_INFORMATION(ctypes.Structure):
    _fields_ = [("reserved1", ctypes.c_void_p), ("peb", ctypes.c_void_p), ("reserved2", ctypes.c_void_p * 2), ("pid", ctypes.c_void_p), ("reserved3", ctypes.c_void_p)]


def current_folder(pid):
    """The current folder of a process by its process block, or None when it cannot be read. For a
    32-bit process (the game, a shell) the 32-bit block has it."""
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    ntdll = ctypes.WinDLL("ntdll")
    kernel.OpenProcess.restype = wt.HANDLE
    kernel.OpenProcess.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
    kernel.ReadProcessMemory.argtypes = [wt.HANDLE, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_size_t, ctypes.POINTER(ctypes.c_size_t)]
    kernel.CloseHandle.argtypes = [wt.HANDLE]
    ntdll.NtQueryInformationProcess.argtypes = [wt.HANDLE, ctypes.c_int, ctypes.c_void_p, wt.ULONG, ctypes.POINTER(wt.ULONG)]
    # PROCESS_QUERY_INFORMATION | PROCESS_VM_READ
    handle = kernel.OpenProcess(0x400 | 0x10, False, pid)
    if not handle:
        return None

    def read(address, size):
        buffer = ctypes.create_string_buffer(size)
        done = ctypes.c_size_t()
        if not kernel.ReadProcessMemory(handle, address, buffer, size, ctypes.byref(done)) or done.value != size:
            raise OSError("no read")
        return buffer.raw

    try:
        wow64 = ctypes.c_void_p()
        got = wt.ULONG()
        # ProcessWow64Information: the address of the 32-bit block.
        if ntdll.NtQueryInformationProcess(handle, 26, ctypes.byref(wow64), ctypes.sizeof(wow64), ctypes.byref(got)) != 0:
            return None
        if wow64.value:
            parameters = int.from_bytes(read(wow64.value + 0x10, 4), "little")
            length = int.from_bytes(read(parameters + 0x24, 2), "little")
            text = int.from_bytes(read(parameters + 0x28, 4), "little")
        else:
            basic = PROCESS_BASIC_INFORMATION()
            if ntdll.NtQueryInformationProcess(handle, 0, ctypes.byref(basic), ctypes.sizeof(basic), ctypes.byref(got)) != 0:
                return None
            parameters = int.from_bytes(read(basic.peb + 0x20, 8), "little")
            length = int.from_bytes(read(parameters + 0x38, 2), "little")
            text = int.from_bytes(read(parameters + 0x40, 8), "little")
        return read(text, length).decode("utf-16-le", errors="replace")
    except OSError:
        return None
    finally:
        kernel.CloseHandle(handle)


def processes():
    """Each process with its program, command line and current folder: dicts, and how many
    processes gave no current folder."""
    command = "Get-CimInstance Win32_Process | Select-Object ProcessId,Name,ExecutablePath,CommandLine | ConvertTo-Json -Compress"
    result = subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", command], capture_output=True, text=True, encoding="utf-8", errors="replace")
    found = []
    unread = 0
    for row in json.loads(result.stdout):
        folder = current_folder(row["ProcessId"])
        unread += folder is None
        found.append({"pid": row["ProcessId"], "name": row["Name"] or "", "exe": row["ExecutablePath"] or "", "command": row["CommandLine"] or "", "cwd": folder or ""})
    return found, unread


def users_of(tree_path, running):
    """The processes that use a tree, by name and id."""
    users = []
    for process in running:
        if process["pid"] == os.getpid():
            continue
        slashes = os.path.normcase(os.path.abspath(tree_path)).replace("\\", "/")
        shell_form = "/" + slashes[0] + slashes[2:]
        command = os.path.normcase(process["command"]).replace("\\", "/")
        in_command = re.search("(?:" + re.escape(slashes) + "|" + re.escape(shell_form) + r")(?=[/\"' ]|$)", command) is not None
        if in_command or (process["exe"] and same_or_under(process["exe"], tree_path)) or (process["cwd"] and same_or_under(process["cwd"], tree_path)):
            users.append("%s %d" % (process["name"], process["pid"]))
    return users


# ---- git


def git(tree, *args):
    return subprocess.run(["git", "-C", tree, *args], capture_output=True, text=True, encoding="utf-8", errors="replace")


def worktrees_of(repository):
    """The worktrees of a repository: dicts with `path`, `branch`, `head`, `locked`."""
    trees = []
    current = None
    for line in git(repository, "worktree", "list", "--porcelain").stdout.splitlines():
        if line.startswith("worktree "):
            # Git lists its main worktree first.
            current = {"path": os.path.abspath(line[9:]), "main": not trees, "branch": None, "head": "", "locked": False, "prunable": False}
            trees.append(current)
        elif line.startswith("HEAD "):
            current["head"] = line[5:]
        elif line.startswith("branch "):
            current["branch"] = line[7:].removeprefix("refs/heads/")
        elif line.startswith("locked"):
            current["locked"] = True
        elif line.startswith("prunable"):
            current["prunable"] = True
    return trees


def facts_of(tree, target, repository, running, dependents, named, idle_limit, now):
    """All facts of a worktree for `decide`."""
    path = tree["path"]
    size, files, newest, links, unread = walk(path)
    head = tree["head"]
    merged = bool(tree["branch"]) and git(repository, "merge-base", "--is-ancestor", head, target).returncode == 0
    status = git(path, "status", "--porcelain", "--ignored").stdout.splitlines()
    ignored = [row[3:] for row in status if row.startswith("!! ")]
    results = real_results(path)
    return {
        **tree,
        "target": target,
        "merged": merged,
        "named": any(same_path(path, name) for name in named),
        "changes": len(status) - len(ignored),
        "other_ignored": [name for name in ignored if not BUILD_OUTPUTS.fullmatch(name)],
        "marker": os.path.exists(os.path.join(path, KEEP_MARKER)),
        "used_by": users_of(path, running),
        "depended_by": sorted(os.path.basename(other) for other, folders in dependents.items() if not same_or_under(other, path) and any(same_or_under(folder, path) for folder in folders)),
        "idle_hours": (now - newest) / 3600.0 if newest else 1e9,
        "idle_limit": idle_limit,
        "unread": unread,
        "size": size,
        "files": files,
        "links": links,
        "results": results,
        "result_runs": len(os.listdir(results)) if results else 0,
    }


def games_in(running):
    return sorted({"%s %d" % (p["name"], p["pid"]) for p in running if p["name"].lower() in GAME_PROGRAMS})


def log_line(text, apply):
    print(text, flush=True)
    if apply:
        with open(LOG, "a", encoding="utf-8") as file:
            file.write(time.strftime("%Y-%m-%d %H:%M:%S ") + text + "\n")


def gigabytes(size):
    return "%.1f GB" % (size / 1e9)


def remove(tree, repository, date):
    """The three steps of removal. Returns what is left there, as (path, bytes), or raises OSError."""
    path = tree["path"]
    if tree["results"]:
        for copied in copy_results(tree["results"], KEEP_ROOT, date):
            log_line("  copied to %s" % copied, True)
    for link in tree["links"]:
        target = os.path.realpath(link)
        remove_link(link)
        log_line("  link removed: %s (its target %s stays: %s)" % (link, target, os.path.exists(target)), True)
    result = git(repository, "worktree", "remove", path)
    if result.returncode != 0:
        raise OSError("git worktree remove: " + result.stderr.strip())
    git(repository, "worktree", "prune")
    if os.path.lexists(path):
        return path, walk(path)[0]
    return None


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
    with ThreadPoolExecutor(8) as pool:
        rows = list(pool.map(lambda job: facts_of(job[0], job[1], job[2], running, dependents, named, args.idle_hours, now), jobs))
    go, stay = [], []
    for row, job in zip(rows, jobs):
        ok, reason = decide(row)
        row["reason"], row["repository"] = reason, job[2]
        (go if ok else stay).append(row)
    print("%d processes, the current folder of %d could not be read" % (len(running), unread_processes))
    if games:
        print("a game or a helper runs: " + ", ".join(games))
    print("\nMAY GO (%d, %s):" % (len(go), gigabytes(sum(row["size"] for row in go))))
    for row in sorted(go, key=lambda r: -r["size"]):
        print("  %9s  %s  [%s] %s; %d links; results: %s; idle %.0f h" % (gigabytes(row["size"]), row["path"], row["branch"] or "detached", row["reason"], len(row["links"]), "%d entries to keep" % row["result_runs"] if row["results"] else "none", min(row["idle_hours"], 9999)))
        for link in row["links"]:
            print("             link: %s -> %s" % (link, os.path.realpath(link)))
    print("\nSTAYS (%d, %s):" % (len(stay), gigabytes(sum(row["size"] for row in stay))))
    for row in sorted(stay, key=lambda r: -r["size"]):
        print("  %9s  %s  [%s] %s" % (gigabytes(row["size"]), row["path"], row["branch"] or "detached", row["reason"]))
    folders = plain_folders()
    print("\nPLAIN FOLDERS in %s, not touched (%s), the largest 15:" % (PLAIN_FOLDERS, gigabytes(sum(size for size, _ in folders))))
    for size, path in folders[:15]:
        print("  %9s  %s" % (gigabytes(size), path))
    if not args.apply:
        print("\nA dry run: nothing changed. --apply acts.")
        return 0
    left = []
    failed = 0
    for row in go:
        log_line("removing %s [%s] %s, %s" % (row["path"], row["branch"] or "detached", row["reason"], gigabytes(row["size"])), True)
        try:
            now_running, _ = processes()
            busy = games_in(now_running) + users_of(row["path"], now_running)
            if busy:
                raise OSError("in use now: " + ", ".join(busy[:3]))
            remains = remove(row, row["repository"], date)
        except OSError as error:
            failed += 1
            log_line("  LEFT AS IT IS: %s" % error, True)
            continue
        log_line("  removed" if remains is None else "  git left the folder: %s" % gigabytes(remains[1]), True)
        if remains:
            left.append(remains)
    if left:
        print("\nFOLDERS THAT REMAIN, for the owner (this tool makes no recursive delete):")
        for path, size in left:
            print("  %9s  %s" % (gigabytes(size), path))
    return 1 if failed else 0


def self_test():
    base = {"path": "C:\\repos\\nirender-plugin-x", "main": False, "branch": "fix/x", "merged": True, "named": False, "changes": 0, "other_ignored": [], "locked": False, "marker": False, "used_by": [], "depended_by": [], "idle_hours": 48.0, "idle_limit": 6.0, "unread": 0}
    assert decide(base) == (True, "merged")
    for change, word in [
        ({"merged": False}, "not merged"),
        ({"branch": None}, "only by name"),
        ({"changes": 3}, "3 changed"),
        ({"other_ignored": ["captures/"]}, "captures/"),
        ({"path": os.path.join(REPOS_ROOT, "MGE-XE-G7-train")}, "main checkout"),
        ({"locked": True}, "locked"),
        ({"marker": True}, KEEP_MARKER),
        ({"used_by": ["cargo.exe 7"]}, "cargo.exe 7"),
        ({"depended_by": ["MGE-XE-G7-x"]}, "by path"),
        ({"idle_hours": 1.0}, "hours ago"),
        ({"unread": 2}, "could not be read"),
        ({"main": True}, "main checkout"),
        ({"path": os.path.join(REPOS_ROOT, "MGE-XE-G7-render-link")}, "main checkout"),
        ({"path": "C:\\tmp\\play185\\nirender-plugin"}, "play set"),
        ({"path": "E:\\nirender-keep\\x"}, "nirender-keep"),
    ]:
        ok, reason = decide({**base, **change})
        assert not ok and word in reason, (change, reason)
    # A name passes the tests of the branch, and no other test.
    assert decide({**base, "merged": False, "named": True})[0]
    assert decide({**base, "branch": None, "named": True})[0]
    for change in ({"changes": 1}, {"used_by": ["x 1"]}, {"marker": True}, {"path": "C:\\tmp\\play185\\x"}, {"main": True}, {"idle_hours": 0.5}, {"depended_by": ["y"]}):
        assert not decide({**base, "named": True, **change})[0], change
    for name in ("target/", "bin/", "results/", "tools/out/", "tools/__pycache__/", "tools/ports/__pycache__/"):
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

    with tempfile.TemporaryDirectory() as temp:
        # A real junction: the walk does not go through it, and its removal leaves its target.
        outside = os.path.join(temp, "outside")
        os.makedirs(outside)
        open(os.path.join(outside, "precious.txt"), "w").write("x" * 1000)
        tree = os.path.join(temp, "tree")
        os.makedirs(os.path.join(tree, "src"))
        open(os.path.join(tree, "src", "a.txt"), "w").write("abc")
        link = os.path.join(tree, "results")
        result = subprocess.run(["cmd", "/c", "mklink", "/J", link, outside], capture_output=True, text=True)
        assert result.returncode == 0, result.stderr
        size, files, _, links, unread = walk(tree)
        assert (size, files, links, unread) == (3, 1, [link], 0), (size, files, links, unread)
        assert real_results(tree) is None
        remove_link(link)
        assert not os.path.lexists(link) and os.path.getsize(os.path.join(outside, "precious.txt")) == 1000

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
        copied = copy_results(results, keep, "today")
        assert sorted(os.path.basename(c) for c in copied) == ["loose.txt", "run1"], copied
        assert open(os.path.join(keep, "runs-today", "run1", "x.log")).read() == "12345"
        assert not os.path.exists(os.path.join(keep, "runs-today", "run2"))
        # A folder of that name with other files: a refusal, not a mix.
        os.makedirs(os.path.join(results, "run3"))
        open(os.path.join(results, "run3", "z.log"), "w").write("abc")
        os.makedirs(os.path.join(keep, "runs-today", "run3"))
        open(os.path.join(keep, "runs-today", "run3", "other.log"), "w").write("d")
        try:
            copy_results(results, keep, "today")
            raise AssertionError("no refusal")
        except OSError as error:
            assert "other files" in str(error)

        # The path of a Cargo.toml, here in a member of the workspace.
        member = os.path.join(temp, "other", "member")
        os.makedirs(member)
        open(os.path.join(member, "Cargo.toml"), "w").write('a = { path = "../../tree/host" }\nb = { path = "../inside" }\n')
        assert path_dependencies(os.path.join(temp, "other")) == {os.path.join(temp, "tree", "host")}
    print("clean_merged_worktrees self-test: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
