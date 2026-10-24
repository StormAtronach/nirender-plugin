"""Writes the pre-commit hooks that run the content checks (`tools/checks.py`).

    python -I tools/install_hooks.py

- This repository: `checks.py plugin` before each commit.
- The MGE XE repository: `checks.py mge`, only in the render-link worktree. The hooks folder of
  MGE-XE-G7 is shared by all its worktrees; in the others the hook does nothing.
- The docs repository: `check_state_pages.py --install-hook` writes its own hook.

A hook that is not ours is not replaced: the tool stops with its path. `git commit
--no-verify` skips a hook for one commit. The hooks need no compile.
"""
import subprocess
import sys
from pathlib import Path

PLUGIN = Path(__file__).resolve().parents[1]
REPOS = PLUGIN.parent
MGE = REPOS / "MGE-XE-G7-render-link"
STATE_PAGES = REPOS / "moreFPS" / "docs" / "Numidium Project" / "scripts" / "state-page" / "check_state_pages.py"
MARK = "# Written by nirender-plugin/tools/install_hooks.py."


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


def main():
    python = sys.executable.replace("\\", "/")
    checks = (PLUGIN / "tools" / "checks.py").as_posix()
    write(PLUGIN, "# The content checks of the plugin (tools/checks.py plugin).\n"
                  "\"%s\" -I \"%s\" plugin || exit 1\n" % (python, checks))
    if MGE.exists():
        write(MGE, "# The content checks of the renderer (nirender-plugin tools/checks.py mge), in the\n"
                   "# render-link worktree only.\n"
                   "case \"$(git rev-parse --show-toplevel)\" in\n"
                   "  */MGE-XE-G7-render-link) ;;\n"
                   "  *) exit 0 ;;\n"
                   "esac\n"
                   "\"%s\" -I \"%s\" mge || exit 1\n" % (python, checks))
    if STATE_PAGES.exists():
        subprocess.run([sys.executable, "-I", str(STATE_PAGES), "--install-hook"], check=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
