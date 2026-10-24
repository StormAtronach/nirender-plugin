//! The ports of `NiDX8IndexBufferManager` and `NiDX8VertexBufferManager`. These classes hold the index
//! and vertex buffers of the renderer and fill them. The module also has the vertex buffer of
//! `NiD3DShaderInterface`.
//!
//! `NiDX8VertexBufferManager::PackSkinnedVB` (0x6BE2B0) has no port, because another mod replaces it
//! in memory.

use core::arch::asm;
use core::mem::transmute;

use crate::d3d::{
    buffer_get_desc, buffer_lock, buffer_unlock, com_release, device_add_ref, device_create_index_buffer,
    device_create_vertex_buffer, device_release, device_set_indices, D3DFMT_INDEX16, D3DFMT_VERTEXDATA, D3DFVF_DIFFUSE,
    D3DFVF_NORMAL, D3DFVF_TEX1, D3DFVF_XYZ, D3DRTYPE_INDEXBUFFER,
};
use crate::engine::{vcall0, virtual_at, CREATE_CRITICAL_SECTION, VTBL_NI_REF_OBJECT};
use crate::gen::layouts::{
    NiD3DShaderInterface as SI, NiD3DShaderInterface_Declaration as DECL, NiD3DShaderInterface_DeclarationElement as ELEM,
    NiD3DShaderInterface_ShaderEntry as SE, NiDX8IndexBufferManager as IM, NiDX8VBResource as VR, NiDX8VertexBufferManager as VM,
    NiDX8VertexBufferManager_VBInfo as VI, NiGeometryData as GD, NiSkinPartition_Partition as SP, THashMap as HM,
    THashMap_Node as HN,
};
use crate::mem::{rd, wr};
use crate::ports::{hash_map, ni};
use crate::shadow::Regions;

// The dwords of D3DVERTEXBUFFER_DESC and D3DINDEXBUFFER_DESC: Format, Type, Usage, Pool, Size, FVF.
// Only D3DVERTEXBUFFER_DESC has the FVF.
const DESC_FORMAT: usize = 0;
const DESC_TYPE: usize = 1;
const DESC_USAGE: usize = 2;
const DESC_POOL: usize = 3;
const DESC_SIZE: usize = 4;
const DESC_FVF: usize = 5;

/// The bucket count of each hash map of the two managers.
const BUCKETS: u32 = 37;
/// The number of resources of one `VBInfo`.
const RING: usize = 5;

/// The call at 0x6BEE96 uses it.
use crate::d3d::DEVICE_CREATE_VERTEX_SHADER;

/// The slot of `GetActiveVertexCount` in the vtable of a `NiGeometryData` (0x6BDE63, 0x6BF0D2).
const GEOMETRY_VTBL_ACTIVE_VERTEX_COUNT: usize = crate::gen::layouts::InterfaceNiGeometryData::GetActiveVertexCount;
/// The slot of the function of `NiD3DShaderInterface` that gives the vertex declaration (0x6BEE75).
const SHADER_VTBL_DECLARATION: usize = crate::gen::layouts::InterfaceNiD3DShaderInterface::GetDeclaration;

const ARRAY_STORAGE: usize = crate::gen::layouts::TArray::storage;
const ARRAY_STORAGE_COUNT: usize = crate::gen::layouts::TArray::storageCount;
const ARRAY_END_INDEX: usize = crate::gen::layouts::TArray::endIndex;
const ARRAY_FILLED_COUNT: usize = crate::gen::layouts::TArray::filledCount;
const ARRAY_GROW_BY: usize = crate::gen::layouts::TArray::growByCount;

// The object that `compileVertexShader` keeps for each shader. It has the vtable 0x74F768.
const SHADER_ENTRY_SIZE: usize = SE::SIZE;
const SHADER_ENTRY_NAME: usize = SE::name;
const SHADER_ENTRY_HANDLE: usize = SE::vertexShader;

// One vertex declaration of `NiD3DShaderInterface::declarations`. The game code multiplies the
// index by its size with a shift.
const DECLARATION_SHIFT: u32 = 4;
const DECLARATION_ELEMENTS: usize = DECL::elements;
const DECLARATION_STRIDE: usize = DECL::stride;
/// One element of a declaration. [`ELEMENT_OFFSET`] is its offset in the vertex.
const ELEMENT_SIZE: usize = ELEM::SIZE;
const ELEMENT_OFFSET: usize = ELEM::offset;
const _: () = assert!(1 << DECLARATION_SHIFT == DECL::SIZE);

const VTBL_INDEX_BUFFER_MANAGER: u32 = 0x74F724;
const VTBL_VERTEX_BUFFER_MANAGER: u32 = 0x74F73C;
const VTBL_SHADER_ENTRY: u32 = 0x74F768;

// The texts that the game code passes to its empty log function.
const TEXT_CREATE_INDEX_BUFFER: usize = 0x7C3E98;
const TEXT_PACK_BUFFER_LOCK: usize = 0x7C3ED0;
const TEXT_PACK_BUFFER_CREATE: usize = 0x7C3F18;
const TEXT_RESIZE_TEMP_POINT: usize = 0x7C3F60;
const TEXT_RESIZE_TEMP_LINE: usize = 0x7C3FB8;
const TEXT_CREATE_VERTEX_BUFFER_FAILED: usize = 0x7C400C;
/// The name that `CreateVertexBuffer` gives to `NiCriticalSection::Lock`.
const NAME_CREATE_VERTEX_BUFFER: usize = 0x7C4048;

const _: () = assert!(IM::SIZE == 0x4C && IM::hashmap_1C == 0x1C && IM::hashmap_2C == 0x2C && IM::hashmap_3C == 0x3C);
const _: () = assert!(VM::SIZE == 0x3C && VM::hashmap_C == 0xC && VM::hashmap_1C == 0x1C && VM::hashmapFVFUsage == 0x2C);
const _: () = assert!(VI::SIZE == 0x1C && VI::nextFreeResourceIndex == 4 && VI::resources == 8);
const _: () = assert!(VR::SIZE == 0x3C && VR::length == 0x14 && VR::pVB == 0x20 && VR::desc == 0x24);
const _: () = assert!(SI::SIZE == 0xC0 && SI::shaderFunctions == 0x30 && SI::shaderUsages == 0x48 && SI::field_90 == 0x90);
const _: () = assert!(SI::vertexShaderHandles == 0xA8 && SI::d3dDevice == 0xC && SI::field_1C == 0x1C && SI::declarations == 0x24);

// ---- engine functions and data at fixed addresses

#[cfg(not(test))]
mod eng {
    use core::arch::asm;
    use core::mem::transmute;

    use crate::engine::{thiscall1, thiscall2};

    pub use crate::engine::{
        array_set_at, array_set_size, critical_section_lock as lock, critical_section_unlock as unlock, debug_print0 as log,
        debug_print1 as log_value, directx_error_name as direct_x_error_name, dx8_resource_release as resource_release,
        dx8_resource_release_thunk as resource_release_thunk, operator_delete as delete, operator_new,
        texture_manager_clear_hash_map as clear_hash_map,
    };
    pub use crate::ports::ni::ref_object_count_add;

    /// 0x6FA000 `NiSkinInstance::Deform`
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn skin_deform(skin: usize, vertices: usize, unused: u32, normals: usize, count: u32, out_vertices: usize, out_normals: usize, stride: u32) {
        type F = unsafe extern "thiscall" fn(usize, usize, u32, usize, u32, usize, usize, u32) -> u32;
        transmute::<usize, F>(0x6FA000)(skin, vertices, unused, normals, count, out_vertices, out_normals, stride);
    }
    /// 0x6EF500 `NiGeometryData::getTextureSet`
    pub unsafe fn texture_set(geometry: usize, set: u32) -> usize {
        thiscall1(0x6EF500, geometry, set) as usize
    }
    /// 0x6BF720 `TArray::resize`, of the array of `NiPointer`.
    pub unsafe fn pointer_array_resize(array: usize, size: u32) {
        thiscall1(0x6BF720, array, size);
    }
    /// 0x6BF880 `TArray_of_NiPointer_to_NiAVObject::SetAt`. `pointer` is the address of the `NiPointer`.
    pub unsafe fn pointer_array_set_at(array: usize, index: u32, pointer: usize) {
        thiscall2(0x6BF880, array, index, pointer as u32);
    }
    /// 0x6BF3A0 `sub_6BF3A0`. It packs one element of a vertex declaration. It takes eight stack
    /// arguments, the shader interface in ECX and the declaration in EDI.
    pub unsafe fn pack_declaration_element(this: usize, declaration: usize, args: &[u32; 8]) {
        asm!(
            "push dword ptr [eax + 28]",
            "push dword ptr [eax + 24]",
            "push dword ptr [eax + 20]",
            "push dword ptr [eax + 16]",
            "push dword ptr [eax + 12]",
            "push dword ptr [eax + 8]",
            "push dword ptr [eax + 4]",
            "push dword ptr [eax]",
            "call edx",
            inout("eax") args.as_ptr() => _,
            inout("ecx") this => _,
            inout("edx") 0x6BF3A0usize => _,
            in("edi") declaration,
            clobber_abi("C"),
        );
    }
    /// `emptyString` (0x7C6180)
    pub fn empty_string() -> usize {
        crate::engine::EMPTY_STRING
    }
    /// `vtbl_sg_74F728`: the hash map of `NiDX8IndexBufferManager`.
    pub fn vtbl_index_map() -> usize {
        0x74F728
    }
    /// `vtbl_sg_74F740`: the hash map of `NiDX8VertexBufferManager`.
    pub fn vtbl_vertex_map() -> usize {
        0x74F740
    }
}

/// Replacements for the unit tests. The unit tests run without the game.
#[cfg(test)]
mod eng {
    use std::cell::RefCell;

    use crate::mem::{rd, wr};

    thread_local! {
        /// The blocks that `delete` got.
        pub static DELETED: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
        /// Calls of `pack_declaration_element`: this, declaration, the stack arguments.
        pub static PACKED: RefCell<Vec<(usize, usize, [u32; 8])>> = const { RefCell::new(Vec::new()) };
        /// Calls of `skin_deform`: vertices, normals, count, out vertices, out normals, stride.
        pub static DEFORMED: RefCell<Vec<(usize, usize, u32, usize, usize, u32)>> = const { RefCell::new(Vec::new()) };
    }

    #[repr(C)]
    pub struct MapVtbl {
        deleting_dtor: usize,
        hash_key: extern "thiscall" fn(usize, u32) -> u32,
        cmp_key: extern "thiscall" fn(usize, u32, u32) -> u32,
        make_key_value_pair: extern "thiscall" fn(usize, usize, u32, usize) -> u32,
        dtor_key_value_pair: extern "thiscall" fn(usize, usize) -> u32,
    }
    extern "thiscall" fn map_hash(map: usize, key: u32) -> u32 {
        key % unsafe { rd::<u32>(map, 8) }
    }
    extern "thiscall" fn map_cmp(_map: usize, a: u32, b: u32) -> u32 {
        // Only AL is the result.
        0x1234_5600 | (a == b) as u32
    }
    extern "thiscall" fn map_make(_map: usize, node: usize, key: u32, value: usize) -> u32 {
        unsafe {
            wr(node, 0, key);
            wr(node, 4, value);
        }
        0
    }
    extern "thiscall" fn map_dtor(_map: usize, _node: usize) -> u32 {
        0
    }
    static MAP_VTBL: MapVtbl =
        MapVtbl { deleting_dtor: 0, hash_key: map_hash, cmp_key: map_cmp, make_key_value_pair: map_make, dtor_key_value_pair: map_dtor };
    static EMPTY: [u8; 4] = [0; 4];

    pub unsafe fn operator_new(size: usize) -> usize {
        Box::leak(vec![0xCDu8; size].into_boxed_slice()).as_mut_ptr() as usize
    }
    pub unsafe fn delete(block: usize) {
        DELETED.with(|d| d.borrow_mut().push(block));
    }
    pub unsafe fn lock(_cs: usize, _name: usize) {}
    pub unsafe fn unlock(_cs: usize) {}
    pub unsafe fn log(_text: usize) {}
    pub unsafe fn log_value(_text: usize, _value: usize) {}
    pub unsafe fn direct_x_error_name(_code: u32) -> usize {
        0
    }
    pub unsafe fn clear_hash_map(_map: usize) {}
    pub unsafe fn resource_release(resource: usize) {
        wr::<u32>(resource, crate::gen::layouts::NiDX8VBResource::pD3DResource, 0);
    }
    pub unsafe fn resource_release_thunk(resource: usize) {
        resource_release(resource)
    }
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn skin_deform(_skin: usize, vertices: usize, _unused: u32, normals: usize, count: u32, out_vertices: usize, out_normals: usize, stride: u32) {
        DEFORMED.with(|d| d.borrow_mut().push((vertices, normals, count, out_vertices, out_normals, stride)));
    }
    /// The sets follow each other in one array, as in the game.
    pub unsafe fn texture_set(geometry: usize, set: u32) -> usize {
        use crate::gen::layouts::NiGeometryData;
        rd::<usize>(geometry, NiGeometryData::textureCoords) + set as usize * rd::<u16>(geometry, NiGeometryData::vertexCount) as usize * 8
    }
    pub unsafe fn array_set_size(array: usize, size: u32) {
        let storage = operator_new(size as usize * 4);
        core::ptr::write_bytes(storage as *mut u8, 0, size as usize * 4);
        wr(array, 4, storage);
        wr(array, 8, size);
    }
    pub unsafe fn array_set_at(array: usize, index: u32, element: usize) {
        wr::<u32>(rd::<usize>(array, 4), index as usize * 4, rd(element, 0));
    }
    pub unsafe fn pointer_array_resize(array: usize, size: u32) {
        array_set_size(array, size)
    }
    pub unsafe fn pointer_array_set_at(array: usize, index: u32, pointer: usize) {
        let object: usize = rd(pointer, 0);
        if object != 0 {
            wr(object, 4, rd::<u32>(object, 4) + 1);
        }
        wr(rd::<usize>(array, 4), index as usize * 4, object);
    }
    pub unsafe fn pack_declaration_element(this: usize, declaration: usize, args: &[u32; 8]) {
        PACKED.with(|p| p.borrow_mut().push((this, declaration, *args)));
    }
    pub fn empty_string() -> usize {
        EMPTY.as_ptr() as usize
    }
    pub fn vtbl_index_map() -> usize {
        &MAP_VTBL as *const MapVtbl as usize
    }
    pub fn vtbl_vertex_map() -> usize {
        &MAP_VTBL as *const MapVtbl as usize
    }
    pub unsafe fn ref_object_count_add(_delta: i32) {}
}

// ---- calls through a vtable

/// `IDirect3DDevice8::CreateVertexShader` through the vtable of the device, slot 0x12C. `d3d` has
/// no function for this slot. `out` is the address of the handle.
#[inline(always)]
unsafe fn device_create_vertex_shader(device: usize, declaration: usize, function: u32, out: usize, usage: u32) -> u32 {
    let f: unsafe extern "system" fn(usize, usize, u32, usize, u32) -> u32 = transmute(virtual_at(device, DEVICE_CREATE_VERTEX_SHADER));
    f(device, declaration, function, out, usage)
}

/// `NiGeometryData::GetActiveVertexCount`. The count is the low word of the result. The function
/// of the same name in `draw_misc` gives the low word only. This one gives the full register,
/// because `buildVertexBuffer` passes the full register on.
#[inline(always)]
unsafe fn geometry_active_vertex_count(geometry: usize) -> u32 {
    vcall0(geometry, GEOMETRY_VTBL_ACTIVE_VERTEX_COUNT)
}

// ---- x87 sequences

/// 0x6BE225: one vertex of a screen polygon.
/// `fld distance; fmul z; fadd near; fld st; fmul x; fstp out.x; fld st; fmul y; fstp out.y; fstp out.z`.
/// `input` has the bits of distance, z, near, x and y. The sum stays in the register between the stores.
fn x87_screen_vertex(input: &[u32; 5]) -> [u32; 3] {
    let mut output = [0u32; 3];
    unsafe {
        asm!(
            "fld dword ptr [{i}]",
            "fmul dword ptr [{i} + 4]",
            "fadd dword ptr [{i} + 8]",
            "fld st(0)",
            "fmul dword ptr [{i} + 12]",
            "fstp dword ptr [{o}]",
            "fld st(0)",
            "fmul dword ptr [{i} + 16]",
            "fstp dword ptr [{o} + 4]",
            "fstp dword ptr [{o} + 8]",
            i = in(reg) input.as_ptr(),
            o = in(reg) output.as_mut_ptr(),
            clobber_abi("C"),
            options(nostack),
        );
    }
    output
}

// ---- shared pieces that the game code has inline

/// Releases the buffers of every `VBInfo` of the map and frees the `VBInfo`. The nodes stay.
/// `thunk` selects which of the two entry points of the resource release the game code calls.
#[inline(always)]
unsafe fn map_release_values(map: usize, thunk: bool) {
    let mut node = hash_map::first(map);
    while node != 0 {
        let info: usize = rd(node, HN::data);
        node = hash_map::next(map, node);
        if info != 0 {
            for slot in 0..RING {
                let resource: usize = rd(info, VI::resources + slot * 4);
                if resource != 0 {
                    let buffer: usize = rd(resource, VR::pVB);
                    if buffer != 0 {
                        com_release(buffer);
                    }
                    if thunk {
                        eng::resource_release_thunk(resource);
                    } else {
                        eng::resource_release(resource);
                    }
                    eng::delete(resource);
                }
            }
            eng::delete(info);
        }
    }
}

/// Takes every node out of the map and frees it. Then the count becomes 0. The allocator is the
/// one of the `eng` module, thus the unit tests see each block.
#[inline(always)]
unsafe fn map_clear(map: usize) {
    hash_map::drain_and_zero_count_with(map, |node| eng::delete(node));
}

/// The constructor of a hash map with 37 buckets. The two manager constructors have it inline.
#[inline(always)]
unsafe fn map_construct(map: usize, vtbl: usize) {
    wr(map, HM::vtbl, vtbl);
    wr::<u32>(map, HM::bucketCount, BUCKETS);
    wr::<u32>(map, HM::count, 0);
    let buckets = eng::operator_new(0x94);
    let bytes = rd::<u32>(map, HM::bucketCount).wrapping_shl(2);
    wr(map, HM::arrayBuckets, buckets);
    core::ptr::write_bytes(buckets as *mut u8, 0, bytes as usize);
}

/// The destructor of a hash map. The two manager destructors have it inline.
#[inline(always)]
unsafe fn map_destruct(map: usize, vtbl: usize) {
    wr(map, HM::vtbl, vtbl);
    eng::clear_hash_map(map);
    eng::delete(rd(map, HM::arrayBuckets));
}

/// The body of the three hash map destructors. It frees every node, then the bucket array.
#[inline(always)]
unsafe fn map_dtor_body(map: usize, vtbl: usize) {
    wr(map, HM::vtbl, vtbl);
    hash_map::drain_with(map, |node| eng::delete(node));
    let buckets: usize = rd(map, HM::arrayBuckets);
    wr::<u32>(map, HM::count, 0);
    eng::delete(buckets);
}

/// Tells if the game code can use an index buffer again. The buffer must have 16 bit indices, the same
/// usage and pool, and a sufficient size. The result is [`None`] when `GetDesc` fails.
#[inline(always)]
unsafe fn index_buffer_fits(buffer: usize, usage: u32, pool: u32, size: u32) -> Option<bool> {
    let mut desc = [0u32; 6];
    if buffer_get_desc(buffer, desc.as_mut_ptr() as usize) != 0 {
        return None;
    }
    Some(
        desc[DESC_FORMAT] == D3DFMT_INDEX16
            && desc[DESC_TYPE] == D3DRTYPE_INDEXBUFFER
            && desc[DESC_USAGE] == usage
            && desc[DESC_POOL] == pool
            && desc[DESC_SIZE] >= size,
    )
}

/// Returns a new 16 bit index buffer, or 0.
#[inline(always)]
unsafe fn new_index_buffer(device: usize, size: u32, usage: u32, pool: u32) -> usize {
    let mut buffer: usize = 0;
    if device_create_index_buffer(device, size, usage, D3DFMT_INDEX16, pool, core::ptr::addr_of_mut!(buffer) as usize) != 0 {
        eng::log(TEXT_CREATE_INDEX_BUFFER);
        buffer = 0;
    }
    buffer
}

// ---- NiDX8IndexBufferManager

/// 0x6BC970 `NiDX8IndexBufferManager::ctor`. Returns `this`.
pub unsafe fn index_manager_ctor(this: usize, device: usize) -> usize {
    crate::shadow::touch(&index_manager_ctor_port::STATE);
    wr(this, IM::vtbl, VTBL_NI_REF_OBJECT);
    wr::<u32>(this, IM::refCount, 0);
    eng::ref_object_count_add(1);
    map_construct(this + IM::hashmap_1C, eng::vtbl_index_map());
    map_construct(this + IM::hashmap_2C, eng::vtbl_index_map());
    map_construct(this + IM::hashmap_3C, eng::vtbl_index_map());
    wr(this, IM::vtbl, VTBL_INDEX_BUFFER_MANAGER);
    wr(this, IM::device, device);
    device_add_ref(device);
    wr::<u32>(this, IM::indexBuffer, 0);
    wr::<u32>(this, IM::indexBufferSize, 0);
    wr::<u32>(this, IM::field_14, 0);
    wr::<u32>(this, IM::field_18, 0);
    for map in [IM::hashmap_1C, IM::hashmap_2C, IM::hashmap_3C] {
        map_clear(this + map);
    }
    this
}

/// 0x6BCBA0 `NiDX8IndexBufferManager::dtor`
pub unsafe fn index_manager_dtor(this: usize) {
    crate::shadow::touch(&index_manager_dtor_port::STATE);
    wr(this, IM::vtbl, VTBL_INDEX_BUFFER_MANAGER);
    for map in [IM::hashmap_1C, IM::hashmap_2C, IM::hashmap_3C] {
        map_release_values(this + map, false);
    }
    for map in [IM::hashmap_1C, IM::hashmap_2C, IM::hashmap_3C] {
        map_clear(this + map);
    }
    device_set_indices(rd(this, IM::device), 0, 0);
    let quads: usize = rd(this, IM::indexBuffer);
    if quads != 0 {
        com_release(quads);
    }
    let lines: usize = rd(this, IM::field_14);
    if lines != 0 {
        com_release(lines);
    }
    let device: usize = rd(this, IM::device);
    if device != 0 {
        device_release(device);
        wr::<u32>(this, IM::device, 0);
    }
    map_destruct(this + IM::hashmap_3C, eng::vtbl_index_map());
    map_destruct(this + IM::hashmap_2C, eng::vtbl_index_map());
    map_destruct(this + IM::hashmap_1C, eng::vtbl_index_map());
    wr(this, IM::vtbl, VTBL_NI_REF_OBJECT);
    eng::ref_object_count_add(-1);
}

/// 0x6BCB80 `NiDX8IndexBufferManager::deleting_dtor`. Returns `this`.
pub unsafe fn index_manager_deleting_dtor(this: usize, flags: u8) -> usize {
    crate::shadow::touch(&index_manager_deleting_dtor_port::STATE);
    index_manager_dtor(this);
    if flags & 1 != 0 {
        eng::delete(this);
    }
    this
}

/// 0x6BCE80 `NiDX8IndexBufferManager::createIndexBuffer`. It puts the indices of a shape into a 16 bit
/// index buffer. It uses `index_buffer` again when that buffer fits.
pub unsafe fn create_index_buffer(this: usize, indices: usize, index_count: u32, index_buffer: usize, out_buffer_size: usize, pool: u32, usage: u32) -> usize {
    crate::shadow::touch(&create_index_buffer_port::STATE);
    if rd::<usize>(this, IM::device) == 0 {
        return 0;
    }
    if indices == 0 {
        return 0;
    }
    let mut buffer = index_buffer;
    let size = index_count.wrapping_add(index_count);
    if buffer == 0 || index_buffer_fits(buffer, usage, pool, size) != Some(true) {
        buffer = new_index_buffer(rd(this, IM::device), size, usage, pool);
        if buffer == 0 {
            eng::log(TEXT_PACK_BUFFER_CREATE);
            return 0;
        }
        wr(out_buffer_size, 0, size);
    }
    let mut data: usize = 0;
    if buffer_lock(buffer, 0, size, core::ptr::addr_of_mut!(data) as usize, 0) != 0 {
        eng::log(TEXT_PACK_BUFFER_LOCK);
        buffer_unlock(buffer);
        com_release(buffer);
        return buffer;
    }
    core::ptr::copy_nonoverlapping(indices as *const u8, data as *mut u8, size as usize);
    buffer_unlock(buffer);
    buffer
}

/// 0x6BCFE0 `NiDX8IndexBufferManager::getOrBuildQuadIndexBuffer`. It gives the index buffer of
/// `quads` quads, with two triangles for each quad. With `fill`, it writes the indices also when the
/// buffer of the manager fits.
pub unsafe fn get_or_build_quad_index_buffer(this: usize, quads: u32, fill: u8, usage: u32, pool: u32) -> usize {
    crate::shadow::touch(&get_or_build_quad_index_buffer_port::STATE);
    if rd::<usize>(this, IM::device) == 0 {
        return 0;
    }
    if quads == 0 {
        return 0;
    }
    let mut buffer: usize = rd(this, IM::indexBuffer);
    let size = quads.wrapping_mul(3).wrapping_shl(2);
    let mut fits = false;
    if buffer != 0 {
        match index_buffer_fits(buffer, usage, pool, size) {
            Some(true) => fits = true,
            Some(false) => {
                com_release(buffer);
            }
            None => {}
        }
    }
    if fits {
        if fill == 0 {
            return buffer;
        }
    } else {
        buffer = new_index_buffer(rd(this, IM::device), size, usage, pool);
        if buffer == 0 {
            eng::log(TEXT_RESIZE_TEMP_POINT);
            return 0;
        }
    }
    let mut data: usize = 0;
    if buffer_lock(buffer, 0, size, core::ptr::addr_of_mut!(data) as usize, 0) != 0 {
        eng::log(TEXT_PACK_BUFFER_LOCK);
        buffer_unlock(buffer);
        com_release(buffer);
        buffer = 0;
    } else {
        let mut at = data;
        let mut base: u32 = 0;
        for _ in 0..quads {
            wr::<u16>(at, 0, base as u16);
            wr::<u16>(at, 2, base.wrapping_add(1) as u16);
            wr::<u16>(at, 4, base.wrapping_add(2) as u16);
            wr::<u16>(at, 6, base as u16);
            wr::<u16>(at, 8, base.wrapping_add(2) as u16);
            wr::<u16>(at, 10, base.wrapping_add(3) as u16);
            base = base.wrapping_add(4);
            at += 12;
        }
        buffer_unlock(buffer);
    }
    wr(this, IM::indexBuffer, buffer);
    wr(this, IM::indexBufferSize, size);
    buffer
}

/// 0x6BD190 `NiDX8IndexBufferManager::buildLineSegmentIndexBuffer`. It makes a line from each vertex
/// with a flag to the next vertex. The last vertex connects to the first vertex.
pub unsafe fn build_line_segment_index_buffer(this: usize, vertex_count: u32, flags: usize, out_index_count: usize, usage: u32, pool: u32) -> usize {
    crate::shadow::touch(&build_line_segment_index_buffer_port::STATE);
    if rd::<usize>(this, IM::device) == 0 {
        return 0;
    }
    if vertex_count == 0 {
        return 0;
    }
    if flags == 0 {
        return 0;
    }
    let mut buffer: usize = rd(this, IM::field_14);
    let size = vertex_count.wrapping_mul(4).wrapping_add(4);
    let mut fits = false;
    if buffer != 0 {
        match index_buffer_fits(buffer, usage, pool, size) {
            Some(true) => fits = true,
            Some(false) => {
                com_release(buffer);
            }
            None => {}
        }
    }
    if !fits {
        buffer = new_index_buffer(rd(this, IM::device), size, usage, pool);
        if buffer == 0 {
            eng::log(TEXT_RESIZE_TEMP_LINE);
            return 0;
        }
    }
    wr::<u32>(out_index_count, 0, 0);
    let mut data: usize = 0;
    if buffer_lock(buffer, 0, size, core::ptr::addr_of_mut!(data) as usize, 0) != 0 {
        eng::log(TEXT_PACK_BUFFER_LOCK);
        buffer_unlock(buffer);
        com_release(buffer);
        buffer = 0;
    } else {
        let mut at = data;
        let mut flag = flags;
        let last = vertex_count.wrapping_sub(1);
        let mut i: u32 = 0;
        while i < last {
            if rd::<u8>(flag, 0) != 0 {
                wr::<u16>(at, 0, i as u16);
                wr::<u16>(at, 2, i.wrapping_add(1) as u16);
                at += 4;
                wr(out_index_count, 0, rd::<u32>(out_index_count, 0).wrapping_add(2));
            }
            flag += 1;
            i += 1;
        }
        if rd::<u8>(flag, 0) != 0 {
            wr::<u16>(at, 0, last as u16);
            wr::<u16>(at, 2, 0);
            wr(out_index_count, 0, rd::<u32>(out_index_count, 0).wrapping_add(2));
        }
        buffer_unlock(buffer);
    }
    wr(this, IM::field_14, buffer);
    wr(this, IM::field_18, size);
    buffer
}

/// 0x6BD370 `NiDX8IndexBufferManager::Recreate`. It empties the first map.
pub unsafe fn index_manager_recreate(this: usize) {
    crate::shadow::touch(&index_manager_recreate_port::STATE);
    let map = this + IM::hashmap_1C;
    map_release_values(map, true);
    map_clear(map);
}

/// 0x6BD560 `NiDX8IndexBufferManager::HashMap::dtor`
pub unsafe fn index_map_dtor(map: usize) {
    crate::shadow::touch(&index_map_dtor_port::STATE);
    map_dtor_body(map, eng::vtbl_index_map());
}

// ---- NiDX8VertexBufferManager

/// 0x6BD780 `NiDX8VertexBufferManager::ctor`. Returns `this`.
pub unsafe fn vertex_manager_ctor(this: usize, device: usize) -> usize {
    crate::shadow::touch(&vertex_manager_ctor_port::STATE);
    wr(this, VM::vtbl, VTBL_NI_REF_OBJECT);
    wr::<u32>(this, VM::refCount, 0);
    eng::ref_object_count_add(1);
    map_construct(this + VM::hashmap_C, eng::vtbl_vertex_map());
    map_construct(this + VM::hashmap_1C, eng::vtbl_vertex_map());
    map_construct(this + VM::hashmapFVFUsage, eng::vtbl_vertex_map());
    wr(this, VM::vtbl, VTBL_VERTEX_BUFFER_MANAGER);
    wr(this, VM::device, device);
    device_add_ref(device);
    for map in [VM::hashmap_C, VM::hashmap_1C, VM::hashmapFVFUsage] {
        map_clear(this + map);
    }
    this
}

/// 0x6BD9B0 `NiDX8VertexBufferManager::dtor`
pub unsafe fn vertex_manager_dtor(this: usize) {
    crate::shadow::touch(&vertex_manager_dtor_port::STATE);
    wr(this, VM::vtbl, VTBL_VERTEX_BUFFER_MANAGER);
    for map in [VM::hashmap_C, VM::hashmap_1C, VM::hashmapFVFUsage] {
        map_release_values(this + map, false);
    }
    for map in [VM::hashmap_C, VM::hashmap_1C, VM::hashmapFVFUsage] {
        map_clear(this + map);
    }
    // The game code does not test for a device here. The index buffer manager does.
    device_release(rd(this, VM::device));
    wr::<u32>(this, VM::device, 0);
    map_destruct(this + VM::hashmapFVFUsage, eng::vtbl_vertex_map());
    map_destruct(this + VM::hashmap_1C, eng::vtbl_vertex_map());
    map_destruct(this + VM::hashmap_C, eng::vtbl_vertex_map());
    wr(this, VM::vtbl, VTBL_NI_REF_OBJECT);
    eng::ref_object_count_add(-1);
}

/// 0x6BD990 `NiDX8VertexBufferManager::deleting_dtor`. Returns `this`.
pub unsafe fn vertex_manager_deleting_dtor(this: usize, flags: u8) -> usize {
    crate::shadow::touch(&vertex_manager_deleting_dtor_port::STATE);
    vertex_manager_dtor(this);
    if flags & 1 != 0 {
        eng::delete(this);
    }
    this
}

/// 0x6C28A0 `NiDX8VBResource::ctor`. Returns `this`.
pub unsafe fn vb_resource_ctor(this: usize) -> usize {
    crate::shadow::touch(&vb_resource_ctor_port::STATE);
    wr::<u32>(this, VR::resourceId, 0xFFFF_FFFF);
    wr::<u32>(this, VR::type_, 0xFFFF);
    wr::<u32>(this, VR::pool, 0);
    wr::<u32>(this, VR::pD3DResource, 0);
    this
}

/// The map of a pool. The pools are default, managed and system memory.
#[inline(always)]
fn pool_map(pool: u32) -> Option<usize> {
    match pool {
        0 => Some(VM::hashmap_C),
        1 => Some(VM::hashmap_1C),
        2 => Some(VM::hashmapFVFUsage),
        _ => None,
    }
}

/// 0x6BE690 `NiDX8VertexBufferManager::GetVBInfo`. It gives the `VBInfo` of a usage and a vertex
/// format in the map of the pool. It makes the `VBInfo` when the map has none.
pub unsafe fn get_vb_info(this: usize, usage: u32, fvf: u32, pool: u32) -> usize {
    crate::shadow::touch(&get_vb_info_port::STATE);
    let key = usage.wrapping_shl(16) | fvf;
    let map = match pool_map(pool) {
        Some(map) => this + map,
        None => return 0,
    };
    let node = hash_map::find(map, key);
    if node != 0 {
        return rd(node, HN::data);
    }
    let info = eng::operator_new(VI::SIZE);
    wr::<u8>(info, VI::nextFreeResourceIndex, 0);
    for slot in 0..RING {
        wr::<u32>(info, VI::resources + slot * 4, 0);
    }
    wr(info, VI::vb_id, key);
    if pool == 2 {
        fvf_usage_hash_insert(map, key, info);
    } else {
        // The game code has the same insert inline.
        hash_map::set_with(map, key, info, |size| eng::operator_new(size));
    }
    info
}

/// 0x6BEA90 `NiDX8VertexBufferManager::FVFUsageHash::insert`
pub unsafe fn fvf_usage_hash_insert(map: usize, key: u32, value: usize) {
    crate::shadow::touch(&fvf_usage_hash_insert_port::STATE);
    hash_map::set_with(map, key, value, |size| eng::operator_new(size));
}

/// 0x6BDC70 `NiDX8VertexBufferManager::CreateVertexBuffer`. It gives the next of the five buffers of
/// a usage, format and pool. It releases a buffer that is too small and makes it again.
pub unsafe fn create_vertex_buffer(this: usize, byte_count: u32, usage: u32, fvf: u32, pool: u32) -> usize {
    crate::shadow::touch(&create_vertex_buffer_port::STATE);
    if rd::<usize>(this, VM::device) == 0 {
        return 0;
    }
    let mut result: usize = 0;
    let info = get_vb_info(this, usage, fvf, pool);
    if info == 0 {
        return result;
    }
    let index: u8 = rd(info, VI::nextFreeResourceIndex);
    let mut resource: usize = rd(info, VI::resources + index as usize * 4);
    if resource != 0 {
        if rd::<u32>(resource, VR::length) >= byte_count {
            result = rd(resource, VR::pVB);
            wr::<u8>(info, VI::nextFreeResourceIndex, index.wrapping_add(1));
        } else {
            com_release(rd(resource, VR::pVB));
            wr::<u32>(resource, VR::pVB, 0);
        }
    } else {
        let block = eng::operator_new(VR::SIZE);
        resource = if block != 0 { vb_resource_ctor(block) } else { 0 };
        wr(resource, VR::usage, usage);
        wr(resource, VR::fvf, fvf);
        wr(resource, VR::length, byte_count);
        wr::<u32>(resource, VR::type_, 3);
        wr(resource, VR::pool, pool);
        wr::<u32>(resource, VR::revisionId, 0);
        wr::<u32>(resource, VR::pVB, 0);
        let index: u8 = rd(info, VI::nextFreeResourceIndex);
        wr(info, VI::resources + index as usize * 4, resource);
    }
    if rd::<usize>(resource, VR::pVB) == 0 {
        let mut created: usize = 0;
        eng::lock(CREATE_CRITICAL_SECTION, NAME_CREATE_VERTEX_BUFFER);
        let error = device_create_vertex_buffer(rd(this, VM::device), byte_count, usage, fvf, pool, core::ptr::addr_of_mut!(created) as usize);
        eng::unlock(CREATE_CRITICAL_SECTION);
        let buffer = if error != 0 {
            eng::log_value(TEXT_CREATE_VERTEX_BUFFER_FAILED, eng::direct_x_error_name(error));
            0
        } else {
            buffer_get_desc(created, resource + VR::desc);
            created
        };
        wr(resource, VR::pVB, buffer);
        wr::<u32>(resource, VR::revisionId, 0);
        wr(resource, VR::length, byte_count);
        wr(resource, VR::fvf, fvf);
        result = buffer;
        wr::<u8>(info, VI::nextFreeResourceIndex, rd::<u8>(info, VI::nextFreeResourceIndex).wrapping_add(1));
    }
    if rd::<u8>(info, VI::nextFreeResourceIndex) == RING as u8 {
        wr::<u8>(info, VI::nextFreeResourceIndex, 0);
    }
    result
}

/// 0x6BDE50 `NiDX8VertexBufferManager::PackUntransformedVB`. It writes the positions, normals, colours
/// and texture coordinates of a shape, one after the other in each vertex. It uses `previous_vb`
/// again when its description fits. With a skin instance, the positions and normals come from
/// `NiSkinInstance::Deform`. Then `refill` tells if the function writes the remaining data too.
#[allow(clippy::too_many_arguments)]
pub unsafe fn pack_untransformed_vb(
    this: usize,
    geometry: usize,
    previous_vb: usize,
    out_buffer_size: usize,
    skin: usize,
    refill: u8,
    pool: u32,
    usage: u32,
    out_stride: usize,
    out_fvf: usize,
) -> usize {
    crate::shadow::touch(&pack_untransformed_vb_port::STATE);
    let active = geometry_active_vertex_count(geometry);
    if active as u16 == 0 {
        return 0;
    }
    let mut vertices: usize = rd(geometry, GD::vertices);
    let mut normals: usize = rd(geometry, GD::normals);
    let mut sets = rd::<u16>(geometry, GD::textureSets) as u32;
    let colors: usize = rd(geometry, GD::colors);
    if sets > 8 {
        sets = 8;
    }
    let mut fvf = D3DFVF_XYZ;
    let mut stride: u32 = 12;
    let mut normal_offset: usize = 0;
    let mut color_offset: usize = 0;
    if normals != 0 {
        fvf = D3DFVF_XYZ | D3DFVF_NORMAL;
        normal_offset = stride as usize;
        stride = 24;
    }
    if colors != 0 {
        fvf |= D3DFVF_DIFFUSE;
        color_offset = stride as usize;
        stride += 4;
    }
    let uv_offset = stride as usize;
    if sets != 0 {
        stride += sets * 8;
        fvf |= sets << 8;
    } else {
        fvf |= D3DFVF_TEX1;
        stride += 8;
    }
    let count = active & 0xFFFF;
    let mut refill = refill;
    let mut buffer = previous_vb;
    let mut fits = false;
    if previous_vb != 0 {
        // The game code does not examine the result of `GetDesc`. It reads the values on its stack in
        // all cases.
        let mut desc = [0u32; 6];
        buffer_get_desc(previous_vb, desc.as_mut_ptr() as usize);
        fits = desc[DESC_FORMAT] == D3DFMT_VERTEXDATA
            && desc[DESC_FVF] == fvf
            && desc[DESC_SIZE] == count.wrapping_mul(stride)
            && desc[DESC_POOL] == pool
            && desc[DESC_USAGE] == usage;
    }
    if !fits {
        let mut created: usize = 0;
        eng::lock(CREATE_CRITICAL_SECTION, NAME_CREATE_VERTEX_BUFFER);
        let length = count.wrapping_mul(stride);
        let error = device_create_vertex_buffer(rd(this, VM::device), length, usage, fvf, pool, core::ptr::addr_of_mut!(created) as usize);
        eng::unlock(CREATE_CRITICAL_SECTION);
        if error != 0 {
            eng::log_value(TEXT_CREATE_VERTEX_BUFFER_FAILED, eng::direct_x_error_name(error));
            created = 0;
        }
        buffer = created;
        wr(out_buffer_size, 0, length);
        refill = 1;
    }
    let mut data: usize = 0;
    if buffer == 0 {
        return 0;
    }
    if buffer_lock(buffer, 0, 0, core::ptr::addr_of_mut!(data) as usize, 0) != 0 {
        return 0;
    }
    if skin != 0 {
        let out_normals = if normals != 0 { normal_offset + data } else { 0 };
        eng::skin_deform(
            skin,
            rd(geometry, GD::vertices),
            12,
            rd(geometry, GD::normals),
            rd::<u16>(geometry, GD::vertexCount) as u32,
            data,
            out_normals,
            stride,
        );
        if refill == 0 {
            return if buffer_unlock(buffer) == 0 { buffer } else { 0 };
        }
    } else {
        let mut to = data;
        for _ in 0..count {
            wr::<u32>(to, 0, rd(vertices, 0));
            wr::<u32>(to, 4, rd(vertices, 4));
            wr::<u32>(to, 8, rd(vertices, 8));
            to += stride as usize;
            vertices += 12;
        }
        if normals != 0 {
            let mut to = normal_offset + data;
            for _ in 0..count {
                wr::<u32>(to, 0, rd(normals, 0));
                wr::<u32>(to, 4, rd(normals, 4));
                wr::<u32>(to, 8, rd(normals, 8));
                to += stride as usize;
                normals += 12;
            }
        }
    }
    if colors != 0 {
        let mut to = color_offset + data;
        let mut from = colors;
        for _ in 0..count {
            wr::<u32>(to, 0, rd(from, 0));
            from += 4;
            to += stride as usize;
        }
    }
    let mut set: u32 = 0;
    while set < sets {
        let mut to = data + set as usize * 8 + uv_offset;
        let mut from = eng::texture_set(geometry, set);
        for _ in 0..count {
            wr::<u32>(to, 0, rd(from, 0));
            wr::<u32>(to, 4, rd(from, 4));
            from += 8;
            to += stride as usize;
        }
        set += 1;
    }
    if buffer_unlock(buffer) != 0 {
        return 0;
    }
    if out_stride != 0 {
        wr(out_stride, 0, stride);
    }
    if out_fvf != 0 {
        wr(out_fvf, 0, fvf);
    }
    buffer
}

/// 0x6BE180 `NiDX8VertexBufferManager::PackScreenPolyVB`. It writes the vertices of a screen polygon
/// and scales each vertex by its depth. `frustum_near` and `frustum_distance` are the bits of the
/// floats.
#[allow(clippy::too_many_arguments)]
pub unsafe fn pack_screen_poly_vb(
    this: usize,
    vertex_count: u32,
    vertices: usize,
    tex_coords: usize,
    colors: usize,
    frustum_near: u32,
    frustum_distance: u32,
    out_fvf: usize,
    out_stride: usize,
) -> usize {
    crate::shadow::touch(&pack_screen_poly_vb_port::STATE);
    wr::<u32>(out_fvf, 0, D3DFVF_XYZ);
    let mut stride: u32 = 12;
    let mut color_offset: usize = 0;
    let mut uv_offset: usize = 0;
    if colors != 0 {
        color_offset = stride as usize;
        wr::<u32>(out_fvf, 0, D3DFVF_XYZ | D3DFVF_DIFFUSE);
        stride = 16;
    }
    if tex_coords != 0 {
        let fvf: u32 = rd(out_fvf, 0);
        uv_offset = stride as usize;
        stride += 8;
        wr(out_fvf, 0, fvf | D3DFVF_TEX1);
    }
    let count = vertex_count & 0xFFFF;
    let buffer = create_vertex_buffer(this, count.wrapping_mul(stride), 0, rd(out_fvf, 0), 0);
    if buffer == 0 {
        return 0;
    }
    let mut data: usize = 0;
    if buffer_lock(buffer, 0, 0, core::ptr::addr_of_mut!(data) as usize, 0) != 0 {
        return 0;
    }
    let mut to = data;
    let mut from = vertices;
    for _ in 0..count {
        let vertex = x87_screen_vertex(&[frustum_distance, rd(from, 8), frustum_near, rd(from, 0), rd(from, 4)]);
        wr::<u32>(to, 0, vertex[0]);
        wr::<u32>(to, 4, vertex[1]);
        wr::<u32>(to, 8, vertex[2]);
        from += 12;
        to += stride as usize;
    }
    if colors != 0 {
        let mut to = data + color_offset;
        let mut from = colors;
        for _ in 0..count {
            wr::<u32>(to, 0, rd(from, 0));
            from += 4;
            to += stride as usize;
        }
    }
    if tex_coords != 0 {
        let mut to = uv_offset + data;
        let mut from = tex_coords;
        for _ in 0..count {
            wr::<u32>(to, 0, rd(from, 0));
            wr::<u32>(to, 4, rd(from, 4));
            from += 8;
            to += stride as usize;
        }
    }
    wr(out_stride, 0, stride);
    if buffer_unlock(buffer) == 0 {
        buffer
    } else {
        0
    }
}

/// 0x6BE910 `NiDX8VertexBufferManager::Recreate`. It empties the map of the default pool.
pub unsafe fn vertex_manager_recreate(this: usize) {
    crate::shadow::touch(&vertex_manager_recreate_port::STATE);
    let map = this + VM::hashmap_C;
    map_release_values(map, true);
    map_clear(map);
}

/// 0x6BEA30 `NiDX8VertexBufferManager::HashMap::dtor`
pub unsafe fn vertex_map_dtor(map: usize) {
    crate::shadow::touch(&vertex_map_dtor_port::STATE);
    map_dtor_body(map, eng::vtbl_vertex_map());
}

/// 0x6BEB20 `NiDX8VertexBufferManager::Hash::deleting_dtor`. Returns `this`.
pub unsafe fn vertex_map_deleting_dtor(map: usize, flags: u8) -> usize {
    crate::shadow::touch(&vertex_map_deleting_dtor_port::STATE);
    map_dtor_body(map, eng::vtbl_vertex_map());
    if flags & 1 != 0 {
        eng::delete(map);
    }
    map
}

// ---- NiD3DShaderInterface

/// 0x6BEE00 `NiD3DShaderInterface::compileVertexShader`. It keeps the function and the usage of
/// shader `index` and makes the shader. Returns the handle, or 0 when there is none.
pub unsafe fn compile_vertex_shader(this: usize, shader_function: u32, shader_usage: u32, index: u32) -> u32 {
    crate::shadow::touch(&compile_vertex_shader_port::STATE);
    let mut handle: u32 = 0;
    let function = shader_function;
    let usage = shader_usage;
    let functions = this + SI::shaderFunctions;
    if index >= rd::<u32>(functions, ARRAY_STORAGE_COUNT) {
        eng::array_set_size(functions, rd::<u32>(functions, ARRAY_GROW_BY).wrapping_add(index));
    }
    eng::array_set_at(functions, index, core::ptr::addr_of!(function) as usize);
    let usages = this + SI::shaderUsages;
    if index >= rd::<u32>(usages, ARRAY_STORAGE_COUNT) {
        eng::array_set_size(usages, rd::<u32>(usages, ARRAY_GROW_BY).wrapping_add(index));
    }
    eng::array_set_at(usages, index, core::ptr::addr_of!(usage) as usize);
    let declaration = vcall0(this, SHADER_VTBL_DECLARATION) as usize;
    if declaration == 0 {
        return handle;
    }
    let entries = this + SI::field_90;
    let handles = this + SI::vertexShaderHandles;
    let result = device_create_vertex_shader(rd(this, SI::d3dDevice), declaration, function, core::ptr::addr_of_mut!(handle) as usize, usage);
    if (result as i32) < 0 {
        let entry: usize = 0;
        if index >= rd::<u32>(entries, ARRAY_STORAGE_COUNT) {
            eng::pointer_array_resize(entries, rd::<u32>(entries, ARRAY_GROW_BY).wrapping_add(index));
        }
        eng::pointer_array_set_at(entries, index, core::ptr::addr_of!(entry) as usize);
        ni::release_if_set(core::ptr::addr_of!(entry).read_volatile());
        let none: u32 = 0;
        if index >= rd::<u32>(handles, ARRAY_STORAGE_COUNT) {
            eng::array_set_size(handles, rd::<u32>(handles, ARRAY_GROW_BY).wrapping_add(index));
        }
        eng::array_set_at(handles, index, core::ptr::addr_of!(none) as usize);
        return 0;
    }
    let mut entry: usize = rd(rd::<usize>(entries, ARRAY_STORAGE), index as usize * 4);
    if entry != 0 {
        wr(entry, 4, rd::<u32>(entry, 4).wrapping_add(1));
    }
    if entry == 0 {
        let block = eng::operator_new(SHADER_ENTRY_SIZE);
        if block != 0 {
            wr(block, SE::vtbl, VTBL_NI_REF_OBJECT);
            wr::<u32>(block, SE::refCount, 0);
            eng::ref_object_count_add(1);
            wr(block, SE::vtbl, VTBL_SHADER_ENTRY);
            wr::<u32>(block, SE::unknown_8, 0);
            wr::<u32>(block, SE::unknown_C, 0);
            // This loop is the `strcpy` of the empty string of the game.
            let mut from = eng::empty_string();
            let mut to = block + SHADER_ENTRY_NAME;
            loop {
                let byte: u8 = rd(from, 0);
                wr(to, 0, byte);
                from += 1;
                to += 1;
                if byte == 0 {
                    break;
                }
            }
            wr::<u32>(block, SHADER_ENTRY_HANDLE, 0);
        }
        if entry != block {
            ni::release_if_set(entry);
            entry = block;
            if block != 0 {
                wr(block, 4, rd::<u32>(block, 4).wrapping_add(1));
            }
        }
    }
    wr(entry, SHADER_ENTRY_HANDLE, handle);
    if index >= rd::<u32>(entries, ARRAY_STORAGE_COUNT) {
        eng::pointer_array_resize(entries, rd::<u32>(entries, ARRAY_GROW_BY).wrapping_add(index));
    }
    eng::pointer_array_set_at(entries, index, core::ptr::addr_of!(entry) as usize);
    if index >= rd::<u32>(handles, ARRAY_STORAGE_COUNT) {
        eng::array_set_size(handles, rd::<u32>(handles, ARRAY_GROW_BY).wrapping_add(index));
    }
    eng::array_set_at(handles, index, core::ptr::addr_of!(handle) as usize);
    ni::release_if_set(entry);
    handle
}

/// 0x6BF0B0 `NiD3DShaderInterface::buildVertexBuffer`. It makes the vertex buffer of a shape for
/// vertex declaration `declaration_index`. It gives the stride and the shader handle.
#[allow(clippy::too_many_arguments)]
pub unsafe fn build_vertex_buffer(
    this: usize,
    geometry: usize,
    skin: usize,
    partition: usize,
    renderer_data: usize,
    previous_vb: usize,
    out_buffer_size: usize,
    pool: u32,
    usage: u32,
    out_stride: usize,
    out_shader: usize,
    declaration_index: u32,
) -> usize {
    crate::shadow::touch(&build_vertex_buffer_port::STATE);
    // With a partition the game code sets only the low word of the count.
    let active = if partition != 0 { rd::<u16>(partition, SP::numVertices) as u32 } else { geometry_active_vertex_count(geometry) };
    if active as u16 == 0 {
        return 0;
    }
    let functions = this + SI::shaderFunctions;
    let usages = this + SI::shaderUsages;
    let handles = this + SI::vertexShaderHandles;
    if rd::<u8>(this, SI::flag_28) != 0 || rd::<u32>(rd::<usize>(handles, ARRAY_STORAGE), 0) == 0 {
        let mut i: u32 = 0;
        while i < rd::<u32>(functions, ARRAY_END_INDEX) {
            let usage_i: u32 = rd(rd::<usize>(usages, ARRAY_STORAGE), i as usize * 4);
            let function_i: u32 = rd(rd::<usize>(functions, ARRAY_STORAGE), i as usize * 4);
            compile_vertex_shader(this, function_i, usage_i, i);
            i += 1;
        }
        if rd::<u32>(rd::<usize>(handles, ARRAY_STORAGE), 0) == 0 {
            return 0;
        }
    }
    let declaration_offset = declaration_index.wrapping_shl(DECLARATION_SHIFT) as usize;
    let stride: u32 = rd(rd::<usize>(this, SI::declarations), declaration_offset + DECLARATION_STRIDE);
    if stride == 0 {
        return 0;
    }
    let count = active & 0xFFFF;
    let mut buffer: usize = 0;
    if previous_vb != 0 {
        // The game code does not examine the result of `GetDesc`. It reads the values on its stack in
        // all cases.
        let mut desc = [0u32; 6];
        buffer_get_desc(previous_vb, desc.as_mut_ptr() as usize);
        if desc[DESC_FORMAT] == D3DFMT_VERTEXDATA && rd::<u32>(out_buffer_size, 0) >= count.wrapping_mul(stride) {
            buffer = previous_vb;
        }
    }
    if buffer == 0 {
        let length = count.wrapping_mul(stride);
        wr(out_buffer_size, 0, length);
        if device_create_vertex_buffer(rd(this, SI::d3dDevice), length, usage, 0, pool, core::ptr::addr_of_mut!(buffer) as usize) != 0 {
            return 0;
        }
        if buffer == 0 {
            return 0;
        }
    }
    let mut data: usize = 0;
    if buffer_lock(buffer, 0, 0, core::ptr::addr_of_mut!(data) as usize, 0) != 0 {
        return 0;
    }
    let declaration = rd::<usize>(this, SI::declarations).wrapping_add(declaration_offset);
    if declaration != 0 && rd::<u32>(this, SI::field_1C) != 0 {
        let mut element_offset: usize = 0;
        let mut i: u32 = 0;
        loop {
            let element = rd::<usize>(declaration, DECLARATION_ELEMENTS) + element_offset;
            let to = rd::<usize>(element, ELEMENT_OFFSET).wrapping_add(data);
            let args = [
                active,
                geometry as u32,
                skin as u32,
                partition as u32,
                renderer_data as u32,
                element as u32,
                rd::<u32>(declaration, DECLARATION_STRIDE),
                to as u32,
            ];
            eng::pack_declaration_element(this, declaration, &args);
            i += 1;
            element_offset += ELEMENT_SIZE;
            if i >= rd::<u32>(this, SI::field_1C) {
                break;
            }
        }
    }
    if buffer_unlock(buffer) != 0 {
        return 0;
    }
    if out_stride != 0 {
        wr(out_stride, 0, stride);
    }
    let mut shader: u32 = rd(rd::<usize>(handles, ARRAY_STORAGE), 0);
    if shader == 0 {
        let entry: usize = rd(rd::<usize>(this + SI::field_90, ARRAY_STORAGE), 0);
        if entry != 0 {
            shader = rd(entry, SHADER_ENTRY_HANDLE);
            if shader != 0 {
                set_first_shader_handle(handles, shader);
            }
        }
    }
    if out_shader != 0 {
        wr(out_shader, 0, shader);
    }
    buffer
}

/// 0x6BF294: `SetAtGrow(0, handle)` of the array of shader handles. The game code has it inline.
#[inline(always)]
unsafe fn set_first_shader_handle(handles: usize, shader: u32) {
    if rd::<u32>(handles, ARRAY_STORAGE_COUNT) == 0 {
        let grow: u32 = rd(handles, ARRAY_GROW_BY);
        if grow != 0 {
            if grow < rd::<u32>(handles, ARRAY_END_INDEX) {
                let mut i = grow;
                loop {
                    let slot = rd::<usize>(handles, ARRAY_STORAGE) + i as usize * 4;
                    if rd::<u32>(slot, 0) != 0 {
                        wr::<u32>(slot, 0, 0);
                        wr(handles, ARRAY_FILLED_COUNT, rd::<u32>(handles, ARRAY_FILLED_COUNT).wrapping_sub(1));
                    }
                    i += 1;
                    if i >= rd::<u32>(handles, ARRAY_END_INDEX) {
                        break;
                    }
                }
                wr(handles, ARRAY_END_INDEX, grow);
            }
            let old: usize = rd(handles, ARRAY_STORAGE);
            wr(handles, ARRAY_STORAGE_COUNT, grow);
            let storage = eng::operator_new(grow.wrapping_mul(4) as usize);
            wr(handles, ARRAY_STORAGE, storage);
            let mut i: u32 = 0;
            while i < rd::<u32>(handles, ARRAY_END_INDEX) {
                wr::<u32>(rd::<usize>(handles, ARRAY_STORAGE), i as usize * 4, rd(old, i as usize * 4));
                i += 1;
            }
            eng::delete(old);
        }
        if rd::<u32>(handles, ARRAY_STORAGE_COUNT) == 0 {
            return;
        }
    }
    if rd::<u32>(handles, ARRAY_END_INDEX) == 0 {
        wr::<u32>(handles, ARRAY_END_INDEX, 1);
        wr(handles, ARRAY_FILLED_COUNT, rd::<u32>(handles, ARRAY_FILLED_COUNT).wrapping_add(1));
    } else if rd::<u32>(rd::<usize>(handles, ARRAY_STORAGE), 0) == 0 {
        wr(handles, ARRAY_FILLED_COUNT, rd::<u32>(handles, ARRAY_FILLED_COUNT).wrapping_add(1));
    }
    wr(rd::<usize>(handles, ARRAY_STORAGE), 0, shader);
}

// ---- the memory that a call can write

/// `createIndexBuffer` writes the size that it gives back. The buffer is a device resource. A new
/// buffer is the same object in the two runs, and its bytes are in the log. No call is a skip.
unsafe fn create_index_buffer_regions(set: &mut Regions, out_buffer_size: usize) {
    set.add(out_buffer_size, 4);
}

/// The regions are the manager and, for the line buffer, the index count. The call is a skip when
/// the buffer of the manager does not fit, because the function releases that buffer. A release
/// cannot occur two times.
unsafe fn shared_index_buffer_regions(set: &mut Regions, this: usize, buffer: usize, usage: u32, pool: u32, size: u32, out: usize) {
    if rd::<usize>(this, IM::device) != 0 && buffer != 0 && index_buffer_fits(buffer, usage, pool, size) == Some(false) {
        set.skip();
        return;
    }
    set.add(this, IM::SIZE);
    set.add(out, 4);
}

/// The call is a skip when the map has a node with a `VBInfo`, because `Recreate` releases the
/// device buffers of a `VBInfo`. With only nodes, it frees the nodes and writes the buckets.
unsafe fn recreate_regions(set: &mut Regions, this: usize, size: usize, map: usize) {
    let buckets: usize = rd(map, HM::arrayBuckets);
    let count = rd::<u32>(map, HM::bucketCount) as usize;
    for i in 0..count {
        let mut node: usize = rd(buckets, i * 4);
        while node != 0 {
            if rd::<usize>(node, HN::data) != 0 {
                set.skip();
                return;
            }
            set.allocates();
            node = rd(node, HN::nextNode);
        }
    }
    set.add(this, size);
    set.add(buckets, count * 4);
}

/// The region is the map. The function frees the bucket array and the nodes of the map. No call is
/// a skip. A deleting destructor with flag 1 frees the map, thus the map is not a region then.
unsafe fn map_dtor_regions(set: &mut Regions, map: usize, flags: u8) {
    set.allocates();
    if flags & 1 == 0 {
        set.add(map, HM::SIZE);
    }
}

/// An insert writes the node of the key, or makes a node and writes the count and the bucket. No
/// call is a skip.
unsafe fn map_insert_regions(set: &mut Regions, map: usize, key: u32) {
    let (bucket, node) = hash_map::find_in_bucket(map, key);
    set.add(map, HM::SIZE);
    if node != 0 {
        set.add(node, HN::SIZE);
    } else {
        set.allocates();
        set.add(rd::<usize>(map, HM::arrayBuckets) + bucket * 4, 4);
    }
}

/// With no node for the key, the function makes a `VBInfo` and a node. Then the regions are the
/// manager and the head of the bucket. With a node, the function writes nothing. No call is a skip.
unsafe fn get_vb_info_regions(set: &mut Regions, this: usize, usage: u32, fvf: u32, pool: u32) {
    let Some(map) = pool_map(pool) else { return };
    let key = usage.wrapping_shl(16) | fvf;
    let (bucket, node) = hash_map::find_in_bucket(this + map, key);
    if node == 0 {
        set.allocates();
        set.add(this, VM::SIZE);
        set.add(rd::<usize>(this + map, HM::arrayBuckets) + bucket * 4, 4);
    }
}

/// The regions are the `VBInfo` and the resource that the call takes. A new `VBInfo` or a new resource
/// is an allocation. The call is a skip when the buffer of the resource is too small, because the
/// function releases that buffer.
unsafe fn create_vertex_buffer_regions(set: &mut Regions, this: usize, byte_count: u32, usage: u32, fvf: u32, pool: u32) {
    if rd::<usize>(this, VM::device) == 0 {
        return;
    }
    let Some(map) = pool_map(pool) else { return };
    let key = usage.wrapping_shl(16) | fvf;
    let (bucket, node) = hash_map::find_in_bucket(this + map, key);
    if node == 0 {
        set.allocates();
        set.add(this, VM::SIZE);
        set.add(rd::<usize>(this + map, HM::arrayBuckets) + bucket * 4, 4);
        return;
    }
    let info: usize = rd(node, HN::data);
    if info == 0 {
        return;
    }
    set.add(info, VI::SIZE);
    let resource: usize = rd(info, VI::resources + rd::<u8>(info, VI::nextFreeResourceIndex) as usize * 4);
    if resource == 0 {
        set.allocates();
    } else if rd::<u32>(resource, VR::length) < byte_count {
        set.skip();
    } else {
        set.add(resource, VR::SIZE);
    }
}

/// The regions are the three results. The vertex buffer is a device resource. No call is a skip.
unsafe fn pack_untransformed_vb_regions(set: &mut Regions, out_buffer_size: usize, out_stride: usize, out_fvf: usize) {
    set.add(out_buffer_size, 4);
    set.add(out_stride, 4);
    set.add(out_fvf, 4);
}

/// The regions are the two results and the regions of `CreateVertexBuffer` for the format and the
/// length of the polygon. The call is a skip when that `CreateVertexBuffer` call is a skip.
unsafe fn pack_screen_poly_vb_regions(set: &mut Regions, this: usize, vertex_count: u32, tex_coords: usize, colors: usize, out_fvf: usize, out_stride: usize) {
    let mut fvf = D3DFVF_XYZ;
    let mut stride: u32 = 12;
    if colors != 0 {
        fvf |= D3DFVF_DIFFUSE;
        stride = 16;
    }
    if tex_coords != 0 {
        fvf |= D3DFVF_TEX1;
        stride += 8;
    }
    set.add(out_fvf, 4);
    set.add(out_stride, 4);
    create_vertex_buffer_regions(set, this, (vertex_count & 0xFFFF).wrapping_mul(stride), 0, fvf, 0);
}

/// The call is a skip when it compiles the shaders: with `flag_28` set or with no first shader
/// handle. That call makes vertex shaders, and the dry device does not make them. A call that must
/// set the first shader handle is in this condition too. If not, the regions are the three results.
unsafe fn build_vertex_buffer_regions(set: &mut Regions, this: usize, out_buffer_size: usize, out_stride: usize, out_shader: usize) {
    if rd::<u8>(this, SI::flag_28) != 0 || rd::<u32>(rd::<usize>(this + SI::vertexShaderHandles, ARRAY_STORAGE), 0) == 0 {
        set.skip();
        return;
    }
    set.add(out_buffer_size, 4);
    set.add(out_stride, 4);
    set.add(out_shader, 4);
}

// The call of a constructor is a skip. A constructor runs one time, when the renderer starts, and
// it takes a reference on the device.
port_thiscall!(index_manager_ctor_port, 0x6BC970, "NiDX8IndexBufferManager::ctor",
    fn(this, device: usize) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = index_manager_ctor);

// The call of a destructor is a skip, because a destructor releases the buffers and the device.
port_thiscall!(index_manager_deleting_dtor_port, 0x6BCB80, "NiDX8IndexBufferManager::deleting_dtor",
    fn(this, flags: u8) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = index_manager_deleting_dtor);

port_thiscall!(index_manager_dtor_port, 0x6BCBA0, "NiDX8IndexBufferManager::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = index_manager_dtor);

port_thiscall!(create_index_buffer_port, 0x6BCE80, "NiDX8IndexBufferManager::createIndexBuffer",
    fn(this, indices: usize, index_count: u32, index_buffer: usize, out_buffer_size: usize, pool: u32, usage: u32) -> usize,
    compare_eax = true,
    regions = |set| create_index_buffer_regions(set, out_buffer_size),
    rust = create_index_buffer);

port_thiscall!(get_or_build_quad_index_buffer_port, 0x6BCFE0, "NiDX8IndexBufferManager::getOrBuildQuadIndexBuffer",
    fn(this, quads: u32, fill: u8, usage: u32, pool: u32) -> usize,
    compare_eax = true,
    regions = |set| shared_index_buffer_regions(set, this, rd(this, IM::indexBuffer), usage, pool, quads.wrapping_mul(12), 0),
    rust = get_or_build_quad_index_buffer);

port_thiscall!(build_line_segment_index_buffer_port, 0x6BD190, "NiDX8IndexBufferManager::buildLineSegmentIndexBuffer",
    fn(this, vertex_count: u32, flags: usize, out_index_count: usize, usage: u32, pool: u32) -> usize,
    compare_eax = true,
    regions = |set| shared_index_buffer_regions(
        set, this, rd(this, IM::field_14), usage, pool, vertex_count.wrapping_mul(4).wrapping_add(4), out_index_count),
    rust = build_line_segment_index_buffer);

port_thiscall!(index_manager_recreate_port, 0x6BD370, "NiDX8IndexBufferManager::Recreate",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| recreate_regions(set, this, IM::SIZE, this + IM::hashmap_1C),
    rust = index_manager_recreate);

port_thiscall!(index_map_dtor_port, 0x6BD560, "NiDX8IndexBufferManager::HashMap::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| map_dtor_regions(set, this, 0),
    rust = index_map_dtor);

port_thiscall!(vertex_manager_ctor_port, 0x6BD780, "NiDX8VertexBufferManager::ctor",
    fn(this, device: usize) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = vertex_manager_ctor);

port_thiscall!(vertex_manager_deleting_dtor_port, 0x6BD990, "NiDX8VertexBufferManager::deleting_dtor",
    fn(this, flags: u8) -> usize,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = vertex_manager_deleting_dtor);

port_thiscall!(vertex_manager_dtor_port, 0x6BD9B0, "NiDX8VertexBufferManager::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| set.skip(),
    rust = vertex_manager_dtor);

port_thiscall!(create_vertex_buffer_port, 0x6BDC70, "NiDX8VertexBufferManager::CreateVertexBuffer",
    fn(this, byte_count: u32, usage: u32, fvf: u32, pool: u32) -> usize,
    compare_eax = true,
    regions = |set| create_vertex_buffer_regions(set, this, byte_count, usage, fvf, pool),
    rust = create_vertex_buffer);

port_thiscall!(pack_untransformed_vb_port, 0x6BDE50, "NiDX8VertexBufferManager::PackUntransformedVB",
    fn(this, geometry: usize, previous_vb: usize, out_buffer_size: usize, skin: usize, refill: u8, pool: u32, usage: u32,
       out_stride: usize, out_fvf: usize) -> usize,
    compare_eax = true,
    regions = |set| pack_untransformed_vb_regions(set, out_buffer_size, out_stride, out_fvf),
    rust = pack_untransformed_vb);

port_thiscall!(pack_screen_poly_vb_port, 0x6BE180, "NiDX8VertexBufferManager::PackScreenPolyVB",
    fn(this, vertex_count: u32, vertices: usize, tex_coords: usize, colors: usize, frustum_near: u32, frustum_distance: u32,
       out_fvf: usize, out_stride: usize) -> usize,
    compare_eax = true,
    regions = |set| pack_screen_poly_vb_regions(set, this, vertex_count, tex_coords, colors, out_fvf, out_stride),
    rust = pack_screen_poly_vb);

port_thiscall!(get_vb_info_port, 0x6BE690, "NiDX8VertexBufferManager::GetVBInfo",
    fn(this, usage: u32, fvf: u32, pool: u32) -> usize,
    compare_eax = true,
    regions = |set| get_vb_info_regions(set, this, usage, fvf, pool),
    rust = get_vb_info);

port_thiscall!(vertex_manager_recreate_port, 0x6BE910, "NiDX8VertexBufferManager::Recreate",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| recreate_regions(set, this, VM::SIZE, this + VM::hashmap_C),
    rust = vertex_manager_recreate);

port_thiscall!(vertex_map_dtor_port, 0x6BEA30, "NiDX8VertexBufferManager::HashMap::dtor",
    fn(this) -> (),
    compare_eax = false,
    regions = |set| map_dtor_regions(set, this, 0),
    rust = vertex_map_dtor);

port_thiscall!(fvf_usage_hash_insert_port, 0x6BEA90, "NiDX8VertexBufferManager::FVFUsageHash::insert",
    fn(this, key: u32, value: usize) -> (),
    compare_eax = false,
    regions = |set| map_insert_regions(set, this, key),
    rust = fvf_usage_hash_insert);

port_thiscall!(vertex_map_deleting_dtor_port, 0x6BEB20, "NiDX8VertexBufferManager::Hash::deleting_dtor",
    fn(this, flags: u8) -> usize,
    compare_eax = true,
    regions = |set| map_dtor_regions(set, this, flags),
    rust = vertex_map_deleting_dtor);

port_thiscall!(vb_resource_ctor_port, 0x6C28A0, "NiDX8VBResource::ctor",
    fn(this) -> usize,
    compare_eax = true,
    regions = |set| set.add(this, VR::length),
    rust = vb_resource_ctor);

// The call is a skip. `CreateVertexShader` returns nothing on the dry device, so the two runs cannot
// be equal.
port_thiscall!(compile_vertex_shader_port, 0x6BEE00, "NiD3DShaderInterface::compileVertexShader",
    fn(this, shader_function: u32, shader_usage: u32, index: u32) -> u32,
    compare_eax = true,
    regions = |set| set.skip(),
    rust = compile_vertex_shader);

port_thiscall!(build_vertex_buffer_port, 0x6BF0B0, "NiD3DShaderInterface::buildVertexBuffer",
    fn(this, geometry: usize, skin: usize, partition: usize, renderer_data: usize, previous_vb: usize, out_buffer_size: usize,
       pool: u32, usage: u32, out_stride: usize, out_shader: usize, declaration_index: u32) -> usize,
    compare_eax = true,
    regions = |set| build_vertex_buffer_regions(set, this, out_buffer_size, out_stride, out_shader),
    rust = build_vertex_buffer);

pub static PORTS: &[&crate::shadow::PortState] = &[
    &index_manager_ctor_port::STATE,
    &index_manager_deleting_dtor_port::STATE,
    &index_manager_dtor_port::STATE,
    &create_index_buffer_port::STATE,
    &get_or_build_quad_index_buffer_port::STATE,
    &build_line_segment_index_buffer_port::STATE,
    &index_manager_recreate_port::STATE,
    &index_map_dtor_port::STATE,
    &vertex_manager_ctor_port::STATE,
    &vertex_manager_deleting_dtor_port::STATE,
    &vertex_manager_dtor_port::STATE,
    &create_vertex_buffer_port::STATE,
    &pack_untransformed_vb_port::STATE,
    &pack_screen_poly_vb_port::STATE,
    &get_vb_info_port::STATE,
    &vertex_manager_recreate_port::STATE,
    &vertex_map_dtor_port::STATE,
    &fvf_usage_hash_insert_port::STATE,
    &vertex_map_deleting_dtor_port::STATE,
    &vb_resource_ctor_port::STATE,
    &compile_vertex_shader_port::STATE,
    &build_vertex_buffer_port::STATE,
];

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::d3d::{
        BUFFER_GET_DESC, BUFFER_LOCK, BUFFER_UNLOCK, COM_RELEASE, DEVICE_ADD_REF, DEVICE_CREATE_INDEX_BUFFER,
        DEVICE_CREATE_VERTEX_BUFFER,
    };

    thread_local! {
        /// The calls that the fake device and the fake buffers got: (name, first value, second value).
        static CALLS: RefCell<Vec<(&'static str, u32, u32)>> = const { RefCell::new(Vec::new()) };
        /// The result of the next [`Lock`] calls.
        static LOCK_RESULT: RefCell<u32> = const { RefCell::new(0) };
    }

    fn calls() -> Vec<(&'static str, u32, u32)> {
        CALLS.with(|c| core::mem::take(&mut *c.borrow_mut()))
    }
    fn call(name: &'static str, a: u32, b: u32) {
        CALLS.with(|c| c.borrow_mut().push((name, a, b)));
    }
    fn names() -> Vec<&'static str> {
        calls().into_iter().map(|c| c.0).collect()
    }

    fn leak(size: usize) -> usize {
        Box::leak(vec![0u8; size].into_boxed_slice()).as_mut_ptr() as usize
    }

    // A fake buffer has a vtable, data, then the six dwords of its description.
    const FAKE_DATA: usize = 4;
    const FAKE_DESC: usize = 8;

    extern "system" fn buf_release(_this: usize) -> u32 {
        call("Release", 0, 0);
        0
    }
    extern "system" fn buf_lock(this: usize, offset: u32, size: u32, out: *mut usize, _flags: u32) -> u32 {
        call("Lock", offset, size);
        let result = LOCK_RESULT.with(|r| *r.borrow());
        if result == 0 {
            unsafe { *out = rd(this, FAKE_DATA) };
        }
        result
    }
    extern "system" fn buf_unlock(_this: usize) -> u32 {
        call("Unlock", 0, 0);
        0
    }
    extern "system" fn buf_get_desc(this: usize, desc: usize) -> u32 {
        unsafe { core::ptr::copy_nonoverlapping((this + FAKE_DESC) as *const u32, desc as *mut u32, 6) };
        0
    }

    unsafe fn fake_buffer(format: u32, kind: u32, usage: u32, pool: u32, size: u32, fvf: u32) -> usize {
        let vtable = leak(0x40);
        wr(vtable, COM_RELEASE, buf_release as *const () as usize);
        wr(vtable, BUFFER_LOCK, buf_lock as *const () as usize);
        wr(vtable, BUFFER_UNLOCK, buf_unlock as *const () as usize);
        wr(vtable, BUFFER_GET_DESC, buf_get_desc as *const () as usize);
        let buffer = leak(FAKE_DESC + 24);
        wr(buffer, 0, vtable);
        wr(buffer, FAKE_DATA, leak(size as usize + 16));
        wr(buffer, FAKE_DESC, [format, kind, usage, pool, size, fvf]);
        buffer
    }
    unsafe fn data_of(buffer: usize) -> usize {
        rd(buffer, FAKE_DATA)
    }

    extern "system" fn dev_add_ref(_device: usize) -> u32 {
        call("AddRef", 0, 0);
        0
    }
    extern "system" fn dev_create_vb(_device: usize, length: u32, usage: u32, fvf: u32, pool: u32, out: *mut usize) -> u32 {
        call("CreateVertexBuffer", length, fvf);
        unsafe { *out = fake_buffer(D3DFMT_VERTEXDATA, 3, usage, pool, length, fvf) };
        0
    }
    extern "system" fn dev_create_ib(_device: usize, length: u32, usage: u32, format: u32, pool: u32, out: *mut usize) -> u32 {
        call("CreateIndexBuffer", length, format);
        unsafe { *out = fake_buffer(format, D3DRTYPE_INDEXBUFFER, usage, pool, length, 0) };
        0
    }

    unsafe fn fake_device() -> usize {
        let vtable = leak(97 * 4);
        wr(vtable, DEVICE_ADD_REF, dev_add_ref as *const () as usize);
        wr(vtable, DEVICE_CREATE_VERTEX_BUFFER, dev_create_vb as *const () as usize);
        wr(vtable, DEVICE_CREATE_INDEX_BUFFER, dev_create_ib as *const () as usize);
        let device = leak(4);
        wr(device, 0, vtable);
        device
    }

    unsafe fn index_manager() -> usize {
        let manager = leak(IM::SIZE);
        index_manager_ctor(manager, fake_device());
        calls();
        manager
    }

    unsafe fn vertex_manager() -> usize {
        let manager = leak(VM::SIZE);
        vertex_manager_ctor(manager, fake_device());
        calls();
        manager
    }

    #[test]
    fn constructors_make_three_empty_maps_and_take_the_device() {
        unsafe {
            let device = fake_device();
            let manager = leak(IM::SIZE);
            core::ptr::write_bytes(manager as *mut u8, 0xEE, IM::SIZE);
            assert_eq!(index_manager_ctor(manager, device), manager);
            assert_eq!(names(), ["AddRef"]);
            assert_eq!(rd::<u32>(manager, IM::vtbl), VTBL_INDEX_BUFFER_MANAGER);
            assert_eq!(rd::<u32>(manager, IM::refCount), 0);
            assert_eq!(rd::<usize>(manager, IM::device), device);
            assert_eq!(rd::<[u32; 4]>(manager, IM::indexBuffer), [0; 4]);
            for map in [IM::hashmap_1C, IM::hashmap_2C, IM::hashmap_3C] {
                assert_eq!(rd::<u32>(manager + map, HM::bucketCount), 37);
                assert_eq!(rd::<u32>(manager + map, HM::count), 0);
                assert_eq!(rd::<[u32; 37]>(rd::<usize>(manager + map, HM::arrayBuckets), 0), [0; 37]);
            }

            let manager = leak(VM::SIZE);
            assert_eq!(vertex_manager_ctor(manager, device), manager);
            assert_eq!(names(), ["AddRef"]);
            assert_eq!(rd::<u32>(manager, VM::vtbl), VTBL_VERTEX_BUFFER_MANAGER);
            for map in [VM::hashmap_C, VM::hashmap_1C, VM::hashmapFVFUsage] {
                assert_eq!(rd::<u32>(manager + map, HM::bucketCount), 37);
                assert_eq!(rd::<[u32; 37]>(rd::<usize>(manager + map, HM::arrayBuckets), 0), [0; 37]);
            }
        }
    }

    #[test]
    fn create_index_buffer_copies_the_indices() {
        unsafe {
            let manager = index_manager();
            let indices: [u16; 5] = [7, 8, 9, 10, 11];
            let mut size = 0u32;
            let out = core::ptr::addr_of_mut!(size) as usize;
            assert_eq!(create_index_buffer(manager, 0, 5, 0, out, 1, 8), 0);
            let buffer = create_index_buffer(manager, indices.as_ptr() as usize, 5, 0, out, 1, 8);
            assert_eq!(calls(), [("CreateIndexBuffer", 10, 101), ("Lock", 0, 10), ("Unlock", 0, 0)]);
            assert_eq!(size, 10);
            assert_eq!(rd::<[u16; 5]>(data_of(buffer), 0), indices);
            assert_eq!(rd::<u16>(data_of(buffer), 10), 0);

            // The port uses the same buffer again when it fits. Then it does not write the size.
            size = 0;
            assert_eq!(create_index_buffer(manager, indices.as_ptr() as usize, 4, buffer, out, 1, 8), buffer);
            assert_eq!(calls(), [("Lock", 0, 8), ("Unlock", 0, 0)]);
            assert_eq!(size, 0);
            // Another pool gives a new buffer. The caller must release the old buffer.
            let other = create_index_buffer(manager, indices.as_ptr() as usize, 4, buffer, out, 0, 8);
            assert_ne!(other, buffer);
            assert_eq!(names(), ["CreateIndexBuffer", "Lock", "Unlock"]);

            // After a failed lock the port unlocks and releases the buffer. The result is still the pointer.
            LOCK_RESULT.with(|r| *r.borrow_mut() = 0x8876_086C);
            assert_eq!(create_index_buffer(manager, indices.as_ptr() as usize, 4, buffer, out, 1, 8), buffer);
            LOCK_RESULT.with(|r| *r.borrow_mut() = 0);
            assert_eq!(names(), ["Lock", "Unlock", "Release"]);
        }
    }

    #[test]
    fn quad_index_buffer_has_two_triangles_per_quad() {
        unsafe {
            let manager = index_manager();
            assert_eq!(get_or_build_quad_index_buffer(manager, 0, 1, 8, 0), 0);
            let buffer = get_or_build_quad_index_buffer(manager, 2, 0, 8, 0);
            assert_eq!(calls(), [("CreateIndexBuffer", 24, 101), ("Lock", 0, 24), ("Unlock", 0, 0)]);
            assert_eq!(rd::<[u16; 12]>(data_of(buffer), 0), [0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]);
            assert_eq!(rd::<usize>(manager, IM::indexBuffer), buffer);
            assert_eq!(rd::<u32>(manager, IM::indexBufferSize), 24);

            // The buffer fits and the call does not ask for a fill. The port does nothing, and the size
            // stays.
            assert_eq!(get_or_build_quad_index_buffer(manager, 1, 0, 8, 0), buffer);
            assert!(calls().is_empty());
            assert_eq!(rd::<u32>(manager, IM::indexBufferSize), 24);
            // With a fill the size becomes the one of this call.
            assert_eq!(get_or_build_quad_index_buffer(manager, 1, 1, 8, 0), buffer);
            assert_eq!(calls(), [("Lock", 0, 12), ("Unlock", 0, 0)]);
            assert_eq!(rd::<u32>(manager, IM::indexBufferSize), 12);
            // The buffer is too small. The port releases it and makes it again.
            let larger = get_or_build_quad_index_buffer(manager, 3, 0, 8, 0);
            assert_ne!(larger, buffer);
            assert_eq!(names(), ["Release", "CreateIndexBuffer", "Lock", "Unlock"]);
            assert_eq!(rd::<usize>(manager, IM::indexBuffer), larger);
        }
    }

    #[test]
    fn line_index_buffer_connects_flagged_vertices() {
        unsafe {
            let manager = index_manager();
            let flags: [u8; 4] = [1, 0, 1, 1];
            let mut count = 99u32;
            let out = core::ptr::addr_of_mut!(count) as usize;
            let buffer = build_line_segment_index_buffer(manager, 4, flags.as_ptr() as usize, out, 8, 0);
            assert_eq!(calls(), [("CreateIndexBuffer", 20, 101), ("Lock", 0, 20), ("Unlock", 0, 0)]);
            assert_eq!(count, 6);
            // The lines are 0 to 1, 2 to 3, and the last vertex to the first.
            assert_eq!(rd::<[u16; 6]>(data_of(buffer), 0), [0, 1, 2, 3, 3, 0]);
            assert_eq!(rd::<usize>(manager, IM::field_14), buffer);
            assert_eq!(rd::<u32>(manager, IM::field_18), 20);

            let open: [u8; 4] = [0, 1, 0, 0];
            assert_eq!(build_line_segment_index_buffer(manager, 4, open.as_ptr() as usize, out, 8, 0), buffer);
            assert_eq!(names(), ["Lock", "Unlock"]);
            assert_eq!(count, 2);
            assert_eq!(rd::<[u16; 2]>(data_of(buffer), 0), [1, 2]);

            // One vertex gives a line to itself.
            let one: [u8; 1] = [1];
            build_line_segment_index_buffer(manager, 1, one.as_ptr() as usize, out, 8, 0);
            assert_eq!(count, 2);
            assert_eq!(rd::<[u16; 2]>(data_of(buffer), 0), [0, 0]);
        }
    }

    #[test]
    fn vb_resource_ctor_sets_four_members() {
        let mut block = [0x55u8; VR::SIZE];
        let resource = block.as_mut_ptr() as usize;
        unsafe {
            assert_eq!(vb_resource_ctor(resource), resource);
            assert_eq!(rd::<[u32; 5]>(resource, 0), [0xFFFF_FFFF, 0xFFFF, 0, 0x5555_5555, 0]);
            assert_eq!(rd::<u32>(resource, VR::length), 0x5555_5555);
        }
    }

    #[test]
    fn get_vb_info_makes_one_info_per_key_and_pool() {
        unsafe {
            let manager = vertex_manager();
            assert_eq!(get_vb_info(manager, 8, 0x112, 3), 0);
            for (pool, map) in [(0, VM::hashmap_C), (1, VM::hashmap_1C), (2, VM::hashmapFVFUsage)] {
                let info = get_vb_info(manager, 8, 0x112, pool);
                assert_eq!(rd::<u32>(info, VI::vb_id), 0x0008_0112);
                assert_eq!(rd::<u8>(info, VI::nextFreeResourceIndex), 0);
                assert_eq!(rd::<[u32; 5]>(info, VI::resources), [0; 5]);
                assert_eq!(rd::<u32>(manager + map, HM::count), 1);
                assert_eq!(get_vb_info(manager, 8, 0x112, pool), info);
                assert_eq!(rd::<u32>(manager + map, HM::count), 1);
                // A key of the same bucket is 37 apart. Its node goes in front of the first node.
                let second = get_vb_info(manager, 8, 0x112 + 37, pool);
                assert_ne!(second, info);
                assert_eq!(rd::<u32>(manager + map, HM::count), 2);
                let bucket = 0x0008_0112 % 37;
                let head: usize = rd(rd::<usize>(manager + map, HM::arrayBuckets), bucket * 4);
                assert_eq!(rd::<usize>(head, HN::data), second);
                assert_eq!(rd::<usize>(rd::<usize>(head, HN::nextNode), HN::data), info);
                assert_eq!(get_vb_info(manager, 8, 0x112, pool), info);
            }
        }
    }

    #[test]
    fn insert_replaces_the_value_of_a_key() {
        unsafe {
            let manager = vertex_manager();
            let map = manager + VM::hashmapFVFUsage;
            fvf_usage_hash_insert(map, 5, 0x1000);
            fvf_usage_hash_insert(map, 5, 0x2000);
            assert_eq!(rd::<u32>(map, HM::count), 1);
            let (bucket, node) = hash_map::find_in_bucket(map, 5);
            assert_eq!(bucket, 5);
            assert_eq!(rd::<usize>(node, HN::data), 0x2000);
            assert_eq!(rd::<usize>(node, HN::nextNode), 0);
        }
    }

    #[test]
    fn create_vertex_buffer_goes_round_five_buffers() {
        unsafe {
            let manager = vertex_manager();
            let mut made = Vec::new();
            for i in 0..5u8 {
                let buffer = create_vertex_buffer(manager, 100, 8, 0x42, 0);
                assert_eq!(calls(), [("CreateVertexBuffer", 100, 0x42)]);
                let info = get_vb_info(manager, 8, 0x42, 0);
                assert_eq!(rd::<u8>(info, VI::nextFreeResourceIndex), (i + 1) % 5);
                let resource: usize = rd(info, VI::resources + i as usize * 4);
                assert_eq!(rd::<[u32; 5]>(resource, 0), [0xFFFF_FFFF, 3, 0, 8, 0]);
                assert_eq!(rd::<[u32; 3]>(resource, VR::length), [100, 0x42, 0]);
                assert_eq!(rd::<usize>(resource, VR::pVB), buffer);
                assert_eq!(rd::<[u32; 6]>(resource, VR::desc), [D3DFMT_VERTEXDATA, 3, 8, 0, 100, 0x42]);
                made.push(buffer);
            }
            // The sixth call gets the first buffer again when it is large enough.
            assert_eq!(create_vertex_buffer(manager, 60, 8, 0x42, 0), made[0]);
            assert!(calls().is_empty());
            // The second buffer is too small. The port releases it and makes it again with the new length.
            let larger = create_vertex_buffer(manager, 200, 8, 0x42, 0);
            assert_ne!(larger, made[1]);
            assert_eq!(calls(), [("Release", 0, 0), ("CreateVertexBuffer", 200, 0x42)]);
            let info = get_vb_info(manager, 8, 0x42, 0);
            assert_eq!(rd::<u8>(info, VI::nextFreeResourceIndex), 2);
            let resource: usize = rd(info, VI::resources + 4);
            assert_eq!(rd::<u32>(resource, VR::length), 200);
            assert_eq!(rd::<usize>(resource, VR::pVB), larger);
            // The manager has no map for this pool.
            assert_eq!(create_vertex_buffer(manager, 60, 8, 0x42, 7), 0);
        }
    }

    extern "thiscall" fn active_count(geometry: usize) -> u32 {
        // Only the low word is the count.
        0xABCD_0000 | unsafe { rd::<u16>(geometry, GD::vertexCount) } as u32
    }

    unsafe fn geometry(count: u16, normals: bool, colors: bool, sets: u16) -> usize {
        let vtable = leak(0x40);
        wr(vtable, GEOMETRY_VTBL_ACTIVE_VERTEX_COUNT, active_count as *const () as usize);
        let data = leak(GD::SIZE);
        wr(data, 0, vtable);
        wr(data, GD::vertexCount, count);
        wr(data, GD::textureSets, sets);
        let n = count as usize;
        let vertices = leak(n * 12);
        for i in 0..n * 3 {
            wr::<u32>(vertices, i * 4, 0x1000 + i as u32);
        }
        wr(data, GD::vertices, vertices);
        if normals {
            let block = leak(n * 12);
            for i in 0..n * 3 {
                wr::<u32>(block, i * 4, 0x2000 + i as u32);
            }
            wr(data, GD::normals, block);
        }
        if colors {
            let block = leak(n * 4);
            for i in 0..n {
                wr::<u32>(block, i * 4, 0x3000 + i as u32);
            }
            wr(data, GD::colors, block);
        }
        let uvs = leak(n * 8 * sets.max(1) as usize);
        for i in 0..n * 2 * sets as usize {
            wr::<u32>(uvs, i * 4, 0x4000 + i as u32);
        }
        wr(data, GD::textureCoords, uvs);
        data
    }

    #[test]
    fn pack_untransformed_puts_the_arrays_side_by_side() {
        unsafe {
            let manager = vertex_manager();
            let shape = geometry(2, true, true, 2);
            let (mut size, mut stride, mut fvf) = (0u32, 0u32, 0u32);
            let outs = (core::ptr::addr_of_mut!(size) as usize, core::ptr::addr_of_mut!(stride) as usize, core::ptr::addr_of_mut!(fvf) as usize);
            let buffer = pack_untransformed_vb(manager, shape, 0, outs.0, 0, 0, 1, 8, outs.1, outs.2);
            assert_eq!(calls(), [("CreateVertexBuffer", 88, 0x252), ("Lock", 0, 0), ("Unlock", 0, 0)]);
            assert_eq!((size, stride, fvf), (88, 44, 0x252));
            let expected: [u32; 22] = [
                0x1000, 0x1001, 0x1002, 0x2000, 0x2001, 0x2002, 0x3000, 0x4000, 0x4001, 0x4004, 0x4005, //
                0x1003, 0x1004, 0x1005, 0x2003, 0x2004, 0x2005, 0x3001, 0x4002, 0x4003, 0x4006, 0x4007,
            ];
            assert_eq!(rd::<[u32; 22]>(data_of(buffer), 0), expected);

            // The buffer fits the next time. The port makes no new buffer and does not write the size.
            size = 0;
            assert_eq!(pack_untransformed_vb(manager, shape, buffer, outs.0, 0, 0, 1, 8, outs.1, outs.2), buffer);
            assert_eq!(names(), ["Lock", "Unlock"]);
            assert_eq!(size, 0);
            // Another usage gives a new buffer.
            assert_ne!(pack_untransformed_vb(manager, shape, buffer, outs.0, 0, 0, 1, 0, outs.1, outs.2), buffer);
            assert_eq!(names(), ["CreateVertexBuffer", "Lock", "Unlock"]);

            // With positions only, one empty texture set is still part of the format.
            let plain = geometry(3, false, false, 0);
            let buffer = pack_untransformed_vb(manager, plain, 0, outs.0, 0, 0, 1, 8, 0, 0);
            assert_eq!(calls(), [("CreateVertexBuffer", 60, 0x102), ("Lock", 0, 0), ("Unlock", 0, 0)]);
            assert_eq!(rd::<[u32; 5]>(data_of(buffer), 20), [0x1003, 0x1004, 0x1005, 0, 0]);
            // With no vertices the port does nothing.
            assert_eq!(pack_untransformed_vb(manager, geometry(0, false, false, 0), 0, outs.0, 0, 0, 1, 8, 0, 0), 0);
            assert!(calls().is_empty());
        }
    }

    #[test]
    fn pack_untransformed_with_a_skin_leaves_positions_to_deform() {
        unsafe {
            let manager = vertex_manager();
            let shape = geometry(2, true, true, 1);
            let (mut size, mut stride, mut fvf) = (0u32, 0u32, 0u32);
            let outs = (core::ptr::addr_of_mut!(size) as usize, core::ptr::addr_of_mut!(stride) as usize, core::ptr::addr_of_mut!(fvf) as usize);
            eng::DEFORMED.with(|d| d.borrow_mut().clear());
            let buffer = pack_untransformed_vb(manager, shape, 0, outs.0, 0x5151, 0, 1, 8, outs.1, outs.2);
            let data = data_of(buffer);
            let deformed = eng::DEFORMED.with(|d| core::mem::take(&mut *d.borrow_mut()));
            assert_eq!(deformed, [(rd(shape, GD::vertices), rd(shape, GD::normals), 2, data, data + 12, 36)]);
            // The port fills a new buffer fully. The colours and the texture coordinates are there.
            assert_eq!(rd::<[u32; 3]>(data, 24), [0x3000, 0x4000, 0x4001]);
            assert_eq!(rd::<u32>(data, 0), 0);
            assert_eq!((stride, fvf), (36, 0x152));

            // With the same buffer and no refill, the port does only the deform. It writes no stride and
            // no format.
            wr::<[u32; 3]>(data, 24, [0; 3]);
            (stride, fvf) = (0, 0);
            calls();
            assert_eq!(pack_untransformed_vb(manager, shape, buffer, outs.0, 0x5151, 0, 1, 8, outs.1, outs.2), buffer);
            assert_eq!(names(), ["Lock", "Unlock"]);
            assert_eq!(rd::<[u32; 3]>(data, 24), [0; 3]);
            assert_eq!((stride, fvf), (0, 0));
            // With a refill the port writes the remaining data again.
            assert_eq!(pack_untransformed_vb(manager, shape, buffer, outs.0, 0x5151, 1, 1, 8, outs.1, outs.2), buffer);
            assert_eq!(rd::<[u32; 3]>(data, 24), [0x3000, 0x4000, 0x4001]);
            assert_eq!((stride, fvf), (36, 0x152));
        }
    }

    #[test]
    fn screen_vertex_scales_by_depth() {
        // w = 2 * 3 + 1 = 7; x = 7 * 0.5; y = 7 * -2
        let bits = |v: f32| v.to_bits();
        let out = x87_screen_vertex(&[bits(2.0), bits(3.0), bits(1.0), bits(0.5), bits(-2.0)]);
        assert_eq!(out.map(f32::from_bits), [3.5, -14.0, 7.0]);
    }

    #[test]
    fn pack_screen_poly_writes_position_colour_and_uv() {
        unsafe {
            let manager = vertex_manager();
            let vertices: [f32; 6] = [0.5, -2.0, 3.0, 1.0, 1.0, 0.0];
            let colors: [u32; 2] = [0xFF11_2233, 0x8044_5566];
            let uvs: [u32; 4] = [0xA0, 0xA1, 0xA2, 0xA3];
            let (mut fvf, mut stride) = (0u32, 0u32);
            let (out_fvf, out_stride) = (core::ptr::addr_of_mut!(fvf) as usize, core::ptr::addr_of_mut!(stride) as usize);
            // The port uses only the low word of the count.
            let buffer = pack_screen_poly_vb(
                manager,
                0x7_0002,
                vertices.as_ptr() as usize,
                uvs.as_ptr() as usize,
                colors.as_ptr() as usize,
                1.0f32.to_bits(),
                2.0f32.to_bits(),
                out_fvf,
                out_stride,
            );
            assert_eq!(calls(), [("CreateVertexBuffer", 48, 0x142), ("Lock", 0, 0), ("Unlock", 0, 0)]);
            assert_eq!((fvf, stride), (0x142, 24));
            let data = data_of(buffer);
            assert_eq!(rd::<[f32; 3]>(data, 0), [3.5, -14.0, 7.0]);
            assert_eq!(rd::<[u32; 3]>(data, 12), [0xFF11_2233, 0xA0, 0xA1]);
            assert_eq!(rd::<[f32; 3]>(data, 24), [1.0, 1.0, 1.0]);
            assert_eq!(rd::<[u32; 3]>(data, 36), [0x8044_5566, 0xA2, 0xA3]);

            // Positions only.
            pack_screen_poly_vb(manager, 2, vertices.as_ptr() as usize, 0, 0, 0, 0, out_fvf, out_stride);
            assert_eq!(calls(), [("CreateVertexBuffer", 24, 2), ("Lock", 0, 0), ("Unlock", 0, 0)]);
            assert_eq!((fvf, stride), (2, 12));
        }
    }

    unsafe fn map_with_values(manager: usize, map: usize) -> (usize, usize) {
        let info = get_vb_info(manager, 8, 0x42, 0);
        let buffer = create_vertex_buffer(manager, 16, 8, 0x42, 0);
        get_vb_info(manager, 8, 0x42 + 37, 0);
        assert_eq!(rd::<u32>(manager + map, HM::count), 2);
        calls();
        eng::DELETED.with(|d| d.borrow_mut().clear());
        (info, buffer)
    }

    #[test]
    fn recreate_releases_the_buffers_of_the_default_pool() {
        unsafe {
            let manager = vertex_manager();
            let (info, _) = map_with_values(manager, VM::hashmap_C);
            let other = get_vb_info(manager, 8, 0x42, 1);
            let resource: usize = rd(info, VI::resources);
            vertex_manager_recreate(manager);
            assert_eq!(names(), ["Release"]);
            let deleted = eng::DELETED.with(|d| core::mem::take(&mut *d.borrow_mut()));
            // Two infos with a resource in one of them, then the two nodes.
            assert_eq!(deleted.len(), 5);
            assert!(deleted.contains(&info) && deleted.contains(&resource));
            assert_eq!(rd::<u32>(manager, VM::hashmap_C_count), 0);
            assert_eq!(rd::<[u32; 37]>(rd::<usize>(manager, VM::hashmap_C_arrayBuckets), 0), [0; 37]);
            // The managed pool stays.
            assert_eq!(rd::<u32>(manager, VM::hashmap_1C_count), 1);
            assert_eq!(get_vb_info(manager, 8, 0x42, 1), other);
        }
    }

    #[test]
    fn map_destructors_free_nodes_and_buckets() {
        unsafe {
            let manager = vertex_manager();
            map_with_values(manager, VM::hashmap_C);
            let map = manager + VM::hashmap_C;
            let buckets: usize = rd(map, HM::arrayBuckets);
            assert_eq!(vertex_map_deleting_dtor(map, 1), map);
            let deleted = eng::DELETED.with(|d| core::mem::take(&mut *d.borrow_mut()));
            assert_eq!(deleted.len(), 4);
            assert_eq!(deleted[2..], [buckets, map]);
            assert_eq!(rd::<u32>(map, HM::count), 0);

            let map = manager + VM::hashmap_1C;
            get_vb_info(manager, 8, 0x42, 1);
            vertex_map_dtor(map);
            index_map_dtor(map);
            let deleted = eng::DELETED.with(|d| core::mem::take(&mut *d.borrow_mut()));
            let buckets: usize = rd(map, HM::arrayBuckets);
            assert_eq!(deleted.len(), 3);
            assert_eq!(deleted[1..], [buckets, buckets]);
        }
    }

    extern "thiscall" fn no_declaration(_this: usize) -> usize {
        0
    }

    /// A shader interface with one declaration of two elements and a stride of 20.
    unsafe fn shader_interface(handle: u32) -> usize {
        let vtable = leak(0x40);
        wr(vtable, SHADER_VTBL_DECLARATION, no_declaration as *const () as usize);
        let this = leak(SI::SIZE);
        wr(this, 0, vtable);
        wr(this, SI::d3dDevice, fake_device());
        wr::<u32>(this, SI::field_1C, 2);
        let elements = leak(2 * ELEMENT_SIZE);
        wr::<u32>(elements, ELEMENT_OFFSET, 0);
        wr::<u32>(elements, ELEMENT_SIZE + ELEMENT_OFFSET, 12);
        let declarations = leak(2 * DECL::SIZE);
        wr(declarations, DECL::SIZE + DECLARATION_ELEMENTS, elements);
        wr::<u32>(declarations, DECL::SIZE + DECLARATION_STRIDE, 20);
        wr(this, SI::declarations, declarations);
        for array in [SI::shaderFunctions, SI::shaderUsages, SI::field_90, SI::vertexShaderHandles] {
            wr(this + array, ARRAY_STORAGE, leak(8));
            wr::<u32>(this + array, ARRAY_STORAGE_COUNT, 2);
            wr::<u32>(this + array, ARRAY_GROW_BY, 1);
        }
        wr(rd::<usize>(this + SI::vertexShaderHandles, ARRAY_STORAGE), 0, handle);
        wr::<u32>(this + SI::vertexShaderHandles, ARRAY_END_INDEX, 1);
        this
    }

    #[test]
    fn build_vertex_buffer_packs_each_element_of_the_declaration() {
        unsafe {
            let this = shader_interface(0x77);
            let shape = geometry(3, false, false, 0);
            let (mut size, mut stride, mut shader) = (0u32, 0u32, 0u32);
            let outs = (core::ptr::addr_of_mut!(size) as usize, core::ptr::addr_of_mut!(stride) as usize, core::ptr::addr_of_mut!(shader) as usize);
            eng::PACKED.with(|p| p.borrow_mut().clear());
            let buffer = build_vertex_buffer(this, shape, 0x22, 0, 0x44, 0, outs.0, 1, 8, outs.1, outs.2, 1);
            assert_eq!(calls(), [("CreateVertexBuffer", 60, 0), ("Lock", 0, 0), ("Unlock", 0, 0)]);
            assert_eq!((size, stride, shader), (60, 20, 0x77));
            let declaration = rd::<usize>(this, SI::declarations) + DECL::SIZE;
            let elements: usize = rd(declaration, DECLARATION_ELEMENTS);
            let data = data_of(buffer) as u32;
            let packed = eng::PACKED.with(|p| core::mem::take(&mut *p.borrow_mut()));
            let common = |element: usize, to: u32| (this, declaration, [0xABCD_0003, shape as u32, 0x22, 0, 0x44, element as u32, 20, to]);
            assert_eq!(packed, [common(elements, data), common(elements + ELEMENT_SIZE, data + 12)]);

            // The port uses the buffer again while the size that the caller keeps is sufficient.
            assert_eq!(build_vertex_buffer(this, shape, 0, 0, 0, buffer, outs.0, 1, 8, 0, 0, 1), buffer);
            assert_eq!(names(), ["Lock", "Unlock"]);
            size = 59;
            assert_ne!(build_vertex_buffer(this, shape, 0, 0, 0, buffer, outs.0, 1, 8, 0, 0, 1), buffer);
            assert_eq!(names(), ["CreateVertexBuffer", "Lock", "Unlock"]);
            assert_eq!(size, 60);

            // The count of a partition comes first. Declaration 0 has no stride, so the port does nothing.
            let partition = leak(SP::SIZE);
            assert_eq!(build_vertex_buffer(this, shape, 0, partition, 0, 0, outs.0, 1, 8, 0, 0, 1), 0);
            wr::<u16>(partition, SP::numVertices, 2);
            assert_eq!(build_vertex_buffer(this, shape, 0, partition, 0, 0, outs.0, 1, 8, 0, 0, 0), 0);
            assert!(calls().is_empty());
            build_vertex_buffer(this, shape, 0, partition, 0, 0, outs.0, 1, 8, 0, 0, 1);
            assert_eq!(calls()[0], ("CreateVertexBuffer", 40, 0));
        }
    }

    #[test]
    fn compile_without_a_declaration_keeps_function_and_usage() {
        unsafe {
            let this = shader_interface(0);
            assert_eq!(compile_vertex_shader(this, 0x1111, 0x2222, 1), 0);
            assert_eq!(rd::<u32>(rd::<usize>(this + SI::shaderFunctions, ARRAY_STORAGE), 4), 0x1111);
            assert_eq!(rd::<u32>(rd::<usize>(this + SI::shaderUsages, ARRAY_STORAGE), 4), 0x2222);
            // For an index after the end, the array first grows by its step.
            compile_vertex_shader(this, 0x3333, 0x4444, 5);
            assert_eq!(rd::<u32>(this + SI::shaderFunctions, ARRAY_STORAGE_COUNT), 6);
            assert_eq!(rd::<u32>(rd::<usize>(this + SI::shaderFunctions, ARRAY_STORAGE), 20), 0x3333);
        }
    }

    #[test]
    fn first_shader_handle_goes_into_an_array_that_may_be_empty() {
        unsafe {
            let array = leak(0x18);
            // With no storage and no step the port does nothing.
            set_first_shader_handle(array, 9);
            assert_eq!(rd::<[u32; 6]>(array, 0), [0; 6]);
            // With no storage the port makes storage of the size of the step.
            wr::<u32>(array, ARRAY_GROW_BY, 3);
            wr(array, ARRAY_STORAGE, 0x1234usize);
            eng::DELETED.with(|d| d.borrow_mut().clear());
            set_first_shader_handle(array, 9);
            assert_eq!(eng::DELETED.with(|d| core::mem::take(&mut *d.borrow_mut())), [0x1234]);
            assert_eq!(rd::<[u32; 3]>(array, ARRAY_STORAGE_COUNT), [3, 1, 1]);
            assert_eq!(rd::<u32>(rd::<usize>(array, ARRAY_STORAGE), 0), 9);
            // The port replaces a value that is there. The filled count stays.
            set_first_shader_handle(array, 10);
            assert_eq!(rd::<[u32; 3]>(array, ARRAY_STORAGE_COUNT), [3, 1, 1]);
            assert_eq!(rd::<u32>(rd::<usize>(array, ARRAY_STORAGE), 0), 10);
        }
    }
}
