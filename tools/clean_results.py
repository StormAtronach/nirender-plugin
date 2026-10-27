"""Keeps the run folders of `results/` that are evidence, the logs of recent runs, and nothing else.

The runs of `run_own_device.py` keep every picture, log and capture in
`results/<YYYYmmdd-HHMMSS>-<label>/`. Most runs are checks that nobody reads again; a few are
the evidence of a recorded result. Each run folder gets one of four treatments:

- cited: its name is in the docs (`--docs`) or in a commit message of the repos (`--repos`),
  where a record, a progress page or a commit gives it as evidence. Kept whole. The short form
  `-HHMMSS-label` (the records use it after a first full name of the same day) counts too.
- recent: younger than `--logs-days` days and not cited. Only its text files stay (the logs,
  the tables, the settings: the numbers can be read again); the pictures, captures and film
  frames go, and a file `trimmed.txt` lists them.
- old: older than that and not cited. Removed.
- running: younger than `--busy-hours` hours (a run can be in progress). Not touched.

Everything that is not a run folder (`baseline-records` with the gate's accepted pictures, the
bench captures, the loose outputs) is not touched.

Without `--delete` the tool only lists. With `--delete` it asks for "yes" first (or `--yes`).

    python tools/clean_results.py                 # the list
    python tools/clean_results.py --delete        # trim and remove, after "yes"
"""

import argparse
import os
import re
import shutil
import stat
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPOS_ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
RESULTS = os.path.join(HERE, "..", "results")
DOCS = os.path.join(REPOS_ROOT, "moreFPS", "docs")
REPOS = [os.path.join(REPOS_ROOT, name) for name in ("nirender-plugin", "MGE-XE-G7-render-link", os.path.join("MWSE", "MWSE"), "moreFPS")]
RUN = re.compile(r"^(\d{8}-\d{6})-.+$")
# A full run name, or the short form `-HHMMSS-label` with no word character before it.
CITATION = re.compile(r"\d{8}-\d{6}-[\w.-]+|(?<!\w)-\d{6}-[\w.-]+")
DOC_TEXT = (".md", ".tsv", ".txt", ".py", ".sh", ".lua", ".json", ".csv")
# The files of a run that stay when it is trimmed.
KEPT_TEXT = (".log", ".txt", ".tsv", ".toml", ".json", ".lua", ".csv", ".md")
GB = 1024 ** 3


def files_of(folder):
    for root, _, names in os.walk(folder):
        for name in names:
            yield os.path.join(root, name)


def size(paths):
    total = 0
    for path in paths:
        try:
            total += os.path.getsize(path)
        except OSError:
            pass
    return total


def citations(docs, repos):
    """The run names that the docs and the commit messages hold."""
    found = set()
    for root, dirs, files in os.walk(docs) if os.path.isdir(docs) else []:
        dirs[:] = [d for d in dirs if d != ".git"]
        for file in files:
            if file.lower().endswith(DOC_TEXT):
                try:
                    with open(os.path.join(root, file), encoding="utf-8", errors="replace") as f:
                        found.update(CITATION.findall(f.read()))
                except OSError:
                    pass
    for repo in repos:
        if os.path.isdir(repo):
            log = subprocess.run(["git", "-C", repo, "log", "--all", "--format=%B"], capture_output=True, text=True, encoding="utf-8", errors="replace")
            found.update(CITATION.findall(log.stdout))
    return found


def cited(name, names):
    # A citation can carry a trailing part (a file name, a dot): match its start. A short
    # citation matches the name without its date.
    short = name[8:]
    return any(text.startswith(short if text.startswith("-") else name) for text in names)


def writable(function, path, _):
    os.chmod(path, stat.S_IWRITE)
    function(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--results", default=RESULTS)
    parser.add_argument("--docs", default=DOCS, help="the docs whose run names are evidence")
    parser.add_argument("--repos", nargs="*", default=REPOS, help="the repos whose commit messages name evidence")
    parser.add_argument("--logs-days", type=float, default=7.0, help="uncited runs younger than this keep their logs (7 by default)")
    parser.add_argument("--busy-hours", type=float, default=1.0, help="runs younger than this are not touched (1 by default)")
    parser.add_argument("--delete", action="store_true", help="trim and remove; without it the tool only lists")
    parser.add_argument("--yes", action="store_true", help="no question first")
    parser.add_argument("--list", action="store_true", help="name every run of each treatment")
    args = parser.parse_args()

    results = os.path.abspath(args.results)
    runs = sorted(name for name in os.listdir(results) if RUN.match(name) and os.path.isdir(os.path.join(results, name)))
    names = citations(args.docs, args.repos)
    now = time.time()
    groups = {"cited": [], "running": [], "recent": [], "old": []}
    for name in runs:
        age = now - time.mktime(time.strptime(RUN.match(name).group(1), "%Y%m%d-%H%M%S"))
        if age < args.busy_hours * 3600:
            groups["running"].append(name)
        elif cited(name, names):
            groups["cited"].append(name)
        elif age < args.logs_days * 86400:
            groups["recent"].append(name)
        else:
            groups["old"].append(name)

    heavy = {}
    for name in groups["recent"]:
        heavy[name] = [p for p in files_of(os.path.join(results, name)) if not p.lower().endswith(KEPT_TEXT)]
    sizes = {name: size(files_of(os.path.join(results, name))) for name in runs}
    freed_trim = sum(size(paths) for paths in heavy.values())
    freed_old = sum(sizes[name] for name in groups["old"])
    total = size(files_of(results))
    print("%s: %.1f GB, %d run folders (%.1f GB)" % (results, total / GB, len(runs), sum(sizes.values()) / GB))
    print("  cited, kept whole:            %4d runs  %6.1f GB" % (len(groups["cited"]), sum(sizes[n] for n in groups["cited"]) / GB))
    print("  running, not touched:         %4d runs  %6.1f GB" % (len(groups["running"]), sum(sizes[n] for n in groups["running"]) / GB))
    print("  recent, trimmed to the logs:  %4d runs  %6.1f GB freed" % (len(groups["recent"]), freed_trim / GB))
    print("  old, removed:                 %4d runs  %6.1f GB freed" % (len(groups["old"]), freed_old / GB))
    print("the folder after: %.1f GB" % ((total - freed_trim - freed_old) / GB))
    if args.list:
        for group in ("cited", "running", "recent", "old"):
            for name in groups[group]:
                print("  %-8s %8.1f MB  %s" % (group, sizes[name] / 2 ** 20, name))
    if not args.delete:
        print("nothing changed: --list names the runs, --delete trims and removes")
        return
    if not args.yes:
        answer = input("trim %d runs and remove %d runs (%.1f GB)? type yes: " % (len(groups["recent"]), len(groups["old"]), (freed_trim + freed_old) / GB))
        if answer.strip().lower() != "yes":
            print("nothing changed")
            return
    for name, paths in heavy.items():
        if not paths:
            continue
        gone = []
        for path in paths:
            try:
                os.chmod(path, stat.S_IWRITE)
                os.remove(path)
                gone.append(os.path.relpath(path, os.path.join(results, name)))
            except OSError as error:
                print("not removed: %s (%s)" % (path, error))
        with open(os.path.join(results, name, "trimmed.txt"), "a", encoding="ascii", errors="replace") as note:
            note.write("trimmed by tools/clean_results.py on %s: the run is not cited; its logs stay\n" % time.strftime("%Y-%m-%d %H:%M"))
            note.writelines(path + "\n" for path in gone)
    for name in groups["old"]:
        try:
            shutil.rmtree(os.path.join(results, name), onerror=writable)
        except OSError as error:
            print("not removed: %s (%s)" % (name, error))
    print("done; the folder is now %.1f GB" % (size(files_of(results)) / GB))


if __name__ == "__main__":
    sys.exit(main())
