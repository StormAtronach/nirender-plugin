//! The capture of the render link stream into a file.
//!
//! The file has each command that the plugin sends to the host of the render link, and the
//! bytes of each resource: the pixels of a texture and the contents of a vertex or index
//! buffer. A renderer without the game can draw the frames of the file again. The file is
//! the first draft of a stream between the game and a renderer in another process.
//!
//! A capture has two parts. In the first part, the warm-up, the module notes the geometry
//! that the game draws. At the end of the warm-up it makes the renderer drop its textures
//! and the buffers of that geometry. The game makes them again in the next frame, and so the
//! second part, the record, has the bytes of each resource that its frames use.
//!
//! The capture needs the Rust code, the textures of the link, and all facts and kinds of the
//! link in use: only then is each resource call and each draw a call of the link.
//!
//! The file: the 8 bytes `NIRCAP01`, then records. A record is a tag (u32), the length of its
//! data in bytes (u32), and the data. All numbers are little endian. The tags are in `tag`.

use core::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufWriter, Write};

use crate::link::{
    DrawStateV1, DrawV1, SceneV1, StateLightV1, TextureV1, CALL_CREATE_DEPTH_BUFFER, CALL_CREATE_IMAGE_SURFACE, CALL_CREATE_INDEX_BUFFER,
    CALL_CREATE_TEXTURE, CALL_CREATE_VERTEX_BUFFER, CALL_CUBE_FACE, CALL_GET_DEPTH_BUFFER, CALL_GET_RENDER_TARGET, CALL_PRESENT,
    CALL_TEXTURE_LEVEL, CALL_VIEWPORT, COMMAND_DRAW, COMMAND_DRAW_INDEXED, COMMAND_TRANSFORM,
};
use crate::mem::rd;

pub const MAGIC: &[u8; 8] = b"NIRCAP01";

/// The tags of the records.
pub mod tag {
    /// handle, flags, width, height, levels, D3D format, pixel format, pixel format tag, bytes
    /// for each pixel; then levels + 1 offsets, the widths and the heights of the levels; then
    /// the pixels.
    pub const TEXTURE: u32 = 1;
    /// handle, offset, size; then the bytes that the game wrote into the buffer.
    pub const BUFFER_DATA: u32 = 2;
    /// A `MgeLinkSceneV1`.
    pub const SCENE_BEGIN: u32 = 3;
    pub const SCENE_END: u32 = 4;
    /// kind, a, b
    pub const FRAME_EVENT: u32 = 5;
    /// The object of the draws that follow: kind, class, flags, geometry, then its name.
    pub const OBJECT: u32 = 6;
    /// A `MgeLinkDrawStateV1`, then its lights.
    pub const DRAW_STATE: u32 = 7;
    /// command, a, b, c; then the data at an address: 16 floats for a transform, the words
    /// of a draw.
    pub const COMMAND: u32 = 8;
    /// call, result, the handle that the call gave (or 0), the number of arguments, the
    /// arguments; then the data at an address: the viewport.
    pub const CALL: u32 = 9;
    /// The frame is complete.
    pub const FRAME_END: u32 = 10;
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Off,
    /// The number of frames that are left of the warm-up.
    Warmup(u32),
    /// The number of frames that are left of the record.
    Record(u32),
}

struct State {
    phase: Phase,
    frames: u32,
    out: Option<BufWriter<File>>,
    /// The geometry data and the skin instances of the draws of the warm-up.
    geometry: HashSet<(u32, u32)>,
    records: u64,
    bytes: u64,
    failed: bool,
}

static mut STATE: Option<State> = None;
/// The warm-up or the record is in progress.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// The record is in progress.
static RECORDING: AtomicBool = AtomicBool::new(false);

unsafe fn state() -> Option<&'static mut State> {
    (*core::ptr::addr_of_mut!(STATE)).as_mut()
}

#[inline]
pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

#[inline]
pub fn recording() -> bool {
    RECORDING.load(Ordering::Relaxed)
}

/// Starts a capture into `path`: `warmup` frames, then `frames` frames of record.
pub unsafe fn start(path: &str, warmup: u32, frames: u32) -> Result<(), String> {
    if active() {
        return Err("a capture is in progress".into());
    }
    let file = File::create(path).map_err(|error| format!("{path}: {error}"))?;
    let mut out = BufWriter::with_capacity(1 << 20, file);
    out.write_all(MAGIC).map_err(|error| error.to_string())?;
    *core::ptr::addr_of_mut!(STATE) = Some(State {
        phase: Phase::Warmup(warmup.max(1)),
        frames: frames.max(1),
        out: Some(out),
        geometry: HashSet::new(),
        records: 0,
        bytes: 8,
        failed: false,
    });
    // The recorder on the buffers gives their bytes at each unlock.
    crate::resource::enable(true);
    ACTIVE.store(true, Ordering::Relaxed);
    crate::log!("capture: start, {warmup} frames of warm-up, {frames} frames of record, into {path}");
    Ok(())
}

/// One line about the capture, for the scenario: the phase, the records and the bytes.
pub unsafe fn status() -> String {
    match state() {
        None => "off 0 0".into(),
        Some(s) => {
            let phase = match s.phase {
                Phase::Off => "done",
                Phase::Warmup(_) => "warmup",
                Phase::Record(_) => "record",
            };
            format!("{phase} {} {}{}", s.records, s.bytes, if s.failed { " FAILED" } else { "" })
        }
    }
}

unsafe fn write(tag: u32, parts: &[&[u8]]) {
    if !recording() {
        return;
    }
    let Some(s) = state() else { return };
    let Some(out) = s.out.as_mut() else { return };
    let length: usize = parts.iter().map(|p| p.len()).sum();
    let mut ok = out.write_all(&tag.to_le_bytes()).is_ok() && out.write_all(&(length as u32).to_le_bytes()).is_ok();
    for part in parts {
        ok = ok && out.write_all(part).is_ok();
    }
    s.failed |= !ok;
    s.records += 1;
    s.bytes += 8 + length as u64;
}

fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

unsafe fn bytes_of<T>(value: &T) -> &[u8] {
    core::slice::from_raw_parts(value as *const T as *const u8, core::mem::size_of::<T>())
}

unsafe fn bytes_at(address: u32, length: usize) -> &'static [u8] {
    if address == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(address as usize as *const u8, length)
    }
}

pub unsafe fn scene_begin(scene: &SceneV1) {
    write(tag::SCENE_BEGIN, &[bytes_of(scene)]);
}

pub unsafe fn scene_end() {
    write(tag::SCENE_END, &[]);
}

pub unsafe fn frame_event(kind: u32, a: u32, b: u32) {
    write(tag::FRAME_EVENT, &[&words(&[kind, a, b])]);
}

/// The object packet of the draws that follow.
pub unsafe fn object(draw: &DrawV1) {
    if !active() {
        return;
    }
    if !recording() {
        if let Some(s) = state() {
            s.geometry.insert((draw.geometry_data, draw.skin_instance));
        }
        return;
    }
    // The name of a NiObjectNET is a C string pointer at offset 8.
    let mut name: &[u8] = &[];
    if draw.geometry != 0 {
        let text: usize = rd(draw.geometry as usize, 8);
        if text != 0 {
            name = core::ffi::CStr::from_ptr(text as *const core::ffi::c_char).to_bytes();
        }
    }
    write(tag::OBJECT, &[&words(&[draw.kind, draw.object_class, draw.flags, draw.geometry]), &name[..name.len().min(96)]]);
}

pub unsafe fn draw_state(packet: &DrawStateV1) {
    if !active() {
        return;
    }
    // A buffer that the game made before the capture gets the recorder here.
    crate::resource::wrap(packet.vertex_buffer as usize);
    crate::resource::wrap(packet.index_buffer as usize);
    let lights = if packet.lights.is_null() {
        &[]
    } else {
        core::slice::from_raw_parts(packet.lights as *const u8, packet.light_count as usize * core::mem::size_of::<StateLightV1>())
    };
    write(tag::DRAW_STATE, &[bytes_of(packet), lights]);
}

pub unsafe fn command(command: u32, a: u32, b: u32, c: u32) {
    if !recording() {
        return;
    }
    let data: &[u8] = match command {
        COMMAND_TRANSFORM => bytes_at(b, 64),
        COMMAND_DRAW_INDEXED => bytes_at(a, 20),
        COMMAND_DRAW => bytes_at(a, 12),
        _ => &[],
    };
    write(tag::COMMAND, &[&words(&[command, a, b, c]), data]);
}

/// A call of the link, after the host did it.
pub unsafe fn call(index: u32, args: &[u32], result: u32) {
    if !active() {
        return;
    }
    // The argument that is the address for the handle that the call gives.
    let out_argument = match index {
        CALL_CREATE_VERTEX_BUFFER | CALL_CREATE_INDEX_BUFFER | CALL_CREATE_DEPTH_BUFFER => Some(4),
        CALL_CREATE_TEXTURE => Some(6),
        CALL_CREATE_IMAGE_SURFACE | CALL_CUBE_FACE => Some(3),
        CALL_TEXTURE_LEVEL => Some(2),
        CALL_GET_RENDER_TARGET | CALL_GET_DEPTH_BUFFER => Some(0),
        _ => None,
    };
    let mut handle = 0u32;
    if let Some(i) = out_argument {
        if (result as i32) >= 0 && args.len() > i && args[i] != 0 {
            handle = rd(args[i] as usize, 0);
        }
    }
    if index == CALL_CREATE_VERTEX_BUFFER || index == CALL_CREATE_INDEX_BUFFER {
        crate::resource::wrap(handle as usize);
    }
    if recording() {
        let mut head = vec![index, result, handle, args.len() as u32];
        head.extend_from_slice(args);
        let data: &[u8] = if index == CALL_VIEWPORT && !args.is_empty() { bytes_at(args[0], 24) } else { &[] };
        write(tag::CALL, &[&words(&head), data]);
    }
    if index == CALL_PRESENT {
        frame_end();
    }
}

/// A texture that the host made from these pixels.
pub unsafe fn texture(texture: &TextureV1, handle: usize) {
    if !recording() {
        return;
    }
    let levels = texture.levels as usize;
    let offsets = core::slice::from_raw_parts(texture.level_offsets, levels + 1);
    let widths = core::slice::from_raw_parts(texture.level_widths, levels);
    let heights = core::slice::from_raw_parts(texture.level_heights, levels);
    let head = [
        handle as u32,
        texture.flags,
        texture.width,
        texture.height,
        texture.levels,
        texture.d3d_format,
        texture.pixel_format,
        texture.pixel_format_tag,
        texture.bytes_per_pixel,
    ];
    let pixels = core::slice::from_raw_parts(texture.pixels, offsets[levels] as usize);
    write(tag::TEXTURE, &[&words(&head), &words(offsets), &words(widths), &words(heights), pixels]);
}

/// The bytes that the game wrote into a vertex or index buffer, at its unlock.
pub unsafe fn buffer_data(handle: usize, offset: u32, bytes: &[u8]) {
    write(tag::BUFFER_DATA, &[&words(&[handle as u32, offset, bytes.len() as u32]), bytes]);
}

/// `NiSkinInstance::skinData`, `NiSkinData::skinPartition`, and a `NiSkinPartition`.
const SKIN_INSTANCE_DATA: usize = 0x8;
const SKIN_DATA_PARTITION: usize = 0x8;
const PARTITION_COUNT: usize = 0x8;
const PARTITIONS: usize = 0xC;
const PARTITION_SIZE: usize = 0x2C;

/// Makes the renderer drop its textures and the buffers of the geometry of the warm-up.
unsafe fn purge(s: &mut State) {
    let renderer = crate::engine::renderer();
    if renderer == 0 {
        return;
    }
    crate::ports::textures::renderer_purge_all_textures(renderer, 0);
    let mut skins = 0;
    for &(geometry_data, skin_instance) in &s.geometry {
        if geometry_data != 0 {
            crate::ports::geometry::purge_geometry_data(renderer, geometry_data as usize);
        }
        if skin_instance == 0 {
            continue;
        }
        let skin_data: usize = rd(skin_instance as usize, SKIN_INSTANCE_DATA);
        let partition: usize = if skin_data != 0 { rd(skin_data, SKIN_DATA_PARTITION) } else { 0 };
        if partition == 0 {
            continue;
        }
        let count: u32 = rd(partition, PARTITION_COUNT);
        let partitions: usize = rd(partition, PARTITIONS);
        for i in 0..count as usize {
            crate::ports::geometry::purge_skin_partition(renderer, partitions + i * PARTITION_SIZE);
        }
        skins += 1;
    }
    crate::log!("capture: the renderer dropped its textures and the buffers of {} objects, {skins} of them with a skin", s.geometry.len());
}

/// The game showed a frame.
unsafe fn frame_end() {
    let Some(s) = state() else { return };
    match s.phase {
        Phase::Off => {}
        Phase::Warmup(left) => {
            if left > 1 {
                s.phase = Phase::Warmup(left - 1);
            } else {
                purge(s);
                s.phase = Phase::Record(s.frames);
                RECORDING.store(true, Ordering::Relaxed);
            }
        }
        Phase::Record(left) => {
            write(tag::FRAME_END, &[]);
            if left > 1 {
                s.phase = Phase::Record(left - 1);
                return;
            }
            RECORDING.store(false, Ordering::Relaxed);
            ACTIVE.store(false, Ordering::Relaxed);
            s.phase = Phase::Off;
            if let Some(mut out) = s.out.take() {
                s.failed |= out.flush().is_err();
            }
            crate::log!("capture: end, {} records, {} bytes{}", s.records, s.bytes, if s.failed { ", with a write error" } else { "" });
        }
    }
}
