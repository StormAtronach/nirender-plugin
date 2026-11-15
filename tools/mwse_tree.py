"""The MWSE source that the checks of MWSE read (W497): the commit that the test installs run, else the sibling checkout.

    python -I tools/mwse_tree.py              # the MWSE source the checks would read now, and why
    python -I tools/mwse_tree.py --self-test  # scratch repository and scratch installs; nothing real is read

The checks check_mwse_native.py, check_mwse_vtable_fills.py and check_sharedse_layouts.py compare this tree with MWSE.
The MWSE that matters is the one under test, and that is the one the test installs run. deploy_rest.py --mwse-dll writes
its commit into each install (mwse-deployed.json, D442). So `resolve` takes the commit that the record of every install
names, when the record describes the install's MWSE.dll (by MD5) and the commit is in the MWSE repository, and reads the
files of that commit from git: no checkout is switched and no file is written. The owner's checkout may hold any branch.

Without one such commit (no record, records that differ, a DLL that its record does not describe, a commit that is not in
the repository), the checks read the sibling checkout as before. The check then prints one line that starts with
FALLBACK and names the check and the reason; checks.py and merge_step.py show those lines in their summaries, so that a
failure after a fallback does not look like a fault of the code.
"""
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import git_env, sibling  # noqa: E402

# The start of the line of a check that read the sibling checkout in place of the installs' commit.
FALLBACK = "MWSE FALLBACK: "
SIBLING = sibling("MWSE", "MWSE")


class Folder:
    """The files of an MWSE tree on disk."""

    def __init__(self, root, label=None):
        self.root = root
        self.label = label or "the folder %s" % root

    def names(self, folder):
        """The names of the files (not the folders) in `folder` of the tree, sorted; none when it does not exist."""
        place = os.path.join(self.root, folder)
        if not os.path.isdir(place):
            return []
        return sorted(name for name in os.listdir(place) if os.path.isfile(os.path.join(place, name)))

    def read(self, path):
        """The bytes of the file `path` (with "/") of the tree."""
        with open(os.path.join(self.root, *path.split("/")), "rb") as file:
            return file.read()


class Commit:
    """The files of an MWSE commit, read from the repository by git."""

    def __init__(self, repo, commit, label=None):
        self.repo = repo
        self.commit = commit
        self.label = label or "commit %s of %s" % (commit, repo)
        self._batch = None

    def _git(self, *args):
        return subprocess.run(["git", "-C", self.repo, *args], env=git_env(), capture_output=True, check=True).stdout

    def names(self, folder):
        try:
            listing = self._git("ls-tree", "%s:%s" % (self.commit, folder)).decode("utf-8", "replace")
        except subprocess.CalledProcessError:
            return []
        # "<mode> <type> <sha>\t<name>" for each entry: only the blobs are files.
        entries = (line.split("\t", 1) for line in listing.splitlines() if "\t" in line)
        return sorted(name for meta, name in entries if meta.split()[1] == "blob")

    def read(self, path):
        # One `git cat-file --batch` for all reads: the vtable check reads about a thousand files.
        if self._batch is None:
            self._batch = subprocess.Popen(["git", "-C", self.repo, "cat-file", "--batch"], env=git_env(),
                                           stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        self._batch.stdin.write(("%s:%s\n" % (self.commit, path)).encode("utf-8"))
        self._batch.stdin.flush()
        header = self._batch.stdout.readline().decode("utf-8", "replace").split()
        if len(header) != 3 or header[1] != "blob":
            raise FileNotFoundError("%s:%s is not a file in %s" % (self.commit, path, self.repo))
        data = self._batch.stdout.read(int(header[2]))
        self._batch.stdout.read(1)
        return data

    def close(self):
        if self._batch is not None:
            self._batch.stdin.close()
            self._batch.wait()
            self._batch = None


def has_commit(repo, commit):
    """Whether `commit` names a commit of the repository `repo`."""
    return subprocess.run(["git", "-C", repo, "cat-file", "-e", "%s^{commit}" % commit], env=git_env(),
                          capture_output=True).returncode == 0


def installed_commit(installs=None, deploys=None):
    """(the MWSE commit that every install's record names for its MWSE.dll, None) or (None, the reason there is none)."""
    import deploy_rest

    installs = deploy_rest.REST_INSTALLS if installs is None else installs
    deploys = deploy_rest.MWSE_DEPLOYS if deploys is None else deploys
    commits = {name: deploy_rest.noted_commit(install, deploys[name]) if name in deploys else None
               for name, install in sorted(installs.items())}
    found = set(commits.values())
    if len(found) == 1 and None not in found:
        return found.pop(), None
    return None, "no single MWSE record in the installs: %s" % ", ".join(
        "%s %s" % (name, commit or "none") for name, commit in commits.items())


def resolve(check, folder=None, commit=None, repo=None, installs=None, deploys=None, fallback=None):
    """The MWSE source of the check `check` and the FALLBACK line to print, or None.

    `folder` (a tree on disk) or `commit` (of `repo`) choose the source; without them it is the commit of the installs'
    records, else the sibling checkout `fallback` with a FALLBACK line."""
    if repo is None:
        import deploy_rest

        repo = deploy_rest.MWSE_REPO
    fallback = SIBLING if fallback is None else fallback
    if folder:
        return Folder(folder), None
    if commit:
        if not has_commit(repo, commit):
            raise SystemExit("%s: the commit %s is not in the MWSE repository %s" % (check, commit, repo))
        return Commit(repo, commit), None
    commit, reason = installed_commit(installs, deploys)
    if commit and not has_commit(repo, commit):
        commit, reason = None, "the commit %s of the installs' records is not in the MWSE repository %s" % (commit, repo)
    if commit:
        return Commit(repo, commit, "commit %s of the installs' records (%s)" % (commit, repo)), None
    tree = Folder(fallback, "the sibling checkout %s" % fallback)
    return tree, "%s%s: read %s: %s" % (FALLBACK, check, tree.label, reason)


def fallback_lines(output):
    """The FALLBACK lines of a check's output, in order."""
    return [line.strip() for line in output.splitlines() if line.strip().startswith(FALLBACK)]


def fallback_checks(output):
    """The names of the checks that fell back, from the FALLBACK lines of `output`, in order and once each."""
    names = []
    for line in fallback_lines(output):
        name = line[len(FALLBACK):].split(": ", 1)[0]
        if name not in names:
            names.append(name)
    return names


def self_test():
    import json
    import tempfile

    from deploy_rest import md5_of

    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp:
        repo = os.path.join(temp, "MWSE")
        os.makedirs(os.path.join(repo, "MWSE"))
        os.makedirs(os.path.join(repo, "SharedSE", "inner"))

        def git(*args):
            # core.autocrlf off: the blobs keep the bytes written, whatever this machine's git setting is.
            return subprocess.run(["git", "-C", repo, "-c", "user.name=t", "-c", "user.email=t@t", "-c", "core.autocrlf=false", *args], env=git_env(),
                                  check=True, capture_output=True, text=True).stdout.strip()

        def commit_with(header):
            with open(os.path.join(repo, "MWSE", "MWSENativeApi.h"), "w", newline="\r\n") as file:
                file.write(header)
            git("add", "-A")
            git("commit", "-q", "-m", header.split()[0])
            return git("rev-parse", "--short=10", "HEAD")

        git("init", "-q")
        with open(os.path.join(repo, "SharedSE", "NIOne.h"), "wb") as file:
            file.write(b"one\n")
        with open(os.path.join(repo, "SharedSE", "inner", "x.h"), "wb") as file:
            file.write(b"x\n")
        old = commit_with("old\n#define MWSE_NATIVE_A 1u\n")
        new = commit_with("new\n#define MWSE_NATIVE_A 2u\n")
        sibling_tree = os.path.join(temp, "sibling")
        os.makedirs(os.path.join(sibling_tree, "MWSE"))
        with open(os.path.join(sibling_tree, "MWSE", "MWSENativeApi.h"), "wb") as file:
            file.write(b"sibling\n")

        installs, deploys = {}, {}
        for name in ("A", "B"):
            root = os.path.join(temp, name)
            os.makedirs(root)
            installs[name] = {"root": root}
            deploys[name] = os.path.join(root, "MWSE.dll")
            with open(deploys[name], "wb") as file:
                file.write(b"dll")

        def record(name, commit, dll_md5=None):
            with open(os.path.join(installs[name]["root"], "mwse-deployed.json"), "w", encoding="ascii") as file:
                json.dump({"commit": commit, "md5": dll_md5 or md5_of(deploys[name])}, file)

        def pick(**options):
            return resolve("mwse native", repo=repo, installs=installs, deploys=deploys, fallback=sibling_tree, **options)

        # No record: the sibling, with a line that names the check and the reason.
        tree, line = pick()
        assert isinstance(tree, Folder) and tree.read("MWSE/MWSENativeApi.h") == b"sibling\n", "no record: the sibling"
        assert line == FALLBACK + "mwse native: read the sibling checkout %s: no single MWSE record in the installs: A none, B none" % sibling_tree, line

        # Both installs name the old commit: its header, from git, with the line ends of the commit.
        record("A", old)
        record("B", old)
        tree, line = pick()
        assert line is None and isinstance(tree, Commit) and tree.commit == old, (tree.label, line)
        assert tree.read("MWSE/MWSENativeApi.h") == b"old\r\n#define MWSE_NATIVE_A 1u\r\n", "the header of the recorded commit"
        assert tree.names("SharedSE") == ["NIOne.h"], "files only, no folder"
        assert tree.names("CSSE") == [], "a folder that the commit lacks has no files"
        assert tree.read("SharedSE/NIOne.h") == b"one\n", "a second read on the same batch"
        try:
            tree.read("MWSE/none.h")
            raise AssertionError("a missing file must raise")
        except FileNotFoundError:
            pass
        tree.close()

        # The records name the new commit: the new header.
        record("A", new)
        record("B", new)
        tree, line = pick()
        assert line is None and tree.read("MWSE/MWSENativeApi.h").startswith(b"new"), "the header of the new commit"
        tree.close()

        # Records that differ, a DLL that its record does not describe, a commit not in the repository: the sibling.
        record("B", old)
        tree, line = pick()
        assert isinstance(tree, Folder) and line.endswith("no single MWSE record in the installs: A %s, B %s" % (new, old)), line
        record("B", new, dll_md5="0" * 32)
        tree, line = pick()
        assert isinstance(tree, Folder) and line.endswith("A %s, B none" % new), line
        record("A", "1234567890")
        record("B", "1234567890")
        tree, line = pick()
        assert isinstance(tree, Folder) and "the commit 1234567890 of the installs' records is not in" in line, line

        # A folder or a commit given to the check wins over the records, with no line.
        tree, line = pick(folder=sibling_tree)
        assert isinstance(tree, Folder) and line is None
        tree, line = pick(commit=old)
        assert isinstance(tree, Commit) and tree.commit == old and line is None
        tree.close()

    output = "x\n%smwse native: read y: z\n  %smwse vtable fills: read y: z\n%smwse native: again\n" % (FALLBACK, FALLBACK, FALLBACK)
    assert fallback_checks(output) == ["mwse native", "mwse vtable fills"], fallback_checks(output)
    assert fallback_checks("mwse native ok\n") == []
    print("mwse tree self-test: ok")
    return 0


def main(argv):
    if argv[:1] == ["--self-test"]:
        return self_test()
    tree, line = resolve("mwse tree")
    print(line or "MWSE: %s" % tree.label)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
