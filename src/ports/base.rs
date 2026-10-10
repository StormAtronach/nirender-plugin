//! This module ports the `NiRenderer` base class and `NiRendererSpecificProperty`. It ports the
//! construction and destruction of `NiDX8RenderState` and `NiDX8MaterialManager`. It also ports
//! the deleting destructors and jump stubs of the classes that the other modules port.

use core::arch::asm;

use crate::d3d;
use crate::engine::{vcall0, vcall1, NI_REF_OBJECT_COUNT, VTBL_NI_ALPHA_PROPERTY, VTBL_NI_REF_OBJECT};
use crate::gen::layouts::{
    NiDX8ConfigurableTexturePipeline as CTP, NiDX8MaterialManager as MM, NiDX8RenderState as RS,
    NiDX8TexturePass as TP, NiDX8TextureStage as TS, NiMaterialProperty as MP, NiRenderer as R,
    NiRendererSpecificProperty as RSP, NiRendererSpecificProperty_RenderData7 as RD7, THashMap as HM,
    THashMap_Node as HN, D3DCAPS8,
};
use crate::mem::{rd, wr};
use crate::ports::ni::{self, REF_COUNT};
use crate::ports::{hash_map, light_manager, material, render_state, texture_pass_a, texture_pass_b, texture_pipeline};
use crate::shadow::Regions;

// ---------------------------------------------------------------------------------------------
// Engine data at fixed addresses
// ---------------------------------------------------------------------------------------------

const VTBL_NI_RENDERER: usize = 0x750F58;
const VTBL_RENDERER_SPECIFIC_PROPERTY: usize = 0x750410;
/// The vtable that `CreateFromStream` gives to the 12-byte link record that it makes.
const VTBL_STREAM_LINK_DATA: usize = 0x750444;
const VTBL_RENDER_DATA_7: usize = 0x74B6E0;
const VTBL_RENDER_DATA_BASE: usize = 0x74B6E8;
const VTBL_RENDER_STATE: usize = 0x74F6CC;
const VTBL_MATERIAL_MANAGER: usize = 0x74F82C;
/// The vtable of the hash map inside the material manager.
const VTBL_MATERIAL_MAP: usize = 0x74F830;

/// `data_NiRTTI_NiRenderer`. The first member is the name.
const RTTI_NI_RENDERER: usize = 0x7DEA9C;
/// `data_NiRTTI_NiRendererSpecificProperty`
const RTTI_RENDERER_SPECIFIC_PROPERTY: usize = 0x7DE2F0;

/// The list of every renderer: count, head, tail, free nodes, node blocks, nodes per block.
const RENDERER_LIST: usize = 0x7DEA54;
const RENDERER_LIST_COUNT: usize = 0x7DEA54;
const RENDERER_LIST_HEAD: usize = 0x7DEA58;
const RENDERER_LIST_TAIL: usize = 0x7DEA5C;
const RENDERER_LIST_FREE: usize = 0x7DEA60;
const RENDERER_LIST_BLOCKS: usize = 0x7DEA64;
const RENDERER_LIST_BLOCK_NODES: usize = 0x7DEA68;
/// The constructor and the destructor write the count, head, tail, free nodes and node blocks.
const RENDERER_LIST_SIZE: usize = 0x14;
/// A node of the list: renderer, next, previous.
const NODE_SIZE: usize = 12;
const NODE_RENDERER: usize = 0;
const NODE_NEXT: usize = 4;
const NODE_PREVIOUS: usize = 8;

/// The default value of each of the 256 render states.
const RENDER_STATE_DEFAULTS: usize = 0x7C3858;
/// The default of D3DRS_ZENABLE (7) in that table.
const DEFAULT_ZENABLE: usize = 0x7C3874;
/// The default of D3DRS_SOFTWAREVERTEXPROCESSING (153) in that table.
const DEFAULT_SOFTWARE_VERTEX_PROCESSING: usize = 0x7C3ABC;
/// The text that the render state gives to the debug output when the device has W fog. The
/// debug output is an empty function.
const W_FOG_NOTE: usize = 0x7C3CD8;
/// The material that the material manager selects when it starts.
const DEFAULT_MATERIAL: usize = 0x7DE29C;
/// 1.0f
const F_ONE: usize = 0x74BFDC;

// Vtable slots.
const SLOT_LOAD_BINARY: usize = 0x0C;
const SLOT_ACCUMULATOR_REGISTER_OBJECT: usize = 0x34;
const SLOT_GET_BACKBUFFER_WIDTH: usize = 0x54;
const SLOT_GET_BACKBUFFER_HEIGHT: usize = 0x58;
const SLOT_GET_PROPERTY_STATE: usize = 0x70;
const SLOT_GET_EFFECT_STATE: usize = 0x74;
const SLOT_DISPLAY: usize = 0x78;
const SLOT_PURGE_EFFECT: usize = 0xB4;
const SLOT_PURGE_GEOMETRY_DATA: usize = 0xE8;
const SLOT_PURGE_MATERIAL: usize = 0xEC;
const SLOT_PURGE_SKIN_PARTITION: usize = 0xF0;

// `TArray`
const ARRAY_SIZE: usize = 0x18;
const ARRAY_STORAGE: usize = 0x04;
const ARRAY_STORAGE_COUNT: usize = 0x08;
const ARRAY_END_INDEX: usize = 0x0C;
const ARRAY_FILLED_COUNT: usize = 0x10;
const ARRAY_GROW_BY: usize = 0x14;

// `NiStream`
const STREAM_LINK_DATA: usize = 0x78;
const STREAM_LINK_ARRAY: usize = 0x7C;

// `NiObjectNET`
const NET_NAME: usize = 0x08;
const NET_EXTRA_DATA: usize = 0x0C;
const NET_CONTROLLERS: usize = 0x10;

// `NiAlphaProperty` as the render state constructor fills it.
const ALPHA_PROPERTY_SIZE: usize = 0x1C;
const ALPHA_PROPERTY_FLAGS: usize = 0x14;
const ALPHA_PROPERTY_TEST_REF: usize = 0x18;

/// `NiDX8ConfigurableTexturePipeline`: the texture pass that the shader functions go to.
const PIPELINE_TEXTURE_PASS: usize = CTP::field_8;

// ---------------------------------------------------------------------------------------------
// Engine functions at fixed addresses
// ---------------------------------------------------------------------------------------------

/// The game functions that this module calls. The functions that `engine` has come from there.
#[cfg(not(test))]
mod eng {
    use crate::engine::{cdecl1, thiscall0, thiscall1};

    pub use crate::engine::{
        debug_print0 as debug_output, ni_object_net_ctor, ni_object_release, ni_pointer_operator_assign, ni_pointer_set,
        operator_delete, operator_new,
    };

    /// 0x6E98A0 `NiObject::ctor`
    pub unsafe fn ni_object_ctor(this: usize) {
        thiscall0(0x6E98A0, this);
    }
    /// 0x6E98F0 `NiObject::dtor`
    pub unsafe fn ni_object_dtor(this: usize) {
        thiscall0(0x6E98F0, this);
    }
    /// 0x6E9F80 `NiObjectNET::dtor`
    pub unsafe fn ni_object_net_dtor(this: usize) {
        thiscall0(0x6E9F80, this);
    }
    /// 0x6F5580. It removes the node at `*node` from the renderer list and puts it with the free nodes.
    pub unsafe fn renderer_list_remove(list: usize, node: *mut usize) {
        thiscall1(0x6F5580, list, node as u32);
    }
    /// 0x6E9C60 `NiObject::AddViewerStrings`
    pub unsafe fn ni_object_add_viewer_strings(this: usize, strings: usize) {
        thiscall1(0x6E9C60, this, strings as u32);
    }
    /// 0x6E96A0 `NiProperty::AddViewerStrings`
    pub unsafe fn ni_property_add_viewer_strings(this: usize, strings: usize) {
        thiscall1(0x6E96A0, this, strings as u32);
    }
    /// 0x6FCFD0 `NiViewer::createSectionText`. It is `__cdecl`. It makes a new string "--- name ---".
    pub unsafe fn viewer_section_text(name: usize) -> usize {
        cdecl1(0x6FCFD0, name as u32) as usize
    }
    /// 0x6E95F0 `NiProperty::CopyMembers`
    pub unsafe fn ni_property_copy_members(this: usize, to: usize) {
        thiscall1(0x6E95F0, this, to as u32);
    }
    /// 0x6E9650 `NiProperty::RegisterStreamables`. The result is in AL.
    pub unsafe fn ni_property_register_streamables(this: usize, stream: usize) -> u32 {
        thiscall1(0x6E9650, this, stream as u32)
    }
    /// 0x6E9690 `NiProperty::IsEqual`. The result is in AL.
    pub unsafe fn ni_property_is_equal(this: usize, other: usize) -> u32 {
        thiscall1(0x6E9690, this, other as u32)
    }
}

/// Replacements for the unit tests. The unit tests run without the game.
#[cfg(test)]
mod eng {
    pub unsafe fn operator_new(size: usize) -> usize {
        Box::leak(vec![0xCDu8; size].into_boxed_slice()).as_mut_ptr() as usize
    }
    pub unsafe fn operator_delete(_block: usize) {}
    pub unsafe fn ni_object_ctor(_this: usize) {}
    pub unsafe fn ni_object_dtor(_this: usize) {}
    pub unsafe fn ni_object_net_ctor(_this: usize) {}
    pub unsafe fn ni_object_net_dtor(_this: usize) {}
    pub unsafe fn ni_object_release(this: usize) {
        crate::ports::ni::release(this)
    }
    pub unsafe fn ni_pointer_set(slot: usize, object: usize) {
        crate::ports::ni::pointer_assign(slot, object)
    }
    pub unsafe fn ni_pointer_operator_assign(slot: usize, object: usize) {
        crate::ports::ni::pointer_assign(slot, object)
    }
    pub unsafe fn renderer_list_remove(_list: usize, _node: *mut usize) {}
    pub unsafe fn ni_object_add_viewer_strings(_this: usize, _strings: usize) {}
    pub unsafe fn ni_property_add_viewer_strings(_this: usize, _strings: usize) {}
    pub unsafe fn viewer_section_text(name: usize) -> usize {
        name
    }
    pub unsafe fn ni_property_copy_members(_this: usize, _to: usize) {}
    pub unsafe fn ni_property_register_streamables(_this: usize, stream: usize) -> u32 {
        stream as u32
    }
    pub unsafe fn ni_property_is_equal(_this: usize, other: usize) -> u32 {
        other as u32
    }
    pub unsafe fn debug_output(_text: usize) {}
}

// ---------------------------------------------------------------------------------------------
// Deleting destructors, reference counts and arrays
// ---------------------------------------------------------------------------------------------

/// The body of every deleting destructor. It calls a destructor. Then it calls `operator delete`
/// when bit 0 of `flags` is set. It returns `this`, as the game code does.
unsafe fn deleting(this: usize, flags: u8, dtor: unsafe fn(usize)) -> usize {
    dtor(this);
    if flags & 1 != 0 {
        eng::operator_delete(this);
    }
    this
}

/// The regions of a release of `object` by the inline code. The function returns false when
/// the release deletes the object. The caller makes the call a skip then.
unsafe fn release_regions(set: &mut Regions, object: usize) -> bool {
    if object == 0 {
        return true;
    }
    if rd::<u32>(object, REF_COUNT) == 1 {
        return false;
    }
    set.add(object + REF_COUNT, 4);
    true
}

/// `TArray<void*>::Add` as the game code has it inline. The element goes to `endIndex`. The
/// storage grows by `growByCount` when it is full.
unsafe fn array_add(array: usize, element: usize) {
    let index: u32 = rd(array, ARRAY_END_INDEX);
    let capacity: u32 = rd(array, ARRAY_STORAGE_COUNT);
    if index >= capacity {
        let size = rd::<u32>(array, ARRAY_GROW_BY).wrapping_add(index);
        if size != capacity {
            if size < index {
                let mut at = size;
                loop {
                    let storage: usize = rd(array, ARRAY_STORAGE);
                    let slot = storage.wrapping_add((at as usize).wrapping_mul(4));
                    if rd::<usize>(slot, 0) != 0 {
                        wr::<usize>(slot, 0, 0);
                        wr(array, ARRAY_FILLED_COUNT, rd::<u32>(array, ARRAY_FILLED_COUNT).wrapping_sub(1));
                    }
                    let end: u32 = rd(array, ARRAY_END_INDEX);
                    at = at.wrapping_add(1);
                    if at >= end {
                        break;
                    }
                }
                wr(array, ARRAY_END_INDEX, size);
            }
            let old: usize = rd(array, ARRAY_STORAGE);
            wr(array, ARRAY_STORAGE_COUNT, size);
            let new = eng::operator_new((size << 2) as usize);
            let end: u32 = rd(array, ARRAY_END_INDEX);
            wr(array, ARRAY_STORAGE, new);
            if end != 0 {
                let mut at = 0u32;
                loop {
                    let storage: usize = rd(array, ARRAY_STORAGE);
                    wr::<usize>(storage, at as usize * 4, rd(old, at as usize * 4));
                    let end: u32 = rd(array, ARRAY_END_INDEX);
                    at += 1;
                    if at >= end {
                        break;
                    }
                }
            }
            eng::operator_delete(old);
        }
        if index >= rd::<u32>(array, ARRAY_STORAGE_COUNT) {
            return;
        }
    }
    let count = |delta: u32| wr(array, ARRAY_FILLED_COUNT, rd::<u32>(array, ARRAY_FILLED_COUNT).wrapping_add(delta));
    let stored = || rd::<usize>(rd::<usize>(array, ARRAY_STORAGE), index as usize * 4);
    if index >= rd::<u32>(array, ARRAY_END_INDEX) {
        wr(array, ARRAY_END_INDEX, index.wrapping_add(1));
        if element != 0 {
            count(1);
        }
    } else if element != 0 {
        if stored() == 0 {
            count(1);
        }
    } else if stored() != 0 {
        count(u32::MAX);
    }
    wr(rd::<usize>(array, ARRAY_STORAGE), index as usize * 4, element);
}

/// The regions of `array_add` and of the engine functions that add to the same array. They are
/// the array and its storage. A new storage block needs no region. The call is a skip when the
/// allocator hooks are not active. The call can allocate a storage block.
unsafe fn array_regions(set: &mut Regions, array: usize) {
    set.add(array, ARRAY_SIZE);
    set.add(rd(array, ARRAY_STORAGE), rd::<u32>(array, ARRAY_STORAGE_COUNT) as usize * 4);
    set.allocates();
}

// ---------------------------------------------------------------------------------------------
// NiRenderer
// ---------------------------------------------------------------------------------------------

/// 0x6F4E30 `NiRenderer::ctor`. The port puts the renderer at the end of the list of renderers.
pub unsafe fn renderer_ctor(this: usize) -> usize {
    crate::shadow::touch(&renderer_ctor_port::STATE);
    eng::ni_object_ctor(this);
    wr::<usize>(this, R::accumulator, 0);
    wr::<usize>(this, R::currentPropertyState, 0);
    wr::<usize>(this, R::currentEffectState, 0);
    wr(this, R::vtbl, VTBL_NI_RENDERER);
    let accumulator: usize = rd(this, R::accumulator);
    if accumulator != 0 {
        ni::release(accumulator);
        wr::<usize>(this, R::accumulator, 0);
    }

    let mut node: usize = rd(RENDERER_LIST_FREE, 0);
    if node == 0 {
        // There is no free node. The port makes a new block of nodes and links them through `next`.
        let block = eng::operator_new(8);
        if block != 0 {
            let count: u32 = rd(RENDERER_LIST_BLOCK_NODES, 0);
            wr::<usize>(block, 0, 0);
            let bytes = count.wrapping_mul(3) << 2;
            let nodes = eng::operator_new(bytes as usize);
            let mut left = count.wrapping_sub(1);
            wr(block, 4, nodes);
            if left != 0 {
                let mut at = 0usize;
                loop {
                    let nodes: usize = rd(block, 4);
                    wr(nodes.wrapping_add(at), NODE_NEXT, nodes.wrapping_add(at).wrapping_add(NODE_SIZE));
                    at += NODE_SIZE;
                    left -= 1;
                    if left == 0 {
                        break;
                    }
                }
            }
            let nodes: usize = rd(block, 4);
            wr::<usize>(nodes.wrapping_add(bytes as usize).wrapping_sub(8), 0, 0);
        }
        // The game code does not test the block again. A failed allocation causes a fault here.
        wr(block, 0, rd::<usize>(RENDERER_LIST_BLOCKS, 0));
        wr(RENDERER_LIST_BLOCKS, 0, block);
        node = rd(block, 4);
        wr(RENDERER_LIST_FREE, 0, node);
    }
    wr(RENDERER_LIST_FREE, 0, rd::<usize>(node, NODE_NEXT));
    wr(node, NODE_RENDERER, this);
    wr::<usize>(node, NODE_NEXT, 0);
    wr(node, NODE_PREVIOUS, rd::<usize>(RENDERER_LIST_TAIL, 0));
    let tail: usize = rd(RENDERER_LIST_TAIL, 0);
    if tail != 0 {
        wr(tail, NODE_NEXT, node);
    } else {
        wr(RENDERER_LIST_HEAD, 0, node);
    }
    wr(RENDERER_LIST_TAIL, 0, node);
    wr(RENDERER_LIST_COUNT, 0, rd::<u32>(RENDERER_LIST_COUNT, 0).wrapping_add(1));
    this
}

/// The call is a skip when the list has no free node and the allocator hooks are not active.
/// The constructor allocates a block of nodes then.
unsafe fn renderer_ctor_regions(set: &mut Regions, this: usize) {
    set.add(this, R::SIZE);
    set.add(NI_REF_OBJECT_COUNT, 4);
    set.add(RENDERER_LIST, RENDERER_LIST_SIZE);
    let free: usize = rd(RENDERER_LIST_FREE, 0);
    if free == 0 {
        // This call makes a block of nodes.
        set.allocates();
    }
    set.add(free, NODE_SIZE);
    set.add(rd(RENDERER_LIST_TAIL, 0), NODE_SIZE);
}

/// The node of the renderer list that holds `renderer`, or 0.
unsafe fn renderer_list_find(renderer: usize) -> usize {
    let mut next: usize = rd(RENDERER_LIST_HEAD, 0);
    while next != 0 {
        let node = next;
        let held: usize = rd(node, NODE_RENDERER);
        next = rd(node, NODE_NEXT);
        if renderer == held {
            return node;
        }
    }
    0
}

/// 0x6F4F80 `NiRenderer::dtor`
pub unsafe fn renderer_dtor(this: usize) {
    crate::shadow::touch(&renderer_dtor_port::STATE);
    wr(this, R::vtbl, VTBL_NI_RENDERER);
    let mut node = renderer_list_find(this);
    if node != 0 {
        eng::renderer_list_remove(RENDERER_LIST, &mut node);
    }
    for member in [R::currentEffectState, R::currentPropertyState, R::accumulator] {
        let object: usize = rd(this, member);
        if object != 0 {
            ni::release(object);
        }
    }
    eng::ni_object_dtor(this);
}

/// The destructor writes the renderer, the list and the nodes adjacent to the node that it
/// removes. It also writes the reference counts of the three objects that the renderer holds.
/// The call is a skip when the destructor deletes one of them. A virtual deleting destructor
/// cannot run two times.
unsafe fn renderer_dtor_regions(set: &mut Regions, this: usize) {
    set.add(this, R::SIZE);
    set.add(NI_REF_OBJECT_COUNT, 4);
    set.add(RENDERER_LIST, RENDERER_LIST_SIZE);
    let node = renderer_list_find(this);
    if node != 0 {
        for other in [node, rd::<usize>(node, NODE_NEXT), rd::<usize>(node, NODE_PREVIOUS)] {
            set.add(other, NODE_SIZE);
        }
    }
    for member in [R::currentEffectState, R::currentPropertyState, R::accumulator] {
        let object: usize = rd(this, member);
        if !release_regions(set, object) {
            return set.skip();
        }
    }
}

/// 0x6F4F60 `NiRenderer::deleting_dtor`
pub unsafe fn renderer_deleting_dtor(this: usize, flags: u8) -> usize {
    crate::shadow::touch(&renderer_deleting_dtor_port::STATE);
    deleting(this, flags, renderer_dtor)
}

/// 0x6F5070 `NiRenderer::RegisterObject`. The port gives the geometry to the accumulator.
pub unsafe fn renderer_register_object(this: usize, geometry: usize) -> u8 {
    crate::shadow::touch(&renderer_register_object_port::STATE);
    let accumulator: usize = rd(this, R::accumulator);
    if accumulator == 0 {
        return 0;
    }
    (vcall1(accumulator, SLOT_ACCUMULATOR_REGISTER_OBJECT, geometry as u32) as u8 != 0) as u8
}

/// This function calls the virtual function at `slot` of every renderer in the list with
/// `object`. The list ends at the first node that has no renderer.
unsafe fn for_each_renderer(slot: usize, object: usize) {
    let mut node: usize = rd(RENDERER_LIST_HEAD, 0);
    while node != 0 {
        let renderer: usize = rd(node, NODE_RENDERER);
        node = rd(node, NODE_NEXT);
        if renderer == 0 {
            break;
        }
        vcall1(renderer, slot, object as u32);
    }
}

/// 0x6F5090 `NiRenderer_static::RemoveEffect`
pub unsafe fn renderer_remove_effect(effect: usize) {
    crate::shadow::touch(&renderer_remove_effect_port::STATE);
    for_each_renderer(SLOT_PURGE_EFFECT, effect);
}

/// 0x6F50C0 `NiRenderer_static::RemoveGeometryData`
pub unsafe fn renderer_remove_geometry_data(data: usize) {
    crate::shadow::touch(&renderer_remove_geometry_data_port::STATE);
    for_each_renderer(SLOT_PURGE_GEOMETRY_DATA, data);
}

/// 0x6F50F0 `NiRenderer_static::RemoveSkinPartitionRendererData`
pub unsafe fn renderer_remove_skin_partition(partition: usize) {
    crate::shadow::touch(&renderer_remove_skin_partition_port::STATE);
    for_each_renderer(SLOT_PURGE_SKIN_PARTITION, partition);
}

/// 0x6F5120 `NiRenderer_static::RemoveMaterial`
pub unsafe fn renderer_remove_material(material: usize) {
    crate::shadow::touch(&renderer_remove_material_port::STATE);
    for_each_renderer(SLOT_PURGE_MATERIAL, material);
}

/// The call is a skip when the list has a renderer. The purge functions of a renderer free
/// device objects and renderer data.
unsafe fn for_each_renderer_regions(set: &mut Regions) {
    if rd::<usize>(RENDERER_LIST_HEAD, 0) != 0 {
        set.skip();
    }
}

/// The two divisions of `MapWindowPointToBufferPoint` (0x6F517F to 0x6F51B9), with the
/// instructions of the game code. The code loads the unsigned point as a 64-bit integer. It
/// loads the size of the buffer as a signed 32-bit integer. The function returns the bits of
/// `x / width` and of `one - y / height`.
fn window_to_buffer(x: u32, width: u32, y: u32, height: u32, one: u32) -> (u32, u32) {
    let input: [u32; 7] = [x, 0, width, y, 0, height, one];
    let mut output = [0u32; 2];
    unsafe {
        asm!(
            "fild qword ptr [{i}]",
            "fidiv dword ptr [{i} + 8]",
            "fstp dword ptr [{o}]",
            "fild qword ptr [{i} + 12]",
            "fidiv dword ptr [{i} + 20]",
            "fsubr dword ptr [{i} + 24]",
            "fstp dword ptr [{o} + 4]",
            i = in(reg) input.as_ptr(),
            o = in(reg) output.as_mut_ptr(),
            clobber_abi("C"),
            options(nostack),
        );
    }
    (output[0], output[1])
}

/// 0x6F5150 `NiRenderer::MapWindowPointToBufferPoint`. The port returns 0 when the back buffer
/// has no size.
pub unsafe fn renderer_map_window_point_to_buffer_point(this: usize, x: u32, y: u32, out_x: usize, out_y: usize) -> u8 {
    crate::shadow::touch(&renderer_map_window_point_to_buffer_point_port::STATE);
    let width = vcall0(this, SLOT_GET_BACKBUFFER_WIDTH);
    let height = vcall0(this, SLOT_GET_BACKBUFFER_HEIGHT);
    if width == 0 || height == 0 {
        return 0;
    }
    let (buffer_x, buffer_y) = window_to_buffer(x, width, y, height, map_one());
    wr(out_x, 0, buffer_x);
    wr(out_y, 0, buffer_y);
    1
}

/// The 1.0 that the game code subtracts from.
#[cfg(not(test))]
unsafe fn map_one() -> u32 {
    rd(F_ONE, 0)
}

#[cfg(test)]
unsafe fn map_one() -> u32 {
    let _ = F_ONE;
    1.0f32.to_bits()
}

/// 0x6F51D0 `NiRenderer::renderFromClusterAccumulator`. The port displays a scene object
/// without the accumulator. The states of the object are the current states for the display.
/// Then the port puts the accumulator and the states back.
pub unsafe fn renderer_render_from_cluster_accumulator(this: usize, camera: usize, scene_object: usize) {
    crate::shadow::touch(&renderer_render_from_cluster_accumulator_port::STATE);
    let saved_accumulator: usize = rd(this, R::accumulator);
    if saved_accumulator != 0 {
        ni::add_reference(saved_accumulator);
    }
    let accumulator: usize = rd(this, R::accumulator);
    if accumulator != 0 {
        eng::ni_object_release(accumulator);
        wr::<usize>(this, R::accumulator, 0);
    }

    let effect_slot = this + R::currentEffectState;
    let saved_effect: usize = rd(effect_slot, 0);
    if saved_effect != 0 {
        ni::add_reference(saved_effect);
    }
    // The game code uses the stack slot of the argument for the smart pointer that the object returns.
    let mut result: usize = scene_object;
    let result_at = core::ptr::addr_of_mut!(result) as u32;
    let returned = vcall1(scene_object, SLOT_GET_EFFECT_STATE, result_at) as usize;
    eng::ni_pointer_set(effect_slot, rd(returned, 0));
    let held = core::ptr::read_volatile(core::ptr::addr_of!(result));
    if held != 0 {
        eng::ni_object_release(held);
    }

    let property_slot = this + R::currentPropertyState;
    let saved_property: usize = rd(property_slot, 0);
    if saved_property != 0 {
        ni::add_reference(saved_property);
    }
    let returned = vcall1(scene_object, SLOT_GET_PROPERTY_STATE, result_at) as usize;
    eng::ni_pointer_operator_assign(property_slot, rd(returned, 0));
    let held = core::ptr::read_volatile(core::ptr::addr_of!(result));
    if held != 0 {
        ni::release(held);
    }

    vcall1(scene_object, SLOT_DISPLAY, camera as u32);

    eng::ni_pointer_operator_assign(property_slot, saved_property);
    let effect: usize = rd(effect_slot, 0);
    if effect != saved_effect {
        if effect != 0 {
            eng::ni_object_release(effect);
        }
        wr(effect_slot, 0, saved_effect);
        if saved_effect != 0 {
            ni::add_reference(saved_effect);
        }
    }
    ni::pointer_assign(this + R::accumulator, saved_accumulator);
    for object in [saved_property, saved_effect, saved_accumulator] {
        if object != 0 {
            ni::release(object);
        }
    }
}

/// 0x6F5380 `NiRenderer::BeginBatch`. The states of the batch become the current states.
pub unsafe fn renderer_begin_batch(this: usize, property_state: usize, effect_state: usize) {
    crate::shadow::touch(&renderer_begin_batch_port::STATE);
    ni::pointer_assign(this + R::currentPropertyState, property_state);
    ni::pointer_assign(this + R::currentEffectState, effect_state);
}

/// The call is a skip when the call deletes a state that it replaces. A virtual deleting
/// destructor cannot run two times.
unsafe fn renderer_begin_batch_regions(set: &mut Regions, this: usize, property_state: usize, effect_state: usize) {
    set.add(this, R::SIZE);
    for (member, new) in [(R::currentPropertyState, property_state), (R::currentEffectState, effect_state)] {
        let now: usize = rd(this, member);
        if now != new {
            if !release_regions(set, now) {
                return set.skip();
            }
            if new != 0 {
                set.add(new + REF_COUNT, 4);
            }
        }
    }
}

/// 0x6F5400 `NiRenderer::AddViewerStrings`
pub unsafe fn renderer_add_viewer_strings(this: usize, strings: usize) {
    crate::shadow::touch(&renderer_add_viewer_strings_port::STATE);
    eng::ni_object_add_viewer_strings(this, strings);
    array_add(strings, eng::viewer_section_text(rd(RTTI_NI_RENDERER, 0)));
}

/// 0x6F5630 `NiRenderer::GetRTTI`
pub unsafe fn renderer_get_rtti(_this: usize) -> usize {
    crate::shadow::touch(&renderer_get_rtti_port::STATE);
    RTTI_NI_RENDERER
}

/// 0x6F5640 `NiRenderer::SetUseWBuffer`. The port returns 1 for "no W buffer". The base class
/// has only "no W buffer".
pub unsafe fn renderer_set_use_w_buffer(_this: usize, use_w_buffer: u8) -> u32 {
    crate::shadow::touch(&renderer_set_use_w_buffer_port::STATE);
    (use_w_buffer == 0) as u32
}

/// 0x6F5660 `NiRenderer::stub_returnsTrue`
pub unsafe fn renderer_stub_returns_true(_this: usize, _a: u32) -> u8 {
    crate::shadow::touch(&renderer_stub_returns_true_port::STATE);
    1
}

/// 0x6F5680 `NiRenderer::PrecacheGeometry`
pub unsafe fn renderer_precache_geometry(_this: usize, _geometry: usize, _flags: u32, _property: usize, _bones_per_partition: u32, _bones_per_vertex: u32) -> u8 {
    crate::shadow::touch(&renderer_precache_geometry_port::STATE);
    0
}

/// 0x6F5690 `NiRenderer::stub_returns800000`
pub unsafe fn renderer_stub_returns_800000(_this: usize, _a: u32, _b: u32) -> u32 {
    crate::shadow::touch(&renderer_stub_returns_800000_port::STATE);
    0x80_0000
}

/// 0x6F56A0 `NiRenderer::GetTextureMemoryStats`
pub unsafe fn renderer_get_texture_memory_stats(_this: usize, _a: u32, _b: u32) -> u8 {
    crate::shadow::touch(&renderer_get_texture_memory_stats_port::STATE);
    0
}

/// 0x6A9C40 `NiRenderer::GetTextureStats`
pub unsafe fn renderer_get_texture_stats(_this: usize, _a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32) -> u8 {
    crate::shadow::touch(&renderer_get_texture_stats_port::STATE);
    0
}

/// 0x6A9C60 `NiRenderer::constantFalse@8`
pub unsafe fn renderer_constant_false_8(_this: usize, _a: u32, _b: u32) -> u32 {
    crate::shadow::touch(&renderer_constant_false_8_port::STATE);
    0
}

// ---------------------------------------------------------------------------------------------
// NiRendererSpecificProperty
// ---------------------------------------------------------------------------------------------

/// 0x6DC7F0 `NiRendererSpecificProperty::RenderDataBase::dtor`
pub unsafe fn render_data_base_dtor(this: usize) {
    crate::shadow::touch(&render_data_base_dtor_port::STATE);
    wr(this, 0, VTBL_RENDER_DATA_BASE);
}

/// 0x65F5A0 `NiRendererSpecificProperty::RenderData7::dtor`. The port releases the three
/// objects. Then it does the three members again, as their own destructors.
pub unsafe fn render_data_7_dtor(this: usize) {
    crate::shadow::touch(&render_data_7_dtor_port::STATE);
    wr(this, RD7::vtbl, VTBL_RENDER_DATA_7);
    for member in [RD7::texturePass, RD7::field_8, RD7::field_C] {
        let object: usize = rd(this, member);
        if object != 0 {
            ni::release(object);
            wr::<usize>(this, member, 0);
        }
    }
    for member in [RD7::field_C, RD7::field_8, RD7::texturePass] {
        let object: usize = rd(this, member);
        if object != 0 {
            ni::release(object);
        }
    }
    render_data_base_dtor(this);
}

/// The call is a skip when the call deletes one of the three objects. A virtual deleting
/// destructor cannot run two times.
unsafe fn render_data_7_dtor_regions(set: &mut Regions, this: usize) {
    set.add(this, RD7::SIZE);
    for member in [RD7::texturePass, RD7::field_8, RD7::field_C] {
        if !release_regions(set, rd(this, member)) {
            return set.skip();
        }
    }
}

/// 0x65F560 `NiRendererSpecificProperty::RenderData7::deleting_dtor`
pub unsafe fn render_data_7_deleting_dtor(this: usize, flags: u8) -> usize {
    crate::shadow::touch(&render_data_7_deleting_dtor_port::STATE);
    deleting(this, flags, render_data_7_dtor)
}

/// 0x65F580 `NiRendererSpecificProperty::RenderDataBase::deleting_dtor`
pub unsafe fn render_data_base_deleting_dtor(this: usize, flags: u8) -> usize {
    crate::shadow::touch(&render_data_base_deleting_dtor_port::STATE);
    deleting(this, flags, render_data_base_dtor)
}

/// The writes that the constructor adds to a `NiObjectNET`.
unsafe fn property_init(this: usize) {
    wr(this, RSP::vtbl, VTBL_RENDERER_SPECIFIC_PROPERTY);
    wr::<u16>(this, RSP::flags, 0);
    wr::<usize>(this, RSP::renderData, 0);
}

/// 0x6DC800 `NiRendererSpecificProperty::ctor`
pub unsafe fn property_ctor(this: usize) -> usize {
    crate::shadow::touch(&property_ctor_port::STATE);
    eng::ni_object_net_ctor(this);
    property_init(this);
    this
}

/// The call is never a skip.
unsafe fn property_ctor_regions(set: &mut Regions, this: usize) {
    set.add(this, RSP::SIZE);
    set.add(NI_REF_OBJECT_COUNT, 4);
}

/// 0x6DC840 `NiRendererSpecificProperty::dtor`
pub unsafe fn property_dtor(this: usize) {
    crate::shadow::touch(&property_dtor_port::STATE);
    wr(this, RSP::vtbl, VTBL_RENDERER_SPECIFIC_PROPERTY);
    let render_data: usize = rd(this, RSP::renderData);
    if render_data != 0 {
        ni::delete_object(render_data);
    }
    eng::ni_object_net_dtor(this);
}

/// The call is a skip when the call deletes the render data, the extra data or the controllers.
/// A virtual deleting destructor cannot run two times. The destructor of `NiObjectNET` frees the
/// name. Thus the call is also a skip when there is a name and the allocator hooks are not
/// active.
unsafe fn property_dtor_regions(set: &mut Regions, this: usize) {
    set.add(this, RSP::SIZE);
    set.add(NI_REF_OBJECT_COUNT, 4);
    if rd::<usize>(this, RSP::renderData) != 0 {
        return set.skip();
    }
    for member in [NET_EXTRA_DATA, NET_CONTROLLERS] {
        if !release_regions(set, rd(this, member)) {
            return set.skip();
        }
    }
    if rd::<usize>(this, NET_NAME) != 0 {
        set.allocates();
    }
}

/// 0x6DC820 `NiRendererSpecificProperty::deleting_dtor`
pub unsafe fn property_deleting_dtor(this: usize, flags: u8) -> usize {
    crate::shadow::touch(&property_deleting_dtor_port::STATE);
    deleting(this, flags, property_dtor)
}

/// A new `NiRendererSpecificProperty`, as `CreateClone` and `CreateFromStream` make it inline.
unsafe fn property_new() -> usize {
    let property = eng::operator_new(RSP::SIZE);
    if property != 0 {
        eng::ni_object_net_ctor(property);
        property_init(property);
    }
    property
}

/// 0x6DC8A0 `NiRendererSpecificProperty::CreateClone`
pub unsafe fn property_create_clone(this: usize) -> usize {
    crate::shadow::touch(&property_create_clone_port::STATE);
    let clone = property_new();
    eng::ni_property_copy_members(this, clone);
    clone
}

/// The call is a skip when the property has extra data or controllers.
/// `NiObjectNET::CopyMembers` clones them through their own virtual functions. The call is also
/// a skip when the allocator hooks are not active. It allocates the clone.
unsafe fn property_create_clone_regions(set: &mut Regions, this: usize) {
    set.add(NI_REF_OBJECT_COUNT, 4);
    set.allocates();
    if rd::<usize>(this, NET_EXTRA_DATA) != 0 || rd::<usize>(this, NET_CONTROLLERS) != 0 {
        set.skip();
    }
}

/// 0x6DC920 `NiRendererSpecificProperty_static::CreateFromStream`. The port makes a new property
/// and its link record. Then the property reads itself from the stream.
pub unsafe fn property_create_from_stream(stream: usize, out: usize) {
    crate::shadow::touch(&property_create_from_stream_port::STATE);
    let property = property_new();
    let link_data = eng::operator_new(12);
    if link_data != 0 {
        wr(link_data, 0, VTBL_STREAM_LINK_DATA);
    }
    array_add(stream + STREAM_LINK_ARRAY, link_data);
    wr(stream, STREAM_LINK_DATA, link_data);
    vcall1(property, SLOT_LOAD_BINARY, stream as u32);
    wr(out, 0, property);
}

/// 0x6DCAE0 `NiRendererSpecificProperty::RegisterStreamables`
pub unsafe fn property_register_streamables(this: usize, stream: usize) -> u8 {
    crate::shadow::touch(&property_register_streamables_port::STATE);
    (eng::ni_property_register_streamables(this, stream) as u8 != 0) as u8
}

/// 0x6DCB00 `NiRendererSpecificProperty::IsEqual`
pub unsafe fn property_is_equal(this: usize, other: usize) -> u8 {
    crate::shadow::touch(&property_is_equal_port::STATE);
    (eng::ni_property_is_equal(this, other) as u8 != 0) as u8
}

/// 0x6DCB20 `NiRendererSpecificProperty::AddViewerStrings`
pub unsafe fn property_add_viewer_strings(this: usize, strings: usize) {
    crate::shadow::touch(&property_add_viewer_strings_port::STATE);
    eng::ni_property_add_viewer_strings(this, strings);
    array_add(strings, eng::viewer_section_text(rd(RTTI_RENDERER_SPECIFIC_PROPERTY, 0)));
}

/// 0x6DCCA0 `NiRendererSpecificProperty::GetRTTI`
pub unsafe fn property_get_rtti(_this: usize) -> usize {
    crate::shadow::touch(&property_get_rtti_port::STATE);
    RTTI_RENDERER_SPECIFIC_PROPERTY
}

/// 0x6DCCD0 `NiRendererSpecificProperty::GetType`
pub unsafe fn property_get_type(_this: usize) -> u32 {
    crate::shadow::touch(&property_get_type_port::STATE);
    11
}

// ---------------------------------------------------------------------------------------------
// NiDX8MaterialManager
// ---------------------------------------------------------------------------------------------

/// Removes every node from the map and frees it. It does not write the count of the map.
unsafe fn map_remove_all(map: usize) {
    hash_map::drain_with(map, |node| eng::operator_delete(node));
}

/// 0x6C22C0 `NiDX8MaterialManager::ctor`. The port makes an empty map with 37 buckets.
pub unsafe fn material_manager_ctor(this: usize) -> usize {
    crate::shadow::touch(&material_manager_ctor_port::STATE);
    wr(this, MM::vtbl, VTBL_NI_REF_OBJECT);
    wr::<u32>(this, MM::refCount, 0);
    ni::ref_object_count_add(1);
    let map = this + MM::hashmap_8;
    wr(map, HM::vtbl, VTBL_MATERIAL_MAP);
    wr::<u32>(map, HM::bucketCount, 37);
    wr::<u32>(map, HM::count, 0);
    let buckets = eng::operator_new(37 * 4);
    let bytes = rd::<u32>(map, HM::bucketCount) << 2;
    wr(map, HM::arrayBuckets, buckets);
    core::ptr::write_bytes(buckets as *mut u8, 0, bytes as usize);
    wr(this, MM::vtbl, VTBL_MATERIAL_MANAGER);
    this
}

/// The call is a skip when the allocator hooks are not active. The constructor allocates the
/// buckets.
unsafe fn material_manager_ctor_regions(set: &mut Regions, this: usize) {
    set.add(this, MM::SIZE);
    set.add(NI_REF_OBJECT_COUNT, 4);
    set.allocates();
}

/// 0x6C2370 `NiDX8MaterialManager::Initialize_maybe`. The port frees the entries of the map,
/// empties the map and selects the default material.
pub unsafe fn material_manager_initialize(this: usize) {
    crate::shadow::touch(&material_manager_initialize_port::STATE);
    let map = this + MM::hashmap_8;
    let mut node = hash_map::first(map);
    while node != 0 {
        let entry: usize = rd(node, HN::data);
        node = hash_map::next(map, node);
        eng::operator_delete(entry);
    }
    map_remove_all(map);
    wr::<u32>(map, HM::count, 0);
    wr::<usize>(this, MM::currentMaterialEntry, 0);
    material::set_current_material(this, default_material());
    wr::<u8>(this, MM::changed, 1);
}

unsafe fn default_material() -> usize {
    rd(DEFAULT_MATERIAL, 0)
}

/// The memory that the manager writes outside itself when it starts or ends. It is the buckets
/// of the map, the default material and the entry of that material. The call is a skip when the
/// allocator hooks are not active. The manager allocates and frees entries.
unsafe fn material_manager_shared_regions(set: &mut Regions, this: usize) {
    let map = this + MM::hashmap_8;
    set.add(rd(map, HM::arrayBuckets), rd::<u32>(map, HM::bucketCount) as usize * 4);
    let default = default_material();
    set.add(default, MP::SIZE);
    if default != 0 {
        set.add(rd(default, MP::rendererData), material::ENTRY_SIZE);
    }
    set.allocates();
}

/// The call is a skip when the allocator hooks are not active. See
/// `material_manager_shared_regions`.
unsafe fn material_manager_initialize_regions(set: &mut Regions, this: usize) {
    set.add(this, MM::SIZE);
    material_manager_shared_regions(set, this);
}

/// 0x6C2450 `NiDX8MaterialManager::dtor`
pub unsafe fn material_manager_dtor(this: usize) {
    crate::shadow::touch(&material_manager_dtor_port::STATE);
    wr(this, MM::vtbl, VTBL_MATERIAL_MANAGER);
    let map = this + MM::hashmap_8;
    while rd::<u32>(map, HM::count) != 0 {
        // The port gets the first node. It also does the step to the node after it. The game
        // code does this step and does not use the result.
        let first = hash_map::first(map);
        let entry: usize = rd(first, HN::data);
        let key: u32 = rd(first, HN::key);
        hash_map::next(map, first);
        hash_map::remove_with(map, key, |node| eng::operator_delete(node));
        eng::operator_delete(entry);
    }

    let mut default = default_material();
    if default != 0 {
        let entry: usize = rd(default, MP::rendererData);
        if entry != 0 {
            eng::operator_delete(entry);
            default = default_material();
        }
    }
    wr::<usize>(default, MP::rendererData, 0);

    wr(map, HM::vtbl, VTBL_MATERIAL_MAP);
    map_remove_all(map);
    let buckets: usize = rd(map, HM::arrayBuckets);
    wr::<u32>(map, HM::count, 0);
    eng::operator_delete(buckets);
    wr(this, MM::vtbl, VTBL_NI_REF_OBJECT);
    ni::ref_object_count_add(-1);
}

/// The destructor removes a node at the head of its bucket, so it writes no other node. This
/// is true only when the hash function of the map gives the bucket that the node is in. The
/// call is a skip when the allocator hooks are not active. See
/// `material_manager_shared_regions`.
unsafe fn material_manager_dtor_regions(set: &mut Regions, this: usize) {
    set.add(this, MM::SIZE);
    set.add(NI_REF_OBJECT_COUNT, 4);
    material_manager_shared_regions(set, this);
}

/// 0x6C2350 `NiDX8MaterialManager::deleting_dtor`
pub unsafe fn material_manager_deleting_dtor(this: usize, flags: u8) -> usize {
    crate::shadow::touch(&material_manager_deleting_dtor_port::STATE);
    deleting(this, flags, material_manager_dtor)
}

/// 0x6C26A0 `NiDX8MaterialManager::PurgeMaterial`. The port frees the entry of the material.
/// The material keeps the pointer.
pub unsafe fn material_manager_purge_material(_this: usize, material: usize) {
    crate::shadow::touch(&material_manager_purge_material_port::STATE);
    if material != 0 {
        let entry: usize = rd(material, MP::rendererData);
        if entry != 0 {
            eng::operator_delete(entry);
        }
    }
}

/// The call is a skip when the material has an entry and the allocator hooks are not active.
/// The call frees the entry then.
unsafe fn material_manager_purge_material_regions(set: &mut Regions, material: usize) {
    if material != 0 && rd::<usize>(material, MP::rendererData) != 0 {
        set.allocates();
    }
}

// ---------------------------------------------------------------------------------------------
// NiDX8RenderState
// ---------------------------------------------------------------------------------------------

/// The table from a `NiAlphaProperty` blend factor to a D3DBLEND value, in the order that the
/// constructor writes it.
const ALPHA_MAPPING: [u32; 11] = [2, 1, 3, 4, 9, 10, 5, 6, 7, 8, 11];

/// The compare, stencil and cull tables, as (offset, value) pairs in the order that the
/// constructor writes them.
const MAPPING_WRITES: [(usize, u32); 45] = [
    (RS::alphaTestMapping + 0x04, 2),
    (RS::alphaTestMapping + 0x14, 6),
    (RS::alphaTestMapping + 0x1C, 1),
    (RS::stencilFuncMapping + 0x04, 2),
    (RS::stencilFuncMapping + 0x14, 6),
    (RS::stencilFuncMapping, 1),
    (RS::stencilActionMapping, 1),
    (RS::stencilActionMapping + 0x04, 2),
    (RS::stencilActionMapping + 0x14, 6),
    (RS::cullModeMapping, 2),
    (RS::cullModeMapping + 0x08, 2),
    (RS::cullModeMapping + 0x18, 1),
    (RS::cullModeMapping + 0x14, 2),
    (RS::cullModeMapping + 0x1C, 1),
    (RS::zFuncMapping + 0x04, 2),
    (RS::zFuncMapping + 0x14, 6),
    (RS::zFuncMapping + 0x1C, 1),
    (RS::alphaTestMapping, 8),
    (RS::alphaTestMapping + 0x08, 3),
    (RS::alphaTestMapping + 0x0C, 4),
    (RS::alphaTestMapping + 0x10, 5),
    (RS::alphaTestMapping + 0x18, 7),
    (RS::stencilFuncMapping + 0x1C, 8),
    (RS::stencilFuncMapping + 0x08, 3),
    (RS::stencilFuncMapping + 0x0C, 4),
    (RS::stencilFuncMapping + 0x10, 5),
    (RS::stencilFuncMapping + 0x18, 7),
    (RS::stencilActionMapping + 0x08, 3),
    (RS::stencilActionMapping + 0x0C, 4),
    (RS::stencilActionMapping + 0x10, 5),
    (RS::cullModeMapping + 0x10, 3),
    (RS::cullModeMapping + 0x04, 3),
    (RS::cullModeMapping + 0x0C, 3),
    (RS::bLeftRightSwap, 0),
    (RS::zFuncMapping, 8),
    (RS::zFuncMapping + 0x08, 3),
    (RS::zFuncMapping + 0x0C, 4),
    (RS::zFuncMapping + 0x10, 5),
    (RS::zFuncMapping + 0x18, 7),
    (RS::frustumNear, 0),
    (RS::frustumFar, 0),
    (RS::frustumDistance, 0),
    (RS::inverseMaxFogValue, 0),
    (RS::maxFogValue, 0x3F80_0000),
    (RS::currentFogProperty, 0),
];

/// The first block of the constructor after the device: the blend caps and the blend table.
unsafe fn ctor_alpha_mapping(this: usize) {
    wr::<u32>(this, RS::sourceBlendCaps, 0);
    wr::<u32>(this, RS::destBlendCaps, 0);
    for (index, value) in ALPHA_MAPPING.into_iter().enumerate() {
        wr(this, RS::alphaMapping + index * 4, value);
    }
    wr::<usize>(this, RS::currentAlphaProperty, 0);
}

/// The second block: the compare, stencil and cull tables and the fog members.
unsafe fn ctor_mapping_tables(this: usize) {
    for (offset, value) in MAPPING_WRITES {
        wr(this, offset, value);
    }
}

/// 0x6B7D70 `NiDX8RenderState::ctor`
pub unsafe fn render_state_ctor(this: usize, device: usize, caps: usize) -> usize {
    crate::shadow::touch(&render_state_ctor_port::STATE);
    wr(this, RS::vtbl, VTBL_NI_REF_OBJECT);
    wr::<u32>(this, RS::refCount, 0);
    ni::ref_object_count_add(1);
    wr::<usize>(this, RS::alphaProperty, 0);
    material_manager_ctor(this + RS::materialManager);
    for index in 0..256 {
        wr::<u32>(this, RS::renderStateSettings + index * 8, 0x7FFF_FFFF);
        wr::<u32>(this, RS::renderStateSettings + index * 8 + 4, 0x7FFF_FFFF);
    }
    wr::<u32>(this, RS::currentPixelShader, 0x7FFF_FFFF);
    wr::<u32>(this, RS::field_9FC, 0x7FFF_FFFF);
    wr::<u32>(this, RS::currentVertexShader, 0x7FFF_FFFF);
    wr::<u32>(this, RS::field_A04, 0x7FFF_FFFF);
    for index in 0..256 {
        wr::<u32>(this, RS::maybe_textureStageStateSettings + index * 8, 0x7FFF_FFFF);
        wr::<u32>(this, RS::maybe_textureStageStateSettings + index * 8 + 4, 0x7FFF_FFFF);
    }
    wr(this, RS::vtbl, VTBL_RENDER_STATE);
    wr(this, RS::device, device);
    d3d::device_add_ref(device);

    ctor_alpha_mapping(this);

    // The default alpha property of the render state. It has the test function "greater" and
    // no blending.
    let alpha_property = eng::operator_new(ALPHA_PROPERTY_SIZE);
    if alpha_property != 0 {
        eng::ni_object_net_ctor(alpha_property);
        wr(alpha_property, 0, VTBL_NI_ALPHA_PROPERTY);
        wr::<u16>(alpha_property, ALPHA_PROPERTY_FLAGS, 0xEC);
        wr::<u8>(alpha_property, ALPHA_PROPERTY_TEST_REF, 0);
    }
    ni::pointer_assign(this + RS::alphaProperty, alpha_property);

    ctor_mapping_tables(this);
    core::ptr::copy_nonoverlapping(caps as *const u32, (this + RS::deviceCapabilities) as *mut u32, D3DCAPS8::SIZE / 4);
    wr::<u32>(this, RS::flags, 0);
    wr::<u8>(this, RS::field_1F4, 0);
    wr::<u8>(this, RS::changed, 0);
    this
}

/// The render states and the texture stage states get their defaults again. This is the start
/// of `Initialize_maybe` and of the destructor.
unsafe fn render_state_reset(this: usize) {
    for index in 0..256 {
        let value: u32 = rd(RENDER_STATE_DEFAULTS, index * 4);
        wr(this, RS::renderStateSettings + index * 8 + 4, value);
        wr(this, RS::renderStateSettings + index * 8, value);
    }
    wr::<u32>(this, RS::field_9FC, 0);
    wr::<u32>(this, RS::currentPixelShader, 0);
    wr::<u32>(this, RS::field_A04, 2);
    wr::<u32>(this, RS::currentVertexShader, 2);
    render_state::reset_texture_stage_states(this);
    for stage in 0..8 {
        wr::<usize>(this, RS::texturePerStage + stage * 4, 0);
    }
}

/// 0x6B80D0 `NiDX8RenderState::dtor`
pub unsafe fn render_state_dtor(this: usize) {
    crate::shadow::touch(&render_state_dtor_port::STATE);
    wr(this, RS::vtbl, VTBL_RENDER_STATE);
    render_state_reset(this);
    let device: usize = rd(this, RS::device);
    if device != 0 {
        d3d::device_release(device);
        wr::<usize>(this, RS::device, 0);
    }
    material_manager_dtor(this + RS::materialManager);
    let alpha_property: usize = rd(this, RS::alphaProperty);
    if alpha_property != 0 {
        ni::release(alpha_property);
    }
    wr(this, RS::vtbl, VTBL_NI_REF_OBJECT);
    ni::ref_object_count_add(-1);
}

/// The call is a skip when there is a device. The destructor releases the reference to the
/// device and that cannot occur two times. The call is also a skip when the reference to the
/// alpha property is the last. The destructor deletes the property then.
unsafe fn render_state_dtor_regions(set: &mut Regions, this: usize) {
    if rd::<usize>(this, RS::device) != 0 {
        return set.skip();
    }
    set.add(this, RS::SIZE);
    set.add(NI_REF_OBJECT_COUNT, 4);
    if !release_regions(set, rd(this, RS::alphaProperty)) {
        return set.skip();
    }
    material_manager_shared_regions(set, this + RS::materialManager);
}

/// 0x6B80B0 `NiDX8RenderState::deleting_dtor`
pub unsafe fn render_state_deleting_dtor(this: usize, flags: u8) -> usize {
    crate::shadow::touch(&render_state_deleting_dtor_port::STATE);
    deleting(this, flags, render_state_dtor)
}

/// 0x6B81E0 `NiDX8RenderState_static::Create`
pub unsafe fn render_state_create(device: usize, caps: usize, flag_2: u8, flag_8: u8, flag_4: u8) -> usize {
    crate::shadow::touch(&render_state_create_port::STATE);
    let block = eng::operator_new(RS::SIZE);
    let this = if block != 0 { render_state_ctor(block, device, caps) } else { 0 };
    if flag_2 != 0 {
        wr(this, RS::flags, rd::<u32>(this, RS::flags) | 2);
    }
    render_state_init_from_device_caps(this);
    if flag_4 != 0 {
        wr(this, RS::flags, rd::<u32>(this, RS::flags) | 4);
        wr::<u32>(DEFAULT_ZENABLE, 0, 1);
    }
    if flag_8 != 0 {
        wr(this, RS::flags, rd::<u32>(this, RS::flags) | 8);
    }
    render_state_initialize(this);
    this
}

/// 0x6B8600 `NiDX8RenderState::PurgeMaterial`. It is a jump to the function of the material manager.
pub unsafe fn render_state_purge_material(this: usize, material: usize) {
    crate::shadow::touch(&render_state_purge_material_port::STATE);
    material_manager_purge_material(this + RS::materialManager, material);
}

/// A state of `Initialize_maybe` that goes to the device when the default is another value.
unsafe fn initialize_state(this: usize, state: u32, value: u32) {
    let setting = this + RS::renderStateSettings + state as usize * 8;
    if rd::<u32>(setting, 0) != value {
        render_state::setting_set_value(setting, value, 0);
        d3d::device_set_render_state(rd(this, RS::device), state, value);
    }
}

/// 0x6B8680 `NiDX8RenderState::Initialize_maybe`. The port sets every state to its default.
/// Then it sends to the device the states that the renderer starts with.
pub unsafe fn render_state_initialize(this: usize) {
    crate::shadow::touch(&render_state_initialize_port::STATE);
    render_state_reset(this);
    material_manager_initialize(this + RS::materialManager);

    initialize_state(this, 22, 2); // D3DRS_CULLMODE: D3DCULL_CW
    initialize_state(this, 161, 0); // D3DRS_MULTISAMPLEANTIALIAS
    initialize_state(this, 8, 3); // D3DRS_FILLMODE: D3DFILL_SOLID
    initialize_state(this, 9, 2); // D3DRS_SHADEMODE: D3DSHADE_GOURAUD
    initialize_state(this, 27, 0); // D3DRS_ALPHABLENDENABLE
    initialize_state(this, 19, rd(this, RS::alphaMapping + 6 * 4)); // D3DRS_SRCBLEND: source alpha
    initialize_state(this, 20, rd(this, RS::alphaMapping + 7 * 4)); // D3DRS_DESTBLEND: inverse source alpha
    initialize_state(this, 15, 0); // D3DRS_ALPHATESTENABLE
    initialize_state(this, 25, 0); // D3DRS_ALPHAFUNC

    render_state::set_render_state(this, 24, 0, 0); // D3DRS_ALPHAREF
    render_state::set_render_state(this, 23, 8, 0); // D3DRS_ZFUNC: D3DCMP_ALWAYS
    render_state::set_render_state(this, 14, 0, 0); // D3DRS_ZWRITEENABLE
    render_state::set_render_state(this, 26, 0, 0); // D3DRS_DITHERENABLE
    render_state::set_render_state(this, 29, 0, 0); // D3DRS_SPECULARENABLE
    render_state::set_render_state(this, 28, 0, 0); // D3DRS_FOGENABLE
    let table_fog = if rd::<u8>(this, RS::flags) & 1 != 0 { 3 } else { 0 };
    render_state::set_render_state(this, 35, table_fog, 0); // D3DRS_FOGTABLEMODE: D3DFOG_LINEAR with W fog
    render_state::set_render_state(this, 140, 3, 0); // D3DRS_FOGVERTEXMODE: D3DFOG_LINEAR
    render_state::set_render_state(this, 48, 0, 0); // D3DRS_RANGEFOGENABLE
    render_state::set_render_state(this, 34, 0, 0); // D3DRS_FOGCOLOR
    render_state::set_render_state(this, 143, 1, 0); // D3DRS_NORMALIZENORMALS
    let z_enable = if (rd::<u32>(this, RS::flags) >> 4) & 1 != 0 { 2 } else { 1 };
    render_state::set_render_state(this, 7, z_enable, 0); // D3DRS_ZENABLE: D3DZB_USEW or D3DZB_TRUE
    render_state::set_render_state(this, 141, 1, 0); // D3DRS_COLORVERTEX
}

/// The call is a skip when the allocator hooks are not active. See
/// `material_manager_shared_regions`.
unsafe fn render_state_initialize_regions(set: &mut Regions, this: usize) {
    set.add(this, RS::SIZE);
    material_manager_shared_regions(set, this + RS::materialManager);
}

/// 0x6B8930 `NiDX8RenderState::initFromDeviceCaps`. The port sets the flags and the blend
/// factors that the device supports, from the capabilities of the device.
pub unsafe fn render_state_init_from_device_caps(this: usize) {
    crate::shadow::touch(&render_state_init_from_device_caps_port::STATE);
    let caps = this + RS::deviceCapabilities;
    let raster: u32 = rd(caps, D3DCAPS8::RasterCaps);
    // D3DPRASTERCAPS_FOGTABLE and D3DPRASTERCAPS_WFOG
    let flags = if raster & 0x100 != 0 && raster & 0x10_0000 != 0 {
        eng::debug_output(W_FOG_NOTE);
        rd::<u32>(this, RS::flags) | 1
    } else {
        rd::<u32>(this, RS::flags) & !1
    };
    wr(this, RS::flags, flags);
    // D3DDEVCAPS_HWTRANSFORMANDLIGHT
    if rd::<u32>(caps, D3DCAPS8::DevCaps) & 0x1_0000 != 0 {
        wr::<u32>(DEFAULT_SOFTWARE_VERTEX_PROCESSING, 0, 0);
    }
    let source: u32 = rd(caps, D3DCAPS8::SrcBlendCaps);
    let dest: u32 = rd(caps, D3DCAPS8::DestBlendCaps);
    wr::<u32>(this, RS::destBlendCaps, 0);
    wr::<u32>(this, RS::sourceBlendCaps, 0);
    for index in 0..11 {
        let factor: u32 = rd(this, RS::alphaMapping + index * 4);
        if factor & source != 0 {
            wr(this, RS::sourceBlendCaps, rd::<u32>(this, RS::sourceBlendCaps) | (1 << index));
        }
        if factor & dest != 0 {
            wr(this, RS::destBlendCaps, rd::<u32>(this, RS::destBlendCaps) | (1 << index));
        }
    }
    // D3DPRASTERCAPS_WBUFFER
    let flags: u32 = rd(this, RS::flags);
    if rd::<u32>(caps, D3DCAPS8::RasterCaps) & 0x4_0000 != 0 {
        wr(this, RS::flags, flags | 0x10);
    } else {
        wr(this, RS::flags, flags & !0x10);
    }
}

/// The call is never a skip.
unsafe fn render_state_init_from_device_caps_regions(set: &mut Regions, this: usize) {
    set.add(this, RS::SIZE);
    set.add(DEFAULT_SOFTWARE_VERTEX_PROCESSING, 4);
}

// ---------------------------------------------------------------------------------------------
// Deleting destructors and stubs of the classes of the other modules
// ---------------------------------------------------------------------------------------------

/// 0x6B3380 `NiDX8TexturePass::deleting_dtor`
pub unsafe fn texture_pass_deleting_dtor(this: usize, flags: u8) -> usize {
    crate::shadow::touch(&texture_pass_deleting_dtor_port::STATE);
    deleting(this, flags, texture_pass_a::dtor)
}

/// 0x6B41A0 `NiDX8TexturePass::stub_returns0_2`
pub unsafe fn texture_pass_stub_returns_0_2(_this: usize, _a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32, _h: u32, _i: u32) -> u32 {
    crate::shadow::touch(&texture_pass_stub_returns_0_2_port::STATE);
    0
}

/// 0x6B56A0 `NiDX8TexturePass::stub_returns0`
pub unsafe fn texture_pass_stub_returns_0(_this: usize, _a: u32, _b: u32, _c: u32, _d: u32, _e: u32, _f: u32, _g: u32, _h: u32) -> u32 {
    crate::shadow::touch(&texture_pass_stub_returns_0_port::STATE);
    0
}

/// 0x6B77C0 `NiDX8TexturePass::$1::deleting_dtor`
pub unsafe fn subpass_deleting_dtor(this: usize, flags: u8) -> usize {
    crate::shadow::touch(&subpass_deleting_dtor_port::STATE);
    deleting(this, flags, texture_pass_b::subpass_dtor)
}

/// 0x6B8D80 `NiDX8ConfigurableTexturePipeline::deleting_dtor`
pub unsafe fn pipeline_deleting_dtor(this: usize, flags: u8) -> usize {
    crate::shadow::touch(&pipeline_deleting_dtor_port::STATE);
    deleting(this, flags, texture_pipeline::dtor)
}

/// 0x6B9E70 `NiDX8ConfigurableTexturePipeline::applyPixelShader`. It is a jump to the function
/// of the texture pass.
pub unsafe fn pipeline_apply_pixel_shader(this: usize) -> u32 {
    crate::shadow::touch(&pipeline_apply_pixel_shader_port::STATE);
    texture_pass_a::apply_pixel_shader(rd(this, PIPELINE_TEXTURE_PASS))
}

/// 0x6B9E80 `NiDX8ConfigurableTexturePipeline::applyVertexShader`. It is a jump to the function
/// of the texture pass.
pub unsafe fn pipeline_apply_vertex_shader(this: usize, handle: u32) -> u32 {
    crate::shadow::touch(&pipeline_apply_vertex_shader_port::STATE);
    texture_pass_a::apply_vertex_shader(rd(this, PIPELINE_TEXTURE_PASS), handle)
}

/// The same as the two functions of the texture pass. They write the render state. The call
/// is never a skip.
unsafe fn pipeline_apply_shader_regions(set: &mut Regions, this: usize) {
    let texture_pass: usize = rd(this, PIPELINE_TEXTURE_PASS);
    set.add(rd(texture_pass, TP::renderState), RS::SIZE);
}

/// 0x6BAEA0 `NiDX8LightManager::deleting_dtor`
pub unsafe fn light_manager_deleting_dtor(this: usize, flags: u8) -> usize {
    crate::shadow::touch(&light_manager_deleting_dtor_port::STATE);
    deleting(this, flags, light_manager::dtor)
}

/// 0x6C1A50 `NiDX8TextureStage::deleting_dtor`
pub unsafe fn texture_stage_deleting_dtor(this: usize, flags: u8) -> usize {
    crate::shadow::touch(&texture_stage_deleting_dtor_port::STATE);
    deleting(this, flags, texture_pipeline::texture_stage_dtor)
}

/// The same as the destructor of the stage. The call is a skip when the release deletes the
/// texture. A virtual deleting destructor cannot run two times.
unsafe fn texture_stage_deleting_dtor_regions(set: &mut Regions, this: usize, flags: u8) {
    set.add(this, TS::SIZE);
    set.add(NI_REF_OBJECT_COUNT, 4);
    let texture: usize = rd(this, TS::ptrTexture);
    if texture != 0 {
        if rd::<u32>(texture, REF_COUNT) == 1 {
            return set.skip();
        }
        set.add(texture, 8);
    }
    freed(set, flags);
}

/// A deleting destructor frees the object when bit 0 of the flags is set. The call is a skip
/// then when the allocator hooks are not active.
fn freed(set: &mut Regions, flags: u8) {
    if flags & 1 != 0 {
        set.allocates();
    }
}

// ---------------------------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------------------------

port_thiscall!(renderer_ctor_port, 0x6F4E30, "NiRenderer::ctor",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| renderer_ctor_regions(set, this),
    rust = renderer_ctor);

port_thiscall!(renderer_deleting_dtor_port, 0x6F4F60, "NiRenderer::deleting_dtor",
    fn(this, flags: u8) -> usize,
    compare_eax = true,
    regions = |set| {
        renderer_dtor_regions(set, this);
        freed(set, flags)
    },
    rust = renderer_deleting_dtor);

port_thiscall!(renderer_dtor_port, 0x6F4F80, "NiRenderer::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| renderer_dtor_regions(set, this),
    rust = renderer_dtor);

// The call is a skip when there is an accumulator. The accumulator keeps the geometry in memory
// of its own class.
port_thiscall!(renderer_register_object_port, 0x6F5070, "NiRenderer::RegisterObject",
    fn(this, geometry: usize) -> u8,
    compare_eax = true,
    regions = |set| if rd::<usize>(this, R::accumulator) != 0 { set.skip() },
    rust = renderer_register_object);

port_cdecl!(renderer_remove_effect_port, 0x6F5090, "NiRenderer_static::RemoveEffect",
    fn(effect: usize) -> (),
    compare_eax = false,
    regions = |set| for_each_renderer_regions(set),
    rust = renderer_remove_effect);

port_cdecl!(renderer_remove_geometry_data_port, 0x6F50C0, "NiRenderer_static::RemoveGeometryData",
    fn(data: usize) -> (),
    compare_eax = false,
    regions = |set| for_each_renderer_regions(set),
    rust = renderer_remove_geometry_data);

port_cdecl!(renderer_remove_skin_partition_port, 0x6F50F0, "NiRenderer_static::RemoveSkinPartitionRendererData",
    fn(partition: usize) -> (),
    compare_eax = false,
    regions = |set| for_each_renderer_regions(set),
    rust = renderer_remove_skin_partition);

port_cdecl!(renderer_remove_material_port, 0x6F5120, "NiRenderer_static::RemoveMaterial",
    fn(material: usize) -> (),
    compare_eax = false,
    regions = |set| for_each_renderer_regions(set),
    rust = renderer_remove_material);

port_thiscall!(renderer_map_window_point_to_buffer_point_port, 0x6F5150, "NiRenderer::MapWindowPointToBufferPoint",
    fn(this, x: u32, y: u32, out_x: usize, out_y: usize) -> u8,
    compare_eax = true,
    regions = |set| {
        set.add(out_x, 4);
        set.add(out_y, 4)
    },
    rust = renderer_map_window_point_to_buffer_point);

// The call is always a skip. The function displays a scene object. The objects below the
// scene object write memory that no list of regions can name.
port_thiscall!(renderer_render_from_cluster_accumulator_port, 0x6F51D0, "NiRenderer::renderFromClusterAccumulator",
    fn(this, camera: usize, scene_object: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = renderer_render_from_cluster_accumulator);

port_thiscall!(renderer_begin_batch_port, 0x6F5380, "NiRenderer::BeginBatch",
    fn(this, property_state: usize, effect_state: usize) -> (),
    compare_eax = false,
    regions = |set| renderer_begin_batch_regions(set, this, property_state, effect_state),
    rust = renderer_begin_batch);

port_thiscall!(renderer_add_viewer_strings_port, 0x6F5400, "NiRenderer::AddViewerStrings",
    fn(this, strings: usize) -> (),
    compare_eax = false,
    regions = |set| array_regions(set, strings),
    rust = renderer_add_viewer_strings);

port_thiscall!(renderer_get_rtti_port, 0x6F5630, "NiRenderer::GetRTTI",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| (),
    rust = renderer_get_rtti);

port_thiscall!(renderer_set_use_w_buffer_port, 0x6F5640, "NiRenderer::SetUseWBuffer",
    fn(this, use_w_buffer: u8) -> u32,
    compare_eax = true,
    regions = |set| (),
    rust = renderer_set_use_w_buffer);

port_thiscall!(renderer_stub_returns_true_port, 0x6F5660, "NiRenderer::stub_returnsTrue",
    fn(this, a: u32) -> u8,
    compare_eax = true,
    regions = |set| (),
    rust = renderer_stub_returns_true);

port_thiscall!(renderer_precache_geometry_port, 0x6F5680, "NiRenderer::PrecacheGeometry",
    fn(this, geometry: usize, flags: u32, property: usize, bones_per_partition: u32, bones_per_vertex: u32) -> u8,
    compare_eax = true,
    regions = |set| (),
    rust = renderer_precache_geometry);

port_thiscall!(renderer_stub_returns_800000_port, 0x6F5690, "NiRenderer::stub_returns800000",
    fn(this, a: u32, b: u32) -> u32,
    compare_eax = true,
    regions = |set| (),
    rust = renderer_stub_returns_800000);

port_thiscall!(renderer_get_texture_memory_stats_port, 0x6F56A0, "NiRenderer::GetTextureMemoryStats",
    fn(this, a: u32, b: u32) -> u8,
    compare_eax = true,
    regions = |set| (),
    rust = renderer_get_texture_memory_stats);

port_thiscall!(renderer_get_texture_stats_port, 0x6A9C40, "NiRenderer::GetTextureStats",
    fn(this, a: u32, b: u32, c: u32, d: u32, e: u32, f: u32) -> u8,
    compare_eax = true,
    regions = |set| (),
    rust = renderer_get_texture_stats);

port_thiscall!(renderer_constant_false_8_port, 0x6A9C60, "NiRenderer::constantFalse@8",
    fn(this, a: u32, b: u32) -> u32,
    compare_eax = true,
    regions = |set| (),
    rust = renderer_constant_false_8);

port_thiscall!(render_data_7_deleting_dtor_port, 0x65F560, "NiRendererSpecificProperty::RenderData7::deleting_dtor",
    fn(this, flags: u8) -> usize,
    compare_eax = true,
    regions = |set| {
        render_data_7_dtor_regions(set, this);
        freed(set, flags)
    },
    rust = render_data_7_deleting_dtor);

port_thiscall!(render_data_base_deleting_dtor_port, 0x65F580, "NiRendererSpecificProperty::RenderDataBase::deleting_dtor",
    fn(this, flags: u8) -> usize,
    compare_eax = true,
    regions = |set| {
        set.add(this, 4);
        freed(set, flags)
    },
    rust = render_data_base_deleting_dtor);

port_thiscall!(render_data_7_dtor_port, 0x65F5A0, "NiRendererSpecificProperty::RenderData7::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| render_data_7_dtor_regions(set, this),
    rust = render_data_7_dtor);

port_thiscall!(render_data_base_dtor_port, 0x6DC7F0, "NiRendererSpecificProperty::RenderDataBase::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| set.add(this, 4),
    rust = render_data_base_dtor);

port_thiscall!(property_ctor_port, 0x6DC800, "NiRendererSpecificProperty::ctor",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| property_ctor_regions(set, this),
    rust = property_ctor);

port_thiscall!(property_deleting_dtor_port, 0x6DC820, "NiRendererSpecificProperty::deleting_dtor",
    fn(this, flags: u8) -> usize,
    compare_eax = true,
    regions = |set| {
        property_dtor_regions(set, this);
        freed(set, flags)
    },
    rust = property_deleting_dtor);

port_thiscall!(property_dtor_port, 0x6DC840, "NiRendererSpecificProperty::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| property_dtor_regions(set, this),
    rust = property_dtor);

port_thiscall!(property_create_clone_port, 0x6DC8A0, "NiRendererSpecificProperty::CreateClone",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| property_create_clone_regions(set, this),
    rust = property_create_clone);

// The call is always a skip. The function reads the property from the stream, and a second
// read of the stream is not possible.
port_cdecl!(property_create_from_stream_port, 0x6DC920, "NiRendererSpecificProperty_static::CreateFromStream",
    fn(stream: usize, out: usize) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = property_create_from_stream);

// The call is always a skip. `NiObjectNET::RegisterStreamables` puts the object and the
// things that it holds into tables of the stream.
port_thiscall!(property_register_streamables_port, 0x6DCAE0, "NiRendererSpecificProperty::RegisterStreamables",
    fn(this, stream: usize) -> u8,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = property_register_streamables);

port_thiscall!(property_is_equal_port, 0x6DCB00, "NiRendererSpecificProperty::IsEqual",
    fn(this, other: usize) -> u8,
    compare_eax = true,
    regions = |set| (),
    rust = property_is_equal);

port_thiscall!(property_add_viewer_strings_port, 0x6DCB20, "NiRendererSpecificProperty::AddViewerStrings",
    fn(this, strings: usize) -> (),
    compare_eax = false,
    regions = |set| array_regions(set, strings),
    rust = property_add_viewer_strings);

port_thiscall!(property_get_rtti_port, 0x6DCCA0, "NiRendererSpecificProperty::GetRTTI",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| (),
    rust = property_get_rtti);

port_thiscall!(property_get_type_port, 0x6DCCD0, "NiRendererSpecificProperty::GetType",
    fn(this) -> u32,
    compare_eax = true,
    regions = |set| (),
    rust = property_get_type);

// The call is always a skip. The constructor takes a reference to the device, and that cannot
// occur two times.
port_thiscall!(render_state_ctor_port, 0x6B7D70, "NiDX8RenderState::ctor",
    fn(this, device: usize, caps: usize) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = render_state_ctor);

port_thiscall!(render_state_deleting_dtor_port, 0x6B80B0, "NiDX8RenderState::deleting_dtor",
    fn(this, flags: u8) -> usize,
    compare_eax = true,
    regions = |set| {
        render_state_dtor_regions(set, this);
        freed(set, flags)
    },
    rust = render_state_deleting_dtor);

port_thiscall!(render_state_dtor_port, 0x6B80D0, "NiDX8RenderState::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| render_state_dtor_regions(set, this),
    rust = render_state_dtor);

// The call is always a skip. The function runs the constructor, and the constructor takes a
// reference to the device.
port_cdecl!(render_state_create_port, 0x6B81E0, "NiDX8RenderState_static::Create",
    fn(device: usize, caps: usize, flag_2: u8, flag_8: u8, flag_4: u8) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = render_state_create);

port_thiscall!(render_state_purge_material_port, 0x6B8600, "NiDX8RenderState::PurgeMaterial",
    fn(this, material: usize) -> (),
    compare_eax = false,
    regions = |set| material_manager_purge_material_regions(set, material),
    rust = render_state_purge_material);

port_thiscall!(render_state_initialize_port, 0x6B8680, "NiDX8RenderState::Initialize_maybe",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| render_state_initialize_regions(set, this),
    rust = render_state_initialize);

port_thiscall!(render_state_init_from_device_caps_port, 0x6B8930, "NiDX8RenderState::initFromDeviceCaps",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| render_state_init_from_device_caps_regions(set, this),
    rust = render_state_init_from_device_caps);

port_thiscall!(material_manager_ctor_port, 0x6C22C0, "NiDX8MaterialManager::ctor",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| material_manager_ctor_regions(set, this),
    rust = material_manager_ctor);

port_thiscall!(material_manager_deleting_dtor_port, 0x6C2350, "NiDX8MaterialManager::deleting_dtor",
    fn(this, flags: u8) -> usize,
    compare_eax = true,
    regions = |set| {
        material_manager_dtor_regions(set, this);
        freed(set, flags)
    },
    rust = material_manager_deleting_dtor);

port_thiscall!(material_manager_initialize_port, 0x6C2370, "NiDX8MaterialManager::Initialize_maybe",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| material_manager_initialize_regions(set, this),
    rust = material_manager_initialize);

port_thiscall!(material_manager_dtor_port, 0x6C2450, "NiDX8MaterialManager::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| material_manager_dtor_regions(set, this),
    rust = material_manager_dtor);

port_thiscall!(material_manager_purge_material_port, 0x6C26A0, "NiDX8MaterialManager::PurgeMaterial",
    fn(this, material: usize) -> (),
    compare_eax = false,
    regions = |set| material_manager_purge_material_regions(set, material),
    rust = material_manager_purge_material);

// The destructors of the texture pass, of its subpass, of the pipeline and of the light
// manager release device objects. They also delete other objects. Thus a call of one of their
// deleting destructors is always a skip.
port_thiscall!(texture_pass_deleting_dtor_port, 0x6B3380, "NiDX8TexturePass::deleting_dtor",
    fn(this, flags: u8) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = texture_pass_deleting_dtor);

port_thiscall!(texture_pass_stub_returns_0_2_port, 0x6B41A0, "NiDX8TexturePass::stub_returns0_2",
    fn(this, a: u32, b: u32, c: u32, d: u32, e: u32, f: u32, g: u32, h: u32, i: u32) -> u32,
    compare_eax = true,
    regions = |set| (),
    rust = texture_pass_stub_returns_0_2);

port_thiscall!(texture_pass_stub_returns_0_port, 0x6B56A0, "NiDX8TexturePass::stub_returns0",
    fn(this, a: u32, b: u32, c: u32, d: u32, e: u32, f: u32, g: u32, h: u32) -> u32,
    compare_eax = true,
    regions = |set| (),
    rust = texture_pass_stub_returns_0);

port_thiscall!(subpass_deleting_dtor_port, 0x6B77C0, "NiDX8TexturePass::$1::deleting_dtor",
    fn(this, flags: u8) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = subpass_deleting_dtor);

port_thiscall!(pipeline_deleting_dtor_port, 0x6B8D80, "NiDX8ConfigurableTexturePipeline::deleting_dtor",
    fn(this, flags: u8) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = pipeline_deleting_dtor);

port_thiscall!(pipeline_apply_pixel_shader_port, 0x6B9E70, "NiDX8ConfigurableTexturePipeline::applyPixelShader",
    fn(this) -> u32,
    compare_eax = true,
    regions = |set| pipeline_apply_shader_regions(set, this),
    rust = pipeline_apply_pixel_shader);

port_thiscall!(pipeline_apply_vertex_shader_port, 0x6B9E80, "NiDX8ConfigurableTexturePipeline::applyVertexShader",
    fn(this, handle: u32) -> u32,
    compare_eax = true,
    regions = |set| pipeline_apply_shader_regions(set, this),
    rust = pipeline_apply_vertex_shader);

port_thiscall!(light_manager_deleting_dtor_port, 0x6BAEA0, "NiDX8LightManager::deleting_dtor",
    fn(this, flags: u8) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = light_manager_deleting_dtor);

port_thiscall!(texture_stage_deleting_dtor_port, 0x6C1A50, "NiDX8TextureStage::deleting_dtor",
    fn(this, flags: u8) -> usize,
    compare_eax = true,
    regions = |set| texture_stage_deleting_dtor_regions(set, this, flags),
    rust = texture_stage_deleting_dtor);

pub static PORTS: &[&crate::shadow::PortState] = &[
    &renderer_ctor_port::STATE,
    &renderer_deleting_dtor_port::STATE,
    &renderer_dtor_port::STATE,
    &renderer_register_object_port::STATE,
    &renderer_remove_effect_port::STATE,
    &renderer_remove_geometry_data_port::STATE,
    &renderer_remove_skin_partition_port::STATE,
    &renderer_remove_material_port::STATE,
    &renderer_map_window_point_to_buffer_point_port::STATE,
    &renderer_render_from_cluster_accumulator_port::STATE,
    &renderer_begin_batch_port::STATE,
    &renderer_add_viewer_strings_port::STATE,
    &renderer_get_rtti_port::STATE,
    &renderer_set_use_w_buffer_port::STATE,
    &renderer_stub_returns_true_port::STATE,
    &renderer_precache_geometry_port::STATE,
    &renderer_stub_returns_800000_port::STATE,
    &renderer_get_texture_memory_stats_port::STATE,
    &renderer_get_texture_stats_port::STATE,
    &renderer_constant_false_8_port::STATE,
    &render_data_7_deleting_dtor_port::STATE,
    &render_data_base_deleting_dtor_port::STATE,
    &render_data_7_dtor_port::STATE,
    &render_data_base_dtor_port::STATE,
    &property_ctor_port::STATE,
    &property_deleting_dtor_port::STATE,
    &property_dtor_port::STATE,
    &property_create_clone_port::STATE,
    &property_create_from_stream_port::STATE,
    &property_register_streamables_port::STATE,
    &property_is_equal_port::STATE,
    &property_add_viewer_strings_port::STATE,
    &property_get_rtti_port::STATE,
    &property_get_type_port::STATE,
    &render_state_ctor_port::STATE,
    &render_state_deleting_dtor_port::STATE,
    &render_state_dtor_port::STATE,
    &render_state_create_port::STATE,
    &render_state_purge_material_port::STATE,
    &render_state_initialize_port::STATE,
    &render_state_init_from_device_caps_port::STATE,
    &material_manager_ctor_port::STATE,
    &material_manager_deleting_dtor_port::STATE,
    &material_manager_initialize_port::STATE,
    &material_manager_dtor_port::STATE,
    &material_manager_purge_material_port::STATE,
    &texture_pass_deleting_dtor_port::STATE,
    &texture_pass_stub_returns_0_2_port::STATE,
    &texture_pass_stub_returns_0_port::STATE,
    &subpass_deleting_dtor_port::STATE,
    &pipeline_deleting_dtor_port::STATE,
    &pipeline_apply_pixel_shader_port::STATE,
    &pipeline_apply_vertex_shader_port::STATE,
    &light_manager_deleting_dtor_port::STATE,
    &texture_stage_deleting_dtor_port::STATE,
];

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicU32, Ordering};

    /// A vtable of `slots` entries with `function` in each entry.
    fn vtable_of(function: usize, slots: usize) -> Vec<usize> {
        vec![function; slots]
    }

    static DELETED: AtomicU32 = AtomicU32::new(0);
    static DELETED_BY_RENDER_DATA: AtomicU32 = AtomicU32::new(0);

    unsafe extern "thiscall" fn count_delete(_this: usize, flags: u32) -> u32 {
        DELETED.fetch_add(flags, Ordering::Relaxed);
        0
    }

    unsafe extern "thiscall" fn count_delete_by_render_data(_this: usize, flags: u32) -> u32 {
        DELETED_BY_RENDER_DATA.fetch_add(flags, Ordering::Relaxed);
        0
    }

    /// An object with a vtable and a reference count.
    fn object(vtable: &[usize], references: u32) -> [usize; 2] {
        [vtable.as_ptr() as usize, references as usize]
    }

    #[test]
    fn stubs_return_what_the_game_code_leaves() {
        unsafe {
            assert_eq!(renderer_set_use_w_buffer(0, 0), 1);
            assert_eq!(renderer_set_use_w_buffer(0, 1), 0);
            assert_eq!(renderer_stub_returns_true(0, 0), 1);
            assert_eq!(renderer_precache_geometry(0, 0, 0, 0, 0, 0), 0);
            assert_eq!(renderer_stub_returns_800000(0, 0, 0), 0x80_0000);
            assert_eq!(renderer_get_texture_memory_stats(0, 0, 0), 0);
            assert_eq!(renderer_get_texture_stats(0, 0, 0, 0, 0, 0, 0), 0);
            assert_eq!(renderer_constant_false_8(0, 0, 0), 0);
            assert_eq!(texture_pass_stub_returns_0_2(0, 0, 0, 0, 0, 0, 0, 0, 0, 0), 0);
            assert_eq!(texture_pass_stub_returns_0(0, 0, 0, 0, 0, 0, 0, 0, 0), 0);
            assert_eq!(property_get_type(0), 11);
            assert_eq!(renderer_get_rtti(0), 0x7DEA9C);
            assert_eq!(property_get_rtti(0), 0x7DE2F0);
        }
    }

    #[test]
    fn offsets_the_disassembly_has_as_numbers() {
        assert_eq!(R::accumulator, 0x08);
        assert_eq!(R::currentPropertyState, 0x0C);
        assert_eq!(R::currentEffectState, 0x10);
        assert_eq!(MP::rendererData, 0x18);
        assert_eq!(RSP::flags, 0x14);
        assert_eq!(RSP::renderData, 0x18);
        assert_eq!((RD7::texturePass, RD7::field_8, RD7::field_C), (4, 8, 12));
        assert_eq!(MM::hashmap_8 + HM::bucketCount, 0x10);
        assert_eq!(MM::hashmap_8 + HM::arrayBuckets, 0x14);
        assert_eq!(MM::currentMaterialEntry, 0x18);
        assert_eq!(RS::materialManager, 0x1B4);
        assert_eq!(RS::deviceCapabilities + D3DCAPS8::RasterCaps, 0x30);
        assert_eq!(RS::deviceCapabilities + D3DCAPS8::DevCaps, 0x28);
        assert_eq!(RS::deviceCapabilities + D3DCAPS8::SrcBlendCaps, 0x38);
        assert_eq!(RS::deviceCapabilities + D3DCAPS8::DestBlendCaps, 0x3C);
        assert_eq!(D3DCAPS8::SIZE / 4, 0x35);
        assert_eq!(RS::alphaMapping + 6 * 4, 0xFC);
        assert_eq!(RS::texturePerStage + 7 * 4, 0x1224);
        assert_eq!(PIPELINE_TEXTURE_PASS, 8);
        assert_eq!(TS::ptrTexture, 0x0C);
    }

    #[test]
    fn mapping_tables_hold_the_d3d_values() {
        let mut block = vec![0xAAu8; RS::SIZE];
        let this = block.as_mut_ptr() as usize;
        let table = |offset: usize, len: usize| -> Vec<u32> { (0..len).map(|i| unsafe { rd(this, offset + i * 4) }).collect() };
        unsafe {
            ctor_alpha_mapping(this);
            ctor_mapping_tables(this);
        }
        // D3DBLEND: ONE, ZERO, SRCCOLOR, INVSRCCOLOR, DESTCOLOR, INVDESTCOLOR, SRCALPHA,
        // INVSRCALPHA, DESTALPHA, INVDESTALPHA, SRCALPHASAT
        assert_eq!(table(RS::alphaMapping, 11), [2, 1, 3, 4, 9, 10, 5, 6, 7, 8, 11]);
        // D3DCMP: ALWAYS, LESS, EQUAL, LESSEQUAL, GREATER, NOTEQUAL, GREATEREQUAL, NEVER
        assert_eq!(table(RS::alphaTestMapping, 8), [8, 2, 3, 4, 5, 6, 7, 1]);
        assert_eq!(table(RS::zFuncMapping, 8), [8, 2, 3, 4, 5, 6, 7, 1]);
        // D3DCMP: NEVER, LESS, EQUAL, LESSEQUAL, GREATER, NOTEQUAL, GREATEREQUAL, ALWAYS
        assert_eq!(table(RS::stencilFuncMapping, 8), [1, 2, 3, 4, 5, 6, 7, 8]);
        // D3DSTENCILOP: KEEP, ZERO, REPLACE, INCRSAT, DECRSAT, INVERT
        assert_eq!(table(RS::stencilActionMapping, 6), [1, 2, 3, 4, 5, 6]);
        // D3DCULL for each draw mode, without and with the left-right swap: CW, CCW; CW, CCW;
        // CCW, CW; NONE, NONE
        assert_eq!(table(RS::cullModeMapping, 8), [2, 3, 2, 3, 3, 2, 1, 1]);
        assert_eq!(table(RS::sourceBlendCaps, 3), [0, 0, 0]); // the caps and the current alpha property
        assert_eq!(table(RS::frustumNear, 6), [0, 0, 0, 0, 0x3F80_0000, 0]);
        assert_eq!(table(RS::bLeftRightSwap, 1), [0]);
        // The constructor writes every entry of the tables, and one time only.
        let mut offsets: Vec<usize> = MAPPING_WRITES.iter().map(|w| w.0).collect();
        offsets.sort();
        offsets.dedup();
        assert_eq!(offsets.len(), MAPPING_WRITES.len());
        // The members between the tables and after the tables do not change.
        assert_eq!(unsafe { rd::<u32>(this, RS::alphaProperty) }, 0xAAAA_AAAA);
        assert_eq!(unsafe { rd::<u32>(this, RS::materialManager) }, 0xAAAA_AAAA);
        assert_eq!(unsafe { rd::<u8>(this, RS::field_1F4) }, 0xAA);
    }

    #[test]
    fn init_from_device_caps_sets_the_flags_and_the_blend_caps() {
        let mut block = vec![0u8; RS::SIZE];
        let this = block.as_mut_ptr() as usize;
        unsafe {
            ctor_alpha_mapping(this);
            let caps = this + RS::deviceCapabilities;
            // Table fog, W fog and a W buffer. No hardware transform, because that writes a global.
            wr::<u32>(caps, D3DCAPS8::RasterCaps, 0x100 | 0x10_0000 | 0x4_0000);
            // D3DPBLENDCAPS_ZERO | ONE | SRCALPHA | INVSRCALPHA
            wr::<u32>(caps, D3DCAPS8::SrcBlendCaps, 1 | 2 | 0x10 | 0x20);
            wr::<u32>(caps, D3DCAPS8::DestBlendCaps, 1);
            wr::<u32>(this, RS::flags, 0x8);
            render_state_init_from_device_caps(this);
            assert_eq!(rd::<u32>(this, RS::flags), 0x8 | 1 | 0x10);
            // The game code tests the D3DBLEND value against the caps as bits. ONE (2) finds the
            // bit of ONE. ZERO (1) finds the bit of ZERO. SRCCOLOR (3) finds the two bits. The
            // other values follow the same rule.
            let expected = |caps: u32| (0..11).filter(|&i| ALPHA_MAPPING[i] & caps != 0).fold(0u32, |bits, i| bits | 1 << i);
            assert_eq!(rd::<u32>(this, RS::sourceBlendCaps), expected(0x33));
            assert_eq!(rd::<u32>(this, RS::destBlendCaps), expected(1));
            assert_eq!(expected(1), 0b101_0101_0110);

            wr::<u32>(caps, D3DCAPS8::RasterCaps, 0x100);
            render_state_init_from_device_caps(this);
            assert_eq!(rd::<u32>(this, RS::flags), 0x8);
        }
    }

    #[test]
    fn begin_batch_moves_the_references() {
        let vtable = vtable_of(count_delete as *const () as usize, 1);
        let mut old_property = object(&vtable, 1);
        let mut old_effect = object(&vtable, 2);
        let mut new_property = object(&vtable, 5);
        let mut renderer = [0u8; R::SIZE];
        let this = renderer.as_mut_ptr() as usize;
        let (op, oe, np) = (old_property.as_mut_ptr() as usize, old_effect.as_mut_ptr() as usize, new_property.as_mut_ptr() as usize);
        unsafe {
            wr(this, R::currentPropertyState, op);
            wr(this, R::currentEffectState, oe);
            renderer_begin_batch(this, np, oe);
            assert_eq!(rd::<usize>(this, R::currentPropertyState), np);
            assert_eq!(rd::<usize>(this, R::currentEffectState), oe);
            assert_eq!(rd::<u32>(np, REF_COUNT), 6);
            assert_eq!(rd::<u32>(oe, REF_COUNT), 2);
            assert_eq!(rd::<u32>(op, REF_COUNT), 0);
            assert_eq!(DELETED.load(Ordering::Relaxed), 1);
            renderer_begin_batch(this, np, 0);
            assert_eq!(rd::<usize>(this, R::currentEffectState), 0);
            assert_eq!(rd::<u32>(oe, REF_COUNT), 1);
            assert_eq!(DELETED.load(Ordering::Relaxed), 1);
        }
    }

    unsafe extern "thiscall" fn register_low_byte(_this: usize, geometry: u32) -> u32 {
        // Only AL is the result. The other bytes of EAX are the values that the function left there.
        0xFFFF_FF00 | geometry
    }

    #[test]
    fn register_object_returns_a_clean_bool() {
        let vtable = vtable_of(register_low_byte as *const () as usize, SLOT_ACCUMULATOR_REGISTER_OBJECT / 4 + 1);
        let mut accumulator = object(&vtable, 1);
        let mut renderer = [0u8; R::SIZE];
        let this = renderer.as_mut_ptr() as usize;
        unsafe {
            assert_eq!(renderer_register_object(this, 7), 0);
            wr(this, R::accumulator, accumulator.as_mut_ptr() as usize);
            assert_eq!(renderer_register_object(this, 7), 1);
            assert_eq!(renderer_register_object(this, 0x100), 0);
        }
    }

    unsafe extern "thiscall" fn backbuffer_size(this: usize) -> u32 {
        rd(this, 4)
    }

    #[test]
    fn map_window_point_divides_by_the_backbuffer() {
        let vtable = vtable_of(backbuffer_size as *const () as usize, SLOT_GET_BACKBUFFER_HEIGHT / 4 + 1);
        let mut renderer = [vtable.as_ptr() as usize, 800];
        let this = renderer.as_mut_ptr() as usize;
        let (mut x, mut y) = (9.0f32, 9.0f32);
        let (out_x, out_y) = (core::ptr::addr_of_mut!(x) as usize, core::ptr::addr_of_mut!(y) as usize);
        unsafe {
            assert_eq!(renderer_map_window_point_to_buffer_point(this, 200, 600, out_x, out_y), 1);
            assert_eq!((x, y), (0.25, 0.25));
            wr::<usize>(this, 4, 0);
            assert_eq!(renderer_map_window_point_to_buffer_point(this, 1, 1, out_x, out_y), 0);
            assert_eq!((x, y), (0.25, 0.25));
        }
        // The point is unsigned and the size is signed.
        let (bx, by) = window_to_buffer(0x8000_0000, 0x4000_0000, 3, 0xFFFF_FFFE, 1.0f32.to_bits());
        assert_eq!((f32::from_bits(bx), f32::from_bits(by)), (2.0, 2.5));
    }

    fn array(storage: &mut [usize], end: u32, filled: u32, grow: u32) -> [u32; 6] {
        [0, storage.as_mut_ptr() as u32, storage.len() as u32, end, filled, grow]
    }

    #[test]
    fn array_add_puts_the_element_at_the_end() {
        let mut storage = [0usize; 4];
        let mut strings = array(&mut storage, 1, 1, 2);
        let a = strings.as_mut_ptr() as usize;
        unsafe {
            array_add(a, 0x1234);
            assert_eq!(storage, [0, 0x1234, 0, 0]);
            assert_eq!((rd::<u32>(a, ARRAY_END_INDEX), rd::<u32>(a, ARRAY_FILLED_COUNT)), (2, 2));
            // A null element moves the end and does not change the count.
            array_add(a, 0);
            assert_eq!((rd::<u32>(a, ARRAY_END_INDEX), rd::<u32>(a, ARRAY_FILLED_COUNT)), (3, 2));
        }
    }

    #[test]
    fn array_add_grows_a_full_array() {
        let mut storage = [0x11usize, 0x22];
        let mut strings = array(&mut storage, 2, 2, 3);
        let a = strings.as_mut_ptr() as usize;
        unsafe {
            array_add(a, 0x33);
            let new: usize = rd(a, ARRAY_STORAGE);
            assert_ne!(new, storage.as_ptr() as usize);
            assert_eq!(rd::<u32>(a, ARRAY_STORAGE_COUNT), 5);
            assert_eq!(rd::<[usize; 3]>(new, 0), [0x11, 0x22, 0x33]);
            assert_eq!((rd::<u32>(a, ARRAY_END_INDEX), rd::<u32>(a, ARRAY_FILLED_COUNT)), (3, 3));
        }
        // There is no free space and no growth, so the function adds nothing.
        let mut storage = [0x11usize];
        let mut strings = array(&mut storage, 1, 1, 0);
        let a = strings.as_mut_ptr() as usize;
        unsafe {
            array_add(a, 0x33);
            assert_eq!(rd::<usize>(a, ARRAY_STORAGE), storage.as_ptr() as usize);
            assert_eq!(strings[3], 1);
        }
    }

    #[test]
    fn render_data_7_dtor_lets_go_of_each_object_once() {
        let vtable = vtable_of(count_delete_by_render_data as *const () as usize, 1);
        let mut pass = object(&vtable, 1);
        let mut other = object(&vtable, 3);
        let mut data = [0usize; 4];
        let this = data.as_mut_ptr() as usize;
        unsafe {
            wr(this, RD7::texturePass, pass.as_mut_ptr() as usize);
            wr(this, RD7::field_C, other.as_mut_ptr() as usize);
            assert_eq!(render_data_7_deleting_dtor(this, 0), this);
        }
        assert_eq!(data, [VTBL_RENDER_DATA_BASE, 0, 0, 0]);
        assert_eq!((pass[1], other[1]), (0, 2));
        assert_eq!(DELETED_BY_RENDER_DATA.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn purge_material_takes_a_null_material() {
        let mut material = [0u8; MP::SIZE];
        unsafe {
            material_manager_purge_material(0, 0);
            material_manager_purge_material(0, material.as_mut_ptr() as usize);
            render_state_purge_material(0x1000, 0);
        }
    }

    #[test]
    fn is_equal_and_register_streamables_make_a_bool_of_al() {
        unsafe {
            // The replacements return the argument. Only the low byte is the result.
            assert_eq!(property_is_equal(0, 0x200), 0);
            assert_eq!(property_is_equal(0, 0x203), 1);
            assert_eq!(property_register_streamables(0, 0x100), 0);
            assert_eq!(property_register_streamables(0, 1), 1);
        }
    }

    #[test]
    fn property_ctor_sets_the_members() {
        let mut block = [0xFFu8; RSP::SIZE];
        let this = block.as_mut_ptr() as usize;
        unsafe {
            assert_eq!(property_ctor(this), this);
            assert_eq!(rd::<usize>(this, RSP::vtbl), 0x750410);
            assert_eq!(rd::<u16>(this, RSP::flags), 0);
            assert_eq!(rd::<u16>(this, RSP::flags + 2), 0xFFFF);
            assert_eq!(rd::<usize>(this, RSP::renderData), 0);
        }
    }
}
