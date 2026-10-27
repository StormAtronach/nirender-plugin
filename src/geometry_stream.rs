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

use crate::checked::Open;
use crate::main_thread::{MainCell, MainThread};
use crate::d3d::{D3DFVF_DIFFUSE, D3DFVF_NORMAL, D3DFVF_TEX1, D3DFVF_XYZ};
use crate::engine::vcall0;
use crate::gen::layouts::{NiBoneData, NiGeometryData as GD, NiSkinData, NiSkinInstance, NiSkinPartition, NiSkinPartition_Partition as P, NiTriStripsData as TS};
use crate::link::{DRAW_LINES, DRAW_SHAPE, DRAW_STRIPS};
use crate::mem::rd;
use crate::ports::ni;

/// One byte for each vertex, not 0 when a segment goes from the vertex to the next one.
pub const LINES_SEGMENT_FLAGS: usize = crate::gen::layouts::NiLinesData::lineSegmentFlags;

/// [`D3DFVF_XYZB4`] and [`D3DFVF_LASTBETA_UBYTE4`]: a position, three weights and four bone
/// indexes in a byte each. The format of a skinned vertex.
const D3DFVF_XYZB4: u32 = 0x00C;
const D3DFVF_LASTBETA_UBYTE4: u32 = 0x1000;
const SKIN_DATA_SKIN_PARTITION: usize = NiSkinData::skinPartition;

/// `NiRTTI` of `NiTriShapeData` and of `NiTriStripsData`.
const RTTI_NI_TRI_SHAPE_DATA: usize = 0x7DE490;
const RTTI_NI_TRI_STRIPS_DATA: usize = 0x7DE588;
const V_GET_ACTIVE_VERTEX_COUNT: usize = crate::gen::layouts::InterfaceNiGeometryData::GetActiveVertexCount;
const V_GET_ACTIVE_TRIANGLE_COUNT: usize = crate::gen::layouts::InterfaceNiTriBasedGeomData::GetActiveTriangleCount;
const V_GET_TRI_LIST: usize = crate::gen::layouts::InterfaceNiTriBasedGeomData::GetTriList_const;

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

/// A geometry that was sent: its version, the frame of its last draw, and the texture set of
/// its UV controller when a `GEOMETRY_UV` record gave the state of its vertices.
struct Sent {
    version: Version,
    used: u32,
    uv_set: Option<u32>,
    /// The helper has the targets of its morph controller: a new revision can go as weights.
    morph: bool,
}

/// The geometries sent, by the address of their data. A destroyed data leaves at its
/// destructor ([`gone`]). An entry of a geometry not drawn for [`FORGET_AFTER`] frames goes
/// too (`prune`): before `gone`, a map kept for the session grew with every address the game
/// ever used (the leak hunt of 2026-10-23, UMDH: 819 KB after 35 minutes of a loop). A
/// geometry still alive is sent again at its next draw.
static VERSIONS: MainCell<Option<HashMap<u32, Sent>>> = MainCell::new(None);
const FORGET_AFTER: u32 = 36_000;
const PRUNE_EVERY: u32 = 4_096;
static LAST_PRUNE: MainCell<u32> = MainCell::new(0);
static FORGOTTEN: MainCell<u64> = MainCell::new(0);
/// The last geometry whose skin gave no bone influences (`skin_influences`), for the count of
/// the draws without a record: such a geometry is not empty.
static NO_INFLUENCES: MainCell<usize> = MainCell::new(0);
/// The last geometry whose skin data was in its destructor (`skin_data_dying`), for the count
/// of the draws without a record.
static DYING: MainCell<usize> = MainCell::new(0);
static RECORDS: MainCell<u64> = MainCell::new(0);
static BYTES: MainCell<u64> = MainCell::new(0);
/// The `GEOMETRY_GONE` records sent.
static GONE: MainCell<u64> = MainCell::new(0);
/// The handles of the stream, by the address and the unique id of a geometry data: a serial
/// made at its first note and kept until its destructor ([`gone`]). The engine gives each
/// data a unique id from a counter that only its constructors touch (`NiGeometryData::ctor`
/// 0x6EF2C0, `ctor_args` 0x6EF1E0), but an address comes back for a later data: a handle of
/// the stream must not, or an instance of the old data takes the new one (the helper's
/// orphans of a gone geometry). A screen polygon has the unique id 0: its vertex array keeps
/// one serial, and its record goes again at each draw.
static SERIALS: MainCell<Option<HashMap<(u32, u32), u32>>> = MainCell::new(None);
/// The serial of the next geometry: 1 for the first, never 0 (no geometry).
static NEXT_SERIAL: MainCell<u32> = MainCell::new(1);

#[track_caller]
fn serials<'a>(t: &'a MainThread) -> Open<'a, HashMap<(u32, u32), u32>> {
    SERIALS.get(t).map(|serials| serials.get_or_insert_with(HashMap::new))
}

/// The handle of a geometry data in the stream: its serial (`SERIALS`), made now for a data
/// the stream has not named yet.
pub unsafe fn stream_handle(t: &MainThread, address: u32, unique: u32) -> u32 {
    if let Some(&serial) = serials(t).get(&(address, unique)) {
        return serial;
    }
    let serial = {
        let mut next = NEXT_SERIAL.get(t);
        let serial = *next;
        *next = next.wrapping_add(1).max(1);
        serial
    };
    serials(t).insert((address, unique), serial);
    serial
}

#[track_caller]
fn versions<'a>(t: &'a MainThread) -> Open<'a, HashMap<u32, Sent>> {
    VERSIONS.get(t).map(|versions| versions.get_or_insert_with(HashMap::new))
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
    let mut last = LAST_PRUNE.get(t);
    if now.wrapping_sub(*last) < PRUNE_EVERY {
        return;
    }
    *last = now;
    let mut map = versions(t);
    let before = map.len();
    map.retain(|_, sent| now.wrapping_sub(sent.used) < FORGET_AFTER);
    *FORGOTTEN.get(t) += (before - map.len()) as u64;
}

/// Forgets the versions: the next draw of each geometry sends it again. For the start of a
/// capture file from the stream, which has no geometry records before it.
pub unsafe fn reset(t: &MainThread) {
    versions(t).clear();
    *MORPH_SENT.get(t) = None;
}

/// A geometry data was destroyed (`NiRenderer_static::RemoveGeometryData`, from its
/// destructor): its version goes when it has this unique id, and the helper gets a
/// `GEOMETRY_GONE` record with its serial. The record goes also for a data that was
/// forgotten by `prune`: the helper can still hold it. A data that the stream never named
/// has no serial and no record.
pub unsafe fn gone(t: &MainThread, handle: u32, unique: u32) {
    let mut map = versions(t);
    if map.get(&handle).is_some_and(|sent| sent.version.unique == unique) {
        map.remove(&handle);
    }
    if let Some(sent_targets) = (*MORPH_SENT.get(t)).as_mut() {
        if sent_targets.get(&handle) == Some(&unique) {
            sent_targets.remove(&handle);
        }
    }
    let serial = serials(t).remove(&(handle, unique));
    if let Some(serial) = serial {
        *GONE.get(t) += 1;
        crate::capture::geometry_gone(serial, unique);
    }
}

/// Forgets these geometries, by their serials: the next draw of each sends it again.
pub unsafe fn forget(t: &MainThread, handles: &[u32]) {
    let addresses: Vec<u32> = serials(t).iter().filter(|(_, serial)| handles.contains(serial)).map(|(&(address, _), _)| address).collect();
    for handle in &addresses {
        versions(t).remove(handle);
    }
}

/// One line for the log: the records and the bytes that were sent, and the counts of the
/// bone influences of the skins.
pub unsafe fn report(t: &MainThread) -> String {
    let (records, bytes) = (*RECORDS.get(t), *BYTES.get(t));
    // Copies: each table opens once here.
    let (uv_check, morph_check) = (*UV_CHECK.get(t), *MORPH_CHECK.get(t));
    let s = &*SKIN_COUNTS.get(t);
    format!(
        "geometry: {} records, {:.1} MB, {} UV steps (checked {}, over 1/1000 {}, the most {:.6}), {} morph steps (checked {}, over 1/1000 {}, the most {:.6}), {} geometries known, {} forgotten after {} frames without a draw, {} destroyed (GEOMETRY_GONE); skins: {} from the skin data, {} from the partitions ({} without bone data), {} with a dying skin data; vertices {}, with more than 4 bones {} (the most {}); triangles {}, with more than 4 bones {}; vertices compared with the partitions {}, different {}",
        records,
        bytes as f64 / 1e6,
        *UV_STEPS.get(t),
        uv_check.0,
        uv_check.1,
        uv_check.2,
        *MORPH_STEPS.get(t),
        morph_check.0,
        morph_check.1,
        morph_check.2,
        versions(t).len(),
        *FORGOTTEN.get(t),
        FORGET_AFTER,
        *GONE.get(t),
        s.from_data,
        s.from_partitions,
        s.without_bone_data,
        s.dying,
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
/// data without its weights takes the partitions. A dying skin data (`skin_data_dying`) gets
/// no record: `NiSkinData::dtor` frees the weights and releases the partitions.
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
/// the skin data has no weights. A dying skin data gets no record (`note`), so it needs none.
pub unsafe fn needs_partitions(skin_data: usize) -> bool {
    (SKIN_FROM_PARTITIONS.load(core::sync::atomic::Ordering::Relaxed) || !has_bone_weights(skin_data)) && !skin_data_dying(skin_data)
}

/// The counts of the skins packed from the skin data, for the report.
struct SkinCounts {
    from_data: u64,
    from_partitions: u64,
    without_bone_data: u64,
    dying: u64,
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
    dying: 0,
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
const DATA_BONE_COUNT: usize = NiSkinData::numBones;
const DATA_BONES: usize = NiSkinData::boneData;
const BONE_DATA_SIZE: usize = NiBoneData::SIZE;
const BONE_VERT_DATA: usize = NiBoneData::vertexWeights;
const BONE_VERT_COUNT: usize = NiBoneData::numVertexWeights;

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

/// True when the skin data is in or after its destructor: a bone has a vertex count and no
/// weight array. Only `NiSkinData::dtor` (0x6D09F0) writes a null array, after it releases the
/// skin partition; `NiSkinData::LoadBinary` (0x6D0C90) gives every bone an array.
pub unsafe fn skin_data_dying(skin_data: usize) -> bool {
    if skin_data == 0 {
        return false;
    }
    let count: u32 = rd(skin_data, DATA_BONE_COUNT);
    let bones: usize = rd(skin_data, DATA_BONES);
    if count == 0 || count > 256 || bones == 0 {
        return false;
    }
    (0..count as usize).any(|b| rd::<u16>(bones + b * BONE_DATA_SIZE, BONE_VERT_COUNT) != 0 && rd::<usize>(bones + b * BONE_DATA_SIZE, BONE_VERT_DATA) == 0)
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
    if geom_data == *DYING.get(t) {
        return ("the skin data is in its destructor", false);
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
/// `shape` is the geometry object that draws the data, for the census only (0 when unknown).
pub unsafe fn note(t: &MainThread, kind: u32, geom_data: usize, skin: usize, shape: usize) -> u32 {
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
    // A dying skin data gets no record, also when its geometry is known: the draw of a shape
    // whose skin data is in its destructor is a lifetime fault. The first 8 go to the log.
    if skin_data_dying(skin_data) {
        let counts = &mut *SKIN_COUNTS.get(t);
        counts.dying += 1;
        if counts.dying <= 8 {
            let root: usize = rd(skin, NiSkinInstance::rootParent);
            crate::log!(
                "geometry: the skin data 0x{skin_data:x} of a shape under '{}' is in its destructor (a bone with a vertex count and no weight array); the shape gets no record",
                crate::ports::draw::object_name(root)
            );
        }
        *DYING.get(t) = geom_data;
        return 0;
    }
    let version = Version { unique: rd(geom_data, GD::uniqueID), revision: rd(geom_data, GD::revisionID), vertices: active, entries, skin_data: skin_data as u32 };
    let stream = stream_handle(t, handle, version.unique);
    if known(t, handle, version) {
        return stream;
    }
    if census_on() {
        census_note(t, handle, version, geom_data, skin, shape);
    }
    // A geometry whose only controller is a UV controller: a new revision alone, after a
    // record with the state of its vertices, goes as the controller's new state.
    let uv = uv_controller(shape, geom_data);
    if let Some(uv) = uv {
        let step = versions(t).get(&handle).is_some_and(|sent| {
            let before = sent.version;
            sent.uv_set == Some(uv.set) && before.unique == version.unique && before.vertices == version.vertices && before.entries == version.entries && before.skin_data == version.skin_data
        });
        if step {
            send_uv(stream, version.unique, &uv, false);
            *UV_STEPS.get(t) += 1;
            if uv_mode() == UvMode::Check {
                check_uv(t, handle, geom_data, &uv);
            } else {
                versions(t).insert(handle, Sent { version, used: crate::camera_space::frame(t), uv_set: Some(uv.set), morph: false });
                return stream;
            }
        }
    }
    // A geometry whose only controller is a morph controller: a new revision alone, after its
    // targets went, goes as the weights of the targets.
    let morph = if uv.is_none() { morph_controller(shape, geom_data) } else { None };
    if let Some(morph) = &morph {
        let step = versions(t).get(&handle).is_some_and(|sent| {
            let before = sent.version;
            sent.morph && before.unique == version.unique && before.vertices == version.vertices && before.entries == version.entries && before.skin_data == version.skin_data
        });
        if step {
            let weights = &mut *MORPH_SCRATCH.get(t);
            morph_weights(morph, weights);
            crate::capture::morph_weights(&[stream, version.unique, weights.len() as u32], weights);
            *MORPH_STEPS.get(t) += 1;
            if morph_mode() == MorphMode::Check {
                check_morph(t, morph, weights, geom_data);
            } else {
                versions(t).insert(handle, Sent { version, used: crate::camera_space::frame(t), uv_set: None, morph: true });
                return stream;
            }
        }
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
    send(t, handle, stream, version, fvf, stride as u32, strip_count, &lengths, &bytes, index_bytes);
    if let Some(uv) = uv {
        send_uv(stream, version.unique, &uv, true);
        if let Some(sent) = versions(t).get_mut(&handle) {
            sent.uv_set = Some(uv.set);
        }
        if uv_mode() == UvMode::Check {
            keep_uv_base(t, handle, geom_data, &uv);
        }
    }
    if let Some(morph) = morph {
        let mut morph_sent = MORPH_SENT.get(t);
        let sent_targets = morph_sent.get_or_insert_with(Default::default);
        if sent_targets.insert(handle, version.unique) != Some(version.unique) {
            send_morph_targets(stream, version.unique, &morph);
        }
        if let Some(sent) = versions(t).get_mut(&handle) {
            sent.morph = true;
        }
    }
    stream
}

/// `NiRTTI` of `NiGeomMorpherController` (`GetRTTI` 0x717900), and its flag that makes the
/// engine compute the normals again after a morph (`OnPreDisplay` 0x717370, bit 4).
const RTTI_NI_GEOM_MORPHER_CONTROLLER: usize = 0x7DF180;
const MORPH_UPDATE_NORMALS: u16 = 0x10;
/// The most targets of a morph that goes as weights.
const MORPH_MOST_TARGETS: u32 = 64;

/// A morph controller: its data, its targets, the vertices of a target, and its time.
struct Morph {
    data: usize,
    targets: u32,
    vertices: u32,
    time: f32,
}

/// How a geometry under a morph controller goes (`NIRENDER_MORPH_WEIGHTS`): its new revisions
/// as the weights of its targets (the default); as whole geometries (`0`, as before); or both
/// (`check`: the plugin compares the weighted targets with the engine's positions).
#[derive(Clone, Copy, PartialEq, Eq)]
enum MorphMode {
    On,
    Off,
    Check,
}

fn morph_mode() -> MorphMode {
    static MODE: std::sync::OnceLock<MorphMode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("NIRENDER_MORPH_WEIGHTS").as_deref() {
        Ok("0") => MorphMode::Off,
        Ok("check") => MorphMode::Check,
        _ => MorphMode::On,
    })
}

/// The morph controller of `shape`, when it is the only controller of the shape, its target
/// is the shape and the shape draws `geom_data`; its morph sets the positions only (no new
/// normals, no sum onto the positions before: `field_39`, `GenMorphInterp` 0x725ED0); and
/// its targets are at most [`MORPH_MOST_TARGETS`], each with vertices.
unsafe fn morph_controller(shape: usize, geom_data: usize) -> Option<Morph> {
    use crate::gen::layouts::{NiMorphData as MD, NiMorphData_MorphTarget as MT, NiMorpherController as MC};
    if shape == 0 || morph_mode() == MorphMode::Off {
        return None;
    }
    let controller: usize = rd(shape, crate::gen::layouts::NiAVObject::controllers);
    if controller == 0 || rd::<usize>(controller, MC::nextController) != 0 {
        return None;
    }
    if ni::get_rtti(controller) != RTTI_NI_GEOM_MORPHER_CONTROLLER || rd::<usize>(controller, MC::target) != shape {
        return None;
    }
    if rd::<usize>(shape, crate::gen::layouts::NiGeometry::spModelData) != geom_data {
        return None;
    }
    if rd::<u16>(controller, MC::flags) & MORPH_UPDATE_NORMALS != 0 && rd::<usize>(geom_data, GD::normals) != 0 {
        return None;
    }
    if rd::<u8>(controller, MC::field_39) != 0 {
        return None;
    }
    let data: usize = rd(controller, MC::morphData);
    if data == 0 {
        return None;
    }
    let targets: u32 = rd(data, MD::numTargets);
    let vertices: u32 = rd(data, MD::numVertsPerTarget);
    let array: usize = rd(data, MD::arrayMorphTargets);
    if targets == 0 || targets > MORPH_MOST_TARGETS || vertices == 0 || array == 0 || vertices > rd::<u16>(geom_data, GD::vertexCount) as u32 {
        return None;
    }
    if (0..targets as usize).any(|i| rd::<usize>(array + i * MT::SIZE, MT::targetVerts) == 0) {
        return None;
    }
    Some(Morph { data, targets, vertices, time: scaled_time(controller) })
}

/// The time that `GenMorphInterp` gets: what `NiTimeController::ComputeScaledTime` (0x6FC710)
/// returns. That call leaves the sum of the scaled steps in `lastScaledTime`; the time is that
/// sum plus the phase, wrapped by the cycle (bits 1 and 2 of the flags: 0 loop, 1 reverse,
/// else clamp) between the low and the high key times, then held between them. The call runs
/// in `OnPreDisplay` (0x717384), or in `Update` with the Morrowind Code Patch, which then keeps
/// the time at `updateTime`; after the display both leave these fields.
unsafe fn scaled_time(controller: usize) -> f32 {
    use crate::gen::layouts::NiTimeController as TC;
    let low = rd::<f32>(controller, TC::lowKeyFrame) as f64;
    let high = rd::<f32>(controller, TC::highKeyFrame) as f64;
    let mut time = rd::<f32>(controller, TC::lastScaledTime) as f64 + rd::<f32>(controller, TC::phase) as f64;
    let interval = high - low;
    match (rd::<u16>(controller, TC::flags) >> 1) & 3 {
        0 => {
            time = if interval == 0.0 { low } else { (time - low) % interval + low };
            if time < low {
                time += interval;
            }
        }
        1 => {
            if interval == 0.0 {
                time = low;
            } else {
                // As the engine does: the remainder is tested against the low key time.
                let twice = interval + interval;
                let mut t = (time - low) % twice;
                if t < low {
                    t += twice;
                }
                if t > interval {
                    t = twice - t;
                }
                time = t + low;
            }
        }
        _ => {}
    }
    if time > high {
        high as f32
    } else if time < low {
        low as f32
    } else {
        time as f32
    }
}

/// The weights of the targets at the controller's time, as `GenMorphInterp` (0x725ED0) takes
/// them: the first target of relative targets has weight 1; a target without keys adds
/// nothing. Each search starts from a copy of the target's key index: the engine's stays.
unsafe fn morph_weights(morph: &Morph, out: &mut Vec<f32>) {
    use crate::gen::layouts::{NiMorphData as MD, NiMorphData_MorphTarget as MT};
    out.clear();
    let relative = rd::<u8>(morph.data, MD::bRelativeTargets) != 0;
    let array: usize = rd(morph.data, MD::arrayMorphTargets);
    for i in 0..morph.targets as usize {
        let target = array + i * MT::SIZE;
        let keys: i32 = rd(target, MT::numKeys);
        let weight = if i == 0 && relative {
            1.0
        } else if keys == 0 {
            0.0
        } else {
            let mut index: u32 = rd(target, MT::lastIndex);
            crate::engine::float_key_interp(morph.time, rd(target, MT::morphKeys), rd(target, MT::eType), keys, &mut index)
        };
        out.push(weight);
    }
}

/// Sends the targets of a morph: each target's positions, copied once into the ring.
unsafe fn send_morph_targets(handle: u32, unique: u32, morph: &Morph) {
    use crate::gen::layouts::{NiMorphData as MD, NiMorphData_MorphTarget as MT};
    let head = [handle, unique, morph.targets, morph.vertices];
    let array: usize = rd(morph.data, MD::arrayMorphTargets);
    let mut parts: Vec<&[u8]> = vec![crate::capture::word_bytes(&head)];
    for i in 0..morph.targets as usize {
        let verts: usize = rd(array + i * MT::SIZE, MT::targetVerts);
        parts.push(core::slice::from_raw_parts(verts as *const u8, morph.vertices as usize * 12));
    }
    crate::capture::morph_targets(&parts);
}

/// The steps sent as weights, the weights of the last step, the targets sent, and the check
/// mode's counts: the steps compared, those with a position further than 1/1000 from the
/// engine's, and the largest distance.
static MORPH_STEPS: MainCell<u64> = MainCell::new(0);
static MORPH_SCRATCH: MainCell<Vec<f32>> = MainCell::new(Vec::new());
/// The unique id of each handle whose targets went: the helper keeps them until the data goes.
static MORPH_SENT: MainCell<Option<HashMap<u32, u32>>> = MainCell::new(None);
static MORPH_CHECK: MainCell<(u64, u64, f32)> = MainCell::new((0, 0, 0.0));
/// The check mode's time and weights of the last step of each geometry data.
static MORPH_BEFORE: MainCell<Option<HashMap<usize, (f32, Vec<f32>)>>> = MainCell::new(None);

/// Compares the weighted targets with the positions the engine's morph wrote.
unsafe fn check_morph(t: &MainThread, morph: &Morph, weights: &[f32], geom_data: usize) {
    use crate::gen::layouts::{NiMorphData as MD, NiMorphData_MorphTarget as MT};
    let array: usize = rd(morph.data, MD::arrayMorphTargets);
    let positions: usize = rd(geom_data, GD::vertices);
    if positions == 0 {
        return;
    }
    let distance = |weights: &[f32]| {
        let mut worst = 0.0f32;
        for v in 0..morph.vertices as usize {
            let mut sum = [0.0f32; 3];
            // As `GenMorphInterp` adds: the product and the sum in the x87's wider precision,
            // rounded to a float at the store after each target.
            for (i, &w) in weights.iter().enumerate() {
                let verts: usize = rd(array + i * MT::SIZE, MT::targetVerts);
                let p: [f32; 3] = rd(verts, v * 12);
                for c in 0..3 {
                    sum[c] = (sum[c] as f64 + w as f64 * p[c] as f64) as f32;
                }
            }
            let engine: [f32; 3] = rd(positions, v * 12);
            for c in 0..3 {
                worst = worst.max((sum[c] - engine[c]).abs());
            }
        }
        worst
    };
    let worst = distance(weights);
    // The weights of the step before of the same data, for the log: a lag of one step shows
    // as a small distance to them.
    let before = (*MORPH_BEFORE.get(t)).get_or_insert_with(Default::default).insert(geom_data, (morph.time, weights.to_vec()));
    let check = &mut *MORPH_CHECK.get(t);
    check.0 += 1;
    if worst > 1e-3 {
        check.1 += 1;
        if check.1 <= 6 {
            let relative = rd::<u8>(morph.data, MD::bRelativeTargets);
            let keys: Vec<(i32, i32)> = (0..morph.targets as usize).map(|i| {
                let target = array + i * MT::SIZE;
                (rd(target, MT::numKeys), rd(target, MT::eType))
            }).collect();
            let (time_before, to_before) = before.as_ref().map_or((f32::NAN, f32::NAN), |(time, w)| (*time, distance(w)));
            crate::log!(
                "geometry: morph check {}: data 0x{:x}, {} targets of {} vertices, relative {}, time {} (the step before {}), keys (count, type) {:?}, weights {:?}, the most {} (to the weights before {})",
                check.1, morph.data, morph.targets, morph.vertices, relative, morph.time, time_before, keys, weights, worst, to_before
            );
        }
    }
    check.2 = check.2.max(worst);
}

/// `NiRTTI` of `NiUVController` (`NiUVController::GetRTTI` 0x722950), and its texture set and
/// its state: the U offset, the V offset, the U tiling and the V tiling (0x4C to 0x58;
/// `NiUVController::Update` 0x7220E0).
const RTTI_NI_UV_CONTROLLER: usize = 0x7DF4A4;
const UV_TEXTURE_SET: usize = crate::gen::layouts::NiUVController::textureSet;
const UV_STATE: usize = crate::gen::layouts::NiUVController::UOffset;

/// The texture set and the state of a UV controller.
#[derive(Clone, Copy)]
struct UvState {
    set: u32,
    state: [f32; 4],
}

/// How a geometry under a UV controller goes (`NIRENDER_UV_STEPS`): its new revisions as the
/// controller's state (the default); as whole geometries (`0`, as before); or both (`check`:
/// the plugin compares the step from the last sent coordinates with the engine's).
#[derive(Clone, Copy, PartialEq, Eq)]
enum UvMode {
    On,
    Off,
    Check,
}

fn uv_mode() -> UvMode {
    static MODE: std::sync::OnceLock<UvMode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("NIRENDER_UV_STEPS").as_deref() {
        Ok("0") => UvMode::Off,
        Ok("check") => UvMode::Check,
        _ => UvMode::On,
    })
}

/// The UV controller of `shape` and its state, when it is the only controller of the shape,
/// its target is the shape, the shape draws `geom_data`, and the data has its texture set.
unsafe fn uv_controller(shape: usize, geom_data: usize) -> Option<UvState> {
    if shape == 0 || uv_mode() == UvMode::Off {
        return None;
    }
    let controller: usize = rd(shape, crate::gen::layouts::NiAVObject::controllers);
    if controller == 0 || rd::<usize>(controller, crate::gen::layouts::NiTimeController::nextController) != 0 {
        return None;
    }
    if ni::get_rtti(controller) != RTTI_NI_UV_CONTROLLER || rd::<usize>(controller, crate::gen::layouts::NiTimeController::target) != shape {
        return None;
    }
    if rd::<usize>(shape, crate::gen::layouts::NiGeometry::spModelData) != geom_data {
        return None;
    }
    let set = rd::<u16>(controller, UV_TEXTURE_SET) as u32;
    if set >= rd::<u16>(geom_data, GD::textureSets) as u32 {
        return None;
    }
    Some(UvState { set, state: rd(controller, UV_STATE) })
}

/// Sends the state of a UV controller; `base` when it is the state of the vertices of the
/// `GEOMETRY` record just sent.
unsafe fn send_uv(handle: u32, unique: u32, uv: &UvState, base: bool) {
    let [u, v, tu, tv] = uv.state.map(f32::to_bits);
    crate::capture::geometry_uv(&[handle, unique, uv.set | (base as u32) << 16, u, v, tu, tv]);
}

/// The steps sent as a state, and the check mode's counts: the steps compared, those with a
/// coordinate further than 1/1000 from the engine's, and the largest distance.
static UV_STEPS: MainCell<u64> = MainCell::new(0);
static UV_CHECK: MainCell<(u64, u64, f32)> = MainCell::new((0, 0, 0.0));
/// The check mode's copy of the sent texture set of each geometry, with its state.
static UV_BASES: MainCell<Option<HashMap<u32, ([f32; 4], Vec<[f32; 2]>)>>> = MainCell::new(None);

unsafe fn texture_set(geom_data: usize, set: u32) -> &'static [[f32; 2]] {
    let all = rd::<u16>(geom_data, GD::vertexCount) as usize;
    let coords: usize = rd(geom_data, GD::textureCoords);
    if coords == 0 {
        return &[];
    }
    core::slice::from_raw_parts((coords + set as usize * all * 8) as *const [f32; 2], all)
}

unsafe fn keep_uv_base(t: &MainThread, handle: u32, geom_data: usize, uv: &UvState) {
    let mut uv_bases = UV_BASES.get(t);
    let bases = uv_bases.get_or_insert_with(Default::default);
    bases.insert(handle, (uv.state, texture_set(geom_data, uv.set).to_vec()));
}

/// Compares the step from the kept coordinates to the controller's new state with the
/// engine's coordinates (the closed form of `uv_animation.rs` of the helper).
unsafe fn check_uv(t: &MainThread, handle: u32, geom_data: usize, uv: &UvState) {
    let uv_bases = UV_BASES.get(t);
    let Some((base, kept)) = uv_bases.as_ref().and_then(|bases| bases.get(&handle)) else { return };
    let [base_u, base_v, base_tu, base_tv] = *base;
    let [now_u, now_v, now_tu, now_tv] = uv.state;
    if base_tu == 0.0 || base_tv == 0.0 {
        return;
    }
    let (ru, rv) = (now_tu / base_tu, now_tv / base_tv);
    let (bu, bv) = ((1.0 - ru) * 0.5 - (now_u - ru * base_u), (1.0 - rv) * 0.5 + (now_v - rv * base_v));
    let engine = texture_set(geom_data, uv.set);
    let mut worst = 0.0f32;
    for (k, e) in kept.iter().zip(engine) {
        worst = worst.max((ru * k[0] + bu - e[0]).abs()).max((rv * k[1] + bv - e[1]).abs());
    }
    let check = &mut *UV_CHECK.get(t);
    check.0 += 1;
    if worst > 1e-3 {
        check.1 += 1;
    }
    check.2 = check.2.max(worst);
}

/// A geometry data of the census (`NIRENDER_GEOMETRY_CENSUS=1`), by its handle and unique id:
/// its sends after the first, with a new revision only or with other changes (the counts, the
/// skin data, or a send after the version was forgotten), the bytes of these sends, and the
/// shape that drew it first: its name, the names of two nodes above it, and the classes of
/// its controllers.
struct CensusEntry {
    revisions: u64,
    others: u64,
    bytes: u64,
    path: String,
    controllers: String,
}

static CENSUS: MainCell<Option<HashMap<(u32, u32), CensusEntry>>> = MainCell::new(None);

fn census_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("NIRENDER_GEOMETRY_CENSUS").as_deref() == Ok("1"))
}

/// The class names of the controllers on `object`, joined with '+', or "none".
unsafe fn controller_classes(object: usize) -> String {
    let mut names = Vec::new();
    let mut controller: usize = rd(object, crate::gen::layouts::NiAVObject::controllers);
    while controller != 0 && names.len() < 8 {
        let rtti = ni::get_rtti(controller);
        let name: usize = if rtti != 0 { rd(rtti, crate::gen::layouts::NiRTTI::name) } else { 0 };
        names.push(if name != 0 { core::ffi::CStr::from_ptr(name as *const core::ffi::c_char).to_string_lossy().into_owned() } else { String::from("?") });
        controller = rd(controller, crate::gen::layouts::NiTimeController::nextController);
    }
    if names.is_empty() { String::from("none") } else { names.join("+") }
}

/// Counts a send of a geometry data for the census: the first send makes its entry; a later
/// one counts as a revision when only the revision changed.
unsafe fn census_note(t: &MainThread, handle: u32, version: Version, geom_data: usize, skin: usize, shape: usize) {
    let before = versions(t).get(&handle).map(|sent| sent.version).filter(|sent| sent.unique == version.unique);
    let mut census_open = CENSUS.get(t);
    let census = census_open.get_or_insert_with(Default::default);
    let key = (handle, version.unique);
    if !census.contains_key(&key) {
        census.insert(key, {
            let parent: usize = if shape != 0 { rd(shape, crate::gen::layouts::NiAVObject::parentNode) } else { 0 };
            let above: usize = if parent != 0 { rd(parent, crate::gen::layouts::NiAVObject::parentNode) } else { 0 };
            let path = format!(
                "{} < {} < {}",
                crate::ports::draw::object_name(shape),
                crate::ports::draw::object_name(parent),
                crate::ports::draw::object_name(above)
            );
            let mut controllers = if shape != 0 { controller_classes(shape) } else { String::from("?") };
            if skin != 0 {
                controllers.push_str(", skinned");
            }
            CensusEntry { revisions: 0, others: 0, bytes: 0, path, controllers }
        });
        return;
    }
    let all = rd::<u16>(geom_data, GD::vertexCount) as u64;
    let per_vertex = 12
        + if rd::<usize>(geom_data, GD::normals) != 0 { 12 } else { 0 }
        + if rd::<usize>(geom_data, GD::colors) != 0 { 4 } else { 0 }
        + 8 * (rd::<u16>(geom_data, GD::textureSets) as u64).min(8);
    let bytes = all * per_vertex + version.entries as u64 * 2;
    if let Some(entry) = census.get_mut(&key) {
        let only_revision = before.is_some_and(|before| before.vertices == version.vertices && before.entries == version.entries && before.skin_data == version.skin_data);
        if only_revision {
            entry.revisions += 1;
        } else {
            entry.others += 1;
        }
        entry.bytes += bytes;
    }
}

/// The census since the last mark, for the log, and a new count from here: the entries stay,
/// their sends start again at 0. Empty without `NIRENDER_GEOMETRY_CENSUS=1`.
pub unsafe fn census_take(t: &MainThread) -> Vec<String> {
    let lines = census_report(t);
    if let Some(census) = (*CENSUS.get(t)).as_mut() {
        for entry in census.values_mut() {
            entry.revisions = 0;
            entry.others = 0;
            entry.bytes = 0;
        }
    }
    lines
}

/// The census for the log: the sends again by the classes of the controllers, then the 20
/// geometries sent again most. Empty without `NIRENDER_GEOMETRY_CENSUS=1`.
pub unsafe fn census_report(t: &MainThread) -> Vec<String> {
    let census_open = CENSUS.get(t);
    let Some(census) = census_open.as_ref() else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    let mut by_class: std::collections::HashMap<&str, (u64, u64, u64, u64)> = Default::default();
    for entry in census.values() {
        let sum = by_class.entry(entry.controllers.as_str()).or_default();
        sum.0 += 1;
        if entry.revisions + entry.others > 0 {
            sum.1 += 1;
        }
        sum.2 += entry.revisions + entry.others;
        sum.3 += entry.bytes;
    }
    let total: u64 = by_class.values().map(|sum| sum.3).sum();
    let mut classes: Vec<_> = by_class.into_iter().collect();
    classes.sort_by_key(|(_, sum)| std::cmp::Reverse(sum.3));
    lines.push(format!("geometry census: {} geometries, sent again {} times, {:.1} MB; by the controllers of the shape:", census.len(), classes.iter().map(|(_, sum)| sum.2).sum::<u64>(), total as f64 / 1e6));
    for (class, (geometries, again, sends, bytes)) in classes.iter().take(16) {
        lines.push(format!("  {class}: {geometries} geometries, {again} of them sent again, {sends} times, {:.1} MB ({:.1} %)", *bytes as f64 / 1e6, *bytes as f64 * 100.0 / total.max(1) as f64));
    }
    let mut most: Vec<_> = census.values().filter(|entry| entry.revisions + entry.others > 0).collect();
    most.sort_by_key(|entry| std::cmp::Reverse(entry.bytes));
    lines.push(String::from("geometry census: the 20 geometries with the most bytes sent again (revision only, other changes, MB, controllers, shape < parent < above):"));
    for entry in most.iter().take(20) {
        lines.push(format!("  {} {} {:.1} [{}] {}", entry.revisions, entry.others, entry.bytes as f64 / 1e6, entry.controllers, entry.path));
    }
    lines
}

/// Sends a geometry record and notes its version. `lengths` has the strip lengths, padded
/// to 4 bytes.
unsafe fn send(t: &MainThread, handle: u32, stream: u32, version: Version, fvf: u32, stride: u32, strip_count: u32, lengths: &[u8], bytes: &[u8], index_bytes: &[u8]) {
    crate::capture::geometry(&[stream, version.unique, version.revision as u32, fvf, stride, version.vertices, version.entries, strip_count], lengths, bytes, index_bytes);
    versions(t).insert(handle, Sent { version, used: crate::camera_space::frame(t), uv_set: None, morph: false });
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
    let stream = stream_handle(t, handle, version.unique);
    if known(t, handle, version) {
        return stream;
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
    send(t, handle, stream, version, fvf, stride as u32, 0, &[], &bytes, &index_bytes);
    stream
}

/// The geometry of a screen polygon (`RenderScreenPoly`) for this draw, as
/// `NiDX8VertexBufferManager::PackScreenPolyVB` packs it: each vertex at `(x * d, y * d, d)`
/// with `d = distance * z + near` (the polygon is at that depth under the screen projection
/// of the renderer), then its colour and its texture coordinate when the polygon has them.
/// The fan of the polygon is a triangle list in the record. The record goes out at each
/// draw: the vertices of a polygon can change. Returns the handle (the serial of the vertex
/// array of the polygon), or 0 for a polygon with fewer than three vertices.
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
    let stream = stream_handle(t, handle, 0);
    send(t, handle, stream, version, fvf, stride as u32, 0, &[], &bytes, &index_bytes);
    // The next draw of the polygon sends it again.
    versions(t).remove(&handle);
    stream
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
    use super::{has_bone_weights, pack_weights, same_influences, scaled_time, skin_data_dying, unitize, unpacked};

    /// A time controller in memory with these fields (`NiTimeController` layout).
    fn controller(flags: u16, phase: f32, low: f32, high: f32, last_scaled: f32) -> Vec<u32> {
        use crate::gen::layouts::NiTimeController as TC;
        let mut words = vec![0u32; TC::SIZE / 4];
        words[TC::flags / 4] = flags as u32;
        words[TC::phase / 4] = phase.to_bits();
        words[TC::lowKeyFrame / 4] = low.to_bits();
        words[TC::highKeyFrame / 4] = high.to_bits();
        words[TC::lastScaledTime / 4] = last_scaled.to_bits();
        words
    }

    fn time(words: &[u32]) -> f32 {
        unsafe { scaled_time(words.as_ptr() as usize) }
    }

    #[test]
    fn a_loop_wraps_the_scaled_sum_between_the_key_times() {
        // The morph of the test level's exhibit 088: keys from 0 to 8.33, a loop (cycle 0).
        let t = time(&controller(8, 0.0, 0.0, 25.0 / 3.0, 65.16));
        assert!((t - (65.16 - 7.0 * 25.0 / 3.0)).abs() < 1e-3, "{t}");
        // The phase moves it; a sum below the low key time wraps up.
        let t = time(&controller(8, 1.0, 2.0, 4.0, 0.5));
        assert!((t - 3.5).abs() < 1e-5, "{t}");
    }

    #[test]
    fn reverse_goes_back_from_the_high_key_time() {
        // Cycle 1 (flags bits 1-2 = 1): 0 to 2, then back.
        assert!((time(&controller(2, 0.0, 0.0, 2.0, 1.5)) - 1.5).abs() < 1e-5);
        assert!((time(&controller(2, 0.0, 0.0, 2.0, 2.5)) - 1.5).abs() < 1e-5);
    }

    #[test]
    fn clamp_holds_the_time_between_the_key_times() {
        // Cycle 2.
        assert_eq!(time(&controller(4, 0.0, 1.0, 3.0, 7.0)), 3.0);
        assert_eq!(time(&controller(4, 0.0, 1.0, 3.0, 0.25)), 1.0);
        assert_eq!(time(&controller(4, 0.0, 1.0, 3.0, 2.0)), 2.0);
    }

    /// A skin data with 2 bones: the first with 3 vertices and its array, the second with
    /// `second_count` vertices and `second_array`.
    fn skin_data_with_bones(second_count: u16, second_array: u32) -> (Vec<u32>, Vec<u32>, Vec<u32>) {
        let mut weights = vec![0u32; 6];
        let mut bones = vec![0u32; 0x4C / 4 * 2];
        bones[0x44 / 4] = weights.as_mut_ptr() as u32;
        bones[0x48 / 4] = 3;
        bones[(0x4C + 0x44) / 4] = second_array;
        bones[(0x4C + 0x48) / 4] = second_count as u32;
        let mut data = vec![0u32; 0x48 / 4];
        data[0x40 / 4] = 2;
        data[0x44 / 4] = bones.as_mut_ptr() as u32;
        (data, bones, weights)
    }

    #[test]
    fn a_bone_with_a_count_and_no_weight_array_is_a_dying_skin_data() {
        let mut storage = vec![0u32; 2];
        let array = storage.as_mut_ptr() as u32;
        // (the count of the second bone, its array, dying)
        for (count, second, dying) in [(1, array, false), (1, 0, true), (0, 0, false), (0, array, false)] {
            let (mut data, _bones, _weights) = skin_data_with_bones(count, second);
            let at = data.as_mut_ptr() as usize;
            assert_eq!(unsafe { skin_data_dying(at) }, dying, "count {count}, array 0x{second:x}");
            assert_eq!(unsafe { has_bone_weights(at) }, !dying, "count {count}, array 0x{second:x}");
        }
        // The destructor's end state: every array null.
        let (mut data, mut bones, _weights) = skin_data_with_bones(1, 0);
        bones[0x44 / 4] = 0;
        assert!(unsafe { skin_data_dying(data.as_mut_ptr() as usize) });
        // No skin data, and a skin data without bones, are not dying.
        assert!(!unsafe { skin_data_dying(0) });
        data[0x40 / 4] = 0;
        assert!(!unsafe { skin_data_dying(data.as_mut_ptr() as usize) });
    }

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
