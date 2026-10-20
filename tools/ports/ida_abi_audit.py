# Runs inside IDA (py_exec_file). For each function of the inventory it finds two things. The
# first is which of EAX, ECX and EDX a caller reads after the call returns. The second is
# whether the function itself can change ECX or EDX.
#
# Morrowind.exe was linked with whole-program optimisation. A caller can know that a callee
# keeps ECX, and use it after the call. Then a detour must leave the same value there.
# Output: tools/out/abi.json
import json
import os
import re

import ida_funcs
import idautils
import idc

OUT = r"c:\Users\ivan_\source\repos\nirender-plugin\tools\out"
FAMILY = {
    "eax": ("eax", "ax", "al", "ah"),
    "ecx": ("ecx", "cx", "cl", "ch"),
    "edx": ("edx", "dx", "dl", "dh"),
}
WRITE_ONLY = {"mov", "lea", "pop", "movzx", "movsx", "setz", "setnz", "setb", "setnb", "setbe", "setnbe", "setl",
              "setnl", "setle", "setnle", "sets", "setns", "seto", "setno", "fnstsw", "fstsw"}
# The pattern finds whole words only. "22Ch" is a number, not CH.
TOKEN = re.compile(r"(?<![0-9a-z_])[a-z]{2,3}(?![0-9a-z_])")


def regs_in(text):
    found = set()
    for token in TOKEN.findall(text.lower()):
        for reg, names in FAMILY.items():
            if token in names:
                found.add(reg)
    return found


def effect(ea):
    """(reads, writes) of EAX/ECX/EDX for one instruction, from its text."""
    mnem = idc.print_insn_mnem(ea)
    ops = [idc.print_operand(ea, i) for i in range(3)]
    ops = [o for o in ops if o]
    reads, writes = set(), set()
    if mnem in ("cdq",):
        return {"eax"}, {"edx"}
    if mnem in ("mul", "imul", "div", "idiv") and len(ops) == 1:
        return {"eax", "edx"} | regs_in(ops[0]), {"eax", "edx"}
    if mnem.startswith("rep") or mnem in ("movsd", "movsb", "movsw", "stosd", "stosb", "stosw", "scasb"):
        return {"ecx", "eax"}, {"ecx"}
    if mnem in ("shl", "shr", "sar", "rol", "ror") and len(ops) == 2 and ops[1].lower() == "cl":
        reads.add("ecx")
    for i, op in enumerate(ops):
        inside = "[" in op
        regs = regs_in(op)
        if i == 0 and not inside:
            same = mnem in ("xor", "sub") and len(ops) == 2 and ops[0] == ops[1]
            if mnem in WRITE_ONLY or same:
                writes |= regs
            elif mnem in ("cmp", "test", "push", "fld", "fild", "fcomp", "fcom"):
                reads |= regs
            else:
                reads |= regs
                writes |= regs
        elif i == 1 and mnem in ("xor", "sub") and len(ops) == 2 and ops[0] == ops[1]:
            pass
        else:
            reads |= regs
    return reads, writes


def live_after(ret_ea, limit=24):
    """The registers that the code reads before it writes them, after a call returns.

    The result is 'unknown' when a branch ends the scan."""
    live, unknown = set(), set()
    pending = {"eax", "ecx", "edx"}
    ea = ret_ea
    for _ in range(limit):
        if not pending:
            break
        mnem = idc.print_insn_mnem(ea)
        if mnem == "call":
            reads, _ = effect(ea)  # An indirect call through a register reads the register.
            live |= reads & pending
            pending -= reads
            # ECX can be the this pointer that the caller set before the call. Then the caller
            # wrote it.
            pending.clear()
            break
        if mnem.startswith("ret"):
            if "eax" in pending:
                unknown.add("eax")  # The caller can return the value of the callee.
            pending.clear()
            break
        if mnem.startswith("j") or mnem.startswith("loop"):
            unknown |= pending
            pending.clear()
            break
        reads, writes = effect(ea)
        live |= reads & pending
        pending -= reads
        pending -= writes
        ea = idc.next_head(ea)
    return live, unknown


inventory = json.load(open(os.path.join(OUT, "inventory.json")))
result = {}
for entry in inventory:
    ea = entry["ea"]
    func = ida_funcs.get_func(ea)
    writes, has_call = set(), False
    for head in idautils.FuncItems(ea):
        if idc.print_insn_mnem(head) == "call":
            has_call = True
        _, w = effect(head)
        writes |= w
    sites = []
    live_total, unknown_total = set(), set()
    for x in idautils.XrefsTo(ea, 0):
        if not x.iscode or idc.print_insn_mnem(x.frm) not in ("call", "jmp"):
            continue
        if idc.print_insn_mnem(x.frm) == "jmp":
            continue
        live, unknown = live_after(idc.next_head(x.frm))
        live_total |= live
        unknown_total |= unknown
        if live or unknown:
            sites.append({"at": x.frm, "live": sorted(live), "unknown": sorted(unknown)})
    result["%x" % ea] = {
        "name": entry["name"],
        "writes": sorted(writes),
        "has_call": has_call,
        "live": sorted(live_total),
        "unknown": sorted(unknown_total),
        "sites": sites,
    }

json.dump(result, open(os.path.join(OUT, "abi.json"), "w"), indent=1)
print("audited", len(result))
None
