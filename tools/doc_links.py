"""Makes the names in backticks of the doc comments links that rustdoc checks.

    python -I tools/doc_links.py --cargo-dir . --src src
    python -I tools/doc_links.py --cargo-dir host --src host/src --cargo-args=--lib
    python -I tools/doc_links.py --cargo-dir ../MGE-XE-G7-render-link --src ../MGE-XE-G7-render-link/mgeHost64/src/renderer.rs ../MGE-XE-G7-render-link/mgeHost64/src/renderer --cargo-args="-p mgeHost64"
    ... --apply        keep the links that resolve (without it, every file is restored)
    ... --list FILE    write each link that resolves, one a line
    ... --kinds K,...  the kinds to convert: path (`a::b`), call (`name()`), type (`Name`),
                       constant (`NAME`), word (`name`); all by default. A word is often the
                       name of a parameter, which rustdoc cannot link: it then resolves to
                       another item of that name.

A doc comment (`///` or `//!`) names an item in backticks: `Renderer::particles`. Rustdoc
checks a name only as a link: [`Renderer::particles`]. The tool writes every name in
backticks that looks like a Rust path (`a::b`, `Name`, `name()`) as a link, runs `cargo doc
--no-deps --document-private-items`, and writes back each link that rustdoc does not
resolve (an engine function, a switch, a word). It runs cargo again until no new warning
comes. A link that resolves then fails the doc build when its item is renamed or removed.

Not converted: a name in a fenced code block, a span of two or more backticks, a span over
two lines, a span already in a link, a span followed by `(` or `[`. The line endings of each
file stay as they are.
"""
import argparse
import collections
import json
import re
import shlex
import subprocess
import sys
from pathlib import Path

PATH = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*(::[A-Za-z_][A-Za-z0-9_]*)*(\(\))?$")
NAME_IN_MESSAGE = re.compile(r"`([^`]+)`")


def doc_marker(line):
    """The column after `///` or `//!`, or None when the line is not a doc comment."""
    stripped = line.lstrip()
    at = len(line) - len(stripped)
    if stripped.startswith("//!") or (stripped.startswith("///") and not stripped.startswith("////")):
        return at + 3
    return None


class Span:
    def __init__(self, path, line, start, end, name, block):
        self.path, self.line, self.start, self.end, self.name, self.block = path, line, start, end, name, block
        self.kept = True


def find_spans(path, lines):
    """The spans of one file that can become links: (line index, column of the first backtick,
    column after the last backtick, name). `block` numbers the doc block of the span."""
    spans = []
    block, in_block, fenced = 0, False, False
    for n, line in enumerate(lines):
        text = line.rstrip("\r\n")
        at = doc_marker(text)
        if at is None:
            if in_block:
                block += 1
            in_block, fenced = False, False
            continue
        in_block = True
        content = text[at:]
        if content.strip().startswith("```"):
            fenced = not fenced
            continue
        if fenced or content.startswith("     "):
            continue
        if "``" in content or content.count("`") % 2:
            continue
        for m in re.finditer(r"`([^`]+)`", content):
            name = m.group(1)
            if not PATH.match(name):
                continue
            start, end = at + m.start(), at + m.end()
            before = text[start - 1] if start > 0 else ""
            after = text[end] if end < len(text) else ""
            if before == "[" or after in ("]", "(", "["):
                continue
            if after == ":" and not content[:m.start()].strip():
                continue
            spans.append(Span(path, n, start, end, name, block))
    return spans


def write(path, lines, spans):
    """Writes the file with the kept spans as links; the rest as the original text."""
    out = list(lines)
    by_line = collections.defaultdict(list)
    for s in spans:
        if s.kept:
            by_line[s.line].append(s)
    for n, items in by_line.items():
        line = out[n]
        for s in sorted(items, key=lambda s: s.start, reverse=True):
            line = line[:s.start] + "[" + line[s.start:s.end] + "]" + line[s.end:]
        out[n] = line
    path.write_text("".join(out), encoding="utf-8", newline="")


def warnings(cargo_dir, cargo_args):
    """The rustdoc warnings of the crate: (file, line 1-based, message)."""
    cmd = ["cargo", "doc", "--no-deps", "--document-private-items", "--message-format=json"] + cargo_args
    run = subprocess.run(cmd, cwd=cargo_dir, capture_output=True, text=True, encoding="utf-8", errors="replace")
    found = []
    for raw in run.stdout.splitlines():
        try:
            msg = json.loads(raw)
        except ValueError:
            continue
        if msg.get("reason") != "compiler-message":
            continue
        diag = msg["message"]
        if diag.get("level") not in ("warning", "error"):
            continue
        for span in diag.get("spans", []):
            if span.get("is_primary"):
                file = (Path(cargo_dir) / span["file_name"]).resolve()
                found.append((file, span["line_start"], span["line_end"], diag["message"]))
    # A broken link is an error where the crate denies it: cargo then fails with diagnostics.
    if run.returncode != 0 and not found:
        print(run.stderr[-3000:], file=sys.stderr)
        raise SystemExit("cargo doc failed with exit code %d" % run.returncode)
    return found


def kind(name):
    if "::" in name:
        return "path"
    if name.endswith("()"):
        return "call"
    if name[0].isupper() and not name.isupper():
        return "type"
    if name.isupper():
        return "constant"
    return "word"


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("--cargo-dir", required=True)
    ap.add_argument("--src", required=True, nargs="+", help="folders or files")
    ap.add_argument("--kinds", default="path,call,type,constant,word",
                    help="the kinds of names to convert (a word can be a parameter's name)")
    ap.add_argument("--cargo-args", default="")
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--list")
    args = ap.parse_args(argv)
    cargo_dir = Path(args.cargo_dir).resolve()
    cargo_args = shlex.split(args.cargo_args)

    files = {}
    kinds = set(args.kinds.split(","))
    paths = []
    for src in args.src:
        src = Path(src).resolve()
        paths += [src] if src.is_file() else sorted(src.rglob("*.rs"))
    for path in paths:
        with open(path, encoding="utf-8", newline="") as source:
            lines = source.read().splitlines(keepends=True)
        files[path] = (lines, [s for s in find_spans(path, lines) if kind(s.name) in kinds])
    total = sum(len(s) for _, s in files.values())
    print("%d spans in %d files" % (total, len(files)))

    baseline = {(f, l, m) for f, l, _, m in warnings(cargo_dir, cargo_args)}
    print("%d warnings before" % len(baseline))
    try:
        for round_ in range(1, 6):
            # Every file with spans, also one whose links were all written back in a round.
            for path, (lines, spans) in files.items():
                if spans:
                    write(path, lines, spans)
            new = [w for w in warnings(cargo_dir, cargo_args) if w[:2] + w[3:] not in baseline]
            reverted = 0
            for file, first, last, message in new:
                if file not in files:
                    continue
                spans = files[file][1]
                # The message names `name` for a link written `name()`.
                names = {n.removesuffix("()") for n in NAME_IN_MESSAGE.findall(message)}
                named = [s for s in spans if s.kept and s.name.removesuffix("()") in names]
                # The span of the warning; rustdoc can give the span of the whole doc block.
                hit = [s for s in named if first - 1 <= s.line <= last - 1]
                if not hit:
                    blocks = {s.block for s in spans if first - 1 <= s.line <= last - 1}
                    hit = [s for s in named if s.block in blocks]
                for s in hit:
                    s.kept = False
                    reverted += 1
            print("round %d: %d new warnings, %d links written back" % (round_, len(new), reverted))
            if not new:
                break
            if not reverted:
                for file, first, last, message in new[:20]:
                    print("  not matched: %s:%d-%d %s" % (file, first, last, message))
                raise SystemExit("new warnings the tool cannot match to a link")
    finally:
        if not args.apply:
            for path, (lines, _) in files.items():
                path.write_text("".join(lines), encoding="utf-8", newline="")

    kept = [s for _, spans in files.values() for s in spans if s.kept]
    by_kind = collections.Counter(kind(s.name) for s in kept)
    all_kind = collections.Counter(kind(s.name) for _, spans in files.values() for s in spans)
    print("%d of %d spans resolve as links" % (len(kept), total))
    for k in ("path", "call", "type", "constant", "word"):
        print("  %-8s %5d of %5d" % (k, by_kind[k], all_kind[k]))
    print("%d files with a link" % len({s.path for s in kept}))
    if args.list:
        with open(args.list, "w", encoding="utf-8", newline="\n") as out:
            for s in kept:
                out.write("%s:%d %s %s\n" % (s.path.name, s.line + 1, kind(s.name), s.name))
    print("applied" if args.apply else "restored every file")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
