"""The expiry of the data rule (D71, D152): one rule for each run folder, dump and set, wherever it lives.
Without `--apply` it only lists.

    python tools/clean_results.py                 # the sums of each place
    python tools/clean_results.py --list          # and each run, dump and set by name
    python tools/clean_results.py --apply         # trim, compress and remove what the list names
    python tools/clean_results.py --during-game   # the list while a game runs: the walk reads the whole
                                                  # disk, so never in a timing
    python -I tools/clean_results.py --self-test  # the pure parts and a real junction

A run folder `<YYYYmmdd-HHMMSS>-<label>` lives in `results/` of a checkout, in E:\\nirender-results or in
E:\\nirender-keep\\runs-*. Each gets one of five treatments:

- kept: the keep list E:\\nirender-keep\\keep-runs.txt names it, with a reason and a date (clean_common.py has
  the form). It stays whole.
- running: younger than one hour. Not touched.
- young: younger than 7 days. It stays whole.
- trim: a cited run of 7 days or more, or an uncited run of 7 to 30 days. The pictures, the captures and the
  film go, all else stays, and `trimmed.txt` lists what went. A cited run stays trimmed: its text does not
  expire.
- expired: an uncited run of 30 days or more. Removed.

A run is cited when the docs, the notes of the sessions, the check cards, a commit message or the text of a
named folder of E:\\nirender-keep names it, or a dump inside it. A full citation counts by its date and time, a
short one by `-HHMMSS-` alone. A run's own log and a copy of a run are no citation. A citation keeps the text of
a run, not its pictures: a record that needs the pictures puts the run on the keep list.

`--apply` refuses when the text may lack a citation (a folder or a file that cannot be read, a repository with
no log, or too little text in all) and when a line of the keep list has no run's name, reason or date. An
uncited run whose label the text holds is listed for the Archivist: its label alone keeps it from nothing.

A `results` that is a link is not followed: its target is a place of its own or none. Each folder that is
no run stays: `baseline-records`, the bench captures, the named folders of E:\\nirender-keep.

A dump in C:\\tmp\\mwdumps or E:\\nirender-keep\\full-dumps-*:

- It is compressed with NTFS LZX after one day with no write. One dump of 1.22 GB: 13.3 % of its size in 10 s,
  and cdb reads it as it is (zlib: 14.1 %, and cdb needs it unpacked). E: holds the compressed attribute,
  and that form gives about 50 %, so it goes first.
- It is removed at 30 days, unless the text names it, its process beside the word dump or pid, or its folder.
- A dump in another folder of E:\\nirender-keep is only listed.

A set of E:\\nirender-keep\\mgebuilds stays when local.bat names it or it is the one before, when a process
uses it, when the text or the name of a baseline-* or gate-* folder names it, or when the keep list names it.
Others go 30 days after their newest write. When local.bat names no set that is here, no set goes.

The tool refuses while a game or a helper runs. A removal never goes through a link. Each act goes to
E:\\nirender-keep\\clean-worktrees.log.
"""
import argparse
import ctypes
import glob
import os
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
# `python -I` leaves the folder of the script off the path.
sys.path.insert(0, HERE)
import clean_common as common  # noqa: E402
from clean_common import DUMP_DAYS, KEEP_ROOT, SETS, Evidence, current_set, files_in, games_in, gigabytes, is_heavy, is_link, is_link_entry, kept_set_names, log_line, long_path, processes, project_trees, remove_file, remove_tree, run_age_days, run_treatment, set_number, set_treatment, users_of, walk  # noqa: E402

RUN_PLACES = ["E:\\nirender-results"]
DUMP_PLACES = ["C:\\tmp\\mwdumps", common.KEPT_DUMPS]
# The folders of dumps by their kind, which no citation of the name of the place can keep.
PLAIN_DUMP_ROOTS = {"mwdumps", os.path.basename(common.KEPT_DUMPS)}
COMPRESS_DAYS = 1
SMALL_DUMP = 1 << 20
FILE_ATTRIBUTE_COMPRESSED = 0x800
TREATMENTS = ("kept", "running", "young", "trim", "expired")


def run_roots():
    """The folders that hold runs, and the `results` links that are not followed."""
    roots, links = [], []
    candidates = [os.path.join(path, "results") for path, _ in project_trees()]
    candidates += RUN_PLACES + sorted(glob.glob(os.path.join(KEEP_ROOT, "runs-*")))
    for path in candidates:
        if not os.path.lexists(path):
            continue
        if is_link(path):
            links.append((path, os.path.realpath(path)))
        elif os.path.isdir(path):
            roots.append(path)
    return roots, links


def plan_root(root, evidence, now):
    """Each run of a folder with its treatment: dicts with `name`, `path`, `treatment`, `size`, `files`
    and `heavy`, the files that go when it is trimmed. A folder that could not be read stays."""
    rows = []
    with os.scandir(root) as entries:
        found = sorted(entries, key=lambda entry: entry.name)
    for entry in found:
        age = run_age_days(entry.name, now)
        if age is None or is_link_entry(entry) or not entry.is_dir(follow_symlinks=False):
            continue
        try:
            files = files_in(entry.path)
        except OSError as error:
            print("not read, and it stays: %s (%s)" % (entry.path, error))
            continue
        cited = evidence.cites_run_with(entry.name, files)
        treatment = run_treatment(age, cited, evidence.keeps(entry.path))
        rows.append({
            "name": entry.name,
            "path": entry.path,
            "treatment": treatment,
            "cited": cited,
            "size": sum(file[1] for file in files),
            "files": files,
            "heavy": [file for file in files if is_heavy(file[0])] if treatment == "trim" else [],
        })
    return rows


def freed_by(row):
    if row["treatment"] == "trim":
        return sum(file[1] for file in row["heavy"])
    return row["size"] if row["treatment"] == "expired" else 0


def trim_run(row):
    """Removes the heavy files of a run and lists them in its `trimmed.txt`. Returns the failures."""
    gone, failures = [], []
    for path, _, _ in row["heavy"]:
        try:
            remove_file(path)
            gone.append(os.path.relpath(path, row["path"]))
        except OSError as error:
            failures.append((path, str(error)))
    with open(long_path(os.path.join(row["path"], "trimmed.txt")), "a", encoding="ascii", errors="replace") as note:
        until = "the run is cited, so they do not expire" if row["cited"] else "until the run expires"
        note.write("trimmed by tools/clean_results.py on %s: the pictures, captures and film went; the text and dumps stay, %s\n" % (time.strftime("%Y-%m-%d %H:%M"), until))
        note.writelines(path + "\n" for path in gone)
    return failures


def act_on_root(rows):
    """The acts of one folder of runs. Returns the number of failures."""
    failed = 0
    running, _ = processes()
    for row in rows:
        if row["treatment"] != "expired" and not row["heavy"]:
            continue
        users = users_of(row["path"], running)
        if users:
            log_line("NOT TOUCHED, in use: %s (%s)" % (row["path"], ", ".join(users[:3])), True)
            failed += 1
            continue
        if row["treatment"] == "trim":
            failures = trim_run(row)
            log_line("run trimmed: %s, %s freed" % (row["path"], gigabytes(freed_by(row))), True)
        else:
            links, failures = remove_tree(row["path"])
            log_line("run removed: %s, %s" % (row["path"], gigabytes(row["size"])), True)
            for link, target in links:
                log_line("  link removed: %s (its target %s stays)" % (link, target), True)
        for path, error in failures:
            log_line("  NOT REMOVED: %s (%s)" % (path, error), True)
        failed += len(failures)
    return failed


# ---- the dumps


def on_disk(path):
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.GetCompressedFileSizeW.restype = ctypes.c_ulong
    kernel.GetCompressedFileSizeW.argtypes = [ctypes.c_wchar_p, ctypes.POINTER(ctypes.c_ulong)]
    high = ctypes.c_ulong()
    low = kernel.GetCompressedFileSizeW(long_path(path), ctypes.byref(high))
    return (high.value << 32) + low


def needs_compressing(size, disk_size, has_attribute):
    """True for a dump that is not packed yet, or that is packed in the weak form of the attribute.
    LZX leaves no attribute, and shows by its size on disk."""
    if size < SMALL_DUMP:
        return False
    return has_attribute or disk_size > 0.9 * size


def dump_treatment(age_days, cited, compress, run=None):
    """What happens to a dump now: "remove", "compress" or "stay". `run`, the treatment of the run whose
    pointer file names the dump: the dump goes with its run, and stays as long as the run does."""
    if run is not None:
        if run == "expired" and not cited:
            return "remove"
        cited = True
    if age_days >= DUMP_DAYS and not cited:
        return "remove"
    if compress and age_days >= COMPRESS_DAYS:
        return "compress"
    return "stay"


def dump_owners(plans):
    """The runs that the pointer files name as the owners of dumps: {the dump's path, case folded: (run, treatment)}."""
    owners = {}
    for _, rows in plans:
        for row in rows:
            for path, _, _ in row["files"]:
                if os.path.basename(path).lower() != common.DUMPS_FILE:
                    continue
                with open(long_path(path), encoding="utf-8", errors="replace") as pointer:
                    for line in pointer.read().splitlines():
                        if line.strip():
                            owners[os.path.normcase(line.split("\t")[0])] = (row["name"], row["treatment"])
    return owners


def plan_dumps(places, evidence, now, owners=None):
    """Each dump of the places with its treatment. A dump that a run's pointer file names (`owners`) follows its run."""
    owners = owners or {}
    rows = []
    for place in places:
        if not os.path.isdir(place) or is_link(place):
            continue
        for path, size, written in files_in(place):
            if not path.lower().endswith(common.DUMP_FILES):
                continue
            parts = os.path.relpath(path, place).split(os.sep)[:-1] + [os.path.basename(place)]
            # A folder named after a run counts by the citations of the run, as the run itself does.
            cited = evidence.cites_dump(path) or any(part not in PLAIN_DUMP_ROOTS and (evidence.cites_word(part) or (run_age_days(part, now) is not None and evidence.cites_run(part))) for part in parts)
            run, run_treatment_now = owners.get(os.path.normcase(path), (None, None))
            disk_size = on_disk(path)
            attribute = bool(os.stat(long_path(path)).st_file_attributes & FILE_ATTRIBUTE_COMPRESSED)
            treatment = dump_treatment((now - written) / 86400, cited, needs_compressing(size, disk_size, attribute), run_treatment_now)
            rows.append({"path": path, "size": size, "disk": disk_size, "attribute": attribute, "cited": cited, "run": run, "treatment": treatment})
    return rows


def compress_dump(row):
    """Packs a dump with LZX: first without the attribute, as LZX passes over a file that has it. Raises OSError."""
    for switches in ([["/u"]] if row["attribute"] else []) + [["/c", "/exe:lzx"]]:
        result = subprocess.run(["compact", *switches, row["path"]], capture_output=True, text=True, errors="replace")
        if result.returncode != 0:
            raise OSError("compact %s: %s" % (" ".join(switches), result.stdout.strip()[-200:]))
    return on_disk(row["path"])


def act_on_dumps(rows):
    failed = 0
    for row in rows:
        try:
            if row["treatment"] == "remove":
                remove_file(row["path"])
                log_line("dump removed: %s, %s" % (row["path"], gigabytes(row["size"])), True)
            elif row["treatment"] == "compress":
                log_line("dump compressed: %s, %s on disk from %s" % (row["path"], gigabytes(compress_dump(row)), gigabytes(row["disk"])), True)
        except OSError as error:
            log_line("  NOT DONE: %s (%s)" % (row["path"], error), True)
            failed += 1
    return failed


def other_dumps():
    """The dumps in the other folders of E:\\nirender-keep: only a list, as these folders are not touched."""
    rows = []
    if not os.path.isdir(KEEP_ROOT):
        return rows
    with os.scandir(KEEP_ROOT) as entries:
        found = [entry for entry in entries if not is_link_entry(entry) and entry.is_dir(follow_symlinks=False)]
    for entry in found:
        if entry.path in (SETS, common.KEPT_DUMPS) or entry.name.startswith(("runs-", "full-dumps-")):
            continue
        for path, size, _ in files_in(entry.path):
            if path.lower().endswith(common.DUMP_FILES):
                rows.append((size, path))
    return sorted(rows, reverse=True)


# ---- the sets


def plan_sets(sets, evidence, current, running, now):
    if not os.path.isdir(sets):
        return []
    with os.scandir(sets) as entries:
        found = sorted((entry for entry in entries if not is_link_entry(entry) and entry.is_dir(follow_symlinks=False)), key=lambda entry: (set_number(entry.name) or 0, entry.name))
    names = [entry.name for entry in found]
    current_known = current in names
    kept = kept_set_names(current, names) if current_known else set()
    rows = []
    for entry in found:
        size, _, newest, links, unread = walk(entry.path)
        treatment = set_treatment((now - newest) / 86400 if newest else 0, evidence.cites_word(entry.name), entry.name in kept, users_of(entry.path, running), current_known)
        if treatment == "go" and evidence.keeps(entry.path):
            treatment = "stay: on the keep list"
        if unread and treatment == "go":
            treatment = "stay: %d folders could not be read" % unread
        rows.append({"name": entry.name, "path": entry.path, "size": size, "links": links, "treatment": treatment})
    return rows


def act_on_sets(rows):
    failed = 0
    for row in rows:
        if row["treatment"] != "go":
            continue
        running, _ = processes()
        users = users_of(row["path"], running)
        if users:
            log_line("NOT TOUCHED, in use: %s (%s)" % (row["path"], ", ".join(users[:3])), True)
            failed += 1
            continue
        links, failures = remove_tree(row["path"])
        log_line("set removed: %s, %s" % (row["path"], gigabytes(row["size"])), True)
        for link, target in links:
            log_line("  link removed: %s (its target %s stays)" % (link, target), True)
        for path, error in failures:
            log_line("  NOT REMOVED: %s (%s)" % (path, error), True)
        failed += len(failures)
    return failed


# ---- the report


def print_runs(plans, links, names):
    print("RUNS:")
    sums = {treatment: [0, 0, 0] for treatment in TREATMENTS}
    for root, rows in plans:
        parts = []
        for treatment in TREATMENTS:
            chosen = [row for row in rows if row["treatment"] == treatment]
            size, freed = sum(row["size"] for row in chosen), sum(freed_by(row) for row in chosen)
            sums[treatment][0] += len(chosen)
            sums[treatment][1] += size
            sums[treatment][2] += freed
            if chosen:
                parts.append("%s %d" % (treatment, len(chosen)))
        total = sum(row["size"] for row in rows)
        print("  %9s  %s: %d runs (%s); %s freed" % (gigabytes(total), root, len(rows), ", ".join(parts) or "none", gigabytes(sum(freed_by(row) for row in rows))))
        if names:
            for row in rows:
                print("      %-8s %9.1f MB  %s" % (row["treatment"], row["size"] / 1e6, row["name"]))
    for path, target in links:
        print("  a link, not followed: %s -> %s" % (path, target))
    print("  in all:")
    for treatment in TREATMENTS:
        count, size, freed = sums[treatment]
        print("    %-8s %5d runs  %9s  %9s freed" % (treatment, count, gigabytes(size), gigabytes(freed)))


def label_names(plans, evidence):
    """The names of the uncited runs whose label the text holds, sorted. A run of the keep list is not one of them: the
    list keeps it whole, so a citation adds nothing (W359)."""
    return sorted({row["name"] for _, rows in plans for row in rows if row["treatment"] != "kept" and evidence.label_only(row["name"])})


def print_labels(plans, evidence):
    """The uncited runs whose label the text holds, for the Archivist (`label_names`): a citation by its time keeps each one."""
    names = label_names(plans, evidence)
    print("\nUNCITED RUNS WHOSE LABEL THE TEXT HOLDS, for the Archivist (%d):" % len(names))
    for name in names:
        print("  " + name)


def print_dumps(rows, others):
    print("\nDUMPS (%d, %s):" % (len(rows), gigabytes(sum(row["size"] for row in rows))))
    for row in sorted(rows, key=lambda r: r["path"]):
        owner = "  (the run %s)" % row["run"] if row.get("run") else ""
        print("  %-8s %-5s %9s, %9s on disk  %s%s" % (row["treatment"], "cited" if row["cited"] else "", gigabytes(row["size"]), gigabytes(row["disk"]), row["path"], owner))
    print("\nDUMPS IN OTHER FOLDERS OF %s, only listed (%d, %s):" % (KEEP_ROOT, len(others), gigabytes(sum(size for size, _ in others))))
    for size, path in others:
        print("  %9s  %s" % (gigabytes(size), path))


def print_sets(rows, names):
    going = [row for row in rows if row["treatment"] == "go"]
    print("\nSETS in %s (%d, %s); %d go, %s freed:" % (SETS, len(rows), gigabytes(sum(row["size"] for row in rows)), len(going), gigabytes(sum(row["size"] for row in going))))
    for row in rows:
        if names or row["treatment"] == "go":
            print("  %9s  %-10s %s" % (gigabytes(row["size"]), row["name"], row["treatment"]))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--apply", action="store_true", help="trim, compress and remove; without it the tool only lists")
    parser.add_argument("--list", action="store_true", help="name each run and each set")
    parser.add_argument("--during-game", action="store_true", help="the list while a game runs; --apply still refuses")
    parser.add_argument("--self-test", action="store_true", help="the tests of the pure parts and of a real junction")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    running, _ = processes()
    games = games_in(running)
    if games and (args.apply or not args.during_game):
        print("NOTHING DONE: a game or a helper runs: " + ", ".join(games))
        print("A run writes into its results, and the walk would disturb a timing. --during-game gives the list anyway.")
        return 1
    now = time.time()
    evidence = Evidence.load()
    print("The evidence: %.1f MB of text, %d full and %d short citations; %d runs on the keep list %s." % (len(evidence.text) / 1e6, len(evidence.stamps), len(evidence.shorts), len(evidence.kept), common.KEEP_LIST))
    for problem in evidence.problems:
        print("THE EVIDENCE IS NOT WHOLE: " + problem)
    if evidence.problems and args.apply:
        print("NOTHING DONE: a run may lose what a citation or the keep list keeps.")
        return 1
    roots, links = run_roots()
    plans = [(root, plan_root(root, evidence, now)) for root in roots]
    dumps = plan_dumps(DUMP_PLACES + sorted(glob.glob(os.path.join(KEEP_ROOT, "full-dumps-*"))), evidence, now, dump_owners(plans))
    current = current_set()
    sets = plan_sets(SETS, evidence, current, running, now)
    print_runs(plans, links, args.list)
    print_labels(plans, evidence)
    print_dumps(dumps, other_dumps())
    print("\nlocal.bat names the set %s" % current)
    print_sets(sets, args.list)
    freed = sum(freed_by(row) for _, rows in plans for row in rows) + sum(row["size"] for row in dumps if row["treatment"] == "remove") + sum(row["size"] for row in sets if row["treatment"] == "go")
    print("\nThe removals free %s, and the compression more." % gigabytes(freed))
    if not args.apply:
        print("A dry run: nothing changed. --apply acts.")
        return 0
    failed = sum(act_on_root(rows) for _, rows in plans)
    failed += act_on_dumps(dumps)
    failed += act_on_sets(sets)
    return 1 if failed else 0


def self_test():
    common.self_test_common()
    assert dump_treatment(40, False, True) == "remove" and dump_treatment(40, True, True) == "compress" and dump_treatment(40, True, False) == "stay"
    assert dump_treatment(5, False, True) == "compress" and dump_treatment(0.5, False, True) == "stay"
    assert needs_compressing(1 << 30, 1 << 29, True) and needs_compressing(1 << 30, 1 << 30, False)
    assert not needs_compressing(1 << 30, 1 << 27, False) and not needs_compressing(1 << 10, 1 << 10, False)
    now = time.time()

    def stamp(days):
        return time.strftime("%Y%m%d-%H%M%S", time.localtime(now - days * 86400))

    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp:
        common.LOG = os.path.join(temp, "test.log")
        root = os.path.join(temp, "results")
        names = {"running": stamp(0.01) + "-a", "young": stamp(3) + "-b", "trim": stamp(10) + "-c", "expired": stamp(40) + "-d", "cited": stamp(50) + "-e", "kept": stamp(55) + "-k"}
        # A cited run past 7 days trims and does not expire; only the keep list keeps a run whole (D152).
        expected = {kind: "trim" if kind == "cited" else kind for kind in names}
        for name in names.values():
            os.makedirs(os.path.join(root, name, "sub"))
            for file in ("a.png", "b.nircap", "c.txt", "d.dmp", os.path.join("sub", "e.log"), os.path.join("sub", "f.png"), "g.ess"):
                with open(os.path.join(root, name, file), "w") as out:
                    out.write("x" * 100)
        # A folder that is no run, and a run's name that is a link: neither is touched.
        os.makedirs(os.path.join(root, "bench"))
        with open(os.path.join(root, "bench", "keep.png"), "w") as out:
            out.write("y")
        outside = os.path.join(temp, "outside")
        os.makedirs(outside)
        with open(os.path.join(outside, "precious.txt"), "w") as out:
            out.write("z" * 1000)
        link_run = os.path.join(root, stamp(60) + "-link")
        subprocess.run(["cmd", "/c", "mklink", "/J", link_run, outside], capture_output=True, check=True)
        # A junction inside a run that goes: the removal takes the link and leaves its target.
        subprocess.run(["cmd", "/c", "mklink", "/J", os.path.join(root, names["expired"], "results"), outside], capture_output=True, check=True)
        subprocess.run(["cmd", "/c", "mklink", "/J", os.path.join(root, names["trim"], "results"), outside], capture_output=True, check=True)

        evidence = Evidence("see %s in a record" % names["cited"], kept={os.path.join(root, names["kept"]).replace("\\", "/").upper()})
        rows = plan_root(root, evidence, now)
        by_name = {row["name"]: row for row in rows}
        assert len(rows) == 6, rows
        for kind, name in names.items():
            assert by_name[name]["treatment"] == expected[kind], (kind, by_name[name]["treatment"])
        for kind in ("trim", "cited"):
            assert sorted(os.path.basename(file[0]) for file in by_name[names[kind]]["heavy"]) == ["a.png", "b.nircap", "f.png"], kind
            assert freed_by(by_name[names[kind]]) == 300, kind
        assert freed_by(by_name[names["expired"]]) == 700 and freed_by(by_name[names["young"]]) == 0 and freed_by(by_name[names["kept"]]) == 0
        before = {name: sorted(file[0] for file in files_in(os.path.join(root, name))) for name in names.values()}
        assert act_on_root(rows) == 0, open(common.LOG).read()
        assert not os.path.lexists(os.path.join(root, names["expired"]))
        for name in (names["running"], names["young"], names["kept"]):
            assert sorted(file[0] for file in files_in(os.path.join(root, name))) == before[name], name
        for kind, until in (("trim", "until the run expires"), ("cited", "do not expire")):
            trimmed = os.path.join(root, names[kind])
            left = sorted(["c.txt", "d.dmp", os.path.join("sub", "e.log"), "g.ess", "trimmed.txt"])
            assert sorted(os.path.relpath(file[0], trimmed) for file in files_in(trimmed)) == left, kind
            note = open(os.path.join(trimmed, "trimmed.txt")).read()
            assert "a.png" in note and until in note, (kind, note)
        trimmed = os.path.join(root, names["trim"])
        assert os.path.lexists(os.path.join(trimmed, "results")), "a trim leaves a link"
        assert os.path.getsize(os.path.join(outside, "precious.txt")) == 1000
        assert os.path.exists(os.path.join(root, "bench", "keep.png")) and os.path.lexists(link_run)
        # A second pass has nothing to do: the trims are done, and the notes do not grow. The cited run stays
        # trimmed at any age, and the kept run whole.
        notes = {kind: os.path.getsize(os.path.join(root, names[kind], "trimmed.txt")) for kind in ("trim", "cited")}
        assert act_on_root(plan_root(root, evidence, now)) == 0
        assert all(os.path.getsize(os.path.join(root, names[kind], "trimmed.txt")) == size for kind, size in notes.items())
        later = {row["name"]: row["treatment"] for row in plan_root(root, evidence, now + 400 * 86400)}
        assert later[names["cited"]] == "trim" and later[names["kept"]] == "kept" and later[names["trim"]] == "expired", later
        # The null: without its line on the keep list, the kept run is an uncited old run, and goes.
        assert {row["name"]: row["treatment"] for row in plan_root(root, Evidence(""), now)}[names["kept"]] == "expired"

        # W359: the list of uncited runs whose label the text holds leaves out a run of the keep list. Both runs below
        # have their label in the text and no time; only the one off the list is named. The null: the same run without
        # its line on the list is named.
        labelled = {"kept": stamp(70) + "-sky-pair-kept", "other": stamp(71) + "-sky-pair-other"}
        for name in labelled.values():
            os.makedirs(os.path.join(root, name))
        text = "the runs sky-pair-kept and sky-pair-other"
        kept_line = {os.path.join(root, labelled["kept"]).replace("\\", "/").upper()}
        listed = label_names([(root, plan_root(root, Evidence(text, kept=kept_line), now))], Evidence(text, kept=kept_line))
        assert listed == [labelled["other"]], listed
        unlisted = label_names([(root, plan_root(root, Evidence(text), now))], Evidence(text))
        assert unlisted == sorted(labelled.values()), unlisted

        # A dump that a record names cites its old run, though the record does not name the run: the run stays trimmed.
        with_dump = stamp(45) + "-f"
        os.makedirs(os.path.join(root, with_dump))
        with open(os.path.join(root, with_dump, "Morrowind.exe.777.dmp"), "w") as out:
            out.write("d")
        kept_run = {row["name"]: row for row in plan_root(root, Evidence("the file Morrowind.exe.777.dmp"), now)}[with_dump]
        assert kept_run["treatment"] == "trim" and kept_run["cited"], kept_run["treatment"]

        # A real LZX, on a dump in a folder of its own.
        place = os.path.join(temp, "full-dumps-x")
        os.makedirs(os.path.join(place, "cited-sub"))
        big = os.path.join(place, "Morrowind.exe.100.dmp")
        with open(big, "wb") as out:
            out.write(b"\0" * (1 << 21))
        old = now - 3 * 86400
        os.utime(big, (old, old))
        gone = os.path.join(place, "Morrowind.exe.200.dmp")
        kept = os.path.join(place, "Morrowind.exe.300.dmp")
        sub = os.path.join(place, "cited-sub", "Morrowind.exe.400.dmp")
        for path in (gone, kept, sub):
            with open(path, "wb") as out:
                out.write(b"\0" * 10)
            os.utime(path, (now - 40 * 86400, now - 40 * 86400))
        with open(os.path.join(place, "notes.txt"), "w") as out:
            out.write("a note")
        os.utime(os.path.join(place, "notes.txt"), (now - 40 * 86400, now - 40 * 86400))
        rows = plan_dumps([place, os.path.join(temp, "none")], Evidence("Morrowind.exe.300.dmp and the folder cited-sub"), now)
        treatments = {os.path.basename(row["path"]): row["treatment"] for row in rows}
        assert treatments == {"Morrowind.exe.100.dmp": "compress", "Morrowind.exe.200.dmp": "remove", "Morrowind.exe.300.dmp": "stay", "Morrowind.exe.400.dmp": "stay"}, treatments
        assert act_on_dumps(rows) == 0, open(common.LOG).read()
        assert not os.path.exists(gone) and os.path.exists(kept) and os.path.exists(sub) and os.path.exists(os.path.join(place, "notes.txt"))
        assert on_disk(big) < os.path.getsize(big) / 2 and os.path.getsize(big) == 1 << 21, on_disk(big)
        with open(big, "rb") as check:
            assert check.read(16) == b"\0" * 16
        assert not any(row["treatment"] == "compress" for row in plan_dumps([place], Evidence(""), now)), "a dump packed with LZX is left alone"
        # The name of the place itself counts, unless it says only what kind of place it is.
        cited_place = plan_dumps([place], Evidence("full-dumps-x"), now + 40 * 86400)
        assert all(row["treatment"] != "remove" for row in cited_place), cited_place
        plain = os.path.join(temp, "mwdumps")
        os.makedirs(plain)
        shutil.copy2(kept, os.path.join(plain, "Morrowind.exe.500.dmp"))
        assert plan_dumps([plain], Evidence("mwdumps"), now)[0]["treatment"] == "remove"

        # The dumps of the runs on E: go with their runs, by the runs' pointer files.
        assert dump_treatment(40, False, True, "expired") == "remove" and dump_treatment(2, False, True, "expired") == "remove"
        assert dump_treatment(40, True, False, "expired") == "stay"
        assert all(dump_treatment(40, False, False, run) == "stay" for run in ("kept", "running", "young", "trim"))
        assert dump_treatment(40, False, True, "trim") == "compress"
        runs, kept_dumps = os.path.join(temp, "pointer-runs"), os.path.join(temp, "dumps")
        # Each run at its own time of day: a short citation counts by the time alone.
        names = {"expired": stamp(45.01) + "-old", "young": stamp(2.02) + "-new", "cited": stamp(46.03) + "-kept", "trim": stamp(10.04) + "-trim"}
        targets = {}
        for kind, name in names.items():
            os.makedirs(os.path.join(runs, name))
            os.makedirs(os.path.join(kept_dumps, name))
            target = os.path.join(kept_dumps, name, "Morrowind.exe.%d.dmp" % (len(targets) + 10))
            with open(target, "wb") as out:
                out.write(b"\0" * 10)
            os.utime(target, (now - 40 * 86400, now - 40 * 86400))
            with open(os.path.join(runs, name, common.DUMPS_FILE), "w", encoding="utf-8") as out:
                out.write("%s\t10\tC:\\tmp\\mwdumps (Windows Error Reporting)\n" % target.upper())
            targets[kind] = target
        # Two dumps that no pointer names: one in the folder of a cited run that is gone, one of no cited run.
        orphans = {"cited": stamp(50.05) + "-gone", "uncited": stamp(51.06) + "-lost"}
        for kind, name in orphans.items():
            os.makedirs(os.path.join(kept_dumps, name))
            target = os.path.join(kept_dumps, name, "MWSE_MiniDump.dmp")
            with open(target, "wb") as out:
                out.write(b"\0" * 10)
            os.utime(target, (now - 40 * 86400, now - 40 * 86400))
            targets["orphan " + kind] = target
        evidence = Evidence("a record of %s, and the short citation %s" % (names["cited"], orphans["cited"][8:16]))
        plans = [(runs, plan_root(runs, evidence, now))]
        # The cited run is past 7 days: it is trimmed, and its dump stays with it.
        assert {row["name"]: row["treatment"] for row in plans[0][1]} == {names[kind]: "trim" if kind == "cited" else kind for kind in names}, plans
        owners = dump_owners(plans)
        assert set(owners) == {os.path.normcase(targets[kind]) for kind in names}, owners
        rows = {row["path"]: row for row in plan_dumps([kept_dumps], evidence, now, owners)}
        treatments = {kind: rows[target]["treatment"] for kind, target in targets.items()}
        assert treatments == {"expired": "remove", "young": "stay", "cited": "stay", "trim": "stay", "orphan cited": "stay", "orphan uncited": "remove"}, treatments
        assert rows[targets["young"]]["run"] == names["young"] and rows[targets["orphan uncited"]]["run"] is None
        # The null: without the pointers the dumps of the young and the trimmed run would go.
        bare = {row["path"]: row["treatment"] for row in plan_dumps([kept_dumps], evidence, now)}
        assert bare[targets["young"]] == "remove" and bare[targets["trim"]] == "remove" and bare[targets["cited"]] == "stay", bare
        # The runs' dumps are no evidence, and no other folder lists them.
        assert common.KEPT_DUMPS not in common.evidence_folders() and os.path.basename(common.KEPT_DUMPS) in PLAIN_DUMP_ROOTS
        assert common.KEPT_DUMPS in DUMP_PLACES

        # The sets.
        sets = os.path.join(temp, "mgebuilds")
        for name in ("f100", "f101", "f102", "f193", "f194", "f195", "vd-a"):
            os.makedirs(os.path.join(sets, name))
            with open(os.path.join(sets, name, "d3d8.dll"), "w") as out:
                out.write("x")
            os.utime(os.path.join(sets, name, "d3d8.dll"), (now - 40 * 86400, now - 40 * 86400))
        os.utime(os.path.join(sets, "f102", "d3d8.dll"), (now, now))
        subprocess.run(["cmd", "/c", "mklink", "/J", os.path.join(sets, "f101", "link"), outside], capture_output=True, check=True)
        running = [{"pid": 7, "name": "x.exe", "exe": os.path.join(sets, "f193", "x.exe"), "command": "", "cwd": ""}]
        rows = plan_sets(sets, Evidence("a record names f100"), "f195", running, now)
        treatments = {row["name"]: row["treatment"] for row in rows}
        assert treatments["f100"] == "stay: cited" and treatments["f101"] == "go" and "younger" in treatments["f102"], treatments
        assert treatments["f193"] == "stay: in use" and "the one before" in treatments["f194"] and "the one before" in treatments["f195"] and treatments["vd-a"] == "go", treatments
        # A current set with no number stays by its name. With no current set known, no set goes.
        by_name_rows = {row["name"]: row["treatment"] for row in plan_sets(sets, Evidence(""), "vd-a", [], now)}
        assert "the current" in by_name_rows["vd-a"] and "the current" in by_name_rows["f195"] and by_name_rows["f100"] == "go", by_name_rows
        for unknown in (None, "f999"):
            assert all(row["treatment"].startswith("stay") for row in plan_sets(sets, Evidence(""), unknown, [], now)), unknown
        # A set on the keep list stays, and the others go as before.
        listed = {row["name"]: row["treatment"] for row in plan_sets(sets, Evidence("a record names f100", kept={os.path.join(sets, "vd-a")}), "f195", running, now)}
        assert listed["vd-a"] == "stay: on the keep list" and listed["f101"] == "go", listed
        assert act_on_sets(rows) == 0, open(common.LOG).read()
        assert not os.path.exists(os.path.join(sets, "f101")) and not os.path.exists(os.path.join(sets, "vd-a")) and os.path.exists(os.path.join(sets, "f100"))
        assert os.path.getsize(os.path.join(outside, "precious.txt")) == 1000

        # A file that cannot go is said, and what could go is gone.
        tree = os.path.join(temp, "held")
        os.makedirs(tree)
        with open(os.path.join(tree, "b.txt"), "w") as out:
            out.write("x")
        held = os.path.join(tree, "a.txt")
        with open(held, "w") as out:
            out.write("x")
            links, failures = remove_tree(tree)
        assert links == [] and any("a.txt" in path for path, _ in failures), failures
        assert os.path.exists(held) and not os.path.exists(os.path.join(tree, "b.txt"))
    print("clean_results self-test: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
