"""Checks that an edit changed comments only.

Compares each given file with its version in git, without the comments and the white space.
The default version is HEAD. Works for Rust, Lua and Python sources.

    python tools/check_comments_only.py [--ref <commit>] <file> [<file> ...]

Prints one line for each file and ends with 1 when the code of a file changed.
"""
import os
import re
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def strip_rust(text):
    out, i, n = [], 0, len(text)
    while i < n:
        c = text[i]
        two = text[i:i + 2]
        if two == "//":
            while i < n and text[i] != "\n":
                i += 1
        elif two == "/*":
            depth, i = 1, i + 2
            while i < n and depth:
                if text[i:i + 2] == "/*":
                    depth, i = depth + 1, i + 2
                elif text[i:i + 2] == "*/":
                    depth, i = depth - 1, i + 2
                else:
                    i += 1
        elif c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            out.append(text[i:j + 1])
            i = j + 1
        elif c == "r" and re.match(r'r#*"', text[i:i + 12]) and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            hashes = len(re.match(r"r(#*)", text[i:]).group(1))
            end = text.index('"' + "#" * hashes, i + 2 + hashes)
            out.append(text[i:end + 1 + hashes])
            i = end + 1 + hashes
        elif c == "'":
            # This is a character literal and not a lifetime: 'x' or '\n' or '\u{..}'.
            m = re.match(r"'(\\.[^']*|[^\\'])'", text[i:i + 12])
            if m:
                out.append(m.group(0))
                i += len(m.group(0))
            else:
                out.append(c)
                i += 1
        else:
            out.append(c)
            i += 1
    return "".join(out)


def strip_hash_or_dash(text, marker):
    """Line comments of Python (#) and Lua (--), outside simple strings."""
    out = []
    for line in text.splitlines():
        result, quote, i = [], None, 0
        while i < len(line):
            c = line[i]
            if quote:
                result.append(c)
                if c == "\\":
                    result.append(line[i + 1:i + 2])
                    i += 1
                elif c == quote:
                    quote = None
            elif c in "\"'":
                quote = c
                result.append(c)
            elif line.startswith(marker, i):
                break
            else:
                result.append(c)
            i += 1
        out.append("".join(result))
    return "\n".join(out)


def code_of(path, text):
    if path.endswith(".rs"):
        text = strip_rust(text)
    elif path.endswith(".lua"):
        text = strip_hash_or_dash(text, "--")
    elif path.endswith(".py"):
        # Docstrings count as code here. Change them with care and read the diff.
        text = strip_hash_or_dash(text, "#")
    return re.sub(r"\s+", "", text)


def main():
    args = sys.argv[1:]
    ref = "HEAD"
    if args[:1] == ["--ref"]:
        ref, args = args[1], args[2:]
    bad = 0
    for path in args:
        rel = os.path.relpath(os.path.abspath(path), REPO).replace("\\", "/")
        old = subprocess.run(["git", "-C", REPO, "show", "%s:%s" % (ref, rel)], capture_output=True)
        if old.returncode != 0:
            print("NEW      %s" % rel)
            continue
        new = open(os.path.join(REPO, rel), encoding="utf-8").read()
        same = code_of(rel, old.stdout.decode("utf-8")) == code_of(rel, new)
        print("%s %s" % ("same code" if same else "CODE CHANGED", rel))
        bad += not same
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
