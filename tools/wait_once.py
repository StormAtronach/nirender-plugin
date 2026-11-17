"""Waits for a fact of a run by polling the run's own log (W557, the owner's D573: "poll every ten seconds or
so instead").

    python -I tools/wait_once.py --run <run folder> loaded
    python -I tools/wait_once.py --label <label> --after <time> live [--interval 10] [--timeout 300]
    python -I tools/wait_once.py --self-test

The fact: `loaded` (the harness's line "save loaded, cell"), `live` ("step sh-live: true") or any other text of a
line. Each poll reads the whole progress.log of the run, so a line that was written before the first poll is
found at once. An event wait sees only what comes after its start, and the bridge of mwse-mcp does not record
`loaded` at all: a session that waited for it sat in the game to the wait's limit.

The run: `--run` names its folder (the runner prints "results in <folder>" when it starts). `--label` with
`--after` takes the first folder `results/<stamp>-<label>` whose stamp is at or after the time: the session takes
the time before it starts the runner, so an older run of the label is never read. The stamp is local time; a time
with a zone is turned into local time first. `--label` without `--after` is refused.

The harness writes "save loaded" once for each run: this tool serves the first load of a run only. A load that
the session itself causes needs a mark taken before it (the skill morrowind-test-harness, mwse-mcp.md).

Exit codes: 0 the fact is there; 2 the time limit; 3 the run ended without it (the line "DONE" in progress.log,
or the runner's end line "pictures in" in runner-log.txt); 4 a refused call.
"""
import argparse
import datetime
import os
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
RESULTS = os.path.join(os.path.dirname(HERE), "results")
FACTS = {"loaded": "save loaded, cell", "live": "step sh-live: true"}
STAMP = "%Y%m%d-%H%M%S"
FOUND, TIMED_OUT, ENDED, REFUSED = 0, 2, 3, 4


def read_lines(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as file:
            return file.read().splitlines()
    except OSError:
        return []


def run_ended(folder):
    """The run's end: "DONE" in progress.log, or the runner's line "pictures in" in its log."""
    if any(line.endswith("] DONE") or line == "DONE" for line in read_lines(os.path.join(folder, "progress.log"))):
        return True
    return any(line.startswith("pictures in ") for line in read_lines(os.path.join(folder, "runner-log.txt")))


def has_fact(folder, text):
    return any(text in line for line in read_lines(os.path.join(folder, "progress.log")))


def local_time(value):
    """A time of --after as a naive local datetime: the form of the folder stamp, or ISO 8601 with or without a
    zone (a zone is turned into local time)."""
    try:
        return datetime.datetime.strptime(value, STAMP)
    except ValueError:
        pass
    moment = datetime.datetime.fromisoformat(value)
    if moment.tzinfo is not None:
        moment = moment.astimezone().replace(tzinfo=None)
    return moment


def folder_after(results, label, after):
    """The oldest folder of the label whose stamp is at or after `after`, or None."""
    suffix = "-" + label
    found = []
    for name in os.listdir(results) if os.path.isdir(results) else []:
        if not name.endswith(suffix):
            continue
        try:
            stamp = datetime.datetime.strptime(name[:15], STAMP)
        except ValueError:
            continue
        if name[15:] == suffix and stamp >= after:
            found.append((stamp, name))
    return os.path.join(results, min(found)[1]) if found else None


def wait(text, interval, timeout, run=None, label=None, after=None, results=RESULTS, say=print):
    """Polls until the fact is there. Returns (exit code, the folder or None)."""
    end = time.monotonic() + timeout
    folder = run
    while True:
        if folder is None:
            folder = folder_after(results, label, after)
        if folder is not None:
            if has_fact(folder, text):
                say("found %r in %s" % (text, folder))
                return FOUND, folder
            if run_ended(folder):
                say("the run ended without %r: %s" % (text, folder))
                return ENDED, folder
        if time.monotonic() >= end:
            say("timed out after %g s: %s" % (timeout, "no folder of the label at or after the time" if folder is None
                                               else "%r not in %s" % (text, folder)))
            return TIMED_OUT, folder
        time.sleep(min(interval, max(0.0, end - time.monotonic())))


def wait_new_lines_only(path, text, timeout):
    """The null of the self-test: the old way, a reader that sees only the lines written after its start (as an
    event wait without `since`, or `tail -n0 -F | grep`)."""
    start = len(read_lines(path))
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if any(text in line for line in read_lines(path)[start:]):
            return FOUND
        time.sleep(0.1)
    return TIMED_OUT


def newest_folder(results, label):
    """The null of C1: the newest folder of the label, whatever its time."""
    names = sorted(name for name in os.listdir(results) if name.endswith("-" + label))
    return os.path.join(results, names[-1]) if names else None


def self_test():
    quiet = lambda _line: None
    loaded = FACTS["loaded"]
    line = "[nirender.owndevice] save loaded, cell 'Seyda Neen', god mode on, AI toggled true\n"
    with tempfile.TemporaryDirectory() as results:
        def make(stamp, label, lines):
            folder = os.path.join(results, "%s-%s" % (stamp, label))
            os.makedirs(folder, exist_ok=True)
            with open(os.path.join(folder, "progress.log"), "a", encoding="utf-8") as file:
                file.writelines(lines)
            return folder

        # The line was written before the wait: found by the first read.
        early = make("20261117-100000", "early", [line])
        started = time.monotonic()
        assert wait(loaded, 10, 5, run=early, say=quiet) == (FOUND, early)
        assert time.monotonic() - started < 1.0
        # The null: a reader of new lines only blocks to its limit on the same file.
        started = time.monotonic()
        assert wait_new_lines_only(os.path.join(early, "progress.log"), loaded, 1) == TIMED_OUT
        assert time.monotonic() - started >= 1.0

        # The line comes 0.5 s after the start of the wait.
        late = make("20261117-100100", "late", [])
        threading.Timer(0.5, lambda: make("20261117-100100", "late", [line])).start()
        started = time.monotonic()
        assert wait(loaded, 0.1, 5, run=late, say=quiet) == (FOUND, late)
        assert time.monotonic() - started < 1.5

        # No line: the end of the run gives 3 at once; no end gives 2 at the limit.
        ended = make("20261117-100200", "ended", ["[nirender.owndevice] DONE\n"])
        assert wait(loaded, 0.2, 5, run=ended, say=quiet) == (ENDED, ended)
        runner_end = make("20261117-100230", "runner-end", [])
        with open(os.path.join(runner_end, "runner-log.txt"), "w", encoding="utf-8") as file:
            file.write("pictures in %s\n" % runner_end)
        assert wait(loaded, 0.2, 5, run=runner_end, say=quiet) == (ENDED, runner_end)
        empty = make("20261117-100300", "empty", [])
        started = time.monotonic()
        assert wait(loaded, 0.1, 0.5, run=empty, say=quiet) == (TIMED_OUT, empty)
        assert time.monotonic() - started >= 0.5

        # C1: an old run of the label holds the line; --after is later. No new folder: the limit, not 0.
        old = make("20261117-110000", "live", [line])
        after = local_time("20261117-110500")
        assert wait(loaded, 0.1, 0.5, label="live", after=after, results=results, say=quiet) == (TIMED_OUT, None)
        # The null of C1: the newest folder of the label is the old run, and its line ends the wait at once.
        assert newest_folder(results, "live") == old and has_fact(old, loaded)
        # A new folder 0.5 s later with the line: found.
        threading.Timer(0.5, lambda: make("20261117-110600", "live", [line])).start()
        code, folder = wait(loaded, 0.1, 5, label="live", after=after, results=results, say=quiet)
        assert code == FOUND and folder.endswith("20261117-110600-live"), (code, folder)
        # A label that only ends like another: "x-live" is not "live".
        make("20261117-110700", "x-live", [line])
        assert folder_after(results, "live", after).endswith("20261117-110600-live")

    # The forms of --after: the stamp, ISO without a zone (local), ISO with a zone (turned into local time).
    assert local_time("20261117-110500") == datetime.datetime(2026, 11, 17, 11, 5, 0)
    assert local_time("2026-11-17T11:05:00") == datetime.datetime(2026, 11, 17, 11, 5, 0)
    zoned = datetime.datetime(2026, 11, 17, 7, 5, 0, tzinfo=datetime.timezone.utc)
    assert local_time("2026-11-17T07:05:00+00:00") == zoned.astimezone().replace(tzinfo=None)
    print("wait_once self-test: ok")


def main():
    ap = argparse.ArgumentParser(description="Waits for a fact of a run by polling its progress.log.")
    ap.add_argument("fact", nargs="?", help="loaded, live, or the text of a line")
    ap.add_argument("--run", help="the run folder")
    ap.add_argument("--label", help="the label of the run; needs --after")
    ap.add_argument("--after", help="a time taken before the start: YYYYmmdd-HHMMSS (local) or ISO 8601")
    ap.add_argument("--interval", type=float, default=10.0, help="seconds between two reads (default 10)")
    ap.add_argument("--timeout", type=float, default=300.0, help="the limit in seconds (default 300)")
    ap.add_argument("--self-test", action="store_true", help="the tests, with their nulls; no game")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return FOUND
    if not args.fact or bool(args.run) == bool(args.label):
        print("refused: name the fact and one of --run or --label")
        return REFUSED
    if args.label and not args.after:
        print("refused: --label needs --after, the time taken before the start (an older run of the label must not count)")
        return REFUSED
    text = FACTS.get(args.fact, args.fact)
    after = local_time(args.after) if args.after else None
    code, _folder = wait(text, args.interval, args.timeout, run=args.run, label=args.label, after=after)
    return code


if __name__ == "__main__":
    sys.exit(main())
