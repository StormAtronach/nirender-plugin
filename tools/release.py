"""Makes a test build release on GitHub, and removes the releases before it.

    python tools/release.py --notes <file.md> [--max-perf] [--mge DIR] [--repo OWNER/NAME]
        [--keep-old] [--no-build] [--add SOURCE=PATH ...] [--add-dir SOURCE=PATH ...]

The steps:

1. The version is the one of Cargo.toml of this repository. A release with that version
   must not be there. The work of the two repositories must be committed and pushed: the
   tag of the release goes on the head of the default branch on GitHub.
2. `tools/checks.py all` runs the content checks of both repositories; `tools/docs.py`
   checks the doc comments of the plugin, the host crate and the helper (no broken link).
   `tools/clippy_baseline.py --check` stops the release when a Clippy lint has more warnings
   than `tools/clippy.baseline` holds. `cargo xtask dist` in the MGE XE repository builds MGE XE and this plugin
   and packs the archive (`--max-perf` for the slow build of a final release).
3. The archive goes up as `MGE-XE-G7-v<MGE XE>-nirender-v<plugin>.7z`, as a pre-release
   with the text of `--notes`. Each `--add SOURCE=PATH` puts a file into it first, at PATH
   from the archive's root (the Morrowind folder): for example the MWSE.dll with the native
   ABI that the plugin needs (`--add E:/nirender-keep/mwse/08f7d20ed4/MWSE.dll=MWSE.dll`).
   Each `--add-dir SOURCE=PATH` puts every file of the folder SOURCE into it, at PATH (`.`
   for the root) and their paths under SOURCE: for example a mod laid out as the game
   folder (`--add-dir <stage of True Water>=.`). One argument for a folder: 157 `--add` did
   not fit on the command line of Windows (v0.2.15).
4. All other releases of the repository are deleted, so that a tester finds one build only.
   Their tags stay. `--keep-old` leaves the releases.

`--dry-run` does the steps up to the packed archive and publishes and deletes nothing; the
checks of step 1 then only print what would stop a release. A test of the archive's contents.

The tool starts no game.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
PLUGIN = os.path.dirname(HERE)
MGE = os.path.join(os.path.dirname(PLUGIN), "MGE-XE-G7-render-link")


def run(command, cwd=None, capture=False):
    result = subprocess.run(command, cwd=cwd, capture_output=capture, text=True)
    if result.returncode != 0:
        raise SystemExit("failed (%d): %s%s" % (result.returncode, " ".join(command), "\n" + result.stderr if capture else ""))
    return result.stdout if capture else ""


def version_of(manifest):
    text = open(manifest, encoding="utf-8").read()
    return re.search(r'^version\s*=\s*"([^"]+)"', text, re.MULTILINE).group(1)


def stop(text, dry_run):
    """Stops a release; a dry run only prints it."""
    if not dry_run:
        raise SystemExit(text)
    print("dry run, a release would stop here: " + text)


def clean_and_pushed(repo, dry_run):
    """Stops when the repository has work that is not committed or not pushed."""
    if run(["git", "-C", repo, "status", "--porcelain", "--untracked-files=no"], capture=True).strip():
        stop("%s has changes that are not committed" % repo, dry_run)
    run(["git", "-C", repo, "fetch", "--quiet"])
    ahead = run(["git", "-C", repo, "rev-list", "--count", "@{upstream}..HEAD"], capture=True).strip()
    if ahead != "0":
        stop("%s has %s commits that are not pushed" % (repo, ahead), dry_run)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--notes", required=True, help="the text of the release, a Markdown file")
    ap.add_argument("--max-perf", action="store_true", help="the slow build of a final release")
    ap.add_argument("--mge", default=MGE, help="the MGE XE repository")
    ap.add_argument("--repo", default="StormAtronach/nirender-plugin", help="the GitHub repository of the releases")
    ap.add_argument("--keep-old", action="store_true", help="do not delete the releases before this one")
    ap.add_argument("--no-build", action="store_true", help="use the archive that is in dist (it must be of this version)")
    ap.add_argument("--add", action="append", default=[], metavar="SOURCE=PATH", help="a file to put into the archive at PATH from its root")
    ap.add_argument("--add-dir", action="append", default=[], metavar="SOURCE=PATH", help="every file of the folder SOURCE, at PATH (. for the root) and their paths under SOURCE")
    ap.add_argument("--dry-run", action="store_true", help="pack the archive, publish and delete nothing")
    args = ap.parse_args()

    assert os.path.isfile(args.notes), args.notes
    additions = []
    for item in args.add:
        source, _, path = item.partition("=")
        assert os.path.isfile(source) and path, "--add needs SOURCE=PATH with an existing file: %s" % item
        additions.append((source, path.replace("\\", "/")))
    for item in args.add_dir:
        folder, _, path = item.partition("=")
        assert os.path.isdir(folder) and path, "--add-dir needs SOURCE=PATH with an existing folder: %s" % item
        files = 0
        for root, _, names in os.walk(folder):
            for name in names:
                source = os.path.join(root, name)
                inside = os.path.relpath(source, folder).replace("\\", "/")
                additions.append((source, inside if path in (".", "/") else path.rstrip("/\\").replace("\\", "/") + "/" + inside))
                files += 1
        assert files, "--add-dir: the folder %s has no file" % folder
        print("the folder %s gives %d files" % (folder, files))
    plugin_version = version_of(os.path.join(PLUGIN, "Cargo.toml"))
    tag = "v" + plugin_version
    releases = json.loads(run(["gh", "release", "list", "--repo", args.repo, "--limit", "100", "--json", "tagName"], capture=True))
    tags = [release["tagName"] for release in releases]
    if tag in tags:
        stop("the release %s is there already: set a new version in Cargo.toml" % tag, args.dry_run)
    for repo in (PLUGIN, args.mge):
        clean_and_pushed(repo, args.dry_run)

    cargo = shutil.which("cargo") or os.path.join(os.path.expanduser("~"), ".cargo", "bin", "cargo.exe")
    if not args.no_build:
        # The content checks of both repositories, then the doc comments of the three crates
        # (no broken link), then no Clippy lint above its baseline.
        run([sys.executable, "-I", os.path.join(HERE, "checks.py"), "all"])
        run([sys.executable, "-I", os.path.join(HERE, "docs.py"), "--mge", args.mge])
        run([sys.executable, "-I", os.path.join(HERE, "clippy_baseline.py"), "--check", "--mge", args.mge])
        run([cargo, "xtask", "dist"] + (["--max-perf"] if args.max_perf else []), cwd=args.mge)
    # The version of MGE XE is in the name of the archive that xtask packs.
    kind = "" if args.max_perf else "-test"
    packed = [name for name in os.listdir(os.path.join(args.mge, "dist")) if re.fullmatch(r"MGE-XE-G7-v[0-9.]+%s\.7z" % re.escape(kind), name)]
    assert len(packed) == 1, "one archive of this kind in dist, found: %s" % packed
    mge_version = re.match(r"MGE-XE-G7-v([0-9.]+)", packed[0]).group(1)
    asset = os.path.join(args.mge, "dist", "MGE-XE-G7-v%s-nirender-%s.7z" % (mge_version, tag))
    shutil.copy2(os.path.join(args.mge, "dist", packed[0]), asset)
    if additions:
        # The files go into a folder laid out as the archive, and 7-Zip adds its entries.
        stage = os.path.join(args.mge, "target", "release-additions")
        if os.path.isdir(stage):
            shutil.rmtree(stage)
        for source, path in additions:
            os.makedirs(os.path.dirname(os.path.join(stage, path)) or stage, exist_ok=True)
            shutil.copy2(source, os.path.join(stage, path))
        seven = r"C:\Program Files\7-Zip\7z.exe" if os.path.isfile(r"C:\Program Files\7-Zip\7z.exe") else "7z"
        tops = sorted({path.split("/")[0] for _, path in additions})
        run([seven, "a", "-t7z", "-mx=9", asset] + tops, cwd=stage)
        for source, path in additions:
            print("added %s as %s" % (source, path))

    title = "Test build: MGE XE G7 %s with nirender %s" % (mge_version, plugin_version)
    if args.dry_run:
        print("dry run: the archive %s; not published (%s); no release deleted" % (asset, title))
        return 0
    print(run(["gh", "release", "create", tag, "--repo", args.repo, "--prerelease", "--title", title, "--notes-file", args.notes, asset], capture=True).strip())
    if not args.keep_old:
        for old in tags:
            run(["gh", "release", "delete", old, "--repo", args.repo, "--yes"])
            print("deleted the release %s (its tag stays)" % old)
    print("the release %s is the only one" % tag if not args.keep_old else "the release %s is out" % tag)
    return 0


if __name__ == "__main__":
    sys.exit(main())
