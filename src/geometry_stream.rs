//! The geometry of a draw as a record of its own, for the retained pipeline (step R1).
//!
//! This module packs the geometry data of a shape as the engine packs it (positions,
//! normals, colours, the texture sets, then the triangle list or the strip lists), and
//! sends it once as a `GEOMETRY` record keyed by the geometry data. The key has the unique
//! id, the revision and the active counts of the data: the engine packs a geometry again
//! only when the revision changes (`packGeometryBuffers`), so these are its version.
//!
//! The helper draws from these buffers when it has them. The buffers of the device are
//! still made by the ports (step R3 stops that), and the helper compares the two once for
//! each version, so a difference of the pack shows as a count.
//!
//! A skinned shape (step R5) is packed with the weights and the bone indexes of each vertex
//! for the whole skin, from the partitions of its skin data, so one draw with the palette
//! of the skin replaces the draws of the partitions. The particles of a system come in the
//! shape record itself (step R7a). Lines are packed as `RenderLines` packs them, with the
//! index list of their segments, and a screen polygon as `PackScreenPolyVB` packs it, at
//! each draw (step R7b).

use std::collections::HashMap;

use crate::d3d::{D3DFVF_DIFFUSE, D3DFVF_NORMAL, D3DFVF_TEX1, D3DFVF_XYZ};
use crate::engine::vcall0;
use crate::gen::layouts::{NiGeometryData as GD, NiSkinInstance, NiSkinPartition, NiSkinPartition_Partition as P, NiTriStripsData as TS};
use crate::link::{DRAW_LINES, DRAW_SHAPE, DRAW_STRIPS};
use crate::mem::rd;
use crate::ports::ni;

/// `NiLinesData::lineSegmentFlags`: one byte for each vertex, not 0 when a segment goes from
/// the vertex to the next one.
pub const LINES_SEGMENT_FLAGS: usize = 0x34;

/// `D3DFVF_XYZB4` and `D3DFVF_LASTBETA_UBYTE4`: a position, three weights and four bone
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

static mut VERSIONS: Option<HashMap<u32, Version>> = None;
static mut RECORDS: u64 = 0;
static mut BYTES: u64 = 0;

unsafe fn versions() -> &'static mut HashMap<u32, Version> {
    (*core::ptr::addr_of_mut!(VERSIONS)).get_or_insert_with(HashMap::new)
}

/// Forgets the versions: the next draw of each geometry sends it again. For the start of a
/// capture file from the stream, which has no geometry records before it.
pub unsafe fn reset() {
    versions().clear();
}

/// Forgets these geometries: the next draw of each sends it again.
pub unsafe fn forget(handles: &[u32]) {
    for handle in handles {
        versions().remove(handle);
    }
}

/// One line for the log: the records and the bytes that were sent.
pub unsafe fn report() -> String {
    let (records, bytes) = (*core::ptr::addr_of!(RECORDS), *core::ptr::addr_of!(BYTES));
    format!("geometry: {} records, {:.1} MB, {} geometries known", records, bytes as f64 / 1e6, versions().len())
}

/// Why `note` gives no handle for this geometry: the checks of `note` in their order, and
/// true when the geometry is empty (the device would draw nothing for it either). For the
/// counts and the log of the draws without a record.
pub unsafe fn why_none(kind: u32, geom_data: usize) -> (&'static str, bool) {
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
pub unsafe fn note(kind: u32, geom_data: usize, skin: usize) -> u32 {
    if !crate::capture::recording() || geom_data == 0 {
        return 0;
    }
    let strips = match kind {
        DRAW_SHAPE => false,
        DRAW_STRIPS => true,
        DRAW_LINES => return note_lines(geom_data),
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
    if versions().get(&handle) == Some(&version) {
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
        match skin_influences(skin_data, all_vertices) {
            Some(influences) => Some(influences),
            None => return 0,
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
    send(handle, version, fvf, stride as u32, strip_count, &lengths, &bytes, index_bytes);
    handle
}

/// Sends a geometry record and notes its version. `lengths` has the strip lengths, padded
/// to 4 bytes.
unsafe fn send(handle: u32, version: Version, fvf: u32, stride: u32, strip_count: u32, lengths: &[u8], bytes: &[u8], index_bytes: &[u8]) {
    crate::capture::geometry(&[handle, version.unique, version.revision as u32, fvf, stride, version.vertices, version.entries, strip_count], lengths, bytes, index_bytes);
    versions().insert(handle, version);
    RECORDS += 1;
    BYTES += (32 + lengths.len() + bytes.len() + index_bytes.len()) as u64;
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
unsafe fn note_lines(geom_data: usize) -> u32 {
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
    if versions().get(&handle) == Some(&version) {
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
    send(handle, version, fvf, stride as u32, 0, &[], &bytes, &index_bytes);
    handle
}

/// The geometry of a screen polygon (`RenderScreenPoly`) for this draw, as
/// `NiDX8VertexBufferManager::PackScreenPolyVB` packs it: each vertex at `(x * d, y * d, d)`
/// with `d = distance * z + near` (the polygon is at that depth under the screen projection
/// of the renderer), then its colour and its texture coordinate when the polygon has them.
/// The fan of the polygon is a triangle list in the record. The record goes out at each
/// draw: the vertices of a polygon can change, and the record is small. Returns the handle,
/// the vertex array of the polygon, or 0 for a polygon with fewer than three vertices.
///
/// The game keeps `d` on the x87 stack with 64 bits of mantissa and rounds at each store;
/// this code keeps it as an `f64` (53 bits) and rounds at each store. The two differ only
/// when the extra bits change a rounding (inference: not measured).
pub unsafe fn note_screen_polygon(count: u32, vertices: usize, tex_coords: usize, colour: usize, near: f32, distance: f32) -> u32 {
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
    send(handle, version, fvf, stride as u32, 0, &[], &bytes, &index_bytes);
    // The next draw of the polygon sends it again.
    versions().remove(&handle);
    handle
}

/// The bone influences of each vertex of a skinned geometry, from the partitions of its
/// skin data: three weights, then the index in the bones of the skin of each of four bones.
/// The fourth weight is 1 less the sum of the three, as the vertex blend of the device has
/// it. The stock packer (`PackSkinnedVB`) gives weight `k` of a vertex to bone `k` of its
/// partition and leaves the last weight to the device: here the last bone of the partition
/// goes to the fourth place, so the arithmetic is the same. `None` when the skin has no
/// partitions yet, or the data does not fit the format.
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
        let per_vertex = rd::<u16>(partition, P::numBones) as usize;
        let in_partition = rd::<u16>(partition, P::numVertices) as usize;
        if bones == 0 || weights == 0 || map == 0 || per_vertex == 0 || per_vertex > 4 {
            return None;
        }
        let mut indexes = [0u8; 4];
        for (k, index) in indexes.iter_mut().enumerate().take(per_vertex) {
            *index = u8::try_from(rd::<u16>(bones, k * 2)).ok()?;
        }
        for v in 0..in_partition {
            let g = rd::<u16>(map, v * 2) as usize;
            if g >= vertices {
                return None;
            }
            let mut entry = ([0.0f32; 3], [0u8; 4]);
            for k in 0..per_vertex - 1 {
                entry.0[k] = rd(weights, (v * per_vertex + k) * 4);
                entry.1[k] = indexes[k];
            }
            entry.1[3] = indexes[per_vertex - 1];
            out[g] = entry;
        }
        partition += P::SIZE;
    }
    Some(out)
}
