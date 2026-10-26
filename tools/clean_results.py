"""Removes old run folders of `results/` until the folder is under a size limit.

The runs of `run_own_device.py` keep every picture, log and capture in
`results/<YYYYmmdd-HHMMSS>-<label>/`, and the folder grows by gigabytes a day. This tool lists the
run folders, keeps some, and removes the oldest of the rest until the whole folder is at most
`--limit-gb` large. It keeps:

- the runs younger than `--keep-days` days;
- the runs named in the docs (`--docs`, the moreFPS docs next to this repo): the records, the
  progress pages and the notes cite runs as evidence;
- everything that is not a run folder (`baseline-records` with the gate's accepted pictures,
  the loose outputs).

Without `--delete` it only lists. With `--delete` it asks for "yes" first (or `--yes`).

    python tools/clean_results.py                      # the list: kept, to remove, sizes
    python tools/clean_results.py --limit-gb 30         # another limit
    python tools/clean_results.py --delete              # remove, after "yes"
"""

import argparse
import os
import re
import shutil
import stat
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
RESULTS = os.path.join(HERE, "..", "results")
DOCS = os.path.join(HERE, "..", "..", "moreFPS", "docs")
RUN = re.compile(r"^(\d{8}-\d{6})-.+$")
TEXT = (".md", ".tsv", ".txt", ".py", ".sh", ".lua", ".json", ".csv")
GB = 1024 ** 3


def size_of(folder):
    total = 0
    for root, _, names in os.walk(folder):
        for name in names:
            try:
                total += os.path.getsize(os.path.join(root, name))
            except OSError:
                pass
    return total


def named_in_docs(docs, names):
    """The run folders whose name a text file of the docs holds."""
    found = set()
    pattern = re.compile(r"\d{8}-\d{6}-[\w.-]+")
    for root, dirs, files in os.walk(docs):
        dirs[:] = [d for d in dirs if d != ".git"]
        for file in files:
            if not file.lower().endswith(TEXT):
                continue
            try:
                with open(os.path.join(root, file), encoding="utf-8", errors="replace") as f:
                    for match in pattern.findall(f.read()):
                        # A cited name can carry a path or a trailing dot: match the prefix.
                        for name in names:
                            if match == name or match.startswith(name):
                                found.add(name)
            except OSError:
                pass
    return found


def remove(folder):
    def writable(function, path, _):
        os.chmod(path, stat.S_IWRITE)
        function(path)

    shutil.rmtree(folder, onerror=writable)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--results", default=RESULTS)
    parser.add_argument("--docs", default=DOCS, help="the docs whose run names are kept")
    parser.add_argument("--limit-gb", type=float, default=40.0, help="the size of the folder after the removal (40 by default)")
    parser.add_argument("--keep-days", type=float, default=3.0, help="runs younger than this are kept (3 by default)")
    parser.add_argument("--delete", action="store_true", help="remove; without it the tool only lists")
    parser.add_argument("--yes", action="store_true", help="no question before the removal")
    args = parser.parse_args()

    results = os.path.abspath(args.results)
    runs = sorted(name for name in os.listdir(results) if RUN.match(name) and os.path.isdir(os.path.join(results, name)))
    print("measuring %d run folders in %s ..." % (len(runs), results))
    sizes = {name: size_of(os.path.join(results, name)) for name in runs}
    total = size_of(results)
    cited = named_in_docs(args.docs, runs) if os.path.isdir(args.docs) else set()
    if not os.path.isdir(args.docs):
        print("no docs at %s: no run is kept for a citation" % args.docs)
    now = time.time()

    def age_days(name):
        stamp = time.mktime(time.strptime(RUN.match(name).group(1), "%Y%m%d-%H%M%S"))
        return (now - stamp) / 86400

    young = {name for name in runs if age_days(name) < args.keep_days}
    candidates = [name for name in runs if name not in young and name not in cited]
    limit = args.limit_gb * GB
    chosen, after = [], total
    for name in candidates:  # oldest first: the names start with the date
        if after <= limit:
            break
        chosen.append(name)
        after -= sizes[name]

    print("the folder: %.1f GB, %d run folders (%.1f GB); the rest is other files" % (total / GB, len(runs), sum(sizes.values()) / GB))
    print("kept: %d younger than %g days (%.1f GB), %d named in the docs (%.1f GB)" % (
        len(young), args.keep_days, sum(sizes[n] for n in young) / GB,
        len(cited - young), sum(sizes[n] for n in cited - young) / GB))
    print("to remove: %d runs, %.1f GB, the oldest first; the folder after: %.1f GB (limit %g GB)" % (
        len(chosen), (total - after) / GB, after / GB, args.limit_gb))
    if after > limit:
        print("the limit is not reached: the kept runs alone are larger (lower --keep-days, or remove cited runs by hand)")
    for name in chosen:
        print("  %8.1f MB  %s" % (sizes[name] / 2 ** 20, name))
    if not args.delete or not chosen:
        if chosen:
            print("nothing removed: run again with --delete")
        return
    if not args.yes:
        answer = input("remove these %d run folders (%.1f GB)? type yes: " % (len(chosen), (total - after) / GB))
        if answer.strip().lower() != "yes":
            print("nothing removed")
            return
    removed = 0
    for name in chosen:
        try:
            remove(os.path.join(results, name))
            removed += 1
        except OSError as error:
            print("not removed: %s (%s)" % (name, error))
    print("removed %d run folders; the folder is now %.1f GB" % (removed, size_of(results) / GB))


if __name__ == "__main__":
    sys.exit(main())
