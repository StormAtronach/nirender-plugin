# Runs inside IDA (py_exec_file). Writes the pseudocode and the disassembly of functions to
# tools/out/dump/<ea>.txt. tools/out/dump_request.txt lists the functions, one address per line.
import os

import ida_funcs
import ida_hexrays
import ida_lines
import idautils
import idc

OUT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "out")  # tools/out; IDA sets __file__ for a script file
os.makedirs(os.path.join(OUT, "dump"), exist_ok=True)
n = 0
for line in open(os.path.join(OUT, "dump_request.txt")):
    line = line.strip()
    if not line:
        continue
    ea = int(line, 16)
    f = ida_funcs.get_func(ea)
    out = ["// %s @ 0x%x  type: %s" % (idc.get_func_name(ea), ea, idc.get_type(ea))]
    try:
        out.append(str(ida_hexrays.decompile(ea)))
    except Exception as e:
        out.append("// decompile failed: %s" % e)
    out.append("\n// ---- disassembly")
    for head in idautils.FuncItems(ea):
        out.append("%06x  %s" % (head, ida_lines.tag_remove(idc.generate_disasm_line(head, 0))))
    open(os.path.join(OUT, "dump", "%06x.txt" % ea), "w", encoding="utf-8").write("\n".join(out))
    n += 1
print("dumped", n)
