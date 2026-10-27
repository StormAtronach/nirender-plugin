"""Makes a test build release on GitHub, and removes the releases before it.

    python tools/release.py --notes <file.md> [--max-perf] [--mge DIR] [--repo OWNER/NAME]
        [--keep-old] [--no-build] [--dry-run] [--add SOURCE=PATH ...] [--add-dir SOURCE=PATH ...]
        [--source NAME=URL@COMMIT ...]
    python -I tools/release.py --self-test    # the tests of the pure parts, no git, no cargo

The steps:

1. The version is the one of Cargo.toml of this repository. A release with that version
   must not be there. The work of the two repositories must be committed and pushed: the
   tag of the release goes on the head of the default branch on GitHub. An untracked file
   under a path that the build reads or the archive takes (the source folders, the
   manifests, the assets: `PLUGIN_SOURCES`, `MGE_SOURCES`) stops the release with its name,
   since it would go into the binaries without being in the tag. The tool must run from the
   nirender-plugin that the build takes, the sibling of the MGE XE tree (xtask's
   `plugin_dir`, mgeHost64's path dependency); from a worktree it stops.
2. `tools/checks.py all --strict` runs the content checks of both repositories (a check
   whose repository is not on this machine is a failure, not a skip); `tools/docs.py`
   checks the doc comments of the plugin, the host crate and the helper (no broken link).
   `tools/clippy_baseline.py --check` stops the release when a Clippy lint has more warnings
   than `tools/clippy.baseline` holds. Then `cargo test` runs in the plugin, in the host crate
   and for the helper (`-p mgeHost64` in the MGE XE repository), each with the toolchain its
   tree pins (the version is printed); a failed test stops the release. `cargo xtask dist`
   in the MGE XE repository builds MGE XE and this plugin and packs the archive (`--max-perf`
   for the slow build of a final release). `--no-build` skips all of step 2 and takes the
   archive that is in dist.
3. The tool prints the commit of each repository: the build id that the binaries of the
   archive log on their first line, for the notes. The archive goes up as
   `MGE-XE-G7-v<MGE XE>-nirender-v<plugin>.7z`, as a pre-release with the text of `--notes`.
   The MGE XE repository then gets the tag `nirender-v<plugin>` on the commit the archive
   was built from, pushed with the release (step 1 stops when that tag is there already).
   Each `--add SOURCE=PATH` puts a file into it first, at PATH
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
   Then the gate: the archive is unpacked into `target/release-gate` of the MGE XE repository
   and `tools/validate_retained.py baseline` replays the captures of the baseline on the
   helper in it, the one that ships. A failed gate stops the release, and so does a baseline
   whose accepted pictures were made on another adapter or driver than this machine's, or
   name none: the pictures are the evidence of one GPU and one driver, so the baseline is
   accepted again on the release machine first (`validate_retained.py baseline --accept`).
   Its result lines go
   into the notes in place of the token `{gate}` of their Tested block; the filled notes are
   written to `dist/notes-<tag>.md` and published. The notes follow
   `tools/notes-template.md`: their Tested block must hold the token (or, with `--no-gate`,
   the gate's result line marked "run by hand") and the run folder of the live checks through the mwse-mcp
   bridge (`YYYYmmdd-HHMMSS-label` under `results`), written by hand; a release stops at
   the start without them. `--no-gate` skips the gate.
   Before the gate, the archive gets `MANIFEST.txt`: the commits of the two repositories, the
   compiler, and the SHA-256, the size and the path of each of its files. Then the SHA-256 of
   the archive itself goes beside it as `<archive>.sha256` (the format of `sha256sum`), up
   with the release as a second asset, and into the notes in place of the token `{sha256}`,
   so that a player can check a download.
4. All other releases of the repository are deleted, so that a tester finds one build only.
   Their tags stay. `--keep-old` leaves the releases.

`--dry-run` does the steps up to the packed archive and publishes and deletes nothing; the
checks of step 1 then only print what would stop a release. A test of the archive's contents.

The tool starts no game.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
PLUGIN = os.path.dirname(HERE)
MGE = os.path.join(os.path.dirname(PLUGIN), "MGE-XE-G7-render-link")


# The environment of every program the tool starts: the shell's, without RUSTUP_TOOLCHAIN, so
# that each tree builds with the toolchain its rust-toolchain.toml pins (the tests, the Clippy
# baseline, the docs check and `cargo xtask dist` alike).
ENVIRONMENT = {key: value for key, value in os.environ.items() if key != "RUSTUP_TOOLCHAIN"}


def run(command, cwd=None, capture=False, env=None):
    result = subprocess.run(command, cwd=cwd, capture_output=capture, text=True, env=env or ENVIRONMENT)
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


# The paths of each repository that the build reads or the archive takes: an untracked file
# under one of them would go into the binaries or the archive without being in the tag.
PLUGIN_SOURCES = ("src/", "host/", "d3d8/", "mod/", "contract/", ".cargo/", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "LICENSE")
MGE_SOURCES = ("d3d8/", "dinput8/", "mgeHost64/", "mge-config/", "MGEXEgui/", "distantland/", "xtask/", "assets/", ".cargo/", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "license.txt")


def status_entries(porcelain_z):
    """The entries of a `git status --porcelain -z --untracked-files=all` output: (state, path),
    the path as git has it (no quotes: a name with a space or a special character is quoted in
    the output without `-z`, and a quoted name passed the untracked stop). A rename carries
    its old name as a second NUL-ended field, which is skipped."""
    fields = porcelain_z.split("\0")
    entries = []
    index = 0
    while index < len(fields) and fields[index]:
        field = fields[index]
        entries.append((field[:2], field[3:]))
        index += 2 if field[0] in "RC" else 1
    return entries


def untracked_in_sources(entries, sources):
    """The untracked files of the status entries that are under one of the source paths (a
    folder ends with a slash; a file is named whole)."""
    untracked = [path for state, path in entries if state == "??"]
    return [path for path in untracked if any(path.startswith(source) if source.endswith("/") else path == source for source in sources)]


def plugin_of_the_build(mge):
    """The nirender-plugin that a build of `mge` takes: `cargo xtask dist` builds the sibling
    `nirender-plugin` of the MGE XE tree (xtask's `plugin_dir`), and mgeHost64 compiles the
    host crate from `../../nirender-plugin/host` (its Cargo.toml). A release must run from
    that checkout, or the archive holds another checkout's plugin while MANIFEST.txt and the
    tag name this one."""
    return os.path.realpath(os.path.join(os.path.dirname(os.path.realpath(mge)), "nirender-plugin"))


def clean_and_pushed(repo, sources, dry_run):
    """Stops when the repository has work that is not committed or not pushed, or an
    untracked file under a path the build or the archive takes."""
    status = status_entries(run(["git", "-C", repo, "status", "--porcelain", "-z", "--untracked-files=all"], capture=True))
    if any(state != "??" for state, _ in status):
        stop("%s has changes that are not committed" % repo, dry_run)
    untracked = untracked_in_sources(status, sources)
    if untracked:
        stop("%s has untracked files under the paths the build takes; add them or move them away: %s" % (repo, ", ".join(untracked)), dry_run)
    run(["git", "-C", repo, "fetch", "--quiet"])
    counted = subprocess.run(["git", "-C", repo, "rev-list", "--count", "@{upstream}..HEAD"], capture_output=True, text=True)
    if counted.returncode != 0:
        # A branch without an upstream (a new branch of a worktree) is not pushed either.
        stop("%s: the branch has no upstream, so its commits are not pushed" % repo, dry_run)
    elif counted.stdout.strip() != "0":
        stop("%s has %s commits that are not pushed" % (repo, counted.stdout.strip()), dry_run)


def unit_tests(mge):
    """Runs `cargo test` in the plugin, in the host crate and for the helper in the MGE XE
    repository. Each tree builds with the toolchain its rust-toolchain.toml pins (rustup
    reads it from the working folder; the plugin and MGE XE pin the same dated nightly, the
    host crate has none and takes the plugin's), and the version is printed first. A failed
    test stops the release."""
    trees = [("the plugin", PLUGIN, []), ("the host crate", os.path.join(PLUGIN, "host"), []), ("the helper", mge, ["-p", "mgeHost64"])]
    for name, folder, extra in trees:
        print("the tests of %s, with %s" % (name, compiler_version(folder)), flush=True)
        run(["cargo", "test"] + extra, cwd=folder)


def compiler_version(folder):
    """The `rustc --version` of a tree: the toolchain its rust-toolchain.toml pins."""
    return run(["rustc", "--version"], cwd=folder, capture=True).strip()


GATE_TOKEN = "{gate}"
GATE_RESULT = re.compile(r"\d+ of \d+ pictures equal to the accepted ones(?: in \S+)?")
# A gate result written by hand into the notes (with `--no-gate`) carries this mark, so that
# it does not read like the tool's.
BY_HAND = "run by hand"


def unpack(mge, asset):
    """Unpacks the archive into `<mge>/target/release-gate` (made anew) and returns that
    folder: the files that ship, for the manifest and for the gate."""
    folder = os.path.join(mge, "target", "release-gate")
    if os.path.isdir(folder):
        shutil.rmtree(folder)
    run([seven_zip(), "x", "-y", "-o" + folder, asset], capture=True)
    assert os.path.isfile(os.path.join(folder, "numidium.exe")), "the archive has no numidium.exe at its root"
    return folder


def gate(folder, dry_run):
    """Replays the captures of the baseline on the helper in the unpacked archive
    (`validate_retained.py baseline --mge-build <folder>`: the helper that ships, not the one
    in bin). Returns the result lines ("N of M pictures equal to the accepted ones in ...");
    a failed gate stops the release."""
    print("the gate on the helper of %s" % folder, flush=True)
    result = subprocess.run([sys.executable, "-I", os.path.join(HERE, "validate_retained.py"), "baseline", "--mge-build", folder], capture_output=True, text=True, cwd=PLUGIN, env=ENVIRONMENT)
    print(result.stdout + result.stderr, flush=True)
    lines = GATE_RESULT.findall(result.stdout)
    if result.returncode != 0 or not lines:
        stop("the gate failed on the packed helper (%s)" % ("; ".join(lines) or "no result line"), dry_run)
    # The pictures are the evidence of one GPU and one driver: on another adapter a pass says
    # nothing about this machine's build, and a fail is not a finding.
    problem = adapter_problem(result.stdout)
    if problem:
        stop(problem, dry_run)
    return lines


def adapter_problem(output):
    """Why the gate's result is not the release's evidence, from the adapter line of
    `validate_retained.py baseline`: the accepted pictures were made on another adapter or
    driver (the pictures are the evidence of one GPU and one driver), or they name none.
    None when the run's adapter is the accepted pictures' one."""
    match = re.search(r"^\s*adapter: (.+?); the accepted pictures were made on (.+?), so a difference", output, re.MULTILINE)
    if match:
        return ("the baseline was accepted on %s; this machine has %s; accept the baseline again on this machine "
                "(validate_retained.py baseline --accept) before the release" % (match.group(2), match.group(1)))
    if re.search(r"^\s*adapter: .+; the accepted pictures name none", output, re.MULTILINE):
        return "the accepted pictures of the baseline name no adapter; accept the baseline again on this machine (validate_retained.py baseline --accept) to record it before the release"
    return None


def gate_text(lines):
    """The gate's result for the notes: its result lines without the local folders they end
    with ("in <path>")."""
    return "the capture gate: " + "; ".join(re.sub(r" in \S+$", "", line) for line in lines)


SHA_TOKEN = "{sha256}"


def fill_tokens(notes, values):
    """The notes with each token (`{gate}`, `{sha256}`) replaced by its value."""
    for token, value in values.items():
        notes = notes.replace(token, value)
    return notes


def notes_problems(notes, gated):
    """What is missing from the notes: the Tested block's parts, and the token `{sha256}`
    for the archive's checksum."""
    problems = tested_block_problems(notes, gated)
    if SHA_TOKEN not in notes:
        problems.append("no %s token for the archive's SHA-256" % SHA_TOKEN)
    if not gated and GATE_TOKEN in notes:
        problems.append("the token %s with --no-gate: write the gate's result by hand" % GATE_TOKEN)
    return problems


def sha256_of(path):
    digest = hashlib.sha256()
    with open(path, "rb") as source:
        for block in iter(lambda: source.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def checksum_line(digest, name):
    """One line in the format of `sha256sum`, which checks it: `sha256sum -c <file>`."""
    return "%s  %s\n" % (digest, name)


def file_digests(folder):
    """Each file under `folder`: its path from the folder with slashes, its size and its
    SHA-256, sorted by path."""
    entries = []
    for root, _, names in os.walk(folder):
        for name in names:
            path = os.path.join(root, name)
            entries.append((os.path.relpath(path, folder).replace("\\", "/"), os.path.getsize(path), sha256_of(path)))
    return sorted(entries)


def manifest_text(archive, provenance, entries):
    """The text of MANIFEST.txt: the archive's name, where its parts come from (`provenance`:
    a line of text), then one line for each file: its SHA-256, its size in bytes and its
    path from the root of the archive."""
    lines = ["THE FILES OF " + archive, "", provenance, "", "Each file: its SHA-256, its size in bytes, its path from the root of the archive.", ""]
    lines += ["%s  %d  %s" % (digest, size, path) for path, size, digest in entries]
    return "\n".join(lines) + "\n"


def provenance(mge, plugin_tag):
    """The commits of the two repositories and the compiler of the MGE XE tree, in one line."""
    mge_commit = run(["git", "-C", mge, "rev-parse", "--short", "HEAD"], capture=True).strip()
    plugin_commit = run(["git", "-C", PLUGIN, "rev-parse", "--short", "HEAD"], capture=True).strip()
    return "MGE XE G7 at %s; nirender at %s (the tag %s); built with %s." % (mge_commit, plugin_commit, plugin_tag, compiler_version(mge))


def write_text(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as out:
        out.write(text)


def add_to_archive(asset, additions, stage):
    """Puts each (source, path) of `additions` into the archive at `path` from its root: the
    files go into `stage`, a folder laid out as the archive, and 7-Zip adds its entries."""
    if os.path.isdir(stage):
        shutil.rmtree(stage)
    for source, path in additions:
        os.makedirs(os.path.dirname(os.path.join(stage, path)) or stage, exist_ok=True)
        shutil.copy2(source, os.path.join(stage, path))
    tops = sorted({path.split("/")[0] for _, path in additions})
    run([seven_zip(), "a", "-t7z", "-mx=9", asset] + tops, cwd=stage)
    for source, path in additions:
        print("added %s as %s" % (source, path))


RUN_FOLDER = re.compile(r"\b\d{8}-\d{6}-[A-Za-z0-9][A-Za-z0-9-]*")


def tested_block_problems(notes, gated):
    """What is missing from the Tested block of the notes (`tools/notes-template.md`): the
    heading, the gate's result (the token `{gate}` for the gate to fill, or its result line
    when `gated` is false and the gate ran by hand), and the run folder of the live checks
    (`YYYYmmdd-HHMMSS-label`, as `results` of this repository names them). Empty when all
    are there."""
    problems = []
    match = re.search(r"^#+ Tested\s*$", notes, re.MULTILINE)
    block = notes[match.end():] if match else ""
    if not match:
        problems.append("no 'Tested' heading")
    if gated and GATE_TOKEN not in block:
        problems.append("no %s token for the gate's result" % GATE_TOKEN)
    if not gated and not (GATE_RESULT.search(block) and BY_HAND in block):
        problems.append("no result line of the gate marked (%s): N of M pictures equal to the accepted ones" % BY_HAND)
    if not RUN_FOLDER.search(block):
        problems.append("no run folder of the live checks (YYYYmmdd-HHMMSS-label)")
    return problems


def render_link_tag(plugin_tag):
    """The tag of the MGE XE repository for a release: `nirender-v<plugin version>`, so that
    the commit of MGE XE that an archive was built from is named there too (its own tags,
    v<MGE XE version>, mark MGE XE's versions, not the releases)."""
    return "nirender-" + plugin_tag


def seven_zip():
    usual = r"C:\Program Files\7-Zip\7z.exe"
    return usual if os.path.isfile(usual) else "7z"


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


def self_test():
    """The tests of the pure parts: no git, no cargo, no GitHub."""
    status = status_entries("?? src/new.rs\0?? notes.txt\0?? host/src/lib2.rs\0?? Cargo.toml.bak\0?? Cargo.lock\0 M src/lib.rs\0?? results/run/x.png\0")
    assert untracked_in_sources(status, PLUGIN_SOURCES) == ["src/new.rs", "host/src/lib2.rs", "Cargo.lock"], untracked_in_sources(status, PLUGIN_SOURCES)
    assert untracked_in_sources(status_entries("?? docs/a.md\0 M src/lib.rs\0"), MGE_SOURCES) == []
    # A file named whole matches itself only, not a longer name that starts with it.
    assert untracked_in_sources(status_entries("?? Cargo.toml.orig\0"), PLUGIN_SOURCES) == []
    # A name with a space is not quoted with -z, and a rename's old name is skipped.
    renamed = status_entries("?? src/new file.rs\0R  src/b.rs\0src/a.rs\0?? mod/x y.lua\0")
    assert renamed == [("??", "src/new file.rs"), ("R ", "src/b.rs"), ("??", "mod/x y.lua")], renamed
    assert untracked_in_sources(renamed, PLUGIN_SOURCES) == ["src/new file.rs", "mod/x y.lua"]
    lines = ["2 of 2 pictures equal to the accepted ones in a", "206 of 206 pictures equal to the accepted ones in b"]
    assert gate_text(lines) == "the capture gate: 2 of 2 pictures equal to the accepted ones; 206 of 206 pictures equal to the accepted ones"
    assert fill_tokens("- {gate}\n- SHA-256 `{sha256}`\n", {GATE_TOKEN: "g", SHA_TOKEN: "abc"}) == "- g\n- SHA-256 `abc`\n"
    assert fill_tokens("- the gate by hand\n", {GATE_TOKEN: "g"}) == "- the gate by hand\n"
    assert checksum_line("ab" * 32, "x.7z") == "ab" * 32 + "  x.7z\n"
    manifest = manifest_text("x.7z", "MGE XE G7 at 1; nirender at 2.", [("Data Files/a.dds", 12, "cd" * 32), ("d3d8.dll", 3, "ef" * 32)])
    assert manifest.startswith("THE FILES OF x.7z\n\nMGE XE G7 at 1; nirender at 2.\n\n")
    assert manifest.endswith("\n%s  12  Data Files/a.dds\n%s  3  d3d8.dll\n" % ("cd" * 32, "ef" * 32)), manifest
    assert notes_problems("## Tested\n\n- {gate}\n- run 20261027-120302-x\n", gated=True) == ["no {sha256} token for the archive's SHA-256"]
    assert notes_problems("SHA-256 {sha256}\n\n## Tested\n\n- {gate}\n- run 20261027-120302-x\n", gated=True) == []
    # With --no-gate a leftover token is found at the start, not after the build.
    leftover = notes_problems("SHA-256 {sha256}\n\n## Tested\n\n- {gate}\n- run 20261027-120302-x\n", gated=False)
    assert leftover[-1] == "the token {gate} with --no-gate: write the gate's result by hand", leftover
    assert GATE_RESULT.findall("  ok   " + lines[0] + "\n  FAIL  1 of 2 pictures equal to the accepted ones in c\n") == [lines[0], "1 of 2 pictures equal to the accepted ones in c"]
    assert render_link_tag("v0.3.3") == "nirender-v0.3.3"
    same = "       adapter: NVIDIA GeForce RTX 5070 (Vulkan, driver NVIDIA 617.14), as the accepted pictures\n  ok   2 of 2 pictures equal to the accepted ones in a\n"
    assert adapter_problem(same) is None
    other = "       adapter: AMD Radeon RX 7800 XT (Vulkan, driver 24.9.1); the accepted pictures were made on NVIDIA GeForce RTX 5070 (Vulkan, driver NVIDIA 617.14), so a difference can be the GPU's\n"
    assert adapter_problem(other).startswith("the baseline was accepted on NVIDIA GeForce RTX 5070 (Vulkan, driver NVIDIA 617.14); this machine has AMD Radeon RX 7800 XT (Vulkan, driver 24.9.1); accept the baseline again")
    none = "       adapter: NVIDIA GeForce RTX 5070 (Vulkan, driver NVIDIA 617.14); the accepted pictures name none (accept again to record it)\n"
    assert adapter_problem(none).startswith("the accepted pictures of the baseline name no adapter")
    assert plugin_of_the_build(os.path.join("C:\\", "repos", "MGE-XE-G7-render-link")) == os.path.realpath(os.path.join("C:\\", "repos", "nirender-plugin"))
    good = "## Changes\n\n- a\n\n## Tested\n\n- {gate}\n- The live checks, run folder `20261027-120302-mge-findings-live2`: the loads wait.\n"
    assert tested_block_problems(good, gated=True) == []
    by_hand = good.replace("{gate}", "the capture gate (run by hand): 206 of 206 pictures equal to the accepted ones")
    assert tested_block_problems(by_hand, gated=False) == []
    assert tested_block_problems(by_hand, gated=True) == ["no {gate} token for the gate's result"]
    unmarked = good.replace("{gate}", "the capture gate: 206 of 206 pictures equal to the accepted ones in now")
    assert tested_block_problems(unmarked, gated=False) == ["no result line of the gate marked (run by hand): N of M pictures equal to the accepted ones"]
    assert tested_block_problems(good, gated=False) == ["no result line of the gate marked (run by hand): N of M pictures equal to the accepted ones"]
    assert tested_block_problems("## Tested\n\n- {gate}\n", gated=True) == ["no run folder of the live checks (YYYYmmdd-HHMMSS-label)"]
    # The token and the folder count inside the Tested block only.
    assert len(tested_block_problems("- {gate} 20261027-120302-x\n\n## Changes\n", gated=True)) == 3
    print("release self-test: ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--notes", help="the text of the release, a Markdown file")
    ap.add_argument("--self-test", action="store_true", help="the tests of the pure parts: no git, no cargo, no GitHub")
    ap.add_argument("--max-perf", action="store_true", help="the slow build of a final release")
    ap.add_argument("--mge", default=MGE, help="the MGE XE repository")
    ap.add_argument("--repo", default="StormAtronach/nirender-plugin", help="the GitHub repository of the releases")
    ap.add_argument("--keep-old", action="store_true", help="do not delete the releases before this one")
    ap.add_argument("--no-build", action="store_true", help="use the archive that is in dist (it must be of this version)")
    ap.add_argument("--add", action="append", default=[], metavar="SOURCE=PATH", help="a file to put into the archive at PATH from its root")
    ap.add_argument("--add-dir", action="append", default=[], metavar="SOURCE=PATH", help="every file of the folder SOURCE, at PATH (. for the root) and their paths under SOURCE")
    ap.add_argument("--source", action="append", default=[], metavar="NAME=URL@COMMIT", help="the source of a part added with --add or --add-dir, for SOURCES.txt")
    ap.add_argument("--dry-run", action="store_true", help="pack the archive, publish and delete nothing")
    ap.add_argument("--no-gate", action="store_true", help="skip the replay gate on the packed helper; the notes must then hold its result by hand")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not args.notes:
        ap.error("--notes is required")

    assert os.path.isfile(args.notes), args.notes
    notes = open(args.notes, encoding="utf-8").read()
    problems = notes_problems(notes, gated=not args.no_gate)
    if problems:
        stop("the notes (tools/notes-template.md): %s" % "; ".join(problems), args.dry_run)
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
    expected_plugin = plugin_of_the_build(args.mge)
    if os.path.realpath(PLUGIN) != expected_plugin:
        stop("this tool runs from %s, but a build of %s takes the plugin from %s (xtask's plugin_dir, mgeHost64's path dependency): the archive would hold that checkout's plugin while MANIFEST.txt and the tag name this one; run the release from that checkout" % (PLUGIN, args.mge, expected_plugin), args.dry_run)
    mge_tag = render_link_tag(tag)
    if run(["git", "-C", args.mge, "tag", "--list", mge_tag], capture=True).strip():
        stop("the MGE XE repository has the tag %s already: set a new version in Cargo.toml" % mge_tag, args.dry_run)
    # A tag on origin that is not here would fail the push after the release is out.
    if run(["git", "-C", args.mge, "ls-remote", "--tags", "origin", "refs/tags/" + mge_tag], capture=True).strip():
        stop("origin of the MGE XE repository has the tag %s already: set a new version in Cargo.toml" % mge_tag, args.dry_run)
    for repo, source_paths in ((PLUGIN, PLUGIN_SOURCES), (args.mge, MGE_SOURCES)):
        clean_and_pushed(repo, source_paths, args.dry_run)

    # The license texts, the notices of the crates and the sources of the parts: before the
    # build, so that a stop here costs no build.
    additions += license_additions(args.mge, "https://github.com/" + args.repo, tag, sources, os.path.join(args.mge, "target", "release-licenses"))

    cargo = shutil.which("cargo") or os.path.join(os.path.expanduser("~"), ".cargo", "bin", "cargo.exe")
    if not args.no_build:
        # The content checks of both repositories, then the doc comments of the three crates
        # (no broken link), then no Clippy lint above its baseline.
        run([sys.executable, "-I", os.path.join(HERE, "checks.py"), "all", "--strict"])
        run([sys.executable, "-I", os.path.join(HERE, "docs.py"), "--mge", args.mge])
        run([sys.executable, "-I", os.path.join(HERE, "clippy_baseline.py"), "--check", "--mge", args.mge])
        unit_tests(args.mge)
        run([cargo, "xtask", "dist"] + (["--max-perf"] if args.max_perf else []), cwd=args.mge)
    # The version of MGE XE is in the name of the archive that xtask packs.
    kind = "" if args.max_perf else "-test"
    packed = [name for name in os.listdir(os.path.join(args.mge, "dist")) if re.fullmatch(r"MGE-XE-G7-v[0-9.]+%s\.7z" % re.escape(kind), name)]
    assert len(packed) == 1, "one archive of this kind in dist, found: %s" % packed
    mge_version = re.match(r"MGE-XE-G7-v([0-9.]+)", packed[0]).group(1)
    asset = os.path.join(args.mge, "dist", "MGE-XE-G7-v%s-nirender-%s.7z" % (mge_version, tag))
    shutil.copy2(os.path.join(args.mge, "dist", packed[0]), asset)
    if additions:
        add_to_archive(asset, additions, os.path.join(args.mge, "target", "release-additions"))

    # The manifest of the archive's files goes into it; then the archive's own SHA-256 goes
    # beside it and into the notes.
    unpacked = unpack(args.mge, asset)
    manifest = os.path.join(args.mge, "target", "MANIFEST.txt")
    write_text(manifest, manifest_text(os.path.basename(asset), provenance(args.mge, tag), file_digests(unpacked)))
    add_to_archive(asset, [(manifest, "MANIFEST.txt")], os.path.join(args.mge, "target", "release-manifest"))
    shutil.copy2(manifest, os.path.join(unpacked, "MANIFEST.txt"))
    digest = sha256_of(asset)
    checksum = asset + ".sha256"
    write_text(checksum, checksum_line(digest, os.path.basename(asset)))
    print("SHA-256 of %s: %s (in %s)" % (os.path.basename(asset), digest, checksum))

    # The gate on the helper that ships, and its result into the notes.
    values = {SHA_TOKEN: digest}
    if not args.no_gate:
        values[GATE_TOKEN] = gate_text(gate(unpacked, args.dry_run))
    notes_path = os.path.join(args.mge, "dist", "notes-%s.md" % tag)
    write_text(notes_path, fill_tokens(notes, values))
    print("the notes with the checksum and the gate's result: %s" % notes_path)

    # The commit of each repository: the build id that its binaries log, for the "Builds"
    # block of the notes (the trees are clean and pushed, so no id is "-dirty").
    for name, repo in (("nirender", PLUGIN), ("MGE XE G7", args.mge)):
        describe = run(["git", "-C", repo, "describe", "--always", "--dirty", "--exclude", "*"], capture=True).strip()
        print("build id: %s %s" % (name, describe))

    title = "Test build: MGE XE G7 %s with nirender %s" % (mge_version, plugin_version)
    if args.dry_run:
        print("dry run: the archive %s; not published (%s); no release deleted; the MGE XE tag %s not made" % (asset, title, mge_tag))
        return 0
    print(run(["gh", "release", "create", tag, "--repo", args.repo, "--prerelease", "--title", title, "--notes-file", notes_path, asset, checksum], capture=True).strip())
    # The commit of MGE XE that the archive was built from gets the release's tag too; it
    # goes up with the release, never before it.
    run(["git", "-C", args.mge, "tag", mge_tag])
    pushed = subprocess.run(["git", "-C", args.mge, "push", "origin", mge_tag], env=ENVIRONMENT)
    if pushed.returncode != 0:
        # The release is out; the tag is here. Say what to run, instead of a silent miss.
        raise SystemExit("the release %s is out, but the push of the MGE XE tag failed (%d); run by hand: git -C \"%s\" push origin %s" % (tag, pushed.returncode, args.mge, mge_tag))
    print("the MGE XE repository is tagged %s" % mge_tag)
    if not args.keep_old:
        for old in tags:
            run(["gh", "release", "delete", old, "--repo", args.repo, "--yes"])
            print("deleted the release %s (its tag stays)" % old)
    print("the release %s is the only one" % tag if not args.keep_old else "the release %s is out" % tag)
    return 0


if __name__ == "__main__":
    sys.exit(main())
