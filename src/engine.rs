//! Engine functions and data that the ports use at their fixed addresses in Morrowind.exe.
//!
//! A function here is outside the port or has no port yet. The scene graph and the allocator
//! are outside the port.

use core::mem::transmute;

use crate::mem::rd;

/// `TES3::WorldController*`
pub const WORLD_CONTROLLER: usize = 0x7C67DC;
pub const WORLD_CONTROLLER_RENDERER: usize = crate::gen::layouts::WorldController::renderer;
/// Initial values of the 32 texture stage states.
pub const TEXTURE_STAGE_STATE_INITIAL: usize = 0x7C3C58;

// ---- shared data at fixed addresses

/// `vtbl_sg_NiRefObject`: the vtable of `NiRefObject`.
pub const VTBL_NI_REF_OBJECT: usize = 0x7469B8;
/// The vtable of `NiAlphaProperty`.
pub const VTBL_NI_ALPHA_PROPERTY: usize = 0x7465A8;
/// `vtbl_sg_NiDX8Renderer`: the vtable of `NiDX8Renderer`.
pub const VTBL_NI_DX8_RENDERER: usize = 0x74F4D0;
/// The vtable of the fixed-function `NiDX8TexturePass`.
pub const VTBL_NI_DX8_TEXTURE_PASS: usize = 0x74F650;
/// The vtable of the hash map of `NiDX8Renderer` for the geometry buffers.
pub const VTBL_RENDERER_MAP_GEOM_BUFFERS: usize = 0x74F620;
/// The vtable of the hash map of `NiDX8Renderer` for the skin partitions.
pub const VTBL_RENDERER_MAP_SKIN_PARTITIONS: usize = 0x74F60C;
/// The vtable of the hash map of `NiDX8Renderer` for the textures.
pub const VTBL_RENDERER_MAP_TEXTURES: usize = 0x74F5F8;
/// The vtable of the hash map of `NiDX8Renderer` for the cube maps.
pub const VTBL_RENDERER_MAP_CUBE_MAPS: usize = 0x74F5E4;
/// `global_count_of_NiRefObject`: the number of `NiRefObject` that exist. It is an `int`.
pub const NI_REF_OBJECT_COUNT: usize = 0x7DECE0;
/// `emptyString`: the empty C string.
pub const EMPTY_STRING: usize = 0x7C6180;
/// `dword_7C3834`: the number of texture stages of one device pass.
pub const MAX_TEXTURE_BLEND_STAGES: usize = 0x7C3834;
/// `dword_7C3838`: the number of textures of one device pass.
pub const MAX_SIMULTANEOUS_TEXTURES: usize = 0x7C3838;
/// `dword_7DD958`: the number of vertex streams of the device.
pub const MAX_STREAMS: usize = 0x7DD958;
/// `data_NiDX8Renderer_vertexBlendLookup`: the `D3DRS_VERTEXBLEND` value for a count of bones.
pub const VERTEX_BLEND_LOOKUP: usize = 0x7C3840;
/// `data_NiDX8TexturePass_minifyFilteringRemapper`: the first minify entry of the filter
/// table. The table has 6 entries of 12 bytes: minify, magnify, mip.
pub const FILTER_REMAP_MINIFY: usize = 0x7DDB80;
/// The first magnify entry of the filter table.
pub const FILTER_REMAP_MAGNIFY: usize = 0x7DDB84;
/// The first mip entry of the filter table.
pub const FILTER_REMAP_MIP: usize = 0x7DDB88;
/// `dword_7DE8FC`: the shift of the first bit field of `NiPixelFormat::compareBits`.
pub const PIXEL_FORMAT_SHIFT_A: usize = 0x7DE8FC;
/// `dword_7C55B0`: the mask of the first bit field of `NiPixelFormat::compareBits`.
pub const PIXEL_FORMAT_MASK_A: usize = 0x7C55B0;
/// `dword_7C55B8`: the shift of the second bit field of `NiPixelFormat::compareBits`.
pub const PIXEL_FORMAT_SHIFT_B: usize = 0x7C55B8;
/// `dword_7C55B4`: the mask of the second bit field of `NiPixelFormat::compareBits`.
pub const PIXEL_FORMAT_MASK_B: usize = 0x7C55B4;
/// `data_zeroVec3`: the `vec3` (0, 0, 0). A static constructor of the game fills it.
pub const VEC3_ZERO: usize = 0x7DE6CC;
/// `data_vecXAxis`: the `vec3` (1, 0, 0). A static constructor of the game fills it.
pub const VEC3_X_AXIS: usize = 0x7DE690;
/// `data_vecYAxis`: the `vec3` (0, 1, 0). A static constructor of the game fills it.
pub const VEC3_Y_AXIS: usize = 0x7DE6A4;
/// The `NiCriticalSection` around `CreateVertexBuffer` and around `CreateTexture`.
pub const CREATE_CRITICAL_SECTION: usize = 0x7DEA78;
/// 0x40B080 `NiPointer::ctor`. The game code gives this address to the vector iterators.
pub const NI_POINTER_CTOR: usize = 0x40B080;
/// 0x40A880 `NiPointer::dtor`. The game code gives this address to the vector iterators.
pub const NI_POINTER_DTOR: usize = 0x40A880;

/// 0x70CFE0 `NiFloatKey_static::genInterp`: `float __cdecl (time, keys, key type, key count,
/// *index)`. It searches from the key at `*index` and moves it.
const FLOAT_KEY_INTERP: usize = 0x70CFE0;

/// The value of a track of float keys at `time`, as the engine's controllers take it. `index`
/// is the key to search from; the call moves it.
pub unsafe fn float_key_interp(time: f32, keys: usize, kind: i32, count: i32, index: &mut u32) -> f32 {
    transmute::<usize, unsafe extern "C" fn(f32, usize, i32, i32, *mut u32) -> f32>(FLOAT_KEY_INTERP)(time, keys, kind, count, index)
}

// ---- call primitives

/// Calls the `__thiscall` function at `addr` with `this` in ECX and no stack argument.
#[inline(always)]
pub unsafe fn thiscall0(addr: usize, this: usize) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize) -> u32>(addr)(this)
}

/// Calls the `__thiscall` function at `addr` with `this` in ECX and 1 stack argument.
#[inline(always)]
pub unsafe fn thiscall1(addr: usize, this: usize, a: u32) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32) -> u32>(addr)(this, a)
}

/// Calls the `__thiscall` function at `addr` with `this` in ECX and 2 stack arguments.
#[inline(always)]
pub unsafe fn thiscall2(addr: usize, this: usize, a: u32, b: u32) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32, u32) -> u32>(addr)(this, a, b)
}

/// Calls the `__thiscall` function at `addr` with `this` in ECX and 3 stack arguments.
#[inline(always)]
pub unsafe fn thiscall3(addr: usize, this: usize, a: u32, b: u32, c: u32) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32, u32, u32) -> u32>(addr)(this, a, b, c)
}

/// Calls the `__thiscall` function at `addr` with `this` in ECX and 4 stack arguments.
#[inline(always)]
pub unsafe fn thiscall4(addr: usize, this: usize, a: u32, b: u32, c: u32, d: u32) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32, u32, u32, u32) -> u32>(addr)(this, a, b, c, d)
}

/// Calls the `__thiscall` function at `addr` with `this` in ECX and 5 stack arguments.
#[inline(always)]
pub unsafe fn thiscall5(addr: usize, this: usize, a: u32, b: u32, c: u32, d: u32, e: u32) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32, u32, u32, u32, u32) -> u32>(addr)(this, a, b, c, d, e)
}

/// Calls the `__cdecl` function at `addr` with no argument. The caller removes the arguments.
#[inline(always)]
pub unsafe fn cdecl0(addr: usize) -> u32 {
    transmute::<usize, unsafe extern "C" fn() -> u32>(addr)()
}

/// Calls the `__cdecl` function at `addr` with 1 argument. The caller removes the arguments.
#[inline(always)]
pub unsafe fn cdecl1(addr: usize, a: u32) -> u32 {
    transmute::<usize, unsafe extern "C" fn(u32) -> u32>(addr)(a)
}

/// Calls the `__cdecl` function at `addr` with 2 arguments. The caller removes the arguments.
#[inline(always)]
pub unsafe fn cdecl2(addr: usize, a: u32, b: u32) -> u32 {
    transmute::<usize, unsafe extern "C" fn(u32, u32) -> u32>(addr)(a, b)
}

/// Calls the `__cdecl` function at `addr` with 3 arguments. The caller removes the arguments.
#[inline(always)]
pub unsafe fn cdecl3(addr: usize, a: u32, b: u32, c: u32) -> u32 {
    transmute::<usize, unsafe extern "C" fn(u32, u32, u32) -> u32>(addr)(a, b, c)
}

/// The virtual function of `object` at `slot_offset` bytes in its vtable. The function reads
/// the vtable pointer at +0 on each call.
#[inline(always)]
pub unsafe fn virtual_at(object: usize, slot_offset: usize) -> usize {
    rd(rd::<usize>(object, 0), slot_offset)
}

/// Calls the `__thiscall` virtual function of `object` at `slot_offset` with no stack argument.
#[inline(always)]
pub unsafe fn vcall0(object: usize, slot_offset: usize) -> u32 {
    thiscall0(virtual_at(object, slot_offset), object)
}

/// Calls the `__thiscall` virtual function of `object` at `slot_offset` with 1 stack argument.
#[inline(always)]
pub unsafe fn vcall1(object: usize, slot_offset: usize, a: u32) -> u32 {
    thiscall1(virtual_at(object, slot_offset), object, a)
}

/// Calls the `__thiscall` virtual function of `object` at `slot_offset` with 2 stack arguments.
#[inline(always)]
pub unsafe fn vcall2(object: usize, slot_offset: usize, a: u32, b: u32) -> u32 {
    thiscall2(virtual_at(object, slot_offset), object, a, b)
}

/// Calls the `__thiscall` virtual function of `object` at `slot_offset` with 3 stack arguments.
#[inline(always)]
pub unsafe fn vcall3(object: usize, slot_offset: usize, a: u32, b: u32, c: u32) -> u32 {
    thiscall3(virtual_at(object, slot_offset), object, a, b, c)
}

/// The first node of the list of renderers of `NiRenderer`, and the renderer in a node.
const RENDERER_LIST_HEAD: usize = 0x7DEA58;
const RENDERER_NODE_RENDERER: usize = 0;

/// The fog property of the cell: `TES3::Game::fogProperty` (the game object at 0x7C6CDC,
/// the property at 0xF0 of it, as MWSE has them). The weather writes its colour and
/// density. 0 before the game object exists.
pub unsafe fn game_fog_property() -> usize {
    let game: usize = rd(0x7C6CDC, 0);
    if game == 0 {
        0
    } else {
        rd(game, crate::gen::layouts::TES3Game::sgFogProperty)
    }
}

/// The renderer that the game draws with, or 0 before it exists. At the start of the game
/// the world controller does not have the renderer yet. The list of renderers has it from
/// its constructor on.
pub unsafe fn renderer() -> usize {
    let world_controller: usize = rd(WORLD_CONTROLLER, 0);
    let known: usize = if world_controller == 0 { 0 } else { rd(world_controller, WORLD_CONTROLLER_RENDERER) };
    if known != 0 {
        return known;
    }
    let node: usize = rd(RENDERER_LIST_HEAD, 0);
    if node == 0 {
        0
    } else {
        rd(node, RENDERER_NODE_RENDERER)
    }
}

/// `operator new` of the C runtime of the game.
pub unsafe fn operator_new(size: usize) -> usize {
    transmute::<usize, unsafe extern "C" fn(usize) -> usize>(0x727692)(size)
}

pub unsafe fn property_state_global_shade_property(property_state: usize) -> usize {
    thiscall0(0x6FE500, property_state) as usize
}

pub unsafe fn property_state_global_dither_property(property_state: usize) -> usize {
    thiscall0(0x6FE4E0, property_state) as usize
}

pub unsafe fn property_state_global_specular_property(property_state: usize) -> usize {
    thiscall0(0x6FE510, property_state) as usize
}

/// Returns the material of the property state, after it moved a pending change of the
/// material into its revision number.
pub unsafe fn property_state_update_current_material_revision(property_state: usize) -> usize {
    thiscall0(0x6FE4C0, property_state) as usize
}


/// A deliberate fault in the Rust code. It shows that the tests find a fault.
/// 0 is no fault. 1 is a wrong value in a device call, the Z compare function.
/// 2 is a wrong value in memory, the fog property that the render state remembers.
pub static FAULT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

pub fn fault() -> u32 {
    FAULT.load(core::sync::atomic::Ordering::Relaxed)
}

// The device calls are in `d3d`; these names are for the callers.
pub use crate::d3d::{device_set_render_state, device_set_texture_stage_state};

// ---- C runtime

/// 0x727530 `operator delete` of the C runtime of the game.
#[inline(always)]
pub unsafe fn operator_delete(block: usize) {
    transmute::<usize, unsafe extern "C" fn(usize)>(0x727530)(block)
}

/// 0x7276B0 `eh vector constructor iterator`. It is `__stdcall`. It calls `ctor` for each of
/// the `count` elements of `size` bytes. `dtor` is for the elements when a constructor throws.
#[inline(always)]
pub unsafe fn eh_vector_constructor_iterator(array: usize, size: usize, count: u32, ctor: usize, dtor: usize) {
    transmute::<usize, unsafe extern "stdcall" fn(usize, usize, u32, usize, usize)>(0x7276B0)(array, size, count, ctor, dtor)
}

/// 0x72759E `eh vector destructor iterator`. It is `__stdcall`. It calls `dtor` for each of
/// the `count` elements of `size` bytes, from the last element to the first.
#[inline(always)]
pub unsafe fn eh_vector_destructor_iterator(array: usize, size: usize, count: u32, dtor: usize) {
    transmute::<usize, unsafe extern "stdcall" fn(usize, usize, u32, usize)>(0x72759E)(array, size, count, dtor)
}

// ---- nullsub_36

/// 0x6E9A00 `nullsub_36`: the debug print of the renderer, a single `retn`. This form is
/// `__cdecl` with the text only.
#[inline(always)]
pub unsafe fn debug_print0(text: usize) {
    transmute::<usize, unsafe extern "C" fn(usize)>(0x6E9A00)(text)
}

/// 0x6E9A00 `nullsub_36` as `__cdecl` with the format and 1 value.
#[inline(always)]
pub unsafe fn debug_print1(format: usize, a: usize) {
    transmute::<usize, unsafe extern "C" fn(usize, usize)>(0x6E9A00)(format, a)
}

/// 0x6E9A00 `nullsub_36` as `__cdecl` with the format and 2 values.
#[inline(always)]
pub unsafe fn debug_print2(format: usize, a: usize, b: usize) {
    transmute::<usize, unsafe extern "C" fn(usize, usize, usize)>(0x6E9A00)(format, a, b)
}

/// 0x6E9A00 `nullsub_36` as the destructor of `NiBound`. This form is `__thiscall`. The bound
/// is in ECX. It is a function of its own because the calling convention is different.
#[inline(always)]
pub unsafe fn ni_bound_dtor(bound: usize) {
    thiscall0(0x6E9A00, bound);
}

// ---- NiCriticalSection

/// 0x693ED0 `NiCriticalSection::ctor`
#[inline(always)]
pub unsafe fn critical_section_ctor(section: usize) {
    thiscall0(0x693ED0, section);
}

/// 0x693EF0 `NiCriticalSection::dtor`
#[inline(always)]
pub unsafe fn critical_section_dtor(section: usize) {
    thiscall0(0x693EF0, section);
}

/// 0x693F00 `NiCriticalSection::Lock`. `name` is the address of the text that the game code
/// gives for the place of the lock.
#[inline(always)]
pub unsafe fn critical_section_lock(section: usize, name: usize) {
    thiscall1(0x693F00, section, name as u32);
}

/// 0x693F10 `NiCriticalSection::Unlock`
#[inline(always)]
pub unsafe fn critical_section_unlock(section: usize) {
    thiscall0(0x693F10, section);
}

// ---- NiRefObject, NiObject and NiPointer as calls of game functions
//
// The forms that the game code has inline are in `ports::ni`.

/// 0x404630 `NiObject::release`. It takes one reference from the object. At zero it deletes
/// the object.
#[inline(always)]
pub unsafe fn ni_object_release(object: usize) {
    thiscall0(0x404630, object);
}

/// 0x6E9ED0 `NiObjectNET::ctor`
#[inline(always)]
pub unsafe fn ni_object_net_ctor(object: usize) {
    thiscall0(0x6E9ED0, object);
}

/// 0x471B00 `NiPointer::set`. `pointer` is the address of the `NiPointer`.
#[inline(always)]
pub unsafe fn ni_pointer_set(pointer: usize, object: usize) {
    thiscall1(0x471B00, pointer, object as u32);
}

/// 0x6EA9C0 `NiPointer::operator=`. `base` calls it. It is not the function at 0x712850.
#[inline(always)]
pub unsafe fn ni_pointer_operator_assign(pointer: usize, object: usize) {
    thiscall1(0x6EA9C0, pointer, object as u32);
}

/// 0x712850 `NiPointer::assign`. `texture_pipeline` calls it. It is not the function at
/// 0x6EA9C0.
#[inline(always)]
pub unsafe fn ni_pointer_assign(pointer: usize, from: usize) {
    thiscall1(0x712850, pointer, from as u32);
}

/// 0x40A880 `NiPointer::dtor`. `pointer` is the address of the `NiPointer`.
#[inline(always)]
pub unsafe fn ni_pointer_dtor(pointer: usize) {
    thiscall0(NI_POINTER_DTOR, pointer);
}

// ---- NiPixelFormat and NiPixelData

/// 0x6ED7C0 `NiPixelFormat::ctor_RGBA`. Returns `this`.
#[inline(always)]
pub unsafe fn pixel_format_ctor_rgba(this: usize, red: u32, green: u32, blue: u32, alpha: u32, bits: u32) -> usize {
    thiscall5(0x6ED7C0, this, red, green, blue, alpha, bits) as usize
}

/// 0x6ED9D0 `NiPixelFormat::ctor_Palette`. Returns `this`.
#[inline(always)]
pub unsafe fn pixel_format_ctor_palette(this: usize, has_alpha: u32) -> usize {
    thiscall1(0x6ED9D0, this, has_alpha) as usize
}

/// 0x6EDA40 `NiPixelFormat::ctor_Format`. Returns `this`.
#[inline(always)]
pub unsafe fn pixel_format_ctor_format(this: usize, format: u32) -> usize {
    thiscall1(0x6EDA40, this, format) as usize
}

/// 0x6EDD30 `NiPixelFormat::ctor_BumpLuma`. Returns `this`.
#[inline(always)]
pub unsafe fn pixel_format_ctor_bump_luma(this: usize, red: u32, green: u32, blue: u32, bits: u32) -> usize {
    thiscall4(0x6EDD30, this, red, green, blue, bits) as usize
}

/// 0x6BFDA0 `NiPixelFormat::copy`
#[inline(always)]
pub unsafe fn pixel_format_copy(to: usize, from: usize) {
    thiscall1(0x6BFDA0, to, from as u32);
}

/// 0x6D4FC0 `NiPixelData::ctor_args`. `format` is the address of a `NiPixelFormat`. Returns
/// `this`.
#[inline(always)]
pub unsafe fn pixel_data_ctor_args(this: usize, width: u32, height: u32, format: usize, mip_map_levels: u32) -> usize {
    thiscall4(0x6D4FC0, this, width, height, format as u32, mip_map_levels) as usize
}

/// 0x6F57A0 `NiImageConverter::GetImageConverter`. It is static and `__cdecl`.
#[inline(always)]
pub unsafe fn get_image_converter() -> usize {
    cdecl0(0x6F57A0) as usize
}

// ---- Direct3D helpers of the game

/// 0x6C20A0 `GetNameForDirectXErrorCode`. It is `__cdecl`. Returns the address of a C string.
#[inline(always)]
pub unsafe fn directx_error_name(result: u32) -> usize {
    cdecl1(0x6C20A0, result) as usize
}

/// 0x6B6DE0 `NiTexture::RendererData::dtor`
#[inline(always)]
pub unsafe fn texture_renderer_data_dtor(data: usize) {
    thiscall0(0x6B6DE0, data);
}

/// 0x6C28C0 `sub_6C28C0`. It releases `pD3DResource` of a `NiDX8Resource`. The destructors of
/// the managers call this entry.
#[inline(always)]
pub unsafe fn dx8_resource_release(resource: usize) {
    thiscall0(0x6C28C0, resource);
}

/// 0x6BD490 `sub_6BD490`: a jump to 0x6C28C0. The `Recreate` functions call this entry. It is
/// a function of its own because the call goes to a different address.
#[inline(always)]
pub unsafe fn dx8_resource_release_thunk(resource: usize) {
    thiscall0(0x6BD490, resource);
}

/// 0x6BC8A0 `NiDX8TextureManager::clearHashMap` as a call of the game function. It removes
/// every node of a `THashMap` and sets the count to 0. `textures::clear_hash_map` is its port.
#[inline(always)]
pub unsafe fn texture_manager_clear_hash_map(map: usize) {
    thiscall0(0x6BC8A0, map);
}

/// 0x6B76E0 `NiDX8TexturePass::$1::ctor`: the constructor of a stage group of a texture pass.
/// Returns `this`.
#[inline(always)]
pub unsafe fn texture_pass_stage_group_ctor(group: usize) -> usize {
    thiscall0(0x6B76E0, group) as usize
}

// ---- containers and math

/// 0x6CB260 `TArray_of_pointer::SetAt`. `element` is the address of the value.
#[inline(always)]
pub unsafe fn array_set_at(array: usize, index: u32, element: usize) {
    thiscall2(0x6CB260, array, index, element as u32);
}

/// 0x6CEE40 `TArray_of_pointer::SetSize`
#[inline(always)]
pub unsafe fn array_set_size(array: usize, size: u32) {
    thiscall1(0x6CEE40, array, size);
}

/// 0x6BBEF0 `NiTListAllocator::Data::ctor`. It allocates `count` items and chains them.
/// Returns `block`.
#[inline(always)]
pub unsafe fn list_allocator_block_ctor(block: usize, count: u32) -> usize {
    thiscall1(0x6BBEF0, block, count) as usize
}

/// 0x6E7CD0 `matrix33::copy`
#[inline(always)]
pub unsafe fn matrix33_copy(to: usize, from: usize) {
    thiscall1(0x6E7CD0, to, from as u32);
}

/// 0x6ED100 `NiBound_or_NiQuaternion::Copy`
#[inline(always)]
pub unsafe fn ni_bound_copy(to: usize, from: usize) {
    thiscall1(0x6ED100, to, from as u32);
}

/// 0x6F25E0 `NiBound_or_NiPlane::ctor`
#[inline(always)]
pub unsafe fn ni_bound_ctor(bound: usize) {
    thiscall0(0x6F25E0, bound);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The values as decimal digits, first value first. It shows the order of the arguments.
    fn mix(values: &[u32]) -> u32 {
        values.iter().fold(0u32, |sum, &value| sum.wrapping_mul(10).wrapping_add(value))
    }

    unsafe extern "thiscall" fn this0(this: usize) -> u32 {
        this as u32 ^ 0x1000
    }
    unsafe extern "thiscall" fn this1(this: usize, a: u32) -> u32 {
        mix(&[this as u32, a])
    }
    unsafe extern "thiscall" fn this2(this: usize, a: u32, b: u32) -> u32 {
        mix(&[this as u32, a, b])
    }
    unsafe extern "thiscall" fn this3(this: usize, a: u32, b: u32, c: u32) -> u32 {
        mix(&[this as u32, a, b, c])
    }
    unsafe extern "thiscall" fn this4(this: usize, a: u32, b: u32, c: u32, d: u32) -> u32 {
        mix(&[this as u32, a, b, c, d])
    }
    unsafe extern "thiscall" fn this5(this: usize, a: u32, b: u32, c: u32, d: u32, e: u32) -> u32 {
        mix(&[this as u32, a, b, c, d, e])
    }
    unsafe extern "C" fn c0() -> u32 {
        7
    }
    unsafe extern "C" fn c1(a: u32) -> u32 {
        a
    }
    unsafe extern "C" fn c2(a: u32, b: u32) -> u32 {
        mix(&[a, b])
    }
    unsafe extern "C" fn c3(a: u32, b: u32, c: u32) -> u32 {
        mix(&[a, b, c])
    }

    fn addr<F>(f: F) -> usize
    where
        F: Copy,
    {
        assert_eq!(core::mem::size_of::<F>(), 4);
        unsafe { core::mem::transmute_copy::<F, usize>(&f) }
    }

    #[test]
    fn thiscall_gives_this_and_the_arguments_in_order() {
        unsafe {
            assert_eq!(thiscall0(addr(this0 as unsafe extern "thiscall" fn(usize) -> u32), 5), 0x1005);
            assert_eq!(thiscall1(addr(this1 as unsafe extern "thiscall" fn(usize, u32) -> u32), 1, 2), 12);
            assert_eq!(thiscall2(addr(this2 as unsafe extern "thiscall" fn(usize, u32, u32) -> u32), 1, 2, 3), 123);
            assert_eq!(thiscall3(addr(this3 as unsafe extern "thiscall" fn(usize, u32, u32, u32) -> u32), 1, 2, 3, 4), 1234);
            assert_eq!(
                thiscall4(addr(this4 as unsafe extern "thiscall" fn(usize, u32, u32, u32, u32) -> u32), 1, 2, 3, 4, 5),
                12345
            );
            assert_eq!(
                thiscall5(addr(this5 as unsafe extern "thiscall" fn(usize, u32, u32, u32, u32, u32) -> u32), 1, 2, 3, 4, 5, 6),
                123456
            );
        }
    }

    #[test]
    fn cdecl_gives_the_arguments_in_order() {
        unsafe {
            assert_eq!(cdecl0(addr(c0 as unsafe extern "C" fn() -> u32)), 7);
            assert_eq!(cdecl1(addr(c1 as unsafe extern "C" fn(u32) -> u32), 4), 4);
            assert_eq!(cdecl2(addr(c2 as unsafe extern "C" fn(u32, u32) -> u32), 4, 5), 45);
            assert_eq!(cdecl3(addr(c3 as unsafe extern "C" fn(u32, u32, u32) -> u32), 4, 5, 6), 456);
        }
    }

    #[test]
    fn a_virtual_call_reads_the_vtable_on_each_call() {
        // Slot 0x0 to slot 0xC of the first vtable have 0 to 3 stack arguments.
        let first = [
            addr(this0 as unsafe extern "thiscall" fn(usize) -> u32),
            addr(this1 as unsafe extern "thiscall" fn(usize, u32) -> u32),
            addr(this2 as unsafe extern "thiscall" fn(usize, u32, u32) -> u32),
            addr(this3 as unsafe extern "thiscall" fn(usize, u32, u32, u32) -> u32),
        ];
        // The second vtable has the function with 1 stack argument in slot 0x0.
        let second = [addr(this1 as unsafe extern "thiscall" fn(usize, u32) -> u32)];
        let mut object = [first.as_ptr() as usize];
        let o = object.as_mut_ptr() as usize;
        unsafe {
            assert_eq!(virtual_at(o, 0x8), first[2]);
            assert_eq!(vcall0(o, 0x0), o as u32 ^ 0x1000);
            assert_eq!(vcall1(o, 0x4, 2), mix(&[o as u32, 2]));
            assert_eq!(vcall2(o, 0x8, 2, 3), mix(&[o as u32, 2, 3]));
            assert_eq!(vcall3(o, 0xC, 2, 3, 4), mix(&[o as u32, 2, 3, 4]));
            // Another vtable is in the object, as after the recorder replaced it.
            crate::mem::wr(o, 0, second.as_ptr() as usize);
            assert_eq!(vcall1(o, 0x0, 9), mix(&[o as u32, 9]));
        }
    }
}
