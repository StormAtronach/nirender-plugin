"""The symbol store of the renderer's builds (W345, the owner's D299).

    python -I tools/symbols.py add --name f231 BINARY... [--pdb-dir DIR]...
    python -I tools/symbols.py pending      # the adds that a timing run put off
    python -I tools/symbols.py status
    python -I tools/symbols.py --self-test  # scratch stores, symstore.exe, the pairs of Visual Studio's test framework

The store, harness.SYMBOL_STORE (E:\\nirender-keep\\symbols), is a symstore store of compressed pdbs. Each pdb is
indexed by the GUID and the age that its binary's CodeView record names, so cdb finds the pdb of a binary by itself
(harness.SYMBOL_PATH). The renderer's binaries go in: d3d8.dll, dinput8.dll, numidium.exe and akulakhan.dll
(RENDERER_BINARIES). MWSE's pdb stays out.

add: the pdb of each binary, beside it or in a --pdb-dir, is taken only when it holds the binary's GUID and age. A
build is one transaction: product nirender, version the build's name, comment the md5 of each binary. When one live
transaction already holds every pdb of the build, nothing is added. Otherwise all its pdbs go in: a pdb that two
builds share then has a reference from each, and symstore keeps it at the delete of the older one.

prune, after each add: while the store is over its budget (BUDGET, 2 GB), the oldest transaction goes, by symstore's
own delete of its id (never a delete of folders). A transaction stays when a line of the keep list
(clean_common.KEEP_LIST) names its build or the md5 of one of its binaries, as a whole word (8 or more hex digits of
an md5 count). The newest transaction stays.

The writes to E: wait only for a timing run (the owner's D297): when `slot.py e-ok` refuses, the add goes into PENDING
on C:, and the next add, or `symbols.py pending`, takes it. Two adds at once take turns at a lock on C:.

Each call prints one line, for example "symbols: +3, store 1.6 of 2.0 GB, pruned f231". A fault prints its line and
stops nothing: the build tools go on. This tool writes only into the store: no pdb goes into an install, an archive
or a play set.
"""
import argparse
import collections
import contextlib
import csv
import hashlib
import io
import msvcrt
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from clean_common import KEEP_LIST  # noqa: E402
from common import docs_place, local  # noqa: E402

HARNESS_DIR = local("harness", docs_place("docs", "scripts", "test-harness-2026-10-03"))


def symbol_store():
    """harness.SYMBOL_STORE, or None when the harness has none. The harness is imported here, not at load, so that the
    build tools that import this module do not need the docs checkout."""
    if HARNESS_DIR not in sys.path:
        sys.path.insert(0, HARNESS_DIR)
    import harness

    return getattr(harness, "SYMBOL_STORE", None)


SYMSTORE = r"C:\Program Files (x86)\Windows Kits\10\Debuggers\x64\symstore.exe"
SLOT_PY = os.path.join(HARNESS_DIR, "slot.py")
GB = 10**9
BUDGET = 2 * GB
PRODUCT = "nirender"
RENDERER_BINARIES = ("d3d8.dll", "dinput8.dll", "numidium.exe", "akulakhan.dll")
# The folders of the pdbs in a build tree: render-link's d3d8 and dinput8, its helper, and the plugin.
TREE_PDB_DIRS = (("target", "i686-pc-windows-msvc", "release-dll"), ("target", "x86_64-pc-windows-msvc", "release"),
                 ("target", "i686-pc-windows-msvc", "release"))
STATE = os.path.join(os.environ.get("LOCALAPPDATA") or tempfile.gettempdir(), "nirender")
PENDING = os.path.join(STATE, "symbols-pending.txt")
LOCK = os.path.join(STATE, "symbols.lock")
LOCK_SECONDS = 600
# The longest symstore run: the add of f222 took 28 s.
SYMSTORE_SECONDS = 300

Entry = collections.namedtuple("Entry", "binary name key md5")
Transaction = collections.namedtuple("Transaction", "id version comment keys")


class SymbolError(Exception):
    """A fault of symstore or of the store."""


# --- the binaries and their pdbs ------------------------------------------------------------

def codeview(binary):
    """The GUID, the age and the pdb's file name of the CodeView record (RSDS) of a binary's bytes, or None.

    This is a scan of the bytes for the first "RSDS" with a pdb's name after it, not a walk of the PE debug
    directory. The 4 binaries of f213 have one RSDS each, and the scan gives the values of a walk of the
    directory (the Reviewer, 2026-11-04)."""
    at = binary.find(b"RSDS")
    while at >= 0:
        end = binary.find(b"\0", at + 24)
        path = binary[at + 24:end] if end > 0 else b""
        if path.lower().endswith(b".pdb"):
            name = path.decode("ascii", "replace").replace("\\", "/").rsplit("/", 1)[-1]
            return binary[at + 4:at + 20], struct.unpack_from("<I", binary, at + 20)[0], name
        at = binary.find(b"RSDS", at + 4)
    return None


def index_key(guid, age):
    """The folder name of a pdb in a symbol store: the GUID in its text order, then the age in hex."""
    first, second, third = struct.unpack_from("<IHH", guid)
    return "%08X%04X%04X%s%X" % (first, second, third, guid[8:].hex().upper(), age)


MSF_MAGIC = b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0"


def pdb_identity(pdb):
    """The GUID of the pdb's info stream (stream 1) and the age of its DBI stream (stream 3), or None for a file that
    is not an MSF 7.0 pdb. A debugger matches these two against a binary's CodeView record. The info stream's own
    age can be higher than the binary's (2 against 1 in the pdbs of Visual Studio's test framework)."""
    with open(pdb, "rb") as file:
        head = file.read(56)
        if len(head) < 56 or not head.startswith(MSF_MAGIC):
            return None
        block, _, _, directory_bytes, _, map_block = struct.unpack_from("<IIIIII", head, 32)

        def blocks(numbers, length):
            data = b""
            for number in numbers:
                file.seek(number * block)
                data += file.read(block)
            return data[:length]

        file.seek(map_block * block)
        directory_blocks = struct.unpack("<%dI" % -(-directory_bytes // block), file.read(4 * -(-directory_bytes // block)))
        directory = blocks(directory_blocks, directory_bytes)
        count = struct.unpack_from("<I", directory)[0]
        if count < 4:
            return None
        sizes = struct.unpack_from("<%dI" % count, directory, 4)
        at, lists = 4 + 4 * count, []
        for size in sizes:
            used = 0 if size == 0xFFFFFFFF else -(-size // block)
            lists.append(struct.unpack_from("<%dI" % used, directory, at))
            at += 4 * used
        info = blocks(lists[1], min(sizes[1], 28))
        dbi = blocks(lists[3], min(sizes[3], 12))
        if len(info) < 28 or len(dbi) < 12:
            return None
        return info[12:28], struct.unpack_from("<I", dbi, 8)[0]


def write_test_pdb(path, guid, age, info_age=None):
    """Writes the smallest MSF 7.0 file that pdb_identity reads as (guid, age), for the self-tests. Its info stream has
    the age `info_age` (default `age`): a linker writes a higher one there than in the DBI stream. Block 0 is the
    superblock, block 1 the map of the directory's blocks, block 2 the directory, block 3 the info stream (stream 1)
    and block 4 the DBI stream (stream 3)."""
    block = 512
    info = struct.pack("<III", 20000404, 0, age if info_age is None else info_age) + guid
    dbi = struct.pack("<iII", -1, 19990903, age)
    directory = struct.pack("<I4I", 4, 0, len(info), 0, len(dbi)) + struct.pack("<II", 3, 4)
    superblock = MSF_MAGIC + struct.pack("<IIIIII", block, 1, 5, len(directory), 0, 1)
    with open(path, "wb") as file:
        for data in (superblock, struct.pack("<I", 2), directory, info, dbi):
            file.write(data.ljust(block, b"\0"))


def pdb_matches(pdb, guid, age):
    """True when the pdb is the one a debugger loads for a binary with this GUID and age."""
    try:
        return pdb_identity(pdb) == (guid, age)
    except (OSError, struct.error):
        return False


def entries_of(binaries):
    """The store entry of each binary, from its CodeView record, and a note for each binary without one."""
    entries, notes = [], []
    for binary in binaries:
        shown = os.path.basename(binary)
        try:
            with open(binary, "rb") as file:
                data = file.read()
        except OSError as error:
            notes.append("%s: %s" % (shown, error.strerror or error))
            continue
        record = codeview(data)
        if record is None:
            notes.append("%s: no CodeView record" % shown)
            continue
        guid, age, name = record
        entries.append((Entry(binary, name, index_key(guid, age), hashlib.md5(data).hexdigest()), guid, age))
    return entries, notes


def pdb_of(binary, name, guid, age, pdb_dirs):
    """The pdb of the binary's build: beside it, else in one of `pdb_dirs`; None when no pdb there matches."""
    for folder in [os.path.dirname(binary)] + list(pdb_dirs):
        candidate = os.path.join(folder, name)
        if os.path.isfile(candidate) and pdb_matches(candidate, guid, age):
            return candidate
    return None


def build_name(folder):
    """The transaction name of a build folder: a play set's name f<N> as it is; any other name with the first 8 hex
    digits of the md5 of its d3d8.dll, for example "build-1a2b3c4d". A plain word such as "build" would be pinned by
    any keep-list line that has that word."""
    base = os.path.basename(os.path.normpath(folder))
    d3d8 = os.path.join(folder, "d3d8.dll")
    if re.fullmatch(r"f\d+", base, re.I) or not os.path.isfile(d3d8):
        return base
    return "%s-%s" % (base, md5_of(d3d8)[:8])


def tree_pdb_dirs(*trees):
    """The pdb folders of build trees (render-link, nirender-plugin), for a build made from them."""
    return [os.path.join(tree, *parts) for tree in trees for parts in TREE_PDB_DIRS]


# --- the store ------------------------------------------------------------------------------

def entry_id(name, key):
    return (name + "\\" + key).lower()


def transactions(store):
    """The live transactions of the store, oldest first, each with the entries it holds."""
    try:
        with open(os.path.join(store, "000Admin", "server.txt"), encoding="utf-8", errors="replace") as file:
            text = file.read()
    except FileNotFoundError:
        return []
    found = []
    for fields in csv.reader(io.StringIO(text)):
        if len(fields) < 8 or fields[1].strip().lower() != "add":
            continue
        found.append(Transaction(fields[0].strip(), fields[6], fields[7], transaction_entries(store, fields[0].strip())))
    return sorted(found, key=lambda transaction: int(transaction.id))


def transaction_entries(store, transaction_id):
    """The entries of a transaction, from its file in 000Admin: "name\\key","source" for each."""
    try:
        with open(os.path.join(store, "000Admin", transaction_id), encoding="utf-8", errors="replace") as file:
            return {fields[0].lower() for fields in csv.reader(file) if fields}
    except FileNotFoundError:
        return set()


def stored(store, entry):
    """True when the store holds the pdb of an entry ("name\\key"): a file other than refs.ptr in its folder."""
    folder = os.path.join(store, *entry.split("\\"))
    try:
        return any(name.lower() != "refs.ptr" for name in os.listdir(folder))
    except OSError:
        return False


def missing(store, live):
    """The entries that a live transaction names and the store has no pdb for, as "build: name\\key". symstore writes
    the transaction first and copies the pdbs after it, so an add that is stopped (the timeout of run_symstore)
    leaves such entries (W352: an add of 5 pdbs stopped at 0.15 to 0.8 s named 5 and held 4). A pdb file that was
    only partly copied counts as held: this finds a missing file, not a cut one."""
    return ["%s: %s" % (transaction.version or transaction.id, entry)
            for transaction in live for entry in sorted(transaction.keys) if not stored(store, entry)]


def store_size(store):
    """The bytes of every file of the store."""
    total = 0
    for folder, _, names in os.walk(store):
        for name in names:
            total += os.path.getsize(os.path.join(folder, name))
    return total


def run_symstore(arguments, symstore, seconds=None):
    """Runs symstore; a nonzero exit code, an error that it counts or a run longer than SYMSTORE_SECONDS is a
    SymbolError (an add then waits in the pending list)."""
    seconds = SYMSTORE_SECONDS if seconds is None else seconds
    try:
        done = subprocess.run([symstore] + arguments, capture_output=True, text=True, errors="replace", stdin=subprocess.DEVNULL, timeout=seconds)
    except subprocess.TimeoutExpired as error:
        raise SymbolError("symstore %s: no end in %g s" % (arguments[0], seconds)) from error
    except OSError as error:
        raise SymbolError("symstore %s: %s" % (arguments[0], error)) from error
    errors = re.search(r"Number of errors = (\d+)", done.stdout)
    if done.returncode != 0 or (errors and int(errors.group(1))):
        tail = (done.stdout + done.stderr).strip().splitlines()[-3:]
        raise SymbolError("symstore %s: exit %d: %s" % (arguments[0], done.returncode, " / ".join(tail)))
    return done.stdout


# --- the keep list --------------------------------------------------------------------------

def keep_lines(keep_list):
    """The lines of the keep list that are not notes, and a problem when the file exists and cannot be read."""
    try:
        with open(keep_list, encoding="utf-8-sig", errors="replace") as file:
            lines = file.read().splitlines()
    except FileNotFoundError:
        return [], None
    except OSError as error:
        return [], "the keep list cannot be read: %s (%s)" % (keep_list, error)
    return [line for line in lines if line.strip() and not line.lstrip().startswith("#")], None


def pin_of(transaction, lines):
    """The keep-list line that pins a transaction: one that names its build, or 8 or more hex digits of the start of
    one of its binaries' md5, as a whole word. None when no line does."""
    md5s = re.findall(r"=([0-9a-f]{32})\b", transaction.comment.lower())
    for line in lines:
        if transaction.version and re.search(r"(?<![\w.-])%s(?![\w.-])" % re.escape(transaction.version), line, re.I):
            return line
        for word in re.findall(r"(?<![0-9a-z])[0-9a-f]{8,32}(?![0-9a-z])", line.lower()):
            if any(md5.startswith(word) for md5 in md5s):
                return line
    return None


# --- add and prune --------------------------------------------------------------------------

def add_build(store, name, binaries, pdb_dirs, symstore):
    """Adds one build as one transaction. Returns (the entries that were not in the store, notes)."""
    found, notes = entries_of(binaries)
    if not found:
        return 0, notes
    wanted = {entry_id(entry.name, entry.key) for entry, _, _ in found}
    for transaction in transactions(store):
        # A transaction of an add that was stopped names pdbs it does not hold: it does not count.
        if wanted <= transaction.keys and all(stored(store, entry) for entry in wanted):
            return 0, notes + ["%s: its pdbs are in the store (%s)" % (name, transaction.version or transaction.id)]
    pdbs, entries = [], []
    for entry, guid, age in found:
        pdb = pdb_of(entry.binary, entry.name, guid, age, pdb_dirs)
        if pdb is None:
            notes.append("%s: no pdb of this build (%s)" % (os.path.basename(entry.binary), entry.name))
            continue
        pdbs.append(pdb)
        entries.append(entry)
    if not pdbs:
        return 0, notes
    new = sum(1 for entry in entries if not stored(store, entry_id(entry.name, entry.key)))
    comment = "md5 " + " ".join("%s=%s" % (os.path.basename(entry.binary), entry.md5) for entry in entries)
    with tempfile.TemporaryDirectory(prefix="symbols-") as folder:
        listing = os.path.join(folder, "pdbs.txt")
        with open(listing, "w", encoding="utf-8") as file:
            file.write("".join(os.path.abspath(pdb) + "\n" for pdb in pdbs))
        run_symstore(["add", "/f", "@" + listing, "/s", store, "/t", PRODUCT, "/v", name, "/c", comment, "/compress"], symstore)
    return new, notes


def prune(store, budget, keep_list, symstore):
    """Deletes the oldest transactions that no keep-list line pins while the store is over the budget; the newest
    stays. Returns (the names pruned, notes)."""
    lines, problem = keep_lines(keep_list)
    if problem:
        return [], [problem + "; nothing pruned"]
    pruned, pinned = [], []
    live = transactions(store)
    size = store_size(store)
    for transaction in live[:-1]:
        if size <= budget:
            break
        if pin_of(transaction, lines):
            pinned.append(transaction.version or transaction.id)
            continue
        run_symstore(["del", "/i", transaction.id, "/s", store], symstore)
        pruned.append(transaction.version or transaction.id)
        size = store_size(store)
    notes = []
    if size > budget:
        notes.append("over the budget: %s pinned by the keep list%s" % (", ".join(pinned) or "none", ", and the newest stays" if live else ""))
    return pruned, notes


def e_ok():
    """None when no timing run holds the game (slot.py e-ok), else why a write to E: waits (D297)."""
    try:
        done = subprocess.run([sys.executable, "-I", SLOT_PY, "e-ok"], capture_output=True, text=True, timeout=120, stdin=subprocess.DEVNULL)
    except (OSError, subprocess.SubprocessError) as error:
        return "slot.py e-ok did not answer: %s" % error
    return None if done.returncode == 0 else (done.stdout.strip() or "slot.py e-ok exit %d" % done.returncode)


@contextlib.contextmanager
def store_lock(path, seconds=LOCK_SECONDS):
    """One add or prune at a time on this machine: a lock on a byte of a file on C:."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "a+b") as file:
        deadline = time.time() + seconds
        while True:
            try:
                file.seek(0)
                msvcrt.locking(file.fileno(), msvcrt.LK_NBLCK, 1)
                break
            except OSError:
                if time.time() > deadline:
                    raise SymbolError("the lock %s is held for more than %d s" % (path, seconds))
                time.sleep(1)
        try:
            yield
        finally:
            file.seek(0)
            msvcrt.locking(file.fileno(), msvcrt.LK_UNLCK, 1)


def md5_of(path):
    with open(path, "rb") as file:
        return hashlib.md5(file.read()).hexdigest()


def read_pending(path):
    """The adds that waited for a timing run: (name, binaries, pdb folders, md5s) for each line. The md5s are those of
    the binaries when the add was put off, in their order; a line of the three fields before W352 has none."""
    try:
        with open(path, encoding="utf-8") as file:
            lines = file.read().splitlines()
    except FileNotFoundError:
        return []
    found = []
    for line in lines:
        fields = line.split("\t")
        if len(fields) in (3, 4) and fields[0]:
            split = [[part for part in field.split("|") if part] for field in fields[1:]]
            found.append((fields[0], split[0], split[1], split[2] if len(split) == 3 else []))
    return found


def write_pending(path, builds):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as file:
        file.write("".join("%s\t%s\t%s\t%s\n" % (name, "|".join(binaries), "|".join(dirs), "|".join(md5s))
                           for name, binaries, dirs, md5s in builds))


def unchanged(build_name, binaries, md5s, notes):
    """The binaries of a build that are there and have the md5 of the time of its add, with a note for each other one.
    A binary built again after its add was put off is not this build's any more."""
    kept = []
    for index, path in enumerate(binaries):
        if not os.path.isfile(path):
            continue
        if index < len(md5s) and md5_of(path) != md5s[index]:
            notes.append("%s: %s changed after the add was put off" % (build_name, os.path.basename(path)))
            continue
        kept.append(path)
    return kept


def add(name, binaries, pdb_dirs=(), *, store=None, budget=BUDGET, keep_list=KEEP_LIST, symstore=SYMSTORE, gate=e_ok,
        pending=PENDING, lock=LOCK):
    """Adds the pdbs of a build, after the builds that waited, then prunes. Returns the one line. With no name it
    takes only the builds that waited (`symbols.py pending`)."""
    try:
        store = store or symbol_store()
    except ImportError as error:
        return "symbols: not added: no harness at %s: %s" % (HARNESS_DIR, error)
    if not store:
        return "symbols: not added: the harness at %s names no SYMBOL_STORE" % HARNESS_DIR
    binaries =[os.path.abspath(path) for path in binaries if os.path.isfile(path)]
    pdb_dirs = [os.path.abspath(path) for path in pdb_dirs]
    try:
        with store_lock(lock):
            waiting = [build for build in read_pending(pending) if build[0] != name]
            if name and binaries:
                waiting.append((name, binaries, pdb_dirs, [md5_of(path) for path in binaries]))
            if not waiting:
                return "symbols: nothing to add"
            refused = gate()
            if refused:
                write_pending(pending, waiting)
                return "symbols: put off, %d builds wait in %s: %s" % (len(waiting), pending, refused)
            added, notes, left = 0, [], []
            for build_name, build_binaries, build_dirs, build_md5s in waiting:
                present = unchanged(build_name, build_binaries, build_md5s, notes)
                if not present:
                    notes.append("%s: its binaries are gone or changed" % build_name)
                    continue
                try:
                    count, build_notes = add_build(store, build_name, present, build_dirs, symstore)
                except SymbolError as error:
                    left.append((build_name, build_binaries, build_dirs, build_md5s))
                    notes.append("%s: %s" % (build_name, error))
                    continue
                added += count
                notes += build_notes
            write_pending(pending, left)
            # The adds are done: a fault of the prune is a note of this line, not "not added".
            try:
                pruned, prune_notes = prune(store, budget, keep_list, symstore)
            except Exception as error:  # noqa: BLE001 - the same rule as below
                pruned, prune_notes = [], ["prune: %s: %s" % (type(error).__name__, error)]
            line = "symbols: +%d, store %.1f of %.1f GB, pruned %s" % (added, store_size(store) / GB, budget / GB, ", ".join(pruned) or "none")
            return line + "".join("; " + note for note in notes + prune_notes)
    except Exception as error:  # noqa: BLE001 - a fault of the store must not stop the build tool that calls it
        return "symbols: not added: %s: %s" % (type(error).__name__, error)


def status(store, budget, keep_list, pending=PENDING):
    lines, problem = keep_lines(keep_list)
    rows = ["symbols: store %s, %.2f of %.1f GB" % (store, store_size(store) / GB if os.path.isdir(store) else 0, budget / GB)]
    live = transactions(store)
    for transaction in live:
        pin = pin_of(transaction, lines)
        rows.append("  %s %s: %d pdbs%s" % (transaction.id, transaction.version, len(transaction.keys), "; pinned: " + pin.replace("\t", " | ") if pin else ""))
    if problem:
        rows.append("  " + problem)
    gaps = missing(store, live)
    if gaps:
        rows.append("  %d entries of transactions have no pdb in the store (an add that was stopped):" % len(gaps))
        rows += ["    " + gap for gap in gaps]
    waiting = read_pending(pending)
    rows.append("  waiting in %s: %s" % (pending, ", ".join(build[0] for build in waiting) or "none"))
    return "\n".join(rows)


# --- the tests ------------------------------------------------------------------------------

# Native dlls with their pdbs beside them, from Visual Studio's C++ test framework: real CodeView records and pdbs.
FIXTURES = r"C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\TestWindow\VsTest"
FIXTURE_PREFIX = "Microsoft.VisualStudio.TestTools.CppUnitTestFramework."


def self_test():
    if not os.path.isfile(SYMSTORE):
        raise SystemExit("symbols self-test: no symstore.exe at %s" % SYMSTORE)
    names = ["ComInterfaces", "CppUnitTestExtension", "Discoverer", "TestEngine", "Executor"]
    for part in names:
        for suffix in (".dll", ".pdb"):
            if not os.path.isfile(os.path.join(FIXTURES, FIXTURE_PREFIX + part + suffix)):
                raise SystemExit("symbols self-test: no fixture %s%s%s" % (FIXTURE_PREFIX, part, suffix))
    work = tempfile.mkdtemp(prefix="symbols-test-")
    try:
        self_test_in(work)
    finally:
        shutil.rmtree(work, ignore_errors=True)
    print("symbols self-test: ok")


def self_test_in(work):
    def build(name, parts, pdb_beside=True):
        """A build folder with the fixtures' dlls; their pdbs beside them, or in a pdb folder of their own."""
        folder = os.path.join(work, name)
        pdbs = folder if pdb_beside else os.path.join(work, name + "-pdb")
        os.makedirs(folder)
        os.makedirs(pdbs, exist_ok=True)
        for part in parts:
            shutil.copy2(os.path.join(FIXTURES, FIXTURE_PREFIX + part + ".dll"), folder)
            shutil.copy2(os.path.join(FIXTURES, FIXTURE_PREFIX + part + ".pdb"), pdbs)
        return [os.path.join(folder, FIXTURE_PREFIX + part + ".dll") for part in parts], ([] if pdb_beside else [pdbs])

    store = os.path.join(work, "store")
    keep = os.path.join(work, "keep-runs.txt")
    pending = os.path.join(work, "pending.txt")
    lock = os.path.join(work, "symbols.lock")
    free = lambda: None  # noqa: E731 - the gate: no timing run

    def run(name, binaries, dirs=(), budget=BUDGET, gate=free):
        return add(name, binaries, dirs, store=store, budget=budget, keep_list=keep, gate=gate, pending=pending, lock=lock)

    # The import of this module leaves the harness out: the build tools import it without the docs checkout.
    tools = os.path.dirname(os.path.abspath(__file__))
    probe = "import sys; sys.path.insert(0, %r); import symbols; sys.exit(3 if 'harness' in sys.modules else 0)" % tools
    imported = subprocess.run([sys.executable, "-I", "-B", "-c", probe], capture_output=True, text=True, stdin=subprocess.DEVNULL)
    assert imported.returncode == 0, "the import of symbols imported the harness: %s" % (imported.stdout + imported.stderr)[-300:]

    # The index: the GUID and age of each fixture's CodeView record name its folder, and its pdb matches.
    a_binaries, _ = build("a", ["ComInterfaces", "CppUnitTestExtension", "Discoverer"])
    with open(a_binaries[0], "rb") as file:
        guid, age, pdb_name = codeview(file.read())
    assert pdb_name.lower() == (FIXTURE_PREFIX + "ComInterfaces.pdb").lower(), pdb_name
    assert pdb_matches(os.path.join(work, "a", pdb_name), guid, age)
    assert not pdb_matches(os.path.join(work, "a", FIXTURE_PREFIX + "Discoverer.pdb"), guid, age), "another build's pdb matched"
    # The test pdb of the other self-tests: its DBI age is the match, not its info age.
    small = os.path.join(work, "small.pdb")
    write_test_pdb(small, guid, 1, info_age=2)
    assert pdb_identity(small) == (guid, 1) and pdb_matches(small, guid, 1) and not pdb_matches(small, guid, 2)

    # The add: 3 pdbs, compressed, in the folders that the debugger reads; one transaction named a.
    line = run("a", a_binaries)
    assert line.startswith("symbols: +3, store "), line
    key = index_key(guid, age)
    packed = os.listdir(os.path.join(store, pdb_name, key))
    assert packed and all(name.endswith("_") for name in packed if name != "refs.ptr"), packed
    live = transactions(store)
    assert [t.version for t in live] == ["a"] and len(live[0].keys) == 3, live
    assert re.search(r"ComInterfaces\.dll=[0-9a-f]{32}", live[0].comment), live[0].comment

    # The dedupe: the same binaries again add nothing, and no transaction.
    again = run("a-again", a_binaries)
    assert again.startswith("symbols: +0,") and "its pdbs are in the store (a)" in again, again
    assert len(transactions(store)) == 1

    # The pdbs in a pdb folder; a pdb of another build there is not taken. b shares Discoverer with a.
    b_binaries, b_dirs = build("b", ["Discoverer", "TestEngine"], pdb_beside=False)
    shutil.copy2(os.path.join(FIXTURES, FIXTURE_PREFIX + "TestEngine.pdb"), os.path.join(work, "b-pdb", FIXTURE_PREFIX + "Discoverer.pdb.other"))
    line = run("b", b_binaries, b_dirs)
    assert line.startswith("symbols: +1,"), line
    assert len(transactions(store)[-1].keys) == 2, "b holds its shared pdb too"
    # A binary without its pdb: a note, and the rest goes in.
    c_binaries, _ = build("c", ["Executor"])
    os.remove(os.path.join(work, "c", FIXTURE_PREFIX + "Executor.pdb"))
    line = run("c", c_binaries)
    assert "no pdb of this build" in line and line.startswith("symbols: +0,"), line
    shutil.copy2(os.path.join(FIXTURES, FIXTURE_PREFIX + "Executor.pdb"), os.path.join(work, "c"))
    line = run("c", c_binaries)
    assert line.startswith("symbols: +1,"), line
    sizes = {t.version: t for t in transactions(store)}
    assert list(sizes) == ["a", "b", "c"], list(sizes)

    # The prune order and the budget: a budget just under the store prunes the oldest, a; the shared pdb stays for b.
    total = store_size(store)
    d_binaries, _ = build("d", ["TestEngine", "Discoverer"])
    line = run("d", d_binaries, budget=total - 1)
    assert "pruned a" in line and [t.version for t in transactions(store)] == ["b", "c"], (line, transactions(store))
    shared = os.path.join(store, FIXTURE_PREFIX + "Discoverer.pdb")
    assert os.listdir(shared), "the delete of a took the pdb that b shares"
    gone = os.path.join(store, FIXTURE_PREFIX + "ComInterfaces.pdb")
    assert not os.path.isdir(gone) or not [n for n in os.listdir(gone) if os.listdir(os.path.join(gone, n))], "a's own pdb stayed"
    # The pin: a keep line that names b (a dump of b) keeps it, and the prune goes on to c; the newest, g, stays. The
    # null: the same prune without the line takes b first (the order test above).
    g_binaries, _ = build("g", ["CppUnitTestExtension"])
    assert run("g", g_binaries).startswith("symbols: +1,")
    with open(keep, "w", encoding="utf-8") as file:
        file.write("# a note names c\nE:\\nirender-keep\\dumps\\x\topen: risk 9, build b\t2026-11-11\n")
    line = run("d", d_binaries, budget=1)
    assert "pruned c" in line and "pruned b" not in line and "over the budget: b pinned by the keep list" in line, line
    assert [t.version for t in transactions(store)] == ["b", "g"], transactions(store)
    # A pin by the md5 of a binary (its first 8 hex digits), and a word that only starts the same does not pin.
    with open(keep, "w", encoding="utf-8") as file:
        file.write("E:\\x\topen: the dump of %s\t2026-11-11\n" % hashlib.md5(open(c_binaries[0], "rb").read()).hexdigest()[:8])
    assert pin_of(Transaction("9", "c", "md5 x.dll=%s" % hashlib.md5(open(c_binaries[0], "rb").read()).hexdigest(), set()), keep_lines(keep)[0])
    assert not pin_of(Transaction("9", "f23", "md5", set()), ["E:\\x\towner: f231\t2026-11-11"]), "f23 matched f231"
    assert pin_of(Transaction("9", "f231", "md5", set()), ["E:\\x\towner: f231, f232\t2026-11-11"])
    # The name of a build folder: f<N> as it is; another name with its d3d8.dll's md5, so that a plain word in a keep
    # line ("its build plan") does not pin it. The null: the plain name "build" is pinned by that line.
    named = os.path.join(work, "names")
    for folder in ("f232", "build", "empty"):
        os.makedirs(os.path.join(named, folder))
        if folder != "empty":
            with open(os.path.join(named, folder, "d3d8.dll"), "wb") as file:
                file.write(b"abc")
    assert [build_name(os.path.join(named, folder)) for folder in ("f232", "build", "empty")] == ["f232", "build-90015098", "empty"]
    plan = ["E:\\x\topen: the design page and its build plan\t2026-11-11"]
    assert pin_of(Transaction("9", "build", "md5", set()), plan) and not pin_of(Transaction("9", "build-90015098", "md5", set()), plan)

    # A timing run puts the add off; the next add takes it. The null: with the gate free, it goes in at once.
    e_binaries, _ = build("e", ["Executor"])
    line = run("e", e_binaries, gate=lambda: "e-ok: no: a timing run holds the game")
    assert line.startswith("symbols: put off, 1 builds wait") and "e" not in [t.version for t in transactions(store)], line
    assert [build[0] for build in read_pending(pending)] == ["e"]
    line = run(None, [])
    assert line.startswith("symbols: +1,") and "e" in [t.version for t in transactions(store)], line
    assert read_pending(pending) == []
    assert run(None, []) == "symbols: nothing to add"
    # A binary built again while its add waits is not added under the old name; the other binary goes in. The null:
    # TestEngine, unchanged, is taken. A line of the three fields before W352 is read with no md5s.
    k_binaries, _ = build("k", ["TestEngine", "CppUnitTestExtension"])
    assert run("k", k_binaries, gate=lambda: "e-ok: no").startswith("symbols: put off")
    assert read_pending(pending)[0][3] == [hashlib.md5(open(path, "rb").read()).hexdigest() for path in k_binaries]
    shutil.copy2(os.path.join(FIXTURES, FIXTURE_PREFIX + "Executor.dll"), k_binaries[1])
    line = run(None, [])
    assert "k: %s changed after the add was put off" % os.path.basename(k_binaries[1]) in line, line
    # TestEngine alone is in b's transaction: the dedupe names it, so it was taken.
    assert "k: its pdbs are in the store (b)" in line, line
    assert read_pending(pending) == []
    with open(pending, "w", encoding="utf-8") as file:
        file.write("old\t%s\t\n" % k_binaries[0])
    assert read_pending(pending) == [("old", [k_binaries[0]], [], [])], read_pending(pending)
    write_pending(pending, [])

    # A fault of symstore is a line, not an exception, and the build waits for the next add.
    f_binaries, _ = build("f", ["ComInterfaces"])
    line = add("f", f_binaries, store=store, symstore=os.path.join(work, "no-symstore.exe"), keep_list=keep, gate=free, pending=pending, lock=lock)
    assert line.startswith("symbols: +0,") and "f: symstore add" in line, line
    assert [build[0] for build in read_pending(pending)] == ["f"], read_pending(pending)
    assert run(None, []).startswith("symbols: +1,") and read_pending(pending) == []

    # A symstore that does not end is stopped at its time, and the build waits (the Reviewer's verdict 193). The
    # stand-in sleeps 5 s and ends by itself, so a killed shell leaves no process for long.
    hang = os.path.join(work, "hang.cmd")
    with open(hang, "w", encoding="ascii") as file:
        file.write('@"%s" -c "import time; time.sleep(5)"\n' % sys.executable)
    # Discoverer is in b and Executor in e: no one transaction holds both, so symstore runs.
    h_binaries, _ = build("h", ["Discoverer"])
    global SYMSTORE_SECONDS
    saved, SYMSTORE_SECONDS = SYMSTORE_SECONDS, 1
    try:
        line = add("h", h_binaries + c_binaries, store=store, symstore=hang, keep_list=keep, gate=free, pending=pending, lock=lock)
    finally:
        SYMSTORE_SECONDS = saved
    assert "no end in 1 s" in line and [build[0] for build in read_pending(pending)] == ["h"], (line, read_pending(pending))
    write_pending(pending, [])
    # Any other fault is a line too: here a gate that raises.
    def broken():
        raise ValueError("a broken gate")
    line = add("i", h_binaries, store=store, keep_list=keep, gate=broken, pending=pending, lock=lock)
    assert line == "symbols: not added: ValueError: a broken gate", line

    # A fault of the prune after the adds: the build is in, and the line says so (the verdict 193 addendum). The
    # stand-in runs symstore for an add and fails a delete. ComInterfaces is in f and Executor in c: no one
    # transaction holds both, so the add runs.
    no_delete = os.path.join(work, "no-delete.cmd")
    with open(no_delete, "w", encoding="ascii") as file:
        file.write('@if /i "%%~1"=="del" exit /b 1\n@"%s" %%*\n' % SYMSTORE)
    j_binaries, _ = build("j", ["ComInterfaces", "Executor"])
    line = add("j", j_binaries, store=store, budget=1, symstore=no_delete, keep_list=keep, gate=free, pending=pending, lock=lock)
    assert line.startswith("symbols: +") and "; prune: SymbolError: symstore del: exit 1" in line, line
    assert "j" in [t.version for t in transactions(store)] and read_pending(pending) == [], line

    # An add that was stopped: symstore wrote the transaction and not every pdb (an add of the 5 fixtures stopped at
    # 0.15 to 0.8 s named 5 and held 4). Here the pdb file of Executor goes from the store. The status names the gap,
    # and a new add of Executor puts it back: the dedupe does not count a transaction without the pdb. The null:
    # before the gap, the same add is "in the store".
    assert "have no pdb in the store" not in status(store, BUDGET, keep, pending)
    m_binaries, _ = build("m", ["Executor"])
    assert "m: its pdbs are in the store" in run("m", m_binaries)
    with open(m_binaries[0], "rb") as file:
        m_guid, m_age, m_pdb = codeview(file.read())
    m_folder = os.path.join(store, m_pdb, index_key(m_guid, m_age))
    for name in os.listdir(m_folder):
        if name.lower() != "refs.ptr":
            os.remove(os.path.join(m_folder, name))
    shown = status(store, BUDGET, keep, pending)
    assert "have no pdb in the store" in shown and entry_id(m_pdb, index_key(m_guid, m_age)) in shown, shown
    line = run("m", m_binaries)
    assert line.startswith("symbols: +1,") and "in the store" not in line, line
    assert stored(store, entry_id(m_pdb, index_key(m_guid, m_age))) and missing(store, transactions(store)) == []


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--self-test", action="store_true", help="the tests: scratch stores, symstore.exe, Visual Studio's dlls")
    commands = parser.add_subparsers(dest="command")
    adding = commands.add_parser("add", help="add the pdbs of a build")
    adding.add_argument("--name", required=True, help="the build's name, for example the set folder f231")
    adding.add_argument("--pdb-dir", action="append", default=[], help="a folder of the build's pdbs, when they are not beside the binaries")
    adding.add_argument("binaries", nargs="+")
    commands.add_parser("pending", help="add the builds that a timing run put off")
    commands.add_parser("status", help="the store, its transactions and their pins")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.command == "add":
        print(add(args.name, args.binaries, args.pdb_dir))
    elif args.command == "pending":
        print(add(None, []))
    elif args.command == "status":
        try:
            store = symbol_store()
        except ImportError as error:
            print("symbols: no harness at %s: %s" % (HARNESS_DIR, error))
            return 1
        print(status(store, BUDGET, KEEP_LIST) if store else "symbols: the harness at %s names no SYMBOL_STORE" % HARNESS_DIR)
    else:
        parser.error("it needs a command, or --self-test")
    return 0


if __name__ == "__main__":
    sys.exit(main())
