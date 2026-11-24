"""The version handshake of the four parts: each number has one home, and the copies agree.

    python -I tools/check_handshake.py            # this repository: the stream format
    python -I tools/check_handshake.py --mge DIR  # MGE XE: the extension format

This repository: `host/src/lib.rs` reads `src/stream_format.rs` as a `#[path]` module, so the
plugin and the host crate compare one STREAM_FORMAT. The file is also the one home of the
layout of the HEADER record and of the shared memories (the ring head words, the reply, tile
and message memories), of the suffixes of the names of a stream's events and memories, and of
the exit codes: no constant of it may have a second definition in the two crates, or with
--mge in mgeHost64, and no name suffix may come back as a literal. MGE XE:
MGE_EXTENSION_FORMAT in
`d3d8/cpp/mge/mgeversion.h` equals EXTENSION_FORMAT in `mgeHost64/src/renderer.rs` (a unit
test of mgeHost64 compares them too; this check needs no compile). And a record's version in
renderer.rs (`const VERSION` of an `impl Name` block) that differs from the one at HEAD of the
tree needs another EXTENSION_FORMAT than at HEAD: W68 changed the environment's version and
layout and kept format 1 (2026-11-09). Limits: the check sees a change only in the commit that
makes it (the tree against its HEAD), and it does not see a layout change with no change of a
VERSION. The ids with no `impl` block of their own have their versions in the table
`RECORD_VERSIONS` of renderer.rs, which the check reads too (W241, W242); before the table they
had the literal `word(0) != 1` of `version_1_only`, read as version 1.
`checks.py` runs both.
The design: handshake-design.md of the Numidium notes (2026-10-27).
"""
import argparse
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
MGE = os.path.join(os.path.dirname(REPO), "MGE-XE-G7-render-link")
sys.path.insert(0, HERE)
import common  # noqa: E402


def read(path):
    with open(path, encoding="utf-8") as file:
        return file.read()


def number(text, pattern, what):
    found = re.search(pattern, text, re.MULTILINE)
    if not found:
        raise SystemExit("%s not found" % what)
    return int(found.group(1))


HOME = os.path.join(REPO, "src", "stream_format.rs")


def home_names():
    """The constants of src/stream_format.rs: each has its one home there."""
    return set(re.findall(r"^pub const ([A-Z][A-Z0-9_]*)\s*:", read(HOME), re.MULTILINE))


def home_strings():
    """The string constants of src/stream_format.rs (the suffixes of the names of a stream's
    events and memories), as {value: name}."""
    return {value: name for name, value in re.findall(r'^pub const ([A-Z][A-Z0-9_]*)\s*:\s*&str\s*=\s*"([^"]+)"', read(HOME), re.MULTILINE)}


def literal_copies(text, strings):
    """The names of `strings` whose value `text` writes as a literal right after a `"` or
    after a `}` of a format string (`"{name}-read"`), and ends with a `"` or a `{`: a copy of
    the name that the home constant should give."""
    found = set()
    for value, name in strings.items():
        if re.search(r'(?<=["}])%s(?=["{])' % re.escape(value), text):
            found.add(name)
    return sorted(found)


def second_definitions(text, names):
    """The names of `names` that `text` defines again, as a const or a static."""
    if not names:
        return []
    pattern = re.compile(r"\b(?:const|static)\s+(%s)\s*:" % "|".join(sorted(names)))
    return sorted(set(pattern.findall(text)))


def check_folders(base, folders, names):
    """A problem for each second definition of a home constant in the .rs files of `folders`,
    and for each literal copy of a home string."""
    problems = []
    strings = home_strings()
    for folder in folders:
        for root, _, files in os.walk(os.path.join(base, folder)):
            if os.sep + "target" in root:
                continue
            for name in files:
                path = os.path.join(root, name)
                if not name.endswith(".rs") or os.path.samefile(path, HOME):
                    continue
                text = read(path)
                where = os.path.relpath(path, base).replace("\\", "/")
                for constant in second_definitions(text, names):
                    problems.append("%s defines %s of its own: its one home is src/stream_format.rs" % (where, constant))
                for constant in literal_copies(text, strings):
                    problems.append("%s writes the value of %s as a literal: use the constant of src/stream_format.rs" % (where, constant))
    return problems


def check_plugin():
    problems = []
    if not os.path.isfile(HOME):
        return ["src/stream_format.rs, the home of STREAM_FORMAT, is missing"]
    host = read(os.path.join(REPO, "host", "src", "lib.rs"))
    if '#[path = "../../src/stream_format.rs"]' not in host:
        problems.append("host/src/lib.rs does not read src/stream_format.rs as a #[path] module")
    names = home_names()
    if "STREAM_FORMAT" not in names:
        problems.append("src/stream_format.rs has no STREAM_FORMAT")
    # One definition only: a second definition anywhere in the two crates would be a second
    # home that can drift.
    problems += check_folders(REPO, ("src", os.path.join("host", "src")), names)
    return problems


def self_test():
    """The tests of the search: no files read but the home."""
    names = {"RING_READ", "STREAM_FORMAT", "TILE_HEAD"}
    assert second_definitions("const RING_READ: usize = 16;", names) == ["RING_READ"]
    assert second_definitions("pub(crate) const TILE_HEAD: usize = 16;\npub const STREAM_FORMAT: u32 = 1;", names) == ["STREAM_FORMAT", "TILE_HEAD"]
    assert second_definitions("static RING_READ: AtomicU64 = AtomicU64::new(0);", names) == ["RING_READ"]
    # A use is no definition, nor a longer name.
    assert second_definitions("let at = RING_READ; const RING_READ_BEFORE: usize = 0;", names) == []
    assert {"RING_READ", "RING_ENDING", "HEADER_ABILITIES_AT", "MESSAGES_MAGIC", "STREAM_FORMAT"} <= home_names()
    strings = {"-read": "READ_EVENT", ".reply": "REPLY_MEMORY"}
    assert literal_copies('format!("{name}-read")', strings) == ["READ_EVENT"]
    assert literal_copies('format!("{ring}.reply{generation}")', strings) == ["REPLY_MEMORY"]
    assert literal_copies('let suffix = ".reply";', strings) == ["REPLY_MEMORY"]
    # A longer word or a use of the constant is no copy.
    assert literal_copies('println!("{name}-ready"); format!("{name}{READ_EVENT}")', strings) == []
    assert literal_copies('"the pipeline-read"', strings) == []
    assert home_strings()["-records"] == "RECORDS_EVENT" and len(home_strings()) == 6
    # A record's version against the extension format: W68 changed the environment's version and kept format 1.
    before = "const EXTENSION_FORMAT: u32 = 1;\nimpl Settings {\n    const VERSION: u32 = 2;\n}\nimpl Environment {\n    pub const VERSION: u32 = 8;\n}\n"
    assert record_versions(before) == {"Settings": 2, "Environment": 8}
    assert version_problems(before, before) == []
    w68 = before.replace("VERSION: u32 = 8", "VERSION: u32 = 9")
    assert "Environment 8 -> 9" in version_problems(before, w68)[0]
    assert version_problems(before, w68.replace("EXTENSION_FORMAT: u32 = 1", "EXTENSION_FORMAT: u32 = 2")) == []
    # A new record with a version, and a record that loses its version, change the format too.
    assert "Lights none -> 1" in version_problems(before, before + "impl Lights {\n    const VERSION: u32 = 1;\n}\n")[0]
    assert "Settings 2 -> none" in version_problems(before, before.replace("impl Settings {\n    const VERSION: u32 = 2;\n}\n", ""))[0]
    # A VERSION outside an impl, or of a trait impl's block, belongs to no record of this file.
    assert record_versions("const VERSION: u32 = 3;\nimpl Extension for DistantLand {\n    const VERSION: u32 = 4;\n}\n") == {}
    # W241, W242: the table RECORD_VERSIONS. A number changed, an id added and an id removed, with the same format, are
    # problems; with a new format they pass. A line of another form stops the check (no entry is skipped).
    table = before + "const RECORD_VERSIONS: &[(u32, u32)] = &[\n    (SHADER_STATE_ID, 1),\n    (WEATHER_ID, 1),\n];\n"
    assert record_versions(table) == {"Settings": 2, "Environment": 8, "SHADER_STATE_ID": 1, "WEATHER_ID": 1}
    assert version_problems(table, table) == []
    for planted, change in ((table.replace("(WEATHER_ID, 1)", "(WEATHER_ID, 2)"), "WEATHER_ID 1 -> 2"),
                            (table.replace("];\n", "    (RELOAD_ID, 1),\n];\n"), "RELOAD_ID none -> 1"),
                            (table.replace("    (SHADER_STATE_ID, 1),\n", ""), "SHADER_STATE_ID 1 -> none")):
        problems = version_problems(table, planted)
        assert len(problems) == 1 and change in problems[0], (change, problems)
        assert version_problems(table, planted.replace("EXTENSION_FORMAT: u32 = 1", "EXTENSION_FORMAT: u32 = 2")) == []
    # The move from the literal of `version_1_only` into the table is no change; the null: a literal of another form
    # (not `word(0) != 1`) gives no versions, so the same move is a change.
    old = before + ("fn extension_needed(id: u32, bytes: &[u8]) -> Option<usize> {\n    let version_1_only = matches!(\n"
                    "        id,\n        SHADER_STATE_ID\n            | WEATHER_ID\n    );\n    if version_1_only && word(0) != 1 {\n")
    assert version_problems(old, table) == []
    assert "WEATHER_ID none -> 1" in version_problems(old.replace("word(0) != 1", "word(0) != 2"), table)[0]
    try:
        record_versions(table.replace("(WEATHER_ID, 1),", "(WEATHER_ID, ONE),"))
        raise AssertionError("a line of another form passed")
    except SystemExit as stop:
        assert "is not (NAME_ID, N)" in str(stop), stop
    self_test_base(before, w68)
    print("check_handshake self-test: ok")


def self_test_base(before, after):
    """W527 (W524 F-G-6): a merge round's tree is at the merge, so HEAD gives the merge's own renderer.rs. The plant: a
    scratch MGE XE tree with a commit that changes a record's version and keeps the format. With --base (the commit
    before) the check finds it. The null: against HEAD, as a round ran it before W527, the check finds nothing."""
    import tempfile

    def git(tree, *args):
        subprocess.run(["git", "-C", tree, "-c", "user.name=t", "-c", "user.email=t@t", *args], check=True,
                       capture_output=True, env=common.git_env())

    def write(tree, path, text):
        os.makedirs(os.path.dirname(os.path.join(tree, path)), exist_ok=True)
        with open(os.path.join(tree, path), "w", encoding="ascii", newline="\n") as file:
            file.write(text)

    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tree:
        git(tree, "init", "-q")
        write(tree, "d3d8/cpp/mge/mgeversion.h", "#define MGE_EXTENSION_FORMAT 1\n")
        write(tree, RENDERER, before)
        git(tree, "add", "-A")
        git(tree, "commit", "-q", "-m", "base")
        base = subprocess.run(["git", "-C", tree, "rev-parse", "HEAD"], capture_output=True, text=True, env=common.git_env()).stdout.strip()
        write(tree, RENDERER, after)
        git(tree, "commit", "-q", "-am", "merge")
        assert check_mge(tree) == [], check_mge(tree)
        problems = check_mge(tree, base)
        assert len(problems) == 1 and "Environment 8 -> 9" in problems[0], problems
        problems = check_mge(tree, "no-such-commit")
        assert len(problems) == 1 and "was not read" in problems[0], problems


RENDERER = "mgeHost64/src/renderer.rs"
FORMAT_PATTERN = r"^const EXTENSION_FORMAT: u32 = (\d+);"


def record_versions(text):
    """The version of each extension record of renderer.rs: `const VERSION: u32 = N;` under the last `impl Name {`, and
    each line `(NAME_ID, N),` of the table `RECORD_VERSIONS` (W241, W242: the ids with no `impl` block of their own),
    by the id's name."""
    versions = {}
    owner = None
    in_table = False
    # Before the table, these ids had the literal version 1 of `version_1_only` in extension_needed (`word(0) != 1`):
    # read as version 1, so that the move into the table is no change.
    literal = re.search(r"let version_1_only = matches!\(\s*id,([^)]*)\);\s*if version_1_only && word\(0\) != 1 \{", text)
    if literal:
        for name in re.findall(r"\b([A-Z][A-Z0-9_]*_ID)\b", literal.group(1)):
            versions[name] = 1
    for line in text.splitlines():
        found = re.match(r"^impl ([A-Za-z_][A-Za-z0-9_]*) \{", line)
        if found:
            owner = found.group(1)
        found = re.match(r"^\s+(?:pub )?const VERSION: u32 = (\d+);", line)
        if found and owner:
            versions[owner] = int(found.group(1))
        if re.match(r"^const RECORD_VERSIONS: &\[\(u32, u32\)\] = &\[", line):
            in_table = True
            continue
        if in_table:
            if line.startswith("];"):
                in_table = False
                continue
            found = re.match(r"^\s+\(([A-Z][A-Z0-9_]*), (\d+)\),$", line)
            if not found:
                raise SystemExit("a line of RECORD_VERSIONS in renderer.rs is not (NAME_ID, N),: %r" % line)
            versions[found.group(1)] = int(found.group(2))
    return versions


def version_problems(before, now):
    """A record whose version changed since `before` (renderer.rs of HEAD) while EXTENSION_FORMAT stayed: W68 changed
    the environment's version and layout and kept format 1, and an old capture replayed with no environment."""
    format_before = number(before, FORMAT_PATTERN, "EXTENSION_FORMAT in renderer.rs of HEAD")
    format_now = number(now, FORMAT_PATTERN, "EXTENSION_FORMAT in renderer.rs")
    if format_before != format_now:
        return []
    old, new = record_versions(before), record_versions(now)
    changed = ["%s %s -> %s" % (name, old.get(name, "none"), new.get(name, "none")) for name in sorted(set(old) | set(new)) if old.get(name) != new.get(name)]
    if not changed:
        return []
    return ["the version of a record changed (%s) and EXTENSION_FORMAT stayed %d: bump EXTENSION_FORMAT in renderer.rs and "
            "MGE_EXTENSION_FORMAT in mgeversion.h" % (", ".join(changed), format_now)]


def head_text(mge, path, rev="HEAD"):
    """The file at `rev` (HEAD by default) of the tree `mge`, or None with no git or no such file (a tree that is not a
    checkout)."""
    done = subprocess.run(["git", "-C", mge, "show", "%s:%s" % (rev, path)], capture_output=True, env=common.git_env())
    return done.stdout.decode("utf-8") if done.returncode == 0 else None


def check_mge(mge, base=None):
    """The problems of the MGE XE tree `mge`. The record versions are compared with renderer.rs at `base`, else at HEAD:
    a merge round gives the base of the merge, as its HEAD is the merge itself (W527, W524 F-G-6)."""
    header = read(os.path.join(mge, "d3d8", "cpp", "mge", "mgeversion.h"))
    renderer = read(os.path.join(mge, *RENDERER.split("/")))
    cpp = number(header, r"^#define MGE_EXTENSION_FORMAT (\d+)", "MGE_EXTENSION_FORMAT in mgeversion.h")
    rust = number(renderer, FORMAT_PATTERN, "EXTENSION_FORMAT in renderer.rs")
    problems = []
    if cpp != rust:
        problems.append("MGE_EXTENSION_FORMAT is %d in mgeversion.h and EXTENSION_FORMAT %d in renderer.rs" % (cpp, rust))
    before = head_text(mge, RENDERER, base or "HEAD")
    if before is not None:
        problems += version_problems(before.replace("\r\n", "\n"), renderer)
    elif base:
        problems.append("%s at the base %s was not read: the record versions were not compared" % (RENDERER, base))
    # mgeHost64 reads the stream's constants through the host crate, never as a copy.
    problems += check_folders(mge, (os.path.join("mgeHost64", "src"),), home_names())
    return problems


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mge", nargs="?", const=MGE, help="check MGE XE's extension format, and that mgeHost64 has no copy of a constant "
                    "of src/stream_format.rs, in this folder (default: the render-link worktree)")
    ap.add_argument("--base", help="with --mge: compare the record versions with renderer.rs at this commit, not at HEAD "
                    "(a merge round: the base of the merge)")
    ap.add_argument("--self-test", action="store_true", help="the tests of the search for a second definition")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    problems = check_mge(args.mge, args.base) if args.mge else check_plugin()
    for problem in problems:
        print(problem)
    print("handshake (%s): %d problems" % ("mge" if args.mge else "plugin", len(problems)))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
