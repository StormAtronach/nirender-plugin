//! The geometry of a draw as a record of its own, for the retained pipeline.
//!
//! This module packs the geometry data of a shape as the engine packs it (positions,
//! normals, colours, the texture sets, then the triangle list or the strip lists). It sends
//! it once as a `GEOMETRY` record keyed by the geometry data. The key has the unique id, the
//! revision and the active counts of the data: the engine packs a geometry again only when
//! the revision changes (`packGeometryBuffers` 0x6AF610, by its port in `ports/geometry.rs`),
//! so these are its version.
//!
//! A skinned shape is packed with the weights and the bone indexes of each vertex for the
//! whole skin, so one draw with the palette of the skin replaces the draws of the
//! partitions. The particles of a system come in the shape record itself. Lines are packed
//! as `RenderLines` packs them, and a screen polygon as `PackScreenPolyVB` packs it, at each
//! draw.

use std::collections::HashMap;

use crate::main_thread::{MainCell, MainThread};
use crate::d3d::{D3DFVF_DIFFUSE, D3DFVF_NORMAL, D3DFVF_TEX1, D3DFVF_XYZ};
use crate::engine::vcall0;
use crate::gen::layouts::{NiGeometryData as GD, NiSkinInstance, NiSkinPartition, NiSkinPartition_Partition as P, NiTriStripsData as TS};
use crate::link::{DRAW_LINES, DRAW_SHAPE, DRAW_STRIPS};
use crate::mem::rd;
use crate::ports::ni;

/// `NiLinesData::lineSegmentFlags`: one byte for each vertex, not 0 when a segment goes from
/// the vertex to the next one.
pub const LINES_SEGMENT_FLAGS: usize = 0x34;

/// [`D3DFVF_XYZB4`] and [`D3DFVF_LASTBETA_UBYTE4`]: a position, three weights and four bone
/// indexes in a byte each. The format of a skinned vertex.
const D3DFVF_XYZB4: u32 = 0x00C;
const D3DFVF_LASTBETA_UBYTE4: u32 = 0x1000;
/// `NiSkinData::skinPartition`
const SKIN_DATA_SKIN_PARTITION: usize = 8;

/// `NiRTTI` of `NiTriShapeData` and of `NiTriStripsData`.
const RTTI_NI_TRI_SHAPE_DATA: usize = 0x7DE490;
const RTTI_NI_TRI_STRIPS_DATA: usize = 0x7DE588;
/// `NiGeometryData::GetActiveVertexCount`, `NiTriBasedGeomData::GetActiveTriangleCount`,
/// `NiTriBasedGeomData::GetTriList_const`.
const V_GET_ACTIVE_VERTEX_COUNT: usize = 0x30;
const V_GET_ACTIVE_TRIANGLE_COUNT: usize = 0x3C;
const V_GET_TRI_LIST: usize = 0x44;

/// The version of a geometry data that was sent.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Version {
    unique: u32,
    revision: u16,
    vertices: u32,
    entries: u32,
    /// The skin data of a skinned shape: its partitions give the weights.
    skin_data: u32,
}

/// A geometry that was sent: its version, and the frame of its last draw.
struct Sent {
    version: Version,
    used: u32,
}

/// The geometries sent, by the address of their data. The engine frees a geometry data with
/// no event to the plugin, so an entry of a geometry not drawn for [`FORGET_AFTER`] frames goes
/// (`prune`): a map kept for the session grew with every address the game ever used (the leak
/// hunt of 2026-10-23, UMDH: 819 KB after 35 minutes of a loop). A geometry still alive is
/// sent again at its next draw.
static VERSIONS: MainCell<Option<HashMap<u32, Sent>>> = MainCell::new(None);
const FORGET_AFTER: u32 = 36_000;
const PRUNE_EVERY: u32 = 4_096;
static LAST_PRUNE: MainCell<u32> = MainCell::new(0);
static FORGOTTEN: MainCell<u64> = MainCell::new(0);
/// The last geometry whose skin gave no bone influences (`skin_influences`), for the count of
/// the draws without a record: such a geometry is not empty.
static NO_INFLUENCES: MainCell<usize> = MainCell::new(0);
static RECORDS: MainCell<u64> = MainCell::new(0);
static BYTES: MainCell<u64> = MainCell::new(0);

fn versions<'a>(t: &'a MainThread) -> &'a mut HashMap<u32, Sent> {
    (*VERSIONS.get(t)).get_or_insert_with(HashMap::new)
}

/// True when `handle` was sent with `version`: the draw sends nothing. Marks it used now.
fn known(t: &MainThread, handle: u32, version: Version) -> bool {
    let now = crate::camera_space::frame(t);
    prune(t, now);
    match versions(t).get_mut(&handle) {
        Some(sent) if sent.version == version => {
            sent.used = now;
            true
        }
        _ => false,
    }
}

/// Forgets the geometries not drawn for [`FORGET_AFTER`] frames, once in [`PRUNE_EVERY`] frames.
fn prune(t: &MainThread, now: u32) {
    let last = LAST_PRUNE.get(t);
    if now.wrapping_sub(*last) < PRUNE_EVERY {
        return;
    }
    *last = now;
    let map = versions(t);
    let before = map.len();
    map.retain(|_, sent| now.wrapping_sub(sent.used) < FORGET_AFTER);
    *FORGOTTEN.get(t) += (before - map.len()) as u64;
}

/// Forgets the versions: the next draw of each geometry sends it again. For the start of a
/// capture file from the stream, which has no geometry records before it.
pub unsafe fn reset(t: &MainThread) {
    versions(t).clear();
}

/// Forgets these geometries: the next draw of each sends it again.
pub unsafe fn forget(t: &MainThread, handles: &[u32]) {
    for handle in handles {
        versions(t).remove(handle);
    }
}

/// One line for the log: the records and the bytes that were sent, and the counts of the
/// bone influences of the skins.
pub unsafe fn report(t: &MainThread) -> String {
    let (records, bytes) = (*RECORDS.get(t), *BYTES.get(t));
    let s = &*SKIN_COUNTS.get(t);
    format!(
        "geometry: {} records, {:.1} MB, {} geometries known, {} forgotten after {} frames without a draw; skins: {} from the skin data, {} from the partitions ({} without bone data); vertices {}, with more than 4 bones {} (the most {}); triangles {}, with more than 4 bones {}; vertices compared with the partitions {}, different {}",
        records,
        bytes as f64 / 1e6,
        versions(t).len(),
        *FORGOTTEN.get(t),
        FORGET_AFTER,
        s.from_data,
        s.from_partitions,
        s.without_bone_data,
        s.vertices,
        s.over_four,
        s.most,
        s.triangles,
        s.triangles_over_four,
        s.compared,
        s.different
    )
}

/// Where the bone influences of a skinned geometry come from: false for the weights of the
/// bones in the skin data (`NiSkinData`), true for the skin partitions of the game. A skin
/// data without its weights always takes the partitions. Only `NiSkinData::dtor` frees the
/// weights: the partitions do not.
static SKIN_FROM_PARTITIONS: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Sets the source of the bone influences (true: the skin partitions) and forgets the
/// geometries, so that each is packed again. Returns the source before.
pub unsafe fn set_skin_from_partitions(t: &MainThread, partitions: bool) -> bool {
    let before = SKIN_FROM_PARTITIONS.swap(partitions, core::sync::atomic::Ordering::Relaxed);
    reset(t);
    before
}

/// True when the skin partitions are the source of the bone influences.
pub fn skin_from_partitions() -> bool {
    SKIN_FROM_PARTITIONS.load(core::sync::atomic::Ordering::Relaxed)
}

/// True when the record of a skinned shape needs the skin partitions: they are the source, or
/// the skin data has no weights.
pub unsafe fn needs_partitions(skin_data: usize) -> bool {
    SKIN_FROM_PARTITIONS.load(core::sync::atomic::Ordering::Relaxed) || !has_bone_weights(skin_data)
}

/// The counts of the skins packed from the skin data, for the report.
struct SkinCounts {
    from_data: u64,
    from_partitions: u64,
    without_bone_data: u64,
    vertices: u64,
    over_four: u64,
    most: usize,
    triangles: u64,
    triangles_over_four: u64,
    compared: u64,
    different: u64,
}

static SKIN_COUNTS: MainCell<SkinCounts> = MainCell::new(SkinCounts {
    from_data: 0,
    from_partitions: 0,
    without_bone_data: 0,
    vertices: 0,
    over_four: 0,
    most: 0,
    triangles: 0,
    triangles_over_four: 0,
    compared: 0,
    different: 0,
});

/// `NiSkinData`: the number of bones and the array of their `BoneData`, each with the skin to
/// bone transform, the bound, then the array of `BoneVertData` (a vertex and its weight, 8
/// bytes) and its length.
const DATA_BONE_COUNT: usize = 0x40;
const DATA_BONES: usize = 0x44;
const BONE_DATA_SIZE: usize = 0x4C;
const BONE_VERT_DATA: usize = 0x44;
const BONE_VERT_COUNT: usize = 0x48;

/// True when the skin data has the weights of its bones.
unsafe fn has_bone_weights(skin_data: usize) -> bool {
    if skin_data == 0 {
        return false;
    }
    let count: u32 = rd(skin_data, DATA_BONE_COUNT);
    let bones: usize = rd(skin_data, DATA_BONES);
    if count == 0 || count > 256 || bones == 0 {
        return false;
    }
    (0..count as usize).all(|b| rd::<u16>(bones + b * BONE_DATA_SIZE, BONE_VERT_COUNT) == 0 || rd::<usize>(bones + b * BONE_DATA_SIZE, BONE_VERT_DATA) != 0)
}

/// The weights of the bones of each vertex from the skin data, as the skin partition code of
/// the engine collects them (`NiSkinPartition::CreateVertexInteractions` 0x6C6B60): the sum
/// of the weights of a vertex is made 1. [`None`] when the data is not complete.
unsafe fn skin_data_weights(skin_data: usize, vertices: usize) -> Option<Vec<Vec<(u8, f32)>>> {
    if !has_bone_weights(skin_data) {
        return None;
    }
    let count: u32 = rd(skin_data, DATA_BONE_COUNT);
    let bones: usize = rd(skin_data, DATA_BONES);
    let mut lists: Vec<Vec<(u8, f32)>> = vec![Vec::new(); vertices];
    for b in 0..count as usize {
        let record = bones + b * BONE_DATA_SIZE;
        let data: usize = rd(record, BONE_VERT_DATA);
        for j in 0..rd::<u16>(record, BONE_VERT_COUNT) as usize {
            let vertex = rd::<u16>(data, j * 8) as usize;
            let weight: f32 = rd(data, j * 8 + 4);
            lists.get_mut(vertex)?.push((b as u8, weight));
        }
    }
    for list in &mut lists {
        unitize(list);
    }
    Some(lists)
}

/// Makes the sum of the weights 1.
fn unitize(list: &mut [(u8, f32)]) {
    let sum: f32 = list.iter().map(|(_, weight)| weight).sum();
    if sum > 0.0 {
        for (_, weight) in list.iter_mut() {
            *weight /= sum;
        }
    }
}

/// The influences of a vertex for the vertex format from its weights: at most 4 bones, the
/// largest weights, with a sum of 1 again (`SortByWeight`, `Truncate`, `Unitize` of the skin
/// partition code); the last bone in the fourth place with the rest of the weight.
fn pack_weights(list: &[(u8, f32)]) -> ([f32; 3], [u8; 4]) {
    let mut entry = ([0.0f32; 3], [0u8; 4]);
    if list.is_empty() {
        return entry;
    }
    let mut kept = list.to_vec();
    if kept.len() > 4 {
        kept.sort_by(|a, b| b.1.total_cmp(&a.1));
        kept.truncate(4);
        unitize(&mut kept);
    }
    let (last, first) = kept.split_last().expect("not empty");
    for (k, &(bone, weight)) in first.iter().enumerate() {
        entry.0[k] = weight;
        entry.1[k] = bone;
    }
    entry.1[3] = last.0;
    entry
}

/// The weight of each bone of a packed vertex, the fourth weight as the rest of 1.
fn unpacked(entry: &([f32; 3], [u8; 4])) -> Vec<(u8, f32)> {
    let mut out: Vec<(u8, f32)> = Vec::new();
    let last = 1.0 - entry.0.iter().sum::<f32>();
    for (k, &bone) in entry.1.iter().enumerate() {
        let weight = if k < 3 { entry.0[k] } else { last };
        if weight.abs() < 1e-6 {
            continue;
        }
        match out.iter_mut().find(|(b, _)| *b == bone) {
            Some(found) => found.1 += weight,
            None => out.push((bone, weight)),
        }
    }
    out.sort_by_key(|(bone, _)| *bone);
    out
}

/// True when two packed vertices have the same weight for each bone, within `1e-3`.
fn same_influences(a: &([f32; 3], [u8; 4]), b: &([f32; 3], [u8; 4])) -> bool {
    let (a, b) = (unpacked(a), unpacked(b));
    a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| x.0 == y.0 && (x.1 - y.1).abs() < 1e-3)
}

/// The influences of a skinned geometry from the skin data, with the counts for the report:
/// the vertices with more than 4 bones, the triangles with more than 4 bones (the partitions
/// take bones from those), and the vertices whose influences are not those of the partitions,
/// when the skin has partitions. `triangles` gives the vertices of each triangle.
unsafe fn skin_data_influences(t: &MainThread, skin_data: usize, vertices: usize, triangles: &[[usize; 3]]) -> Option<Vec<([f32; 3], [u8; 4])>> {
    let lists = skin_data_weights(skin_data, vertices)?;
    let packed: Vec<([f32; 3], [u8; 4])> = lists.iter().map(|list| pack_weights(list)).collect();
    let s = &mut *SKIN_COUNTS.get(t);
    s.from_data += 1;
    s.vertices += vertices as u64;
    s.over_four += lists.iter().filter(|list| list.len() > 4).count() as u64;
    s.most = s.most.max(lists.iter().map(Vec::len).max().unwrap_or(0));
    s.triangles += triangles.len() as u64;
    for triangle in triangles {
        let mut bones: Vec<u8> = triangle.iter().flat_map(|&v| lists.get(v).into_iter().flatten().map(|(bone, _)| *bone)).collect();
        bones.sort_unstable();
        bones.dedup();
        s.triangles_over_four += (bones.len() > 4) as u64;
    }
    if let Some(partitions) = skin_influences(skin_data, vertices) {
        s.compared += vertices as u64;
        s.different += packed.iter().zip(&partitions).filter(|(a, b)| !same_influences(a, b)).count() as u64;
    }
    Some(packed)
}

/// The vertices of each triangle of a geometry: of the triangle list, or of each strip.
unsafe fn triangle_vertices(indexes: usize, entries: u32, strip_count: u32, strip_lengths: usize) -> Vec<[usize; 3]> {
    let index = |i: usize| rd::<u16>(indexes, i * 2) as usize;
    let mut out = Vec::new();
    if strip_count == 0 {
        for t in 0..entries as usize / 3 {
            out.push([index(t * 3), index(t * 3 + 1), index(t * 3 + 2)]);
        }
    } else {
        let mut start = 0usize;
        for s in 0..strip_count as usize {
            let length = rd::<u16>(strip_lengths, s * 2) as usize;
            for i in 2..length {
                out.push([index(start + i - 2), index(start + i - 1), index(start + i)]);
            }
            start += length;
        }
    }
    out
}

/// Why `note` gives no handle for this geometry: the checks of `note` in their order, and
/// true when the geometry is empty (the device would draw nothing for it either). For the
/// counts and the log of the draws without a record.
pub unsafe fn why_none(t: &MainThread, kind: u32, geom_data: usize) -> (&'static str, bool) {
    if geom_data == 0 {
        return ("no geometry data", false);
    }
    let strips = match kind {
        DRAW_SHAPE => false,
        DRAW_STRIPS => true,
        DRAW_LINES => return ("lines without vertices or segments", true),
        _ => return ("a draw kind without a geometry record", false),
    };
    if !ni::is_kind_of(geom_data, if strips { RTTI_NI_TRI_STRIPS_DATA } else { RTTI_NI_TRI_SHAPE_DATA }) {
        return ("the data is not of the class of its draw function", false);
    }
    if vcall0(geom_data, V_GET_ACTIVE_VERTEX_COUNT) & 0xFFFF == 0 {
        return ("no active vertex", true);
    }
    if geom_data == *NO_INFLUENCES.get(t) {
        return ("the skin partitions give no bone influences in the vertex format", false);
    }
    if strips {
        ("no strips", true)
    } else if vcall0(geom_data, V_GET_TRI_LIST) == 0 {
        ("no triangle list", true)
    } else {
        ("no active triangle", true)
    }
}

/// Notes the geometry of a shape that is about to be drawn: sends it when it is new or
/// changed. Returns the handle for the draws that follow, or 0 for a geometry that has
/// nothing to draw (see `why_none`).
pub unsafe fn note(t: &MainThread, kind: u32, geom_data: usize, skin: usize) -> u32 {
    if !crate::capture::recording() || geom_data == 0 {
        return 0;
    }
    let strips = match kind {
        DRAW_SHAPE => false,
        DRAW_STRIPS => true,
        DRAW_LINES => return note_lines(t, geom_data),
        _ => return 0,
    };
    if !ni::is_kind_of(geom_data, if strips { RTTI_NI_TRI_STRIPS_DATA } else { RTTI_NI_TRI_SHAPE_DATA }) {
        return 0;
    }
    let active = vcall0(geom_data, V_GET_ACTIVE_VERTEX_COUNT) & 0xFFFF;
    if active == 0 {
        return 0;
    }
    // The indexes: the triangle list, or the strip lists one after the other.
    let (indexes, entries, strip_count, strip_lengths) = if strips {
        let count = rd::<u16>(geom_data, TS::numStrips) as u32;
        let lengths: usize = rd(geom_data, TS::stripLengths);
        let lists: usize = rd(geom_data, TS::stripLists);
        if count == 0 || lengths == 0 || lists == 0 {
            return 0;
        }
        let sum: u32 = (0..count as usize).map(|i| rd::<u16>(lengths, i * 2) as u32).sum();
        (lists, sum, count, lengths)
    } else {
        let list = vcall0(geom_data, V_GET_TRI_LIST) as usize;
        let triangles = vcall0(geom_data, V_GET_ACTIVE_TRIANGLE_COUNT) & 0xFFFF;
        (list, triangles * 3, 0, 0)
    };
    if indexes == 0 || entries == 0 {
        return 0;
    }
    let handle = geom_data as u32;
    let skin_data: usize = if skin != 0 { rd(skin, NiSkinInstance::skinData) } else { 0 };
    let version = Version { unique: rd(geom_data, GD::uniqueID), revision: rd(geom_data, GD::revisionID), vertices: active, entries, skin_data: skin_data as u32 };
    if known(t, handle, version) {
        return handle;
    }
    // The vertices, as `NiDX8VertexBufferManager::PackUntransformedVB` packs them, with the
    // weights and the bone indexes of a skinned shape after the position.
    let vertices: usize = rd(geom_data, GD::vertices);
    let normals: usize = rd(geom_data, GD::normals);
    let colors: usize = rd(geom_data, GD::colors);
    let coords: usize = rd(geom_data, GD::textureCoords);
    let all_vertices = rd::<u16>(geom_data, GD::vertexCount) as usize;
    let sets = (rd::<u16>(geom_data, GD::textureSets) as u32).min(8);
    let influences = if skin != 0 {
        let from_data = if SKIN_FROM_PARTITIONS.load(core::sync::atomic::Ordering::Relaxed) {
            None
        } else {
            let triangles = triangle_vertices(indexes, entries, strip_count, strip_lengths);
            let found = skin_data_influences(t, skin_data, all_vertices, &triangles);
            if found.is_none() {
                (*SKIN_COUNTS.get(t)).without_bone_data += 1;
            }
            found
        };
        let found = from_data.or_else(|| {
            let partitions = skin_influences(skin_data, all_vertices);
            (*SKIN_COUNTS.get(t)).from_partitions += partitions.is_some() as u64;
            partitions
        });
        match found {
            Some(influences) => Some(influences),
            None => {
                *NO_INFLUENCES.get(t) = geom_data;
                return 0;
            }
        }
    } else {
        None
    };
    let mut fvf = if skin != 0 { D3DFVF_XYZB4 | D3DFVF_LASTBETA_UBYTE4 } else { D3DFVF_XYZ };
    let mut stride = if skin != 0 { 28usize } else { 12usize };
    let normal_offset = stride;
    if normals != 0 {
        fvf |= D3DFVF_NORMAL;
        stride += 12;
    }
    let color_offset = stride;
    if colors != 0 {
        fvf |= D3DFVF_DIFFUSE;
        stride += 4;
    }
    let uv_offset = stride;
    if sets != 0 {
        fvf |= sets << 8;
        stride += sets as usize * 8;
    } else {
        fvf |= D3DFVF_TEX1;
        stride += 8;
    }
    let count = active as usize;
    let mut bytes = vec![0u8; count * stride];
    for i in 0..count {
        let to = &mut bytes[i * stride..];
        to[..12].copy_from_slice(core::slice::from_raw_parts((vertices + i * 12) as *const u8, 12));
        if let Some(influences) = &influences {
            let (weights, bones) = influences[i];
            for (k, weight) in weights.iter().enumerate() {
                to[12 + k * 4..16 + k * 4].copy_from_slice(&weight.to_le_bytes());
            }
            to[24..28].copy_from_slice(&bones);
        }
        if normals != 0 {
            to[normal_offset..normal_offset + 12].copy_from_slice(core::slice::from_raw_parts((normals + i * 12) as *const u8, 12));
        }
        if colors != 0 {
            to[color_offset..color_offset + 4].copy_from_slice(core::slice::from_raw_parts((colors + i * 4) as *const u8, 4));
        }
        for set in 0..sets as usize {
            // `NiGeometryData::getTextureSet`: the sets one after the other, each with all
            // the vertices.
            let from = coords + (set * all_vertices + i) * 8;
            let at = uv_offset + set * 8;
            to[at..at + 8].copy_from_slice(core::slice::from_raw_parts(from as *const u8, 8));
        }
    }
    let index_bytes = core::slice::from_raw_parts(indexes as *const u8, entries as usize * 2);
    let mut lengths = vec![0u8; ((strip_count as usize * 2) + 3) & !3];
    if strip_count != 0 {
        lengths[..strip_count as usize * 2].copy_from_slice(core::slice::from_raw_parts(strip_lengths as *const u8, strip_count as usize * 2));
    }
    send(t, handle, version, fvf, stride as u32, strip_count, &lengths, &bytes, index_bytes);
    handle
}

/// Sends a geometry record and notes its version. `lengths` has the strip lengths, padded
/// to 4 bytes.
unsafe fn send(t: &MainThread, handle: u32, version: Version, fvf: u32, stride: u32, strip_count: u32, lengths: &[u8], bytes: &[u8], index_bytes: &[u8]) {
    crate::capture::geometry(&[handle, version.unique, version.revision as u32, fvf, stride, version.vertices, version.entries, strip_count], lengths, bytes, index_bytes);
    versions(t).insert(handle, Sent { version, used: crate::camera_space::frame(t) });
    *RECORDS.get(t) += 1;
    *BYTES.get(t) += (32 + lengths.len() + bytes.len() + index_bytes.len()) as u64;
}

/// The vertex format and the stride of a vertex with a position, a colour when `colours`
/// and one texture coordinate when `coords`: the format of `RenderLines` and of
/// `PackScreenPolyVB`, which pack no normal.
fn plain_format(colours: bool, coords: bool) -> (u32, usize) {
    let (mut fvf, mut stride) = (D3DFVF_XYZ, 12usize);
    if colours {
        fvf |= D3DFVF_DIFFUSE;
        stride += 4;
    }
    if coords {
        fvf |= D3DFVF_TEX1;
        stride += 8;
    }
    (fvf, stride)
}

/// The geometry of a lines data (`RenderLines`), once for each version: the vertices as the
/// function packs them (the position, the colour, the texture coordinate of each), and the
/// index list of the segments as `NiDX8IndexBufferManager::buildLineSegmentIndexBuffer`
/// builds it: the vertex and the next one for each vertex with its segment flag. For the
/// last vertex with the flag the function writes one index and counts two: the second index
/// is what the buffer had there. The record has the vertex twice: a segment without length.
unsafe fn note_lines(t: &MainThread, geom_data: usize) -> u32 {
    let count = rd::<u16>(geom_data, GD::vertexCount) as usize;
    let flags: usize = rd(geom_data, LINES_SEGMENT_FLAGS);
    let vertices: usize = rd(geom_data, GD::vertices);
    if count == 0 || flags == 0 || vertices == 0 {
        return 0;
    }
    let mut indexes: Vec<u16> = Vec::new();
    for i in 0..count {
        if rd::<u8>(flags, i) != 0 {
            let next = if i + 1 < count { i + 1 } else { i };
            indexes.extend([i as u16, next as u16]);
        }
    }
    if indexes.is_empty() {
        return 0;
    }
    let handle = geom_data as u32;
    let version = Version { unique: rd(geom_data, GD::uniqueID), revision: rd(geom_data, GD::revisionID), vertices: count as u32, entries: indexes.len() as u32, skin_data: 0 };
    if known(t, handle, version) {
        return handle;
    }
    let colours: usize = rd(geom_data, GD::colors);
    let coords: usize = rd(geom_data, GD::textureCoords);
    let (fvf, stride) = plain_format(colours != 0, coords != 0);
    let mut bytes = Vec::with_capacity(count * stride);
    for i in 0..count {
        bytes.extend_from_slice(&rd::<[u8; 12]>(vertices, i * 12));
        if colours != 0 {
            bytes.extend_from_slice(&rd::<[u8; 4]>(colours, i * 4));
        }
        if coords != 0 {
            bytes.extend_from_slice(&rd::<[u8; 8]>(coords, i * 8));
        }
    }
    let index_bytes: Vec<u8> = indexes.iter().flat_map(|i| i.to_le_bytes()).collect();
    send(t, handle, version, fvf, stride as u32, 0, &[], &bytes, &index_bytes);
    handle
}

/// The geometry of a screen polygon (`RenderScreenPoly`) for this draw, as
/// `NiDX8VertexBufferManager::PackScreenPolyVB` packs it: each vertex at `(x * d, y * d, d)`
/// with `d = distance * z + near` (the polygon is at that depth under the screen projection
/// of the renderer), then its colour and its texture coordinate when the polygon has them.
/// The fan of the polygon is a triangle list in the record. The record goes out at each
/// draw: the vertices of a polygon can change. Returns the handle, the vertex array of the
/// polygon, or 0 for a polygon with fewer than three vertices.
///
/// The game keeps `d` on the x87 stack (64-bit mantissa); this code uses an `f64`, so a
/// rounding can differ.
pub unsafe fn note_screen_polygon(t: &MainThread, count: u32, vertices: usize, tex_coords: usize, colour: usize, near: f32, distance: f32) -> u32 {
    if !crate::capture::recording() || vertices == 0 || count < 3 {
        return 0;
    }
    let count = count as usize;
    let (fvf, stride) = plain_format(colour != 0, tex_coords != 0);
    let mut bytes = Vec::with_capacity(count * stride);
    for i in 0..count {
        let [x, y, z]: [f32; 3] = rd(vertices, i * 12);
        let d = distance as f64 * z as f64 + near as f64;
        for value in [(d * x as f64) as f32, (d * y as f64) as f32, d as f32] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        if colour != 0 {
            bytes.extend_from_slice(&rd::<[u8; 4]>(colour, i * 4));
        }
        if tex_coords != 0 {
            bytes.extend_from_slice(&rd::<[u8; 8]>(tex_coords, i * 8));
        }
    }
    let indexes: Vec<u16> = (0..count as u16 - 2).flat_map(|n| [0, n + 1, n + 2]).collect();
    let index_bytes: Vec<u8> = indexes.iter().flat_map(|i| i.to_le_bytes()).collect();
    let handle = vertices as u32;
    let version = Version { unique: 0, revision: 0, vertices: count as u32, entries: indexes.len() as u32, skin_data: 0 };
    send(t, handle, version, fvf, stride as u32, 0, &[], &bytes, &index_bytes);
    // The next draw of the polygon sends it again.
    versions(t).remove(&handle);
    handle
}

/// The bone influences of each vertex of a skinned geometry, from the partitions of its
/// skin data: three weights, then the index in the bones of the skin of each of four bones.
/// The fourth weight is 1 less the sum of the three, as the vertex blend of the device has
/// it. A partition has `numBonesPerVertex` weights for each vertex. Without a bone palette
/// that is all its bones, and weight `k` is for bone `k` of the partition, as the stock
/// packer (`PackSkinnedVB` 0x6BE2B0) has it [I: from the Gamebryo 1.2 source, not read in
/// the binary]; with a palette (a partition with more bones than a
/// vertex has weights), the palette gives for each weight of a vertex the bone of the
/// partition. The last bone of a vertex goes to the fourth place, so the arithmetic is the
/// same. [`None`] when the skin has no partitions yet, or the data does not fit the format.
unsafe fn skin_influences(skin_data: usize, vertices: usize) -> Option<Vec<([f32; 3], [u8; 4])>> {
    if skin_data == 0 {
        return None;
    }
    let skin_partition: usize = rd(skin_data, SKIN_DATA_SKIN_PARTITION);
    if skin_partition == 0 {
        return None;
    }
    let count: u32 = rd(skin_partition, NiSkinPartition::partitionCount);
    let mut partition: usize = rd(skin_partition, NiSkinPartition::partitions);
    if count == 0 || partition == 0 {
        return None;
    }
    let mut out = vec![([0.0f32; 3], [0u8; 4]); vertices];
    for _ in 0..count {
        let bones: usize = rd(partition, P::bones);
        let weights: usize = rd(partition, P::weights);
        let map: usize = rd(partition, P::vertices);
        let palette: usize = rd(partition, P::bonePalette);
        let bone_count = rd::<u16>(partition, P::numBones) as usize;
        let per_vertex = rd::<u16>(partition, P::numBonesPerVertex) as usize;
        let in_partition = rd::<u16>(partition, P::numVertices) as usize;
        if bones == 0 || weights == 0 || map == 0 || per_vertex == 0 || per_vertex > 4 || (palette == 0 && per_vertex != bone_count) {
            return None;
        }
        // The bone of the skin for weight `k` of vertex `v` of the partition.
        let bone = |v: usize, k: usize| -> Option<u8> {
            let local = if palette != 0 { rd::<u8>(palette, v * per_vertex + k) as usize } else { k };
            if local >= bone_count {
                return None;
            }
            u8::try_from(rd::<u16>(bones, local * 2)).ok()
        };
        for v in 0..in_partition {
            let g = rd::<u16>(map, v * 2) as usize;
            if g >= vertices {
                return None;
            }
            let mut entry = ([0.0f32; 3], [0u8; 4]);
            for k in 0..per_vertex - 1 {
                entry.0[k] = rd(weights, (v * per_vertex + k) * 4);
                entry.1[k] = bone(v, k)?;
            }
            entry.1[3] = bone(v, per_vertex - 1)?;
            out[g] = entry;
        }
        partition += P::SIZE;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{pack_weights, same_influences, unitize, unpacked};

    #[test]
    fn a_vertex_with_up_to_four_bones_keeps_them() {
        let entry = pack_weights(&[(3, 0.5), (7, 0.3), (9, 0.2)]);
        assert_eq!(entry.1, [3, 7, 0, 9]);
        assert_eq!(unpacked(&entry).len(), 3);
        assert!((unpacked(&entry).iter().find(|(b, _)| *b == 9).unwrap().1 - 0.2).abs() < 1e-6);
        // One bone: all of the weight on it.
        assert_eq!(unpacked(&pack_weights(&[(5, 1.0)])), vec![(5, 1.0)]);
    }

    #[test]
    fn a_vertex_with_more_than_four_bones_keeps_the_four_largest() {
        let mut list = vec![(1, 0.05), (2, 0.4), (3, 0.25), (4, 0.2), (5, 0.1)];
        unitize(&mut list);
        let kept = unpacked(&pack_weights(&list));
        assert_eq!(kept.iter().map(|(b, _)| *b).collect::<Vec<_>>(), vec![2, 3, 4, 5]);
        let sum: f32 = kept.iter().map(|(_, w)| w).sum();
        assert!((sum - 1.0).abs() < 1e-6);
        // 0.4 of the 0.95 kept.
        assert!((kept[0].1 - 0.4 / 0.95).abs() < 1e-5);
    }

    #[test]
    fn the_order_of_the_bones_does_not_matter() {
        let a = pack_weights(&[(3, 0.6), (7, 0.4)]);
        let b = pack_weights(&[(7, 0.4), (3, 0.6)]);
        assert!(same_influences(&a, &b));
        assert!(!same_influences(&a, &pack_weights(&[(3, 0.5), (7, 0.5)])));
    }
}
