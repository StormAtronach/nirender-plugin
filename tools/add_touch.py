"""Puts the execution counter at the top of each port function.

A port function is called through its detour, or directly from another port. Only the first
kind is counted by the detour, so each port function counts itself:

    crate::shadow::touch(&some_port::STATE);

This script adds that line to every function named by `rust = ...` in a port macro that does
not have it yet. Run it after adding ports.

    python tools/add_touch.py
"""
import glob
import os
import re

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PORT = re.compile(r"port_(?:thiscall|cdecl)!\(\s*(\w+),\s*0x[0-9A-Fa-f]+,.*?rust\s*=\s*([\w:]+)\s*\)", re.S)


def main():
    added = 0
    for path in sorted(glob.glob(os.path.join(REPO, "src", "ports", "*.rs"))):
        if path.endswith("mod.rs"):
            continue
        text = open(path, encoding="utf-8").read()
        for port, function in PORT.findall(text):
            name = function.split("::")[-1]
            if "::" in function and not function.startswith("self::"):
                print("%s: %s is not a function of this file, skipped" % (os.path.basename(path), function))
                continue
            start = re.search(r"^pub(?:\(crate\))? unsafe fn %s\b|^unsafe fn %s\b|^pub fn %s\b|^fn %s\b" % ((re.escape(name),) * 4), text, re.M)
            if not start:
                print("%s: no definition of %s" % (os.path.basename(path), name))
                continue
            brace = text.index("{\n", start.start())
            line = "\n    crate::shadow::touch(&%s::STATE);" % port
            if text[brace + 1:brace + 1 + len(line)] == line:
                continue
            # A generic function has `<` before its `(`; its body still starts at the first "{\n".
            text = text[:brace + 1] + line + text[brace + 1:]
            added += 1
        open(path, "w", encoding="utf-8", newline="\n").write(text)
    print("counters added: %d" % added)


if __name__ == "__main__":
    main()
