"""Checks the render link contract.

    python tools/check_contract.py            # the header against the plugin
    python tools/check_contract.py --mge DIR  # also against MGE XE's d3d8.dll in DIR
    python tools/check_contract.py --self-test

1. Each `#define` number of the header has the same value in `src/link.rs`.
2. Each struct size of the header has the same value in `src/link.rs`.
3. Each function of MGE XE's d3d8.dll that the header declares (all but the host's own
   MGE_RenderLinkConnect) is looked up by its name in the plugin's `src` (GetProcAddress with
   b"NAME\0" or c"NAME"): the plugin uses each call of the contract.
4. With --mge: each of those functions is an export of `d3d8/cpp/exports.def` there, and
   each `extern "C"` declaration or definition of it in `d3d8/cpp` has the header's return
   type and parameter types (the names may differ), and no exception specification
   (`noexcept`, `throw()`), which the C header cannot have. A file of d3d8.dll that sees both
   declarations stops at a difference (C2382 on 2026-10-27, after the merge of step 3); d3d8's
   build reads the header from the main nirender-plugin checkout, so a build of a branch can
   miss it, and this check needs no compile.

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


SIGNATURE = r"(?:^|\n)[ \t]*%s([\w \t\*]+?)\s*__cdecl\s+(%s)\s*\(([^)]*)\)\s*(noexcept|throw\s*\(\s*\))?"


def normal_type(text):
    """A type as one spelling: single spaces, none around `*`."""
    text = re.sub(r"\s+", " ", text.strip())
    return re.sub(r"\s*\*\s*", "*", text)


def parameter_types(parameters):
    """The types of a parameter list, without the names."""
    types = []
    for parameter in [p.strip() for p in parameters.split(",") if p.strip()]:
        if parameter != "void":
            # The last word is the name, unless the parameter is only a type ending in `*`.
            parameter = re.sub(r"(?<=[\w\*\]\s])\s*\b[A-Za-z_]\w*\s*(\[[^\]]*\])?$", lambda m: "*" if m.group(1) else "", parameter) or parameter
        types.append(normal_type(parameter))
    return types


def signatures(text, names, extern_c):
    """{name: [(return type, parameter types, exception specification)]} of the functions of
    `names` that `text` declares or defines (with `extern "C"` first when `extern_c`)."""
    found = {}
    prefix = r'extern\s+"C"\s+' if extern_c else ""
    for match in re.finditer(SIGNATURE % (prefix, "|".join(re.escape(n) for n in names)), text):
        returned, name, parameters, exception = match.groups()
        found.setdefault(name, []).append((normal_type(returned), parameter_types(parameters), exception))
    return found


def declaration_problems(header, cpp_files, names):
    """The differences between the header's declaration of each function and d3d8's."""
    problems = []
    contract = signatures(header, names, False)
    for path, text in cpp_files:
        for name, found in signatures(text, names, True).items():
            want = contract.get(name)
            if not want:
                continue
            returned, parameters, _ = want[0]
            for got_returned, got_parameters, exception in found:
                if exception:
                    problems.append("%s: %s is declared %s, and the contract (a C header) is not: a file that sees both stops (C2382)" % (path, name, exception))
                if (got_returned, got_parameters) != (returned, parameters):
                    problems.append("%s: %s is %s(%s), and the contract has %s(%s)" % (path, name, got_returned, ", ".join(got_parameters), returned, ", ".join(parameters)))
    return problems


def cpp_files(mge):
    found = []
    for root, _, files in os.walk(os.path.join(mge, "d3d8", "cpp")):
        for name in files:
            if name.endswith((".h", ".hpp", ".cpp")):
                path = os.path.join(root, name)
                found.append((os.path.relpath(path, mge).replace("\\", "/"), open(path, encoding="utf-8", errors="replace").read()))
    return found


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
    # The declarations of d3d8.dll against the contract's.
    contract = "void __cdecl MGE_LogLine(const char* text);\nvoid __cdecl MGE_StreamSceneBegin(const MgeLinkSceneV1* scene);\n"
    names = ["MGE_LogLine", "MGE_StreamSceneBegin"]
    good = [("log.h", 'extern "C" void __cdecl MGE_LogLine(const char* line);\n'),
            ("main.cpp", 'extern "C" void __cdecl MGE_StreamSceneBegin(const MgeLinkSceneV1 *packet) {\n')]
    assert declaration_problems(contract, good, names) == [], declaration_problems(contract, good, names)
    noexcept = [("log.h", 'extern "C" void __cdecl MGE_LogLine(const char* text) noexcept;\n')]
    assert len(declaration_problems(contract, noexcept, names)) == 1 and "C2382" in declaration_problems(contract, noexcept, names)[0]
    other = [("log.cpp", 'extern "C" int __cdecl MGE_LogLine(const wchar_t* text) {\n')]
    assert len(declaration_problems(contract, other, names)) == 1
    assert parameter_types("const char* text") == ["const char*"] and parameter_types("void") == ["void"]
    assert parameter_types("uint32_t clientVersion") == ["uint32_t"] and parameter_types("double origin[3]") == ["double*"]
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
        problems += declaration_problems(text, cpp_files(args.mge), functions)

    for problem in problems:
        print("FAIL: " + problem)
    if not problems:
        print("contract ok: %d constants, %d structs, %d functions%s" % (len(defines), len(header_sizes), len(functions), " (and exported by d3d8.dll)" if args.mge else ""))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
