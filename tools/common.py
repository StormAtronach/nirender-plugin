"""What the tools share: the names of the renderer functions, the port macros, the report.

The game side of a run is in `session.py`.
"""
import os
import re

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)

# The functions of the renderer subsystem, by their name in the IDA database.
FUNC_RE = re.compile(r"^(NiDX8|NiD3D|NiRenderState|NiRenderer|NiGetD3DFormat|NiPropertyState)")
FUNC_SKIP = re.compile(r"_SEH$|NiDX8Application|NiDX8SelectResolution|StdMap")

# A port declaration in src/ports/*.rs: the module name, the address, the Rust function.
PORT_RE = re.compile(r"port_(?:thiscall|cdecl)!\(\s*(\w+),\s*0x([0-9A-Fa-f]+),.*?rust\s*=\s*([\w:]+)\s*\)", re.S)


def is_renderer_function(name):
    return bool(FUNC_RE.search(name)) and not FUNC_SKIP.search(name)


def read_report(path):
    """The rows of a report.tsv, each as a dict from column name to text."""
    if not os.path.isfile(path):
        return []
    lines = open(path, encoding="utf-8").read().splitlines()
    if not lines:
        return []
    header = lines[0].split("\t")
    return [dict(zip(header, line.split("\t"))) for line in lines[1:] if line.strip()]
