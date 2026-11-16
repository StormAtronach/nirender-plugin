"""Builds the rustdoc pages of the plugin, the host crate and the MGE XE helper, and checks
their doc comments.

    python -I tools/docs.py           # build, check, write target/docs.html
    python -I tools/docs.py --open    # the same, then open the page in the browser
    python -I tools/docs.py --mge DIR # the MGE XE repository, if not the sibling
    python -I tools/docs.py --self-test  # the tests of the choice of crates: no cargo

Each crate is documented with its private items (`cargo doc --no-deps
--document-private-items`). A link in a doc comment that rustdoc cannot resolve is an error
in the plugin and the host crate (`[lints.rustdoc]` in their Cargo.toml) and in the helper's
`renderer` module (`#![deny]` in renderer.rs). The rest of the helper is MGE XE G7's code:
its warnings are listed, not failed.

The tool fails (exit 1) when a doc build fails or when a doc comment of our code has a
warning. The pages stay in the target folder of each repository; target/docs.html links them.
The tool runs cargo: ask before it runs, as before any build.

Without --mge, a tree with no render-link tree beside it (a worktree under C:/tmp) skips the
helper with one line, "the helper: no render-link tree at <path>; not built", and the exit
code is that of the plugin and the host crate (W510). A --mge folder with no helper fails:
merge_step.py and release.py name it, and for them a missing repository is a failure.
"""
import argparse
import contextlib
import datetime
import html
import io
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

PLUGIN = Path(__file__).resolve().parents[1]
# The render-link tree beside this tree, when --mge does not name one.
SIBLING_MGE = PLUGIN.parent / "MGE-XE-G7-render-link"
HELPER = "the helper (numidium)"


def crates(mge, helper=True):
    """(name, cargo folder, cargo arguments, the doc page, whether a file is ours); the helper
    only with `helper`."""
    renderer = (mge / "mgeHost64" / "src" / "renderer").resolve()
    renderer_rs = renderer.with_suffix(".rs")
    found = [
        ("the plugin (akulakhan)", PLUGIN, [],
         PLUGIN / "target" / "i686-pc-windows-msvc" / "doc" / "akulakhan" / "index.html", lambda f: True),
        ("the host crate (nirender_wgpu)", PLUGIN / "host", ["--lib"],
         PLUGIN / "host" / "target" / "x86_64-pc-windows-msvc" / "doc" / "nirender_wgpu" / "index.html", lambda f: True),
    ]
    if helper:
        found.append((HELPER, mge, ["-p", "mgeHost64"],
                      mge / "target" / "doc" / "numidium" / "index.html",
                      lambda f: f == renderer_rs or renderer in f.parents))
    return found


def helper_plan(mge, named):
    """Whether the helper is built, and the line when it is not: (True, None) with a render-link
    tree at `mge`; else a skip line when `mge` is the default sibling (`named` False), and a
    failure line when --mge named it (`named` True). Returns (build, line, fails)."""
    if (mge / "mgeHost64").is_dir():
        return True, None, False
    if named:
        return False, "the helper: no render-link tree at %s (--mge); not built, and that fails" % mge, True
    return False, "the helper: no render-link tree at %s; not built" % mge, False


def sha(repo):
    run = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=repo, capture_output=True, text=True)
    dirty = subprocess.run(["git", "status", "--porcelain", "--untracked-files=no"], cwd=repo, capture_output=True, text=True)
    return run.stdout.strip() + (" + changes" if dirty.stdout.strip() else "")


def build(folder, args):
    """Runs the doc build; returns (exit code, [(file, line, level, text)])."""
    cmd = ["cargo", "doc", "--no-deps", "--document-private-items", "--message-format=json"] + args
    run = subprocess.run(cmd, cwd=folder, capture_output=True, text=True, encoding="utf-8", errors="replace")
    found = []
    for raw in run.stdout.splitlines():
        try:
            msg = json.loads(raw)
        except ValueError:
            continue
        if msg.get("reason") != "compiler-message":
            continue
        diag = msg["message"]
        if diag.get("level") not in ("warning", "error") or not diag.get("spans"):
            continue
        span = next((s for s in diag["spans"] if s.get("is_primary")), diag["spans"][0])
        found.append(((folder / span["file_name"]).resolve(), span["line_start"], diag["level"], diag["message"]))
    if run.returncode != 0 and not found:
        print(run.stderr[-3000:])
    return run.returncode, found


def page(rows):
    out = ["<!doctype html><meta charset=utf-8><title>Numidium code docs</title>",
           "<style>body{font:15px system-ui,sans-serif;margin:2em;max-width:50em;background:#fff;color:#111}"
           "td,th{padding:.3em .8em;text-align:left;border-bottom:1px solid #ddd}"
           "@media (prefers-color-scheme:dark){body{background:#181818;color:#ddd}a{color:#8cf}td,th{border-color:#333}}</style>",
           "<h1>Numidium code docs</h1><p>Written by <code>nirender-plugin/tools/docs.py</code> on %s.</p>"
           % html.escape(datetime.datetime.now().strftime("%Y-%m-%d %H:%M")),
           "<table><tr><th>Crate</th><th>Commit</th><th>Warnings in our code</th><th>Other warnings</th></tr>"]
    for name, index, commit, ours, others in rows:
        out.append("<tr><td><a href=\"%s\">%s</a></td><td>%s</td><td>%d</td><td>%d</td></tr>"
                   % (index.as_uri(), html.escape(name), html.escape(commit), ours, others))
    out.append("</table>")
    return "\n".join(out) + "\n"


def main(argv, out=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--mge", help="the render-link tree of the helper; by default the one beside this tree, skipped when absent")
    ap.add_argument("--open", action="store_true")
    ap.add_argument("--self-test", action="store_true", help="the tests of the choice of crates: no cargo")
    args = ap.parse_args(argv)
    if args.self_test:
        return self_test()
    mge = Path(args.mge or SIBLING_MGE).resolve()
    with_helper, helper_line, failed = helper_plan(mge, args.mge is not None)
    if helper_line:
        print(helper_line)
    rows = []
    for name, folder, cargo_args, index, is_ours in crates(mge, with_helper):
        code, found = build(folder, cargo_args)
        ours = [w for w in found if is_ours(w[0])]
        others = [w for w in found if not is_ours(w[0])]
        print("%s: exit %d, %d warnings in our code, %d other" % (name, code, len(ours), len(others)))
        for file, line, level, text in ours:
            print("  %s %s:%d %s" % (level, file, line, text))
        if code != 0 or ours:
            failed = True
        rows.append((name, index, sha(folder), len(ours), len(others)))
    out = out or PLUGIN / "target" / "docs.html"
    out.parent.mkdir(exist_ok=True)
    out.write_text(page(rows), encoding="utf-8", newline="\n")
    print("wrote %s" % out)
    if args.open:
        os.startfile(out)
    return 1 if failed else 0


def self_test():
    """The choice of crates with a stand-in for cargo: a tree with no render-link tree beside it
    skips the helper with its line and passes on clean crates; a named --mge with no helper fails;
    a render-link tree is built. The null: a warning in our code still fails with the helper
    skipped."""
    global build, sha, SIBLING_MGE
    real_build, real_sha, real_sibling = build, sha, SIBLING_MGE
    built = []

    def fake_build(folder, args, warnings=()):
        built.append(args)
        return 0, list(warnings)

    try:
        sha = lambda folder: "abc1234"  # noqa: E731
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            page_path = root / "target" / "docs.html"
            mge = (root / "no-render-link").resolve()
            assert helper_plan(mge, False) == (False, "the helper: no render-link tree at %s; not built" % mge, False)
            assert helper_plan(mge, True)[2], "a named --mge with no helper fails"
            assert [name for name, *_ in crates(mge, False)] == ["the plugin (akulakhan)", "the host crate (nirender_wgpu)"]

            def run(argv):
                built.clear()
                with contextlib.redirect_stdout(io.StringIO()) as said:
                    code = main(argv, page_path)
                return code, said.getvalue(), list(built)

            build = fake_build
            code, said, cargo = run(["--mge", str(root / "no-render-link")])
            assert code == 1 and "(--mge); not built, and that fails" in said and len(cargo) == 2, said
            # The default sibling of a scratch tree: a stand-in folder.
            SIBLING_MGE = mge
            code, said, cargo = run([])
            assert code == 0, said
            assert said.splitlines()[0] == "the helper: no render-link tree at %s; not built" % mge, said
            assert ["-p", "mgeHost64"] not in cargo and len(cargo) == 2, cargo
            warning = (PLUGIN / "src" / "lib.rs", 1, "warning", "unresolved link")
            build = lambda folder, args: fake_build(folder, args, [warning])  # noqa: E731
            assert run([])[0] == 1, "a warning in our code fails with the helper skipped"
            build = fake_build
            (mge / "mgeHost64").mkdir(parents=True)
            code, said, cargo = run([])
            assert code == 0 and "no render-link tree" not in said and ["-p", "mgeHost64"] in cargo, said
    finally:
        build, sha, SIBLING_MGE = real_build, real_sha, real_sibling
    print("docs self-test: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
