//! The scene traversal of the engine, taken over 1:1 (plan 0.5 entry T2): the cull of a
//! subtree against the planes of the camera (`NiAVObject::CullShow`), the display of a node
//! and of each kind of node, the display of each kind of shape (the states, the
//! accumulator, the controllers, the render call of the renderer), the test of a bound
//! against a plane (`NiBound::WhichSide`) and the flag test of a node. Each port does the
//! reads and the writes of the engine function. The accumulators themselves (the register
//! of a blended shape, the sort and the draw at the end) stay engine code for now: the
//! ports call them.
//!
//! The test of these ports is a set of captures of a still scene in one game session, with
//! the ports off, then live, then off again (`batch-t2-live.lua` of the docs); the object
//! records of one frame of each capture must be the same (`tools/capture_dump.py <capture>
//! sequence 15`). A shadow run cannot compare them, as the Rust run of a traversal draws,
//! and the renderer ports skip the comparison of a call that draws. Two sessions of the
//! same place differ in the records with no port live, so the captures come from one.
//!
//! The FPU of the game runs at single precision (the control word has precision 24 bits),
//! so a sum of products of floats in the order of the engine gives the same bits in `f32`
//! arithmetic; `WhichSide` keeps the x87 instructions, as its compare reads the FPU flags.

use core::arch::asm;

use super::ni;
use crate::engine::{thiscall1, thiscall3, thiscall4, vcall0, vcall1, vcall3, virtual_at};
use crate::gen::layouts::{
    NiAVObject as AV, NiCamera as C, NiClusterAccumulator as CA, NiClusterAccumulator__1 as L, NiClusterAccumulator__4 as H,
    NiGeometry as G, NiGeometryData as GD, NiNode as N, NiRenderer as R, NiTimeController as TC,
};
use crate::mem::{rd, wr};
use crate::shadow::Regions;

/// The RTTI objects of the classes that the walk of a subtree tells apart.
const RTTI_NI_NODE: usize = 0x7DDEB8;
const RTTI_NI_GEOMETRY: usize = 0x7DEA2C;
/// `global_NiClusterAccumulator_cullingPlanesBitfield`: the bits of the planes the camera
/// passed when a blended shape was registered, 16 bytes (see `NiClusterAccumulator`).
const ACCUMULATOR_PLANES: usize = 0x7DE7BC;
/// `global_NiTriShape::countRenderCalls`, `global_NiGeometry::countPrimsRendered` and
/// `countVertsRendered`: the statistics of the frame.
const COUNT_RENDER_CALLS: usize = 0x7DE968;
const COUNT_PRIMS_RENDERED: usize = 0x7DE96C;
const COUNT_VERTS_RENDERED: usize = 0x7DE970;

/// The vtable slots of `NiObject` and `NiAVObject`.
const SLOT_DELETING_DTOR: usize = 0x0;
const SLOT_GET_WORLD_BOUND: usize = 0x34;
pub(crate) const SLOT_GET_APP_CULLED: usize = 0x54;
const SLOT_UPDATE_DOWNWARD_PASS: usize = 0x60;
const SLOT_IS_VISUAL_OBJECT: usize = 0x64;
pub(crate) const SLOT_DISPLAY: usize = 0x78;
/// The vtable slots of `NiRenderer`.
const SLOT_RENDER_SHAPE: usize = 0xBC;
const SLOT_RENDER_TRISTRIPS: usize = 0xC0;
const SLOT_RENDER_POINTS: usize = 0xC4;
const SLOT_RENDER_LINES: usize = 0xC8;
/// The vtable slots of `NiAccumulator`.
pub(crate) const SLOT_START_ACCUMULATING: usize = 0x2C;
pub(crate) const SLOT_FINISH_ACCUMULATING: usize = 0x30;
/// The vtable slot of `NiTimeController::OnPreDisplay`.
const SLOT_ON_PRE_DISPLAY: usize = 0x40;
/// The vtable slots of `NiGeometryData` and `NiTriBasedGeomData`.
const SLOT_GET_ACTIVE_VERTEX_COUNT: usize = 0x30;
const SLOT_GET_ACTIVE_TRIANGLE_COUNT: usize = 0x3C;

/// Bit 3 of the flags of a node: the node contains renderable geometry.
const NODE_HAS_GEOMETRY_BIT: u32 = 3;
/// Bit 4: the node was displayed in this frame.
pub(crate) const NODE_DISPLAYED: u16 = 0x10;
/// `BSAnimFlags_Displayed` in the byte at 0xB0 of a `BSAnimationNode`.
pub(crate) const ANIM_FLAGS: usize = 0xB0;
pub(crate) const ANIM_DISPLAYED: u8 = 8;
/// `NiSwitchNode`: the index of the child shown, the saved time, the revision of the update,
/// the array of the revision of each child (a TArray: storage, storage count, end index,
/// filled count), and whether the controllers update.
pub(crate) const SWITCH_INDEX: usize = 0xB0;
pub(crate) const SWITCH_SAVED_TIME: usize = 0xB4;
const SWITCH_UPDATE_REVISION: usize = 0xB8;
const SWITCH_REVISIONS_STORAGE: usize = 0xC0;
const SWITCH_REVISIONS_STORAGE_COUNT: usize = 0xC4;
const SWITCH_REVISIONS_END: usize = 0xC8;
const SWITCH_REVISIONS_FILLED: usize = 0xCC;
const SWITCH_UPDATE_CONTROLLERS: usize = 0xD4;
/// `NiLODNode`: the centre in the world and the array of `NiLODData` (16 bytes each: near,
/// far, the squares of both in the world).
pub(crate) const LOD_WORLD_CENTRE: usize = 0xE4;
pub(crate) const LOD_DATA: usize = 0xF0;
pub(crate) const LOD_DATA_SIZE: usize = 16;
pub(crate) const LOD_WORLD_NEAR_SQUARED: usize = 0x8;
pub(crate) const LOD_WORLD_FAR_SQUARED: usize = 0xC;
/// `NiCamera::fLODAdjust`, and the translation of the camera in its world transform.
pub(crate) const CAMERA_LOD_ADJUST: usize = 0x1DC;
pub(crate) const CAMERA_TRANSLATION: usize = AV::worldTransform + 0x24;
/// `NiBSPNode::worldPlane`.
const BSP_WORLD_PLANE: usize = 0xC0;
/// `NiSortAdjustNode`: the sorting mode and the accumulator of its own.
pub(crate) const SORT_ADJUST_MODE: usize = 0xB0;
pub(crate) const SORT_ADJUST_ACCUMULATOR: usize = 0xB4;
pub(crate) const SORTING_OFF: u32 = 1;
pub(crate) const SORTING_SUBSORT: u32 = 2;

const CULL_SHOW: usize = 0x6EB480;
pub(crate) const WHICH_SIDE: usize = 0x6F2660;
const NODE_DISPLAY: usize = 0x6C9190;
const SWITCH_NODE_DISPLAY: usize = 0x6D8320;
const PUSH_STATE: usize = 0x6F3F40;
const RENDERER_REGISTER_OBJECT: usize = 0x6F5070;
/// Engine functions that the ports call and that have no port: the child of a flt animation
/// node by its time, the turn of a billboard to the camera.
pub(crate) const SWITCH_CHILD_FROM_TIMING: usize = 0x6D8FF0;
pub(crate) const ROTATE_TO_CAMERA: usize = 0x6D3C00;

/// Takes one reference from a `NiObject`; at zero the object deletes itself.
unsafe fn release_ref(object: usize) {
    let count = rd::<i32>(object, 4) - 1;
    wr(object, 4, count);
    if count == 0 {
        vcall1(object, SLOT_DELETING_DTOR, 1);
    }
}

unsafe fn add_ref(object: usize) {
    wr(object, 4, rd::<i32>(object, 4) + 1);
}

// ---------------------------------------------------------------------------------------
// NiBound::WhichSide
// ---------------------------------------------------------------------------------------

port_thiscall!(which_side_port, 0x6F2660, "NiBound::WhichSide",
    fn(this, plane: usize) -> u32,
    compare_eax = true,
    regions = |set| { let _ = set; },
    rust = which_side);

/// The side of the plane that the bound is on: 2 behind the plane (culled), 0 across it, 1
/// in front of it. The engine keeps the signed distance on the x87 stack and compares it
/// there, so the port runs the same instructions.
pub unsafe fn which_side(this: usize, plane: usize) -> u32 {
    crate::shadow::touch(&which_side_port::STATE);
    let result: u32;
    asm!(
        "fld dword ptr [{p} + 8]",
        "fmul dword ptr [{b} + 8]",
        "fld dword ptr [{p} + 4]",
        "fmul dword ptr [{b} + 4]",
        "faddp st(1), st(0)",
        "fld dword ptr [{p}]",
        "fmul dword ptr [{b}]",
        "faddp st(1), st(0)",
        "fsub dword ptr [{p} + 12]",
        "fld dword ptr [{b} + 12]",
        "fchs",
        "fld st(1)",
        "fcompp",
        "fnstsw ax",
        "test ah, 0x41",
        "jp 2f",
        "fstp st(0)",
        "mov eax, 2",
        "jmp 3f",
        "2:",
        "fcomp dword ptr [{b} + 12]",
        "fnstsw ax",
        "and eax, 0x100",
        "jnz 4f",
        "mov eax, 1",
        "jmp 3f",
        "4:",
        "xor eax, eax",
        "3:",
        b = in(reg) this,
        p = in(reg) plane,
        out("eax") result,
        options(nostack),
    );
    result
}

// ---------------------------------------------------------------------------------------
// NiNode::IsVisualObject
// ---------------------------------------------------------------------------------------

port_thiscall!(is_visual_object_port, 0x445A60, "NiNode::IsVisualObject",
    fn(this) -> u32,
    compare_eax = true,
    regions = |set| { let _ = set; },
    rust = is_visual_object);

/// Bit 3 of the flags of the node.
pub unsafe fn is_visual_object(this: usize) -> u32 {
    crate::shadow::touch(&is_visual_object_port::STATE);
    ((rd::<u8>(this, AV::flags) as u32) >> NODE_HAS_GEOMETRY_BIT) & 1
}

// ---------------------------------------------------------------------------------------
// NiAVObject::CullShow
// ---------------------------------------------------------------------------------------

port_thiscall!(cull_show_port, 0x6EB480, "NiAVObject::CullShow",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| traversal_regions(set, this, camera),
    rust = cull_show);

/// Tests the world bound of the object against each culling plane of the camera that a
/// parent did not pass already (a bit for each plane in the camera), and displays the
/// object when no plane culls it. A plane that the bound is fully in front of is marked
/// for the subtree and unmarked after it.
pub unsafe fn cull_show(this: usize, camera: usize) {
    crate::shadow::touch(&cull_show_port::STATE);
    if crate::live_walk::node_served(this) {
        return;
    }
    if vcall0(this, SLOT_GET_APP_CULLED) & 0xFF != 0 {
        return;
    }
    let mut marked = [0u32; 4];
    let count = rd::<i32>(camera, C::countCullingPlanes);
    let mut shown = true;
    let mut index = count - 1;
    while index >= 0 {
        let bit = 1u32 << (index & 31);
        let word = (index >> 5) as usize;
        let at = C::usedCullingPlanesBitfield + word * 4;
        if rd::<u32>(camera, at) & bit == 0 {
            // The array of the camera holds the addresses of its planes.
            let plane = rd::<usize>(rd::<usize>(camera, C::cullingPlanes_storage), index as usize * 4);
            match thiscall1(WHICH_SIDE, this + AV::worldBound, plane as u32) {
                2 => {
                    shown = false;
                    break;
                }
                1 => {
                    wr(camera, at, rd::<u32>(camera, at) | bit);
                    marked[word] |= bit;
                }
                _ => {}
            }
        }
        index -= 1;
    }
    if shown {
        vcall1(this, SLOT_DISPLAY, camera as u32);
    }
    let count = rd::<u32>(camera, C::countCullingPlanes) as usize;
    for index in 0..count {
        let bit = 1u32 << (index & 31);
        if marked[index >> 5] & bit != 0 {
            let at = C::usedCullingPlanesBitfield + (index >> 5) * 4;
            wr(camera, at, rd::<u32>(camera, at) & !bit);
        }
    }
}

// ---------------------------------------------------------------------------------------
// NiNode::Display and the kinds of node
// ---------------------------------------------------------------------------------------

port_thiscall!(node_display_port, 0x6C9190, "NiNode::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| traversal_regions(set, this, camera),
    rust = node_display);

/// Marks the node as displayed, and culls and displays each child when the node contains
/// renderable geometry. The count of the children is read again for each child, as the
/// engine does.
pub unsafe fn node_display(this: usize, camera: usize) {
    crate::shadow::touch(&node_display_port::STATE);
    wr::<u16>(this, AV::flags, rd::<u16>(this, AV::flags) | NODE_DISPLAYED);
    if vcall0(this, SLOT_IS_VISUAL_OBJECT) & 0xFF != 0 {
        let mut i = 0usize;
        while i < rd::<u32>(this, N::kChildren_endIndex) as usize {
            let child = rd::<usize>(rd::<usize>(this, N::kChildren_storage), i * 4);
            if child != 0 {
                thiscall1(CULL_SHOW, child, camera as u32);
                // The sky-only walk (F8 part 3): after the sky root the table walk and the
                // live walk serve the world, and the rest of the world root is not walked.
                if crate::live_walk::after_child(this, child, camera) {
                    break;
                }
            }
            i += 1;
        }
    }
    crate::live_walk::after_node(this);
}

port_thiscall!(bs_animation_node_display_port, 0x6F21C0, "BSAnimationNode::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| traversal_regions(set, this, camera),
    rust = bs_animation_node_display);

/// Marks the animation node as displayed in its own flags (its controllers run only then),
/// then displays it as a node.
pub unsafe fn bs_animation_node_display(this: usize, camera: usize) {
    crate::shadow::touch(&bs_animation_node_display_port::STATE);
    wr::<u8>(this, ANIM_FLAGS, rd::<u8>(this, ANIM_FLAGS) | ANIM_DISPLAYED);
    thiscall1(NODE_DISPLAY, this, camera as u32);
}

port_thiscall!(billboard_node_display_port, 0x6D4040, "NiBillboardNode::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| traversal_regions(set, this, camera),
    rust = billboard_node_display);

/// Turns the node to the camera (engine code), then displays it as a node.
pub unsafe fn billboard_node_display(this: usize, camera: usize) {
    crate::shadow::touch(&billboard_node_display_port::STATE);
    thiscall1(ROTATE_TO_CAMERA, this, camera as u32);
    thiscall1(NODE_DISPLAY, this, camera as u32);
}

port_thiscall!(switch_node_display_port, 0x6D8320, "NiSwitchNode::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| traversal_regions(set, this, camera),
    rust = switch_node_display);

/// Culls and displays the child that the switch selects. A child whose revision is not the
/// node's gets an update first (its subtree was not updated while another child was shown),
/// and the node keeps the revision of each child in an array that grows with the index.
pub unsafe fn switch_node_display(this: usize, camera: usize) {
    crate::shadow::touch(&switch_node_display_port::STATE);
    let index = rd::<i32>(this, SWITCH_INDEX);
    if index < 0 {
        return;
    }
    let child = rd::<usize>(rd::<usize>(this, N::kChildren_storage), index as usize * 4);
    if child == 0 {
        return;
    }
    switch_prepare_child(this, index, child);
    thiscall1(CULL_SHOW, child, camera as u32);
}

/// The lazy update of the child a switch node selects (`NiSwitchNode::Display`): a child
/// whose revision is not the node's gets an update with the node's saved time, and the
/// node keeps the child's revision. The live walk calls it too (plan 8b, R26).
pub(crate) unsafe fn switch_prepare_child(this: usize, index: i32, child: usize) {
    let revision = rd::<u32>(this, SWITCH_UPDATE_REVISION);
    let storage = rd::<usize>(this, SWITCH_REVISIONS_STORAGE);
    let at = index as usize * 4;
    if rd::<u32>(storage, at) != revision {
        if index < rd::<i32>(this, SWITCH_REVISIONS_STORAGE_COUNT) {
            if index < rd::<i32>(this, SWITCH_REVISIONS_END) {
                let old = rd::<u32>(storage, at);
                if revision != 0 {
                    if old == 0 {
                        wr(this, SWITCH_REVISIONS_FILLED, rd::<i32>(this, SWITCH_REVISIONS_FILLED) + 1);
                    }
                } else if old != 0 {
                    wr(this, SWITCH_REVISIONS_FILLED, rd::<i32>(this, SWITCH_REVISIONS_FILLED) - 1);
                }
            } else {
                wr(this, SWITCH_REVISIONS_END, index + 1);
                if revision != 0 {
                    wr(this, SWITCH_REVISIONS_FILLED, rd::<i32>(this, SWITCH_REVISIONS_FILLED) + 1);
                }
            }
            wr(storage, at, revision);
        }
        let time = rd::<u32>(this, SWITCH_SAVED_TIME);
        let controllers = rd::<u8>(this, SWITCH_UPDATE_CONTROLLERS) as u32;
        vcall3(child, SLOT_UPDATE_DOWNWARD_PASS, time, controllers, 1);
    }
}

port_thiscall!(flt_animation_node_display_port, 0x6D90B0, "NiFltAnimationNode::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| traversal_regions(set, this, camera),
    rust = flt_animation_node_display);

/// Picks the child by the saved time (engine code), then displays the node as a switch node.
pub unsafe fn flt_animation_node_display(this: usize, camera: usize) {
    crate::shadow::touch(&flt_animation_node_display_port::STATE);
    thiscall1(SWITCH_CHILD_FROM_TIMING, this, rd::<u32>(this, SWITCH_SAVED_TIME));
    thiscall1(SWITCH_NODE_DISPLAY, this, camera as u32);
}

port_thiscall!(lod_node_display_port, 0x7412B0, "NiLODNode::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| traversal_regions(set, this, camera),
    rust = lod_node_display);

/// Picks the level of detail by the square of the distance of the node's centre from the
/// camera, scaled by the camera's adjustment: the last level whose range holds it, from
/// the last level down; then displays the node as a switch node.
pub unsafe fn lod_node_display(this: usize, camera: usize) {
    crate::shadow::touch(&lod_node_display_port::STATE);
    let adjust = rd::<f32>(camera, CAMERA_LOD_ADJUST);
    let dx = rd::<f32>(this, LOD_WORLD_CENTRE) - rd::<f32>(camera, CAMERA_TRANSLATION);
    let dy = rd::<f32>(this, LOD_WORLD_CENTRE + 4) - rd::<f32>(camera, CAMERA_TRANSLATION + 4);
    let dz = rd::<f32>(this, LOD_WORLD_CENTRE + 8) - rd::<f32>(camera, CAMERA_TRANSLATION + 8);
    let levels = rd::<usize>(this, LOD_DATA);
    let mut index = rd::<i32>(this, N::kChildren_endIndex) - 1;
    wr(this, SWITCH_INDEX, index);
    let distance = ((dz * dz + dy * dy) + dx * dx) * (adjust * adjust);
    while index >= 0 {
        let level = levels + index as usize * LOD_DATA_SIZE;
        if rd::<f32>(level, LOD_WORLD_NEAR_SQUARED) <= distance && distance < rd::<f32>(level, LOD_WORLD_FAR_SQUARED) {
            break;
        }
        index -= 1;
        wr(this, SWITCH_INDEX, index);
    }
    thiscall1(SWITCH_NODE_DISPLAY, this, camera as u32);
}

port_thiscall!(bsp_node_display_port, 0x6D6470, "NiBSPNode::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| traversal_regions(set, this, camera),
    rust = bsp_node_display);

/// The side of the node's plane that the camera is on decides the order of the two children:
/// the far one first. On the plane, the direction of the camera decides.
pub unsafe fn bsp_node_display(this: usize, camera: usize) {
    crate::shadow::touch(&bsp_node_display_port::STATE);
    let plane = |i: usize| rd::<f32>(this, BSP_WORLD_PLANE + i * 4);
    let (nx, ny, nz, constant) = (plane(0), plane(1), plane(2), plane(3));
    let side_of = |x: f32, y: f32, z: f32| -> Option<u32> {
        let distance = (z * nz + y * ny) + x * nx - constant;
        if distance < 0.0 {
            Some(2)
        } else if distance > 0.0 {
            Some(1)
        } else {
            None
        }
    };
    let translation = |i: usize| rd::<f32>(camera, CAMERA_TRANSLATION + i * 4);
    let direction = |i: usize| rd::<f32>(camera, C::kWorldDir + i * 4);
    let side = match side_of(translation(0), translation(1), translation(2)) {
        Some(side) => side,
        None => side_of(direction(0), direction(1), direction(2)).unwrap_or(0),
    };
    let storage = rd::<usize>(this, N::kChildren_storage);
    let (left, right) = (rd::<usize>(storage, 0), rd::<usize>(storage, 4));
    let order = if side == 2 { [right, left] } else { [left, right] };
    for child in order {
        if child != 0 {
            thiscall1(CULL_SHOW, child, camera as u32);
        }
    }
}

port_thiscall!(sort_adjust_node_display_port, 0x6DE030, "NiSortAdjustNode::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| traversal_regions(set, this, camera),
    rust = sort_adjust_node_display);

/// Displays the node with the renderer's accumulator swapped for the node's own (sub-sort)
/// or for none (sorting off), and puts the renderer's accumulator back after.
pub unsafe fn sort_adjust_node_display(this: usize, camera: usize) {
    crate::shadow::touch(&sort_adjust_node_display_port::STATE);
    let renderer = rd::<usize>(camera, C::spRenderer);
    let pointer = renderer + R::accumulator;
    let before = rd::<usize>(pointer, 0);
    if before != 0 {
        add_ref(before);
    }
    let mode = rd::<u32>(this, SORT_ADJUST_MODE);
    if mode == SORTING_OFF {
        crate::engine::ni_pointer_set(pointer, 0);
        thiscall1(NODE_DISPLAY, this, camera as u32);
    } else if mode != SORTING_SUBSORT {
        thiscall1(NODE_DISPLAY, this, camera as u32);
    } else {
        let own = rd::<usize>(this, SORT_ADJUST_ACCUMULATOR);
        let current = rd::<usize>(pointer, 0);
        if current != own {
            if current != 0 {
                crate::engine::ni_object_release(current);
            }
            wr(pointer, 0, own);
            if own != 0 {
                add_ref(own);
            }
        }
        let own = rd::<usize>(this, SORT_ADJUST_ACCUMULATOR);
        if own != 0 && vcall1(own, SLOT_START_ACCUMULATING, camera as u32) & 0xFF != 0 {
            thiscall1(NODE_DISPLAY, this, camera as u32);
            vcall0(rd::<usize>(this, SORT_ADJUST_ACCUMULATOR), SLOT_FINISH_ACCUMULATING);
        } else {
            let current = rd::<usize>(pointer, 0);
            if current != 0 {
                release_ref(current);
                wr(pointer, 0, 0);
            }
            thiscall1(NODE_DISPLAY, this, camera as u32);
        }
    }
    let renderer = rd::<usize>(camera, C::spRenderer);
    let pointer = renderer + R::accumulator;
    let after = rd::<usize>(pointer, 0);
    if after != before {
        if after != 0 {
            release_ref(after);
        }
        wr(pointer, 0, before);
        if before != 0 {
            add_ref(before);
        }
    }
    if before != 0 {
        release_ref(before);
    }
}

// ---------------------------------------------------------------------------------------
// The shapes
// ---------------------------------------------------------------------------------------

port_thiscall!(push_state_port, 0x6F3F40, "NiGeometry::pushStateToRenderer",
    fn(this, renderer: usize) -> (),
    compare_eax = false,
    regions = |set| push_state_regions(set, this, renderer),
    rust = push_state_to_renderer);

/// Makes the shape's property state and effect state the renderer's current ones, with
/// the references counted. A shape without a property state leaves the renderer's.
pub unsafe fn push_state_to_renderer(this: usize, renderer: usize) {
    crate::shadow::touch(&push_state_port::STATE);
    let property = rd::<usize>(this, G::spPropertyState);
    if property != 0 {
        let current = rd::<usize>(renderer, R::currentPropertyState);
        if current != property {
            if current != 0 {
                release_ref(current);
            }
            wr(renderer, R::currentPropertyState, property);
            add_ref(property);
        }
    }
    let effect = rd::<usize>(this, G::spEffectState);
    let current = rd::<usize>(renderer, R::currentEffectState);
    if current != effect {
        if current != 0 {
            release_ref(current);
        }
        wr(renderer, R::currentEffectState, effect);
        if effect != 0 {
            add_ref(effect);
        }
    }
}

unsafe fn push_state_regions(set: &mut Regions, this: usize, renderer: usize) {
    set.add(renderer + R::currentPropertyState, 8);
    for state in [
        rd::<usize>(renderer, R::currentPropertyState),
        rd::<usize>(renderer, R::currentEffectState),
        rd::<usize>(this, G::spPropertyState),
        rd::<usize>(this, G::spEffectState),
    ] {
        if state != 0 {
            set.add(state + 4, 4);
        }
    }
    // A state that the renderer alone held is deleted.
    set.allocates();
}

/// The start of the display of a shape: the states to the renderer, the accumulator, and
/// the controllers' `OnPreDisplay`. Returns the renderer when the shape draws now, or
/// `None` when the accumulator took it.
unsafe fn shape_display_start(this: usize, camera: usize) -> Option<usize> {
    let renderer = rd::<usize>(camera, C::spRenderer);
    thiscall1(PUSH_STATE, this, renderer as u32);
    if thiscall1(RENDERER_REGISTER_OBJECT, renderer, this as u32) & 0xFF != 0 {
        return None;
    }
    let mut controller = rd::<usize>(this, AV::controllers);
    while controller != 0 {
        vcall0(controller, SLOT_ON_PRE_DISPLAY);
        controller = rd::<usize>(controller, TC::nextController);
    }
    Some(renderer)
}

unsafe fn count_render_call(primitives: u32, vertices: u32) {
    wr(COUNT_RENDER_CALLS, 0, rd::<u32>(COUNT_RENDER_CALLS, 0) + 1);
    wr(COUNT_PRIMS_RENDERED, 0, rd::<u32>(COUNT_PRIMS_RENDERED, 0) + primitives);
    wr(COUNT_VERTS_RENDERED, 0, rd::<u32>(COUNT_VERTS_RENDERED, 0) + vertices);
}

port_thiscall!(tri_shape_display_port, 0x6E4F90, "NiTriShape::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = tri_shape_display);

/// The states, the accumulator, the controllers, then `RenderShape` of the renderer with
/// the shape's data, skin, world transform and world bound; and the counts of the frame.
pub unsafe fn tri_shape_display(this: usize, camera: usize) {
    crate::shadow::touch(&tri_shape_display_port::STATE);
    // The live walk displayed the shape in this run of the world root (plan F8, part 2).
    if crate::live_walk::served(this) {
        return;
    }
    // A shape the helper draws from its table (plan 0.5 entry T5): the plugin notes its
    // changes, and the engine does not display it.
    if rd::<u16>(this, AV::flags) & crate::instance_stream::RETAINED_BIT != 0 && crate::instance_stream::skipped_display(this, crate::link::DRAW_SHAPE) {
        return;
    }
    if crate::link_client::water_plane_display_skipped(this) {
        return;
    }
    let Some(renderer) = shape_display_start(this, camera) else { return };
    let skin = rd::<usize>(this, G::skinInstance);
    let data = rd::<usize>(this, G::spModelData);
    let bound = vcall0(this, SLOT_GET_WORLD_BOUND) as usize;
    thiscall4(virtual_at(renderer, SLOT_RENDER_SHAPE), renderer, data as u32, skin as u32, (this + AV::worldTransform) as u32, bound as u32);
    let data = rd::<usize>(this, G::spModelData);
    count_render_call(vcall0(data, SLOT_GET_ACTIVE_TRIANGLE_COUNT), vcall0(data, SLOT_GET_ACTIVE_VERTEX_COUNT));
    // After the draw the table knows the shape: a retained instance is the table's from its
    // next display; every other shape is the engine's (plan F8, part 2).
    if crate::instance_stream::engine_serves(this) {
        crate::link_client::note_world_shape(this, crate::live_walk::Kind::Shape);
    }
}

port_thiscall!(tri_strips_display_port, 0x742660, "NiTriStrips::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = tri_strips_display);

/// As `NiTriShape::Display`, with `RenderTristrips`; the counts take the triangle count and
/// the vertex count of the data as they are.
pub unsafe fn tri_strips_display(this: usize, camera: usize) {
    crate::shadow::touch(&tri_strips_display_port::STATE);
    if crate::live_walk::served(this) {
        return;
    }
    if rd::<u16>(this, AV::flags) & crate::instance_stream::RETAINED_BIT != 0 && crate::instance_stream::skipped_display(this, crate::link::DRAW_STRIPS) {
        return;
    }
    if crate::link_client::water_plane_display_skipped(this) {
        return;
    }
    let Some(renderer) = shape_display_start(this, camera) else { return };
    let skin = rd::<usize>(this, G::skinInstance);
    let data = rd::<usize>(this, G::spModelData);
    let bound = vcall0(this, SLOT_GET_WORLD_BOUND) as usize;
    thiscall4(virtual_at(renderer, SLOT_RENDER_TRISTRIPS), renderer, data as u32, skin as u32, (this + AV::worldTransform) as u32, bound as u32);
    let data = rd::<usize>(this, G::spModelData);
    count_render_call(rd::<u16>(data, GD::SIZE) as u32, rd::<u16>(data, GD::vertexCount) as u32);
    if crate::instance_stream::engine_serves(this) {
        crate::link_client::note_world_shape(this, crate::live_walk::Kind::Strips);
    }
}

/// The number of active particles of a `NiParticlesData`, at the end of its geometry data.
const PARTICLES_NUM_ACTIVE: usize = 0x3C;

port_thiscall!(particles_display_port, 0x6DD3E0, "NiParticles::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = particles_display);

/// As `NiTriShape::Display`, with `RenderPoints` (no skin); the counts take the active
/// particles for both.
pub unsafe fn particles_display(this: usize, camera: usize) {
    crate::shadow::touch(&particles_display_port::STATE);
    if crate::live_walk::served(this) {
        return;
    }
    crate::link_client::note_world_shape(this, crate::live_walk::Kind::Particles);
    let Some(renderer) = shape_display_start(this, camera) else { return };
    render_points(this, renderer);
}

unsafe fn render_points(this: usize, renderer: usize) {
    let data = rd::<usize>(this, G::spModelData);
    let bound = vcall0(this, SLOT_GET_WORLD_BOUND) as usize;
    thiscall3(virtual_at(renderer, SLOT_RENDER_POINTS), renderer, data as u32, (this + AV::worldTransform) as u32, bound as u32);
    let active = rd::<u16>(rd::<usize>(this, G::spModelData), PARTICLES_NUM_ACTIVE) as u32;
    count_render_call(active, active);
}

port_thiscall!(auto_normal_particles_display_port, 0x6D3540, "NiAutoNormalParticles::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = auto_normal_particles_display);

/// As `NiParticles::Display`, with the normal of each active particle set to the direction
/// towards the camera first.
pub unsafe fn auto_normal_particles_display(this: usize, camera: usize) {
    crate::shadow::touch(&auto_normal_particles_display_port::STATE);
    if crate::live_walk::served(this) {
        return;
    }
    crate::link_client::note_world_shape(this, crate::live_walk::Kind::Particles);
    let Some(renderer) = shape_display_start(this, camera) else { return };
    let normal = [-rd::<f32>(camera, C::kWorldDir), -rd::<f32>(camera, C::kWorldDir + 4), -rd::<f32>(camera, C::kWorldDir + 8)];
    let data = rd::<usize>(this, G::spModelData);
    let normals = rd::<usize>(data, GD::normals);
    if normals != 0 {
        let active = rd::<u16>(data, PARTICLES_NUM_ACTIVE) as usize;
        for i in 0..active {
            for (k, value) in normal.iter().enumerate() {
                wr(normals, i * 12 + k * 4, *value);
            }
        }
    }
    render_points(this, renderer);
}

port_thiscall!(lines_display_port, 0x6DA850, "NiLines::Display",
    fn(this, camera: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = lines_display);

/// As `NiTriShape::Display`, with `RenderLines` (no skin) and no counts.
pub unsafe fn lines_display(this: usize, camera: usize) {
    crate::shadow::touch(&lines_display_port::STATE);
    if crate::live_walk::served(this) {
        return;
    }
    crate::link_client::note_world_shape(this, crate::live_walk::Kind::Lines);
    let Some(renderer) = shape_display_start(this, camera) else { return };
    let data = rd::<usize>(this, G::spModelData);
    let bound = vcall0(this, SLOT_GET_WORLD_BOUND) as usize;
    thiscall3(virtual_at(renderer, SLOT_RENDER_LINES), renderer, data as u32, (this + AV::worldTransform) as u32, bound as u32);
}

// ---------------------------------------------------------------------------------------
// The end of a geometry
// ---------------------------------------------------------------------------------------

port_thiscall!(geometry_dtor_port, 0x6F3DB0, "NiGeometry::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = geometry_dtor);

/// The destructor of a geometry: the plugin forgets the shape (its retained instance of
/// `instance_stream`), then the engine code runs.
pub unsafe fn geometry_dtor(this: usize) {
    crate::shadow::touch(&geometry_dtor_port::STATE);
    crate::instance_stream::forget(this);
    crate::live_walk::forget(this);
    geometry_dtor_port::engine(this);
}

// ---------------------------------------------------------------------------------------
// The memory that a traversal writes
// ---------------------------------------------------------------------------------------

/// The memory that the traversal of a subtree can write: the bits of the planes in the
/// camera and in the accumulator's copy, the current states and the accumulator of the
/// renderer with the counts of the states, the list of the accumulator (its nodes live in
/// hunks), the flags of each node of the subtree, and the counts of the states of each shape.
/// A traversal that draws cannot be compared (the renderer ports skip a call that draws):
/// this list serves a subtree that draws nothing.
unsafe fn traversal_regions(set: &mut Regions, root: usize, camera: usize) {
    set.add(camera + C::usedCullingPlanesBitfield, 16);
    set.add(ACCUMULATOR_PLANES, 16);
    set.add(COUNT_RENDER_CALLS, 12);
    let renderer = rd::<usize>(camera, C::spRenderer);
    if renderer != 0 {
        set.add(renderer + R::accumulator, 12);
        for state in [rd::<usize>(renderer, R::currentPropertyState), rd::<usize>(renderer, R::currentEffectState)] {
            if state != 0 {
                set.add(state + 4, 4);
            }
        }
        let accumulator = rd::<usize>(renderer, R::accumulator);
        if accumulator != 0 {
            let list = rd::<usize>(accumulator, CA::sortedObjects);
            if list != 0 {
                set.add(list, L::SIZE);
                let grow = rd::<u32>(list, L::growByCount) as usize;
                let mut hunk = rd::<usize>(list, L::hunkFreeList);
                while hunk != 0 {
                    set.add(rd::<usize>(hunk, H::allocation), grow * 12);
                    hunk = rd::<usize>(hunk, H::nextHunk);
                }
            }
        }
    }
    subtree_regions(set, root);
    // A new hunk of the list, or a grown array of a switch node.
    set.allocates();
}

unsafe fn subtree_regions(set: &mut Regions, object: usize) {
    set.add(object + AV::flags, 2);
    if ni::is_kind_of(object, RTTI_NI_GEOMETRY) {
        for state in [rd::<usize>(object, G::spPropertyState), rd::<usize>(object, G::spEffectState)] {
            if state != 0 {
                set.add(state + 4, 4);
            }
        }
    } else if ni::is_kind_of(object, RTTI_NI_NODE) {
        let (storage, end) = (rd::<usize>(object, N::kChildren_storage), rd::<u32>(object, N::kChildren_endIndex));
        for i in 0..end as usize {
            let child = rd::<usize>(storage, i * 4);
            if child != 0 {
                subtree_regions(set, child);
            }
        }
    }
}

pub static SITES: &[usize] = &[];

pub static PORTS: &[&crate::shadow::PortState] = &[
    &which_side_port::STATE,
    &is_visual_object_port::STATE,
    &cull_show_port::STATE,
    &node_display_port::STATE,
    &bs_animation_node_display_port::STATE,
    &billboard_node_display_port::STATE,
    &switch_node_display_port::STATE,
    &flt_animation_node_display_port::STATE,
    &lod_node_display_port::STATE,
    &bsp_node_display_port::STATE,
    &sort_adjust_node_display_port::STATE,
    &push_state_port::STATE,
    &tri_shape_display_port::STATE,
    &tri_strips_display_port::STATE,
    &particles_display_port::STATE,
    &auto_normal_particles_display_port::STATE,
    &lines_display_port::STATE,
    &geometry_dtor_port::STATE,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook;

    #[test]
    fn the_functions_are_in_the_table() {
        for port in PORTS {
            assert!(hook::find(port.addr).is_some(), "{:#x} {}", port.addr, port.name);
        }
    }

    #[test]
    fn which_side_tells_the_three_sides() {
        let bound = [0.0f32, 0.0, 0.0, 1.0];
        let behind = [0.0f32, 0.0, 1.0, 2.0];
        let across = [0.0f32, 0.0, 1.0, 0.0];
        let in_front = [0.0f32, 0.0, 1.0, -5.0];
        let at_the_edge = [0.0f32, 0.0, 1.0, 1.0];
        unsafe {
            let b = bound.as_ptr() as usize;
            assert_eq!(which_side(b, behind.as_ptr() as usize), 2);
            assert_eq!(which_side(b, across.as_ptr() as usize), 0);
            assert_eq!(which_side(b, in_front.as_ptr() as usize), 1);
            // The distance is -1 and the radius 1: behind, as `dist <= -radius`.
            assert_eq!(which_side(b, at_the_edge.as_ptr() as usize), 2);
        }
    }
}
