"""The Clippy warnings of each lint in the three places of a release, against a baseline.

The places: the plugin (this repository), the host crate (`host`) and the MGE XE render-link
workspace (`--workspace`). Each runs `cargo clippy --message-format=json` with the toolchain
of its folder, without `-D warnings`. No place passed `-D warnings` on 2026-10-27, so the gate
is the count of each lint, not zero.

`python -I tools/clippy_baseline.py` lists the counts. `--check` compares them with
`tools/clippy.baseline` and fails when a lint has more warnings than its baseline, or when a
place does not compile; release.py runs it. A lint with fewer lowers its line with `--update`.
The baseline only goes down: raise a line by hand, with the reason in the commit message (a
new toolchain can add lints; see `rust-toolchain.toml` of each folder).

Clippy compiles: ask for a build slot first. `--mge` names another render-link folder.
`--mge-host DIR` builds the render-link place against the host crate in DIR, through cargo's
`--config paths=[...]` override, which leaves the render-link tree as it is: for a render-link
branch whose code needs a host crate that the main checkout does not have yet. release.py runs
without it. `--self-test` tests the arguments (no cargo).
"""
import argparse
import collections
import datetime
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
PLUGIN = os.path.dirname(HERE)
MGE = os.path.join(os.path.dirname(PLUGIN), "MGE-XE-G7-render-link")
BASELINE = os.path.join(HERE, "clippy.baseline")


def host_override(host):
    """The cargo arguments that build against the host crate in the folder `host`."""
    path = os.path.abspath(host).replace("\\", "/")
    if "'" in path:
        raise SystemExit("--mge-host: a path with a quote cannot go into cargo's --config: %s" % path)
    manifest = os.path.join(host, "Cargo.toml")
    package = open(manifest, encoding="utf-8").read() if os.path.isfile(manifest) else ""
    if not re.search(r'^name\s*=\s*"nirender-host"', package, re.MULTILINE):
        raise SystemExit("--mge-host: %s is not the folder of the crate nirender-host (nirender-plugin's host)" % host)
    return ["--config", "paths=['%s']" % path]


def places(mge, host=None):
    """The places, as {name: (folder, cargo arguments)}; with `host`, the render-link place
    builds against that host crate."""
    return {
        "plugin": (PLUGIN, []),
        "host": (os.path.join(PLUGIN, "host"), []),
        "render-link": (mge, ["--workspace"] + (host_override(host) if host else [])),
    }


def count_warnings(folder, args):
    """The warnings of each lint in one place; None when the place does not compile."""
    run = subprocess.run(["cargo", "clippy", "--message-format=json", *args], cwd=folder, capture_output=True,
                         text=True, encoding="utf-8", errors="replace")
    counts = collections.Counter()
    for line in run.stdout.splitlines():
        if not line.startswith("{"):
            continue
        message = json.loads(line)
        if message.get("reason") != "compiler-message":
            continue
        diagnostic = message["message"]
        # The summary line ("generated 12 warnings") has no code.
        if diagnostic["level"] != "warning" or not diagnostic.get("code"):
            continue
        counts[diagnostic["code"]["code"]] += 1
    if run.returncode != 0:
        print(run.stderr.rstrip()[-4000:])
        return None
    return counts


def scan(mge, host=None):
    """{(place, lint): count} of all places; None when a place does not compile."""
    found = {}
    for place, (folder, args) in places(mge, host).items():
        counts = count_warnings(folder, args)
        if counts is None:
            print("clippy: %s does not compile" % place)
            return None
        for lint, count in counts.items():
            found[(place, lint)] = count
    return found


def read_baseline():
    base = {}
    if os.path.exists(BASELINE):
        for line in open(BASELINE, encoding="utf-8"):
            line = line.strip()
            if line and not line.startswith("#"):
                place, lint, count = line.split()
                base[(place, lint)] = int(count)
    return base


def head(repo):
    return subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=repo, capture_output=True, text=True).stdout.strip()


def write_baseline(found, mge):
    with open(BASELINE, "w", encoding="utf-8", newline="\n") as out:
        out.write("# The Clippy warnings of each lint in each place (tools/clippy_baseline.py). It only goes down.\n")
        # The reference of the counts: a later toolchain or commit can differ.
        out.write("# Counted %s at nirender %s and render-link %s.\n" % (datetime.date.today(), head(PLUGIN), head(mge)))
        for (place, lint), count in sorted(found.items()):
            out.write("%s %s %d\n" % (place, lint, count))


def self_test():
    """The tests of the arguments: no cargo."""
    host = os.path.join(PLUGIN, "host")
    assert places("E:/mge")["render-link"] == ("E:/mge", ["--workspace"])
    assert places("E:/mge")["plugin"] == (PLUGIN, []) and places("E:/mge")["host"] == (host, [])
    folder, args = places("E:/mge", host)["render-link"]
    assert folder == "E:/mge" and args[:2] == ["--workspace", "--config"], args
    assert args[2] == "paths=['%s']" % os.path.abspath(host).replace("\\", "/") and "\\" not in args[2], args
    # The other places keep their own build.
    assert places("E:/mge", host)["host"] == (host, [])
    for bad in (PLUGIN, os.path.join(PLUGIN, "no-such-folder")):
        try:
            host_override(bad)
        except SystemExit:
            pass
        else:
            raise AssertionError("--mge-host %s was taken" % bad)
    print("clippy_baseline self-test: ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mge", default=MGE, help="the MGE XE render-link repository")
    ap.add_argument("--mge-host", help="the folder of a host crate (nirender-plugin's host) that the render-link place builds against, "
                    "through cargo's --config paths override; without it, the path of render-link's Cargo.toml")
    ap.add_argument("--check", action="store_true", help="fail when a lint has more warnings than its baseline")
    ap.add_argument("--update", action="store_true", help="lower the baseline to the counts of this run")
    ap.add_argument("--self-test", action="store_true", help="the tests of the arguments: no cargo")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.update and args.mge_host:
        # The baseline is counted on the layout of a release, the one that release.py checks.
        raise SystemExit("--update counts the baseline without --mge-host: the layout that release.py checks")

    found = scan(args.mge, args.mge_host)
    if found is None:
        return 1
    if args.update:
        old = read_baseline()
        grown = ["%s %s" % key for key, count in found.items() if count > old.get(key, 0)]
        if old and grown:
            print("clippy: refusing to raise the baseline of %s" % ", ".join(sorted(grown)))
            return 1
        write_baseline(found, args.mge)
        print("clippy: baseline written, %d warnings of %d lints" % (sum(found.values()), len(found)))
        return 0
    if args.check:
        base = read_baseline()
        failed = 0
        for key, count in sorted(found.items()):
            allowed = base.get(key, 0)
            if count > allowed:
                failed += 1
                print("clippy: %s %s has %d, the baseline %d: fix the new warnings" % (key + (count, allowed)))
        lower = sum(1 for key, allowed in base.items() if found.get(key, 0) < allowed)
        print("clippy: %d warnings; %s%s" % (sum(found.values()), "%d lints above the baseline" % failed if failed else "none above the baseline",
                                            "; %d lints below the baseline (--update)" % lower if lower else ""))
        return 1 if failed else 0
    for (place, lint), count in sorted(found.items()):
        print("%-12s %-45s %5d" % (place, lint, count))
    return 0


if __name__ == "__main__":
    sys.exit(main())
