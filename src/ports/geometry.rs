//! `NiDX8Renderer`: the cache of geometry buffers (one `NiGeometryBufferData` for each
//! `NiGeometryData` and for each skin partition), the precache functions, the screen polygon
//! and the two functions that put an object into the batch list.

use core::mem::transmute;

use crate::engine::operator_new;
use crate::gen::layouts::{
    NiDX8IndexBufferManager as IBM, NiDX8Renderer as R, NiDX8Renderer_BatchedObject as Batched,
    NiDX8VBResource as VBResource, NiDX8VertexBufferManager as VBM, NiDX8VertexBufferManager_VBInfo as VBInfo,
    NiGeometry, NiGeometryBufferData as GB, NiGeometryData, NiRefObject, NiRendererSpecificProperty,
    NiRendererSpecificProperty_RenderData7 as RenderData7, NiSkinInstance, NiSkinPartition,
    NiSkinPartition_Partition as Partition, NiTransform, NiTriShapeData, NiTriStripsData, THashMap, THashMap_Node,
};
use crate::mem::{rd, wr};
use crate::shadow::Regions;

// ---------------------------------------------------------------------------------------
// Engine data and functions at fixed addresses
// ---------------------------------------------------------------------------------------

/// `NiRTTI` of `NiTriShape`.
const RTTI_NI_TRI_SHAPE: usize = 0x7DE550;
/// `NiRTTI` of `NiTriStrips`.
const RTTI_NI_TRI_STRIPS: usize = 0x7DE5A8;
/// `NiRTTI` of `NiTriShapeData`.
const RTTI_NI_TRI_SHAPE_DATA: usize = 0x7DE490;
/// `NiRTTI` of `NiTriStripsData`.
const RTTI_NI_TRI_STRIPS_DATA: usize = 0x7DE588;
/// `NiRTTI::baseRTTI`
const RTTI_BASE: usize = 4;
/// The identity `matrix33`. A static constructor of the game fills it.
const IDENTITY_MATRIX33: usize = 0x7DE664;
/// `vec3` (0, 0, 0). A static constructor of the game fills it.
const ZERO_VEC3: usize = 0x7DE6CC;
/// 1.0 as bits.
const ONE: u32 = 0x3F80_0000;

const D3DTS_VIEW: u32 = 2;
const D3DTS_PROJECTION: u32 = 3;
const D3DPT_TRIANGLELIST: u32 = 4;
const D3DPT_TRIANGLESTRIP: u32 = 5;
const D3DPT_TRIANGLEFAN: u32 = 6;
const D3DPOOL_MANAGED: u32 = 1;
const D3DFVF_XYZ: u32 = 0x002;
const D3DFVF_DIFFUSE: u32 = 0x040;
const D3DFVF_TEX1: u32 = 0x100;

// Virtual functions.
/// `NiObject::GetRTTI`
const V_GET_RTTI: usize = 0x04;
/// `NiGeometryData::GetActiveVertexCount`
const V_GET_ACTIVE_VERTEX_COUNT: usize = 0x30;
/// `NiTriBasedGeomData::GetActiveTriangleCount`
const V_GET_ACTIVE_TRIANGLE_COUNT: usize = 0x3C;
/// `NiTriBasedGeomData::GetTriList_const`
const V_GET_TRI_LIST: usize = 0x44;
/// `NiRendererSpecificProperty::RenderDataBase::getRendererTypeId`
const V_GET_RENDERER_TYPE_ID: usize = 0x04;
/// `InterfaceTHashMap`
const V_HASH_KEY: usize = 0x04;
const V_CMP_KEY: usize = 0x08;
const V_MAKE_KEY_VALUE_PAIR: usize = 0x0C;
const V_DTOR_KEY_VALUE_PAIR: usize = 0x10;
/// `IUnknown::Release`
const V_RELEASE: usize = 0x08;

/// `NiSkinData::skinPartition`
const SKIN_DATA_SKIN_PARTITION: usize = 0x08;

unsafe fn thiscall0(addr: usize, this: usize) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize) -> u32>(addr)(this)
}

unsafe fn thiscall1(addr: usize, this: usize, a: u32) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32) -> u32>(addr)(this, a)
}

unsafe fn thiscall2(addr: usize, this: usize, a: u32, b: u32) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32, u32) -> u32>(addr)(this, a, b)
}

unsafe fn thiscall3(addr: usize, this: usize, a: u32, b: u32, c: u32) -> u32 {
    transmute::<usize, unsafe extern "thiscall" fn(usize, u32, u32, u32) -> u32>(addr)(this, a, b, c)
}

/// A virtual function of `object` at `offset` in its vtable.
unsafe fn virtual_at(object: usize, offset: usize) -> usize {
    rd(rd::<usize>(object, 0), offset)
}

/// A virtual function without arguments.
unsafe fn vcall0(object: usize, offset: usize) -> u32 {
    thiscall0(virtual_at(object, offset), object)
}

/// 0x727530 `operator delete` of the game's C runtime.
unsafe fn operator_delete(block: usize) {
    transmute::<usize, unsafe extern "C" fn(usize)>(0x727530)(block)
}

/// 0x6BDE50 `NiDX8VertexBufferManager::PackUntransformedVB`. Returns the vertex buffer.
#[allow(clippy::too_many_arguments)]
unsafe fn vbm_pack_untransformed(
    manager: usize, geom_data: usize, previous: u32, out_size: usize, skin: usize, flag: u32, pool: u32, usage: u32,
    out_stride: usize, out_fvf: usize,
) -> u32 {
    let f: unsafe extern "thiscall" fn(usize, usize, u32, usize, usize, u32, u32, u32, usize, usize) -> u32 =
        transmute(0x6BDE50usize);
    f(manager, geom_data, previous, out_size, skin, flag, pool, usage, out_stride, out_fvf)
}

/// 0x6BE2B0 `NiDX8VertexBufferManager::PackSkinnedVB`. Another mod changes it in memory, so
/// it stays engine code. Returns the vertex buffer.
#[allow(clippy::too_many_arguments)]
unsafe fn vbm_pack_skinned(
    manager: usize, geom_data: usize, skin: usize, partition: usize, previous: u32, out_size: usize, pool: u32, usage: u32,
    out_stride: usize, out_fvf: usize,
) -> u32 {
    let f: unsafe extern "thiscall" fn(usize, usize, usize, usize, u32, usize, u32, u32, usize, usize) -> u32 =
        transmute(0x6BE2B0usize);
    f(manager, geom_data, skin, partition, previous, out_size, pool, usage, out_stride, out_fvf)
}

/// 0x6BE180 `NiDX8VertexBufferManager::PackScreenPolyVB`. Returns the vertex buffer.
#[allow(clippy::too_many_arguments)]
unsafe fn vbm_pack_screen_poly(
    manager: usize, count: u32, vertices: usize, tex_coords: usize, colour: usize, near: u32, distance: u32, out_fvf: usize,
    out_stride: usize,
) -> u32 {
    let f: unsafe extern "thiscall" fn(usize, u32, usize, usize, usize, u32, u32, usize, usize) -> u32 =
        transmute(0x6BE180usize);
    f(manager, count, vertices, tex_coords, colour, near, distance, out_fvf, out_stride)
}

/// 0x6BCE80 `NiDX8IndexBufferManager::createIndexBuffer`. Returns the index buffer.
unsafe fn ibm_create_index_buffer(manager: usize, indices: u32, count: u32, previous: u32, out_size: usize, pool: u32, usage: u32) -> u32 {
    let f: unsafe extern "thiscall" fn(usize, u32, u32, u32, usize, u32, u32) -> u32 = transmute(0x6BCE80usize);
    f(manager, indices, count, previous, out_size, pool, usage)
}

/// 0x6BF0B0 `NiD3DShaderInterface::buildVertexBuffer`. Returns the vertex buffer.
#[allow(clippy::too_many_arguments)]
unsafe fn shader_build_vertex_buffer(
    shader: usize, geom_data: usize, skin: usize, partition: usize, data2: u32, previous: u32, out_size: usize, pool: u32,
    usage: u32, out_stride: usize, out_fvf: usize, last: u32,
) -> u32 {
    let f: unsafe extern "thiscall" fn(usize, usize, usize, usize, u32, u32, usize, u32, u32, usize, usize, u32) -> u32 =
        transmute(0x6BF0B0usize);
    f(shader, geom_data, skin, partition, data2, previous, out_size, pool, usage, out_stride, out_fvf, last)
}

/// 0x6C6AC0 `NiSkinPartition::ctor`. Returns the object.
unsafe fn skin_partition_ctor(block: usize) -> usize {
    thiscall0(0x6C6AC0, block) as usize
}

/// 0x6C78F0 `NiSkinPartition::MakePartitions`. The two counts are bytes for the function;
/// the game code pushes whole registers.
unsafe fn skin_partition_make_partitions(object: usize, geom_data: usize, skin_data: usize, per_partition: u32, per_vertex: u32) -> u8 {
    let f: unsafe extern "thiscall" fn(usize, usize, usize, u32, u32) -> u32 = transmute(0x6C78F0usize);
    f(object, geom_data, skin_data, per_partition, per_vertex) as u8
}

/// 0x741E30 `NiTriStripsData::GetStripLengthSum`. The result is in AX.
unsafe fn tri_strips_data_strip_length_sum(data: usize) -> u32 {
    thiscall0(0x741E30, data)
}

/// 0x6ED100 `NiBound_or_NiQuaternion::Copy`
unsafe fn bound_copy(to: usize, from: usize) {
    thiscall1(0x6ED100, to, from as u32);
}

/// 0x6F25E0 `NiBound_or_NiPlane::ctor`
unsafe fn bound_ctor(bound: usize) {
    thiscall0(0x6F25E0, bound);
}

/// 0x6E9A00 `nullsub_36`: `retn`. The game code calls it as the destructor of the bound.
unsafe fn nullsub_36(bound: usize) {
    thiscall0(0x6E9A00, bound);
}

/// 0x6E7CD0 `matrix33::copy`
unsafe fn matrix33_copy(to: usize, from: usize) {
    thiscall1(0x6E7CD0, to, from as u32);
}

/// 0x6AEB80 `NiDX8Renderer::DrawPrimitive`. Another mod changes it in memory, so it stays
/// engine code.
unsafe fn renderer_draw_primitive(this: usize, geom_data: usize, skin: usize, transform: usize, bound: usize, buffer: usize, last: u32) {
    let f: unsafe extern "thiscall" fn(usize, usize, usize, usize, usize, usize, u32) -> u32 = transmute(0x6AEB80usize);
    f(this, geom_data, skin, transform, bound, buffer, last);
}

// ---------------------------------------------------------------------------------------
// The device and the buffers, through their vtables on each call
// ---------------------------------------------------------------------------------------

unsafe fn device_set_transform(device: usize, state: u32, matrix: usize) -> u32 {
    transmute::<usize, unsafe extern "system" fn(usize, u32, usize) -> u32>(virtual_at(device, 0x94))(device, state, matrix)
}

/// `IUnknown::Release` of a vertex or index buffer.
unsafe fn com_release(object: u32) -> u32 {
    let object = object as usize;
    transmute::<usize, unsafe extern "system" fn(usize) -> u32>(virtual_at(object, V_RELEASE))(object)
}

// ---------------------------------------------------------------------------------------
// Shared pieces
// ---------------------------------------------------------------------------------------

/// True when the run-time type of `object` is `rtti` or comes from it.
unsafe fn is_kind_of(object: usize, rtti: usize) -> bool {
    let mut at = vcall0(object, V_GET_RTTI) as usize;
    while at != 0 {
        if at == rtti {
            return true;
        }
        at = rd(at, RTTI_BASE);
    }
    false
}

unsafe fn hash_key(map: usize, key: u32) -> usize {
    thiscall1(virtual_at(map, V_HASH_KEY), map, key) as usize
}

unsafe fn hash_cmp_key(map: usize, key: u32, node: usize) -> bool {
    thiscall2(virtual_at(map, V_CMP_KEY), map, key, rd(node, THashMap_Node::key)) as u8 != 0
}

unsafe fn hash_bucket_head(map: usize, bucket: usize) -> usize {
    rd(rd::<usize>(map, THashMap::arrayBuckets), bucket * 4)
}

/// The node a `THashMap` has for `key`, or 0.
unsafe fn hash_find_node(map: usize, key: u32) -> usize {
    let bucket = hash_key(map, key);
    let mut node = hash_bucket_head(map, bucket);
    while node != 0 {
        if hash_cmp_key(map, key, node) {
            return node;
        }
        node = rd(node, THashMap_Node::nextNode);
    }
    0
}

/// `THashMap::SetAt` as the game code has it inline: the node of the key gets the new pair,
/// or a new node goes to the head of its bucket.
unsafe fn hash_set(map: usize, key: u32, value: u32) {
    let bucket = hash_key(map, key);
    let mut node = hash_bucket_head(map, bucket);
    while node != 0 {
        if hash_cmp_key(map, key, node) {
            thiscall1(virtual_at(map, V_DTOR_KEY_VALUE_PAIR), map, node as u32);
            thiscall3(virtual_at(map, V_MAKE_KEY_VALUE_PAIR), map, node as u32, key, value);
            return;
        }
        node = rd(node, THashMap_Node::nextNode);
    }
    let node = operator_new(THashMap_Node::SIZE);
    thiscall3(virtual_at(map, V_MAKE_KEY_VALUE_PAIR), map, node as u32, key, value);
    wr(node, THashMap_Node::nextNode, hash_bucket_head(map, bucket));
    wr(rd::<usize>(map, THashMap::arrayBuckets), bucket * 4, node);
    wr(map, THashMap::count, rd::<u32>(map, THashMap::count).wrapping_add(1));
}

/// Takes the node of `key` out of its bucket. Returns the node, or 0 when there is none.
unsafe fn hash_unlink(map: usize, key: u32) -> usize {
    let bucket = hash_key(map, key);
    let mut node = hash_bucket_head(map, bucket);
    if node == 0 {
        return 0;
    }
    if hash_cmp_key(map, key, node) {
        let next: usize = rd(node, THashMap_Node::nextNode);
        wr(rd::<usize>(map, THashMap::arrayBuckets), bucket * 4, next);
        return node;
    }
    loop {
        let previous = node;
        node = rd(node, THashMap_Node::nextNode);
        if node == 0 {
            return 0;
        }
        if hash_cmp_key(map, key, node) {
            wr(previous, THashMap_Node::nextNode, rd::<usize>(node, THashMap_Node::nextNode));
            return node;
        }
    }
}

/// `THashMap::RemoveAt` as the game code has it inline.
unsafe fn hash_remove(map: usize, key: u32) {
    let node = hash_unlink(map, key);
    if node == 0 {
        return;
    }
    thiscall1(virtual_at(map, V_DTOR_KEY_VALUE_PAIR), map, node as u32);
    operator_delete(node);
    wr(map, THashMap::count, rd::<u32>(map, THashMap::count).wrapping_sub(1));
}

/// The members a new `NiGeometryBufferData` gets as zero, in the order of the game code.
/// `primitiveType` (+0x2C) is not one of them.
const NEW_BUFFER_ZEROED: [usize; 15] = [
    GB::geomDataRevisionID,
    GB::flags,
    GB::fvf,
    GB::verticesCount,
    GB::vertexBufferSize,
    GB::d3dVertexBuffer,
    GB::vertexStride,
    GB::indexBufferEntries,
    GB::indexBufferSize,
    GB::indexBuffer,
    GB::field_28,
    GB::trianglesCount,
    GB::stripCount,
    GB::stripVertexCounts,
    GB::triList,
];

/// Makes a `NiGeometryBufferData` and puts it into `*out`. The game code reads `*out` again
/// for each member.
unsafe fn new_geometry_buffer(out: usize) {
    let block = operator_new(GB::SIZE);
    wr(out, 0, block);
    for offset in NEW_BUFFER_ZEROED {
        wr::<u32>(rd::<usize>(out, 0), offset, 0);
    }
}

/// The assignment to a `NiPointer` member as the game code has it inline.
unsafe fn assign_pointer(slot: usize, object: usize) {
    let old: usize = rd(slot, 0);
    if old == object {
        return;
    }
    if old != 0 {
        let count = rd::<u32>(old, NiRefObject::refCount).wrapping_sub(1);
        wr(old, NiRefObject::refCount, count);
        if count == 0 {
            // The deleting destructor, with "free the memory".
            thiscall1(virtual_at(old, 0), old, 1);
        }
    }
    wr(slot, 0, object);
    if object != 0 {
        wr(object, NiRefObject::refCount, rd::<u32>(object, NiRefObject::refCount).wrapping_add(1));
    }
}

/// `new NiSkinPartition`. Returns the object and what the constructor left in EAX.
unsafe fn new_skin_partition() -> usize {
    let block = operator_new(NiSkinPartition::SIZE);
    if block == 0 {
        0
    } else {
        skin_partition_ctor(block)
    }
}

/// Index buffer entries of a buffer whose counts are set: three for each triangle of a
/// list, or the triangles plus two for each strip.
unsafe fn index_entries_of(buffer: usize) -> u32 {
    let triangles: u32 = rd(buffer, GB::trianglesCount);
    if rd::<u32>(buffer, GB::primitiveType) == D3DPT_TRIANGLELIST {
        triangles.wrapping_mul(3)
    } else {
        triangles.wrapping_add(rd::<u32>(buffer, GB::stripCount).wrapping_mul(2))
    }
}

/// Adds regions and counts them: `Regions` holds 8 and drops the rest without a word.
struct Tally<'a> {
    set: &'a mut Regions,
    used: usize,
}

impl<'a> Tally<'a> {
    const MAX: usize = 8;

    fn new(set: &'a mut Regions) -> Tally<'a> {
        Tally { set, used: 0 }
    }

    fn add(&mut self, addr: usize, size: usize) {
        if addr == 0 || size == 0 {
            return;
        }
        self.used += 1;
        if self.used > Self::MAX {
            self.set.skip();
        } else {
            self.set.add(addr, size);
        }
    }

    /// The bucket array of a hash map.
    unsafe fn buckets(&mut self, map: usize) {
        self.add(rd(map, THashMap::arrayBuckets), rd::<usize>(map, THashMap::bucketCount) * 4);
    }

    /// The two buffer managers. Their functions run as game code inside the ports here.
    unsafe fn managers(&mut self, this: usize) {
        self.add(rd(this, R::vertexBufferManager), VBM::SIZE);
        self.add(rd(this, R::indexBufferManager), IBM::SIZE);
    }
}

// ---------------------------------------------------------------------------------------
// Purges
// ---------------------------------------------------------------------------------------

/// Releases the two device buffers of a `NiGeometryBufferData` and frees it.
unsafe fn destroy_geometry_buffer(buffer: usize) {
    let vertex_buffer: u32 = rd(buffer, GB::d3dVertexBuffer);
    if vertex_buffer != 0 {
        com_release(vertex_buffer);
    }
    let index_buffer: u32 = rd(buffer, GB::indexBuffer);
    if index_buffer != 0 {
        com_release(index_buffer);
    }
    operator_delete(buffer);
}

/// 0x6AF2D0 `NiDX8Renderer::PurgeSkinPartition`
pub unsafe fn purge_skin_partition(this: usize, partition: usize) {
    let buffer: usize = rd(partition, Partition::bufferData);
    if buffer == 0 {
        return;
    }
    destroy_geometry_buffer(buffer);
    wr::<u32>(partition, Partition::bufferData, 0);
    hash_remove(this + R::hashmap_skinPartitionsToGeomBuffers, partition as u32);
}

/// 0x6AF3A0 `NiDX8Renderer::PurgeGeometryData`
pub unsafe fn purge_geometry_data(this: usize, geom_data: usize) {
    let key: u32 = rd(geom_data, NiGeometryData::uniqueID);
    let map = this + R::hashGeomBuffers;
    let node = hash_find_node(map, key);
    if node == 0 {
        return;
    }
    destroy_geometry_buffer(rd(node, THashMap_Node::data));
    hash_remove(map, key);
}

/// What a purge writes: the map (in the renderer), the bucket array, the node before the
/// one that goes. The buffer data and the node are freed (tracked). A buffer data with a
/// device buffer is not compared: `Release` is real in both runs.
unsafe fn purge_regions(set: &mut Regions, this: usize, map: usize, key: u32, buffer: usize, partition: usize) {
    let mut tally = Tally::new(set);
    tally.add(this, R::SIZE);
    tally.add(partition, Partition::SIZE);
    if buffer == 0 {
        return;
    }
    if rd::<u32>(buffer, GB::d3dVertexBuffer) != 0 || rd::<u32>(buffer, GB::indexBuffer) != 0 {
        tally.set.skip();
        return;
    }
    tally.set.allocates();
    tally.buckets(map);
    let mut node = hash_bucket_head(map, hash_key(map, key));
    while node != 0 && !hash_cmp_key(map, key, node) {
        tally.add(node, THashMap_Node::SIZE);
        node = rd(node, THashMap_Node::nextNode);
    }
}

unsafe fn purge_skin_partition_regions(set: &mut Regions, this: usize, partition: usize) {
    let map = this + R::hashmap_skinPartitionsToGeomBuffers;
    purge_regions(set, this, map, partition as u32, rd(partition, Partition::bufferData), partition);
}

unsafe fn purge_geometry_data_regions(set: &mut Regions, this: usize, geom_data: usize) {
    let map = this + R::hashGeomBuffers;
    let key: u32 = rd(geom_data, NiGeometryData::uniqueID);
    let node = hash_find_node(map, key);
    let buffer = if node == 0 { 0 } else { rd(node, THashMap_Node::data) };
    purge_regions(set, this, map, key, buffer, 0);
}

// ---------------------------------------------------------------------------------------
// The cache
// ---------------------------------------------------------------------------------------

/// 0x6AF4A0 `NiDX8Renderer::createGeometryBufferData`. Puts the buffer data of the geometry
/// data into `*out`. Returns 1 when it made a new one.
pub unsafe fn create_geometry_buffer_data(this: usize, out: usize, geom_data: usize) -> u8 {
    if geom_data == 0 {
        return 0;
    }
    let map = this + R::hashGeomBuffers;
    let node = hash_find_node(map, rd(geom_data, NiGeometryData::uniqueID));
    if node != 0 {
        wr::<u32>(out, 0, rd(node, THashMap_Node::data));
        return 0;
    }
    new_geometry_buffer(out);
    let key: u32 = rd(geom_data, NiGeometryData::uniqueID);
    let value: u32 = rd(out, 0);
    hash_set(map, key, value);
    1
}

/// A new buffer data and a new node are new blocks; the bucket array and the count exist.
unsafe fn create_geometry_buffer_data_regions(set: &mut Regions, this: usize, out: usize, geom_data: usize) {
    let mut tally = Tally::new(set);
    tally.add(this, R::SIZE);
    tally.add(out, 4);
    if geom_data == 0 {
        return;
    }
    tally.set.allocates();
    tally.buckets(this + R::hashGeomBuffers);
}

/// 0x6AF930 `NiDX8Renderer::findCachedPartitionGeomBuffer`. Puts the buffer data of the
/// partition into `*out`. Returns 1 when it made a new one. `*out` is an input too: with
/// no entry in the map and a value in `*out`, nothing is made.
pub unsafe fn find_cached_partition_geom_buffer(this: usize, out: usize, geom_data: usize, skin: usize, partition: usize) -> u8 {
    if geom_data == 0 || skin == 0 {
        return 0;
    }
    let map = this + R::hashmap_skinPartitionsToGeomBuffers;
    let node = hash_find_node(map, partition as u32);
    if node != 0 {
        wr::<u32>(out, 0, rd(node, THashMap_Node::data));
    }
    if rd::<u32>(out, 0) != 0 {
        return 0;
    }
    new_geometry_buffer(out);
    wr::<u32>(partition, Partition::bufferData, rd(out, 0));
    let value: u32 = rd(out, 0);
    hash_set(map, partition as u32, value);
    1
}

unsafe fn find_cached_partition_geom_buffer_regions(set: &mut Regions, this: usize, out: usize, geom_data: usize, skin: usize, partition: usize) {
    let mut tally = Tally::new(set);
    tally.add(this, R::SIZE);
    tally.add(out, 4);
    if geom_data == 0 || skin == 0 {
        return;
    }
    tally.set.allocates();
    tally.add(partition, Partition::SIZE);
    tally.buckets(this + R::hashmap_skinPartitionsToGeomBuffers);
}

/// What `packGeometryBuffers` reads before it decides: the active vertex count (EAX of the
/// virtual call; the low word counts), the triangle list and the index buffer entries.
unsafe fn pack_inputs(out: usize, geom_data: usize) -> (u32, u32, u32) {
    let active_vertices = vcall0(geom_data, V_GET_ACTIVE_VERTEX_COUNT) & 0xFFFF;
    let mut tri_list: u32 = rd(rd::<usize>(out, 0), GB::triList);
    if is_kind_of(geom_data, RTTI_NI_TRI_SHAPE_DATA) {
        tri_list = vcall0(geom_data, V_GET_TRI_LIST);
    }
    let buffer: usize = rd(out, 0);
    let entries = if rd::<u32>(buffer, GB::primitiveType) == D3DPT_TRIANGLELIST {
        (vcall0(geom_data, V_GET_ACTIVE_TRIANGLE_COUNT) & 0xFFFF).wrapping_mul(3)
    } else {
        rd::<u32>(buffer, GB::trianglesCount).wrapping_add(rd::<u32>(buffer, GB::stripCount).wrapping_mul(2))
    };
    (active_vertices, tri_list, entries)
}

/// The buffers of `buffer` that `packGeometryBuffers` keeps for a geometry data that has
/// its vertex buffer: (vertex buffer, index buffer), 0 for one that it makes again.
unsafe fn pack_kept(buffer: usize, geom_data: usize, active_vertices: u32, tri_list: u32, entries: u32) -> (u32, u32) {
    let (mut vertex_buffer, mut index_buffer) = (0, 0);
    if rd::<u32>(buffer, GB::geomDataRevisionID) == rd::<u16>(geom_data, NiGeometryData::revisionID) as u32 {
        if rd::<u32>(buffer, GB::verticesCount) >= active_vertices {
            vertex_buffer = rd(buffer, GB::d3dVertexBuffer);
        }
        if tri_list != 0 {
            let current: u32 = rd(buffer, GB::indexBuffer);
            if current != 0 && rd::<u32>(buffer, GB::indexBufferEntries) >= entries {
                index_buffer = current;
            }
        }
    }
    (vertex_buffer, index_buffer)
}

/// 0x6AF610 `NiDX8Renderer::packGeometryBuffers`. `shader` is a `NiD3DShaderInterface` or 0.
/// Returns 0 without a buffer data or a geometry data, else 1.
pub unsafe fn pack_geometry_buffers(this: usize, out: usize, geom_data: usize, skin: usize, shader: usize, shader_data: u32) -> u8 {
    let buffer: usize = rd(out, 0);
    if buffer == 0 || geom_data == 0 {
        return 0;
    }
    if rd::<u32>(buffer, GB::d3dVertexBuffer) != 0 && rd::<u8>(buffer, GB::flags) & 1 != 0 {
        return 1;
    }
    let (active_vertices, tri_list, entries) = pack_inputs(out, geom_data);

    let buffer: usize = rd(out, 0);
    let old_vertex_buffer: u32 = rd(buffer, GB::d3dVertexBuffer);
    if old_vertex_buffer != 0 {
        let (mut vertex_buffer, mut index_buffer) = pack_kept(buffer, geom_data, active_vertices, tri_list, entries);
        if vertex_buffer == 0 || skin != 0 {
            let flag = (vertex_buffer == 0) as u32;
            if shader != 0 {
                vertex_buffer = shader_build_vertex_buffer(
                    shader, geom_data, 0, 0, shader_data, old_vertex_buffer, buffer + GB::vertexBufferSize, D3DPOOL_MANAGED, 0,
                    buffer + GB::vertexStride, buffer + GB::fvf, 0,
                );
            }
            if vertex_buffer == 0 || skin != 0 {
                let buffer: usize = rd(out, 0);
                vertex_buffer = vbm_pack_untransformed(
                    rd(this, R::vertexBufferManager), geom_data, rd(buffer, GB::d3dVertexBuffer), buffer + GB::vertexBufferSize,
                    skin, flag, D3DPOOL_MANAGED, 0, buffer + GB::vertexStride, buffer + GB::fvf,
                );
            }
        }
        if index_buffer == 0 && tri_list != 0 {
            let buffer: usize = rd(out, 0);
            index_buffer = ibm_create_index_buffer(
                rd(this, R::indexBufferManager), tri_list, entries, rd(buffer, GB::indexBuffer), buffer + GB::indexBufferSize,
                D3DPOOL_MANAGED, 0,
            );
        }
        let current: u32 = rd(rd::<usize>(out, 0), GB::d3dVertexBuffer);
        if current != vertex_buffer {
            if current != 0 {
                com_release(current);
            }
            wr(rd::<usize>(out, 0), GB::d3dVertexBuffer, vertex_buffer);
            wr(rd::<usize>(out, 0), GB::verticesCount, active_vertices);
        }
        if tri_list != 0 {
            let current: u32 = rd(rd::<usize>(out, 0), GB::indexBuffer);
            if current != index_buffer {
                if current != 0 {
                    com_release(current);
                }
                wr(rd::<usize>(out, 0), GB::indexBuffer, index_buffer);
                wr(rd::<usize>(out, 0), GB::indexBufferEntries, entries);
            }
        }
    } else {
        wr::<u32>(buffer, GB::vertexBufferSize, 0);
        wr::<u32>(rd::<usize>(out, 0), GB::indexBufferSize, 0);
        if shader != 0 {
            let buffer: usize = rd(out, 0);
            let made = shader_build_vertex_buffer(
                shader, geom_data, 0, 0, shader_data, rd(buffer, GB::d3dVertexBuffer), buffer + GB::vertexBufferSize,
                D3DPOOL_MANAGED, 0, buffer + GB::vertexStride, buffer + GB::fvf, 0,
            );
            wr(rd::<usize>(out, 0), GB::d3dVertexBuffer, made);
        }
        let buffer: usize = rd(out, 0);
        let current: u32 = rd(buffer, GB::d3dVertexBuffer);
        if current == 0 || skin != 0 {
            // `setz cl; push ecx` with the vertex buffer in ECX.
            let flag = (current & !0xFF) | (current == 0) as u32;
            let made = vbm_pack_untransformed(
                rd(this, R::vertexBufferManager), geom_data, 0, buffer + GB::vertexBufferSize, skin, flag, D3DPOOL_MANAGED, 0,
                buffer + GB::vertexStride, buffer + GB::fvf,
            );
            wr(rd::<usize>(out, 0), GB::d3dVertexBuffer, made);
        }
        if tri_list != 0 {
            let made = ibm_create_index_buffer(
                rd(this, R::indexBufferManager), tri_list, entries, 0, rd::<usize>(out, 0) + GB::indexBufferSize, D3DPOOL_MANAGED, 0,
            );
            wr(rd::<usize>(out, 0), GB::indexBuffer, made);
        }
        wr::<u32>(rd::<usize>(out, 0), GB::flags, 0);
        wr::<u32>(rd::<usize>(out, 0), GB::field_28, 0);
        wr(rd::<usize>(out, 0), GB::verticesCount, active_vertices);
        wr(rd::<usize>(out, 0), GB::indexBufferEntries, entries);
    }
    wr(rd::<usize>(out, 0), GB::geomDataRevisionID, rd::<u16>(geom_data, NiGeometryData::revisionID) as u32);
    let triangles = vcall0(geom_data, V_GET_ACTIVE_TRIANGLE_COUNT) & 0xFFFF;
    wr(rd::<usize>(out, 0), GB::trianglesCount, triangles);
    1
}

/// Not compared:
/// - with a shader interface: `buildVertexBuffer` can make a vertex shader on the device;
/// - when the vertex buffer is packed again over one that exists, or the index buffer is
///   made again over one that exists: the manager gives the same buffer or a new one, and
///   with a new one the function releases the old one. `Release` is real in both runs.
unsafe fn pack_geometry_buffers_regions(set: &mut Regions, this: usize, out: usize, geom_data: usize, skin: usize, shader: usize) {
    let mut tally = Tally::new(set);
    let buffer: usize = rd(out, 0);
    if buffer == 0 || geom_data == 0 {
        return;
    }
    tally.add(buffer, GB::SIZE);
    if rd::<u32>(buffer, GB::d3dVertexBuffer) == 0 {
        if shader != 0 {
            tally.set.skip();
        }
        tally.managers(this);
        return;
    }
    if rd::<u8>(buffer, GB::flags) & 1 != 0 {
        return;
    }
    tally.managers(this);
    let (active_vertices, tri_list, entries) = pack_inputs(out, geom_data);
    let (vertex_buffer, index_buffer) = pack_kept(buffer, geom_data, active_vertices, tri_list, entries);
    let packs_again = vertex_buffer == 0 || skin != 0;
    let indexes_again = index_buffer == 0 && tri_list != 0 && rd::<u32>(buffer, GB::indexBuffer) != 0;
    if packs_again || indexes_again {
        tally.set.skip();
    }
}

/// 0x6AFAB0 `NiDX8Renderer::createPartitionGeomBuffer`: packs the buffers of a skin
/// partition. Returns 0 without a buffer data, geometry data or skin instance, else 1.
pub unsafe fn create_partition_geom_buffer(this: usize, out: usize, geom_data: usize, skin: usize, partition: usize) -> u8 {
    let buffer: usize = rd(out, 0);
    if buffer == 0 || geom_data == 0 || skin == 0 {
        return 0;
    }
    let old_vertex_buffer: u32 = rd(buffer, GB::d3dVertexBuffer);
    if old_vertex_buffer != 0 && rd::<u8>(buffer, GB::flags) & 1 != 0 {
        return 1;
    }
    let tri_list: u32 = rd(buffer, GB::triList);
    let vertices = rd::<u16>(partition, Partition::numVertices) as u32;
    let entries = index_entries_of(buffer);
    if old_vertex_buffer != 0 {
        let (mut vertex_buffer, mut index_buffer) = (0u32, 0u32);
        if rd::<u32>(buffer, GB::geomDataRevisionID) == rd::<u16>(geom_data, NiGeometryData::revisionID) as u32 {
            if rd::<u32>(buffer, GB::verticesCount) >= vertices {
                vertex_buffer = old_vertex_buffer;
            }
            let current: u32 = rd(buffer, GB::indexBuffer);
            if current != 0 && rd::<u32>(buffer, GB::indexBufferEntries) >= entries {
                index_buffer = current;
            }
        }
        if vertex_buffer == 0 {
            vertex_buffer = vbm_pack_skinned(
                rd(this, R::vertexBufferManager), geom_data, skin, partition, old_vertex_buffer, buffer + GB::vertexBufferSize,
                D3DPOOL_MANAGED, 0, 0, 0,
            );
        }
        if index_buffer == 0 {
            let buffer: usize = rd(out, 0);
            index_buffer = ibm_create_index_buffer(
                rd(this, R::indexBufferManager), tri_list, entries, rd(buffer, GB::indexBuffer), buffer + GB::indexBufferSize,
                D3DPOOL_MANAGED, 0,
            );
        }
        let current: u32 = rd(rd::<usize>(out, 0), GB::d3dVertexBuffer);
        if current != vertex_buffer {
            if current != 0 {
                com_release(current);
            }
            wr(rd::<usize>(out, 0), GB::d3dVertexBuffer, vertex_buffer);
            wr(rd::<usize>(out, 0), GB::verticesCount, vertices);
        }
        let current: u32 = rd(rd::<usize>(out, 0), GB::indexBuffer);
        if current != index_buffer {
            if current != 0 {
                com_release(current);
            }
            wr(rd::<usize>(out, 0), GB::indexBuffer, index_buffer);
            wr(rd::<usize>(out, 0), GB::indexBufferEntries, entries);
        }
    } else {
        wr::<u32>(buffer, GB::vertexBufferSize, 0);
        wr::<u32>(rd::<usize>(out, 0), GB::indexBufferSize, 0);
        let buffer: usize = rd(out, 0);
        let made = vbm_pack_skinned(
            rd(this, R::vertexBufferManager), geom_data, skin, partition, 0, buffer + GB::vertexBufferSize, D3DPOOL_MANAGED, 0,
            buffer + GB::vertexStride, buffer + GB::fvf,
        );
        wr(rd::<usize>(out, 0), GB::d3dVertexBuffer, made);
        let made = ibm_create_index_buffer(
            rd(this, R::indexBufferManager), tri_list, entries, 0, rd::<usize>(out, 0) + GB::indexBufferSize, D3DPOOL_MANAGED, 0,
        );
        wr(rd::<usize>(out, 0), GB::indexBuffer, made);
        wr::<u32>(rd::<usize>(out, 0), GB::flags, 0);
        wr::<u32>(rd::<usize>(out, 0), GB::field_28, 0);
        wr(rd::<usize>(out, 0), GB::verticesCount, vertices);
        wr(rd::<usize>(out, 0), GB::indexBufferEntries, entries);
    }
    wr(rd::<usize>(out, 0), GB::geomDataRevisionID, rd::<u16>(geom_data, NiGeometryData::revisionID) as u32);
    1
}

/// Not compared when a buffer that exists is replaced: `PackSkinnedVB` always makes a new
/// vertex buffer and the function releases the old one; the index buffer manager gives the
/// same index buffer or a new one. `Release` is real in both runs.
unsafe fn create_partition_geom_buffer_regions(set: &mut Regions, this: usize, out: usize, geom_data: usize, skin: usize, partition: usize) {
    let mut tally = Tally::new(set);
    let buffer: usize = rd(out, 0);
    if buffer == 0 || geom_data == 0 || skin == 0 {
        return;
    }
    tally.add(buffer, GB::SIZE);
    let old_vertex_buffer: u32 = rd(buffer, GB::d3dVertexBuffer);
    if old_vertex_buffer == 0 {
        tally.managers(this);
        return;
    }
    if rd::<u8>(buffer, GB::flags) & 1 != 0 {
        return;
    }
    tally.managers(this);
    let same_revision = rd::<u32>(buffer, GB::geomDataRevisionID) == rd::<u16>(geom_data, NiGeometryData::revisionID) as u32;
    let keeps_vertices = same_revision && rd::<u32>(buffer, GB::verticesCount) >= rd::<u16>(partition, Partition::numVertices) as u32;
    let index_buffer: u32 = rd(buffer, GB::indexBuffer);
    let keeps_indices = same_revision && index_buffer != 0 && rd::<u32>(buffer, GB::indexBufferEntries) >= index_entries_of(buffer);
    if !keeps_vertices || (!keeps_indices && index_buffer != 0) {
        tally.set.skip();
    }
}

// ---------------------------------------------------------------------------------------
// The screen polygon
// ---------------------------------------------------------------------------------------

/// 0x6AFCC0 `NiDX8Renderer::PackScreenPolyGeomBuffer`. `near` and `distance` are floats the
/// game code moves as integers. Returns the buffer data in the renderer, or 0.
pub unsafe fn pack_screen_poly_geom_buffer(this: usize, count: u32, vertices: usize, tex_coords: usize, colour: usize, near: u32, distance: u32) -> usize {
    // The game code gives the places of its two last arguments for the results.
    let mut fvf = distance;
    let mut stride = near;
    let vertex_buffer = vbm_pack_screen_poly(
        rd(this, R::vertexBufferManager), count, vertices, tex_coords, colour, near, distance, &mut fvf as *mut u32 as usize,
        &mut stride as *mut u32 as usize,
    );
    if vertex_buffer == 0 {
        return 0;
    }
    let buffer = this + R::tempGeomBuffer;
    wr(buffer, GB::d3dVertexBuffer, vertex_buffer);
    wr(buffer, GB::fvf, fvf);
    wr(buffer, GB::vertexStride, stride);
    let count = count & 0xFFFF;
    wr(buffer, GB::verticesCount, count);
    wr::<u32>(buffer, GB::flags, 1);
    wr::<u32>(buffer, GB::indexBufferEntries, 0);
    wr::<u32>(buffer, GB::indexBuffer, 0);
    wr::<u32>(buffer, GB::field_28, 0);
    wr(buffer, GB::primitiveType, D3DPT_TRIANGLEFAN);
    wr(buffer, GB::trianglesCount, count.wrapping_sub(2));
    wr::<u32>(buffer, GB::stripCount, 1);
    wr::<u32>(buffer, GB::stripVertexCounts, 0);
    wr::<u32>(buffer, GB::triList, 0);
    wr::<u32>(buffer, GB::geomDataRevisionID, 0);
    buffer
}

/// `PackScreenPolyVB` takes the vertex buffer from the ring of five the manager has for a
/// format (`CreateVertexBuffer` at 0x6BDC70, pool 0). The ring entry and its resource are
/// regions. Not compared when the manager has no ring for the format yet (it makes one and
/// puts it into its map), or when the buffer of the entry is too small (it releases it).
unsafe fn pack_screen_poly_geom_buffer_regions(set: &mut Regions, this: usize, count: u32, tex_coords: usize, colour: usize) {
    let mut tally = Tally::new(set);
    tally.add(this, R::SIZE);
    let manager: usize = rd(this, R::vertexBufferManager);
    tally.add(manager, VBM::SIZE);
    let (mut fvf, mut stride) = (D3DFVF_XYZ, 12u32);
    if colour != 0 {
        fvf |= D3DFVF_DIFFUSE;
        stride += 4;
    }
    if tex_coords != 0 {
        fvf |= D3DFVF_TEX1;
        stride += 8;
    }
    let node = hash_find_node(manager + VBM::hashmap_C, fvf);
    if node == 0 {
        tally.set.skip();
        return;
    }
    tally.set.allocates();
    let info: usize = rd(node, THashMap_Node::data);
    tally.add(info, VBInfo::SIZE);
    let resource: usize = rd(info, VBInfo::resources + rd::<u8>(info, VBInfo::nextFreeResourceIndex) as usize * 4);
    if resource != 0 {
        if rd::<u32>(resource, VBResource::length) < stride.wrapping_mul(count) {
            tally.set.skip();
        }
        tally.add(resource, VBResource::SIZE);
    }
}

/// 0x6ADD20 `NiDX8Renderer::RenderScreenPoly`
pub unsafe fn render_screen_poly(this: usize, count: u32, vertices: usize, tex_coords: usize, colour: usize) {
    if rd::<u8>(this, R::bSceneOpen) == 0 || count == 0 {
        return;
    }
    bound_copy(this + R::worldBound, this + R::defaultBound);
    device_set_transform(rd(this, R::d3dDevice), D3DTS_VIEW, this + R::screenPolyView);
    device_set_transform(rd(this, R::d3dDevice), D3DTS_PROJECTION, this + R::screenPolyProj);
    let buffer = pack_screen_poly_geom_buffer(this, count, vertices, tex_coords, colour, rd(this, R::frustumNear), rd(this, R::frustumDistance));
    if buffer != 0 {
        let mut transform = [0u32; NiTransform::SIZE / 4];
        let at = transform.as_mut_ptr() as usize;
        matrix33_copy(at + NiTransform::rotation, IDENTITY_MATRIX33);
        let z: u32 = rd(ZERO_VEC3, 8);
        let x: u32 = rd(ZERO_VEC3, 0);
        let y: u32 = rd(ZERO_VEC3, 4);
        wr(at, NiTransform::translation_z, z);
        wr(at, NiTransform::translation_x, x);
        wr(at, NiTransform::translation_y, y);
        wr(at, NiTransform::scale, ONE);
        let mut bound = [0u32; 4];
        bound_ctor(bound.as_mut_ptr() as usize);
        renderer_draw_primitive(this, 0, 0, at, bound.as_mut_ptr() as usize, buffer, 0);
        nullsub_36(bound.as_mut_ptr() as usize);
    }
    device_set_transform(rd(this, R::d3dDevice), D3DTS_VIEW, this + R::d3dViewMat);
    device_set_transform(rd(this, R::d3dDevice), D3DTS_PROJECTION, this + R::d3dProjMat);
}

/// Not compared when it draws: `DrawPrimitive` is game code that another mod changes, and
/// it writes the render state, the texture pipeline and the light manager through the
/// renderer. The regions of that are not known here.
unsafe fn render_screen_poly_regions(set: &mut Regions, this: usize, count: u32) {
    set.add(this, R::SIZE);
    if rd::<u8>(this, R::bSceneOpen) != 0 && count != 0 {
        set.skip();
    }
}

// ---------------------------------------------------------------------------------------
// Precache
// ---------------------------------------------------------------------------------------

/// 0x6AFD90 `NiDX8Renderer::precacheGeometryData`: makes and fills the buffer data of a
/// geometry data that has none. `_extra` is not read. Returns 1 when the geometry data has
/// a buffer data at the end.
#[allow(clippy::too_many_arguments)]
pub unsafe fn precache_geometry_data(this: usize, geom_data: usize, indices: u32, index_count: u32, flags: u32, _extra: usize, shader: usize, shader_data: u32) -> u8 {
    let key: u32 = rd(geom_data, NiGeometryData::uniqueID);
    let map = this + R::hashGeomBuffers;
    let node = hash_find_node(map, key);
    if node != 0 && rd::<u32>(node, THashMap_Node::data) != 0 {
        return 1;
    }
    let mut index_buffer = 0u32;
    let mut vertex_buffer_size = 0u32;
    let mut index_buffer_size = 0u32;
    // The game code does not set these two before the calls.
    let mut fvf = 0u32;
    let mut stride = 0u32;
    let mut vertex_buffer = 0u32;
    if shader != 0 {
        vertex_buffer = shader_build_vertex_buffer(
            shader, geom_data, 0, 0, shader_data, 0, &mut vertex_buffer_size as *mut u32 as usize, D3DPOOL_MANAGED, 8,
            &mut stride as *mut u32 as usize, &mut fvf as *mut u32 as usize, 0,
        );
    }
    if vertex_buffer == 0 {
        vertex_buffer = vbm_pack_untransformed(
            rd(this, R::vertexBufferManager), geom_data, 0, &mut vertex_buffer_size as *mut u32 as usize, 0, 1, D3DPOOL_MANAGED,
            0, &mut stride as *mut u32 as usize, &mut fvf as *mut u32 as usize,
        );
    }
    if indices != 0 {
        index_buffer = ibm_create_index_buffer(
            rd(this, R::indexBufferManager), indices, index_count, 0, &mut index_buffer_size as *mut u32 as usize,
            D3DPOOL_MANAGED, 0,
        );
    }
    if vertex_buffer == 0 || index_buffer == 0 {
        return 0;
    }
    let buffer = operator_new(GB::SIZE);
    wr(buffer, GB::d3dVertexBuffer, vertex_buffer);
    wr(buffer, GB::flags, flags);
    wr(buffer, GB::fvf, fvf);
    wr(buffer, GB::vertexStride, stride);
    wr(buffer, GB::indexBuffer, index_buffer);
    wr(buffer, GB::vertexBufferSize, vertex_buffer_size);
    wr::<u32>(buffer, GB::field_28, 0);
    let active_vertices = vcall0(geom_data, V_GET_ACTIVE_VERTEX_COUNT) & 0xFFFF;
    wr(buffer, GB::verticesCount, active_vertices);
    wr(buffer, GB::indexBufferSize, index_buffer_size);
    wr(buffer, GB::indexBufferEntries, index_count);
    wr(buffer, GB::geomDataRevisionID, rd::<u16>(geom_data, NiGeometryData::revisionID) as u32);
    if is_kind_of(geom_data, RTTI_NI_TRI_SHAPE_DATA) {
        wr(buffer, GB::primitiveType, D3DPT_TRIANGLELIST);
        let triangles = vcall0(geom_data, V_GET_ACTIVE_TRIANGLE_COUNT) & 0xFFFF;
        wr::<u32>(buffer, GB::stripCount, 1);
        wr(buffer, GB::trianglesCount, triangles);
        wr::<u32>(buffer, GB::stripVertexCounts, 0);
    } else if is_kind_of(geom_data, RTTI_NI_TRI_STRIPS_DATA) {
        let entries: u32 = rd(buffer, GB::indexBufferEntries);
        wr(buffer, GB::primitiveType, D3DPT_TRIANGLESTRIP);
        let strips = rd::<u16>(geom_data, NiTriStripsData::numStrips) as u32;
        wr(buffer, GB::stripCount, strips);
        wr(buffer, GB::trianglesCount, entries.wrapping_sub(strips.wrapping_add(strips)));
        wr::<u32>(buffer, GB::stripVertexCounts, rd(geom_data, NiTriStripsData::stripLengths));
    }
    wr(buffer, GB::triList, indices);
    hash_set(map, key, buffer as u32);
    1
}

/// The vertex and index buffers are made on the device (replayed). Not compared with a
/// shader interface: `buildVertexBuffer` can make a vertex shader on the device.
unsafe fn precache_geometry_data_regions(set: &mut Regions, this: usize, geom_data: usize, shader: usize) {
    let mut tally = Tally::new(set);
    tally.add(this, R::SIZE);
    let map = this + R::hashGeomBuffers;
    let node = hash_find_node(map, rd(geom_data, NiGeometryData::uniqueID));
    if node != 0 && rd::<u32>(node, THashMap_Node::data) != 0 {
        return;
    }
    if shader != 0 {
        tally.set.skip();
        return;
    }
    tally.set.allocates();
    tally.buckets(map);
    // A node with no data gets the new pair.
    tally.add(node, THashMap_Node::SIZE);
    tally.managers(this);
}

/// 0x6B0030 `NiDX8Renderer::maybe_PrePackSkinnedGeometryBuffer`: makes the skin partitions
/// when the skin data has none, then the buffer data of each partition up to the first one
/// that has its buffer data. `_property` is not read.
#[allow(clippy::too_many_arguments)]
pub unsafe fn maybe_pre_pack_skinned_geometry_buffer(
    this: usize, geom_data: usize, skin: usize, flags: u32, _property: usize, shader: usize, shader_data: u32, per_partition: u32,
    per_vertex: u32,
) -> u8 {
    if skin == 0 {
        return 0;
    }
    let per_partition = if per_partition != 0 { per_partition } else { rd(this, R::countHWBones) };
    let per_vertex = if per_vertex != 0 { per_vertex } else { rd(this, R::countHWBones) };
    let skin_data: usize = rd(skin, NiSkinInstance::skinData);
    let mut object: usize = if skin_data != 0 { rd(skin_data, SKIN_DATA_SKIN_PARTITION) } else { 0 };
    if object == 0 {
        object = new_skin_partition();
        if skin_partition_make_partitions(object, geom_data, skin_data, per_partition, per_vertex) == 0 {
            return 0;
        }
        assign_pointer(rd::<usize>(skin, NiSkinInstance::skinData) + SKIN_DATA_SKIN_PARTITION, object);
    }
    let partitions: usize = rd(object, NiSkinPartition::partitions);
    let partition_count: u32 = rd(object, NiSkinPartition::partitionCount);
    let no_strips = rd::<u16>(partitions, Partition::numStripLengths) == 0;
    let primitive_type = if no_strips { D3DPT_TRIANGLELIST } else { D3DPT_TRIANGLESTRIP };
    let map = this + R::hashmap_skinPartitionsToGeomBuffers;
    let mut partition = partitions;
    let mut done = 0u32;
    while done < partition_count {
        if rd::<u32>(partition, Partition::bufferData) != 0 {
            break;
        }
        let vertices = rd::<u16>(partition, Partition::numVertices) as u32;
        let mut vertex_buffer_size = 0u32;
        let mut index_buffer_size = 0u32;
        // The game code does not set these two before the calls.
        let mut fvf = 0u32;
        let mut stride = 0u32;
        let mut vertex_buffer = 0u32;
        if shader != 0 {
            vertex_buffer = shader_build_vertex_buffer(
                shader, geom_data, skin, partition, shader_data, 0, &mut vertex_buffer_size as *mut u32 as usize, D3DPOOL_MANAGED,
                8, &mut stride as *mut u32 as usize, &mut fvf as *mut u32 as usize, 0,
            );
        }
        if vertex_buffer == 0 {
            vertex_buffer = vbm_pack_skinned(
                rd(this, R::vertexBufferManager), geom_data, skin, partition, 0, &mut vertex_buffer_size as *mut u32 as usize,
                D3DPOOL_MANAGED, 0, &mut stride as *mut u32 as usize, &mut fvf as *mut u32 as usize,
            );
        }
        let buffer = operator_new(GB::SIZE);
        let tri_list: u32 = rd(partition, Partition::triangles);
        let mut entries = 0u32;
        let mut strip_lengths = 0u32;
        let strips;
        if no_strips {
            entries = (rd::<u16>(partition, Partition::numTriangles) as u32).wrapping_mul(3);
            strips = 1;
        } else {
            let count = rd::<u16>(partition, Partition::numStripLengths) as usize;
            let lengths: usize = rd(partition, Partition::stripLengths);
            for index in 0..count {
                entries = entries.wrapping_add(rd::<u16>(lengths, index * 2) as u32);
            }
            strips = rd::<u16>(partition, Partition::numStripLengths) as u32;
            strip_lengths = rd(partition, Partition::stripLengths);
        }
        let index_buffer = ibm_create_index_buffer(
            rd(this, R::indexBufferManager), tri_list, entries, 0, &mut index_buffer_size as *mut u32 as usize, D3DPOOL_MANAGED, 0,
        );
        wr(buffer, GB::primitiveType, primitive_type);
        wr(buffer, GB::stripCount, strips);
        wr(buffer, GB::stripVertexCounts, strip_lengths);
        wr(buffer, GB::triList, tri_list);
        wr(buffer, GB::d3dVertexBuffer, vertex_buffer);
        wr(buffer, GB::flags, flags);
        wr(buffer, GB::fvf, fvf);
        wr(buffer, GB::vertexStride, stride);
        wr(buffer, GB::vertexBufferSize, vertex_buffer_size);
        wr(buffer, GB::indexBuffer, index_buffer);
        wr::<u32>(buffer, GB::field_28, 0);
        wr(buffer, GB::verticesCount, vertices);
        wr(buffer, GB::indexBufferSize, index_buffer_size);
        wr(buffer, GB::indexBufferEntries, entries);
        wr(buffer, GB::geomDataRevisionID, rd::<u16>(geom_data, NiGeometryData::revisionID) as u32);
        wr(buffer, GB::trianglesCount, rd::<u16>(partition, Partition::numTriangles) as u32);
        wr(partition, Partition::bufferData, buffer);
        hash_set(map, partition as u32, buffer as u32);
        done += 1;
        partition += Partition::SIZE;
    }
    1
}

/// The regions of a function that works on the partitions of a skin instance: the map, the
/// partitions, the skin data (it gets new partitions when it has none; `MakePartitions`
/// writes only blocks it allocates).
unsafe fn skin_regions(tally: &mut Tally, this: usize, skin: usize) {
    tally.set.allocates();
    tally.buckets(this + R::hashmap_skinPartitionsToGeomBuffers);
    tally.managers(this);
    let skin_data: usize = rd(skin, NiSkinInstance::skinData);
    if skin_data == 0 {
        return;
    }
    tally.add(skin_data, SKIN_DATA_SKIN_PARTITION + 4);
    let object: usize = rd(skin_data, SKIN_DATA_SKIN_PARTITION);
    if object != 0 {
        tally.add(rd(object, NiSkinPartition::partitions), rd::<usize>(object, NiSkinPartition::partitionCount) * Partition::SIZE);
    }
}

/// Not compared with a shader interface: `buildVertexBuffer` can make a vertex shader on
/// the device.
unsafe fn maybe_pre_pack_skinned_geometry_buffer_regions(set: &mut Regions, this: usize, skin: usize, shader: usize) {
    let mut tally = Tally::new(set);
    tally.add(this, R::SIZE);
    if skin == 0 {
        return;
    }
    if shader != 0 {
        tally.set.skip();
        return;
    }
    skin_regions(&mut tally, this, skin);
}

/// The shader interface and its data in a renderer specific property, or (0, 0).
unsafe fn shader_of(property: usize) -> (usize, u32) {
    if property != 0 {
        let data: usize = rd(property, NiRendererSpecificProperty::renderData);
        if data != 0 && vcall0(data, V_GET_RENDERER_TYPE_ID) == 7 {
            let data: usize = rd(property, NiRendererSpecificProperty::renderData);
            if data != 0 {
                return (rd(data, RenderData7::field_C), rd(data, RenderData7::field_8));
            }
        }
    }
    (0, 0)
}

/// 0x6AC420 `NiDX8Renderer::PrecacheGeometry`
pub unsafe fn precache_geometry(this: usize, geometry: usize, flags: u32, property: usize, per_partition: u32, per_vertex: u32) -> u8 {
    if geometry == 0 {
        return 0;
    }
    let skin: usize = rd(geometry, NiGeometry::skinInstance);
    let geom_data: usize = rd(geometry, NiGeometry::spModelData);
    let (shader, shader_data) = shader_of(property);
    let result = if skin != 0 {
        maybe_pre_pack_skinned_geometry_buffer(this, geom_data, skin, flags, property, shader, shader_data, per_partition, per_vertex)
    } else if vcall0(geometry, V_GET_RTTI) as usize == RTTI_NI_TRI_SHAPE {
        let length: u32 = rd(geom_data, NiTriShapeData::uiTriListLength);
        let tri_list = vcall0(geom_data, V_GET_TRI_LIST);
        precache_geometry_data(this, geom_data, tri_list, length, flags, property, shader, shader_data)
    } else if vcall0(geometry, V_GET_RTTI) as usize == RTTI_NI_TRI_STRIPS {
        let strip_lists: u32 = rd(geom_data, NiTriStripsData::stripLists);
        let length = tri_strips_data_strip_length_sum(geom_data) & 0xFFFF;
        precache_geometry_data(this, geom_data, strip_lists, length, flags, property, shader, shader_data)
    } else {
        return 0;
    };
    (result != 0) as u8
}

unsafe fn precache_geometry_regions(set: &mut Regions, this: usize, geometry: usize, property: usize) {
    if geometry == 0 {
        return;
    }
    let skin: usize = rd(geometry, NiGeometry::skinInstance);
    let (shader, _) = shader_of(property);
    if skin != 0 {
        maybe_pre_pack_skinned_geometry_buffer_regions(set, this, skin, shader);
    } else {
        let rtti = vcall0(geometry, V_GET_RTTI) as usize;
        if rtti == RTTI_NI_TRI_SHAPE || rtti == RTTI_NI_TRI_STRIPS {
            precache_geometry_data_regions(set, this, rd(geometry, NiGeometry::spModelData), shader);
        }
    }
}

// ---------------------------------------------------------------------------------------
// The batch list
// ---------------------------------------------------------------------------------------

/// Puts a new `BatchedObject` at the end of the batch list. `values` has the order in which
/// the game code writes the seven members.
unsafe fn append_batched(this: usize, values: [(usize, u32); 7]) {
    let object = operator_new(Batched::SIZE);
    for (offset, value) in values {
        wr(object, offset, value);
    }
    let tail: usize = rd(this, R::batchedObjectListTail);
    if tail != 0 {
        wr(tail, Batched::nextBatch, object);
    } else {
        wr(this, R::batchedObjectListHead, object);
    }
    wr(this, R::batchedObjectListTail, object);
}

/// The skinned half of `BatchRenderShape` (from 0x6AE65B) and `BatchRenderStrips` (from
/// 0x6AE919), which are the same code: one batched object for each skin partition.
unsafe fn batch_skinned(this: usize, geom_data: usize, skin: usize, transform: usize, bound: usize) {
    let skin_data: usize = rd(skin, NiSkinInstance::skinData);
    let mut object: usize = if skin_data != 0 { rd(skin_data, SKIN_DATA_SKIN_PARTITION) } else { 0 };
    if object == 0 {
        object = new_skin_partition();
        // `mov al, [countHWBones]; push eax; push eax` with the new object in EAX.
        let bones = (object as u32 & !0xFF) | rd::<u8>(this, R::countHWBones) as u32;
        if skin_partition_make_partitions(object, geom_data, skin_data, bones, bones) != 0 {
            assign_pointer(rd::<usize>(skin, NiSkinInstance::skinData) + SKIN_DATA_SKIN_PARTITION, object);
        }
    }
    let partitions: usize = rd(object, NiSkinPartition::partitions);
    let no_strips = rd::<u16>(partitions, Partition::numStripLengths) == 0;
    let primitive_type = if no_strips { D3DPT_TRIANGLELIST } else { D3DPT_TRIANGLESTRIP };
    let partition_count: u32 = rd(object, NiSkinPartition::partitionCount);
    let mut partition = partitions;
    for _ in 0..partition_count {
        let mut buffer = 0u32;
        let out = &mut buffer as *mut u32 as usize;
        let mut strip_lengths = 0u32;
        let tri_list: u32 = rd(partition, Partition::triangles);
        let mut strips = 1u32;
        if !no_strips {
            strips = rd::<u16>(partition, Partition::numStripLengths) as u32;
            strip_lengths = rd(partition, Partition::stripLengths);
        }
        if find_cached_partition_geom_buffer(this, out, geom_data, skin, partition) != 0 {
            wr(rd::<usize>(out, 0), GB::primitiveType, primitive_type);
            wr(rd::<usize>(out, 0), GB::trianglesCount, rd::<u16>(partition, Partition::numTriangles) as u32);
            wr(rd::<usize>(out, 0), GB::stripCount, strips);
            wr(rd::<usize>(out, 0), GB::stripVertexCounts, strip_lengths);
            wr(rd::<usize>(out, 0), GB::triList, tri_list);
            create_partition_geom_buffer(this, out, geom_data, skin, partition);
        }
        append_batched(
            this,
            [
                (Batched::skinInstance, skin as u32),
                (Batched::geomData, geom_data as u32),
                (Batched::transform, transform as u32),
                (Batched::worldBound, bound as u32),
                (Batched::partition, partition as u32),
                (Batched::nextBatch, 0),
                (Batched::geomBuffer, rd(out, 0)),
            ],
        );
        partition += Partition::SIZE;
    }
}

/// True when the two batch functions take the skinned half.
unsafe fn batches_skinned(this: usize, skin: usize) -> bool {
    skin != 0 && rd::<u32>(this, R::countHWBones) >= 4
}

/// 0x6AE600 `NiDX8Renderer::BatchRenderShape`
pub unsafe fn batch_render_shape(this: usize, geom_data: usize, skin: usize, transform: usize, bound: usize) {
    if rd::<u8>(this, R::bSceneOpen) == 0 {
        return;
    }
    if vcall0(geom_data, V_GET_ACTIVE_VERTEX_COUNT) as u16 == 0 {
        return;
    }
    if batches_skinned(this, skin) {
        batch_skinned(this, geom_data, skin, transform, bound);
        return;
    }
    let mut buffer = 0u32;
    let out = &mut buffer as *mut u32 as usize;
    if create_geometry_buffer_data(this, out, geom_data) != 0 {
        wr(rd::<usize>(out, 0), GB::primitiveType, D3DPT_TRIANGLELIST);
        let triangles = vcall0(geom_data, V_GET_ACTIVE_TRIANGLE_COUNT) & 0xFFFF;
        wr(rd::<usize>(out, 0), GB::trianglesCount, triangles);
        wr::<u32>(rd::<usize>(out, 0), GB::stripCount, 1);
        wr::<u32>(rd::<usize>(out, 0), GB::stripVertexCounts, 0);
        let tri_list = vcall0(geom_data, V_GET_TRI_LIST);
        wr(rd::<usize>(out, 0), GB::triList, tri_list);
    }
    append_batched(
        this,
        [
            (Batched::geomData, geom_data as u32),
            (Batched::skinInstance, skin as u32),
            (Batched::transform, transform as u32),
            (Batched::worldBound, bound as u32),
            (Batched::partition, 0),
            (Batched::geomBuffer, rd(out, 0)),
            (Batched::nextBatch, 0),
        ],
    );
}

/// 0x6AE8C0 `NiDX8Renderer::BatchRenderStrips`
pub unsafe fn batch_render_strips(this: usize, geom_data: usize, skin: usize, transform: usize, bound: usize) {
    if rd::<u8>(this, R::bSceneOpen) == 0 {
        return;
    }
    if vcall0(geom_data, V_GET_ACTIVE_VERTEX_COUNT) as u16 == 0 {
        return;
    }
    if batches_skinned(this, skin) {
        batch_skinned(this, geom_data, skin, transform, bound);
        return;
    }
    let mut buffer = 0u32;
    let out = &mut buffer as *mut u32 as usize;
    if create_geometry_buffer_data(this, out, geom_data) != 0 {
        wr(rd::<usize>(out, 0), GB::primitiveType, D3DPT_TRIANGLESTRIP);
        wr(rd::<usize>(out, 0), GB::stripCount, rd::<u16>(geom_data, NiTriStripsData::numStrips) as u32);
        let sum = tri_strips_data_strip_length_sum(geom_data) & 0xFFFF;
        let strips: u32 = rd(rd::<usize>(out, 0), GB::stripCount);
        wr(rd::<usize>(out, 0), GB::trianglesCount, sum.wrapping_sub(strips.wrapping_add(strips)));
        wr::<u32>(rd::<usize>(out, 0), GB::stripVertexCounts, rd(geom_data, NiTriStripsData::stripLengths));
        wr::<u32>(rd::<usize>(out, 0), GB::triList, rd(geom_data, NiTriStripsData::stripLists));
    }
    append_batched(
        this,
        [
            (Batched::worldBound, bound as u32),
            (Batched::geomData, geom_data as u32),
            (Batched::skinInstance, skin as u32),
            (Batched::transform, transform as u32),
            (Batched::partition, 0),
            (Batched::nextBatch, 0),
            (Batched::geomBuffer, rd(out, 0)),
        ],
    );
}

/// The renderer (the list head and tail, the two maps), the last object of the list, the
/// bucket array of the map, and for a skin what `skin_regions` says. Every buffer data the
/// call packs is new, so no device buffer is released.
unsafe fn batch_regions(set: &mut Regions, this: usize, geom_data: usize, skin: usize) {
    let mut tally = Tally::new(set);
    tally.add(this, R::SIZE);
    if rd::<u8>(this, R::bSceneOpen) == 0 || vcall0(geom_data, V_GET_ACTIVE_VERTEX_COUNT) as u16 == 0 {
        return;
    }
    tally.add(rd(this, R::batchedObjectListTail), Batched::SIZE);
    if batches_skinned(this, skin) {
        skin_regions(&mut tally, this, skin);
    } else {
        tally.set.allocates();
        tally.buckets(this + R::hashGeomBuffers);
    }
}

// ---------------------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------------------

port_thiscall!(precache_geometry_port, 0x6AC420, "NiDX8Renderer::PrecacheGeometry",
    fn(this, geometry: usize, flags: u32, property: usize, per_partition: u32, per_vertex: u32) -> u8,
    compare_eax = true,
    regions = |set| precache_geometry_regions(set, this, geometry, property),
    rust = precache_geometry);

port_thiscall!(render_screen_poly_port, 0x6ADD20, "NiDX8Renderer::RenderScreenPoly",
    fn(this, count: u32, vertices: usize, tex_coords: usize, colour: usize) -> (),
    compare_eax = false,
    regions = |set| render_screen_poly_regions(set, this, count),
    rust = render_screen_poly);

port_thiscall!(batch_render_shape_port, 0x6AE600, "NiDX8Renderer::BatchRenderShape",
    fn(this, geom_data: usize, skin: usize, transform: usize, bound: usize) -> (),
    compare_eax = false,
    regions = |set| batch_regions(set, this, geom_data, skin),
    rust = batch_render_shape);

port_thiscall!(batch_render_strips_port, 0x6AE8C0, "NiDX8Renderer::BatchRenderStrips",
    fn(this, geom_data: usize, skin: usize, transform: usize, bound: usize) -> (),
    compare_eax = false,
    regions = |set| batch_regions(set, this, geom_data, skin),
    rust = batch_render_strips);

port_thiscall!(purge_skin_partition_port, 0x6AF2D0, "NiDX8Renderer::PurgeSkinPartition",
    fn(this, partition: usize) -> (),
    compare_eax = false,
    regions = |set| purge_skin_partition_regions(set, this, partition),
    rust = purge_skin_partition);

port_thiscall!(purge_geometry_data_port, 0x6AF3A0, "NiDX8Renderer::PurgeGeometryData",
    fn(this, geom_data: usize) -> (),
    compare_eax = false,
    regions = |set| purge_geometry_data_regions(set, this, geom_data),
    rust = purge_geometry_data);

port_thiscall!(create_geometry_buffer_data_port, 0x6AF4A0, "NiDX8Renderer::createGeometryBufferData",
    fn(this, out: usize, geom_data: usize) -> u8,
    compare_eax = true,
    regions = |set| create_geometry_buffer_data_regions(set, this, out, geom_data),
    rust = create_geometry_buffer_data);

port_thiscall!(pack_geometry_buffers_port, 0x6AF610, "NiDX8Renderer::packGeometryBuffers",
    fn(this, out: usize, geom_data: usize, skin: usize, shader: usize, shader_data: u32) -> u8,
    compare_eax = true,
    regions = |set| pack_geometry_buffers_regions(set, this, out, geom_data, skin, shader),
    rust = pack_geometry_buffers);

port_thiscall!(find_cached_partition_geom_buffer_port, 0x6AF930, "NiDX8Renderer::findCachedPartitionGeomBuffer",
    fn(this, out: usize, geom_data: usize, skin: usize, partition: usize) -> u8,
    compare_eax = true,
    regions = |set| find_cached_partition_geom_buffer_regions(set, this, out, geom_data, skin, partition),
    rust = find_cached_partition_geom_buffer);

port_thiscall!(create_partition_geom_buffer_port, 0x6AFAB0, "NiDX8Renderer::createPartitionGeomBuffer",
    fn(this, out: usize, geom_data: usize, skin: usize, partition: usize) -> u8,
    compare_eax = true,
    regions = |set| create_partition_geom_buffer_regions(set, this, out, geom_data, skin, partition),
    rust = create_partition_geom_buffer);

port_thiscall!(pack_screen_poly_geom_buffer_port, 0x6AFCC0, "NiDX8Renderer::PackScreenPolyGeomBuffer",
    fn(this, count: u32, vertices: usize, tex_coords: usize, colour: usize, near: u32, distance: u32) -> usize,
    compare_eax = true,
    regions = |set| pack_screen_poly_geom_buffer_regions(set, this, count, tex_coords, colour),
    rust = pack_screen_poly_geom_buffer);

port_thiscall!(precache_geometry_data_port, 0x6AFD90, "NiDX8Renderer::precacheGeometryData",
    fn(this, geom_data: usize, indices: u32, index_count: u32, flags: u32, extra: usize, shader: usize, shader_data: u32) -> u8,
    compare_eax = true,
    regions = |set| precache_geometry_data_regions(set, this, geom_data, shader),
    rust = precache_geometry_data);

port_thiscall!(maybe_pre_pack_skinned_geometry_buffer_port, 0x6B0030, "NiDX8Renderer::maybe_PrePackSkinnedGeometryBuffer",
    fn(this, geom_data: usize, skin: usize, flags: u32, property: usize, shader: usize, shader_data: u32, per_partition: u32, per_vertex: u32) -> u8,
    compare_eax = true,
    regions = |set| maybe_pre_pack_skinned_geometry_buffer_regions(set, this, skin, shader),
    rust = maybe_pre_pack_skinned_geometry_buffer);

pub static PORTS: &[&crate::shadow::PortState] = &[
    &precache_geometry_port::STATE,
    &render_screen_poly_port::STATE,
    &batch_render_shape_port::STATE,
    &batch_render_strips_port::STATE,
    &purge_skin_partition_port::STATE,
    &purge_geometry_data_port::STATE,
    &create_geometry_buffer_data_port::STATE,
    &pack_geometry_buffers_port::STATE,
    &find_cached_partition_geom_buffer_port::STATE,
    &create_partition_geom_buffer_port::STATE,
    &pack_screen_poly_geom_buffer_port::STATE,
    &precache_geometry_data_port::STATE,
    &maybe_pre_pack_skinned_geometry_buffer_port::STATE,
];

#[cfg(test)]
mod tests {
    use super::*;

    // A hash map whose key is a number: the bucket is the key modulo the bucket count.
    unsafe extern "thiscall" fn fake_hash_key(map: usize, key: u32) -> u32 {
        key % rd::<u32>(map, THashMap::bucketCount)
    }
    unsafe extern "thiscall" fn fake_cmp_key(_map: usize, a: u32, b: u32) -> u32 {
        // Only AL is the result.
        0xABCD_EF00 | (a == b) as u32
    }

    struct Map {
        vtable: Box<[usize; 5]>,
        buckets: Box<[u32; 4]>,
        nodes: Vec<Box<[u32; 3]>>,
    }

    impl Map {
        fn new() -> Map {
            Map {
                vtable: Box::new([0, fake_hash_key as *const () as usize, fake_cmp_key as *const () as usize, 0, 0]),
                buckets: Box::new([0; 4]),
                nodes: Vec::new(),
            }
        }

        /// Puts the map at `at` and gives it the pairs, each at the head of its bucket.
        unsafe fn install(&mut self, at: usize, pairs: &[(u32, u32)]) {
            wr(at, THashMap::vtbl, self.vtable.as_ptr() as usize);
            wr::<u32>(at, THashMap::count, pairs.len() as u32);
            wr::<u32>(at, THashMap::bucketCount, 4);
            wr(at, THashMap::arrayBuckets, self.buckets.as_ptr() as usize);
            for &(key, value) in pairs {
                let bucket = (key % 4) as usize;
                let node = Box::new([key, value, self.buckets[bucket]]);
                self.buckets[bucket] = node.as_ptr() as u32;
                self.nodes.push(node);
            }
        }
    }

    fn renderer() -> Vec<u8> {
        vec![0u8; R::SIZE]
    }

    #[test]
    fn hash_find_walks_the_bucket() {
        let mut block = [0u8; THashMap::SIZE];
        let at = block.as_mut_ptr() as usize;
        let mut map = Map::new();
        unsafe {
            map.install(at, &[(1, 100), (5, 500), (9, 900), (2, 200)]);
            assert_eq!(rd::<u32>(hash_find_node(at, 5), THashMap_Node::data), 500);
            assert_eq!(rd::<u32>(hash_find_node(at, 1), THashMap_Node::data), 100);
            assert_eq!(rd::<u32>(hash_find_node(at, 2), THashMap_Node::data), 200);
            assert_eq!(hash_find_node(at, 13), 0);
            assert_eq!(hash_find_node(at, 3), 0);
        }
    }

    #[test]
    fn hash_unlink_takes_the_head_or_a_later_node() {
        let mut block = [0u8; THashMap::SIZE];
        let at = block.as_mut_ptr() as usize;
        let mut map = Map::new();
        unsafe {
            // Bucket 1 is 9 -> 5 -> 1.
            map.install(at, &[(1, 100), (5, 500), (9, 900)]);
            let node5 = hash_find_node(at, 5);
            assert_eq!(hash_unlink(at, 5), node5);
            assert_eq!(hash_find_node(at, 5), 0);
            assert_ne!(hash_find_node(at, 1), 0);
            let node9 = hash_find_node(at, 9);
            assert_eq!(hash_unlink(at, 9), node9);
            assert_eq!(map.buckets[1] as usize, hash_find_node(at, 1));
            assert_eq!(hash_unlink(at, 13), 0);
            assert_eq!(hash_unlink(at, 2), 0);
        }
    }

    #[test]
    fn create_geometry_buffer_data_gives_the_cached_one() {
        let mut this = renderer();
        let base = this.as_mut_ptr() as usize;
        let mut map = Map::new();
        let mut geom_data = [0u8; NiGeometryData::SIZE];
        let mut out = 0x1111u32;
        unsafe {
            map.install(base + R::hashGeomBuffers, &[(7, 0x7000), (11, 0xB000)]);
            wr::<u32>(geom_data.as_mut_ptr() as usize, NiGeometryData::uniqueID, 11);
            let made = create_geometry_buffer_data(base, &mut out as *mut u32 as usize, geom_data.as_ptr() as usize);
            assert_eq!(made, 0);
            assert_eq!(out, 0xB000);
            out = 0x1111;
            assert_eq!(create_geometry_buffer_data(base, &mut out as *mut u32 as usize, 0), 0);
            assert_eq!(out, 0x1111);
        }
    }

    #[test]
    fn find_cached_partition_geom_buffer_without_a_new_one() {
        let mut this = renderer();
        let base = this.as_mut_ptr() as usize;
        let mut map = Map::new();
        let mut partition = [0u8; Partition::SIZE];
        let key = partition.as_mut_ptr() as usize;
        let mut out = 0u32;
        let at = &mut out as *mut u32 as usize;
        unsafe {
            map.install(base + R::hashmap_skinPartitionsToGeomBuffers, &[(key as u32, 0xC000)]);
            assert_eq!(find_cached_partition_geom_buffer(base, at, 0, 1, key), 0);
            assert_eq!(find_cached_partition_geom_buffer(base, at, 1, 0, key), 0);
            assert_eq!(out, 0);
            assert_eq!(find_cached_partition_geom_buffer(base, at, 1, 1, key), 0);
            assert_eq!(out, 0xC000);
            // No entry, but the caller has a value: nothing is made.
            out = 0x55;
            assert_eq!(find_cached_partition_geom_buffer(base, at, 1, 1, key + 1), 0);
            assert_eq!(out, 0x55);
        }
    }

    #[test]
    fn index_entries_for_lists_and_strips() {
        let mut buffer = [0u8; GB::SIZE];
        let at = buffer.as_mut_ptr() as usize;
        unsafe {
            wr::<u32>(at, GB::trianglesCount, 10);
            wr::<u32>(at, GB::stripCount, 3);
            wr::<u32>(at, GB::primitiveType, D3DPT_TRIANGLELIST);
            assert_eq!(index_entries_of(at), 30);
            wr::<u32>(at, GB::primitiveType, D3DPT_TRIANGLESTRIP);
            assert_eq!(index_entries_of(at), 16);
        }
    }

    #[test]
    fn pack_functions_leave_early() {
        let mut this = renderer();
        let base = this.as_mut_ptr() as usize;
        let mut buffer = [0u8; GB::SIZE];
        let at = buffer.as_mut_ptr() as usize;
        let mut out = 0u32;
        let slot = &mut out as *mut u32 as usize;
        unsafe {
            // No buffer data.
            assert_eq!(pack_geometry_buffers(base, slot, 1, 0, 0, 0), 0);
            assert_eq!(create_partition_geom_buffer(base, slot, 1, 1, 1), 0);
            wr(slot, 0, at as u32);
            assert_eq!(pack_geometry_buffers(base, slot, 0, 0, 0, 0), 0);
            assert_eq!(create_partition_geom_buffer(base, slot, 0, 1, 1), 0);
            assert_eq!(create_partition_geom_buffer(base, slot, 1, 0, 1), 0);
            // A vertex buffer and flag 1: the buffers stay as they are.
            wr::<u32>(at, GB::d3dVertexBuffer, 0x1234);
            wr::<u32>(at, GB::flags, 0x101);
            let before = buffer;
            assert_eq!(pack_geometry_buffers(base, slot, 1, 0, 0, 0), 1);
            assert_eq!(create_partition_geom_buffer(base, slot, 1, 1, 1), 1);
            assert_eq!(buffer, before);
        }
    }

    #[test]
    fn precache_geometry_data_with_a_buffer_data() {
        let mut this = renderer();
        let base = this.as_mut_ptr() as usize;
        let mut map = Map::new();
        let mut geom_data = [0u8; NiGeometryData::SIZE];
        unsafe {
            map.install(base + R::hashGeomBuffers, &[(6, 0x6000)]);
            wr::<u32>(geom_data.as_mut_ptr() as usize, NiGeometryData::uniqueID, 6);
            assert_eq!(precache_geometry_data(base, geom_data.as_ptr() as usize, 0, 0, 0, 0, 0, 0), 1);
            assert_eq!(precache_geometry(base, 0, 0, 0, 0, 0), 0);
            assert_eq!(maybe_pre_pack_skinned_geometry_buffer(base, 1, 0, 0, 0, 0, 0, 0, 0), 0);
        }
    }

    unsafe extern "thiscall" fn fake_type_id_7(_data: usize) -> u32 {
        7
    }
    unsafe extern "thiscall" fn fake_type_id_8(_data: usize) -> u32 {
        8
    }

    #[test]
    fn shader_of_reads_only_render_data_of_type_7() {
        let mut property = [0u8; NiRendererSpecificProperty::SIZE];
        let mut data = [0u8; RenderData7::SIZE];
        let vtable7 = [0usize, fake_type_id_7 as *const () as usize];
        let vtable8 = [0usize, fake_type_id_8 as *const () as usize];
        let at = property.as_mut_ptr() as usize;
        let data_at = data.as_mut_ptr() as usize;
        unsafe {
            assert_eq!(shader_of(0), (0, 0));
            assert_eq!(shader_of(at), (0, 0));
            wr(at, NiRendererSpecificProperty::renderData, data_at);
            wr::<u32>(data_at, RenderData7::field_8, 0x88);
            wr::<u32>(data_at, RenderData7::field_C, 0xCC);
            wr(data_at, 0, vtable8.as_ptr() as usize);
            assert_eq!(shader_of(at), (0, 0));
            wr(data_at, 0, vtable7.as_ptr() as usize);
            assert_eq!(shader_of(at), (0xCC, 0x88));
        }
    }

    #[test]
    fn batch_and_screen_poly_do_nothing_outside_a_scene() {
        let mut this = renderer();
        let base = this.as_mut_ptr() as usize;
        let before = this.clone();
        unsafe {
            batch_render_shape(base, 1, 0, 0, 0);
            batch_render_strips(base, 1, 0, 0, 0);
            render_screen_poly(base, 4, 0, 0, 0);
            wr::<u8>(base, R::bSceneOpen, 1);
            render_screen_poly(base, 0, 0, 0, 0);
            wr::<u8>(base, R::bSceneOpen, 0);
        }
        assert_eq!(this, before);
    }

    #[test]
    fn tally_skips_past_eight_regions() {
        let mut set = Regions::new();
        let mut tally = Tally::new(&mut set);
        for index in 0..8 {
            tally.add(0x1000 + index * 0x10, 4);
        }
        tally.add(0, 4);
        tally.add(0x2000, 0);
        assert_eq!(tally.used, 8);
        tally.add(0x3000, 4);
        assert_eq!(tally.used, 9);
        assert_eq!(set.as_slice().len(), 8);
    }
}
