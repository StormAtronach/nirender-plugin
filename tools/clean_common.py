"""The parts that the two cleaning tools share: the walk that never goes through a link, a removal of the
same kind, the processes, the log, the citations and the expiry of the data rule (D71). No entry point:
`clean_merged_worktrees.py` and `clean_results.py` are the tools.

A run or a dump is evidence when a record, a plan page or a commit names it. A run stays whole only on the
keep list, KEEP_LIST (D152). The rest expires:

- a run folder: pictures and captures at PICTURE_DAYS; text and dumps at TEXT_DAYS, or never for a cited run;
- a dump outside a run: at TEXT_DAYS.

The keep list has one line for each kept path, its three fields split by tabs: the full path, the reason and
the date the line was written (YYYY-MM-DD). The reason starts with `open: <id>` (the evidence of an open item),
`tool: <name>` (a tool reads it) or `owner` (the owner's word). A run stays whole when its folder is a path of
the list or lies in one; a set of mgebuilds stays too. Other paths (a file, a folder of E:\\nirender-keep) are no
problem: the tools do not remove them. These lines are problems, and no removal runs while one is there:
- a line without a full path, a reason or a date;
- a path too wide to keep (a drive, a place of runs, dumps, sets or checkouts, or a folder above one): such a line
  would stop the cleaning of a whole place;
- a path that is a link or goes through one: the tools never follow a link, so the line would keep nothing.
A line that starts with `#` is a note. A run on the list in a side worktree stays with its tree:
clean_merged_worktrees.py does not remove that tree, as a copy of the run would not be on the list.
"""
import bisect
import ctypes
import ctypes.wintypes as wt
import glob
import json
import os
import re
import stat
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import common  # noqa: E402

# The folder of the repositories: the one of the main checkout of this repository, also for a tool of a worktree.
REPOS_ROOT = os.path.dirname(common.main_checkout()) if common.main_checkout() else common.SIBLINGS
KEEP_ROOT = "E:\\nirender-keep"
LOG = os.path.join(KEEP_ROOT, "clean-worktrees.log")
# The docs that cite the runs: those of the docs main checkout (common.find_docs without the key "docs"). A run cited
# only on docs main must stay when the key names a branch tree, and never a stale docs worktree.
DOCS = os.path.join(common.find_docs(common.main_checkout(), None), "docs")
# The notes of the sessions also name runs.
MEMORY = os.path.join(os.path.expanduser("~"), ".claude", "projects", "c--Users-ivan--source-repos-MWSE-MWSE", "memory")
COMMIT_REPOS = [os.path.join(REPOS_ROOT, name) for name in ("nirender-plugin", "MGE-XE-G7-render-link", os.path.join("MWSE", "MWSE"), "moreFPS")]
# The repositories whose worktrees can go: a checkout of each, and the branch that their work merges into.
REPOSITORIES = [("nirender-plugin", "master"), ("MGE-XE-G7-render-link", "feature/wgpu-renderer")]
# The worktrees that lose only their build output.
OUTPUT_ONLY_REPOSITORIES = [os.path.join("MWSE", "MWSE")]
# local.bat of this checkout names the current play set.
PLUGIN_CHECKOUT = os.path.join(REPOS_ROOT, "nirender-plugin")
SETS = os.path.join(KEEP_ROOT, "mgebuilds")
# The WER crash dumps of the runs, one folder for each run under its run folder's name (D94; the
# harness's RUN_DUMPS). The run folder holds the pointer file DUMPS_FILE: a line for each dump,
# its path first, then its bytes and its source, with tabs.
KEPT_DUMPS = os.path.join(KEEP_ROOT, "dumps")
DUMPS_FILE = "dumps.txt"
# The runs that stay whole, each with its reason (D152).
KEEP_LIST = os.path.join(KEEP_ROOT, "keep-runs.txt")
KEEP_REASON = re.compile(r"^(?:(?:open|tool):\s*\S.*|owner\b.*)$")
KEEP_DATE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
# The places of runs, dumps, sets and checkouts. A keep-list line on one of them, or above one, is a problem.
WIDE_PLACES = (KEEP_ROOT, "E:\\nirender-results", SETS, KEPT_DUMPS, "C:\\tmp\\mwdumps", REPOS_ROOT)
GAME_PROGRAMS = {"morrowind.exe", "numidium.exe", "nirender-host.exe"}
FILE_ATTRIBUTE_REPARSE_POINT = 0x400

# The check cards of the test train name runs too.
CARDS = "C:\\tmp\\coord\\train\\cards"
RUN = re.compile(r"^(\d{8}-\d{6})-.+$")
STAMP = re.compile(r"\d{8}-\d{6}")
# The short form of a run's name, `-HHMMSS-`, with no word character before it. The label may differ.
SHORT_CITATION = re.compile(r"(?<!\w)-\d{6}(?=-)")
DUMP_NUMBER = re.compile(r"(?i)(?:dump|pid)\W{0,3}(\d+)")
EVIDENCE_TEXT = (".md", ".tsv", ".txt", ".py", ".sh", ".lua", ".json", ".csv", ".log", ".toml", ".cdb", ".ps1")
# The docs and the commit messages alone hold more. Less says that a part was not read.
LEAST_EVIDENCE = 10_000_000
LEAST_LABEL = 4
# What a trim of a run takes: pictures, captures and film. The rest stays to the end of the run.
HEAVY_FILES = (".png", ".tga", ".bmp", ".jpg", ".jpeg", ".gif", ".nircap", ".mp4", ".mkv", ".avi", ".webm")
DUMP_FILES = (".dmp", ".mdmp")
GENERIC_DUMPS = {"mwse_minidump", "numidium", "morrowind"}
BUSY_DAYS = 1 / 24
PICTURE_DAYS = 7
TEXT_DAYS = 30
DUMP_DAYS = TEXT_DAYS
SET_DAYS = 30


def same_path(path, other):
    return os.path.normcase(os.path.abspath(path)) == os.path.normcase(os.path.abspath(other))


def same_or_under(path, root):
    """True when `path` is `root` or a path below it, on Windows' terms."""
    path, root = os.path.normcase(os.path.abspath(path)), os.path.normcase(os.path.abspath(root))
    return path == root or path.startswith(root.rstrip("\\/") + os.sep)


def is_link_entry(entry):
    """True for a reparse point: a symbolic link or a junction. The walk does not go into it."""
    return bool(entry.stat(follow_symlinks=False).st_file_attributes & FILE_ATTRIBUTE_REPARSE_POINT)


def is_link(path):
    return bool(os.lstat(path).st_file_attributes & FILE_ATTRIBUTE_REPARSE_POINT)


def long_path(path):
    """The form of a path that passes MAX_PATH: a target folder holds deeper paths than that."""
    return "\\\\?\\" + os.path.abspath(path)


def short_path(path):
    return path[4:] if path.startswith("\\\\?\\") else path


def walk(root):
    """The facts of a tree by a walk that never goes through a link: (bytes, files, the newest
    time of a write, the reparse points, the folders that could not be read)."""
    size = files = 0
    newest = 0.0
    links = []
    unread = 0
    stack = [long_path(root)]
    while stack:
        folder = stack.pop()
        try:
            with os.scandir(folder) as entries:
                for entry in entries:
                    try:
                        if is_link_entry(entry):
                            links.append(short_path(entry.path))
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


def files_in(root):
    """Each file of a tree, never through a link: (path, bytes, the time of its write)."""
    found = []
    stack = [long_path(root)]
    while stack:
        with os.scandir(stack.pop()) as entries:
            for entry in entries:
                if is_link_entry(entry):
                    continue
                if entry.is_dir(follow_symlinks=False):
                    stack.append(entry.path)
                else:
                    info = entry.stat(follow_symlinks=False)
                    found.append((short_path(entry.path), info.st_size, info.st_mtime))
    return found


def remove_link(path):
    """Removes a reparse point as a link: its target stays. os.rmdir takes a folder link and never goes
    through it, and os.remove a file link."""
    if os.lstat(long_path(path)).st_file_attributes & stat.FILE_ATTRIBUTE_DIRECTORY:
        os.rmdir(long_path(path))
    else:
        os.remove(long_path(path))
    if os.path.lexists(long_path(path)):
        raise OSError("the link %s is still there" % path)


def remove_file(path):
    os.chmod(long_path(path), stat.S_IWRITE)
    os.remove(long_path(path))


def remove_tree(root):
    """Removes a real folder: each file by itself, and each reparse point as a link, so that nothing
    outside the folder changes. Refuses a folder that is itself a link. Returns the links it removed
    (link, target) and the failures (path, error). A failure stops nothing, and what failed stays."""
    if is_link(root):
        raise OSError("%s is a link, and it stays" % root)
    links, failures = [], []

    def empty(folder):
        with os.scandir(folder) as entries:
            found = list(entries)
        for entry in found:
            try:
                if is_link_entry(entry):
                    link = short_path(entry.path)
                    target = os.path.realpath(link)
                    remove_link(link)
                    links.append((link, target))
                elif entry.is_dir(follow_symlinks=False):
                    empty(entry.path)
                    os.rmdir(entry.path)
                else:
                    os.chmod(entry.path, stat.S_IWRITE)
                    os.remove(entry.path)
            except OSError as error:
                failures.append((short_path(entry.path), str(error)))

    try:
        empty(long_path(root))
        os.rmdir(long_path(root))
    except OSError as error:
        failures.append((root, str(error)))
    return links, failures


def log_line(text, apply):
    print(text, flush=True)
    if apply:
        with open(LOG, "a", encoding="utf-8") as file:
            file.write(time.strftime("%Y-%m-%d %H:%M:%S ") + text + "\n")


def gigabytes(size):
    return "%.1f GB" % (size / 1e9)


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
    # The creation time as a number (Windows file time, UTC), 0 when Windows gives none.
    command = ("Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,ExecutablePath,CommandLine,"
               "@{n='Created';e={if ($_.CreationDate) { $_.CreationDate.ToFileTimeUtc() } else { 0 }}} | ConvertTo-Json -Compress")
    result = subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", command], capture_output=True, text=True, encoding="utf-8", errors="replace")
    found = []
    unread = 0
    for row in json.loads(result.stdout):
        folder = current_folder(row["ProcessId"])
        unread += folder is None
        found.append({"pid": row["ProcessId"], "parent": row["ParentProcessId"], "created": row["Created"] or 0, "name": row["Name"] or "",
                      "exe": row["ExecutablePath"] or "", "command": row["CommandLine"] or "", "cwd": folder or ""})
    return found, unread


def own_line(running, pid):
    """This process and those that started it: the shell that runs this tool does not use a tree. Windows gives the id
    of an ended process to a new process, so a parent id can name a stranger: the line ends at a process that was
    created after its child, or whose time is not known."""
    by_id = {process["pid"]: process for process in running}
    line = {pid}
    while pid in by_id:
        child = by_id[pid]
        parent = by_id.get(child.get("parent"))
        if parent is None or parent["pid"] in line:
            break
        if not (parent.get("created") and child.get("created") and parent["created"] <= child["created"]):
            break
        line.add(parent["pid"])
        pid = parent["pid"]
    return line


def users_of(tree_path, running, skip=None):
    """The processes that use a tree, by name and id. `skip` holds the process ids to leave out: by default this process.
    The command line of a process that started this tool names the tree when it gives the tree's path (--name), so for
    those processes only the program and the current folder count. A shell whose current folder is in the tree still
    uses it."""
    skip = {os.getpid()} if skip is None else skip
    starters = own_line(running, os.getpid())
    users = []
    for process in running:
        if process["pid"] in skip:
            continue
        slashes = os.path.normcase(os.path.abspath(tree_path)).replace("\\", "/")
        shell_form = "/" + slashes[0] + slashes[2:]
        command = os.path.normcase(process["command"]).replace("\\", "/")
        in_command = process["pid"] not in starters and re.search("(?:" + re.escape(slashes) + "|" + re.escape(shell_form) + r")(?=[/\"' ]|$)", command) is not None
        if in_command or (process["exe"] and same_or_under(process["exe"], tree_path)) or (process["cwd"] and same_or_under(process["cwd"], tree_path)):
            users.append("%s %d" % (process["name"], process["pid"]))
    return users


def games_in(running):
    return sorted({"%s %d" % (p["name"], p["pid"]) for p in running if p["name"].lower() in GAME_PROGRAMS})


# ---- git


def git(tree, *args):
    # A hook of git sets GIT_DIR, which turns each command to the repository of that hook.
    clean = {name: value for name, value in os.environ.items() if not name.startswith("GIT_")}
    return subprocess.run(["git", "-C", tree, *args], capture_output=True, text=True, encoding="utf-8", errors="replace", env=clean)


def worktrees_of(repository):
    """The worktrees of a repository: dicts with `path`, `branch`, `head`, `locked` and `lock_reason`
    (the words of `git worktree lock --reason`, empty when none were given)."""
    trees = []
    current = None
    for line in git(repository, "worktree", "list", "--porcelain").stdout.splitlines():
        if line.startswith("worktree "):
            # Git lists its main worktree first.
            current = {"path": os.path.abspath(line[9:]), "main": not trees, "branch": None, "head": "", "locked": False, "lock_reason": "", "prunable": False}
            trees.append(current)
        elif line.startswith("HEAD "):
            current["head"] = line[5:]
        elif line.startswith("branch "):
            current["branch"] = line[7:].removeprefix("refs/heads/")
        elif line.startswith("locked"):
            current["locked"] = True
            current["lock_reason"] = line[6:].strip()
        elif line.startswith("prunable"):
            current["prunable"] = True
    return trees


def project_trees():
    """Each worktree of the project's repositories that exists: (path, is it only for its build output)."""
    trees = {}
    for names, output_only in (([name for name, _ in REPOSITORIES], False), (OUTPUT_ONLY_REPOSITORIES, True)):
        for name in names:
            for tree in worktrees_of(os.path.join(REPOS_ROOT, name)):
                if os.path.isdir(tree["path"]):
                    trees[os.path.normcase(tree["path"])] = (tree["path"], output_only)
    return sorted(trees.values())


# ---- the play sets


def set_number(name):
    """The number of a set or of a play set's folder: f195 and play195 give 195, and any other name None."""
    match = re.fullmatch(r"(?:f|play[-_]?)(\d+)", name, re.I)
    return int(match.group(1)) if match else None


def current_set(checkout=None):
    """The name of the set that local.bat names, or None."""
    try:
        with open(os.path.join(checkout or PLUGIN_CHECKOUT, "local.bat"), encoding="utf-8", errors="replace") as file:
            found = re.search(r'^set "MGE_BUILD=([^"\r\n]*)"', file.read(), re.MULTILINE)
    except OSError:
        return None
    return os.path.basename(found.group(1).rstrip("\\/")) if found else None


def kept_set_numbers(current, names):
    """The numbers of the current play set and of the one before it among `names`, or an empty set
    when local.bat names no numbered set: then no play set is old."""
    number = set_number(current) if current else None
    if number is None:
        return set()
    before = [found for found in map(set_number, names) if found is not None and found < number]
    return {number, max(before)} if before else {number}


def kept_set_names(current, names):
    """The names of the sets that stay as the current one and the one before. A current set with no number stays by
    its name, and the highest number counts as the one before."""
    number = set_number(current)
    numbered = [(found, name) for found, name in ((set_number(name), name) for name in names) if found is not None and (number is None or found < number)]
    return {current, max(numbered)[1]} if numbered else {current}


def old_play_set(path, kept):
    """True for a tree of a numbered play set older than the two that stay: C:\\tmp\\play185\\x. The rolling trees of
    C:\\tmp\\playset have no number and stay."""
    if not kept:
        return False
    for part in os.path.abspath(path).split(os.sep):
        number = set_number(part) if part.lower().startswith("play") else None
        if number is not None:
            return number < min(kept)
    return False


# ---- the citations


def evidence_folders():
    """The folders whose text names runs. Not the copies of runs, not the sets and not the runs' dumps: a run's log
    names its own folder, and that is no citation. A folder that is missing shows in the load."""
    folders = [DOCS, MEMORY, CARDS]
    try:
        with os.scandir(KEEP_ROOT) as entries:
            folders += sorted(entry.path for entry in entries if entry.is_dir(follow_symlinks=False) and not is_link_entry(entry) and not entry.name.startswith("runs-") and entry.path not in (SETS, KEPT_DUMPS))
    except OSError:
        folders.append(KEEP_ROOT)
    return folders


def keep_key(path):
    """A path of the keep list in one spelling: absolute, Windows' case and separators."""
    return os.path.normcase(os.path.normpath(path))


def too_wide_to_keep(path):
    """Why a path of the keep list would keep the runs of a whole place, or None. Such a line would stop the cleaning of
    that place without a word: a drive, a place of runs, dumps or sets or a folder above one, a `results` folder, a
    `runs-*` or `full-dumps-*` copy, or a folder that holds a `results` folder (a checkout)."""
    path = os.path.normpath(path)
    name = os.path.basename(path).lower()
    if os.path.dirname(path) == path:
        return "a drive"
    for place in WIDE_PLACES:
        if same_or_under(place, path):
            return "the place %s or a folder above it" % place
    if name == "results" or name.startswith(("runs-", "full-dumps-")):
        return "a folder of runs"
    if os.path.isdir(os.path.join(path, "results")):
        return "a folder that holds a results folder"
    return None


def through_link(path):
    """The first link (a junction or a symbolic link) on a path, from its drive down, or None. The tools never go
    through a link, so a line that names one or a path inside one keeps nothing."""
    path = os.path.normpath(path)
    parts = []
    while os.path.dirname(path) != path:
        parts.append(path)
        path = os.path.dirname(path)
    for part in reversed(parts):
        try:
            if is_link(part):
                return part
        except OSError:
            return None
    return None


def read_keep_list(path=KEEP_LIST):
    """The paths on the keep list, in `keep_key` spelling, each with its line ("line N: text"), and the list's problems:
    a line without a full path, a reason or a date, a path too wide to keep or through a link, or a file that cannot
    be read. No file is an empty list. A byte order mark is read as none."""
    try:
        with open(long_path(path), encoding="utf-8-sig", errors="replace") as file:
            lines = file.read().splitlines()
    except FileNotFoundError:
        return {}, []
    except OSError as error:
        return {}, ["the keep list cannot be read: %s (%s)" % (path, error)]
    kept, problems = {}, []
    for number, line in enumerate(lines, 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        fields = [field.strip() for field in line.split("\t")]
        kept_path = fields[0]
        reason = fields[1] if len(fields) > 1 else ""
        date = fields[2] if len(fields) > 2 else ""
        where = "the keep list, line %d" % number
        if len(fields) > 3:
            problems.append("%s: more than three fields (path, reason, date): %s" % (where, line))
        elif not os.path.isabs(kept_path) or not os.path.splitdrive(kept_path)[0]:
            problems.append("%s: no full path first: %s" % (where, line))
        elif not KEEP_REASON.match(reason):
            problems.append("%s: no reason (open: <id> | tool: <name> | owner) second: %s" % (where, line))
        elif not KEEP_DATE.match(date):
            problems.append("%s: no date (YYYY-MM-DD) third: %s" % (where, line))
        elif too_wide_to_keep(kept_path):
            problems.append("%s: the path is too wide to keep (%s): %s" % (where, too_wide_to_keep(kept_path), line))
        elif through_link(kept_path):
            problems.append("%s: the path goes through a link, %s, which the tools never follow; name the real path, %s: %s" % (where, through_link(kept_path), os.path.realpath(kept_path), line))
        else:
            kept[keep_key(kept_path)] = "line %d: %s" % (number, line.replace("\t", " | "))
    return kept, problems


class Evidence:
    """What the docs, the notes, the accepted baselines and the commit messages name, and the runs of the keep list."""

    def __init__(self, text, problems=(), kept=()):
        self.text = text
        # Why the text may lack a citation, or the keep list a run. A removal must not run on it: every run
        # would read as uncited, or a kept run as not kept.
        self.problems = list(problems)
        # The kept paths, each with its line of the list; a plain set of paths (the tests) gets the path as its line.
        lines = kept if isinstance(kept, dict) else {path: path for path in kept}
        self.kept = {keep_key(path): line for path, line in lines.items()}
        self.stamps = set(STAMP.findall(text))
        self.shorts = set(SHORT_CITATION.findall(text))
        self.dump_numbers = set(DUMP_NUMBER.findall(text))

    @classmethod
    def load(cls, folders=None, names=None, repos=COMMIT_REPOS, least=LEAST_EVIDENCE, keep_list=KEEP_LIST):
        """The text of each folder and each commit message, and the keep list. `names` are folders whose own name
        counts too."""
        folders = evidence_folders() if folders is None else folders
        names = glob.glob(os.path.join(KEEP_ROOT, "baseline-*")) + glob.glob(os.path.join(KEEP_ROOT, "gate-*")) if names is None else names
        # The name of a baseline can hold the set that it needs: baseline-accepted-20261102-f194.
        parts = [os.path.basename(name) for name in names]
        problems = []
        for folder in folders:
            try:
                found = files_in(folder)
            except OSError as error:
                problems.append("a folder cannot be read: %s (%s)" % (folder, error))
                continue
            for path, _, _ in found:
                if not path.lower().endswith(EVIDENCE_TEXT) or os.sep + ".git" + os.sep in path:
                    continue
                try:
                    with open(long_path(path), encoding="utf-8", errors="replace") as file:
                        parts.append(file.read())
                except OSError as error:
                    problems.append("a file cannot be read: %s (%s)" % (path, error))
        for repo in repos:
            log = git(repo, "log", "--all", "--format=%B") if os.path.isdir(repo) else None
            if log is None or log.returncode != 0 or not log.stdout.strip():
                problems.append("no commit messages from %s" % repo)
            else:
                parts.append(log.stdout)
        text = "\n".join(parts)
        if len(text) < least:
            problems.append("only %d characters of text, and at least %d are expected" % (len(text), least))
        kept, keep_problems = read_keep_list(keep_list)
        return cls(text, problems + keep_problems, kept)

    def keeps(self, path):
        """True for a path that the keep list names, or that lies in a folder that it names."""
        return self.kept_line(path) is not None

    def kept_line(self, path):
        """The line of the keep list that keeps a path, or None."""
        return next((line for kept, line in self.kept.items() if same_or_under(path, kept)), None)

    def cites_run(self, name):
        """A full citation counts by its date and time, as a record can shorten the label. A short
        citation counts by its time alone: a false keep costs little, and a false removal loses evidence."""
        return name[:15] in self.stamps or name[8:15] in self.shorts

    def cites_word(self, word):
        """True when the text holds this name as a word of its own: f94 is not in f945 or xf94."""
        return word in self.text and re.search(r"(?<!\w)" + re.escape(word) + r"(?!\w)", self.text) is not None

    def cites_dump(self, path):
        """True when the text names a dump by its file, its name with no extension, or the number
        of its process beside the word dump or pid. The name of a mini dump is the same in each run, and says nothing."""
        stem = os.path.splitext(os.path.basename(path))[0]
        if stem.lower() in GENERIC_DUMPS:
            return False
        if self.cites_word(stem):
            return True
        number = re.search(r"[.-](\d+)$", stem)
        return number is not None and number.group(1) in self.dump_numbers

    def cites_run_with(self, name, files):
        """True when the text names a run, or a dump among its files: (path, bytes, time)."""
        return self.cites_run(name) or any(file[0].lower().endswith(DUMP_FILES) and self.cites_dump(file[0]) for file in files)

    def label_only(self, name):
        """True for an uncited run whose label the text holds: a record that may mean it, with no time to prove it."""
        label = name[16:]
        return len(label) >= LEAST_LABEL and not self.cites_run(name) and self.cites_word(label)


# ---- the expiry


def is_heavy(name):
    """True for a file that a trim takes: a picture, a capture or a film. All else lives as long as its run."""
    return name.lower().endswith(HEAVY_FILES)


def run_age_days(name, now):
    """The age of a run from its folder name, or None for a folder that is no run."""
    match = RUN.match(name)
    if not match:
        return None
    try:
        return (now - time.mktime(time.strptime(match.group(1), "%Y%m%d-%H%M%S"))) / 86400
    except ValueError:
        return None


def run_treatment(age_days, cited, kept=False):
    """What happens to a run folder: "kept", "running" and "young" stay whole, "trim" loses its pictures,
    "expired" goes. A citation keeps the text only: a cited run stays trimmed and never expires."""
    if kept:
        return "kept"
    if age_days < BUSY_DAYS:
        return "running"
    if age_days < PICTURE_DAYS:
        return "young"
    if cited or age_days < TEXT_DAYS:
        return "trim"
    return "expired"


def set_treatment(age_days, cited, kept, used, current_known=True):
    """What happens to a set of mgebuilds: "stay" with the reason, or "go". With no current set known, no set goes."""
    if not current_known:
        return "stay: local.bat names no set that is here"
    if kept:
        return "stay: the current play set or the one before"
    if used:
        return "stay: in use"
    if cited:
        return "stay: cited"
    if age_days < SET_DAYS:
        return "stay: younger than %d days" % SET_DAYS
    return "go"


def self_test_common():
    assert same_or_under("C:\\a\\b\\c", "C:\\a\\b") and not same_or_under("C:\\a\\bc", "C:\\a\\b")
    assert set_number("f195") == 195 and set_number("play185") == 185 and set_number("f91-null") is None and set_number("playset") is None
    assert kept_set_numbers("f195", ["f27", "f104p", "f193", "f195", "vd-a28"]) == {195, 193}
    assert kept_set_numbers("f27", ["f27", "f28"]) == {27} and kept_set_numbers(None, ["f1"]) == set() and kept_set_numbers("vd-a28", ["f1"]) == set()
    assert kept_set_names("f195", ["f27", "f193", "f195", "vd-a"]) == {"f195", "f193"} and kept_set_names("vd-a", ["f27", "f28", "vd-a"]) == {"vd-a", "f28"}
    assert kept_set_names("f27", ["f27"]) == {"f27"} and kept_set_names("vd-a", ["vd-a"]) == {"vd-a"}
    assert old_play_set("C:\\tmp\\play185\\nirender-plugin", {195, 194}) and not old_play_set("C:\\tmp\\play194\\x", {195, 194})
    assert not old_play_set("C:\\tmp\\playset\\x", {195, 194}) and not old_play_set("C:\\tmp\\play185\\x", set()) and not old_play_set("C:\\tmp\\walk\\x", {195})
    assert own_line([{"pid": 5, "parent": 3, "created": 30}, {"pid": 3, "parent": 1, "created": 20}, {"pid": 1, "parent": 0, "created": 10}, {"pid": 9, "parent": 3, "created": 40}], 5) == {5, 3, 1}
    # Windows gives the id of an ended process to a new one: a "parent" created after its child, or with no known
    # time, did not start it, and the line ends before it.
    assert own_line([{"pid": 5, "parent": 3, "created": 30}, {"pid": 3, "parent": 1, "created": 40}], 5) == {5}
    assert own_line([{"pid": 5, "parent": 3, "created": 30}, {"pid": 3, "parent": 1, "created": 0}], 5) == {5}
    assert users_of("C:\\t", [{"pid": 3, "name": "sh", "exe": "", "command": "", "cwd": "C:\\t\\x"}], skip={3}) == []
    # The shell that runs this tool with --name of a tree does not use the tree by that command line (2026-11-09: the
    # Coordinator's Bash tool showed as "in use: bash.exe 42772"). Another shell with the same command line uses it, and
    # the starting shell uses it when its current folder is in the tree.
    me = os.getpid()
    tool_line = 'python tools/clean_merged_worktrees.py --name "C:\\t"'
    starter = {"pid": me + 1, "parent": me + 2, "created": 200, "name": "bash.exe", "exe": "", "command": "bash -c '%s'" % tool_line, "cwd": "C:\\other"}
    this = {"pid": me, "parent": me + 1, "created": 300, "name": "python.exe", "exe": "", "command": tool_line, "cwd": "C:\\other"}
    stranger = dict(starter, pid=me + 3, parent=me + 2)
    assert users_of("C:\\t", [this, starter, stranger]) == ["bash.exe %d" % (me + 3)]
    # The Reviewer's case: the starting shell's own parent has ended, and its id now belongs to a cargo in the tree. That
    # cargo is not of the tool's line, and it uses the tree.
    reused = {"pid": me + 2, "parent": 1, "created": 400, "name": "cargo.exe", "exe": "", "command": "cargo test --manifest-path C:/t/host/Cargo.toml", "cwd": "C:\\other"}
    assert users_of("C:\\t", [this, starter, reused]) == ["cargo.exe %d" % (me + 2)]
    assert users_of("C:\\t", [this, dict(starter, cwd="C:\\t\\src")]) == ["bash.exe %d" % (me + 1)]
    assert set_treatment(40, False, False, False) == "go" and "younger" in set_treatment(10, False, False, False)
    for kept, used, cited in ((True, False, False), (False, True, False), (False, False, True)):
        assert "stay" in set_treatment(400, cited, kept, used)
    assert "names no set" in set_treatment(400, False, False, False, False)
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp:
        with open(os.path.join(temp, "local.bat"), "w", encoding="ascii") as file:
            file.write('set "OTHER=1"\nset "MGE_BUILD=E:\\nirender-keep\\mgebuilds\\f195"\n')
        assert current_set(temp) == "f195" and current_set(os.path.join(temp, "none")) is None

        # Each way for the text to lack a citation shows, so that no removal runs on it.
        docs = os.path.join(temp, "docs")
        os.makedirs(docs)
        with open(os.path.join(docs, "a.md"), "w", encoding="ascii") as file:
            file.write("see 20261103-010838-h14-w2\n" * 10)
        repo = os.path.join(temp, "repo")
        assert git(temp, "init", "-q", "-b", "master", repo).returncode == 0
        assert git(repo, "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "commit", "-q", "--allow-empty", "-m", "a commit names 20261103-020202-x").returncode == 0
        # The tests never read the real keep list: no file is an empty list.
        no_list = os.path.join(temp, "no-keep-runs.txt")
        good = Evidence.load([docs], [], [repo], least=100, keep_list=no_list)
        assert good.problems == [] and good.cites_run("20261103-010838-h14-w2") and good.cites_run("20261103-020202-x"), good.problems
        assert good.kept == {} and not good.keeps("C:\\r\\20261103-010838-h14-w2")
        assert "cannot be read" in " ".join(Evidence.load([os.path.join(temp, "no-docs")], [], [repo], least=1, keep_list=no_list).problems)
        assert "no commit messages" in " ".join(Evidence.load([docs], [], [os.path.join(temp, "no-repo")], least=1, keep_list=no_list).problems)
        no_commits = os.path.join(temp, "empty-repo")
        assert git(temp, "init", "-q", "-b", "master", no_commits).returncode == 0
        assert "no commit messages" in " ".join(Evidence.load([docs], [], [no_commits], least=1, keep_list=no_list).problems), "a git log that fails"
        assert "characters" in " ".join(Evidence.load([docs], [], [repo], keep_list=no_list).problems), "too little text"
        # A file held open with no sharing cannot be read.
        held = subprocess.Popen(["powershell", "-NoProfile", "-Command", "$f=[IO.File]::Open('%s','Open','ReadWrite','None'); Write-Output held; [Console]::In.ReadLine() | Out-Null; $f.Close()" % os.path.join(docs, "a.md")], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        assert held.stdout.readline().strip() == "held"
        assert "a file cannot be read" in " ".join(Evidence.load([docs], [], [repo], least=1, keep_list=no_list).problems)
        held.communicate("\n")

        # The keep list (D152), in the Archivist's form: path, reason, date, split by tabs. Each good line keeps its
        # path and what lies in it; a path that is no run is no problem. A line without a full path, a reason or a
        # date is a problem, and the problem reaches the evidence, so that no removal runs.
        keep_list = os.path.join(temp, "keep-runs.txt")
        good_line = "C:/r/results/20261101-010101-open\topen: risk 76; open: D150 C (the design)\t2026-11-09"
        # A byte order mark is no part of the first line (A1).
        with open(keep_list, "w", encoding="utf-8-sig") as file:
            file.write(good_line + "\n"
                       "E:/keep/named-folder\ttool: proj76.py\t2026-11-09\n"
                       "E:/keep/mgebuilds/f199\towner\t2026-11-09\n"
                       "E:/bench/walk.nircap\ttool: bench_replay.py (the walk)\t2026-11-09\n"
                       "# runs that stay whole\n#   open: <id>\tthe evidence\n\n")
        kept, problems = read_keep_list(keep_list)
        assert problems == [] and len(kept) == 4, (kept, problems)
        listed = Evidence.load([docs], [], [repo], least=1, keep_list=keep_list)
        assert listed.problems == [] and listed.keeps("c:\\R\\results\\20261101-010101-open") and listed.keeps("E:\\keep\\named-folder\\20261001-000000-y")
        assert listed.keeps("E:/keep/mgebuilds/f199") and not listed.keeps("E:/keep/mgebuilds/f19") and not listed.keeps("C:/r/results/20261101-010101-opener")
        assert listed.kept_line("C:/r/results/20261101-010101-open/a.png") == "line 1: " + good_line.replace("\t", " | ") and listed.kept_line("C:/r/x") is None
        # A path too wide to keep (A2), and a path through a link (A8), are problems.
        checkout = os.path.join(temp, "checkout")
        os.makedirs(os.path.join(checkout, "results"))
        subprocess.run(["cmd", "/c", "mklink", "/J", os.path.join(temp, "link-to-docs"), docs], capture_output=True, check=True)
        wide = ["E:/", "C:\\", "E:/nirender-keep", "e:/NIRENDER-RESULTS/", "E:/nirender-keep/mgebuilds", "E:/nirender-keep/dumps", "C:/tmp/mwdumps",
                "C:/r/results", "E:/nirender-keep/runs-20261109", "E:/x/full-dumps-1", os.path.dirname(REPOS_ROOT), REPOS_ROOT, checkout]
        for path in wide:
            assert too_wide_to_keep(path), path
        for path in ("C:/r/results/20261101-010101-open", "E:/nirender-keep/mgebuilds/f199", "E:/nirender-keep/skin-regress-2026-11-09", "E:/nirender-results/bench/a.nircap", os.path.join(checkout, "results", "20261101-010101-x")):
            assert too_wide_to_keep(path) is None, path
        assert through_link(os.path.join(temp, "link-to-docs")) == os.path.join(temp, "link-to-docs")
        assert through_link(os.path.join(temp, "link-to-docs", "20261101-010101-x")) == os.path.join(temp, "link-to-docs")
        assert through_link(os.path.join(docs, "20261101-010101-x")) is None and through_link("Q:/none/20261101-010101-x") is None
        links = [os.path.join(temp, "link-to-docs"), os.path.join(temp, "link-to-docs", "20261101-010101-x")]
        for path, words in [(path, "too wide") for path in wide] + [(path, "through a link") for path in links]:
            with open(keep_list, "w", encoding="ascii") as file:
                file.write(good_line + "\n" + path + "\towner\t2026-11-09\n")
            kept, problems = read_keep_list(keep_list)
            assert len(kept) == 1 and len(problems) == 1 and words in problems[0] and "line 2" in problems[0], (path, problems)
            assert words in " ".join(Evidence.load([docs], [], [repo], least=1, keep_list=keep_list).problems), path
            if words == "through a link":
                assert "name the real path, " + os.path.realpath(path) in problems[0], problems[0]
        for bad, words in (("C:/r/20261101-040404-x\t\t2026-11-09", "no reason"), ("C:/r/20261101-040404-x\tbecause it is cited\t2026-11-09", "no reason"),
                           ("C:/r/20261101-040404-x\topen:\t2026-11-09", "no reason"), ("C:/r/20261101-040404-x", "no reason"),
                           ("C:/r/20261101-040404-x\towner", "no date"), ("C:/r/20261101-040404-x\topen: risk 77\t9 Nov", "no date"),
                           ("20261101-040404-x\towner\t2026-11-09", "no full path"), ("/r/20261101-040404-x\towner\t2026-11-09", "no full path"),
                           ("C:/r/20261101-040404-x  owner  2026-11-09", "no reason"), ("C:/r/x\towner\t2026-11-09\textra", "more than three")):
            with open(keep_list, "w", encoding="ascii") as file:
                file.write(good_line + "\n" + bad + "\n")
            kept, problems = read_keep_list(keep_list)
            assert set(kept) == {keep_key("C:/r/results/20261101-010101-open")} and len(problems) == 1 and words in problems[0] and "line 2" in problems[0], (bad, problems)
            assert words in " ".join(Evidence.load([docs], [], [repo], least=1, keep_list=keep_list).problems), bad
        # A keep list that cannot be read is a problem too: a folder of that name.
        assert "cannot be read" in read_keep_list(docs)[1][0]
        # A copy of a run and the sets are no evidence, and each named folder is.
        names = [os.path.basename(path) for path in evidence_folders()]
        assert not any(name.startswith("runs-") or name == "mgebuilds" for name in names)
    for name in ("host-a.PNG", "a.nircap", "x.mp4", "y.tga"):
        assert is_heavy(name), name
    for name in ("host.txt", "MWSE.log", "a.DMP", "b.py", "c.sh", "d.ess", "e.ini", "f.xml", "g.wgsl", "h.dll", "none"):
        assert not is_heavy(name), name
    # A citation keeps the text only: a cited run trims at 7 days and never expires. Only the keep list keeps a run whole.
    for age, cited, kept, expected in [(0.01, False, False, "running"), (0.01, True, False, "running"), (6.9, True, False, "young"),
                                       (6.9, False, False, "young"), (7.0, False, False, "trim"), (7.0, True, False, "trim"),
                                       (29.9, False, False, "trim"), (30.0, False, False, "expired"), (400, True, False, "trim"),
                                       (400, False, True, "kept"), (0.01, False, True, "kept"), (400, True, True, "kept")]:
        assert run_treatment(age, cited, kept) == expected, (age, cited, kept)
    now = time.mktime(time.strptime("20261103-120000", "%Y%m%d-%H%M%S"))
    assert run_age_days("20261102-120000-x", now) == 1.0
    assert run_age_days("bench", now) is None and run_age_days("20261399-120000-x", now) is None
    evidence = Evidence("see 20261103-010838-h14-w2/a.png and -012408-h3-w1 and 20261103-020000-short, set f94 and gate-f96 and xf97 f985; the dump Morrowind.exe.111.dmp, pid 222, 333 frames")
    assert evidence.cites_run("20261103-010838-h14-w2") and evidence.cites_run("20261103-012408-h3-w1")
    assert evidence.cites_run("20261103-020000-short-longer-label"), "a citation with a shorter label"
    assert evidence.cites_run("20261102-012408-other"), "a short citation counts by its time alone"
    assert not evidence.cites_run("20261103-012630-h3-d") and not evidence.cites_run("20261102-012630-h3")
    assert evidence.cites_word("f94") and evidence.cites_word("f96") and not evidence.cites_word("f97") and not evidence.cites_word("f98")
    assert evidence.cites_dump("C:\\d\\Morrowind.exe.111.dmp") and evidence.cites_dump("C:\\d\\Morrowind.exe.222.dmp")
    assert not evidence.cites_dump("C:\\d\\Morrowind.exe.333.dmp") and not evidence.cites_dump("C:\\d\\Morrowind.exe.22.dmp")
    assert Evidence("the hang dump 60548").cites_dump("C:\\d\\hang-60548.dmp")
    # The name of a mini dump is in the docs, and it keeps no run.
    plain = Evidence("MWSE_MiniDump.dmp holds it; the file Morrowind.exe.111.dmp; the run h14-w2-fine")
    assert not plain.cites_dump("C:\\r\\MWSE_MiniDump.dmp") and plain.cites_dump("C:\\r\\Morrowind.exe.111.dmp")
    # The same for the names of the two programs, which each text holds.
    programs = Evidence("numidium.dmp and Morrowind.dmp")
    assert not programs.cites_dump("C:\\r\\numidium.dmp") and not programs.cites_dump("C:\\r\\Morrowind.dmp")
    # A run counts as cited by a dump inside it, and by nothing else among its files.
    uncited = "20261001-000000-x"
    assert plain.cites_run_with(uncited, [("C:\\r\\a.png", 1, 0.0), ("C:\\r\\Morrowind.exe.111.dmp", 1, 0.0)])
    assert not plain.cites_run_with(uncited, [("C:\\r\\Morrowind.exe.111.png", 1, 0.0), ("C:\\r\\MWSE_MiniDump.dmp", 1, 0.0)])
    # A label alone is listed for the Archivist. It keeps nothing.
    assert plain.label_only("20261001-000000-h14-w2-fine") and not plain.label_only("20261001-000000-other") and not plain.label_only("20261001-000000-the")
