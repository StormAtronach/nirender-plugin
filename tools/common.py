"""What the tools share: the names of the renderer functions, the port macros, the report.

The game side of a run is in `session.py`.
"""
import json
import os
import re

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
# The folder of the sibling repos (MGE-XE-G7-render-link, moreFPS), next to this one.
SIBLINGS = os.path.dirname(REPO)


def local(key, default):
    """A place of this machine that a tool needs and the repo does not hold: the key of
    `local.json` at the repo root (not tracked; `local.example.json` names the keys), else
    `default`, a place next to this repo or under the home folder."""
    path = os.path.join(REPO, "local.json")
    if os.path.isfile(path):
        values = json.load(open(path, encoding="utf-8"))
        if key in values:
            return values[key]
    return default

# The functions of the renderer subsystem, by their name in the IDA database.
FUNC_RE = re.compile(r"^(NiDX8|NiD3D|NiRenderState|NiRenderer|NiGetD3DFormat|NiPropertyState"
                     # The scene traversal (plan 0.5 entry T2, 2026-10-16).
                     r"|NiCamera::Click$|NiAVObject::CullShow$|NiNode::Display$|NiNode::IsVisualObject$|NiTriShape::Display$"
                     r"|NiParticles::Display$|NiLines::Display$|NiAutoNormalParticles::Display$|NiSwitchNode::Display$"
                     r"|NiFltAnimationNode::Display$|NiBSPNode::Display$|NiBillboardNode::Display$|NiBillboardNode::RotateToCamera$"
                     r"|NiSortAdjustNode::Display$|NiGeometry::pushStateToRenderer$|NiAccumulator::|NiClusterAccumulator::"
                     r"|NiAlphaAccumulator::RegisterObject$|NiBound::WhichSide$"
                     r"|BSAnimationNode::Display$|NiLODNode::Display$|NiTriStrips::Display$"
                     # The readers of the display flag (plan F8, part 1, 2026-10-17).
                     r"|BSAnimationManager::Update$|BSAnimationNode::UpdateControllers$"
                     r"|BSAnimationNode::UpdateDownwardPass$"
                     # The update of a subtree from its root (D6 step 2, 2026-10-19; MWSE hooks
                     # its entry, so it is in the table but never a port).
                     r"|NiAVObject::Update$"
                     r"|NiNode::UpdateDownwardPass$"
                     # The new property and effect states of a shape (plan 8b, 2026-10-17).
                     r"|NiGeometry::UpdatePropertiesDownward$|NiGeometry::UpdateEffectsDownward$"
                     # The events of the table walk (entry 13, 2026-10-17; the walk and its ports
                     # were removed on 2026-10-20, the functions stay in the table for the census).
                     r"|NiGeometry::UpdateWorldData$|NiAVObject::SetAppCulled$"
                     r"|NiNode::AttachChild$|NiNode::DetachChild$|NiNode::DetachChildAt$|NiNode::SetChildAt$"
                     r"|ActorAnimController::setOpacity$"
                     r"|NiFlipController::Update$|NiAlphaController::Update$|NiMaterialColorController::Update$"
                     r"|NiUVController::Update$|WorldController::updateEnchantedItemEffect$"
                     r"|NiGeometry::dtor$)")
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
