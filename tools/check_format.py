"""Checks that the Rust code is formatted as `cargo fmt` writes it.

    python -I tools/check_format.py               # each Rust root of this repository
    python -I tools/check_format.py --mge DIR     # the workspace of the MGE XE tree DIR
    python -I tools/check_format.py --self-test   # a formatted and an unformatted crate

The roots of this repository are the folders of its tracked Cargo.toml files (the plugin, host,
d3d8, spikes/wgpu32): each is a crate or a workspace of its own. In MGE XE the workspace root is
the one root. The check runs `cargo fmt -- --check` in each root, without `--all`: with `--all`
the MGE XE workspace would also check its path dependency, the host crate of this repository.

The output of rustfmt goes to a buffer, never to a pipe that the reader closes early: rustfmt
panics on a closed pipe and leaves a rustc-ice file in the tree. The exit code is 1 when a root
is not formatted or rustfmt fails; the output names the files.
"""
import os
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import REPO  # noqa: E402


def roots(repo):
    """The folders of the tracked Cargo.toml files of `repo`."""
    listed = subprocess.run(["git", "ls-files", "*Cargo.toml"], cwd=repo, capture_output=True, text=True, check=True)
    return sorted({os.path.join(repo, os.path.dirname(path)) for path in listed.stdout.split()})


def unformatted(root):
    """The output of `cargo fmt -- --check -l` in `root`, empty when it is formatted."""
    run = subprocess.run(["cargo", "fmt", "--", "--check", "-l"], cwd=root, capture_output=True, text=True,
                         encoding="utf-8", errors="replace")
    if run.returncode == 0:
        return ""
    return (run.stdout + run.stderr).strip() or "cargo fmt exited with %d" % run.returncode


def check(folders):
    failed = 0
    for root in folders:
        problem = unformatted(root)
        if problem:
            failed += 1
            print("not formatted in %s (run cargo fmt there, without --all):\n%s" % (root, problem))
    return 1 if failed else 0


def self_test():
    """A crate whose file cargo fmt would change fails the check; the same crate after cargo fmt passes."""
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp:
        crate = os.path.join(temp, "crate")
        os.makedirs(os.path.join(crate, "src"))
        with open(os.path.join(crate, "Cargo.toml"), "w") as file:
            file.write('[package]\nname = "format_test"\nversion = "0.1.0"\nedition = "2021"\n')
        # The toolchain of this repository: this machine has no default toolchain.
        shutil.copy(os.path.join(REPO, "rust-toolchain.toml"), crate)
        with open(os.path.join(crate, "src", "main.rs"), "w") as file:
            file.write("fn main( ) { let x=1 ; println!(\"{}\",x) ; }\n")
        assert unformatted(crate), "an unformatted file must fail"
        subprocess.run(["cargo", "fmt"], cwd=crate, capture_output=True, check=True)
        assert unformatted(crate) == "", "the file after cargo fmt must pass"
    print("check_format self-test: ok")
    return 0


def main(argv):
    if argv[:1] == ["--self-test"]:
        return self_test()
    if "--mge" in argv:
        # The last --mge counts: checks.py adds the tree of a worktree's hook after its own.
        at = len(argv) - 1 - argv[::-1].index("--mge")
        if at + 1 >= len(argv):
            raise SystemExit("usage: check_format.py [--mge DIR | --self-test]")
        return check([argv[at + 1]])
    return check(roots(REPO))


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
