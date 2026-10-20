//! `NiDX8Renderer`: the draw functions.
//!
//! Other mods, for example MGE XE, change the target of `call` instructions in these
//! functions in memory. Thus each direct call here reads its target from the call instruction
//! in memory (`site::call_target`). This is also true when the callee has a port: the call
//! goes through the detour of the port. `SITES` lists the call instructions.

use core::arch::asm;
use core::mem::transmute;

use crate::main_thread::MainThread;
use crate::d3d::{
    device_draw_indexed_primitive, device_draw_primitive, device_set_indices, device_set_stream_source, D3DFVF_NORMAL,
    D3DPT_TRIANGLELIST, D3DPT_TRIANGLESTRIP, D3DRS_VERTEXBLEND, DEVICE_DRAW_INDEXED_PRIMITIVE, DEVICE_DRAW_PRIMITIVE,
};
use crate::engine::{device_set_render_state, vcall0};
use crate::gen::layouts::{
    NiDX8ConfigurableTexturePipeline as CTP, NiDX8RenderState as RS, NiDX8Renderer as R, NiDX8Renderer_BatchedObject as B,
    NiGeometryBufferData as GB, NiGeometryData as GD, NiPropertyState as PS, NiRefObject, NiRendererSpecificProperty,
    NiSkinInstance, NiSkinPartition, NiSkinPartition_Partition as P, NiTriStripsData,
};
use crate::mem::{rd, wr};
use crate::ports::ni;
use crate::shadow::Regions;
use crate::site::{site_cdecl, site_thiscall};

// ---------------------------------------------------------------------------------------
// Call sites: the address of each `call rel32` and the callee the database has there
// ---------------------------------------------------------------------------------------

// RenderShape
const RS_DRAW_SKINNED_PRIMITIVE: usize = 0x6ACF36; // NiDX8Renderer::DrawSkinnedPrimitive
const RS_CREATE_GEOMETRY_BUFFER_DATA: usize = 0x6ACF51; // NiDX8Renderer::createGeometryBufferData
const RS_DRAW_PRIMITIVE: usize = 0x6ACFB1; // NiDX8Renderer::DrawPrimitive

// RenderTriStrips
const RT_DRAW_SKINNED_PRIMITIVE: usize = 0x6AD006; // NiDX8Renderer::DrawSkinnedPrimitive
const RT_CREATE_GEOMETRY_BUFFER_DATA: usize = 0x6AD021; // NiDX8Renderer::createGeometryBufferData
const RT_GET_STRIP_LENGTH_SUM: usize = 0x6AD044; // NiTriStripsData::GetStripLengthSum
const RT_DRAW_PRIMITIVE: usize = 0x6AD089; // NiDX8Renderer::DrawPrimitive

// DrawSkinnedPrimitive
const SP_OPERATOR_NEW: usize = 0x6ADEA8; // operator new
const SP_SKIN_PARTITION_CTOR: usize = 0x6ADEBE; // NiSkinPartition::ctor
const SP_MAKE_PARTITIONS: usize = 0x6ADEDF; // NiSkinPartition::MakePartitions
const SP_FIND_CACHED_PARTITION_GEOM_BUFFER: usize = 0x6ADF85; // NiDX8Renderer::findCachedPartitionGeomBuffer
const SP_CREATE_PARTITION_GEOM_BUFFER: usize = 0x6ADFD9; // NiDX8Renderer::createPartitionGeomBuffer
const SP_DRAW_SKINNED_PRIMITIVE_2: usize = 0x6AE00F; // NiDX8Renderer::DrawSkinnedPrimitive2

// EndBatch
const EB_GET_RENDERER_SPECIFIC: usize = 0x6AE183; // NiPropertyState::GetRendererSpecific
const EB_SELECT_AND_LINK_PASS: usize = 0x6AE1D5; // NiDX8ConfigurableTexturePipeline::selectAndLinkPass
const EB_UPDATE_D3D_STATE: usize = 0x6AE1E4; // NiDX8RenderState::updateD3DState
const EB_BUILD_PASS: usize = 0x6AE209; // NiDX8ConfigurableTexturePipeline::buildPass
const EB_LIGHT_SET_STATE: usize = 0x6AE22E; // NiDX8LightManager::setState
const EB_LOAD_BINARY: usize = 0x6AE272; // NiDX8ConfigurableTexturePipeline::loadBinary
const EB_APPLY_PIXEL_SHADER: usize = 0x6AE2A0; // NiDX8ConfigurableTexturePipeline::applyPixelShader
const EB_CREATE_PARTITION_GEOM_BUFFER: usize = 0x6AE318; // NiDX8Renderer::createPartitionGeomBuffer
const EB_PACK_GEOMETRY_BUFFERS: usize = 0x6AE32E; // NiDX8Renderer::packGeometryBuffers
const EB_APPLY_VERTEX_SHADER: usize = 0x6AE349; // NiDX8ConfigurableTexturePipeline::applyVertexShader
const EB_SET_SKINNED_MODEL_TRANSFORMS: usize = 0x6AE3B9; // NiDX8Renderer::SetSkinnedModelTransforms
const EB_SET_MODEL_TRANSFORM: usize = 0x6AE404; // NiDX8Renderer::SetModelTransform
const EB_BOUND_COPY: usize = 0x6AE414; // NiBound_or_NiQuaternion::Copy
const EB_BUILD_SKIN_PARTITION_PASS: usize = 0x6AE4CA; // NiDX8ConfigurableTexturePipeline::buildSkinPartitionPass
const EB_ENSURE_AND_REGISTER_STREAMABLES: usize = 0x6AE599; // NiDX8ConfigurableTexturePipeline::ensureAndRegisterStreamables
const EB_DELETE: usize = 0x6AE5E1; // operator delete

// DrawPrimitive
const DP_GET_RENDERER_SPECIFIC: usize = 0x6AEBF7; // NiPropertyState::GetRendererSpecific
const DP_SELECT_AND_LINK_PASS: usize = 0x6AEC4D; // NiDX8ConfigurableTexturePipeline::selectAndLinkPass
const DP_UPDATE_D3D_STATE: usize = 0x6AEC5C; // NiDX8RenderState::updateD3DState
const DP_BUILD_PASS: usize = 0x6AEC81; // NiDX8ConfigurableTexturePipeline::buildPass
const DP_SET_SKINNED_MODEL_TRANSFORMS: usize = 0x6AECE7; // NiDX8Renderer::SetSkinnedModelTransforms
const DP_SET_MODEL_TRANSFORM: usize = 0x6AED2B; // NiDX8Renderer::SetModelTransform
const DP_BOUND_COPY: usize = 0x6AED37; // NiBound_or_NiQuaternion::Copy
const DP_LIGHT_SET_STATE: usize = 0x6AED5B; // NiDX8LightManager::setState
const DP_LOAD_BINARY: usize = 0x6AED85; // NiDX8ConfigurableTexturePipeline::loadBinary
const DP_CREATE_PARTITION_GEOM_BUFFER: usize = 0x6AEDCC; // NiDX8Renderer::createPartitionGeomBuffer
const DP_PACK_GEOMETRY_BUFFERS: usize = 0x6AEDE6; // NiDX8Renderer::packGeometryBuffers
const DP_APPLY_PIXEL_SHADER: usize = 0x6AEDF9; // NiDX8ConfigurableTexturePipeline::applyPixelShader
const DP_APPLY_VERTEX_SHADER: usize = 0x6AEE54; // NiDX8ConfigurableTexturePipeline::applyVertexShader
const DP_BUILD_SKIN_PARTITION_PASS: usize = 0x6AEECD; // NiDX8ConfigurableTexturePipeline::buildSkinPartitionPass
const DP_DRAW_INDEXED_PRIMITIVE: usize = 0x6AEF14; // patchDrawIndexedPrimitive (Code Patch)
const DP_ENSURE_AND_REGISTER_STREAMABLES: usize = 0x6AEF7E; // NiDX8ConfigurableTexturePipeline::ensureAndRegisterStreamables

// DrawSkinnedPrimitive2
const S2_SELECT_AND_LINK_PASS: usize = 0x6AF048; // NiDX8ConfigurableTexturePipeline::selectAndLinkPass
const S2_UPDATE_D3D_STATE: usize = 0x6AF057; // NiDX8RenderState::updateD3DState
const S2_BUILD_PASS: usize = 0x6AF07C; // NiDX8ConfigurableTexturePipeline::buildPass
const S2_LIGHT_SET_STATE: usize = 0x6AF0A4; // NiDX8LightManager::setState
const S2_LOAD_BINARY: usize = 0x6AF0D2; // NiDX8ConfigurableTexturePipeline::loadBinary
const S2_APPLY_PIXEL_SHADER: usize = 0x6AF0F2; // NiDX8ConfigurableTexturePipeline::applyPixelShader
const S2_BOUND_COPY: usize = 0x6AF0FE; // NiBound_or_NiQuaternion::Copy
const S2_SET_SKINNED_MODEL_TRANSFORMS: usize = 0x6AF188; // NiDX8Renderer::SetSkinnedModelTransforms
const S2_APPLY_VERTEX_SHADER: usize = 0x6AF1B0; // NiDX8ConfigurableTexturePipeline::applyVertexShader
const S2_BUILD_SKIN_PARTITION_PASS: usize = 0x6AF1EC; // NiDX8ConfigurableTexturePipeline::buildSkinPartitionPass
const S2_DRAW_INDEXED_PRIMITIVE: usize = 0x6AF243; // patchDrawIndexedPrimitive (Code Patch)
const S2_DRAW_PRIMITIVE: usize = 0x6AF250; // patchDrawPrimitive (Code Patch)
const S2_ENSURE_AND_REGISTER_STREAMABLES: usize = 0x6AF2A6; // NiDX8ConfigurableTexturePipeline::ensureAndRegisterStreamables

/// Every `call rel32` of the six functions. The ports call the target that is in memory there.
pub static SITES: &[usize] = &[
    RS_DRAW_SKINNED_PRIMITIVE,
    RS_CREATE_GEOMETRY_BUFFER_DATA,
    RS_DRAW_PRIMITIVE,
    RT_DRAW_SKINNED_PRIMITIVE,
    RT_CREATE_GEOMETRY_BUFFER_DATA,
    RT_GET_STRIP_LENGTH_SUM,
    RT_DRAW_PRIMITIVE,
    SP_OPERATOR_NEW,
    SP_SKIN_PARTITION_CTOR,
    SP_MAKE_PARTITIONS,
    SP_FIND_CACHED_PARTITION_GEOM_BUFFER,
    SP_CREATE_PARTITION_GEOM_BUFFER,
    SP_DRAW_SKINNED_PRIMITIVE_2,
    EB_GET_RENDERER_SPECIFIC,
    EB_SELECT_AND_LINK_PASS,
    EB_UPDATE_D3D_STATE,
    EB_BUILD_PASS,
    EB_LIGHT_SET_STATE,
    EB_LOAD_BINARY,
    EB_APPLY_PIXEL_SHADER,
    EB_CREATE_PARTITION_GEOM_BUFFER,
    EB_PACK_GEOMETRY_BUFFERS,
    EB_APPLY_VERTEX_SHADER,
    EB_SET_SKINNED_MODEL_TRANSFORMS,
    EB_SET_MODEL_TRANSFORM,
    EB_BOUND_COPY,
    EB_BUILD_SKIN_PARTITION_PASS,
    EB_ENSURE_AND_REGISTER_STREAMABLES,
    EB_DELETE,
    DP_GET_RENDERER_SPECIFIC,
    DP_SELECT_AND_LINK_PASS,
    DP_UPDATE_D3D_STATE,
    DP_BUILD_PASS,
    DP_SET_SKINNED_MODEL_TRANSFORMS,
    DP_SET_MODEL_TRANSFORM,
    DP_BOUND_COPY,
    DP_LIGHT_SET_STATE,
    DP_LOAD_BINARY,
    DP_CREATE_PARTITION_GEOM_BUFFER,
    DP_PACK_GEOMETRY_BUFFERS,
    DP_APPLY_PIXEL_SHADER,
    DP_APPLY_VERTEX_SHADER,
    DP_BUILD_SKIN_PARTITION_PASS,
    DP_DRAW_INDEXED_PRIMITIVE,
    DP_ENSURE_AND_REGISTER_STREAMABLES,
    S2_SELECT_AND_LINK_PASS,
    S2_UPDATE_D3D_STATE,
    S2_BUILD_PASS,
    S2_LIGHT_SET_STATE,
    S2_LOAD_BINARY,
    S2_APPLY_PIXEL_SHADER,
    S2_BOUND_COPY,
    S2_SET_SKINNED_MODEL_TRANSFORMS,
    S2_APPLY_VERTEX_SHADER,
    S2_BUILD_SKIN_PARTITION_PASS,
    S2_DRAW_INDEXED_PRIMITIVE,
    S2_DRAW_PRIMITIVE,
    S2_ENSURE_AND_REGISTER_STREAMABLES,
];

/// The target that the call at `site` has in memory now. The macros of `site` call this
/// function.
#[cfg(not(test))]
#[inline(always)]
unsafe fn target(site: usize) -> usize {
    crate::site::call_target(site)
}

/// The unit tests have no game code. Each site has a function that records the call.
#[cfg(test)]
unsafe fn target(site: usize) -> usize {
    tests::fake_target(site)
}

/// Calls `patchDrawIndexedPrimitive` as the game code does in this file. The device is in EAX
/// and its vtable is in EDX. `ecx` is the value that the game code has in ECX at the call. The
/// arguments of `IDirect3DDevice8::DrawIndexedPrimitive` are on the stack, and the callee
/// removes them. `min_index` is always 0. The function of the same name in `draw_misc` is
/// different: it takes `min_index` as an argument and does not pass the ECX of the game.
unsafe fn site_draw_indexed_primitive(
    site: usize,
    device: usize,
    vtable: usize,
    ecx: u32,
    primitive_type: u32,
    vertices: u32,
    start_index: u32,
    primitives: u32,
) -> u32 {
    let args = [device as u32, primitive_type, 0, vertices, start_index, primitives];
    // A call site of the engine: the main thread when the port's Rust runs; the device's
    // own vtable for a call of another thread.
    let vtable = match MainThread::here() {
        Some(t) => crate::link_client::draw_vtable(&t, vtable),
        None => vtable,
    };
    if !crate::site::is_call(site) {
        // The Code Patch option "Show transparent clothes in the inventory" is off: the game
        // calls `DrawIndexedPrimitive` of the device here, with the same arguments.
        let f: unsafe extern "system" fn(u32, u32, u32, u32, u32, u32) -> u32 = transmute(rd::<usize>(vtable, DEVICE_DRAW_INDEXED_PRIMITIVE));
        return f(args[0], args[1], args[2], args[3], args[4], args[5]);
    }
    let result: u32;
    asm!(
        "push dword ptr [{a} + 20]",
        "push dword ptr [{a} + 16]",
        "push dword ptr [{a} + 12]",
        "push dword ptr [{a} + 8]",
        "push dword ptr [{a} + 4]",
        "push dword ptr [{a}]",
        "call {t}",
        a = in(reg) args.as_ptr(),
        t = in(reg) target(site),
        inout("eax") device => result,
        inout("ecx") ecx => _,
        inout("edx") vtable => _,
        clobber_abi("system"),
    );
    result
}

/// Calls `patchDrawPrimitive` as the game code does. The vtable of the device is in ECX, the
/// device is in EAX and the primitive type is in EDX. The arguments of
/// `IDirect3DDevice8::DrawPrimitive` are on the stack, and the callee removes them.
unsafe fn site_draw_primitive(site: usize, device: usize, vtable: usize, primitive_type: u32, primitives: u32) -> u32 {
    let args = [device as u32, primitive_type, 0, primitives];
    // A call site of the engine: the main thread when the port's Rust runs; the device's
    // own vtable for a call of another thread.
    let vtable = match MainThread::here() {
        Some(t) => crate::link_client::draw_vtable(&t, vtable),
        None => vtable,
    };
    if !crate::site::is_call(site) {
        // The Code Patch option "Show transparent clothes in the inventory" is off: the game
        // calls `DrawPrimitive` of the device here, with the same arguments.
        let f: unsafe extern "system" fn(u32, u32, u32, u32) -> u32 = transmute(rd::<usize>(vtable, DEVICE_DRAW_PRIMITIVE));
        return f(args[0], args[1], args[2], args[3]);
    }
    let result: u32;
    asm!(
        "push dword ptr [{a} + 12]",
        "push dword ptr [{a} + 8]",
        "push dword ptr [{a} + 4]",
        "push dword ptr [{a}]",
        "call {t}",
        a = in(reg) args.as_ptr(),
        t = in(reg) target(site),
        inout("eax") device => result,
        inout("ecx") vtable => _,
        inout("edx") primitive_type => _,
        clobber_abi("system"),
    );
    result
}

// ---------------------------------------------------------------------------------------
// Engine data
// ---------------------------------------------------------------------------------------

/// `data_NiDX8Renderer_vertexBlendLookup`: the `D3DRS_VERTEXBLEND` value for a count of bones.
#[cfg(not(test))]
#[inline(always)]
fn vertex_blend_lookup() -> usize {
    crate::engine::VERTEX_BLEND_LOOKUP
}

#[cfg(test)]
fn vertex_blend_lookup() -> usize {
    tests::VERTEX_BLEND_TABLE.as_ptr() as usize
}

/// `NiDX8RenderState::renderStateSettings[D3DRS_VERTEXBLEND].currentValue`
const RS_VERTEX_BLEND_CURRENT: usize = 0x6B0;
const _: () = assert!(NiSkinPartition::refCount == ni::REF_COUNT && NiRefObject::refCount == ni::REF_COUNT);

/// `NiSkinData::skinPartition`
const SKIN_DATA_SKIN_PARTITION: usize = 8;
/// `NiRendererSpecificProperty::RenderData7`: the object the game code passes as `prop2`.
const RENDER_DATA_FIELD_8: usize = 8;
/// `NiRendererSpecificProperty::RenderData7`: the object the game code passes as `prop1`.
const RENDER_DATA_FIELD_C: usize = 0xC;
/// In `prop1`: a `__stdcall` function that replaces `buildSkinPartitionPass`.
const PROP_1_PASS_CALLBACK: usize = 0x2C;
/// The renderer type the render data must have.
const RENDERER_TYPE_DX8: u32 = 7;

// Bits of the value `buildPass` returns.
const BUILD_KEEP_VERTEX_BLEND: u32 = 0x40_0000;
const BUILD_KEEP_TRANSFORM: u32 = 0x80_0000;
const BUILD_KEEP_LIGHTS: u32 = 0x1000_0000;
const BUILD_NO_DRAW: u32 = 0x4000_0000;
// Bits of the value `loadBinary` returns. The low 16 bits tell that more passes follow.
const PASS_KEEP_PARTITION_PASS: u32 = 0x100_0000;
const PASS_KEEP_BUFFERS: u32 = 0x200_0000;
const PASS_KEEP_VERTEX_SHADER: u32 = 0x400_0000;
const PASS_KEEP_PIXEL_SHADER: u32 = 0x800_0000;
const PASS_KEEP_STREAMS: u32 = 0x2000_0000;
const PASS_NO_DRAW: u32 = 0x4000_0000;

// Virtual functions of the geometry data.
const VT_GET_ACTIVE_VERTEX_COUNT: usize = 0x30;
const VT_GET_ACTIVE_TRIANGLE_COUNT: usize = 0x3C;
const VT_GET_TRI_LIST: usize = 0x44;

// ---------------------------------------------------------------------------------------
// Parts the functions share. The game code has them inline in each function.
// ---------------------------------------------------------------------------------------

/// Returns the "has normals" flag and the count of texture sets, as `DrawPrimitive` and
/// `EndBatch` find them. A filled buffer gives them from its vertex format. If there is no
/// filled buffer, the geometry data gives them. The result is `None` when there is neither,
/// and the function returns then. The count of texture sets is 8 at most.
unsafe fn vertex_format(geom_buffer: usize, geom_data: usize) -> Option<(u32, u32)> {
    let mut has_normals = 0;
    let mut texture_sets: u32;
    if rd::<usize>(geom_buffer, GB::d3dVertexBuffer) != 0 && rd::<u8>(geom_buffer, GB::flags) & 1 != 0 {
        let fvf: u32 = rd(geom_buffer, GB::fvf);
        if fvf & D3DFVF_NORMAL != 0 {
            has_normals = 1;
        }
        texture_sets = (fvf >> 8) & 0xF;
    } else {
        if geom_data == 0 {
            return None;
        }
        texture_sets = rd::<u16>(geom_data, GD::textureSets) as u32;
        if rd::<usize>(geom_data, GD::normals) != 0 {
            has_normals = 1;
        }
    }
    if texture_sets > 8 {
        texture_sets = 8;
    }
    Some((has_normals, texture_sets))
}

/// The two objects of the renderer-specific property of a property state, or (0, 0).
/// `site` is the call to `NiPropertyState::GetRendererSpecific`.
unsafe fn renderer_specific_objects(site: usize, property_state: usize) -> (usize, usize) {
    let property = site_thiscall!(site, property_state) as usize;
    if property != 0 {
        let data: usize = rd(property, NiRendererSpecificProperty::renderData);
        if data != 0 && vcall0(data, 4) == RENDERER_TYPE_DX8 {
            let data: usize = rd(property, NiRendererSpecificProperty::renderData);
            if data != 0 {
                return (rd(data, RENDER_DATA_FIELD_C), rd(data, RENDER_DATA_FIELD_8));
            }
        }
    }
    (0, 0)
}

/// Sets `D3DRS_VERTEXBLEND` for a count of bones when the render state has another value.
unsafe fn set_vertex_blend(this: usize, bones: u16) {
    let render_state: usize = rd(this, R::renderState);
    let value: u32 = rd(vertex_blend_lookup(), bones as usize * 4);
    if rd::<u32>(render_state, RS_VERTEX_BLEND_CURRENT) != value {
        wr(render_state, RS_VERTEX_BLEND_CURRENT, value);
        device_set_render_state(rd(render_state, RS::device), D3DRS_VERTEXBLEND, value);
    }
}

/// The count of primitives of strip `strip` of a buffer: the length of the strip minus 2.
/// When the buffer has no strip lengths, it is the triangle count of the buffer.
unsafe fn strip_primitives(geom_buffer: usize, strip: u32) -> u32 {
    let lengths: usize = rd(geom_buffer, GB::stripVertexCounts);
    if lengths != 0 {
        (rd::<u16>(lengths, strip as usize * 2) as u32).wrapping_sub(2)
    } else {
        rd(geom_buffer, GB::trianglesCount)
    }
}

/// Sets up the pass of one object. The port calls the function of `prop1` when `prop1` has
/// one. If it has none, the port calls `buildSkinPartitionPass` through `site`.
#[allow(clippy::too_many_arguments)]
unsafe fn build_partition_pass(
    site: usize,
    this: usize,
    property_state: usize,
    effect_state: usize,
    has_normals: u32,
    geom_data: usize,
    skin: usize,
    partition: usize,
    transform: usize,
    world_bound: usize,
    prop1: usize,
    prop2: usize,
) {
    let callback: usize = if prop1 != 0 { rd(prop1, PROP_1_PASS_CALLBACK) } else { 0 };
    let pipeline: usize = rd(this, R::configurableTexturePipeline);
    if callback != 0 {
        let pass: usize = rd(pipeline, CTP::field_8);
        let f: unsafe extern "system" fn(usize, usize, u32, u32, usize, usize, usize, usize, usize, usize, usize) -> u32 = transmute(callback);
        f(property_state, effect_state, has_normals, 1, geom_data, skin, partition, transform, world_bound, prop2, pass);
    } else {
        site_thiscall!(site, pipeline, property_state, effect_state, has_normals, 1, geom_data, skin, partition, transform, world_bound);
    }
}

// ---------------------------------------------------------------------------------------
// RenderShape, RenderTriStrips
// ---------------------------------------------------------------------------------------

/// The 8 bytes at 0x6ACF1F in `RenderShape`, after the test for a skin. They are the
/// instruction that selects hardware or software skinning, and the opcode of the jump after it.
const RS_SKINNING_CHECK: usize = 0x6ACF1F;
/// The bytes in the database: `cmp dword ptr [edi+3E0h], 4` and `jb`. The game uses software
/// skinning when the device has fewer than 4 hardware bones.
#[cfg(test)]
const SKINNING_CHECK_DATABASE: [u8; 8] = [0x83, 0xBF, 0xE0, 0x03, 0x00, 0x00, 0x04, 0x72];
/// The bytes MWSE writes: `nop`, `test word ptr [esi+36h], 200h` and `jnz`. The patch is
/// `PatchNIDX8Renderer_RenderShape` in PatchUtil.cpp, "Allow control of whether software or
/// hardware skinning is used through TriShape flags". The game uses software skinning when
/// the geometry data (ESI) has bit 0x200 in its flags. The jump has the same target as in the
/// database.
const SKINNING_CHECK_MWSE: [u8; 8] = [0x90, 0x66, 0xF7, 0x46, 0x36, 0x00, 0x02, 0x75];
/// `NiTriBasedGeomData` + 0x36: the flags the MWSE patch reads (`patchRenderFlags`).
const GEOM_DATA_PATCH_RENDER_FLAGS: usize = 0x36;
const PATCH_RENDER_FLAG_SOFTWARE_SKINNING: u16 = 0x200;

/// The condition in `RenderShape` that stops hardware skinning for a skinned shape.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum SkinningCheck {
    /// The game: the count of hardware bones of the device.
    HardwareBones,
    /// MWSE: a flag of the geometry data.
    GeometryFlag,
}

/// Returns the check that the bytes at `RS_SKINNING_CHECK` are. Bytes that are not the
/// database bytes and not the MWSE bytes count as the database. The port does not run with
/// such bytes: `ports::FOLLOWED` lists exactly the MWSE bytes for this place, and
/// `install::census` locks the port when the memory has other bytes.
fn skinning_check(bytes: &[u8; 8]) -> SkinningCheck {
    if *bytes == SKINNING_CHECK_MWSE {
        SkinningCheck::GeometryFlag
    } else {
        SkinningCheck::HardwareBones
    }
}

/// The bytes of the check as they are in memory now.
#[cfg(not(test))]
#[inline(always)]
unsafe fn skinning_check_bytes() -> [u8; 8] {
    rd(RS_SKINNING_CHECK, 0)
}

#[cfg(test)]
unsafe fn skinning_check_bytes() -> [u8; 8] {
    tests::SKINNING_CHECK_BYTES.with(|bytes| bytes.get())
}

/// True when the game takes the jump at 0x6ACF26. Then the game draws a shape that has a
/// skin without hardware skinning.
unsafe fn software_skinning(check: SkinningCheck, this: usize, geom_data: usize) -> bool {
    match check {
        SkinningCheck::HardwareBones => rd::<u32>(this, R::countHWBones) < 4,
        SkinningCheck::GeometryFlag => rd::<u16>(geom_data, GEOM_DATA_PATCH_RENDER_FLAGS) & PATCH_RENDER_FLAG_SOFTWARE_SKINNING != 0,
    }
}

/// The shape record of a draw (see `shape_stream`): the transform, the bound and the
/// lights of the draw in the order of `DrawPrimitive` 0x6AEB80 (the port below), but
/// always: `DrawPrimitive` skips the transform on BUILD_KEEP_TRANSFORM and the lights on
/// BUILD_KEEP_LIGHTS, which the fixed-function pass never sets. Then the record. A skinned shape (`skin`
/// not 0, hardware skinning) gets the lights and the bound as `DrawSkinnedPrimitive2` sets
/// them. It gets the palette of the skin in place of the transform. Returns true when the
/// pipeline of the device must not run for the draw. `RenderScreenPoly` uses it too, with
/// the identity transform that it gives `DrawPrimitive`.
pub(crate) unsafe fn shape_record(t: &MainThread, this: usize, skin: usize, transform: usize, world_bound: usize) -> bool {
    if !crate::shape_stream::on() {
        return false;
    }
    let property_state: usize = rd(this, R::currentPropertyState);
    let (light_manager, effect_state, texturing, vertex_color) = crate::shape_stream::light_state_arguments(this);
    thread_local! {
        /// The palette of a skinned draw, kept between the draws so that a draw does not
        /// allocate it.
        static PALETTE: core::cell::RefCell<Vec<[f32; 16]>> = const { core::cell::RefCell::new(Vec::new()) };
    }
    let mut palette = PALETTE.with(|kept| core::mem::take(&mut *kept.borrow_mut()));
    palette.clear();
    let (world, relative) = if skin != 0 {
        let relative = match crate::camera_space::skin_palette(t, skin, transform, true, &mut palette) {
            Ok(relative) => relative,
            Err(why) => {
                crate::shape_stream::without_palette(t, why);
                return false;
            }
        };
        site_thiscall!(S2_LIGHT_SET_STATE, light_manager, effect_state, texturing, vertex_color);
        site_thiscall!(S2_BOUND_COPY, this + R::worldBound, world_bound);
        let mut identity = [0f32; 16];
        for i in 0..4 {
            identity[i * 5] = 1.0;
        }
        (identity, relative)
    } else {
        if !crate::camera_space::take_model_transform(t, this, transform, true) {
            site_thiscall!(DP_SET_MODEL_TRANSFORM, this, transform);
            crate::camera_space::check_model_transform(t, this, transform, true);
        }
        site_thiscall!(DP_BOUND_COPY, this + R::worldBound, world_bound);
        site_thiscall!(DP_LIGHT_SET_STATE, light_manager, effect_state, texturing, vertex_color);
        (crate::shape_stream::current_world(this), crate::link_client::world_is_relative(t))
    };
    let fog = crate::shape_stream::fog_values(this, property_state);
    let sent = crate::shape_stream::send(t, this, world, relative, crate::shape_stream::ambient(this), crate::shape_stream::material_values(property_state), fog, &palette);
    PALETTE.with(|kept| *kept.borrow_mut() = palette);
    sent
}

/// 0x6ACEF0 `NiDX8Renderer::RenderShape`. The port uses the skinning check that MWSE puts at
/// +0x2F when the memory has that check. Unlike the engine, the port sends the shape record
/// first and does not draw when `shape_record` returns true.
pub unsafe fn render_shape(t: &MainThread, this: usize, geom_data: usize, skin: usize, transform: usize, world_bound: usize) {
    crate::shadow::touch(&render_shape_port::STATE);
    if rd::<u8>(this, R::bSceneOpen) == 0 {
        return;
    }
    if vcall0(geom_data, VT_GET_ACTIVE_VERTEX_COUNT) as u16 == 0 {
        return;
    }
    if skin != 0 && !software_skinning(skinning_check(&skinning_check_bytes()), this, geom_data) {
        if shape_record(t, this, skin, transform, world_bound) {
            return;
        }
        site_thiscall!(RS_DRAW_SKINNED_PRIMITIVE, this, geom_data, skin, transform, world_bound);
        return;
    }
    if skin != 0 {
        let check = skinning_check(&skinning_check_bytes());
        crate::shape_stream::software_skinned(t, match check {
            SkinningCheck::HardwareBones => "less than 4 bones of the device",
            SkinningCheck::GeometryFlag => "the flag of MWSE on the geometry",
        });
    }
    if skin == 0 && shape_record(t, this, 0, transform, world_bound) {
        return;
    }
    let mut geom_buffer: usize = 0;
    let out = core::ptr::addr_of_mut!(geom_buffer) as usize;
    if site_thiscall!(RS_CREATE_GEOMETRY_BUFFER_DATA, this, out, geom_data) as u8 != 0 {
        wr(geom_buffer, GB::primitiveType, D3DPT_TRIANGLELIST);
        let triangles = vcall0(geom_data, VT_GET_ACTIVE_TRIANGLE_COUNT) & 0xFFFF;
        wr(geom_buffer, GB::trianglesCount, triangles);
        wr::<u32>(geom_buffer, GB::stripCount, 1);
        wr::<u32>(geom_buffer, GB::stripVertexCounts, 0);
        let list = vcall0(geom_data, VT_GET_TRI_LIST);
        wr(geom_buffer, GB::triList, list);
    }
    site_thiscall!(RS_DRAW_PRIMITIVE, this, geom_data, skin, transform, world_bound, geom_buffer, 0);
}

/// 0x6ACFC0 `NiDX8Renderer::RenderTriStrips`. Unlike the engine, the port sends the shape
/// record first and does not draw when `shape_record` returns true.
pub unsafe fn render_tri_strips(t: &MainThread, this: usize, geom_data: usize, skin: usize, transform: usize, world_bound: usize) {
    crate::shadow::touch(&render_tri_strips_port::STATE);
    if rd::<u8>(this, R::bSceneOpen) == 0 {
        return;
    }
    if vcall0(geom_data, VT_GET_ACTIVE_VERTEX_COUNT) as u16 == 0 {
        return;
    }
    if skin != 0 && rd::<u32>(this, R::countHWBones) >= 4 {
        if shape_record(t, this, skin, transform, world_bound) {
            return;
        }
        site_thiscall!(RT_DRAW_SKINNED_PRIMITIVE, this, geom_data, skin, transform, world_bound);
        return;
    }
    if skin != 0 {
        crate::shape_stream::software_skinned(t, "strips, less than 4 bones of the device");
    }
    if skin == 0 && shape_record(t, this, 0, transform, world_bound) {
        return;
    }
    let mut geom_buffer: usize = 0;
    let out = core::ptr::addr_of_mut!(geom_buffer) as usize;
    if site_thiscall!(RT_CREATE_GEOMETRY_BUFFER_DATA, this, out, geom_data) as u8 != 0 {
        wr(geom_buffer, GB::primitiveType, D3DPT_TRIANGLESTRIP);
        wr(geom_buffer, GB::stripCount, rd::<u16>(geom_data, NiTriStripsData::numStrips) as u32);
        let sum = site_thiscall!(RT_GET_STRIP_LENGTH_SUM, geom_data) & 0xFFFF;
        let strips: u32 = rd(geom_buffer, GB::stripCount);
        wr(geom_buffer, GB::trianglesCount, sum.wrapping_sub(strips.wrapping_add(strips)));
        wr::<u32>(geom_buffer, GB::stripVertexCounts, rd(geom_data, NiTriStripsData::stripLengths));
        wr::<u32>(geom_buffer, GB::triList, rd(geom_data, NiTriStripsData::stripLists));
    }
    site_thiscall!(RT_DRAW_PRIMITIVE, this, geom_data, skin, transform, world_bound, geom_buffer, 0);
}

/// The call is a skip when the scene is open and the geometry data has active vertices.
/// `RenderShape` and `RenderTriStrips` then call `DrawSkinnedPrimitive` or `DrawPrimitive`.
/// `draw_regions` gives the reason. The two functions write nothing before that call.
unsafe fn render_regions(set: &mut Regions, this: usize, geom_data: usize) {
    set.add(this, R::SIZE);
    if rd::<u8>(this, R::bSceneOpen) != 0 && vcall0(geom_data, VT_GET_ACTIVE_VERTEX_COUNT) as u16 != 0 {
        set.skip();
    }
}

// ---------------------------------------------------------------------------------------
// DrawPrimitive
// ---------------------------------------------------------------------------------------

/// 0x6AEB80 `NiDX8Renderer::DrawPrimitive`
pub unsafe fn draw_primitive(
    t: &MainThread,
    this: usize,
    geom_data: usize,
    skin: usize,
    transform: usize,
    world_bound: usize,
    geom_buffer: usize,
    partition: usize,
) {
    crate::shadow::touch(&draw_primitive_port::STATE);
    // The game code gives the address of its argument to the functions that fill the buffer.
    let mut geom_buffer = geom_buffer;
    let Some((has_normals, texture_sets)) = vertex_format(geom_buffer, geom_data) else {
        return;
    };
    let (prop1, prop2) = renderer_specific_objects(DP_GET_RENDERER_SPECIFIC, rd(this, R::currentPropertyState));

    site_thiscall!(
        DP_SELECT_AND_LINK_PASS,
        rd::<usize>(this, R::configurableTexturePipeline),
        rd::<usize>(this, R::currentPropertyState),
        rd::<usize>(this, R::currentEffectState),
        has_normals,
        1,
        geom_data,
        skin,
        transform,
        world_bound,
    );
    site_thiscall!(DP_UPDATE_D3D_STATE, rd::<usize>(this, R::renderState), rd::<usize>(this, R::currentPropertyState));
    let flags = site_thiscall!(
        DP_BUILD_PASS,
        rd::<usize>(this, R::configurableTexturePipeline),
        rd::<usize>(this, R::currentPropertyState),
        rd::<usize>(this, R::currentEffectState),
        has_normals,
        texture_sets,
        geom_data,
        skin,
        transform,
        world_bound,
    );

    let mut set_bound = true;
    if partition != 0 {
        if flags & BUILD_KEEP_VERTEX_BLEND == 0 {
            set_vertex_blend(this, rd(partition, P::numBones));
        }
        if flags & BUILD_KEEP_TRANSFORM != 0 {
            set_bound = false;
        } else {
            if !crate::camera_space::take_skinned_transforms(t, this, skin, partition, transform, true) {
                site_thiscall!(DP_SET_SKINNED_MODEL_TRANSFORMS, this, skin, partition, transform, world_bound);
                crate::camera_space::check_skinned_transforms(t, skin, partition, transform, true);
            }
        }
    } else {
        set_vertex_blend(this, 0);
        if flags & BUILD_KEEP_TRANSFORM != 0 {
            set_bound = false;
        } else {
            if !crate::camera_space::take_model_transform(t, this, transform, true) {
                site_thiscall!(DP_SET_MODEL_TRANSFORM, this, transform);
                crate::camera_space::check_model_transform(t, this, transform, true);
            }
        }
    }
    if set_bound {
        site_thiscall!(DP_BOUND_COPY, this + R::worldBound, world_bound);
    }
    if flags & BUILD_KEEP_LIGHTS == 0 {
        let property_state: usize = rd(this, R::currentPropertyState);
        let vertex_color: usize = rd(property_state, PS::propVertexColor);
        let texturing: usize = rd(property_state, PS::propTexturing);
        let effect_state: usize = rd(this, R::currentEffectState);
        site_thiscall!(DP_LIGHT_SET_STATE, rd::<usize>(this, R::lightManager), effect_state, texturing, vertex_color);
    }

    let mut passes: u32 = 0;
    if flags & BUILD_NO_DRAW == 0 {
        loop {
            let binary = site_thiscall!(DP_LOAD_BINARY, rd::<usize>(this, R::configurableTexturePipeline), geom_data, skin, transform, world_bound);
            let more = binary & 0xFFFF != 0;
            if binary & PASS_NO_DRAW == 0 {
                let first = passes == 0;
                passes = passes.wrapping_add(1);
                if first && binary & PASS_KEEP_BUFFERS == 0 {
                    let out = core::ptr::addr_of_mut!(geom_buffer) as usize;
                    if partition != 0 {
                        site_thiscall!(DP_CREATE_PARTITION_GEOM_BUFFER, this, out, geom_data, skin, partition);
                    } else {
                        site_thiscall!(DP_PACK_GEOMETRY_BUFFERS, this, out, geom_data, skin, prop1, prop2);
                    }
                }
                if binary & PASS_KEEP_PIXEL_SHADER == 0 {
                    site_thiscall!(DP_APPLY_PIXEL_SHADER, rd::<usize>(this, R::configurableTexturePipeline));
                }
                if binary & PASS_KEEP_STREAMS == 0 {
                    let device: usize = rd(this, R::d3dDevice);
                    device_set_stream_source(device, 0, rd(geom_buffer, GB::d3dVertexBuffer), rd(geom_buffer, GB::vertexStride));
                    let index_buffer: usize = rd(geom_buffer, GB::indexBuffer);
                    if index_buffer != 0 {
                        device_set_indices(rd(this, R::d3dDevice), index_buffer, 0);
                    }
                }
                if binary & PASS_KEEP_VERTEX_SHADER == 0 {
                    let fvf: u32 = rd(geom_buffer, GB::fvf);
                    site_thiscall!(DP_APPLY_VERTEX_SHADER, rd::<usize>(this, R::configurableTexturePipeline), fvf);
                }
                if binary & PASS_KEEP_PARTITION_PASS == 0 {
                    build_partition_pass(
                        DP_BUILD_SKIN_PARTITION_PASS,
                        this,
                        rd(this, R::currentPropertyState),
                        rd(this, R::currentEffectState),
                        has_normals,
                        geom_data,
                        skin,
                        partition,
                        transform,
                        world_bound,
                        prop1,
                        prop2,
                    );
                }
                if rd::<usize>(geom_buffer, GB::indexBuffer) != 0 {
                    let mut start_index: u32 = 0;
                    let mut strip: u32 = 0;
                    if rd::<u32>(geom_buffer, GB::stripCount) != 0 {
                        loop {
                            let primitives = strip_primitives(geom_buffer, strip);
                            let vertices: u32 = rd(geom_buffer, GB::verticesCount);
                            let primitive_type: u32 = rd(geom_buffer, GB::primitiveType);
                            let device: usize = rd(this, R::d3dDevice);
                            let vtable: usize = rd(device, 0);
                            crate::link_client::draw_state(t, this, primitive_type, vertices, start_index, primitives, false, passes > 1);
                            // ECX has the primitive type at this call.
                            site_draw_indexed_primitive(
                                DP_DRAW_INDEXED_PRIMITIVE,
                                device,
                                vtable,
                                primitive_type,
                                primitive_type,
                                vertices,
                                start_index,
                                primitives,
                            );
                            strip = strip.wrapping_add(1);
                            start_index = primitives.wrapping_add(start_index).wrapping_add(2);
                            if strip >= rd::<u32>(geom_buffer, GB::stripCount) {
                                break;
                            }
                        }
                    }
                } else {
                    let primitives: u32 = rd(geom_buffer, GB::trianglesCount);
                    let device: usize = rd(this, R::d3dDevice);
                    crate::link_client::draw_state(t, this, rd(geom_buffer, GB::primitiveType), 0, 0, primitives, false, passes > 1);
                    device_draw_primitive(device, rd(geom_buffer, GB::primitiveType), 0, primitives);
                }
            }
            if !more {
                break;
            }
        }
    }
    site_thiscall!(
        DP_ENSURE_AND_REGISTER_STREAMABLES,
        rd::<usize>(this, R::configurableTexturePipeline),
        rd::<usize>(this, R::currentPropertyState),
        rd::<usize>(this, R::currentEffectState),
        has_normals,
        1,
        geom_data,
        skin,
        transform,
        world_bound,
    );
}

/// The call is a skip when the function gets to `buildPass` and `loadBinary`. Each call that
/// draws does.
///
/// `buildPass` and `loadBinary` call a virtual function of the pass, and its writes are not
/// known. The shadow test also skips the ports of these two functions. The pass can make
/// textures, and the buffer functions can release device buffers. The shadow test compares
/// each callee that has a port when the function here runs as game code.
unsafe fn draw_regions(set: &mut Regions) {
    set.skip();
}

/// The call is a skip when there is a filled buffer or geometry data. `DrawPrimitive` then
/// draws, and `draw_regions` gives the reason. If there is neither, `DrawPrimitive` returns
/// immediately.
unsafe fn draw_primitive_regions(set: &mut Regions, this: usize, geom_data: usize, geom_buffer: usize) {
    set.add(this, R::SIZE);
    if vertex_format(geom_buffer, geom_data).is_some() {
        draw_regions(set);
    }
}

// ---------------------------------------------------------------------------------------
// DrawSkinnedPrimitive
// ---------------------------------------------------------------------------------------

/// The skin partitions of a skinned shape, made as `DrawSkinnedPrimitive` makes them (6ADE70
/// to 6ADF0B) when the skin data has none. That is a `NiSkinPartition` with `MakePartitions`
/// for the bones of the device. Returns the skin partition, or the one that `MakePartitions`
/// did not fill when it failed (as the game goes on with it). The record of a skinned shape
/// needs the partitions before the draw function runs. The game makes them at the first draw
/// of a skin, and the inventory draws a new copy of the character for each picture.
pub(crate) unsafe fn ensure_skin_partitions(this: usize, geom_data: usize, skin: usize) -> usize {
    let skin_data: usize = rd(skin, NiSkinInstance::skinData);
    let skin_partition: usize = if skin_data != 0 { rd(skin_data, SKIN_DATA_SKIN_PARTITION) } else { 0 };
    if skin_partition != 0 {
        return skin_partition;
    }
    let mut new_partition: usize = 0;
    let mut eax = site_cdecl!(SP_OPERATOR_NEW, NiSkinPartition::SIZE) as usize;
    if eax != 0 {
        eax = site_thiscall!(SP_SKIN_PARTITION_CTOR, eax) as usize;
        new_partition = eax;
    }
    // Unlike the engine: a bone with a vertex count and no weight array gets no partitions.
    // `NiSkinPartition::CreateVertexInteractions` (0x6C6B60) reads the array of each bone
    // with a count and faults on a null one. Such a skin gets the empty partition, as the
    // engine's failure path gives it. The shape draws nothing on the hardware path. The
    // first 8 go to the log with their names.
    if skin_data != 0 && !skin_data_complete(skin_data) {
        static REFUSED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
        if REFUSED.fetch_add(1, core::sync::atomic::Ordering::Relaxed) < 8 {
            let root: usize = rd(skin, NiSkinInstance::rootParent);
            crate::log!(
                "skin partitions: the skin data 0x{skin_data:x} of a shape under '{}' has a bone with vertex weights counted and no weight array; the shape gets no partition (the engine's builder would crash on it)",
                object_name(root)
            );
        }
        return new_partition;
    }
    // `mov al, countHWBones; push eax; push eax`: the high bytes keep the value EAX had.
    let bones = (eax as u32 & 0xFFFF_FF00) | rd::<u8>(this, R::countHWBones) as u32;
    if site_thiscall!(SP_MAKE_PARTITIONS, new_partition, geom_data, skin_data, bones, bones) as u8 != 0 {
        let slot = rd::<usize>(skin, NiSkinInstance::skinData) + SKIN_DATA_SKIN_PARTITION;
        ni::pointer_assign(slot, new_partition);
    }
    new_partition
}

/// `NiSkinData` (NetImmerse 4.0.0.2, as `camera_space` reads it): the bone count is at 0x40.
/// The bone records are at 0x44, 0x4C bytes each. A bone record has the skin-to-bone
/// transform, the bound, the vertex count (u16, +0x44) and the vertex weight array (+0x48).
/// True when every bone with a count has its array: the engine's partition builder reads the
/// array of each such bone.
unsafe fn skin_data_complete(skin_data: usize) -> bool {
    const BONE_COUNT: usize = 0x40;
    const BONES: usize = 0x44;
    const BONE_SIZE: usize = 0x4C;
    const BONE_VERTICES: usize = 0x44;
    const BONE_WEIGHTS: usize = 0x48;
    let count: u32 = rd(skin_data, BONE_COUNT);
    let bones: usize = rd(skin_data, BONES);
    if count == 0 {
        return true;
    }
    if bones == 0 || count > 4096 {
        return false;
    }
    (0..count as usize).all(|i| {
        let bone = bones + i * BONE_SIZE;
        rd::<u16>(bone, BONE_VERTICES) == 0 || rd::<usize>(bone, BONE_WEIGHTS) != 0
    })
}

/// The name of an object (`NiObjectNET::name` at +8), or "-".
unsafe fn object_name(object: usize) -> String {
    if object == 0 {
        return String::from("-");
    }
    let at: usize = rd(object, 8);
    if at == 0 {
        return String::from("-");
    }
    core::ffi::CStr::from_ptr(at as *const core::ffi::c_char).to_string_lossy().into_owned()
}

/// 0x6ADE70 `NiDX8Renderer::DrawSkinnedPrimitive`. The port makes the skin partitions when
/// the skin data has none. Then it fills the geometry buffer of each partition and draws the
/// partitions.
pub unsafe fn draw_skinned_primitive(this: usize, geom_data: usize, skin: usize, transform: usize, world_bound: usize) {
    crate::shadow::touch(&draw_skinned_primitive_port::STATE);
    let skin_partition = ensure_skin_partitions(this, geom_data, skin);

    let partitions: usize = rd(skin_partition, NiSkinPartition::partitions);
    let count: u32 = rd(skin_partition, NiSkinPartition::partitionCount);
    let is_list = rd::<u16>(partitions, P::numStripLengths) == 0;
    let primitive_type = if is_list { D3DPT_TRIANGLELIST } else { D3DPT_TRIANGLESTRIP };
    if count != 0 {
        let mut left = count;
        let mut partition = partitions;
        loop {
            let triangles: u32 = rd(partition, P::triangles);
            let mut geom_buffer: usize = 0;
            let mut strips: u32 = 1;
            let mut strip_lengths: u32 = 0;
            if !is_list {
                strip_lengths = rd(partition, P::stripLengths);
                strips = rd::<u16>(partition, P::numStripLengths) as u32;
            }
            let out = core::ptr::addr_of_mut!(geom_buffer) as usize;
            if site_thiscall!(SP_FIND_CACHED_PARTITION_GEOM_BUFFER, this, out, geom_data, skin, partition) as u8 != 0 {
                wr(geom_buffer, GB::primitiveType, primitive_type);
                wr(geom_buffer, GB::trianglesCount, rd::<u16>(partition, P::numTriangles) as u32);
                wr(geom_buffer, GB::stripCount, strips);
                wr(geom_buffer, GB::stripVertexCounts, strip_lengths);
                wr(geom_buffer, GB::triList, triangles);
                let out = core::ptr::addr_of_mut!(geom_buffer) as usize;
                site_thiscall!(SP_CREATE_PARTITION_GEOM_BUFFER, this, out, geom_data, skin, partition);
            }
            partition += P::SIZE;
            left -= 1;
            if left == 0 {
                break;
            }
        }
    }
    site_thiscall!(SP_DRAW_SKINNED_PRIMITIVE_2, this, primitive_type, geom_data, skin, transform, world_bound, 0, 0);
}

// ---------------------------------------------------------------------------------------
// DrawSkinnedPrimitive2
// ---------------------------------------------------------------------------------------

/// 0x6AEF90 `NiDX8Renderer::DrawSkinnedPrimitive2`: draws the partitions of a skin. The game
/// code does not read the last two arguments.
#[allow(clippy::too_many_arguments)]
pub unsafe fn draw_skinned_primitive_2(
    t: &MainThread,
    this: usize,
    primitive_type: u32,
    geom_data: usize,
    skin: usize,
    transform: usize,
    world_bound: usize,
    _unused_1: u32,
    _unused_2: u32,
) {
    crate::shadow::touch(&draw_skinned_primitive_2_port::STATE);
    if skin == 0 {
        return;
    }
    let skin_data: usize = rd(skin, NiSkinInstance::skinData);
    let skin_partition: usize = if skin_data != 0 { rd(skin_data, SKIN_DATA_SKIN_PARTITION) } else { 0 };
    let partitions: usize = rd(skin_partition, NiSkinPartition::partitions);
    let first_buffer: usize = rd(partitions, P::bufferData);

    // This is different from DrawPrimitive. The game code does not test the geometry data
    // here and has no limit of 8 texture sets.
    let mut has_normals: u32 = 0;
    let texture_sets: u32;
    if rd::<usize>(first_buffer, GB::d3dVertexBuffer) != 0 && rd::<u8>(first_buffer, GB::flags) & 1 != 0 {
        let fvf: u32 = rd(first_buffer, GB::fvf);
        if fvf & D3DFVF_NORMAL != 0 {
            has_normals = 1;
        }
        texture_sets = (fvf >> 8) & 0xF;
    } else {
        texture_sets = rd::<u16>(geom_data, GD::textureSets) as u32;
        if rd::<usize>(geom_data, GD::normals) != 0 {
            has_normals = 1;
        }
    }

    site_thiscall!(
        S2_SELECT_AND_LINK_PASS,
        rd::<usize>(this, R::configurableTexturePipeline),
        rd::<usize>(this, R::currentPropertyState),
        rd::<usize>(this, R::currentEffectState),
        has_normals,
        1,
        geom_data,
        skin,
        transform,
        world_bound,
    );
    site_thiscall!(S2_UPDATE_D3D_STATE, rd::<usize>(this, R::renderState), rd::<usize>(this, R::currentPropertyState));
    let flags = site_thiscall!(
        S2_BUILD_PASS,
        rd::<usize>(this, R::configurableTexturePipeline),
        rd::<usize>(this, R::currentPropertyState),
        rd::<usize>(this, R::currentEffectState),
        has_normals,
        texture_sets,
        geom_data,
        skin,
        transform,
        world_bound,
    );
    if flags & BUILD_KEEP_LIGHTS == 0 {
        let property_state: usize = rd(this, R::currentPropertyState);
        let vertex_color: usize = rd(property_state, PS::propVertexColor);
        let texturing: usize = rd(property_state, PS::propTexturing);
        let effect_state: usize = rd(this, R::currentEffectState);
        site_thiscall!(S2_LIGHT_SET_STATE, rd::<usize>(this, R::lightManager), effect_state, texturing, vertex_color);
    }
    let count: u32 = rd(skin_partition, NiSkinPartition::partitionCount);
    let mut link_pass = 0u32;
    if flags & BUILD_NO_DRAW == 0 {
        loop {
            let binary = site_thiscall!(S2_LOAD_BINARY, rd::<usize>(this, R::configurableTexturePipeline), geom_data, skin, transform, world_bound);
            let more = binary & 0xFFFF != 0;
            // For the state packet only.
            let later_pass = link_pass != 0;
            link_pass += 1;
            if binary & PASS_KEEP_PIXEL_SHADER == 0 {
                site_thiscall!(S2_APPLY_PIXEL_SHADER, rd::<usize>(this, R::configurableTexturePipeline));
            }
            site_thiscall!(S2_BOUND_COPY, this + R::worldBound, world_bound);
            let mut index: u32 = 0;
            if count != 0 {
                let mut partition = partitions;
                loop {
                    if flags & BUILD_KEEP_VERTEX_BLEND == 0 {
                        set_vertex_blend(this, rd(partition, P::numBonesPerVertex));
                    }
                    if flags & BUILD_KEEP_TRANSFORM == 0 {
                        if !crate::camera_space::take_skinned_transforms(t, this, skin, partition, transform, true) {
                            site_thiscall!(S2_SET_SKINNED_MODEL_TRANSFORMS, this, skin, partition, transform, world_bound);
                            crate::camera_space::check_skinned_transforms(t, skin, partition, transform, true);
                        }
                    }
                    let buffer: usize = rd(partition, P::bufferData);
                    if buffer == 0 {
                        // The game code returns here and does not call
                        // `ensureAndRegisterStreamables`.
                        return;
                    }
                    let primitives: u32 = rd(buffer, GB::trianglesCount);
                    if binary & PASS_KEEP_VERTEX_SHADER == 0 {
                        let fvf: u32 = rd(buffer, GB::fvf);
                        site_thiscall!(S2_APPLY_VERTEX_SHADER, rd::<usize>(this, R::configurableTexturePipeline), fvf);
                    }
                    if binary & PASS_KEEP_PARTITION_PASS == 0 {
                        site_thiscall!(
                            S2_BUILD_SKIN_PARTITION_PASS,
                            rd::<usize>(this, R::configurableTexturePipeline),
                            rd::<usize>(this, R::currentPropertyState),
                            rd::<usize>(this, R::currentEffectState),
                            has_normals,
                            1,
                            geom_data,
                            skin,
                            partition,
                            transform,
                            world_bound,
                        );
                    }
                    if binary & PASS_KEEP_STREAMS == 0 {
                        let device: usize = rd(this, R::d3dDevice);
                        device_set_stream_source(device, 0, rd(buffer, GB::d3dVertexBuffer), rd(buffer, GB::vertexStride));
                        let device: usize = rd(this, R::d3dDevice);
                        device_set_indices(device, rd(buffer, GB::indexBuffer), 0);
                    }
                    let index_buffer: usize = rd(buffer, GB::indexBuffer);
                    let device: usize = rd(this, R::d3dDevice);
                    let vtable: usize = rd(device, 0);
                    if index_buffer != 0 {
                        let vertices = rd::<u16>(partition, P::numVertices) as u32;
                        crate::link_client::draw_state(t, this, primitive_type, vertices, 0, primitives, true, later_pass);
                        // ECX has the vtable at this call, as EDX does.
                        site_draw_indexed_primitive(S2_DRAW_INDEXED_PRIMITIVE, device, vtable, vtable as u32, primitive_type, vertices, 0, primitives);
                    } else {
                        crate::link_client::draw_state(t, this, primitive_type, 0, 0, primitives, true, later_pass);
                        site_draw_primitive(S2_DRAW_PRIMITIVE, device, vtable, primitive_type, primitives);
                    }
                    index = index.wrapping_add(1);
                    partition += P::SIZE;
                    if index >= count {
                        break;
                    }
                }
            }
            if !more {
                break;
            }
        }
    }
    site_thiscall!(
        S2_ENSURE_AND_REGISTER_STREAMABLES,
        rd::<usize>(this, R::configurableTexturePipeline),
        rd::<usize>(this, R::currentPropertyState),
        rd::<usize>(this, R::currentEffectState),
        has_normals,
        1,
        geom_data,
        skin,
        transform,
        world_bound,
    );
}

/// The call is a skip when `skin` is not 0. `DrawSkinnedPrimitive2` then draws, and
/// `draw_regions` gives the reason. With no skin, `DrawSkinnedPrimitive2` returns immediately.
unsafe fn draw_skinned_primitive_2_regions(set: &mut Regions, this: usize, skin: usize) {
    set.add(this, R::SIZE);
    if skin != 0 {
        draw_regions(set);
    }
}

// ---------------------------------------------------------------------------------------
// EndBatch
// ---------------------------------------------------------------------------------------

/// The start of `EndBatch`: an empty state pointer of the batch gets the current state.
unsafe fn adopt_current_state(slot: usize, current: usize) {
    if rd::<usize>(slot, 0) == 0 && current != 0 {
        wr(slot, 0, current);
        ni::add_reference(current);
    }
}

/// The draw calls of one batched object in `EndBatch` (0x6AE4D3 to 0x6AE55D). Here the game
/// code calls the device directly. The Code Patch did not change these two calls.
unsafe fn end_batch_draw(t: &MainThread, this: usize, geom_buffer: usize) {
    if rd::<usize>(geom_buffer, GB::indexBuffer) != 0 {
        let mut strip: u32 = 0;
        let mut start_index: u32 = 0;
        if rd::<u32>(geom_buffer, GB::stripCount) != 0 {
            loop {
                let primitives = strip_primitives(geom_buffer, strip);
                let device: usize = rd(this, R::d3dDevice);
                let vertices: u32 = rd(geom_buffer, GB::verticesCount);
                let primitive_type: u32 = rd(geom_buffer, GB::primitiveType);
                crate::link_client::draw_state(t, this, primitive_type, vertices, start_index, primitives, false, false);
                device_draw_indexed_primitive(device, primitive_type, 0, vertices, start_index, primitives);
                start_index = start_index.wrapping_add(primitives).wrapping_add(2);
                strip = strip.wrapping_add(1);
                if strip >= rd::<u32>(geom_buffer, GB::stripCount) {
                    break;
                }
            }
        }
    } else {
        let primitives: u32 = rd(geom_buffer, GB::trianglesCount);
        let primitive_type: u32 = rd(geom_buffer, GB::primitiveType);
        crate::link_client::draw_state(t, this, primitive_type, 0, 0, primitives, false, false);
        device_draw_primitive(rd(this, R::d3dDevice), primitive_type, 0, primitives);
    }
}

/// 0x6AE090 `NiDX8Renderer::EndBatch`: draws the batched objects with one set-up of the
/// pass, then frees the list.
pub unsafe fn end_batch(t: &MainThread, this: usize) {
    crate::shadow::touch(&end_batch_port::STATE);
    let head: usize = rd(this, R::batchedObjectListHead);
    if head == 0 {
        return;
    }
    let first_buffer: usize = rd(head, B::geomBuffer);
    let mut geom_data: usize = rd(head, B::geomData);
    let mut skin: usize = rd(head, B::skinInstance);
    let mut transform: usize = rd(head, B::transform);
    let mut world_bound: usize = rd(head, B::worldBound);
    let Some((has_normals, texture_sets)) = vertex_format(first_buffer, geom_data) else {
        return;
    };
    adopt_current_state(this + R::propertyStatePtr, rd(this, R::currentPropertyState));
    adopt_current_state(this + R::effectStatePtr, rd(this, R::currentEffectState));
    let (prop1, prop2) = renderer_specific_objects(EB_GET_RENDERER_SPECIFIC, rd(this, R::propertyStatePtr));

    site_thiscall!(
        EB_SELECT_AND_LINK_PASS,
        rd::<usize>(this, R::configurableTexturePipeline),
        rd::<usize>(this, R::propertyStatePtr),
        rd::<usize>(this, R::effectStatePtr),
        has_normals,
        1,
        geom_data,
        skin,
        transform,
        world_bound,
    );
    site_thiscall!(EB_UPDATE_D3D_STATE, rd::<usize>(this, R::renderState), rd::<usize>(this, R::propertyStatePtr));
    let flags = site_thiscall!(
        EB_BUILD_PASS,
        rd::<usize>(this, R::configurableTexturePipeline),
        rd::<usize>(this, R::propertyStatePtr),
        rd::<usize>(this, R::effectStatePtr),
        has_normals,
        texture_sets,
        geom_data,
        skin,
        transform,
        world_bound,
    );
    if flags & BUILD_KEEP_LIGHTS == 0 {
        let property_state: usize = rd(this, R::propertyStatePtr);
        let vertex_color: usize = rd(property_state, PS::propVertexColor);
        let texturing: usize = rd(property_state, PS::propTexturing);
        let effect_state: usize = rd(this, R::effectStatePtr);
        site_thiscall!(EB_LIGHT_SET_STATE, rd::<usize>(this, R::lightManager), effect_state, texturing, vertex_color);
    }

    let mut passes: u32 = 0;
    if flags & BUILD_NO_DRAW == 0 {
        loop {
            let mut batch: usize = rd(this, R::batchedObjectListHead);
            geom_data = rd(batch, B::geomData);
            world_bound = rd(batch, B::worldBound);
            transform = rd(batch, B::transform);
            skin = rd(batch, B::skinInstance);
            let binary = site_thiscall!(EB_LOAD_BINARY, rd::<usize>(this, R::configurableTexturePipeline), geom_data, skin, transform, world_bound);
            let more = binary & 0xFFFF != 0;
            if binary & PASS_NO_DRAW == 0 {
                if binary & PASS_KEEP_PIXEL_SHADER == 0 {
                    site_thiscall!(EB_APPLY_PIXEL_SHADER, rd::<usize>(this, R::configurableTexturePipeline));
                }
                loop {
                    let mut geom_buffer: usize = rd(batch, B::geomBuffer);
                    geom_data = rd(batch, B::geomData);
                    world_bound = rd(batch, B::worldBound);
                    skin = rd(batch, B::skinInstance);
                    transform = rd(batch, B::transform);
                    if passes == 0 && binary & PASS_KEEP_BUFFERS == 0 {
                        let partition: usize = rd(batch, B::partition);
                        let out = core::ptr::addr_of_mut!(geom_buffer) as usize;
                        if partition != 0 {
                            site_thiscall!(EB_CREATE_PARTITION_GEOM_BUFFER, this, out, geom_data, skin, partition);
                        } else {
                            site_thiscall!(EB_PACK_GEOMETRY_BUFFERS, this, out, geom_data, skin, prop1, prop2);
                        }
                    }
                    if binary & PASS_KEEP_VERTEX_SHADER == 0 {
                        let fvf: u32 = rd(geom_buffer, GB::fvf);
                        site_thiscall!(EB_APPLY_VERTEX_SHADER, rd::<usize>(this, R::configurableTexturePipeline), fvf);
                    }
                    let partition: usize = rd(batch, B::partition);
                    if partition != 0 {
                        if flags & BUILD_KEEP_VERTEX_BLEND == 0 {
                            set_vertex_blend(this, rd(partition, P::numBones));
                        }
                        if flags & BUILD_KEEP_TRANSFORM == 0 {
                            let partition: usize = rd(batch, B::partition);
                            if !crate::camera_space::take_skinned_transforms(t, this, skin, partition, transform, false) {
                                site_thiscall!(EB_SET_SKINNED_MODEL_TRANSFORMS, this, skin, partition, transform, world_bound);
                                crate::camera_space::check_skinned_transforms(t, skin, partition, transform, false);
                            }
                            site_thiscall!(EB_BOUND_COPY, this + R::worldBound, world_bound);
                        }
                    } else {
                        set_vertex_blend(this, 0);
                        if flags & BUILD_KEEP_TRANSFORM == 0 {
                            site_thiscall!(EB_SET_MODEL_TRANSFORM, this, transform);
                            crate::camera_space::check_model_transform(t, this, transform, false);
                            site_thiscall!(EB_BOUND_COPY, this + R::worldBound, world_bound);
                        }
                    }
                    if binary & PASS_KEEP_STREAMS == 0 {
                        let vertex_buffer: usize = rd(geom_buffer, GB::d3dVertexBuffer);
                        let stride: u32 = rd(geom_buffer, GB::vertexStride);
                        device_set_stream_source(rd(this, R::d3dDevice), 0, vertex_buffer, stride);
                        let index_buffer: usize = rd(geom_buffer, GB::indexBuffer);
                        if index_buffer != 0 {
                            device_set_indices(rd(this, R::d3dDevice), index_buffer, 0);
                        }
                    }
                    if binary & PASS_KEEP_PARTITION_PASS == 0 {
                        build_partition_pass(
                            EB_BUILD_SKIN_PARTITION_PASS,
                            this,
                            rd(this, R::propertyStatePtr),
                            rd(this, R::effectStatePtr),
                            has_normals,
                            geom_data,
                            skin,
                            rd(batch, B::partition),
                            transform,
                            world_bound,
                            prop1,
                            prop2,
                        );
                    }
                    end_batch_draw(t, this, geom_buffer);
                    batch = rd(batch, B::nextBatch);
                    if batch == 0 {
                        break;
                    }
                }
                passes = passes.wrapping_add(1);
            }
            if !more {
                break;
            }
        }
    }
    // The arguments are the geometry data, skin, transform and bound of the object the loop
    // read last.
    site_thiscall!(
        EB_ENSURE_AND_REGISTER_STREAMABLES,
        rd::<usize>(this, R::configurableTexturePipeline),
        rd::<usize>(this, R::propertyStatePtr),
        rd::<usize>(this, R::effectStatePtr),
        has_normals,
        1,
        geom_data,
        skin,
        transform,
        world_bound,
    );
    ni::release_and_clear(this + R::propertyStatePtr);
    ni::release_and_clear(this + R::effectStatePtr);

    let mut batch: usize = rd(this, R::batchedObjectListHead);
    while batch != 0 {
        let next: usize = rd(batch, B::nextBatch);
        site_cdecl!(EB_DELETE, batch);
        batch = next;
    }
    wr::<usize>(this, R::batchedObjectListHead, 0);
    wr::<usize>(this, R::batchedObjectListTail, 0);
}

/// The call is a skip when the list has an object with a filled buffer or geometry data.
/// `EndBatch` then draws, and `draw_regions` gives the reason. `EndBatch` returns immediately
/// when the list is empty, or when its first object has no filled buffer and no geometry data.
unsafe fn end_batch_regions(set: &mut Regions, this: usize) {
    set.add(this, R::SIZE);
    let head: usize = rd(this, R::batchedObjectListHead);
    if head != 0 && vertex_format(rd(head, B::geomBuffer), rd(head, B::geomData)).is_some() {
        // `EndBatch` also frees the list with `operator delete`. A call without the skip
        // needs `set.allocates()` for that.
        draw_regions(set);
    }
}

// ---------------------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------------------

port_thiscall!(render_shape_port, 0x6ACEF0, "NiDX8Renderer::RenderShape",
    fn(this, geom_data: usize, skin: usize, transform: usize, world_bound: usize) -> (),
    compare_eax = false,
    regions = |set| render_regions(set, this, geom_data),
    rust_main = render_shape);

port_thiscall!(render_tri_strips_port, 0x6ACFC0, "NiDX8Renderer::RenderTriStrips",
    fn(this, geom_data: usize, skin: usize, transform: usize, world_bound: usize) -> (),
    compare_eax = false,
    regions = |set| render_regions(set, this, geom_data),
    rust_main = render_tri_strips);

port_thiscall!(draw_skinned_primitive_port, 0x6ADE70, "NiDX8Renderer::DrawSkinnedPrimitive",
    fn(this, geom_data: usize, skin: usize, transform: usize, world_bound: usize) -> (),
    compare_eax = false,
    // The call is always a skip. The function always gets to `DrawSkinnedPrimitive2`, which
    // draws. It can also make skin partitions (`MakePartitions`).
    regions = |set| draw_regions(set),
    rust = draw_skinned_primitive);

port_thiscall!(end_batch_port, 0x6AE090, "NiDX8Renderer::EndBatch",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| end_batch_regions(set, this),
    rust_main = end_batch);

port_thiscall!(draw_primitive_port, 0x6AEB80, "NiDX8Renderer::DrawPrimitive",
    fn(this, geom_data: usize, skin: usize, transform: usize, world_bound: usize, geom_buffer: usize, partition: usize) -> (),
    compare_eax = false,
    regions = |set| draw_primitive_regions(set, this, geom_data, geom_buffer),
    rust_main = draw_primitive);

port_thiscall!(draw_skinned_primitive_2_port, 0x6AEF90, "NiDX8Renderer::DrawSkinnedPrimitive2",
    fn(this, primitive_type: u32, geom_data: usize, skin: usize, transform: usize, world_bound: usize, unused_1: u32, unused_2: u32) -> (),
    compare_eax = false,
    regions = |set| draw_skinned_primitive_2_regions(set, this, skin),
    rust_main = draw_skinned_primitive_2);

pub static PORTS: &[&crate::shadow::PortState] = &[
    &render_shape_port::STATE,
    &render_tri_strips_port::STATE,
    &draw_skinned_primitive_port::STATE,
    &end_batch_port::STATE,
    &draw_primitive_port::STATE,
    &draw_skinned_primitive_2_port::STATE,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::census;
    use crate::hook;
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};

    // -----------------------------------------------------------------------------------
    // The call sites against the database
    // -----------------------------------------------------------------------------------

    /// The offset of each instruction of 0x6ACEF0, from the disassembly.
    #[rustfmt::skip]
    const INSTRUCTIONS_6ACEF0: &[u16] = &[
        0x0, 0x1, 0x2, 0x3, 0x5, 0xb, 0xd, 0x13, 0x17, 0x19, 0x1b, 0x1e, 0x21, 0x27, 0x2b, 0x2d, 0x2f, 0x36, 0x38, 0x3c,
        0x40, 0x41, 0x42, 0x43, 0x44, 0x46, 0x4b, 0x4c, 0x4d, 0x4e, 0x51, 0x55, 0x56, 0x57, 0x59, 0x61, 0x66, 0x68, 0x6a, 0x6e,
        0x75, 0x77, 0x79, 0x7c, 0x80, 0x85, 0x88, 0x8c, 0x8e, 0x95, 0x99, 0xa0, 0xa2, 0xa5, 0xa9, 0xac, 0xb0, 0xb4, 0xb8, 0xba,
        0xbb, 0xbc, 0xbd, 0xbe, 0xbf, 0xc1, 0xc6, 0xc7, 0xc8, 0xc9,
    ];
    /// The offset of each instruction of 0x6ACFC0, from the disassembly.
    #[rustfmt::skip]
    const INSTRUCTIONS_6ACFC0: &[u16] = &[
        0x0, 0x1, 0x2, 0x3, 0x5, 0xb, 0xd, 0x13, 0x17, 0x19, 0x1b, 0x1e, 0x21, 0x27, 0x2b, 0x2d, 0x2f, 0x36, 0x38, 0x3c,
        0x40, 0x41, 0x42, 0x43, 0x44, 0x46, 0x4b, 0x4c, 0x4d, 0x4e, 0x51, 0x55, 0x56, 0x57, 0x59, 0x61, 0x66, 0x68, 0x6a, 0x6e,
        0x70, 0x77, 0x7b, 0x7f, 0x81, 0x84, 0x89, 0x8c, 0x90, 0x96, 0x99, 0x9b, 0x9d, 0xa0, 0xa4, 0xa7, 0xaa, 0xae, 0xb1, 0xb4,
        0xb8, 0xbc, 0xc0, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc9, 0xce, 0xcf, 0xd0, 0xd1,
    ];
    /// The offset of each instruction of 0x6ADE70, from the disassembly.
    #[rustfmt::skip]
    const INSTRUCTIONS_6ADE70: &[u16] = &[
        0x0, 0x2, 0x7, 0xd, 0xe, 0x15, 0x18, 0x1c, 0x1d, 0x1e, 0x1f, 0x22, 0x24, 0x26, 0x28, 0x29, 0x2d, 0x2f, 0x32, 0x34,
        0x36, 0x38, 0x3d, 0x40, 0x44, 0x46, 0x4a, 0x4c, 0x4e, 0x53, 0x55, 0x5b, 0x5f, 0x60, 0x61, 0x62, 0x63, 0x65, 0x6d, 0x6f,
        0x74, 0x76, 0x78, 0x7c, 0x7f, 0x82, 0x85, 0x87, 0x89, 0x8b, 0x8d, 0x90, 0x91, 0x94, 0x96, 0x98, 0x9a, 0x9c, 0x9e, 0xa1,
        0xa3, 0xa6, 0xa9, 0xac, 0xb1, 0xb4, 0xb7, 0xb9, 0xbd, 0xbf, 0xc1, 0xc4, 0xc6, 0xca, 0xd0, 0xd4, 0xd6, 0xda, 0xdd, 0xdf,
        0xe1, 0xe5, 0xea, 0xee, 0xf2, 0xf4, 0xf7, 0xf9, 0xfc, 0x100, 0x104, 0x108, 0x10b, 0x10f, 0x110, 0x111, 0x112, 0x113, 0x115, 0x11a,
        0x11e, 0x120, 0x122, 0x126, 0x12a, 0x12c, 0x12d, 0x130, 0x134, 0x138, 0x13b, 0x13f, 0x143, 0x146, 0x14a, 0x14d, 0x151, 0x155, 0x159, 0x15c,
        0x160, 0x161, 0x165, 0x166, 0x167, 0x169, 0x16e, 0x172, 0x175, 0x176, 0x17a, 0x180, 0x184, 0x188, 0x18c, 0x18e, 0x190, 0x191, 0x195, 0x196,
        0x19a, 0x19b, 0x19c, 0x19d, 0x19f, 0x1a4, 0x1a8, 0x1a9, 0x1aa, 0x1ab, 0x1ac, 0x1b3, 0x1b6,
    ];
    /// The offset of each instruction of 0x6AE090, from the disassembly.
    #[rustfmt::skip]
    const INSTRUCTIONS_6AE090: &[u16] = &[
        0x0, 0x3, 0x4, 0x5, 0x6, 0x8, 0xa, 0xb, 0xe, 0x10, 0x16, 0x19, 0x1e, 0x22, 0x24, 0x27, 0x2a, 0x2d, 0x31, 0x35,
        0x38, 0x3a, 0x3c, 0x40, 0x42, 0x45, 0x48, 0x4a, 0x4f, 0x52, 0x55, 0x57, 0x5b, 0x5d, 0x5f, 0x65, 0x68, 0x6a, 0x6e, 0x70,
        0x74, 0x76, 0x7b, 0x7e, 0x80, 0x88, 0x8b, 0x8d, 0x8f, 0x91, 0x94, 0x96, 0x98, 0x9a, 0x9c, 0x9f, 0xa0, 0xa3, 0xa5, 0xa7,
        0xa9, 0xab, 0xae, 0xb0, 0xb3, 0xb5, 0xb8, 0xbb, 0xbd, 0xbf, 0xc1, 0xc4, 0xc6, 0xc8, 0xca, 0xcc, 0xcf, 0xd0, 0xd3, 0xd5,
        0xd7, 0xd9, 0xdb, 0xde, 0xe0, 0xe3, 0xe5, 0xe8, 0xeb, 0xef, 0xf3, 0xf8, 0xfa, 0xfc, 0xfe, 0x101, 0x103, 0x105, 0x107, 0x10a,
        0x10d, 0x10f, 0x112, 0x114, 0x116, 0x119, 0x11c, 0x120, 0x124, 0x128, 0x12c, 0x130, 0x133, 0x134, 0x137, 0x138, 0x139, 0x13a, 0x13c, 0x13d,
        0x13e, 0x144, 0x145, 0x14a, 0x14d, 0x153, 0x154, 0x159, 0x15d, 0x160, 0x163, 0x164, 0x168, 0x169, 0x16d, 0x16e, 0x16f, 0x170, 0x171, 0x172,
        0x173, 0x179, 0x17e, 0x183, 0x187, 0x189, 0x18c, 0x18f, 0x192, 0x195, 0x196, 0x19c, 0x19d, 0x19e, 0x1a3, 0x1a7, 0x1af, 0x1b4, 0x1ba, 0x1bd,
        0x1c0, 0x1c3, 0x1c7, 0x1ca, 0x1cd, 0x1d0, 0x1d1, 0x1d2, 0x1d6, 0x1dc, 0x1dd, 0x1de, 0x1e2, 0x1e7, 0x1ec, 0x1f1, 0x1f6, 0x1fa, 0x200, 0x202,
        0x208, 0x20a, 0x210, 0x215, 0x217, 0x21c, 0x220, 0x224, 0x229, 0x22d, 0x22f, 0x234, 0x23a, 0x23e, 0x242, 0x245, 0x248, 0x24c, 0x24f, 0x252,
        0x255, 0x259, 0x25d, 0x25f, 0x263, 0x267, 0x26b, 0x26d, 0x275, 0x277, 0x27a, 0x27e, 0x280, 0x282, 0x283, 0x284, 0x285, 0x286, 0x288, 0x28d,
        0x28f, 0x293, 0x297, 0x298, 0x299, 0x29a, 0x29b, 0x29c, 0x29e, 0x2a3, 0x2a7, 0x2ab, 0x2ad, 0x2af, 0x2b2, 0x2b8, 0x2b9, 0x2be, 0x2c2, 0x2c5,
        0x2c7, 0x2c9, 0x2d1, 0x2d3, 0x2d9, 0x2db, 0x2df, 0x2e6, 0x2ec, 0x2ee, 0x2f0, 0x2f6, 0x2f9, 0x2fa, 0x2ff, 0x301, 0x302, 0x308, 0x30c, 0x310,
        0x314, 0x316, 0x318, 0x31c, 0x320, 0x323, 0x324, 0x325, 0x326, 0x327, 0x329, 0x32e, 0x332, 0x333, 0x335, 0x33b, 0x341, 0x347, 0x349, 0x34f,
        0x352, 0x353, 0x358, 0x35a, 0x35b, 0x361, 0x365, 0x369, 0x36b, 0x36d, 0x371, 0x372, 0x374, 0x379, 0x37d, 0x37e, 0x384, 0x389, 0x38d, 0x391,
        0x393, 0x395, 0x398, 0x39b, 0x39e, 0x39f, 0x3a0, 0x3a3, 0x3a5, 0x3a7, 0x3a8, 0x3ae, 0x3b2, 0x3b5, 0x3b7, 0x3b9, 0x3bc, 0x3be, 0x3bf, 0x3c0,
        0x3c2, 0x3c8, 0x3cc, 0x3d0, 0x3d2, 0x3d4, 0x3d8, 0x3da, 0x3dc, 0x3df, 0x3e1, 0x3e3, 0x3e9, 0x3ec, 0x3f0, 0x3f1, 0x3f5, 0x3f6, 0x3fa, 0x3fb,
        0x3fe, 0x3ff, 0x403, 0x404, 0x407, 0x408, 0x409, 0x40b, 0x40c, 0x40f, 0x410, 0x411, 0x413, 0x415, 0x419, 0x41d, 0x420, 0x421, 0x425, 0x426,
        0x429, 0x42a, 0x42d, 0x42e, 0x42f, 0x431, 0x432, 0x433, 0x434, 0x43a, 0x43f, 0x443, 0x446, 0x448, 0x44a, 0x44c, 0x44e, 0x451, 0x455, 0x459,
        0x45b, 0x45e, 0x460, 0x462, 0x464, 0x468, 0x46a, 0x46d, 0x46f, 0x472, 0x475, 0x476, 0x477, 0x47a, 0x47d, 0x47f, 0x480, 0x482, 0x483, 0x484,
        0x48a, 0x48e, 0x492, 0x496, 0x49a, 0x49d, 0x49e, 0x4a0, 0x4a4, 0x4a8, 0x4aa, 0x4ae, 0x4b2, 0x4b4, 0x4b7, 0x4b9, 0x4bc, 0x4bf, 0x4c0, 0x4c2,
        0x4c3, 0x4c6, 0x4c7, 0x4cd, 0x4d0, 0x4d2, 0x4d8, 0x4dc, 0x4e0, 0x4e2, 0x4e8, 0x4ec, 0x4f0, 0x4f4, 0x4f7, 0x4f8, 0x4fb, 0x4fc, 0x4fd, 0x4fe,
        0x500, 0x501, 0x502, 0x503, 0x509, 0x50e, 0x511, 0x513, 0x515, 0x518, 0x519, 0x51c, 0x51e, 0x520, 0x522, 0x524, 0x526, 0x529, 0x52b, 0x52d,
        0x530, 0x532, 0x534, 0x537, 0x538, 0x53b, 0x53d, 0x53f, 0x541, 0x543, 0x546, 0x549, 0x54b, 0x54d, 0x550, 0x551, 0x556, 0x559, 0x55b, 0x55d,
        0x55f, 0x562, 0x565, 0x566, 0x567, 0x568, 0x569, 0x56c,
    ];
    /// The offset of each instruction of 0x6AEB80, from the disassembly.
    #[rustfmt::skip]
    const INSTRUCTIONS_6AEB80: &[u16] = &[
        0x0, 0x3, 0x7, 0x8, 0xc, 0xd, 0xe, 0x10, 0x13, 0x15, 0x17, 0x1c, 0x1e, 0x22, 0x24, 0x27, 0x29, 0x2b, 0x30, 0x33,
        0x36, 0x3a, 0x3c, 0x3e, 0x44, 0x46, 0x4a, 0x4e, 0x51, 0x53, 0x55, 0x5a, 0x5e, 0x63, 0x65, 0x67, 0x6b, 0x6e, 0x6f, 0x73,
        0x77, 0x7c, 0x7e, 0x80, 0x82, 0x85, 0x87, 0x89, 0x8b, 0x8e, 0x91, 0x93, 0x96, 0x98, 0x9a, 0x9d, 0xa0, 0xa4, 0xa8, 0xac,
        0xb0, 0xb4, 0xb8, 0xbb, 0xbc, 0xbd, 0xc0, 0xc1, 0xc2, 0xc4, 0xc5, 0xcb, 0xcc, 0xcd, 0xd2, 0xd5, 0xd6, 0xdc, 0xe1, 0xe5,
        0xe8, 0xeb, 0xec, 0xed, 0xf1, 0xf2, 0xf3, 0xf4, 0xf8, 0xf9, 0xfa, 0xfb, 0x101, 0x106, 0x108, 0x10c, 0x10e, 0x112, 0x114, 0x11a,
        0x11c, 0x122, 0x124, 0x128, 0x12f, 0x135, 0x137, 0x13d, 0x140, 0x141, 0x146, 0x148, 0x149, 0x14f, 0x153, 0x15b, 0x15d, 0x161, 0x162, 0x163,
        0x164, 0x165, 0x167, 0x16c, 0x16e, 0x174, 0x17a, 0x180, 0x182, 0x188, 0x18b, 0x18c, 0x191, 0x193, 0x194, 0x19a, 0x1a2, 0x1a4, 0x1a8, 0x1aa,
        0x1ab, 0x1b0, 0x1b6, 0x1b7, 0x1bc, 0x1c4, 0x1c6, 0x1c9, 0x1cc, 0x1cf, 0x1d2, 0x1d3, 0x1d9, 0x1da, 0x1db, 0x1e0, 0x1e4, 0x1ec, 0x1f1, 0x1f7,
        0x1fb, 0x1fc, 0x1fd, 0x203, 0x204, 0x205, 0x20a, 0x20c, 0x212, 0x217, 0x21d, 0x223, 0x227, 0x229, 0x22a, 0x22c, 0x230, 0x232, 0x238, 0x23a,
        0x23e, 0x240, 0x242, 0x243, 0x244, 0x248, 0x249, 0x24a, 0x24c, 0x251, 0x253, 0x257, 0x25b, 0x25c, 0x25d, 0x25e, 0x262, 0x263, 0x264, 0x266,
        0x26b, 0x271, 0x273, 0x279, 0x27e, 0x284, 0x286, 0x28a, 0x28d, 0x290, 0x292, 0x293, 0x297, 0x29a, 0x29b, 0x29d, 0x29e, 0x2a4, 0x2a8, 0x2ab,
        0x2ad, 0x2af, 0x2b2, 0x2b4, 0x2b5, 0x2b6, 0x2b8, 0x2be, 0x2c2, 0x2c8, 0x2ca, 0x2cd, 0x2d3, 0x2d4, 0x2d9, 0x2dd, 0x2e3, 0x2e5, 0x2e9, 0x2eb,
        0x2ed, 0x2f0, 0x2f2, 0x2f4, 0x2fa, 0x2fd, 0x301, 0x302, 0x306, 0x307, 0x30b, 0x30c, 0x310, 0x311, 0x315, 0x316, 0x319, 0x31a, 0x31b, 0x31d,
        0x31e, 0x321, 0x322, 0x323, 0x325, 0x327, 0x32b, 0x32f, 0x333, 0x334, 0x338, 0x339, 0x33c, 0x33d, 0x340, 0x341, 0x342, 0x344, 0x345, 0x346,
        0x347, 0x34d, 0x352, 0x356, 0x359, 0x35b, 0x35d, 0x360, 0x362, 0x364, 0x366, 0x36a, 0x36c, 0x36f, 0x371, 0x373, 0x375, 0x379, 0x37c, 0x37e,
        0x381, 0x384, 0x387, 0x38a, 0x38b, 0x38d, 0x38e, 0x38f, 0x391, 0x392, 0x393, 0x394, 0x399, 0x39d, 0x3a1, 0x3a2, 0x3a6, 0x3a9, 0x3ad, 0x3af,
        0x3b1, 0x3b5, 0x3b9, 0x3bb, 0x3be, 0x3c1, 0x3c4, 0x3c5, 0x3c7, 0x3c9, 0x3ca, 0x3cb, 0x3d1, 0x3d5, 0x3d9, 0x3db, 0x3e1, 0x3e5, 0x3e9, 0x3ec,
        0x3ed, 0x3ee, 0x3f1, 0x3f2, 0x3f3, 0x3f5, 0x3f6, 0x3f7, 0x3f8, 0x3fe, 0x403, 0x404, 0x405, 0x406, 0x407, 0x40a,
    ];
    /// The offset of each instruction of 0x6AEF90, from the disassembly.
    #[rustfmt::skip]
    const INSTRUCTIONS_6AEF90: &[u16] = &[
        0x0, 0x3, 0x4, 0x5, 0x6, 0x8, 0xc, 0xd, 0xf, 0x15, 0x18, 0x1d, 0x1f, 0x21, 0x24, 0x28, 0x2a, 0x32, 0x36, 0x39,
        0x3c, 0x40, 0x42, 0x45, 0x47, 0x49, 0x4d, 0x4f, 0x52, 0x56, 0x58, 0x5a, 0x5f, 0x61, 0x65, 0x67, 0x6a, 0x6c, 0x6e, 0x73,
        0x76, 0x79, 0x7b, 0x7d, 0x81, 0x83, 0x86, 0x8a, 0x8c, 0x8e, 0x93, 0x97, 0x9b, 0x9f, 0xa2, 0xa3, 0xa4, 0xa7, 0xa8, 0xac,
        0xad, 0xaf, 0xb0, 0xb6, 0xb7, 0xb8, 0xbd, 0xc0, 0xc1, 0xc7, 0xcc, 0xd0, 0xd3, 0xd6, 0xd7, 0xd8, 0xdc, 0xdd, 0xe1, 0xe2,
        0xe3, 0xe4, 0xe5, 0xe6, 0xec, 0xf1, 0xf3, 0xf9, 0xfd, 0xff, 0x102, 0x105, 0x108, 0x10b, 0x10c, 0x112, 0x113, 0x114, 0x119, 0x11d,
        0x123, 0x126, 0x12a, 0x130, 0x134, 0x138, 0x139, 0x13a, 0x13b, 0x141, 0x142, 0x147, 0x14c, 0x151, 0x156, 0x15a, 0x15c, 0x162, 0x167, 0x168,
        0x16e, 0x173, 0x177, 0x17f, 0x181, 0x187, 0x18b, 0x18d, 0x192, 0x198, 0x19c, 0x1a0, 0x1a2, 0x1a6, 0x1aa, 0x1ac, 0x1ae, 0x1b4, 0x1b6, 0x1ba,
        0x1c1, 0x1c7, 0x1c9, 0x1cb, 0x1d1, 0x1d4, 0x1d5, 0x1da, 0x1dc, 0x1dd, 0x1e3, 0x1e5, 0x1e7, 0x1eb, 0x1ef, 0x1f0, 0x1f3, 0x1f4, 0x1f5, 0x1f6,
        0x1f8, 0x1fd, 0x200, 0x202, 0x208, 0x20c, 0x20f, 0x214, 0x216, 0x219, 0x21f, 0x220, 0x225, 0x22d, 0x22f, 0x233, 0x237, 0x238, 0x23c, 0x23f,
        0x240, 0x244, 0x245, 0x249, 0x24a, 0x24d, 0x24e, 0x251, 0x253, 0x254, 0x255, 0x25b, 0x25c, 0x261, 0x269, 0x26b, 0x26e, 0x271, 0x272, 0x275,
        0x277, 0x278, 0x27a, 0x27b, 0x281, 0x284, 0x287, 0x289, 0x28a, 0x28c, 0x28d, 0x293, 0x296, 0x297, 0x299, 0x29c, 0x29e, 0x2a0, 0x2a2, 0x2a4,
        0x2a8, 0x2a9, 0x2ad, 0x2af, 0x2b0, 0x2b1, 0x2b3, 0x2b8, 0x2ba, 0x2be, 0x2bf, 0x2c0, 0x2c5, 0x2c9, 0x2cd, 0x2d1, 0x2d2, 0x2d5, 0x2d7, 0x2db,
        0x2e1, 0x2e5, 0x2e9, 0x2ed, 0x2ef, 0x2f5, 0x2f9, 0x2fd, 0x301, 0x302, 0x303, 0x306, 0x307, 0x30a, 0x30b, 0x30d, 0x30e, 0x30f, 0x310, 0x316,
        0x31b, 0x31c, 0x31d, 0x31e, 0x31f, 0x322,
    ];

    #[test]
    fn sites_are_the_direct_calls_of_the_six_functions() {
        let functions: [(usize, &[u16]); 6] = [
            (0x6ACEF0, INSTRUCTIONS_6ACEF0),
            (0x6ACFC0, INSTRUCTIONS_6ACFC0),
            (0x6ADE70, INSTRUCTIONS_6ADE70),
            (0x6AE090, INSTRUCTIONS_6AE090),
            (0x6AEB80, INSTRUCTIONS_6AEB80),
            (0x6AEF90, INSTRUCTIONS_6AEF90),
        ];
        // Each site is a `call rel32` inside one of the six functions.
        for &site in SITES {
            let (ea, _) = functions
                .iter()
                .find(|(ea, _)| site >= *ea && site < *ea + hook::find(*ea).unwrap().size)
                .unwrap_or_else(|| panic!("{site:#x} is in none of the functions"));
            let bytes = census::expected(hook::find(*ea).unwrap());
            assert_eq!(bytes[site - ea], 0xE8, "{site:#x} is not a call");
        }
        // Each `call rel32` of the six functions is a site.
        let mut calls = 0;
        for (ea, instructions) in functions {
            let func = hook::find(ea).unwrap();
            let bytes = census::expected(func);
            assert_eq!(instructions[0], 0);
            for pair in instructions.windows(2) {
                assert!(pair[0] < pair[1], "{ea:#x}: the instruction list is not in order");
            }
            assert!((*instructions.last().unwrap() as usize) < func.size);
            for &offset in instructions {
                if bytes[offset as usize] == 0xE8 {
                    calls += 1;
                    assert!(SITES.contains(&(ea + offset as usize)), "the call at {:#x} is not in SITES", ea + offset as usize);
                }
            }
        }
        assert_eq!(calls, SITES.len());
        let mut unique = SITES.to_vec();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), SITES.len());
        for &site in SITES {
            assert!(crate::ports::serves_site(site));
        }
    }

    // -----------------------------------------------------------------------------------
    // Functions that replace the callees and record the calls
    // -----------------------------------------------------------------------------------

    pub static VERTEX_BLEND_TABLE: [u32; 5] = [0, 0, 1, 2, 3];

    thread_local! {
        /// The bytes the tests have at 0x6ACF1F.
        pub static SKINNING_CHECK_BYTES: std::cell::Cell<[u8; 8]> = const { std::cell::Cell::new(SKINNING_CHECK_DATABASE) };
    }

    #[test]
    fn the_skinning_check_of_render_shape_is_recognised() {
        // The database has the bytes of the game at the place of the check.
        let func = hook::find(0x6ACEF0).unwrap();
        let at = RS_SKINNING_CHECK - func.ea;
        assert_eq!(at, 0x2F);
        assert_eq!(&census::expected(func)[at..at + 8], &SKINNING_CHECK_DATABASE);
        assert_eq!(skinning_check(&SKINNING_CHECK_DATABASE), SkinningCheck::HardwareBones);
        assert_eq!(skinning_check(&SKINNING_CHECK_MWSE), SkinningCheck::GeometryFlag);
        // All other bytes count as the database. Examples are bytes with one byte different,
        // and the form of MGE XE, which this code does not know.
        let mut other = SKINNING_CHECK_MWSE;
        other[7] = 0x72;
        assert_eq!(skinning_check(&other), SkinningCheck::HardwareBones);
        assert_eq!(skinning_check(&[0x90; 8]), SkinningCheck::HardwareBones);

        let mut renderer = vec![0u8; R::SIZE];
        let r = renderer.as_mut_ptr() as usize;
        let mut geom = [0u16; 0x20];
        let g = geom.as_mut_ptr() as usize;
        unsafe {
            wr::<u32>(r, R::countHWBones, 3);
            assert!(software_skinning(SkinningCheck::HardwareBones, r, g));
            assert!(!software_skinning(SkinningCheck::GeometryFlag, r, g));
            wr::<u32>(r, R::countHWBones, 4);
            assert!(!software_skinning(SkinningCheck::HardwareBones, r, g));
            wr::<u16>(g, GEOM_DATA_PATCH_RENDER_FLAGS, 0x200);
            assert!(software_skinning(SkinningCheck::GeometryFlag, r, g));
            assert!(!software_skinning(SkinningCheck::HardwareBones, r, g));
            wr::<u16>(g, GEOM_DATA_PATCH_RENDER_FLAGS, 0xFDFF);
            assert!(!software_skinning(SkinningCheck::GeometryFlag, r, g));
        }
    }

    // Ids of the calls that are not sites.
    const DEV_SET_RENDER_STATE: usize = 50;
    const DEV_DRAW_PRIMITIVE: usize = 70;
    const DEV_DRAW_INDEXED_PRIMITIVE: usize = 71;
    const DEV_SET_STREAM_SOURCE: usize = 83;
    const DEV_SET_INDICES: usize = 85;
    const GEOM_VERTEX_COUNT: usize = 1;
    const GEOM_TRIANGLE_COUNT: usize = 2;
    const GEOM_TRI_LIST: usize = 3;
    const OBJECT_DELETE: usize = 4;
    const RENDER_DATA_TYPE: usize = 5;
    const PROP_CALLBACK: usize = 6;

    thread_local! {
        static CALLS: RefCell<Vec<(usize, Vec<u32>)>> = const { RefCell::new(Vec::new()) };
        static RETURNS: RefCell<HashMap<usize, VecDeque<u32>>> = RefCell::new(HashMap::new());
        /// For a call id: the value to write to the address in argument `index`.
        static POKES: RefCell<HashMap<usize, (usize, u32)>> = RefCell::new(HashMap::new());
    }

    fn note(id: usize, args: &[u32]) -> u32 {
        CALLS.with(|calls| calls.borrow_mut().push((id, args.to_vec())));
        if let Some((index, value)) = POKES.with(|pokes| pokes.borrow().get(&id).copied()) {
            unsafe { wr(args[index] as usize, 0, value) };
        }
        RETURNS.with(|returns| returns.borrow_mut().get_mut(&id).and_then(|queue| queue.pop_front())).unwrap_or(0)
    }

    fn reset() {
        CALLS.with(|calls| calls.borrow_mut().clear());
        RETURNS.with(|returns| returns.borrow_mut().clear());
        POKES.with(|pokes| pokes.borrow_mut().clear());
    }

    fn returns(id: usize, values: &[u32]) {
        RETURNS.with(|returns| returns.borrow_mut().insert(id, values.iter().copied().collect()));
    }

    fn poke(id: usize, index: usize, value: u32) {
        POKES.with(|pokes| pokes.borrow_mut().insert(id, (index, value)));
    }

    fn calls() -> Vec<(usize, Vec<u32>)> {
        CALLS.with(|calls| calls.borrow_mut().drain(..).collect())
    }

    unsafe extern "thiscall" fn t0<const S: usize>(this: usize) -> u32 {
        note(S, &[this as u32])
    }
    unsafe extern "thiscall" fn t1<const S: usize>(this: usize, a: u32) -> u32 {
        note(S, &[this as u32, a])
    }
    unsafe extern "thiscall" fn t2<const S: usize>(this: usize, a: u32, b: u32) -> u32 {
        note(S, &[this as u32, a, b])
    }
    unsafe extern "thiscall" fn t3<const S: usize>(this: usize, a: u32, b: u32, c: u32) -> u32 {
        note(S, &[this as u32, a, b, c])
    }
    unsafe extern "thiscall" fn t4<const S: usize>(this: usize, a: u32, b: u32, c: u32, d: u32) -> u32 {
        note(S, &[this as u32, a, b, c, d])
    }
    unsafe extern "thiscall" fn t5<const S: usize>(this: usize, a: u32, b: u32, c: u32, d: u32, e: u32) -> u32 {
        note(S, &[this as u32, a, b, c, d, e])
    }
    unsafe extern "thiscall" fn t6<const S: usize>(this: usize, a: u32, b: u32, c: u32, d: u32, e: u32, f: u32) -> u32 {
        note(S, &[this as u32, a, b, c, d, e, f])
    }
    unsafe extern "thiscall" fn t7<const S: usize>(this: usize, a: u32, b: u32, c: u32, d: u32, e: u32, f: u32, g: u32) -> u32 {
        note(S, &[this as u32, a, b, c, d, e, f, g])
    }
    unsafe extern "thiscall" fn t8<const S: usize>(this: usize, a: u32, b: u32, c: u32, d: u32, e: u32, f: u32, g: u32, h: u32) -> u32 {
        note(S, &[this as u32, a, b, c, d, e, f, g, h])
    }
    #[allow(clippy::too_many_arguments)]
    unsafe extern "thiscall" fn t9<const S: usize>(this: usize, a: u32, b: u32, c: u32, d: u32, e: u32, f: u32, g: u32, h: u32, i: u32) -> u32 {
        note(S, &[this as u32, a, b, c, d, e, f, g, h, i])
    }
    unsafe extern "C" fn c1<const S: usize>(a: u32) -> u32 {
        note(S, &[a])
    }
    unsafe extern "stdcall" fn s3<const S: usize>(a: u32, b: u32, c: u32) -> u32 {
        note(S, &[a, b, c])
    }
    unsafe extern "stdcall" fn s4<const S: usize>(a: u32, b: u32, c: u32, d: u32) -> u32 {
        note(S, &[a, b, c, d])
    }
    unsafe extern "stdcall" fn s6<const S: usize>(a: u32, b: u32, c: u32, d: u32, e: u32, f: u32) -> u32 {
        note(S, &[a, b, c, d, e, f])
    }
    #[allow(clippy::too_many_arguments)]
    unsafe extern "stdcall" fn s11<const S: usize>(a: u32, b: u32, c: u32, d: u32, e: u32, f: u32, g: u32, h: u32, i: u32, j: u32, k: u32) -> u32 {
        note(S, &[a, b, c, d, e, f, g, h, i, j, k])
    }

    macro_rules! fakes {
        ($site:ident; $($f:ident: $($s:ident),+;)+) => {
            $($(if $site == $s { return $f::<{ $s }> as *const () as usize; })+)+
        };
    }

    /// The function a call site has in the tests. It has the calling convention and the count
    /// of arguments of the callee in the database.
    pub fn fake_target(site: usize) -> usize {
        fakes!(site;
            t0: RT_GET_STRIP_LENGTH_SUM, SP_SKIN_PARTITION_CTOR, EB_GET_RENDERER_SPECIFIC, DP_GET_RENDERER_SPECIFIC,
                EB_APPLY_PIXEL_SHADER, DP_APPLY_PIXEL_SHADER, S2_APPLY_PIXEL_SHADER;
            t1: EB_UPDATE_D3D_STATE, DP_UPDATE_D3D_STATE, S2_UPDATE_D3D_STATE, EB_SET_MODEL_TRANSFORM, DP_SET_MODEL_TRANSFORM,
                EB_BOUND_COPY, DP_BOUND_COPY, S2_BOUND_COPY, EB_APPLY_VERTEX_SHADER, DP_APPLY_VERTEX_SHADER, S2_APPLY_VERTEX_SHADER;
            t2: RS_CREATE_GEOMETRY_BUFFER_DATA, RT_CREATE_GEOMETRY_BUFFER_DATA;
            t3: EB_LIGHT_SET_STATE, DP_LIGHT_SET_STATE, S2_LIGHT_SET_STATE;
            t4: RS_DRAW_SKINNED_PRIMITIVE, RT_DRAW_SKINNED_PRIMITIVE, SP_MAKE_PARTITIONS, SP_FIND_CACHED_PARTITION_GEOM_BUFFER,
                SP_CREATE_PARTITION_GEOM_BUFFER, EB_CREATE_PARTITION_GEOM_BUFFER, DP_CREATE_PARTITION_GEOM_BUFFER,
                EB_SET_SKINNED_MODEL_TRANSFORMS, DP_SET_SKINNED_MODEL_TRANSFORMS, S2_SET_SKINNED_MODEL_TRANSFORMS,
                EB_LOAD_BINARY, DP_LOAD_BINARY, S2_LOAD_BINARY;
            t5: EB_PACK_GEOMETRY_BUFFERS, DP_PACK_GEOMETRY_BUFFERS;
            t6: RS_DRAW_PRIMITIVE, RT_DRAW_PRIMITIVE;
            t7: SP_DRAW_SKINNED_PRIMITIVE_2;
            t8: EB_SELECT_AND_LINK_PASS, DP_SELECT_AND_LINK_PASS, S2_SELECT_AND_LINK_PASS, EB_BUILD_PASS, DP_BUILD_PASS, S2_BUILD_PASS,
                EB_ENSURE_AND_REGISTER_STREAMABLES, DP_ENSURE_AND_REGISTER_STREAMABLES, S2_ENSURE_AND_REGISTER_STREAMABLES;
            t9: EB_BUILD_SKIN_PARTITION_PASS, DP_BUILD_SKIN_PARTITION_PASS, S2_BUILD_SKIN_PARTITION_PASS;
            c1: SP_OPERATOR_NEW, EB_DELETE;
            s6: DP_DRAW_INDEXED_PRIMITIVE, S2_DRAW_INDEXED_PRIMITIVE;
            s4: S2_DRAW_PRIMITIVE;
        );
        panic!("no function for the site {site:#x}");
    }

    #[test]
    fn every_site_has_a_function_in_the_tests() {
        for &site in SITES {
            assert_ne!(fake_target(site), 0);
        }
    }

    /// An object that is one pointer to a vtable of `slots` entries.
    struct Object {
        _vtable: Box<[usize]>,
        object: Box<[usize; 4]>,
    }

    impl Object {
        fn new(slots: &[(usize, usize)]) -> Object {
            let size = slots.iter().map(|(slot, _)| slot + 1).max().unwrap();
            let mut vtable = vec![0usize; size].into_boxed_slice();
            for &(slot, function) in slots {
                vtable[slot] = function;
            }
            let object = Box::new([vtable.as_ptr() as usize, 0, 0, 0]);
            Object { _vtable: vtable, object }
        }
        fn address(&self) -> usize {
            self.object.as_ptr() as usize
        }
    }

    fn device() -> Object {
        Object::new(&[
            (50, s3::<DEV_SET_RENDER_STATE> as *const () as usize),
            (70, s4::<DEV_DRAW_PRIMITIVE> as *const () as usize),
            (71, s6::<DEV_DRAW_INDEXED_PRIMITIVE> as *const () as usize),
            (83, s4::<DEV_SET_STREAM_SOURCE> as *const () as usize),
            (85, s3::<DEV_SET_INDICES> as *const () as usize),
        ])
    }

    /// Geometry data: the three virtual functions, and room for the members.
    struct Geometry {
        _vtable: Box<[usize]>,
        data: Vec<u32>,
    }

    fn geometry() -> Geometry {
        let mut vtable = vec![0usize; 18].into_boxed_slice();
        vtable[VT_GET_ACTIVE_VERTEX_COUNT / 4] = t0::<GEOM_VERTEX_COUNT> as *const () as usize;
        vtable[VT_GET_ACTIVE_TRIANGLE_COUNT / 4] = t0::<GEOM_TRIANGLE_COUNT> as *const () as usize;
        vtable[VT_GET_TRI_LIST / 4] = t0::<GEOM_TRI_LIST> as *const () as usize;
        let mut data = vec![0u32; NiTriStripsData::SIZE / 4];
        data[0] = vtable.as_ptr() as u32;
        Geometry { _vtable: vtable, data }
    }

    /// A renderer with a device, a render state and the other objects as plain numbers.
    struct World {
        renderer: Vec<u8>,
        _render_state: Vec<u8>,
        _pipeline: Vec<u32>,
        property_state: Vec<u32>,
        device: Object,
    }

    const LIGHT_MANAGER: usize = 0x1100;
    const EFFECT_STATE: usize = 0x1200;
    const PASS: u32 = 0x1300;
    const TEXTURING: u32 = 0x1400;
    const VERTEX_COLOR: u32 = 0x1500;

    fn world() -> World {
        reset();
        let mut renderer = vec![0u8; R::SIZE];
        let mut render_state = vec![0u8; RS::SIZE];
        let mut pipeline = vec![0u32; 4];
        let mut property_state = vec![0u32; PS::SIZE / 4];
        let device = device();
        pipeline[CTP::field_8 / 4] = PASS;
        property_state[PS::propTexturing / 4] = TEXTURING;
        property_state[PS::propVertexColor / 4] = VERTEX_COLOR;
        let r = renderer.as_mut_ptr() as usize;
        unsafe {
            wr(render_state.as_mut_ptr() as usize, RS::device, device.address());
            wr(r, R::d3dDevice, device.address());
            wr(r, R::renderState, render_state.as_mut_ptr() as usize);
            wr(r, R::configurableTexturePipeline, pipeline.as_mut_ptr() as usize);
            wr(r, R::lightManager, LIGHT_MANAGER);
            wr(r, R::currentPropertyState, property_state.as_mut_ptr() as usize);
            wr(r, R::currentEffectState, EFFECT_STATE);
            wr::<u8>(r, R::bSceneOpen, 1);
        }
        World { renderer, _render_state: render_state, _pipeline: pipeline, property_state, device }
    }

    impl World {
        fn r(&mut self) -> usize {
            self.renderer.as_mut_ptr() as usize
        }
        fn rs(&mut self) -> u32 {
            unsafe { rd(self.r(), R::renderState) }
        }
        fn pipeline(&mut self) -> u32 {
            unsafe { rd(self.r(), R::configurableTexturePipeline) }
        }
        fn ps(&mut self) -> u32 {
            self.property_state.as_mut_ptr() as u32
        }
        fn dev(&self) -> u32 {
            self.device.address() as u32
        }
    }

    fn buffer() -> Vec<u32> {
        vec![0u32; GB::SIZE / 4]
    }

    // -----------------------------------------------------------------------------------
    // The shared parts
    // -----------------------------------------------------------------------------------

    #[test]
    fn vertex_format_from_the_buffer_or_the_geometry_data() {
        let mut buffer = buffer();
        let mut geom = geometry();
        let (b, g) = (buffer.as_mut_ptr() as usize, geom.data.as_mut_ptr() as usize);
        unsafe {
            // There is no vertex buffer and no geometry data.
            assert_eq!(vertex_format(b, 0), None);
            // The geometry data gives 3 texture sets and no normals.
            wr::<u16>(g, GD::textureSets, 3);
            assert_eq!(vertex_format(b, g), Some((0, 3)));
            wr::<u32>(g, GD::normals, 0x100);
            wr::<u16>(g, GD::textureSets, 9);
            assert_eq!(vertex_format(b, g), Some((1, 8)));
            // The port does not use a vertex buffer that has no "filled" flag.
            wr::<u32>(b, GB::d3dVertexBuffer, 0x200);
            wr::<u32>(b, GB::fvf, 0x212);
            assert_eq!(vertex_format(b, g), Some((1, 8)));
            wr::<u32>(b, GB::flags, 1);
            assert_eq!(vertex_format(b, g), Some((1, 2)));
            assert_eq!(vertex_format(b, 0), Some((1, 2)));
            wr::<u32>(b, GB::fvf, 0xF02);
            assert_eq!(vertex_format(b, 0), Some((0, 8)));
        }
    }

    #[test]
    fn vertex_blend_goes_to_the_device_when_it_changes() {
        let mut w = world();
        let r = w.r();
        unsafe {
            set_vertex_blend(r, 0);
            assert_eq!(calls(), vec![]);
            set_vertex_blend(r, 3);
            assert_eq!(calls(), vec![(DEV_SET_RENDER_STATE, vec![w.dev(), 151, 2])]);
            assert_eq!(rd::<u32>(w.rs() as usize, RS_VERTEX_BLEND_CURRENT), 2);
            set_vertex_blend(r, 3);
            assert_eq!(calls(), vec![]);
        }
    }

    #[test]
    fn end_batch_draw_goes_through_the_strips() {
        let mut w = world();
        let r = w.r();
        let mut b = buffer();
        let gb = b.as_mut_ptr() as usize;
        let lengths = [5u16, 7, 3];
        unsafe {
            wr::<u32>(gb, GB::primitiveType, 5);
            wr::<u32>(gb, GB::trianglesCount, 11);
            wr::<u32>(gb, GB::verticesCount, 40);
            // No index buffer: the port makes one DrawPrimitive call.
            end_batch_draw(&MainThread::for_test(), r, gb);
            assert_eq!(calls(), vec![(DEV_DRAW_PRIMITIVE, vec![w.dev(), 5, 0, 11])]);
            // An index buffer and no strips: the port makes no call.
            wr::<u32>(gb, GB::indexBuffer, 0x300);
            end_batch_draw(&MainThread::for_test(), r, gb);
            assert_eq!(calls(), vec![]);
            // One "strip" without lengths: the port draws the triangle count.
            wr::<u32>(gb, GB::stripCount, 1);
            end_batch_draw(&MainThread::for_test(), r, gb);
            assert_eq!(calls(), vec![(DEV_DRAW_INDEXED_PRIMITIVE, vec![w.dev(), 5, 0, 40, 0, 11])]);
            // Three strips.
            wr::<u32>(gb, GB::stripCount, 3);
            wr(gb, GB::stripVertexCounts, lengths.as_ptr() as usize);
            end_batch_draw(&MainThread::for_test(), r, gb);
            assert_eq!(
                calls(),
                vec![
                    (DEV_DRAW_INDEXED_PRIMITIVE, vec![w.dev(), 5, 0, 40, 0, 3]),
                    (DEV_DRAW_INDEXED_PRIMITIVE, vec![w.dev(), 5, 0, 40, 5, 5]),
                    (DEV_DRAW_INDEXED_PRIMITIVE, vec![w.dev(), 5, 0, 40, 12, 1]),
                ]
            );
        }
    }

    // -----------------------------------------------------------------------------------
    // RenderShape, RenderTriStrips
    // -----------------------------------------------------------------------------------

    #[test]
    fn render_shape_fills_a_new_buffer_and_draws() {
        let mut w = world();
        let r = w.r();
        let mut geom = geometry();
        let g = geom.data.as_mut_ptr() as usize;
        let mut b = buffer();
        let gb = b.as_mut_ptr() as usize;
        unsafe {
            // The scene is not open: the port makes no call.
            wr::<u8>(r, R::bSceneOpen, 0);
            render_shape(&MainThread::for_test(), r, g, 0, 0x10, 0x20);
            assert_eq!(calls(), vec![]);
            let mut set = Regions::new();
            render_regions(&mut set, r, g);
            assert_eq!(set.as_slice(), &[(r, R::SIZE)]);
            assert_eq!(calls(), vec![]);

            // No vertices: the port makes no call after it gets the vertex count.
            wr::<u8>(r, R::bSceneOpen, 1);
            returns(GEOM_VERTEX_COUNT, &[0x1_0000]);
            render_shape(&MainThread::for_test(), r, g, 0, 0x10, 0x20);
            assert_eq!(calls(), vec![(GEOM_VERTEX_COUNT, vec![g as u32])]);

            // A new buffer: the port fills it as a triangle list.
            returns(GEOM_VERTEX_COUNT, &[9]);
            returns(RS_CREATE_GEOMETRY_BUFFER_DATA, &[0x7701]);
            poke(RS_CREATE_GEOMETRY_BUFFER_DATA, 1, gb as u32);
            returns(GEOM_TRIANGLE_COUNT, &[0xABCD_0007]);
            returns(GEOM_TRI_LIST, &[0x4444]);
            // A skin, and a device without hardware bones: the port does not use the skinned
            // path.
            wr::<u32>(r, R::countHWBones, 3);
            render_shape(&MainThread::for_test(), r, g, 0x30, 0x10, 0x20);
            let log = calls();
            assert_eq!(log[0].0, GEOM_VERTEX_COUNT);
            assert_eq!(log[1].0, RS_CREATE_GEOMETRY_BUFFER_DATA);
            assert_eq!((log[1].1[0], log[1].1[2]), (r as u32, g as u32));
            assert_eq!(log[2].0, GEOM_TRIANGLE_COUNT);
            assert_eq!(log[3].0, GEOM_TRI_LIST);
            assert_eq!(log[4], (RS_DRAW_PRIMITIVE, vec![r as u32, g as u32, 0x30, 0x10, 0x20, gb as u32, 0]));
            assert_eq!(log.len(), 5);
            assert_eq!(rd::<u32>(gb, GB::primitiveType), 4);
            assert_eq!(rd::<u32>(gb, GB::trianglesCount), 7);
            assert_eq!(rd::<u32>(gb, GB::stripCount), 1);
            assert_eq!(rd::<u32>(gb, GB::stripVertexCounts), 0);
            assert_eq!(rd::<u32>(gb, GB::triList), 0x4444);

            // A buffer that exists: the callee returns 0 and the port draws the buffer with no
            // change.
            poke(RS_CREATE_GEOMETRY_BUFFER_DATA, 1, 0x8888);
            returns(GEOM_VERTEX_COUNT, &[9]);
            render_shape(&MainThread::for_test(), r, g, 0, 0x10, 0x20);
            let log = calls();
            assert_eq!(log.len(), 3);
            assert_eq!(log[2], (RS_DRAW_PRIMITIVE, vec![r as u32, g as u32, 0, 0x10, 0x20, 0x8888, 0]));

            // A skin and hardware bones.
            wr::<u32>(r, R::countHWBones, 4);
            returns(GEOM_VERTEX_COUNT, &[9]);
            render_shape(&MainThread::for_test(), r, g, 0x30, 0x10, 0x20);
            assert_eq!(calls()[1], (RS_DRAW_SKINNED_PRIMITIVE, vec![r as u32, g as u32, 0x30, 0x10, 0x20]));

            // With the check of MWSE in memory, the port reads the flag of the geometry data
            // and not the device. Without the flag, the port uses hardware skinning, also with
            // 3 hardware bones.
            SKINNING_CHECK_BYTES.with(|bytes| bytes.set(SKINNING_CHECK_MWSE));
            wr::<u32>(r, R::countHWBones, 3);
            returns(GEOM_VERTEX_COUNT, &[9]);
            render_shape(&MainThread::for_test(), r, g, 0x30, 0x10, 0x20);
            assert_eq!(calls()[1].0, RS_DRAW_SKINNED_PRIMITIVE);
            wr::<u32>(r, R::countHWBones, 4);
            wr::<u16>(g, GEOM_DATA_PATCH_RENDER_FLAGS, 0x200);
            returns(GEOM_VERTEX_COUNT, &[9]);
            render_shape(&MainThread::for_test(), r, g, 0x30, 0x10, 0x20);
            let log = calls();
            assert_eq!(log[1].0, RS_CREATE_GEOMETRY_BUFFER_DATA);
            assert_eq!(log[2].0, RS_DRAW_PRIMITIVE);
            SKINNING_CHECK_BYTES.with(|bytes| bytes.set(SKINNING_CHECK_DATABASE));
        }
    }

    #[test]
    fn render_tri_strips_counts_the_triangles_of_the_strips() {
        let mut w = world();
        let r = w.r();
        let mut geom = geometry();
        let g = geom.data.as_mut_ptr() as usize;
        let mut b = buffer();
        let gb = b.as_mut_ptr() as usize;
        unsafe {
            wr::<u16>(g, NiTriStripsData::numStrips, 3);
            wr::<u32>(g, NiTriStripsData::stripLengths, 0x5550);
            wr::<u32>(g, NiTriStripsData::stripLists, 0x6660);
            returns(GEOM_VERTEX_COUNT, &[9]);
            returns(RT_CREATE_GEOMETRY_BUFFER_DATA, &[1]);
            poke(RT_CREATE_GEOMETRY_BUFFER_DATA, 1, gb as u32);
            returns(RT_GET_STRIP_LENGTH_SUM, &[0x0001_0014]);
            render_tri_strips(&MainThread::for_test(), r, g, 0, 0x10, 0x20);
            let log = calls();
            assert_eq!(log[2], (RT_GET_STRIP_LENGTH_SUM, vec![g as u32]));
            assert_eq!(log[3], (RT_DRAW_PRIMITIVE, vec![r as u32, g as u32, 0, 0x10, 0x20, gb as u32, 0]));
            assert_eq!(rd::<u32>(gb, GB::primitiveType), 5);
            assert_eq!(rd::<u32>(gb, GB::stripCount), 3);
            // 20 indices in 3 strips make 14 triangles.
            assert_eq!(rd::<u32>(gb, GB::trianglesCount), 14);
            assert_eq!(rd::<u32>(gb, GB::stripVertexCounts), 0x5550);
            assert_eq!(rd::<u32>(gb, GB::triList), 0x6660);

            wr::<u32>(r, R::countHWBones, 8);
            returns(GEOM_VERTEX_COUNT, &[9]);
            render_tri_strips(&MainThread::for_test(), r, g, 0x30, 0x10, 0x20);
            assert_eq!(calls()[1], (RT_DRAW_SKINNED_PRIMITIVE, vec![r as u32, g as u32, 0x30, 0x10, 0x20]));
        }
    }

    // -----------------------------------------------------------------------------------
    // DrawPrimitive
    // -----------------------------------------------------------------------------------

    #[test]
    fn draw_primitive_without_buffer_and_geometry_does_nothing() {
        let mut w = world();
        let r = w.r();
        let mut b = buffer();
        let gb = b.as_mut_ptr() as usize;
        unsafe {
            draw_primitive(&MainThread::for_test(), r, 0, 0, 0x10, 0x20, gb, 0);
            assert_eq!(calls(), vec![]);
            let mut set = Regions::new();
            draw_primitive_regions(&mut set, r, 0, gb);
            assert_eq!(set.as_slice(), &[(r, R::SIZE)]);
        }
    }

    #[test]
    fn draw_primitive_draws_each_pass() {
        let mut w = world();
        let r = w.r();
        let (rs, pl, ps, dev) = (w.rs(), w.pipeline(), w.ps(), w.dev());
        let mut geom = geometry();
        let g = geom.data.as_mut_ptr() as usize;
        let mut b = buffer();
        let gb = b.as_mut_ptr() as usize;
        let lengths = [4u16, 6];
        let gu = g as u32;
        let (t, wb) = (0x10u32, 0x20u32);
        unsafe {
            wr::<u16>(g, GD::textureSets, 2);
            wr::<u32>(g, GD::normals, 0x100);
            wr::<u32>(gb, GB::fvf, 0x112);
            wr::<u32>(gb, GB::d3dVertexBuffer, 0x800);
            wr::<u32>(gb, GB::vertexStride, 32);
            wr::<u32>(gb, GB::indexBuffer, 0x900);
            wr::<u32>(gb, GB::verticesCount, 24);
            wr::<u32>(gb, GB::primitiveType, 5);
            wr::<u32>(gb, GB::stripCount, 2);
            wr(gb, GB::stripVertexCounts, lengths.as_ptr() as usize);
            wr::<u32>(rs as usize, RS_VERTEX_BLEND_CURRENT, 7);
            // Two passes: the first pass returns "more passes follow".
            returns(DP_LOAD_BINARY, &[1, 0]);

            draw_primitive(&MainThread::for_test(), r, g, 0, t as usize, wb as usize, gb, 0);
            let state = [ps, EFFECT_STATE as u32];
            let pass = |textures: u32| vec![pl, state[0], state[1], 1, textures, gu, 0, t, wb];
            let one_pass = |first: bool| {
                let mut log = vec![(DP_LOAD_BINARY, vec![pl, gu, 0, t, wb])];
                if first {
                    log.push((DP_PACK_GEOMETRY_BUFFERS, vec![r as u32, 0, gu, 0, 0, 0]));
                }
                log.extend([
                    (DP_APPLY_PIXEL_SHADER, vec![pl]),
                    (DEV_SET_STREAM_SOURCE, vec![dev, 0, 0x800, 32]),
                    (DEV_SET_INDICES, vec![dev, 0x900, 0]),
                    (DP_APPLY_VERTEX_SHADER, vec![pl, 0x112]),
                    (DP_BUILD_SKIN_PARTITION_PASS, vec![pl, state[0], state[1], 1, 1, gu, 0, 0, t, wb]),
                    (DP_DRAW_INDEXED_PRIMITIVE, vec![dev, 5, 0, 24, 0, 2]),
                    (DP_DRAW_INDEXED_PRIMITIVE, vec![dev, 5, 0, 24, 4, 4]),
                ]);
                log
            };
            let mut expected = vec![
                (DP_GET_RENDERER_SPECIFIC, vec![ps]),
                (DP_SELECT_AND_LINK_PASS, pass(1)),
                (DP_UPDATE_D3D_STATE, vec![rs, ps]),
                (DP_BUILD_PASS, pass(2)),
                (DEV_SET_RENDER_STATE, vec![dev, 151, 0]),
                (DP_SET_MODEL_TRANSFORM, vec![r as u32, t]),
                (DP_BOUND_COPY, vec![(r + R::worldBound) as u32, wb]),
                (DP_LIGHT_SET_STATE, vec![LIGHT_MANAGER as u32, EFFECT_STATE as u32, TEXTURING, VERTEX_COLOR]),
            ];
            expected.extend(one_pass(true));
            expected.extend(one_pass(false));
            expected.push((DP_ENSURE_AND_REGISTER_STREAMABLES, pass(1)));
            let mut log = calls();
            // The buffer pointer is a local of the function, and its address is not fixed.
            assert_eq!(log[9].0, DP_PACK_GEOMETRY_BUFFERS);
            log[9].1[1] = 0;
            assert_eq!(log, expected);
        }
    }

    #[test]
    fn draw_primitive_keeps_what_the_flags_say() {
        let mut w = world();
        let r = w.r();
        let dev = w.dev();
        let mut b = buffer();
        let gb = b.as_mut_ptr() as usize;
        let mut other = buffer();
        let gb2 = other.as_mut_ptr() as usize;
        let mut partition = vec![0u16; P::SIZE / 2];
        let p = partition.as_mut_ptr() as usize;
        unsafe {
            wr::<u32>(gb, GB::flags, 1);
            wr::<u32>(gb, GB::d3dVertexBuffer, 0x800);
            wr::<u32>(gb, GB::fvf, 0x200);
            wr::<u32>(gb2, GB::primitiveType, 4);
            wr::<u32>(gb2, GB::trianglesCount, 12);
            wr::<u16>(p, P::numBones, 4);

            // There is a partition, and the pass keeps the transform and the lights. The port
            // does not draw the first pass and draws the second pass. The second pass is the
            // first pass in the count for the buffers.
            returns(DP_BUILD_PASS, &[BUILD_KEEP_TRANSFORM | BUILD_KEEP_LIGHTS]);
            let keep_all = PASS_KEEP_PIXEL_SHADER | PASS_KEEP_STREAMS | PASS_KEEP_VERTEX_SHADER | PASS_KEEP_PARTITION_PASS;
            returns(DP_LOAD_BINARY, &[PASS_NO_DRAW | 1, keep_all]);
            poke(DP_CREATE_PARTITION_GEOM_BUFFER, 1, gb2 as u32);
            draw_primitive(&MainThread::for_test(), r, 0, 0x30, 0x10, 0x20, gb, p);
            let log = calls();
            let ids: Vec<usize> = log.iter().map(|call| call.0).collect();
            assert_eq!(
                ids,
                vec![
                    DP_GET_RENDERER_SPECIFIC,
                    DP_SELECT_AND_LINK_PASS,
                    DP_UPDATE_D3D_STATE,
                    DP_BUILD_PASS,
                    DEV_SET_RENDER_STATE,
                    DP_LOAD_BINARY,
                    DP_LOAD_BINARY,
                    DP_CREATE_PARTITION_GEOM_BUFFER,
                    DEV_DRAW_PRIMITIVE,
                    DP_ENSURE_AND_REGISTER_STREAMABLES,
                ]
            );
            // The vertex format has no normals and 2 texture sets.
            assert_eq!(&log[3].1[3..5], &[0, 2]);
            assert_eq!(log[4].1, vec![dev, 151, 3]);
            assert_eq!(&log[7].1[2..], &[0, 0x30, p as u32]);
            // The port draws the buffer that the callee gave.
            assert_eq!(log[8].1, vec![dev, 4, 0, 12]);

            // The pass does not draw.
            returns(DP_BUILD_PASS, &[BUILD_NO_DRAW | BUILD_KEEP_VERTEX_BLEND]);
            draw_primitive(&MainThread::for_test(), r, 0, 0x30, 0x10, 0x20, gb, p);
            let ids: Vec<usize> = calls().iter().map(|call| call.0).collect();
            assert_eq!(
                ids,
                vec![
                    DP_GET_RENDERER_SPECIFIC,
                    DP_SELECT_AND_LINK_PASS,
                    DP_UPDATE_D3D_STATE,
                    DP_BUILD_PASS,
                    DP_SET_SKINNED_MODEL_TRANSFORMS,
                    DP_BOUND_COPY,
                    DP_LIGHT_SET_STATE,
                    DP_ENSURE_AND_REGISTER_STREAMABLES,
                ]
            );

        }
    }

    #[test]
    fn the_function_of_the_property_takes_the_place_of_the_partition_pass() {
        let mut w = world();
        let r = w.r();
        let ps = w.ps();
        let mut b = buffer();
        let gb = b.as_mut_ptr() as usize;
        // prop1 with the function at +0x2C, the render data and the property.
        let mut prop1 = [0usize; 12];
        prop1[PROP_1_PASS_CALLBACK / 4] = s11::<PROP_CALLBACK> as *const () as usize;
        let data = Object::new(&[(1, t0::<RENDER_DATA_TYPE> as *const () as usize)]);
        let mut property = vec![0usize; NiRendererSpecificProperty::SIZE / 4];
        unsafe {
            wr(data.address(), RENDER_DATA_FIELD_C, prop1.as_ptr() as usize);
            wr::<usize>(data.address(), RENDER_DATA_FIELD_8, 0x777);
            property[NiRendererSpecificProperty::renderData / 4] = data.address();
            wr::<u32>(gb, GB::flags, 1);
            wr::<u32>(gb, GB::d3dVertexBuffer, 0x800);
            returns(DP_GET_RENDERER_SPECIFIC, &[property.as_ptr() as u32]);
            returns(RENDER_DATA_TYPE, &[7]);
            draw_primitive(&MainThread::for_test(), r, 0x40, 0, 0x10, 0x20, gb, 0);
            let log = calls();
            assert_eq!(log[1], (RENDER_DATA_TYPE, vec![data.address() as u32]));
            let pack = log.iter().find(|call| call.0 == DP_PACK_GEOMETRY_BUFFERS).unwrap();
            assert_eq!(&pack.1[4..], &[prop1.as_ptr() as u32, 0x777]);
            let callback = log.iter().find(|call| call.0 == PROP_CALLBACK).unwrap();
            assert_eq!(callback.1, vec![ps, EFFECT_STATE as u32, 0, 1, 0x40, 0, 0, 0x10, 0x20, 0x777, PASS]);
            assert!(log.iter().all(|call| call.0 != DP_BUILD_SKIN_PARTITION_PASS));

            // The port does not use render data of another renderer.
            returns(DP_GET_RENDERER_SPECIFIC, &[property.as_ptr() as u32]);
            returns(RENDER_DATA_TYPE, &[6]);
            draw_primitive(&MainThread::for_test(), r, 0x40, 0, 0x10, 0x20, gb, 0);
            let log = calls();
            assert!(log.iter().any(|call| call.0 == DP_BUILD_SKIN_PARTITION_PASS));
            assert!(log.iter().all(|call| call.0 != PROP_CALLBACK));
        }
    }

    // -----------------------------------------------------------------------------------
    // DrawSkinnedPrimitive, DrawSkinnedPrimitive2
    // -----------------------------------------------------------------------------------

    /// A skin instance, its skin data, a skin partition and `count` partitions.
    struct Skin {
        instance: Vec<u32>,
        _data: Vec<u32>,
        skin_partition: Vec<u32>,
        partitions: Vec<u16>,
    }

    fn skin(count: usize) -> Skin {
        let mut partitions = vec![0u16; P::SIZE / 2 * count.max(1)];
        let mut skin_partition = vec![0u32; NiSkinPartition::SIZE / 4];
        skin_partition[NiSkinPartition::partitionCount / 4] = count as u32;
        skin_partition[NiSkinPartition::partitions / 4] = partitions.as_mut_ptr() as u32;
        // The skin data up to its bone count at 0x40 (0 bones: complete for the guard of
        // `ensure_skin_partitions`).
        let mut data = vec![0u32; 0x48 / 4];
        data[SKIN_DATA_SKIN_PARTITION / 4] = skin_partition.as_mut_ptr() as u32;
        let mut instance = vec![0u32; NiSkinInstance::SIZE / 4];
        instance[NiSkinInstance::skinData / 4] = data.as_mut_ptr() as u32;
        Skin { instance, _data: data, skin_partition, partitions }
    }

    impl Skin {
        fn address(&mut self) -> usize {
            self.instance.as_mut_ptr() as usize
        }
        fn partition(&mut self, index: usize) -> usize {
            self.partitions.as_mut_ptr() as usize + index * P::SIZE
        }
    }

    #[test]
    fn draw_skinned_primitive_fills_the_buffers_that_are_new() {
        let mut w = world();
        let r = w.r();
        let mut s = skin(2);
        let (si, p0, p1) = (s.address(), s.partition(0), s.partition(1));
        let mut b = buffer();
        let gb = b.as_mut_ptr() as usize;
        unsafe {
            // Strips: the first partition sets the primitive type for all partitions.
            wr::<u16>(p0, P::numStripLengths, 3);
            wr::<u32>(p0, P::stripLengths, 0xA00);
            wr::<u32>(p0, P::triangles, 0xB00);
            wr::<u16>(p0, P::numTriangles, 17);
            wr::<u16>(p1, P::numStripLengths, 2);
            // The buffer of the first partition is new. The buffer of the second is not new.
            returns(SP_FIND_CACHED_PARTITION_GEOM_BUFFER, &[0x5501, 0x5500]);
            poke(SP_FIND_CACHED_PARTITION_GEOM_BUFFER, 1, gb as u32);
            draw_skinned_primitive(r, 0x40, si, 0x10, 0x20);
            let log = calls();
            let ids: Vec<usize> = log.iter().map(|call| call.0).collect();
            assert_eq!(
                ids,
                vec![
                    SP_FIND_CACHED_PARTITION_GEOM_BUFFER,
                    SP_CREATE_PARTITION_GEOM_BUFFER,
                    SP_FIND_CACHED_PARTITION_GEOM_BUFFER,
                    SP_DRAW_SKINNED_PRIMITIVE_2
                ]
            );
            assert_eq!(&log[0].1[2..], &[0x40, si as u32, p0 as u32]);
            assert_eq!(&log[1].1[2..], &[0x40, si as u32, p0 as u32]);
            assert_eq!(&log[2].1[2..], &[0x40, si as u32, p1 as u32]);
            assert_eq!(log[3].1, vec![r as u32, 5, 0x40, si as u32, 0x10, 0x20, 0, 0]);
            assert_eq!(rd::<u32>(gb, GB::primitiveType), 5);
            assert_eq!(rd::<u32>(gb, GB::trianglesCount), 17);
            assert_eq!(rd::<u32>(gb, GB::stripCount), 3);
            assert_eq!(rd::<u32>(gb, GB::stripVertexCounts), 0xA00);
            assert_eq!(rd::<u32>(gb, GB::triList), 0xB00);

            // Triangle lists: the buffer gets one "strip" and no lengths.
            wr::<u16>(p0, P::numStripLengths, 0);
            returns(SP_FIND_CACHED_PARTITION_GEOM_BUFFER, &[1, 0]);
            draw_skinned_primitive(r, 0x40, si, 0x10, 0x20);
            assert_eq!(calls()[3].1[1], 4);
            assert_eq!(rd::<u32>(gb, GB::primitiveType), 4);
            assert_eq!(rd::<u32>(gb, GB::stripCount), 1);
            assert_eq!(rd::<u32>(gb, GB::stripVertexCounts), 0);
        }
    }

    #[test]
    fn draw_skinned_primitive_makes_the_partitions_when_there_are_none() {
        let mut w = world();
        let r = w.r();
        let mut s = skin(0);
        let si = s.address();
        let made = s.skin_partition.as_mut_ptr() as usize;
        let data: usize = unsafe { rd(si, NiSkinInstance::skinData) };
        unsafe {
            wr::<u32>(data, SKIN_DATA_SKIN_PARTITION, 0);
            wr::<u32>(r, R::countHWBones, 4);
            // The allocator returns a block and the constructor returns the object.
            let block = 0x00AB_CD00u32;
            returns(SP_OPERATOR_NEW, &[block]);
            returns(SP_SKIN_PARTITION_CTOR, &[made as u32]);
            returns(SP_MAKE_PARTITIONS, &[0x100]);
            draw_skinned_primitive(r, 0x40, si, 0x10, 0x20);
            let log = calls();
            assert_eq!(log[0], (SP_OPERATOR_NEW, vec![0x10]));
            assert_eq!(log[1], (SP_SKIN_PARTITION_CTOR, vec![block]));
            let bones = (made as u32 & 0xFFFF_FF00) | 4;
            assert_eq!(log[2], (SP_MAKE_PARTITIONS, vec![made as u32, 0x40, data as u32, bones, bones]));
            assert_eq!(log[3].0, SP_DRAW_SKINNED_PRIMITIVE_2);
            // MakePartitions returned false (AL is 0): the skin data does not get the object.
            assert_eq!(rd::<u32>(data, SKIN_DATA_SKIN_PARTITION), 0);

            // MakePartitions returns true: the skin data gets the object, and the object gets
            // a reference.
            returns(SP_OPERATOR_NEW, &[block]);
            returns(SP_SKIN_PARTITION_CTOR, &[made as u32]);
            returns(SP_MAKE_PARTITIONS, &[1]);
            draw_skinned_primitive(r, 0x40, si, 0x10, 0x20);
            assert_eq!(calls().len(), 4);
            assert_eq!(rd::<usize>(data, SKIN_DATA_SKIN_PARTITION), made);
            assert_eq!(rd::<u32>(made, NiSkinPartition::refCount), 1);
        }
    }

    #[test]
    fn draw_skinned_primitive_2_draws_the_partitions() {
        let mut w = world();
        let r = w.r();
        let (rs, pl, ps, dev) = (w.rs(), w.pipeline(), w.ps(), w.dev());
        let mut s = skin(2);
        let (si, p0, p1) = (s.address(), s.partition(0), s.partition(1));
        let mut geom = geometry();
        let g = geom.data.as_mut_ptr() as usize;
        let (mut b0, mut b1) = (buffer(), buffer());
        let (gb0, gb1) = (b0.as_mut_ptr() as usize, b1.as_mut_ptr() as usize);
        unsafe {
            // No skin: the port makes no call.
            draw_skinned_primitive_2(&MainThread::for_test(), r, 5, g, 0, 0x10, 0x20, 0, 0);
            assert_eq!(calls(), vec![]);

            wr::<u16>(g, GD::textureSets, 11);
            wr(p0, P::bufferData, gb0);
            wr(p1, P::bufferData, gb1);
            wr::<u16>(p0, P::numBonesPerVertex, 2);
            wr::<u16>(p1, P::numBonesPerVertex, 2);
            wr::<u16>(p0, P::numVertices, 100);
            wr::<u16>(p1, P::numVertices, 60);
            for (gb, vb, ib, triangles) in [(gb0, 0x800u32, 0x900u32, 30u32), (gb1, 0x810, 0, 20)] {
                wr(gb, GB::d3dVertexBuffer, vb);
                wr::<u32>(gb, GB::vertexStride, 40);
                wr(gb, GB::indexBuffer, ib);
                wr(gb, GB::trianglesCount, triangles);
                wr::<u32>(gb, GB::fvf, 0x1116);
            }
            draw_skinned_primitive_2(&MainThread::for_test(), r, 5, g, si, 0x10, 0x20, 0, 0);
            let state = [ps, EFFECT_STATE as u32];
            let (gu, su) = (g as u32, si as u32);
            // The buffer has no "filled" flag. The geometry data gives 11 texture sets, and
            // there is no limit.
            let pass = |textures: u32| vec![pl, state[0], state[1], 0, textures, gu, su, 0x10, 0x20];
            let expected = vec![
                (S2_SELECT_AND_LINK_PASS, pass(1)),
                (S2_UPDATE_D3D_STATE, vec![rs, ps]),
                (S2_BUILD_PASS, pass(11)),
                (S2_LIGHT_SET_STATE, vec![LIGHT_MANAGER as u32, EFFECT_STATE as u32, TEXTURING, VERTEX_COLOR]),
                (S2_LOAD_BINARY, vec![pl, gu, su, 0x10, 0x20]),
                (S2_APPLY_PIXEL_SHADER, vec![pl]),
                (S2_BOUND_COPY, vec![(r + R::worldBound) as u32, 0x20]),
                // The first partition, with an index buffer.
                (DEV_SET_RENDER_STATE, vec![dev, 151, 1]),
                (S2_SET_SKINNED_MODEL_TRANSFORMS, vec![r as u32, su, p0 as u32, 0x10, 0x20]),
                (S2_APPLY_VERTEX_SHADER, vec![pl, 0x1116]),
                (S2_BUILD_SKIN_PARTITION_PASS, vec![pl, state[0], state[1], 0, 1, gu, su, p0 as u32, 0x10, 0x20]),
                (DEV_SET_STREAM_SOURCE, vec![dev, 0, 0x800, 40]),
                (DEV_SET_INDICES, vec![dev, 0x900, 0]),
                (S2_DRAW_INDEXED_PRIMITIVE, vec![dev, 5, 0, 100, 0, 30]),
                // The second partition, without an index buffer. The port sets the indices also
                // for this partition.
                (S2_SET_SKINNED_MODEL_TRANSFORMS, vec![r as u32, su, p1 as u32, 0x10, 0x20]),
                (S2_APPLY_VERTEX_SHADER, vec![pl, 0x1116]),
                (S2_BUILD_SKIN_PARTITION_PASS, vec![pl, state[0], state[1], 0, 1, gu, su, p1 as u32, 0x10, 0x20]),
                (DEV_SET_STREAM_SOURCE, vec![dev, 0, 0x810, 40]),
                (DEV_SET_INDICES, vec![dev, 0, 0]),
                (S2_DRAW_PRIMITIVE, vec![dev, 5, 0, 20]),
                (S2_ENSURE_AND_REGISTER_STREAMABLES, pass(1)),
            ];
            assert_eq!(calls(), expected);

            // A partition without a buffer makes the function return immediately.
            wr::<usize>(p1, P::bufferData, 0);
            returns(S2_BUILD_PASS, &[BUILD_KEEP_VERTEX_BLEND | BUILD_KEEP_TRANSFORM | BUILD_KEEP_LIGHTS]);
            returns(S2_LOAD_BINARY, &[keep_everything()]);
            draw_skinned_primitive_2(&MainThread::for_test(), r, 4, g, si, 0x10, 0x20, 0, 0);
            let ids: Vec<usize> = calls().iter().map(|call| call.0).collect();
            assert_eq!(
                ids,
                vec![S2_SELECT_AND_LINK_PASS, S2_UPDATE_D3D_STATE, S2_BUILD_PASS, S2_LOAD_BINARY, S2_BOUND_COPY, S2_DRAW_INDEXED_PRIMITIVE]
            );

            let mut set = Regions::new();
            draw_skinned_primitive_2_regions(&mut set, r, 0);
            assert_eq!(set.as_slice(), &[(r, R::SIZE)]);
        }
    }

    fn keep_everything() -> u32 {
        PASS_KEEP_PIXEL_SHADER | PASS_KEEP_STREAMS | PASS_KEEP_VERTEX_SHADER | PASS_KEEP_PARTITION_PASS | PASS_KEEP_BUFFERS
    }

    // -----------------------------------------------------------------------------------
    // EndBatch
    // -----------------------------------------------------------------------------------

    #[test]
    fn end_batch_with_an_empty_list_does_nothing() {
        let mut w = world();
        let r = w.r();
        unsafe {
            end_batch(&MainThread::for_test(), r);
            assert_eq!(calls(), vec![]);
            let mut set = Regions::new();
            end_batch_regions(&mut set, r);
            assert_eq!(set.as_slice(), &[(r, R::SIZE)]);
        }
    }

    #[test]
    fn end_batch_draws_the_objects_and_frees_the_list() {
        let mut w = world();
        let r = w.r();
        let (rs, pl, dev) = (w.rs(), w.pipeline(), w.dev());
        let (mut b0, mut b1) = (buffer(), buffer());
        let (gb0, gb1) = (b0.as_mut_ptr() as usize, b1.as_mut_ptr() as usize);
        let mut partition = vec![0u16; P::SIZE / 2];
        let p = partition.as_mut_ptr() as usize;
        let mut geom = geometry();
        let g = geom.data.as_mut_ptr() as usize;
        // geometry data, skin, transform, bound, partition, buffer, next
        let mut second = [0x41u32, 0x31, 0x11, 0x21, p as u32, gb1 as u32, 0];
        let mut first = [g as u32, 0, 0x10, 0x20, 0, gb0 as u32, second.as_mut_ptr() as u32];
        // The states of the batch. The property state has one reference, and the list
        // releases it. The effect state comes from the renderer.
        let mut batch_properties = vec![0u32; PS::SIZE / 4];
        let property_deleter = Object::new(&[(0, t1::<OBJECT_DELETE> as *const () as usize)]);
        batch_properties[0] = unsafe { rd(property_deleter.address(), 0) };
        batch_properties[1] = 1;
        batch_properties[PS::propTexturing / 4] = 0x1410;
        batch_properties[PS::propVertexColor / 4] = 0x1510;
        let bp = batch_properties.as_mut_ptr() as u32;
        let mut effects = [0u32, 5];
        let es = effects.as_mut_ptr() as u32;
        unsafe {
            wr(r, R::batchedObjectListHead, first.as_mut_ptr() as usize);
            wr(r, R::batchedObjectListTail, second.as_mut_ptr() as usize);
            wr(r, R::propertyStatePtr, bp);
            wr(r, R::currentEffectState, es);
            wr::<u16>(g, GD::textureSets, 1);
            wr::<u16>(p, P::numBones, 2);
            for (gb, vb, triangles) in [(gb0, 0x800u32, 30u32), (gb1, 0x810, 20)] {
                wr(gb, GB::d3dVertexBuffer, vb);
                wr::<u32>(gb, GB::vertexStride, 40);
                wr(gb, GB::trianglesCount, triangles);
                wr::<u32>(gb, GB::primitiveType, 4);
                wr::<u32>(gb, GB::fvf, 0x112);
            }
            // Two passes. The port fills the buffers in the first pass.
            returns(EB_LOAD_BINARY, &[1, 0]);

            end_batch(&MainThread::for_test(), r);
            let pass = |textures: u32, tail: [u32; 4]| vec![pl, bp, es, 0, textures, tail[0], tail[1], tail[2], tail[3]];
            let head = [g as u32, 0, 0x10, 0x20];
            let one_pass = |first_pass: bool| {
                let mut log = vec![(EB_LOAD_BINARY, vec![pl, g as u32, 0, 0x10, 0x20]), (EB_APPLY_PIXEL_SHADER, vec![pl])];
                if first_pass {
                    log.push((EB_PACK_GEOMETRY_BUFFERS, vec![r as u32, 0, g as u32, 0, 0, 0]));
                }
                log.push((EB_APPLY_VERTEX_SHADER, vec![pl, 0x112]));
                if !first_pass {
                    // The second object left the vertex blend of 2 bones.
                    log.push((DEV_SET_RENDER_STATE, vec![dev, 151, 0]));
                }
                log.extend([
                    (EB_SET_MODEL_TRANSFORM, vec![r as u32, 0x10]),
                    (EB_BOUND_COPY, vec![(r + R::worldBound) as u32, 0x20]),
                    (DEV_SET_STREAM_SOURCE, vec![dev, 0, 0x800, 40]),
                    (EB_BUILD_SKIN_PARTITION_PASS, vec![pl, bp, es, 0, 1, g as u32, 0, 0, 0x10, 0x20]),
                    (DEV_DRAW_PRIMITIVE, vec![dev, 4, 0, 30]),
                ]);
                if first_pass {
                    log.push((EB_CREATE_PARTITION_GEOM_BUFFER, vec![r as u32, 0, 0x41, 0x31, p as u32]));
                }
                log.extend([
                    (EB_APPLY_VERTEX_SHADER, vec![pl, 0x112]),
                    (DEV_SET_RENDER_STATE, vec![dev, 151, 1]),
                    (EB_SET_SKINNED_MODEL_TRANSFORMS, vec![r as u32, 0x31, p as u32, 0x11, 0x21]),
                    (EB_BOUND_COPY, vec![(r + R::worldBound) as u32, 0x21]),
                    (DEV_SET_STREAM_SOURCE, vec![dev, 0, 0x810, 40]),
                    (EB_BUILD_SKIN_PARTITION_PASS, vec![pl, bp, es, 0, 1, 0x41, 0x31, p as u32, 0x11, 0x21]),
                    (DEV_DRAW_PRIMITIVE, vec![dev, 4, 0, 20]),
                ]);
                log
            };
            let mut expected = vec![
                (EB_GET_RENDERER_SPECIFIC, vec![bp]),
                (EB_SELECT_AND_LINK_PASS, pass(1, head)),
                (EB_UPDATE_D3D_STATE, vec![rs, bp]),
                (EB_BUILD_PASS, pass(1, head)),
                (EB_LIGHT_SET_STATE, vec![LIGHT_MANAGER as u32, es, 0x1410, 0x1510]),
            ];
            expected.extend(one_pass(true));
            expected.extend(one_pass(false));
            expected.extend([
                // The object the loop read last.
                (EB_ENSURE_AND_REGISTER_STREAMABLES, pass(1, [0x41, 0x31, 0x11, 0x21])),
                (OBJECT_DELETE, vec![bp, 1]),
                (EB_DELETE, vec![first.as_mut_ptr() as u32]),
                (EB_DELETE, vec![second.as_mut_ptr() as u32]),
            ]);
            let mut log = calls();
            // The buffer pointer is a local of the function, and its address is not fixed.
            for call in log.iter_mut() {
                if call.0 == EB_PACK_GEOMETRY_BUFFERS || call.0 == EB_CREATE_PARTITION_GEOM_BUFFER {
                    call.1[1] = 0;
                }
            }
            assert_eq!(log, expected);

            assert_eq!(rd::<u32>(r, R::batchedObjectListHead), 0);
            assert_eq!(rd::<u32>(r, R::batchedObjectListTail), 0);
            assert_eq!(rd::<u32>(r, R::propertyStatePtr), 0);
            assert_eq!(rd::<u32>(r, R::effectStatePtr), 0);
            // The effect state got a reference at the start, and the port released it at the
            // end.
            assert_eq!(effects[1], 5);
            assert_eq!(batch_properties[1], 0);
        }
    }

    #[test]
    fn end_batch_takes_the_current_states_when_the_batch_has_none() {
        let mut w = world();
        let r = w.r();
        let ps = w.ps();
        let mut b = buffer();
        let gb = b.as_mut_ptr() as usize;
        let mut only = [0u32, 0, 0x10, 0x20, 0, gb as u32, 0];
        unsafe {
            wr::<u32>(gb, GB::flags, 1);
            wr::<u32>(gb, GB::d3dVertexBuffer, 0x800);
            wr::<u32>(gb, GB::fvf, 0x310);
            wr(r, R::batchedObjectListHead, only.as_mut_ptr() as usize);
            wr::<u32>(r, R::currentEffectState, 0);
            // The property state of the renderer has 3 references.
            wr::<u32>(ps as usize, NiRefObject::refCount, 3);
            returns(EB_BUILD_PASS, &[BUILD_NO_DRAW | BUILD_KEEP_LIGHTS]);
            end_batch(&MainThread::for_test(), r);
            let log = calls();
            let ids: Vec<usize> = log.iter().map(|call| call.0).collect();
            assert_eq!(
                ids,
                vec![
                    EB_GET_RENDERER_SPECIFIC,
                    EB_SELECT_AND_LINK_PASS,
                    EB_UPDATE_D3D_STATE,
                    EB_BUILD_PASS,
                    EB_ENSURE_AND_REGISTER_STREAMABLES,
                    EB_DELETE
                ]
            );
            // The vertex format gives normals and 3 texture sets. There is no effect state.
            assert_eq!(&log[3].1[1..5], &[ps, 0, 1, 3]);
            assert_eq!(rd::<u32>(ps as usize, NiRefObject::refCount), 3);
            assert_eq!(rd::<u32>(r, R::propertyStatePtr), 0);
            assert_eq!(rd::<u32>(r, R::batchedObjectListHead), 0);

            // The first object has no filled buffer and no geometry data: the port keeps the
            // list.
            wr::<u32>(gb, GB::flags, 0);
            wr(r, R::batchedObjectListHead, only.as_mut_ptr() as usize);
            end_batch(&MainThread::for_test(), r);
            assert_eq!(calls(), vec![]);
            assert_eq!(rd::<usize>(r, R::batchedObjectListHead), only.as_mut_ptr() as usize);
        }
    }
}
