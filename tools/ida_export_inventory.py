# Runs inside IDA (py_exec_file). Exports the renderer function inventory and the struct
# layouts that the port needs. Output: tools/out/inventory.json, tools/out/structs.json
import json
import os
import re

import ida_bytes
import ida_funcs
import ida_typeinf
import idautils
import idc

OUT = r"c:\Users\ivan_\source\repos\nirender-plugin\tools\out"
os.makedirs(OUT, exist_ok=True)

# The renderer subsystem by name. The address window also finds the helpers without a name
# between them.
NAME_RE = re.compile(r"NiDX8|NiD3D|NiRenderState|NiRenderer|NiGeometryBufferData|NiDX8", re.I)
WINDOWS = [(0x6A9000, 0x6C6000), (0x65ED00, 0x65EF00)]
# The scene traversal that the plugin takes over (plan 0.5 entry T2, 2026-10-16): the cull
# and display of the scene graph, and the accumulators that sort the blended shapes.
EXTRA = [
    0x6CC7B0,  # NiCamera::Click
    0x6EB480,  # NiAVObject::CullShow
    0x6C9190,  # NiNode::Display
    0x445A60,  # NiNode::IsVisualObject
    0x6E4F90,  # NiTriShape::Display
    0x6DD3E0,  # NiParticles::Display
    0x6DA850,  # NiLines::Display
    0x6D3540,  # NiAutoNormalParticles::Display
    0x6D8320,  # NiSwitchNode::Display
    0x6D90B0,  # NiFltAnimationNode::Display
    0x6D6470,  # NiBSPNode::Display
    0x6D4040,  # NiBillboardNode::Display
    0x6D3C00,  # NiBillboardNode::RotateToCamera
    0x6DE030,  # NiSortAdjustNode::Display
    0x6F3F40,  # NiGeometry::pushStateToRenderer
    0x6F5070,  # NiRenderer::RegisterObject
    0x6F51D0,  # NiRenderer::renderFromClusterAccumulator
    0x6F9170,  # NiAccumulator::StartAccumulating
    0x6F9190,  # NiAccumulator::FinishAccumulating
    0x6CF090,  # NiClusterAccumulator::StartAccumulating
    0x6CF0E0,  # NiClusterAccumulator::FinishAccumulating
    0x6CF200,  # NiClusterAccumulator::RegisterObject
    0x42BAD0,  # NiClusterAccumulator::$1::registerNewNode
    0x429B90,  # NiAlphaAccumulator::RegisterObject
    0x6F2660,  # NiBound::WhichSide
    0x6F21C0,  # BSAnimationNode::Display
    0x7412B0,  # NiLODNode::Display
    0x742660,  # NiTriStrips::Display
    0x6F3DB0,  # NiGeometry::dtor (the end of a retained instance, entry T3)
]


def in_window(ea):
    return any(lo <= ea < hi for lo, hi in WINDOWS)


funcs = []
for ea in idautils.Functions():
    name = idc.get_func_name(ea)
    if not (NAME_RE.search(name) or in_window(ea) or ea in EXTRA):
        continue
    f = ida_funcs.get_func(ea)
    size = f.end_ea - f.start_ea
    chunks = [(s, e) for s, e in idautils.Chunks(ea)]
    callers = sorted({ida_funcs.get_func(x.frm).start_ea for x in idautils.XrefsTo(ea, 0)
                      if x.iscode and ida_funcs.get_func(x.frm)})
    data_refs = sorted({x.frm for x in idautils.XrefsTo(ea, 0) if not x.iscode})
    # The bytes that a 5-byte detour takes from the entry, as whole instructions. `pic` tells if
    # the detour can copy them to another address unchanged. That is true when they have no
    # relative branch and no branch from the function goes into them.
    # `relocs` are the offsets of 32-bit relative targets (call or jmp rel32) in those bytes.
    # The detour moves them with the code. The detour takes all of a function shorter than 5
    # bytes.
    steal, pic, relocs = 0, True, []
    while steal < 5 and ea + steal < f.end_ea:
        insn = idautils.DecodeInstruction(ea + steal)
        if insn is None:
            pic = False
            break
        mnem = insn.get_canon_mnem()
        first = ida_bytes.get_byte(ea + steal)
        if first in (0xE8, 0xE9) and insn.size == 5:
            relocs.append(steal + 1)
        elif mnem.startswith("ret") or (mnem in ("jmp", "call") and first == 0xFF):
            pass  # A return or an indirect branch runs the same at any address.
        elif mnem.startswith("j") or mnem.startswith("call") or mnem.startswith("loop"):
            pic = False
        steal += insn.size
    if steal < 5 and steal != size:
        pic = False
    for head in idautils.FuncItems(ea):
        for x in idautils.XrefsFrom(head, 0):
            if x.iscode and ea < x.to < ea + steal and head != idc.prev_head(x.to):
                pic = False
    funcs.append({
        "bytes": ida_bytes.get_bytes(ea, size).hex(),
        "steal": steal,
        "steal_pic": pic,
        "relocs": relocs,
        "ea": ea,
        "name": name,
        "size": size,
        "chunks": chunks,
        "type": idc.get_type(ea) or "",
        "argsize": f.argsize,
        "head": ida_bytes.get_bytes(ea, min(size, 16)).hex(),
        "callers": callers,
        "data_refs": data_refs,
        "in_window": in_window(ea),
    })

json.dump(funcs, open(os.path.join(OUT, "inventory.json"), "w"), indent=1)

STRUCT_RE = re.compile(
    r"^(NiDX8|NiD3D|NiRender|NiGeometryBufferData|NiPropertyState|NiDynamicEffectState|NiRenderStateSetting|"
    r"NiAlphaProperty|NiZBufferProperty|NiStencilProperty|NiFogProperty|NiMaterialProperty|NiTexturingProperty|"
    r"NiVertexColorProperty|NiWireframeProperty|NiShadeProperty|NiDitherProperty|NiSpecularProperty|NiProperty$|"
    r"NiTransform$|NiBound$|NiMatrix33$|NiPoint3$|NiFrustum$|NiObject$|NiObjectNET$|NiPointer$|THashMap|"
    r"NiGeometryData$|NiTriBasedGeomData$|NiTriShapeData$|NiTriStripsData$|NiSkinInstance$|NiSkinPartition|"
    r"NiSourceTexture$|NiTexture$|NiPixelData$|NiPixelFormat$|NiLight$|NiDynamicEffect$|NiAVObject$|NiGeometry$|"
    r"BatchedObject|NiPackedColor|NiColor$|NiColorA$|D3DMATRIX|D3DVIEWPORT8|D3DMATERIAL8|D3DLIGHT8|"
    r"NiNode$|NiSwitchNode$|NiFltAnimationNode$|NiBSPNode$|NiBillboardNode$|NiSortAdjustNode$|NiCamera$|NiPlane$|"
    r"NiAccumulator$|NiClusterAccumulator|NiAlphaAccumulator$|NiParticles$|NiParticlesData$|NiLines$|NiLinesData$|"
    r"NiTimeController$|NiTriShape$|NiAutoNormalParticles$|TArray|NiTArray|NiScreenPolygon$|"
    r"BSAnimationNode$|NiLODNode$|NiLODData$|NiTriStrips$|NiTriBasedGeometry$)")

til = ida_typeinf.get_idati()
structs = {}
limit = ida_typeinf.get_ordinal_limit(til)
for ordinal in range(1, limit):
    tif = ida_typeinf.tinfo_t()
    if not tif.get_numbered_type(til, ordinal):
        continue
    name = tif.get_type_name()
    if not name or not tif.is_udt():
        continue
    udt = ida_typeinf.udt_type_data_t()
    if not tif.get_udt_details(udt):
        continue
    members = []
    for m in udt:
        members.append({
            "name": m.name,
            "offset": m.offset // 8,
            "size": m.size // 8,
            "type": m.type.dstr(),
        })
    structs[name] = {"size": tif.get_size(), "union": tif.is_union(), "members": members}

json.dump(structs, open(os.path.join(OUT, "structs.json"), "w"), indent=1)
print("functions: %d (named %d), structs: %d" % (
    len(funcs), sum(1 for f in funcs if not f["name"].startswith("sub_")), len(structs)))
