"""Writes the pre-commit hooks that run the content checks (`tools/checks.py`).

    python -I tools/install_hooks.py
    python -I tools/install_hooks.py --self-test    # the hook texts, no file written

- This repository: `checks.py plugin` before each commit, with the tools of the tree the
  commit is in (`git rev-parse --show-toplevel`): the hooks folder is shared by the worktrees
  of a repository, and a hook that named one checkout's tools checked that checkout, not the
  tree of the commit.
- The MGE XE repository: `checks.py mge --mge <the tree of the commit>`, in every worktree.
  The tools come from this repository (the sibling `nirender-plugin` of the tree, or the
  path written at install time when there is none).
- The docs repository: `check_state_pages.py --install-hook` writes its own hook.

A hook that is not ours is not replaced: the tool stops with its path. `git commit
--no-verify` skips a hook for one commit. The hooks need no compile; a check whose
repository is not on the machine is skipped, as `checks.py` does without `--strict`.
"""
import subprocess
import sys
from pathlib import Path

PLUGIN = Path(__file__).resolve().parents[1]
REPOS = PLUGIN.parent
MGE = REPOS / "MGE-XE-G7-render-link"
STATE_PAGES = REPOS / "moreFPS" / "docs" / "Numidium Project" / "scripts" / "state-page" / "check_state_pages.py"
MARK = "# Written by nirender-plugin/tools/install_hooks.py."


def plugin_hook(python):
    """The hook of this repository: the checks of the tree the commit is in, with that tree's
    own tools."""
    return ("# The content checks of the plugin (tools/checks.py plugin) of the tree the commit is in.\n"
            "tree=\"$(git rev-parse --show-toplevel)\"\n"
            "\"%s\" -I \"$tree/tools/checks.py\" plugin || exit 1\n" % python)


def mge_hook(python, plugin):
    """The hook of the MGE XE repository: the mge checks against the tree the commit is in,
    with the tools of the sibling nirender-plugin of that tree, else of `plugin`."""
    return ("# The content checks of the renderer (nirender-plugin tools/checks.py mge) against the\n"
            "# tree the commit is in, in every worktree.\n"
            "tree=\"$(git rev-parse --show-toplevel)\"\n"
            "plugin=\"$(dirname \"$tree\")/nirender-plugin\"\n"
            "[ -f \"$plugin/tools/checks.py\" ] || plugin=\"%s\"\n"
            "\"%s\" -I \"$plugin/tools/checks.py\" mge --mge \"$tree\" || exit 1\n" % (plugin, python))


def hooks_folder(repo):
    path = subprocess.run(["git", "rev-parse", "--git-path", "hooks"], cwd=repo, capture_output=True, text=True, check=True).stdout.strip()
    return (repo / path).resolve()


def write(repo, body):
    hook = hooks_folder(repo) / "pre-commit"
    if hook.exists() and MARK not in hook.read_text(encoding="utf-8", errors="replace"):
        raise SystemExit("%s exists and is not ours: merge it by hand" % hook)
    hook.parent.mkdir(exist_ok=True)
    hook.write_text("#!/bin/sh\n%s\n%s" % (MARK, body), encoding="utf-8", newline="\n")
    print("wrote %s" % hook)


def self_test():
    """The hook texts name no checkout's tools by an absolute path: the tree of the commit."""
    body = plugin_hook("C:/py/python.exe")
    assert "$(git rev-parse --show-toplevel)" in body and "\"$tree/tools/checks.py\" plugin" in body, body
    assert "C:/Users" not in body
    body = mge_hook("C:/py/python.exe", "C:/repos/nirender-plugin")
    assert "mge --mge \"$tree\"" in body and "$(dirname \"$tree\")/nirender-plugin" in body, body
    assert "plugin=\"C:/repos/nirender-plugin\"" in body
    print("install_hooks self-test: ok")


def main():
    if "--self-test" in sys.argv[1:]:
        self_test()
        return 0
    python = sys.executable.replace("\\", "/")
    write(PLUGIN, plugin_hook(python))
    if MGE.exists():
        write(MGE, mge_hook(python, PLUGIN.as_posix()))
    if STATE_PAGES.exists():
        subprocess.run([sys.executable, "-I", str(STATE_PAGES), "--install-hook"], check=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
