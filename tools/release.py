"""Makes a test build release on GitHub, and removes the releases before it.

    python tools/release.py --notes <file.md> [--max-perf] [--mge DIR] [--repo OWNER/NAME]
        [--keep-old] [--no-build] [--dry-run] [--add SOURCE=PATH ...] [--add-dir SOURCE=PATH ...]
        [--source NAME=URL@COMMIT ...] (--sign-key <the release key's .pub> | --no-sign) [--ssh-keygen EXE]
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
   for the slow build of a final release). `--no-build` skips only the build and takes the
   archive that is in dist; the checks and the tests still run.
   Each check of this step and of step 3 is a release check: its numbers come from its
   summary line and are judged by a fixed pass rule (`checks_numbers`, `docs_numbers`,
   `clippy_numbers`, `test_numbers`, `core_files_numbers`, `gate_numbers`; W604). A check
   whose output has no summary line fails, so a check that printed nothing cannot pass;
   `checks.py` must also have run and passed the MGE XE handshake and the self-tests of
   release.py, build_set.py and sets.py by name. Each check, its command, its numbers, its
   rule and its verdict (PASS, FAIL, NOT RUN, RECORD) go to `dist/checks-<tag>.txt`, written
   again after each check, so that a stop leaves the checks up to it; a FAIL stops the release.
   The file goes up with the release as an asset, beside the archive and not in it (the
   archive's SHA-256 must not depend on it), and its count of each verdict goes into the
   notes in place of the token `{checks}`.
3. The build ids of the binaries of the archive (akulakhan.dll, numidium.exe, d3d8.dll) must
   name the HEADs of the two trees, with no "-dirty" (the check "the build ids of the
   archive", `common.parts_problems`): the tag, MANIFEST.txt and the notes name the HEADs,
   and with `--no-build` the archive in dist can be of older commits. The tool prints those
   build ids for the notes. The archive goes up as
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
   helper in it, the one that ships. A failed gate stops the release, and so does a gate
   whose adapter word is not SAME: OTHER (the accepted pictures were made on another adapter
   or driver than this machine's), UNKNOWN (the helper printed no adapter, or the pictures
   name none) or no word. The pictures are the evidence of one GPU and one driver, so the
   baseline is accepted again on the release machine first (`validate_retained.py baseline --accept`).
   The adapter of its pictures (the GPU, the backend and the driver) and its result lines go
   into the notes in place of the token `{gate}` of their Tested block; the filled notes are
   written to `dist/notes-<tag>.md` and published. The notes follow
   `tools/notes-template.md`: their Tested block must hold the token (or, with `--no-gate`,
   the gate's result line marked "run by hand") and the run folder of the live checks through the mwse-mcp
   bridge (`YYYYmmdd-HHMMSS-label` under `results`), written by hand; a release stops at
   the start without them. `--no-gate` skips the gate.
   Before the gate, the archive gets `MANIFEST.txt`: the commits of the two repositories (as
   the binaries name them), the compiler, and the SHA-256, the size and the path of each of its
   files. The archive is then unpacked again, and its files must be those of MANIFEST.txt with
   their sizes and SHA-256, none missing and none extra (the check "MANIFEST.txt against the
   archive"). Then the SHA-256 of
   the archive itself goes beside it as `<archive>.sha256` (the format of `sha256sum`), up
   with the release as a second asset, and into the notes in place of the token `{sha256}`,
   so that a player can check a download.
   Then the signatures (W604 part D, the owner's D662): with `--sign-key <the release key's
   .pub>`, the Windows ssh-agent signs the archive and MANIFEST.txt (`ssh-keygen -Y sign -n
   nirender-release`, Windows' own OpenSSH by its path, `--ssh-keygen`; SSH_AUTH_SOCK out of
   its environment, so that it asks the Windows agent), and each signature is checked against
   `tools/release-signers` (the check "the signatures"). `<archive>.sig` and `MANIFEST.txt.sig`
   go up as assets, and the token `{signature}` of the notes gets the key's fingerprint, its line
   of release-signers and the command that checks a download. The tool reads no private key.
   Before the build it stops when the .pub is not one ed25519 key, when release-signers does not
   name that key for the principal nirender-release, or when the agent does not hold it
   (`ssh-add -L`): so ssh-keygen never asks for a passphrase. A release names `--sign-key` or
   `--no-sign` (a test build; the notes say "not signed"). A dry run signs nothing and asks no
   agent. What is published names the key by its fingerprint only (D316).
4. All other releases of the repository are deleted, so that a tester finds one build only.
   Their tags stay. `--keep-old` leaves the releases.

`--dry-run` does the steps up to the packed archive and publishes and deletes nothing; the
checks of step 1 then only print what would stop a release. A test of the archive's contents.

The tool starts no game.
"""
import argparse
import base64
import binascii
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
sys.path.insert(0, HERE)
from common import TEST_RESULT, part_ids, parts_problems  # noqa: E402
from validate_retained import ADAPTER_LINE, ADAPTER_OTHER, ADAPTER_SAME, ADAPTER_UNKNOWN, adapter_note  # noqa: E402


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


def unit_tests(mge, record):
    """Runs `cargo test` in the plugin, in the host crate and for the helper in the MGE XE
    repository, one check of `record` each. Each tree builds with the toolchain its
    rust-toolchain.toml pins (rustup reads it from the working folder; the plugin and MGE XE
    pin the same dated nightly, the host crate has none and takes the plugin's), and the
    version is printed first. A failed test stops the release."""
    trees = [("the plugin", PLUGIN, []), ("the host crate", os.path.join(PLUGIN, "host"), []), ("the helper", mge, ["-p", "mgeHost64"])]
    for name, folder, extra in trees:
        compiler = compiler_version(folder)
        print("the tests of %s, with %s" % (name, compiler), flush=True)
        record.run("cargo test, %s (%s)" % (name, compiler), ["cargo", "test"] + extra, folder, test_numbers)


# The verdicts of a release check (`CheckRecord`): RECORD is a number kept with no rule.
PASS, FAIL, NOT_RUN, RECORD = "PASS", "FAIL", "NOT RUN", "RECORD"
CHECKS_TOKEN = "{checks}"


class CheckRecord:
    """The release checks of one release, each with its command, its numbers, its pass rule and
    its verdict, in `dist/checks-<tag>.txt`. The file is written again after each check, so it
    holds the checks up to a stop. A check whose output has no numbers fails: a check that
    printed nothing cannot pass."""

    def __init__(self, path, tag, dry_run, roots):
        self.path, self.tag, self.dry_run, self.rows = path, tag, dry_run, []
        # The repositories by their names, for the public text of a command (`command_text`).
        self.roots = roots

    def add(self, name, command, numbers, rule, verdict):
        """Keeps one check and writes the file; a FAIL stops the release (a dry run prints it)."""
        self.rows.append((name, command, numbers, rule, verdict))
        write_text(self.path, checks_text(self.tag, self.rows))
        print("release check %s: %s (%s)" % (name, verdict, numbers), flush=True)
        if verdict == FAIL:
            stop("the release check %s failed: %s; the rule: %s (%s)" % (name, numbers, rule, self.path), self.dry_run)

    def run(self, name, command, cwd, judge):
        """Runs `command`, prints its output as it comes, and judges the output with `judge`
        (output, exit code) -> (numbers, rule, passed). Returns the output."""
        code, output = run_teed(command, cwd)
        numbers, rule, passed = judge(output, code)
        self.add(name, command_text(command, self.roots), numbers, rule, PASS if passed else FAIL)
        return output

    def summary(self):
        """The line of the notes (`{checks}`)."""
        return checks_summary(self.rows, os.path.basename(self.path))


def command_text(command, roots):
    """The text of `command` for the public checks file (D316: no user name, no local path): a
    program by its name ("python", "cargo"), a path inside a repository of `roots` ({name:
    folder}) relative to it, the folder of a repository by its name in angle brackets, any
    other absolute path by its file name."""
    def shown(argument, first):
        if not os.path.isabs(argument):
            return argument
        if first:
            return re.sub(r"\.exe$", "", os.path.basename(argument), flags=re.IGNORECASE).lower()
        full = os.path.normcase(os.path.realpath(argument))
        for name, root in roots.items():
            root = os.path.normcase(os.path.realpath(root))
            if full == root:
                return "<%s>" % name
            if full.startswith(root + os.sep):
                return os.path.relpath(full, root).replace("\\", "/")
        return os.path.basename(argument)
    return " ".join(shown(argument, index == 0) for index, argument in enumerate(command))


def run_teed(command, cwd=None):
    """Runs `command`, prints its output (stderr joined) as it comes, and returns its exit code
    and the output."""
    process = subprocess.Popen(command, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                               encoding="utf-8", errors="replace", env=ENVIRONMENT)
    lines = []
    for line in process.stdout:
        print(line, end="", flush=True)
        lines.append(line)
    return process.wait(), "".join(lines)


def checks_text(tag, rows):
    """The text of the checks file."""
    lines = ["THE RELEASE CHECKS OF %s" % tag, "",
             "Each check: its verdict (PASS, FAIL, NOT RUN, or RECORD for numbers with no rule), its command, its numbers",
             "and its pass rule. A check whose output has no numbers fails.", ""]
    for name, command, numbers, rule, verdict in rows:
        lines += ["== %s: %s" % (name, verdict), "command: %s" % command, "numbers: %s" % numbers, "rule: %s" % rule, ""]
    return "\n".join(lines)


def checks_summary(rows, name):
    """One line for the notes: the count of each verdict, the checks that did not run, the file."""
    counts = [(verdict, sum(1 for row in rows if row[4] == verdict)) for verdict in (PASS, FAIL, NOT_RUN)]
    text = ", ".join("%d %s" % (count, {PASS: "passed", FAIL: "FAILED", NOT_RUN: "not run"}[verdict]) for verdict, count in counts if count)
    not_run = [row[0] for row in rows if row[4] == NOT_RUN]
    if not_run:
        text += " (%s)" % "; ".join("%s: %s" % (row[0], row[2]) for row in rows if row[4] == NOT_RUN)
    return "the release checks: %s; their numbers and rules: %s" % (text or "none", name)


NO_NUMBERS = "no numbers: the output has no summary line"

# The names of `checks.py all` that a release needs by name: the version handshake against
# render-link and the self-tests of the release tools (the Architect's answer A5 to W604).
REQUIRED_CHECKS = ("mge handshake", "release", "build sets", "sets")
CHECKS_SUMMARY = re.compile(r"^checks \(\w+\): (\d+) passed.*$", re.MULTILINE)


def checks_numbers(output, code, listed):
    """Judges the output of `checks.py all --strict` by its summary line: (numbers, rule,
    passed). `listed` are the names of its list `all`."""
    rule = "failed = 0, not run = 0, skipped = 0, passed > 0, and these ran and passed: %s; exit 0" % ", ".join(REQUIRED_CHECKS)
    match = CHECKS_SUMMARY.search(output)
    if not match:
        return NO_NUMBERS, rule, False
    line = match.group(0)

    def named(pattern):
        found = re.search(pattern, line)
        return [name.strip() for name in found.group(1).split(",")] if found else []
    skipped = named(r"\d+ skipped \(not on this machine: ([^)]*)\)")
    not_run = named(r"\d+ NOT RUN \(--strict; not on this machine: ([^)]*)\)")
    failed = named(r"\d+ FAILED: ([^.]*)\.")
    passed = int(match.group(1))
    missing = [name for name in REQUIRED_CHECKS if name not in listed or name in skipped + not_run + failed]
    numbers = "passed %d, skipped %d, not run %d, failed %d" % (passed, len(skipped), len(not_run), len(failed))
    for label, names in (("skipped", skipped), ("not run", not_run), ("failed", failed), ("required and not passed", missing)):
        if names:
            numbers += "; %s: %s" % (label, ", ".join(names))
    return numbers, rule, code == 0 and passed > 0 and not (skipped or not_run or failed or missing)


DOCS_LINE = re.compile(r"^(.+?): exit (\d+), (\d+) warnings in our code, (\d+) other$", re.MULTILINE)


def docs_numbers(output, code):
    """Judges the output of `docs.py`: one line for each crate."""
    rule = "three crates (the plugin, the host crate, the helper), each exit 0 and 0 warnings in our code; exit 0"
    crates = DOCS_LINE.findall(output)
    if not crates:
        return NO_NUMBERS, rule, False
    numbers = "; ".join("%s: exit %s, %s warnings in our code, %s other" % crate for crate in crates)
    return numbers, rule, code == 0 and len(crates) == 3 and all(exit == "0" and ours == "0" for _, exit, ours, _ in crates)


CLIPPY_LINE = re.compile(r"^clippy: (\d+) warnings; (?:(\d+) lints above the baseline|none above the baseline)", re.MULTILINE)


def clippy_numbers(output, code, baseline_commit):
    """Judges the output of `clippy_baseline.py --check`; the last commit of the baseline file is
    a record, so that a stale baseline shows."""
    rule = "0 lints above tools/clippy.baseline; exit 0"
    match = CLIPPY_LINE.search(output)
    if not match:
        return NO_NUMBERS, rule, False
    above = int(match.group(2) or 0)
    numbers = "%s warnings, %d lints above the baseline; tools/clippy.baseline last changed in %s" % (match.group(1), above, baseline_commit)
    return numbers, rule, code == 0 and above == 0


TEST_IGNORED = re.compile(r"^test result: \w+\. \d+ passed; \d+ failed; (\d+) ignored", re.MULTILINE)


def test_numbers(output, code):
    """Judges the output of `cargo test`: the sums of its "test result" lines."""
    rule = "failed = 0, passed > 0; exit 0"
    results = TEST_RESULT.findall(output)
    if not results:
        return NO_NUMBERS, rule, False
    passed, failed = sum(int(p) for p, _ in results), sum(int(f) for _, f in results)
    ignored = sum(int(i) for i in TEST_IGNORED.findall(output))
    numbers = "passed %d, failed %d, ignored %d, in %d test binaries" % (passed, failed, ignored, len(results))
    return numbers, rule, code == 0 and failed == 0 and passed > 0


# The line of `cargo xtask dist` after its release check of the core files (render-link W609 9b).
CORE_FILES_LINE = re.compile(r"^The core files of the release: (\d+) with the build line of (.+?), equal to the repo's; numidium\.exe has the id$", re.MULTILINE)


def core_files_numbers(output, code, in_repo):
    """Judges the output of `cargo xtask dist` by its line of the core files: `in_repo` is the
    count of `assets/Data Files/shaders/core/*.wgsl` of the MGE XE tree."""
    rule = "the core files checked = the repo's core files (%d), > 0; exit 0" % in_repo
    match = CORE_FILES_LINE.search(output)
    if not match:
        return NO_NUMBERS, rule, False
    count = int(match.group(1))
    return "%d core files with the build line of %s" % (count, match.group(2)), rule, code == 0 and count == in_repo and count > 0


def gate_numbers(output, code):
    """Judges the output of the gate: each result line N of M with N = M > 0, and each adapter
    word SAME. Returns (numbers, rule, passed, the result lines, the adapters)."""
    rule = "each stage N of M pictures equal with N = M > 0; each adapter word %s; exit 0" % ADAPTER_SAME
    lines = GATE_RESULT.findall(output)
    adapters = adapters_of(output)
    if not lines:
        return NO_NUMBERS, rule, False, lines, adapters
    counts = [re.match(r"(\d+) of (\d+)", line).groups() for line in lines]
    words = sorted({word for word, _ in ADAPTER_LINE.findall(output)})
    numbers = "%s; adapter words: %s; %s" % ("; ".join("%s of %s" % count for count in counts), ", ".join(words) or "none",
                                            " and ".join(adapters) or "no adapter")
    passed = code == 0 and all(n == m and int(m) > 0 for n, m in counts) and adapter_problem(output) is None
    return numbers, rule, passed, lines, adapters


def adapters_of(output):
    """The adapters of the gate's SAME lines, without the words after them ("as the accepted
    pictures"): the GPU, the backend and the driver of the evidence."""
    return sorted({re.sub(r", as the accepted pictures$", "", text) for word, text in ADAPTER_LINE.findall(output) if word == ADAPTER_SAME})


def build_id_numbers(folder, plugin_head, mge_head):
    """Judges the build ids of the binaries of the unpacked archive against the HEADs of the two
    trees: the tag, MANIFEST.txt and the notes name the HEADs."""
    rule = "each binary names the HEAD of its tree, one commit for each repository, no -dirty"
    ids, _ = part_ids(folder)
    _, problems = parts_problems(folder, plugin_head, mge_head)
    numbers = "%s; HEAD: nirender-plugin %s, render-link %s" % (
        ", ".join("%s %s" % item for item in sorted(ids.items())) or "no build id", plugin_head, mge_head)
    if problems:
        numbers += "; " + "; ".join(problems)
    return numbers, rule, not problems


MANIFEST_LINE = re.compile(r"^([0-9a-f]{64})  (\d+)  (.+)$", re.MULTILINE)


def manifest_differences(manifest, entries):
    """The differences between the text of MANIFEST.txt and the files of an unpacked archive
    (`file_digests`), MANIFEST.txt itself out: each (path, what)."""
    listed = {path: (int(size), digest) for digest, size, path in MANIFEST_LINE.findall(manifest)}
    found = {path: (size, digest) for path, size, digest in entries if path != "MANIFEST.txt"}
    return ([(path, "not in the archive") for path in sorted(listed) if path not in found]
            + [(path, "not in MANIFEST.txt") for path in sorted(found) if path not in listed]
            + [(path, "another size or SHA-256") for path in sorted(listed) if path in found and listed[path] != found[path]])


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


def gate_command(folder):
    """The command of the gate on the unpacked archive `folder`. The archive has no manifest, so it goes as --archive
    (W545): replay.py refuses a folder that is not a set under --mge-build."""
    return [sys.executable, "-I", os.path.join(HERE, "validate_retained.py"), "baseline", "--archive", folder]


def gate(folder, record):
    """Replays the captures of the baseline on the helper in the unpacked archive
    (`validate_retained.py baseline --archive <folder>`: the helper that ships, not the one
    in bin), one check of `record`. Returns the result lines ("N of M pictures equal to the
    accepted ones in ...") and the adapters of the pictures; a failed gate stops the release."""
    print("the gate on the helper of %s" % folder, flush=True)
    command = gate_command(folder)
    code, output = run_teed(command, PLUGIN)
    numbers, rule, passed, lines, adapters = gate_numbers(output, code)
    record.add("the capture gate", command_text(command, record.roots), numbers, rule, PASS if passed else FAIL)
    # The pictures are the evidence of one GPU and one driver: on another adapter a pass says
    # nothing about this machine's build, and a fail is not a finding.
    problem = adapter_problem(output)
    if problem:
        stop(problem, record.dry_run)
    return lines, adapters


def adapter_problem(output):
    """Why the gate's result is not the release's evidence, from the fixed word of the adapter
    line of `validate_retained.py baseline` (`ADAPTER_LINE`): the pictures are the evidence of
    one GPU and one driver, so a release needs the word SAME. OTHER (another adapter or
    driver), UNKNOWN (no adapter printed, or none recorded) and no word at all are problems.
    None when every adapter line says SAME."""
    words = ADAPTER_LINE.findall(output)
    if not words:
        return "the gate printed no adapter word (%s, %s or %s), so the release cannot know the GPU of its pictures" % (ADAPTER_SAME, ADAPTER_OTHER, ADAPTER_UNKNOWN)
    for word, text in words:
        if word != ADAPTER_SAME:
            return ("the gate's adapter is %s (%s), and a release needs %s; accept the baseline again on this machine "
                    "(validate_retained.py baseline --accept) before the release" % (word, text, ADAPTER_SAME))
    return None


def gate_text(lines, adapters):
    """The gate's result for the notes: the adapter of its pictures (the GPU, the backend and
    the driver), then its result lines without the local folders they end with ("in <path>")."""
    return "the capture gate on %s: %s" % (" and ".join(adapters) or "an adapter it did not name",
                                          "; ".join(re.sub(r" in \S+$", "", line) for line in lines))


SHA_TOKEN = "{sha256}"


def fill_tokens(notes, values):
    """The notes with each token (`{gate}`, `{sha256}`, `{checks}`) replaced by its value."""
    for token, value in values.items():
        notes = notes.replace(token, value)
    return notes


def notes_problems(notes, gated):
    """What is missing from the notes: the Tested block's parts, the token `{sha256}` for the
    archive's checksum, the token `{checks}` for the line of the release checks, and the token
    `{signature}` for the line of the signature."""
    problems = tested_block_problems(notes, gated)
    if SHA_TOKEN not in notes:
        problems.append("no %s token for the archive's SHA-256" % SHA_TOKEN)
    if CHECKS_TOKEN not in notes:
        problems.append("no %s token for the line of the release checks" % CHECKS_TOKEN)
    if SIGNATURE_TOKEN not in notes:
        problems.append("no %s token for the line of the signature" % SIGNATURE_TOKEN)
    if not gated and GATE_TOKEN in notes:
        problems.append("the token %s with --no-gate: write the gate's result by hand" % GATE_TOKEN)
    return problems


# The signature of a release (W604 part D, the owner's D662): the owner's ed25519 key, held by the Windows ssh-agent,
# signs the archive and its MANIFEST.txt with `ssh-keygen -Y sign`; tools/release-signers (the allowed signers format
# of ssh-keygen) names that key for `ssh-keygen -Y verify`. The tool never reads a private key: it names the key by
# its public file, and the agent signs. What is published names the key by its fingerprint only (D316).
SIGN_PRINCIPAL = "nirender-release"
SIGN_NAMESPACE = "nirender-release"
SIGNERS = os.path.join(HERE, "release-signers")
SIGNATURE_TOKEN = "{signature}"
# The programs of Windows' OpenSSH, by their paths: Git's ssh-keygen is first on PATH and does not use the Windows
# agent [I]. Without SSH_AUTH_SOCK they ask the Windows agent's own pipe (`agent_environment`).
SSH_KEYGEN = os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32", "OpenSSH", "ssh-keygen.exe")
WINDOWS_AGENT = r"\\.\pipe\openssh-ssh-agent"


def agent_environment(environment):
    """The environment of the calls that use the agent (ssh-add -L, ssh-keygen -Y sign): `environment` without
    SSH_AUTH_SOCK, so that Windows' OpenSSH asks the Windows agent (WINDOWS_AGENT), not a socket that a Git Bash shell
    set (verdict 756 5.1)."""
    return {key: value for key, value in environment.items() if key != "SSH_AUTH_SOCK"}


def public_key(text):
    """The one ed25519 public key of the text of a .pub file, as (type, blob), or None."""
    lines = [line for line in text.splitlines() if line.strip()]
    if len(lines) != 1:
        return None
    fields = lines[0].split()
    if len(fields) < 2 or fields[0] != "ssh-ed25519":
        return None
    try:
        base64.b64decode(fields[1], validate=True)
    except (binascii.Error, ValueError):
        return None
    return fields[0], fields[1]


def fingerprint(key):
    """The SHA256 fingerprint of a (type, blob) key, as `ssh-keygen -l` prints it."""
    return "SHA256:" + base64.b64encode(hashlib.sha256(base64.b64decode(key[1])).digest()).decode("ascii").rstrip("=")


def signer_keys(text, principal=SIGN_PRINCIPAL):
    """The keys (type, blob) of `principal` in the text of an allowed signers file: each line is "<principals>
    [options] <type> <blob> [comment]", the principals split by commas; a line that starts with '#' is a comment."""
    keys = []
    for line in text.splitlines():
        fields = line.split()
        if not fields or fields[0].startswith("#") or principal not in fields[0].split(","):
            continue
        for index in range(1, len(fields) - 1):
            if fields[index].startswith("ssh-"):
                keys.append((fields[index], fields[index + 1]))
                break
    return keys


def signers_line(key):
    """The line of an allowed signers file that names `key` for the release's principal and namespace."""
    return '%s namespaces="%s" %s %s' % (SIGN_PRINCIPAL, SIGN_NAMESPACE, key[0], key[1])


def sign_options_problem(sign_key, no_sign):
    """The stop line for the signing options, or None: a release names its key or says it is not signed."""
    if sign_key and no_sign:
        return "--sign-key and --no-sign together: name one"
    if not sign_key and not no_sign:
        return "no --sign-key (the public file of the release key) and no --no-sign: a release is signed, or says that it is not"
    return None


def sign_key_problems(pub_text, signers_text):
    """The key of the public file's text and what stops a signed release before its build: the file is not one ed25519
    key, or release-signers does not name that key for the principal. The lines name keys by fingerprint only."""
    key = public_key(pub_text)
    if key is None:
        return None, ["--sign-key does not name one ed25519 public key (a .pub file)"]
    allowed = signer_keys(signers_text)
    if key not in allowed:
        return key, ["tools/release-signers does not name the release key %s for %s (it names %s)" % (
            fingerprint(key), SIGN_PRINCIPAL, ", ".join(fingerprint(other) for other in allowed) or "no key")]
    return key, []


def agent_problem(key, listing, code):
    """The stop line when the Windows agent does not hold `key`, or None. `listing` and `code` are the output and the
    exit code of `ssh-add -L` (exit 1: no keys; exit 2: no agent). The line names the agent and the fingerprint."""
    held = code == 0 and any(line.split()[1:2] == [key[1]] for line in listing.splitlines())
    if held:
        return None
    return "the Windows ssh-agent (%s) does not hold the release key %s; load it: ssh-add <the private key file>" % (
        WINDOWS_AGENT, fingerprint(key))


def ssh_add_of(ssh_keygen):
    """The ssh-add.exe beside an ssh-keygen.exe."""
    return os.path.join(os.path.dirname(ssh_keygen), "ssh-add.exe")


def sign_file(ssh_keygen, key_file, path, environment):
    """Signs `path` into `<path>.sig` with `ssh-keygen -Y sign`, its input closed so that it can ask for nothing. With
    a public key file the agent signs; the self-test passes a private key file. Returns the exit code and the text."""
    if os.path.exists(path + ".sig"):
        os.remove(path + ".sig")
    result = subprocess.run([ssh_keygen, "-Y", "sign", "-n", SIGN_NAMESPACE, "-f", key_file, path], stdin=subprocess.DEVNULL,
                            capture_output=True, text=True, env=environment)
    return result.returncode, result.stdout + result.stderr


def without_path(text, key_file):
    """ssh-keygen's text on one line, with the key file's path (in either slash form) as "<the key file>" (D316)."""
    for form in {key_file, key_file.replace("\\", "/"), key_file.replace("/", "\\"), os.path.abspath(key_file)}:
        text = text.replace(form, "<the key file>")
    return " ".join(text.split())


def verify_command(ssh_keygen, signers, path, principal=SIGN_PRINCIPAL, namespace=SIGN_NAMESPACE):
    """The command that checks `<path>.sig` against `signers`; `path` itself is its input."""
    return [ssh_keygen, "-Y", "verify", "-f", signers, "-I", principal, "-n", namespace, "-s", path + ".sig"]


def verify_file(command, path):
    """Runs a verify command with `path` as its input. Returns the exit code and the text."""
    with open(path, "rb") as source:
        result = subprocess.run(command, stdin=source, capture_output=True, env=ENVIRONMENT)
    return result.returncode, (result.stdout + result.stderr).decode("utf-8", "replace")


def good_signature(code, text):
    """Whether a verify passed: exit 0 and ssh-keygen's line for the namespace and the principal."""
    return code == 0 and ('Good "%s" signature for %s' % (SIGN_NAMESPACE, SIGN_PRINCIPAL)) in text


def signature_numbers(key, results):
    """The numbers, the rule and the pass of the check "the signatures": `results` are (file name, exit code, text)."""
    numbers = "the key %s; %s" % (fingerprint(key), "; ".join(
        "%s: %s" % (name, "good" if good_signature(code, text) else "NOT GOOD (exit %d: %s)" % (code, " ".join(text.split())[:200]))
        for name, code, text in results))
    rule = 'each file: exit 0 and "Good "%s" signature for %s" against tools/release-signers' % (SIGN_NAMESPACE, SIGN_PRINCIPAL)
    return numbers, rule, bool(results) and all(good_signature(code, text) for _, code, text in results)


def player_command(archive_name):
    """The command of the notes that checks a download, with the signers file saved beside it as release-signers, for
    cmd and Git Bash (its input from the file with '<')."""
    return 'ssh-keygen -Y verify -f release-signers -I %s -n %s -s "%s.sig" < "%s"' % (SIGN_PRINCIPAL, SIGN_NAMESPACE, archive_name, archive_name)


def player_command_powershell(archive_name):
    """The same command for Windows PowerShell, which refuses '<' (verdict 757 4.1): cmd runs it."""
    return "cmd /c '%s'" % player_command(archive_name)


def agent_check(ssh_keygen, key, dry_run):
    """Stops when the Windows agent does not hold `key` (`ssh-add -L`, SSH_AUTH_SOCK out). Before the build, and again
    right before the signatures: a key that left the agent in between would make ssh-keygen load the private file
    beside the .pub and ask for its passphrase on the console (verdict 757 4.2)."""
    listed = subprocess.run([ssh_add_of(ssh_keygen), "-L"], stdin=subprocess.DEVNULL, capture_output=True, text=True,
                            env=agent_environment(ENVIRONMENT))
    problem = agent_problem(key, listed.stdout, listed.returncode)
    if problem:
        stop(problem, dry_run)


def signature_text(key, repo, tag, archive_name, reason=None):
    """The line of the notes (`{signature}`): the key by its fingerprint, its line of release-signers, where that file
    is at the tag, and the command that checks the download; or `reason` for a release that is not signed."""
    if key is None:
        return "not signed (%s)" % (reason or "a test build")
    return ("signed with the release key `%s` (principal `%s`): `<archive>.sig` and `MANIFEST.txt.sig`. To check the "
            "download, save this line as the file `release-signers` beside it: `%s` (the same line is "
            "https://github.com/%s/blob/%s/tools/release-signers), then run in cmd or Git Bash `%s`, or in "
            "PowerShell `%s`" % (fingerprint(key), SIGN_PRINCIPAL, signers_line(key), repo, tag, player_command(archive_name),
                                 player_command_powershell(archive_name)))


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


def head_of(repo):
    """The short hash of the HEAD of `repo`."""
    return run(["git", "-C", repo, "rev-parse", "--short", "HEAD"], capture=True).strip()


def provenance(folder, mge, plugin_tag):
    """The commits that the binaries of the unpacked archive `folder` name (their build ids) and
    the compiler of the MGE XE tree, in one line. The release check of the build ids proved them
    the HEADs of the two trees."""
    ids, _ = part_ids(folder)
    return "MGE XE G7 at %s; nirender at %s (the tag %s); built with %s." % (
        ids.get("numidium", "unknown"), ids.get("nirender", "unknown"), plugin_tag, compiler_version(mge))


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


def release_checks_self_test(gate_output, nvidia, amd):
    """The readers and the rules of the release checks (W604): for each, a passing output, a
    failing one, and one with no summary line, which fails."""
    listed = ["readme", "mge handshake", "release", "build sets", "sets"]
    good = "== a line\nchecks (all): 61 passed\n"
    assert checks_numbers(good, 0, listed)[0::2] == ("passed 61, skipped 0, not run 0, failed 0", True)
    assert not checks_numbers(good, 1, listed)[2], "a summary of passes with exit code 1"
    failed = "checks (all): 59 passed, 2 FAILED: readme, sets. Fix the drift, or commit with --no-verify.\n"
    numbers, _, passed = checks_numbers(failed, 1, listed)
    assert not passed and numbers == "passed 59, skipped 0, not run 0, failed 2; failed: readme, sets; required and not passed: sets", numbers
    not_run = "checks (all): 60 passed, 1 NOT RUN (--strict; not on this machine: mge handshake)\n"
    assert checks_numbers(not_run, 5, listed)[0].endswith("not run: mge handshake; required and not passed: mge handshake")
    assert not checks_numbers(not_run, 5, listed)[2]
    # A required check that the list no longer has fails, although every check passed.
    assert not checks_numbers(good, 0, ["readme", "release", "build sets", "sets"])[2]
    assert checks_numbers("== the check 'x' failed\n", 1, listed)[0::2] == (NO_NUMBERS, False)

    docs = "the plugin (akulakhan): exit 0, 0 warnings in our code, 0 other\nthe host crate (nirender_wgpu): exit 0, 0 warnings in our code, 0 other\nthe helper (numidium): exit 0, 0 warnings in our code, 2 other\n"
    assert docs_numbers(docs, 0)[2]
    assert not docs_numbers(docs.replace("exit 0, 0 warnings in our code, 2", "exit 0, 1 warnings in our code, 2"), 0)[2]
    assert not docs_numbers(docs.split("\n", 1)[1], 0)[2], "two crates"
    assert not docs_numbers(docs, 1)[2]
    assert docs_numbers("wrote docs.html\n", 0)[0::2] == (NO_NUMBERS, False)

    assert clippy_numbers("clippy: 317 warnings; none above the baseline\n", 0, "abc1234 2026-11-20") == (
        "317 warnings, 0 lints above the baseline; tools/clippy.baseline last changed in abc1234 2026-11-20", "0 lints above tools/clippy.baseline; exit 0", True)
    assert not clippy_numbers("clippy: 320 warnings; 2 lints above the baseline\n", 1, "x")[2]
    assert clippy_numbers("clippy: render-link does not compile\n", 1, "x")[0::2] == (NO_NUMBERS, False)

    tests = "test result: ok. 520 passed; 0 failed; 18 ignored; 0 measured; 0 filtered out\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured\n"
    assert test_numbers(tests, 0)[0::2] == ("passed 521, failed 0, ignored 18, in 2 test binaries", True)
    assert not test_numbers(tests.replace("1 passed; 0 failed", "0 passed; 1 failed"), 101)[2]
    assert not test_numbers("test result: ok. 0 passed; 0 failed; 0 ignored\n", 0)[2], "no test ran"
    assert test_numbers("error: could not compile\n", 101)[0::2] == (NO_NUMBERS, False)

    core = "The core files of the release: 12 with the build line of numidium v0.21.0 abc1234, equal to the repo's; numidium.exe has the id\n"
    assert core_files_numbers(core, 0, 12)[0::2] == ("12 core files with the build line of numidium v0.21.0 abc1234", True)
    assert not core_files_numbers(core, 0, 13)[2]
    assert core_files_numbers("Packed dist\\x.7z\n", 0, 12)[0::2] == (NO_NUMBERS, False)

    gate_lines = gate_output(nvidia, nvidia) + "  ok   206 of 206 pictures equal to the accepted ones in b\n"
    numbers, _, passed, lines, adapters = gate_numbers(gate_lines, 0)
    assert passed and numbers == "2 of 2; 206 of 206; adapter words: SAME; %s" % nvidia and len(lines) == 2 and adapters == [nvidia], numbers
    assert not gate_numbers(gate_lines.replace("206 of 206", "205 of 206"), 1)[2]
    assert not gate_numbers(gate_output(nvidia, amd), 0)[2], "an adapter that is not SAME"
    assert not gate_numbers(gate_output(nvidia, nvidia).replace("2 of 2", "0 of 0"), 0)[2], "no picture"
    assert gate_numbers("the gate ended\n", 1)[:3:2] == (NO_NUMBERS, False)

    # The commands of the public checks file name no user and no local path (D316; verdict 738, 4.1): the commands of
    # a release as main makes them, with this machine's real folders.
    mge = os.path.join(os.path.dirname(PLUGIN), "MGE-XE-G7-render-link")
    roots = {"nirender-plugin": PLUGIN, "render-link": mge}
    cargo = os.path.join(os.path.expanduser("~"), ".cargo", "bin", "cargo.EXE")
    commands = [[sys.executable, "-I", os.path.join(HERE, "checks.py"), "all", "--strict"],
                [sys.executable, "-I", os.path.join(HERE, "docs.py"), "--mge", mge],
                [sys.executable, "-I", os.path.join(HERE, "clippy_baseline.py"), "--check", "--mge", mge],
                ["cargo", "test", "-p", "mgeHost64"], [cargo, "xtask", "dist", "--max-perf"],
                gate_command(os.path.join(mge, "target", "release-gate"))]
    texts = [command_text(command, roots) for command in commands]
    assert texts == ["python -I tools/checks.py all --strict", "python -I tools/docs.py --mge <render-link>",
                     "python -I tools/clippy_baseline.py --check --mge <render-link>", "cargo test -p mgeHost64",
                     "cargo xtask dist --max-perf", "python -I tools/validate_retained.py baseline --archive target/release-gate"], texts
    assert command_text(["git", os.path.join(os.path.expanduser("~"), "elsewhere", "x.txt")], roots) == "git x.txt"
    public = checks_text("v1", [("check %d" % index, text, "n", "r", PASS) for index, text in enumerate(texts)])
    assert os.path.expanduser("~") not in public and not re.search(r"\b[A-Za-z]:[\\/]", public), public
    rows = [("a", "x", "n", "r", PASS), ("b", "y", "--no-build: the archive in dist", "the build", NOT_RUN), ("c", "z", "m", "-", RECORD)]
    assert checks_summary(rows, "checks-v1.txt") == "the release checks: 1 passed, 1 not run (b: --no-build: the archive in dist); their numbers and rules: checks-v1.txt"
    assert checks_text("v1", rows[:1]).endswith("== a: PASS\ncommand: x\nnumbers: n\nrule: r\n")

    manifest = manifest_text("x.7z", "p", [("a.dll", 3, "ab" * 32), ("Data Files/b.dds", 5, "cd" * 32)])
    assert manifest_differences(manifest, [("a.dll", 3, "ab" * 32), ("Data Files/b.dds", 5, "cd" * 32), ("MANIFEST.txt", 9, "ef" * 32)]) == []
    assert manifest_differences(manifest, [("a.dll", 3, "00" * 32), ("c.txt", 1, "11" * 32)]) == [
        ("Data Files/b.dds", "not in the archive"), ("c.txt", "not in MANIFEST.txt"), ("a.dll", "another size or SHA-256")]

    # The build ids against the HEADs, on planted dist folders (common.parts_problems is the rule).
    import tempfile
    with tempfile.TemporaryDirectory() as root:
        def plant(plugin, host, mge, d3d8):
            for path, text in ((os.path.join(root, "Data Files", "mwse", "lib", "akulakhan.dll"), b"x nirender v0.3.0 %s y" % plugin),
                               (os.path.join(root, "numidium.exe"), b"nirender-host v0.3.0 %s z numidium v0.21.0 %s" % (host, mge)),
                               (os.path.join(root, "d3d8.dll"), b"MGE-XE G7 Fork v0.21.0 %s" % d3d8)):
                os.makedirs(os.path.dirname(path), exist_ok=True)
                open(path, "wb").write(text)
        plant(b"79c752d", b"79c752d", b"c610f3b", b"c610f3b")
        numbers, _, passed = build_id_numbers(root, "79c752d", "c610f3b")
        assert passed and numbers == "MGE-XE G7 Fork c610f3b, nirender 79c752d, nirender-host 79c752d, numidium c610f3b; HEAD: nirender-plugin 79c752d, render-link c610f3b", numbers
        numbers, _, passed = build_id_numbers(root, "79c752d", "99c8b62")
        assert not passed and numbers.endswith("numidium is c610f3b, but the commit of render-link under test is 99c8b62"), numbers
        plant(b"79c752d", b"79c752d", b"c610f3b-dirty", b"c610f3b-dirty")
        assert not build_id_numbers(root, "79c752d", "c610f3b")[2]
        os.remove(os.path.join(root, "d3d8.dll"))
        assert "d3d8.dll is not in the build" in build_id_numbers(root, "79c752d", "c610f3b")[0]


def test_signature_rules():
    """The pure parts of the signature (W604 part D): no program, no agent, no key file."""
    blob = base64.b64encode(b"\0\0\0\x0bssh-ed25519\0\0\0\x20" + bytes(range(32))).decode("ascii")
    other = base64.b64encode(b"\0\0\0\x0bssh-ed25519\0\0\0\x20" + bytes(range(1, 33))).decode("ascii")
    key = ("ssh-ed25519", blob)
    assert public_key("ssh-ed25519 %s nirender-release\n" % blob) == key
    assert public_key("ssh-rsa %s x\n" % blob) is None and public_key("") is None and public_key("ssh-ed25519 !!\n") is None
    assert public_key("ssh-ed25519 %s a\nssh-ed25519 %s b\n" % (blob, other)) is None, "two keys in one .pub"
    assert fingerprint(key) == "SHA256:" + base64.b64encode(hashlib.sha256(base64.b64decode(blob)).digest()).decode().rstrip("=")
    # release-signers: the key of the principal; comments, another principal and the repository's file with no key.
    signers = "# a comment\n%s\nsomeone ssh-ed25519 %s\n" % (signers_line(key), other)
    assert signer_keys(signers) == [key] and signer_keys("a,nirender-release ssh-ed25519 %s\n" % blob) == [key]
    assert signer_keys(open(SIGNERS, encoding="utf-8").read()) == [], "tools/release-signers names a key before the owner sent it"
    assert sign_key_problems("ssh-ed25519 %s\n" % blob, signers) == (key, [])
    found, problems = sign_key_problems("ssh-ed25519 %s\n" % other, signers)
    assert found and problems == ["tools/release-signers does not name the release key %s for nirender-release (it names %s)" % (
        fingerprint(found), fingerprint(key))], problems
    assert sign_key_problems("ssh-ed25519 %s\n" % blob, open(SIGNERS, encoding="utf-8").read())[1][0].endswith("(it names no key)")
    assert sign_key_problems("not a key", signers) == (None, ["--sign-key does not name one ed25519 public key (a .pub file)"])
    # The options: one of the two.
    assert sign_options_problem("k.pub", False) is None and sign_options_problem(None, True) is None
    assert sign_options_problem("k.pub", True).startswith("--sign-key and --no-sign") and sign_options_problem(None, False).startswith("no --sign-key")
    # The agent's keys (ssh-add -L): held; another key only; no keys (exit 1); no agent (exit 2).
    assert agent_problem(key, "ssh-ed25519 %s nirender-release\n" % blob, 0) is None
    for listing, code in (("ssh-ed25519 %s x\n" % other, 0), ("The agent has no identities.\n", 1), ("Error connecting to agent: No such file\n", 2)):
        assert agent_problem(key, listing, code) == ("the Windows ssh-agent (\\\\.\\pipe\\openssh-ssh-agent) does not hold the release key %s; "
                                                     "load it: ssh-add <the private key file>" % fingerprint(key)), (listing, code)
    # The agent calls ask the Windows agent: SSH_AUTH_SOCK out, the rest kept (verdict 756 5.1).
    assert agent_environment({"SSH_AUTH_SOCK": "/tmp/ssh-x/agent.1", "PATH": "p"}) == {"PATH": "p"}
    # The notes' line and the verify.
    line = signature_text(key, "o/r", "v1.2.3", "a.7z")
    assert signers_line(key) in line and "https://github.com/o/r/blob/v1.2.3/tools/release-signers" in line and fingerprint(key) in line
    command = 'ssh-keygen -Y verify -f release-signers -I nirender-release -n nirender-release -s "a.7z.sig" < "a.7z"'
    assert line.endswith("run in cmd or Git Bash `%s`, or in PowerShell `cmd /c '%s'`" % (command, command)), line
    assert signature_text(None, "o/r", "v1", "a.7z") == "not signed (a test build)"
    assert signature_text(None, "o/r", "v1", "a.7z", "a dry run") == "not signed (a dry run)"
    assert good_signature(0, 'Good "nirender-release" signature for nirender-release with ED25519 key SHA256:x\n')
    assert not good_signature(0, "") and not good_signature(255, 'Good "nirender-release" signature for nirender-release')
    numbers, _, passed = signature_numbers(key, [("a.7z", 0, 'Good "nirender-release" signature for nirender-release'), ("MANIFEST.txt", 255, "Could not verify signature.")])
    assert not passed and "a.7z: good; MANIFEST.txt: NOT GOOD (exit 255: Could not verify signature.)" in numbers, numbers
    assert not signature_numbers(key, [])[2], "no file signed is no pass"
    # No user folder or drive letter in what is published or stopped on (verdict 756 5.3): a key file under a planted
    # user folder, and the text of the checks file's row.
    planted = "C:\\Users\\someone\\.ssh\\release.pub"
    local = re.compile(r"(?<![A-Za-z])[A-Za-z]:[\\/]|\\Users\\|/Users/|\.ssh")
    roots = {"nirender-plugin": PLUGIN, "render-link": MGE}
    row = command_text(verify_command(SSH_KEYGEN, SIGNERS, os.path.join(MGE, "dist", "a.7z")), roots)
    assert row == 'ssh-keygen -Y verify -f tools/release-signers -I nirender-release -n nirender-release -s dist/a.7z.sig', row
    texts = [line, row, agent_problem(key, "", 1), sign_key_problems("ssh-ed25519 %s\n" % other, signers)[1][0],
             without_path("Load key \"%s\": invalid format" % planted, planted), without_path("couldn't load C:/Users/someone/.ssh/release.pub", planted)]
    assert not [text for text in texts if local.search(text)], [text for text in texts if local.search(text)]


def test_signature_live():
    """A signature made and checked by Windows' ssh-keygen (W604 part D) with a throwaway key in a temp folder,
    signed with the private key file directly: no agent and no ~/.ssh (verdict 737 5.1), and SSH_AUTH_SOCK names a pipe
    that is not there, so no agent can answer. The notes' command must pass (the null) and fail for a changed byte, for
    another principal and for another namespace. A failed call shows ssh-keygen's text (verdict 756 5.5)."""
    import shlex
    import tempfile
    assert os.path.isfile(SSH_KEYGEN), "no ssh-keygen of Windows' OpenSSH at %s" % SSH_KEYGEN
    folder = tempfile.mkdtemp(prefix="release-sign-test-")
    try:
        key_file = os.path.join(folder, "key")
        made = subprocess.run([SSH_KEYGEN, "-q", "-t", "ed25519", "-N", "", "-C", SIGN_PRINCIPAL, "-f", key_file], stdin=subprocess.DEVNULL,
                              capture_output=True, text=True)
        assert made.returncode == 0, "ssh-keygen made no key (%d): %s" % (made.returncode, made.stdout + made.stderr)
        with open(key_file + ".pub", encoding="utf-8") as pub:
            key = public_key(pub.read())
        assert key, "the throwaway .pub is not one ed25519 key"
        archive = os.path.join(folder, "MGE-XE-G7-v1.0-nirender-v0.1.0.7z")
        with open(archive, "wb") as out:
            out.write(bytes(range(256)) * 64)
        no_agent = dict(agent_environment(ENVIRONMENT), SSH_AUTH_SOCK=r"\\.\pipe\release-sign-test-no-agent-%d" % os.getpid())
        code, text = sign_file(SSH_KEYGEN, key_file, archive, no_agent)
        assert code == 0 and os.path.isfile(archive + ".sig"), "ssh-keygen -Y sign (%d): %s" % (code, text)
        # The notes' key line as the file release-signers, and the notes' command as the notes give it.
        line = signature_text(key, "o/r", "v0.1.0", os.path.basename(archive))
        with open(os.path.join(folder, "release-signers"), "w", encoding="utf-8") as out:
            out.write(signers_line(key) + "\n")
        quoted = line.split("`")[1::2]
        cmd_form = [text for text in quoted if text.startswith("ssh-keygen -Y verify")]
        powershell_form = [text for text in quoted if text.startswith("cmd /c ")]
        assert len(cmd_form) == 1 and len(powershell_form) == 1, quoted
        words = shlex.split(cmd_form[0])
        assert words[0] == "ssh-keygen" and words[-2] == "<", words
        command = [SSH_KEYGEN] + [os.path.join(folder, word) if word in ("release-signers", os.path.basename(archive) + ".sig") else word for word in words[1:-2]]
        code, text = verify_file(command, os.path.join(folder, words[-1]))
        assert good_signature(code, text), "the notes' command on the signed file (%d): %s" % (code, text)
        # The fingerprint of the notes and the checks file is the one ssh-keygen gives.
        assert fingerprint(key) in text, (fingerprint(key), text)
        # The release's own verify against a signers file: the same pass.
        assert good_signature(*verify_file(verify_command(SSH_KEYGEN, os.path.join(folder, "release-signers"), archive), archive))
        # Another principal, another namespace: no pass.
        signers = os.path.join(folder, "release-signers")
        for wrong in (verify_command(SSH_KEYGEN, signers, archive, principal="someone"), verify_command(SSH_KEYGEN, signers, archive, namespace="file")):
            code, text = verify_file(wrong, archive)
            assert code != 0 and not good_signature(code, text), "a verify with %s passed: %s" % (wrong, text)
        # The notes' PowerShell form, in Windows PowerShell in the download's folder (verdict 757 4.1); the command goes
        # in as -EncodedCommand, so that no quote of it changes on the way.
        def in_powershell():
            encoded = base64.b64encode(powershell_form[0].encode("utf-16-le")).decode("ascii")
            result = subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-EncodedCommand", encoded], cwd=folder,
                                    stdin=subprocess.DEVNULL, capture_output=True)
            return result.returncode, (result.stdout + result.stderr).decode("utf-8", "replace")

        code, text = in_powershell()
        assert good_signature(code, text), "the notes' PowerShell command on the signed file (%d): %s" % (code, text)
        # One changed byte: no pass, in either form.
        with open(archive, "r+b") as out:
            out.seek(1000)
            out.write(b"\xff")
        code, text = verify_file(command, archive)
        assert code != 0 and not good_signature(code, text), "the changed archive passed: %s" % text
        code, text = in_powershell()
        assert code != 0 and not good_signature(code, text), "the changed archive passed in PowerShell: %s" % text
    finally:
        for name in os.listdir(folder):
            os.remove(os.path.join(folder, name))
        os.rmdir(folder)


def self_test():
    """The tests of the pure parts: no git, no cargo, no GitHub. The signature's live test runs Windows' ssh-keygen
    on a throwaway key in a temp folder."""
    test_signature_rules()
    test_signature_live()
    # W545: the gate names the unpacked archive with --archive, never with --mge-build, which replay.py refuses for a
    # folder with no manifest.
    command = gate_command("C:/tmp/unpacked")
    assert command[-3:] == ["baseline", "--archive", "C:/tmp/unpacked"] and "--mge-build" not in command, command
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
    nvidia = "NVIDIA GeForce RTX 5070 (Vulkan, driver NVIDIA 617.14)"
    assert gate_text(lines, [nvidia]) == "the capture gate on %s: 2 of 2 pictures equal to the accepted ones; 206 of 206 pictures equal to the accepted ones" % nvidia
    assert gate_text(lines[:1], ["a", "b"]).startswith("the capture gate on a and b: 2 of 2")
    assert gate_text(lines[:1], []).startswith("the capture gate on an adapter it did not name: ")
    assert fill_tokens("- {gate}\n- SHA-256 `{sha256}`\n", {GATE_TOKEN: "g", SHA_TOKEN: "abc"}) == "- g\n- SHA-256 `abc`\n"
    assert fill_tokens("- the gate by hand\n", {GATE_TOKEN: "g"}) == "- the gate by hand\n"
    assert checksum_line("ab" * 32, "x.7z") == "ab" * 32 + "  x.7z\n"
    manifest = manifest_text("x.7z", "MGE XE G7 at 1; nirender at 2.", [("Data Files/a.dds", 12, "cd" * 32), ("d3d8.dll", 3, "ef" * 32)])
    assert manifest.startswith("THE FILES OF x.7z\n\nMGE XE G7 at 1; nirender at 2.\n\n")
    assert manifest.endswith("\n%s  12  Data Files/a.dds\n%s  3  d3d8.dll\n" % ("cd" * 32, "ef" * 32)), manifest
    assert notes_problems("{signature}\n## Tested\n\n- {gate}\n- {checks}\n- run 20261027-120302-x\n", gated=True) == ["no {sha256} token for the archive's SHA-256"]
    assert notes_problems("SHA-256 {sha256} {signature}\n\n## Tested\n\n- {gate}\n- {checks}\n- run 20261027-120302-x\n", gated=True) == []
    assert notes_problems("SHA-256 {sha256} {signature}\n\n## Tested\n\n- {gate}\n- run 20261027-120302-x\n", gated=True) == ["no {checks} token for the line of the release checks"]
    # W604 part D: the notes carry the signature's line.
    assert notes_problems("SHA-256 {sha256}\n\n## Tested\n\n- {gate}\n- {checks}\n- run 20261027-120302-x\n", gated=True) == ["no {signature} token for the line of the signature"]
    # With --no-gate a leftover token is found at the start, not after the build.
    leftover = notes_problems("SHA-256 {sha256} {signature}\n\n## Tested\n\n- {gate}\n- {checks}\n- run 20261027-120302-x\n", gated=False)
    assert leftover[-1] == "the token {gate} with --no-gate: write the gate's result by hand", leftover
    assert GATE_RESULT.findall("  ok   " + lines[0] + "\n  FAIL  1 of 2 pictures equal to the accepted ones in c\n") == [lines[0], "1 of 2 pictures equal to the accepted ones in c"]
    assert render_link_tag("v0.3.3") == "nirender-v0.3.3"
    # The adapter word, from the lines the gate prints (validate_retained.adapter_note): only
    # SAME passes. The plant: the helper printed no adapter (UNKNOWN). The null: SAME.
    amd = "AMD Radeon RX 7800 XT (Vulkan, driver 24.9.1)"
    def gate_output(recorded, now):
        return "       %s\n  ok   2 of 2 pictures equal to the accepted ones in a\n" % adapter_note(recorded, now)
    assert adapter_problem(gate_output(nvidia, nvidia)) is None
    assert adapter_problem(gate_output(nvidia, amd)).startswith("the gate's adapter is OTHER (%s; the accepted pictures were made on %s" % (amd, nvidia))
    assert adapter_problem(gate_output(None, nvidia)).startswith("the gate's adapter is UNKNOWN (%s; the accepted pictures name none" % nvidia)
    assert adapter_problem(gate_output(nvidia, None)).startswith("the gate's adapter is UNKNOWN (not printed by the helper)")
    # A gate without the word (today's sentence alone, or no adapter line) is a problem too.
    assert adapter_problem("       adapter: %s, as the accepted pictures\n" % nvidia).startswith("the gate printed no adapter word")
    assert adapter_problem("  ok   2 of 2 pictures equal to the accepted ones in a\n").startswith("the gate printed no adapter word")
    assert adapter_problem(gate_output(nvidia, nvidia) + gate_output(nvidia, amd)).startswith("the gate's adapter is OTHER")
    assert adapters_of(gate_output(nvidia, nvidia) + gate_output(nvidia, nvidia)) == [nvidia]
    assert adapters_of(gate_output(nvidia, amd)) == []
    release_checks_self_test(gate_output, nvidia, amd)
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
    ap.add_argument("--sign-key", metavar="PUB", help="the PUBLIC file (.pub) of the release key: the Windows ssh-agent, which holds the key, "
                    "signs the archive and MANIFEST.txt; tools/release-signers must name the key")
    ap.add_argument("--no-sign", action="store_true", help="a test build with no signature: the notes say that it is not signed")
    ap.add_argument("--ssh-keygen", default=SSH_KEYGEN, help="the ssh-keygen.exe of Windows' OpenSSH (its ssh-add.exe beside it)")
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
    # The signature, before the build: the options, the key against release-signers, and (not in a dry run, which
    # signs nothing and asks no agent) the agent's keys. No private key is read here.
    problem = sign_options_problem(args.sign_key, args.no_sign)
    if problem:
        stop(problem, args.dry_run)
    sign_key = None
    if args.sign_key:
        if not os.path.isfile(args.sign_key):
            # The path stays out of the line (D316).
            raise SystemExit("--sign-key names no file")
        with open(args.sign_key, encoding="utf-8") as pub, open(SIGNERS, encoding="utf-8") as signers:
            sign_key, problems = sign_key_problems(pub.read(), signers.read())
        for problem in problems:
            stop(problem, args.dry_run)
        if sign_key and not args.dry_run:
            agent_check(args.ssh_keygen, sign_key, args.dry_run)
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
    # Each release check keeps its numbers and its pass rule in dist/checks-<tag>.txt, an asset
    # of the release beside the archive, not in it: the archive's SHA-256 must not depend on it.
    record = CheckRecord(os.path.join(args.mge, "dist", "checks-%s.txt" % tag), tag, args.dry_run,
                         {"nirender-plugin": PLUGIN, "render-link": args.mge})
    # The content checks of both repositories, then the doc comments of the three crates (no
    # broken link), then no Clippy lint above its baseline, then the unit tests. They run with
    # --no-build too: the build ids below prove the archive a build of the HEADs, so the checks
    # of the HEADs are the archive's.
    import checks
    record.run("checks.py all --strict", [sys.executable, "-I", os.path.join(HERE, "checks.py"), "all", "--strict"], None,
               lambda output, code: checks_numbers(output, code, checks.LISTS["all"]))
    record.run("docs.py", [sys.executable, "-I", os.path.join(HERE, "docs.py"), "--mge", args.mge], None, docs_numbers)
    baseline_commit = run(["git", "-C", PLUGIN, "log", "-1", "--format=%h %cs", "--", "tools/clippy.baseline"], capture=True).strip()
    record.run("clippy_baseline.py --check", [sys.executable, "-I", os.path.join(HERE, "clippy_baseline.py"), "--check", "--mge", args.mge], None,
               lambda output, code: clippy_numbers(output, code, baseline_commit or "no commit"))
    unit_tests(args.mge, record)
    dist_command = [cargo, "xtask", "dist"] + (["--max-perf"] if args.max_perf else [])
    if args.no_build:
        record.add("cargo xtask dist", command_text(dist_command, record.roots), "--no-build: the archive in dist", "the build", NOT_RUN)
    else:
        in_repo = len([name for name in os.listdir(os.path.join(args.mge, "assets", "Data Files", "shaders", "core")) if name.endswith(".wgsl")])
        code, output = run_teed(dist_command, args.mge)
        numbers, rule, passed = core_files_numbers(output, code, in_repo)
        record.add("cargo xtask dist", command_text(dist_command, record.roots), numbers, rule, PASS if passed else FAIL)
        if code != 0:
            # No archive: nothing after this step can run, a dry run neither.
            raise SystemExit("failed (%d): %s" % (code, " ".join(dist_command)))
    # The version of MGE XE is in the name of the archive that xtask packs.
    kind = "" if args.max_perf else "-test"
    packed = [name for name in os.listdir(os.path.join(args.mge, "dist")) if re.fullmatch(r"MGE-XE-G7-v[0-9.]+%s\.7z" % re.escape(kind), name)]
    assert len(packed) == 1, "one archive of this kind in dist, found: %s" % packed
    mge_version = re.match(r"MGE-XE-G7-v([0-9.]+)", packed[0]).group(1)
    asset = os.path.join(args.mge, "dist", "MGE-XE-G7-v%s-nirender-%s.7z" % (mge_version, tag))
    shutil.copy2(os.path.join(args.mge, "dist", packed[0]), asset)
    if additions:
        add_to_archive(asset, additions, os.path.join(args.mge, "target", "release-additions"))

    # The commits of the archive are those its binaries name: they must be the HEADs, which the
    # tag, MANIFEST.txt and the notes name (with --no-build the archive in dist can be older).
    unpacked = unpack(args.mge, asset)
    plugin_head, mge_head = head_of(PLUGIN), head_of(args.mge)
    numbers, rule, passed = build_id_numbers(unpacked, plugin_head, mge_head)
    record.add("the build ids of the archive", "the build ids of akulakhan.dll, numidium.exe and d3d8.dll", numbers, rule, PASS if passed else FAIL)

    # The manifest of the archive's files goes into it; the archive is unpacked again and must
    # hold the files of the manifest and no other. Then the archive's own SHA-256 goes beside it
    # and into the notes.
    manifest = os.path.join(args.mge, "target", "MANIFEST.txt")
    write_text(manifest, manifest_text(os.path.basename(asset), provenance(unpacked, args.mge, tag), file_digests(unpacked)))
    add_to_archive(asset, [(manifest, "MANIFEST.txt")], os.path.join(args.mge, "target", "release-manifest"))
    unpacked = unpack(args.mge, asset)
    shipped = file_digests(unpacked)
    shipped_manifest = os.path.join(unpacked, "MANIFEST.txt")
    listed = open(shipped_manifest, encoding="utf-8").read() if os.path.isfile(shipped_manifest) else ""
    differences = manifest_differences(listed, shipped)
    record.add("MANIFEST.txt against the archive", "the files of the archive unpacked again, against its MANIFEST.txt",
               "%d files in MANIFEST.txt, %d files unpacked besides it, %d differences%s" % (
                   len(MANIFEST_LINE.findall(listed)), len([entry for entry in shipped if entry[0] != "MANIFEST.txt"]), len(differences),
                   "".join("; %s: %s" % difference for difference in differences[:20])),
               "0 differences, and MANIFEST.txt lists > 0 files", PASS if listed and not differences and MANIFEST_LINE.search(listed) else FAIL)
    digest = sha256_of(asset)
    checksum = asset + ".sha256"
    write_text(checksum, checksum_line(digest, os.path.basename(asset)))
    print("SHA-256 of %s: %s (in %s)" % (os.path.basename(asset), digest, checksum))
    record.add("the archive", os.path.basename(asset), "SHA-256 %s, %d bytes, %d files" % (digest, os.path.getsize(asset), len(shipped)),
               "a record of what goes up", RECORD)

    # The signatures of the archive and of its MANIFEST.txt, by the agent, each checked against the repository's
    # release-signers; the checks file names the key by its fingerprint (D316).
    signed = []
    signed_files = [asset, manifest]
    verify_text = " and ".join(command_text(verify_command(args.ssh_keygen, SIGNERS, path), record.roots) + " < " + os.path.basename(path)
                               for path in signed_files)
    if args.no_sign:
        record.add("the signatures", verify_text, "--no-sign: a test build", "the signature", NOT_RUN)
    elif args.dry_run:
        record.add("the signatures", verify_text, "a dry run signs nothing", "the signature", NOT_RUN)
    elif sign_key:
        results = []
        agent_check(args.ssh_keygen, sign_key, args.dry_run)
        # The archive itself, and the MANIFEST.txt that went into it; the verify of the manifest reads the copy
        # unpacked from the archive (verdict 757 4.3).
        for path, shipped_copy in ((asset, asset), (manifest, shipped_manifest)):
            code, text = sign_file(args.ssh_keygen, args.sign_key, path, agent_environment(ENVIRONMENT))
            if code != 0:
                stop("ssh-keygen -Y sign of %s failed (%d): %s" % (os.path.basename(path), code, without_path(text, args.sign_key)), args.dry_run)
            code, text = verify_file(verify_command(args.ssh_keygen, SIGNERS, path), shipped_copy)
            results.append((os.path.basename(path), code, text))
            signed.append(path + ".sig")
        numbers, rule, passed = signature_numbers(sign_key, results)
        record.add("the signatures", verify_text, numbers, rule, PASS if passed else FAIL)

    # The gate on the helper that ships, and its result into the notes.
    values = {SHA_TOKEN: digest}
    values[SIGNATURE_TOKEN] = signature_text(sign_key if signed else None, args.repo, tag, os.path.basename(asset),
                                             None if args.no_sign else "a dry run")
    if args.no_gate:
        record.add("the capture gate", command_text(gate_command(unpacked), record.roots), "--no-gate: the notes hold its line by hand", "the gate", NOT_RUN)
    else:
        values[GATE_TOKEN] = gate_text(*gate(unpacked, record))
    values[CHECKS_TOKEN] = record.summary()
    notes_path = os.path.join(args.mge, "dist", "notes-%s.md" % tag)
    write_text(notes_path, fill_tokens(notes, values))
    print("the notes with the checksum, the gate's result and the release checks: %s" % notes_path)
    print("the release checks: %s" % record.path)

    # The commit of each repository as its binaries log it, for the "Builds" block of the notes.
    ids, _ = part_ids(unpacked)
    for program in ("nirender", "nirender-host", "numidium", "MGE-XE G7 Fork"):
        print("build id: %s %s" % (program, ids.get(program, "none")))

    title = "Test build: MGE XE G7 %s with nirender %s" % (mge_version, plugin_version)
    if args.dry_run:
        print("dry run: the archive %s; not published (%s); no release deleted; the MGE XE tag %s not made" % (asset, title, mge_tag))
        return 0
    print(run(["gh", "release", "create", tag, "--repo", args.repo, "--prerelease", "--title", title, "--notes-file", notes_path, asset, checksum, record.path] + signed, capture=True).strip())
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
