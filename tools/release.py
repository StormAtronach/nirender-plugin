"""Makes a test build release on GitHub, and removes the releases before it.

    python tools/release.py --notes <file.md> [--max-perf] [--mge DIR] [--repo OWNER/NAME]
        [--keep-old] [--no-build] [--dry-run] [--add SOURCE=PATH ...] [--add-dir SOURCE=PATH ...]
        [--source NAME=URL@COMMIT ...]

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
   The license files go in with them: `COPYING-GPL-2.0.txt` (the text of the GNU General
   Public License, version 2, from the MGE XE repository: the license of MGE XE G7, of
   MWSE.dll and of watervolumes.dll), `LICENSE-nirender.txt` (this repository's LICENSE),
   `THIRD-PARTY-NOTICES.txt` (the crates inside the programs, written by
   `tools/third_party_notices.py` from the MGE XE lock file), and `SOURCES.txt`, which names
   each part of the archive with its license and its source repository at its commit. The
   MGE XE and nirender commits come from the two repositories; each `--source NAME=URL@COMMIT`
   names a part whose files come from `--add` or `--add-dir`, for example
   `--source "MWSE.dll=https://github.com/StormAtronach/MWSE@652cb8ae31"` and
   `--source "True Water=https://github.com/StormAtronach/MWSE-water-volumes@208499f"`.
   A release stops when it has fewer `--source` than `--add` and `--add-dir` arguments:
   SOURCES.txt is the written offer of the source of the GPL binaries in the archive, and no
   later step would catch a part without one. The license files are made before the build,
   so that a problem with them costs no build.
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


def sources_text(mge_url, mge_commit, plugin_url, plugin_tag, plugin_commit, sources):
    """The text of SOURCES.txt: each part of the archive, its license and its source at its
    commit. `sources` is a list of (name, url, commit) for the parts added from outside the
    two repositories."""
    outside = " and ".join(name for name, _, _ in sources)
    lines = [
        "THE SOURCES OF THIS ARCHIVE",
        "",
        "Each part of the archive, its license, and its complete source at the commit it was",
        "built from. The license texts are in the archive: COPYING-GPL-2.0.txt (the GNU General",
        "Public License, version 2), LICENSE-nirender.txt (MIT), THIRD-PARTY-NOTICES.txt (the",
        "crates inside the programs)%s." % (", and the license files of " + outside if outside else ""),
        "",
        "MGE XE G7 (d3d8.dll, dinput8.dll, MGEXEgui.exe, the MGE XE part of numidium.exe, the",
        "shaders, textures, meshes, the esp and the MGE XE Options mod): GNU General Public",
        "License, version 2.",
        "  %s at %s" % (mge_url, mge_commit),
        "",
        "nirender (Data Files/MWSE/lib/akulakhan.dll, Data Files/MWSE/mods/nirender, the host",
        "crate of numidium.exe): MIT. numidium.exe as a whole is distributed under the GNU",
        "General Public License, version 2, since it contains the code of MGE XE G7.",
        "  %s at %s (the tag %s)" % (plugin_url, plugin_commit, plugin_tag),
        "",
    ]
    for name, url, commit in sources:
        lines += ["%s: see its license files in the archive, or its repository." % name, "  %s at %s" % (url, commit), ""]
    lines += [
        "The crates inside numidium.exe, MGEXEgui.exe, d3d8.dll and dinput8.dll: see",
        "THIRD-PARTY-NOTICES.txt.",
        "",
    ]
    return "\n".join(lines)


def license_additions(mge, plugin_url, plugin_tag, sources, stage):
    """Writes SOURCES.txt and THIRD-PARTY-NOTICES.txt into `stage` and returns the license
    files to add to the archive, as (source, path) pairs."""
    os.makedirs(stage, exist_ok=True)
    mge_url = run(["git", "-C", mge, "remote", "get-url", "origin"], capture=True).strip()
    mge_commit = run(["git", "-C", mge, "rev-parse", "HEAD"], capture=True).strip()
    plugin_commit = run(["git", "-C", PLUGIN, "rev-parse", "HEAD"], capture=True).strip()
    sources_path = os.path.join(stage, "SOURCES.txt")
    with open(sources_path, "w", encoding="utf-8", newline="\n") as out:
        out.write(sources_text(mge_url, mge_commit, plugin_url, plugin_tag, plugin_commit, sources))
    notices_path = os.path.join(stage, "THIRD-PARTY-NOTICES.txt")
    run([sys.executable, "-I", os.path.join(HERE, "third_party_notices.py"), "--mge", mge, "--out", notices_path])
    gpl = os.path.join(mge, "license.txt")
    assert os.path.isfile(gpl), "the MGE XE repository has no license.txt"
    plugin_license = os.path.join(PLUGIN, "LICENSE")
    assert os.path.isfile(plugin_license), "this repository has no LICENSE"
    return [(gpl, "COPYING-GPL-2.0.txt"), (plugin_license, "LICENSE-nirender.txt"), (notices_path, "THIRD-PARTY-NOTICES.txt"), (sources_path, "SOURCES.txt")]


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
    ap.add_argument("--source", action="append", default=[], metavar="NAME=URL@COMMIT", help="the source of a part added with --add or --add-dir, for SOURCES.txt")
    ap.add_argument("--dry-run", action="store_true", help="pack the archive, publish and delete nothing")
    args = ap.parse_args()

    assert os.path.isfile(args.notes), args.notes
    sources = []
    for item in args.source:
        name, _, rest = item.partition("=")
        url, _, commit = rest.rpartition("@")
        assert name and url and commit, "--source needs NAME=URL@COMMIT: %s" % item
        sources.append((name, url, commit))
    # Every part added from outside the two repositories is named in SOURCES.txt: it is the
    # written offer of the source of a GPL binary, and nothing later would catch its absence.
    outside = args.add + args.add_dir
    if len(sources) < len(outside):
        raise SystemExit("\n".join(
            ["%s has no --source NAME=URL@COMMIT; SOURCES.txt must name every part added from outside the two repositories (%d parts, %d --source)" % (item, len(outside), len(sources)) for item in outside]
        ))
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

    # The license texts, the notices of the crates and the sources of the parts: before the
    # build, so that a stop here costs no build.
    additions += license_additions(args.mge, "https://github.com/" + args.repo, tag, sources, os.path.join(args.mge, "target", "release-licenses"))

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
