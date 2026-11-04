"""The parts that the two cleaning tools share: the walk that never goes through a link, a removal of the
same kind, the processes, the log, the citations and the expiry of the data rule (D71). No entry point:
`clean_merged_worktrees.py` and `clean_results.py` are the tools.

A run or a dump is evidence when a record, a plan page or a commit names it. Anything else expires:

- a run folder: pictures and captures at PICTURE_DAYS, text and dumps at TEXT_DAYS;
- a dump outside a run: at TEXT_DAYS.
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
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPOS_ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
KEEP_ROOT = "E:\\nirender-keep"
LOG = os.path.join(KEEP_ROOT, "clean-worktrees.log")
DOCS = os.path.join(REPOS_ROOT, "moreFPS", "docs")
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
    command = "Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,ExecutablePath,CommandLine | ConvertTo-Json -Compress"
    result = subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", command], capture_output=True, text=True, encoding="utf-8", errors="replace")
    found = []
    unread = 0
    for row in json.loads(result.stdout):
        folder = current_folder(row["ProcessId"])
        unread += folder is None
        found.append({"pid": row["ProcessId"], "parent": row["ParentProcessId"], "name": row["Name"] or "", "exe": row["ExecutablePath"] or "", "command": row["CommandLine"] or "", "cwd": folder or ""})
    return found, unread


def own_line(running, pid):
    """This process and those that started it: the shell that runs this tool does not use a tree."""
    parents = {process["pid"]: process.get("parent") for process in running}
    line = set()
    while pid in parents and pid not in line:
        line.add(pid)
        pid = parents[pid]
    return line | {pid}


def users_of(tree_path, running, skip=None):
    """The processes that use a tree, by name and id. `skip` holds the process ids to leave out: by default this process."""
    skip = {os.getpid()} if skip is None else skip
    users = []
    for process in running:
        if process["pid"] in skip:
            continue
        slashes = os.path.normcase(os.path.abspath(tree_path)).replace("\\", "/")
        shell_form = "/" + slashes[0] + slashes[2:]
        command = os.path.normcase(process["command"]).replace("\\", "/")
        in_command = re.search("(?:" + re.escape(slashes) + "|" + re.escape(shell_form) + r")(?=[/\"' ]|$)", command) is not None
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


class Evidence:
    """What the docs, the notes, the accepted baselines and the commit messages name."""

    def __init__(self, text, problems=()):
        self.text = text
        # Why the text may lack a citation. A removal must not run on it: every run would read as uncited.
        self.problems = list(problems)
        self.stamps = set(STAMP.findall(text))
        self.shorts = set(SHORT_CITATION.findall(text))
        self.dump_numbers = set(DUMP_NUMBER.findall(text))

    @classmethod
    def load(cls, folders=None, names=None, repos=COMMIT_REPOS, least=LEAST_EVIDENCE):
        """The text of each folder and each commit message. `names` are folders whose own name counts too."""
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
        return cls(text, problems)

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


def run_treatment(age_days, cited):
    """What happens to a run folder: "cited" and "running" and "young" stay whole, "trim" loses its pictures,
    "expired" goes."""
    if cited:
        return "cited"
    if age_days < BUSY_DAYS:
        return "running"
    if age_days < PICTURE_DAYS:
        return "young"
    if age_days < TEXT_DAYS:
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
    assert own_line([{"pid": 5, "parent": 3}, {"pid": 3, "parent": 1}, {"pid": 9, "parent": 3}], 5) == {5, 3, 1}
    assert users_of("C:\\t", [{"pid": 3, "name": "sh", "exe": "", "command": "", "cwd": "C:\\t\\x"}], skip={3}) == []
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
        good = Evidence.load([docs], [], [repo], least=100)
        assert good.problems == [] and good.cites_run("20261103-010838-h14-w2") and good.cites_run("20261103-020202-x"), good.problems
        assert "cannot be read" in " ".join(Evidence.load([os.path.join(temp, "no-docs")], [], [repo], least=1).problems)
        assert "no commit messages" in " ".join(Evidence.load([docs], [], [os.path.join(temp, "no-repo")], least=1).problems)
        no_commits = os.path.join(temp, "empty-repo")
        assert git(temp, "init", "-q", "-b", "master", no_commits).returncode == 0
        assert "no commit messages" in " ".join(Evidence.load([docs], [], [no_commits], least=1).problems), "a git log that fails"
        assert "characters" in " ".join(Evidence.load([docs], [], [repo]).problems), "too little text"
        # A file held open with no sharing cannot be read.
        held = subprocess.Popen(["powershell", "-NoProfile", "-Command", "$f=[IO.File]::Open('%s','Open','ReadWrite','None'); Write-Output held; [Console]::In.ReadLine() | Out-Null; $f.Close()" % os.path.join(docs, "a.md")], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        assert held.stdout.readline().strip() == "held"
        assert "a file cannot be read" in " ".join(Evidence.load([docs], [], [repo], least=1).problems)
        held.communicate("\n")
        # A copy of a run and the sets are no evidence, and each named folder is.
        names = [os.path.basename(path) for path in evidence_folders()]
        assert not any(name.startswith("runs-") or name == "mgebuilds" for name in names)
    for name in ("host-a.PNG", "a.nircap", "x.mp4", "y.tga"):
        assert is_heavy(name), name
    for name in ("host.txt", "MWSE.log", "a.DMP", "b.py", "c.sh", "d.ess", "e.ini", "f.xml", "g.wgsl", "h.dll", "none"):
        assert not is_heavy(name), name
    for age, cited, expected in [(0.01, False, "running"), (0.01, True, "cited"), (6.9, False, "young"), (7.0, False, "trim"), (29.9, False, "trim"), (30.0, False, "expired"), (400, True, "cited")]:
        assert run_treatment(age, cited) == expected, (age, cited)
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
