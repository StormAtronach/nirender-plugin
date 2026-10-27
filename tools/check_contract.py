"""Checks the render link contract.

    python tools/check_contract.py            # the header against the plugin
    python tools/check_contract.py --mge DIR  # also against MGE XE's d3d8.dll in DIR
    python tools/check_contract.py --self-test

1. Each `#define` number of the header has the same value in `src/link.rs`.
2. Each struct size of the header has the same value in `src/link.rs`.
3. Each function of MGE XE's d3d8.dll that the header declares (all but the host's own
   MGE_RenderLinkConnect) is looked up by its name in the plugin's `src` (GetProcAddress with
   b"NAME\0" or c"NAME"): the plugin uses each call of the contract.
4. With --mge: each of those functions is an export of `d3d8/cpp/exports.def` there.

The exit code is 0 when all checks pass.
"""
import argparse
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import REPO  # noqa: E402

HEADER = os.path.join(REPO, "contract", "mge_render_link.h")
RUST = os.path.join(REPO, "src", "link.rs")

# Header prefixes, longest first. The Rust name is the header name without its prefix.
PREFIXES = ("MGE_RENDER_LINK_", "MGE_LINK_")


def rust_name(name):
    for prefix in PREFIXES:
        if name.startswith(prefix):
            return name[len(prefix):]
    return name


def camel(name):
    """`MgeLinkSceneV1` and `MgeRenderLinkHostV1` to `SceneV1` and `HostV1`."""
    return re.sub(r"^Mge(Render)?Link", "", name)


MGE = os.path.join(os.path.dirname(REPO), "MGE-XE-G7-render-link")
# The one export of the host, which the plugin does not look up: it is the host.
HOST_EXPORTS = {"MGE_RenderLinkConnect"}


def declared_functions(text):
    """The functions of MGE XE's d3d8.dll that the header declares."""
    names = re.findall(r"^[^/#\n]*?\b__cdecl\s+(MGE_\w+)\s*\(", text, re.M)
    return sorted(set(names) - HOST_EXPORTS)


def looked_up(name, sources):
    """True when a Rust text of `sources` looks the function up by its name."""
    return any('b"%s\\0"' % name in text or 'c"%s"' % name in text for text in sources)


def exported_names(definition):
    """The names that a .def file exports: `NAME` or `NAME=INTERNAL` lines after EXPORTS."""
    names, inside = set(), False
    for line in definition.splitlines():
        line = line.split(";")[0].strip()
        if line.upper() == "EXPORTS":
            inside = True
        elif inside and line:
            names.add(line.split("=")[0].split()[0])
    return names


def rust_sources():
    texts = []
    for root, _, files in os.walk(os.path.join(REPO, "src")):
        texts += [open(os.path.join(root, name), encoding="utf-8").read() for name in files if name.endswith(".rs")]
    return texts


def self_test():
    header = "// void __cdecl MGE_Old(void);\n#define X 1u\nvoid __cdecl MGE_StreamSceneBegin(const MgeLinkSceneV1* scene);\n" \
             "const MgeRenderLinkHostV1* __cdecl MGE_RenderLinkConnect(uint32_t clientVersion);\nvoid __cdecl MGE_LogLine(const char* text);\n" \
             "    void (__cdecl* sceneEnd)(void);\n"
    assert declared_functions(header) == ["MGE_LogLine", "MGE_StreamSceneBegin"], declared_functions(header)
    rust = ['let f = GetProcAddress(module, b"MGE_LogLine\\0".as_ptr());', 'GetProcAddress(m, c"MGE_StreamSceneBegin".as_ptr())']
    assert looked_up("MGE_LogLine", rust) and looked_up("MGE_StreamSceneBegin", rust)
    assert not looked_up("MGE_Log", rust)
    definition = "LIBRARY d3d8\nEXPORTS\nDirect3DCreate8=FakeDirect3DCreate\nMGE_LogLine\n; a note\nNirenderD3D8=MgeRendererWgpuActive\n"
    assert exported_names(definition) == {"Direct3DCreate8", "MGE_LogLine", "NirenderD3D8"}, exported_names(definition)
    print("check_contract self-test: ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mge", nargs="?", const=MGE, help="also check the exports of MGE XE's d3d8.dll in this folder (default: the render-link worktree)")
    ap.add_argument("--self-test", action="store_true", help="the tests of the parsing")
    args = ap.parse_args()
    if args.self_test:
        self_test()
        return 0
    header = open(HEADER, "rb").read()
    rust = open(RUST, encoding="utf-8").read()
    problems = []

    text = header.decode("ascii")
    rust_consts = {m.group(1): int(m.group(2), 0) for m in re.finditer(r"pub const (\w+): u32 = (\w+);", rust)}
    defines = re.findall(r"^#define (MGE_\w+) (0x[0-9A-Fa-f]+|\d+)u$", text, re.M)
    for name, value in defines:
        short = rust_name(name)
        if short not in rust_consts:
            problems.append("%s has no constant %s in link.rs" % (name, short))
        elif rust_consts[short] != int(value, 0):
            problems.append("%s is %s in the header and %d in link.rs" % (name, value, rust_consts[short]))

    header_sizes = dict(re.findall(r"static_assert\(sizeof\((\w+)\) == (\d+)", text))
    header_sizes.pop("void", None)
    rust_sizes = dict(re.findall(r"assert!\(size_of::<(\w+)>\(\) == (\d+)\);", rust))
    for name, size in header_sizes.items():
        short = camel(name)
        if rust_sizes.get(short) != size:
            problems.append("%s is %s bytes in the header and %s in link.rs" % (name, size, rust_sizes.get(short)))

    functions = declared_functions(text)
    sources = rust_sources()
    for name in functions:
        if not looked_up(name, sources):
            problems.append("%s is declared in the header, and no file of src looks it up by its name" % name)
    if args.mge:
        definition = os.path.join(args.mge, "d3d8", "cpp", "exports.def")
        exported = exported_names(open(definition, encoding="utf-8").read())
        for name in functions:
            if name not in exported:
                problems.append("%s is declared in the header and not exported in %s" % (name, definition))

    for problem in problems:
        print("FAIL: " + problem)
    if not problems:
        print("contract ok: %d constants, %d structs, %d functions%s" % (len(defines), len(header_sizes), len(functions), " (and exported by d3d8.dll)" if args.mge else ""))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
