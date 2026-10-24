"""Builds the rustdoc pages of the plugin, the host crate and the MGE XE helper, and checks
their doc comments.

    python -I tools/docs.py           # build, check, write target/docs.html
    python -I tools/docs.py --open    # the same, then open the page in the browser
    python -I tools/docs.py --mge DIR # the MGE XE repository, if not the sibling

Each crate is documented with its private items (`cargo doc --no-deps
--document-private-items`). A link in a doc comment that rustdoc cannot resolve is an error
in the plugin and the host crate (`[lints.rustdoc]` in their Cargo.toml) and in the helper's
`renderer` module (`#![deny]` in renderer.rs). The rest of the helper is MGE XE G7's code:
its warnings are listed, not failed.

The tool fails (exit 1) when a doc build fails or when a doc comment of our code has a
warning. The pages stay in the target folder of each repository; target/docs.html links them.
The tool runs cargo: ask before it runs, as before any build.
"""
import argparse
import datetime
import html
import json
import os
import subprocess
import sys
from pathlib import Path

PLUGIN = Path(__file__).resolve().parents[1]


def crates(mge):
    """(name, cargo folder, cargo arguments, the doc page, whether a file is ours)."""
    renderer = (mge / "mgeHost64" / "src" / "renderer").resolve()
    renderer_rs = renderer.with_suffix(".rs")
    return [
        ("the plugin (akulakhan)", PLUGIN, [],
         PLUGIN / "target" / "i686-pc-windows-msvc" / "doc" / "akulakhan" / "index.html", lambda f: True),
        ("the host crate (nirender_wgpu)", PLUGIN / "host", ["--lib"],
         PLUGIN / "host" / "target" / "x86_64-pc-windows-msvc" / "doc" / "nirender_wgpu" / "index.html", lambda f: True),
        ("the helper (numidium)", mge, ["-p", "mgeHost64"],
         mge / "target" / "doc" / "numidium" / "index.html",
         lambda f: f == renderer_rs or renderer in f.parents),
    ]


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


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("--mge", default=str(PLUGIN.parent / "MGE-XE-G7-render-link"))
    ap.add_argument("--open", action="store_true")
    args = ap.parse_args(argv)
    mge = Path(args.mge).resolve()
    failed, rows = False, []
    for name, folder, cargo_args, index, is_ours in crates(mge):
        code, found = build(folder, cargo_args)
        ours = [w for w in found if is_ours(w[0])]
        others = [w for w in found if not is_ours(w[0])]
        print("%s: exit %d, %d warnings in our code, %d other" % (name, code, len(ours), len(others)))
        for file, line, level, text in ours:
            print("  %s %s:%d %s" % (level, file, line, text))
        if code != 0 or ours:
            failed = True
        rows.append((name, index, sha(folder), len(ours), len(others)))
    out = PLUGIN / "target" / "docs.html"
    out.parent.mkdir(exist_ok=True)
    out.write_text(page(rows), encoding="utf-8", newline="\n")
    print("wrote %s" % out)
    if args.open:
        os.startfile(out)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
