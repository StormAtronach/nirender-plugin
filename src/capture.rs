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
//! The capture can also go into a ring in shared memory, without an end: a stream. A host
//! program in another process reads the ring and draws each frame while the game runs. The
//! ring has the same records as the file. In a stream, a draw can use a buffer that the game
//! made before the stream and that was not in view at its start. The module makes the
//! renderer drop the buffers of such an object after the frame, so the next frame has them.
//!
//! The file: the 8 bytes `NIRCAP01`, then records. A record is a tag (u32), the length of its
//! data in bytes (u32), and the data. All numbers are little endian. The tags are in `tag`.

use core::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufWriter, Write};

use crate::link::{
    DrawStateV1, DrawV1, SceneV1, StateLightV1, TextureV1, CALL_COPY_PIXELS, CALL_CREATE_DEPTH_BUFFER, CALL_CREATE_IMAGE_SURFACE, CALL_CREATE_INDEX_BUFFER,
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
    /// The first record: the width and the height of the back buffer, and the window of
    /// the game.
    pub const HEADER: u32 = 11;
    /// The game wrote a level of a texture: handle, level, width and height of the level,
    /// D3D format, size; then the bytes.
    pub const TEXTURE_UPDATE: u32 = 12;
    /// The game asks for the pixels of a target: the number of the request, and the texture
    /// (0 for the back buffer). Only in a stream. The host puts the answer into the reply
    /// memory.
    pub const READ_PIXELS: u32 = 13;
    /// The host saves the picture of the frame in progress into the file with this name.
    pub const SHOT: u32 = 14;
}

// ---- the ring in shared memory

extern "system" {
    fn CreateFileMappingA(file: isize, security: *const u8, protect: u32, size_high: u32, size_low: u32, name: *const u8) -> isize;
    fn MapViewOfFile(mapping: isize, access: u32, offset_high: u32, offset_low: u32, bytes: usize) -> *mut u8;
    fn UnmapViewOfFile(address: *const u8) -> i32;
    fn CloseHandle(handle: isize) -> i32;
    fn QueryPerformanceCounter(count: *mut i64) -> i32;
    fn QueryPerformanceFrequency(frequency: *mut i64) -> i32;
    fn Sleep(milliseconds: u32);
}

fn ticks() -> i64 {
    let mut count = 0;
    unsafe { QueryPerformanceCounter(&mut count) };
    count
}

fn ticks_per_second() -> i64 {
    let mut frequency = 0;
    unsafe { QueryPerformanceFrequency(&mut frequency) };
    frequency.max(1)
}

/// The head of the shared memory: `RING_MAGIC`, the number of data bytes, the bytes that the
/// game wrote (u64), the bytes that the host read (u64), and 1 when the game closed the
/// stream. The data starts at `RING_HEAD`. A record can go around the end of the data.
pub const RING_MAGIC: u32 = 0x474E_5231;
pub const RING_HEAD: usize = 64;
const RING_WRITTEN: usize = 8;
const RING_READ: usize = 16;
const RING_CLOSED: usize = 24;
/// The game waits this long for room in the ring. Then it ends the stream: the host is gone.
const RING_WAIT_SECONDS: i64 = 10;

struct Ring {
    mapping: isize,
    base: *mut u8,
    capacity: u64,
    /// The time that the game waited for room, in ticks.
    waited: i64,
}

impl Ring {
    unsafe fn create(name: &str, megabytes: u32) -> Result<Ring, String> {
        let capacity = megabytes.clamp(1, 512) as u64 * (1 << 20);
        let mut text = name.as_bytes().to_vec();
        text.push(0);
        // INVALID_HANDLE_VALUE: memory of the page file. PAGE_READWRITE is 4.
        let mapping = CreateFileMappingA(-1, core::ptr::null(), 4, 0, (capacity as usize + RING_HEAD) as u32, text.as_ptr());
        if mapping == 0 {
            return Err(format!("no shared memory with the name {name}"));
        }
        // FILE_MAP_ALL_ACCESS
        let base = MapViewOfFile(mapping, 0xF001F, 0, 0, 0);
        if base.is_null() {
            CloseHandle(mapping);
            return Err("the shared memory does not fit into the address space".into());
        }
        core::ptr::write_bytes(base, 0, RING_HEAD);
        (base as *mut u32).write(RING_MAGIC);
        (base.add(4) as *mut u32).write(capacity as u32);
        Ok(Ring { mapping, base, capacity, waited: 0 })
    }

    unsafe fn counter(&self, offset: usize) -> &core::sync::atomic::AtomicU64 {
        &*(self.base.add(offset) as *const core::sync::atomic::AtomicU64)
    }

    unsafe fn copy_in(&self, at: u64, bytes: &[u8]) {
        let start = (at % self.capacity) as usize;
        let first = bytes.len().min(self.capacity as usize - start);
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), self.base.add(RING_HEAD + start), first);
        core::ptr::copy_nonoverlapping(bytes.as_ptr().add(first), self.base.add(RING_HEAD), bytes.len() - first);
    }

    /// Puts one record into the ring. Returns false when the host did not make room in time.
    unsafe fn push(&mut self, tag: u32, parts: &[&[u8]]) -> bool {
        let length: usize = parts.iter().map(|p| p.len()).sum();
        let total = 8 + length as u64;
        if total > self.capacity {
            crate::log!("stream: a record of {length} bytes does not fit into the ring");
            return true;
        }
        let written = self.counter(RING_WRITTEN).load(Ordering::Relaxed);
        if self.capacity - (written - self.counter(RING_READ).load(Ordering::Acquire)) < total {
            let start = ticks();
            while self.capacity - (written - self.counter(RING_READ).load(Ordering::Acquire)) < total {
                if ticks() - start > RING_WAIT_SECONDS * ticks_per_second() {
                    return false;
                }
                Sleep(0);
            }
            self.waited += ticks() - start;
        }
        let mut at = written;
        self.copy_in(at, &tag.to_le_bytes());
        self.copy_in(at + 4, &(length as u32).to_le_bytes());
        at += 8;
        for part in parts {
            self.copy_in(at, part);
            at += part.len() as u64;
        }
        self.counter(RING_WRITTEN).store(at, Ordering::Release);
        true
    }

    unsafe fn close(&mut self) {
        (self.base.add(RING_CLOSED) as *mut u32).write_volatile(1);
        UnmapViewOfFile(self.base);
        CloseHandle(self.mapping);
    }
}

/// The reply memory of a stream: shared memory with the name of the ring and `.reply`. The
/// host writes the answer to a `READ_PIXELS` record into it: the number of the request (the
/// host writes it last), the width, the height, one unused word, and then the pixels with 4
/// bytes each, blue first. A width of 0 says that the host has no such target.
pub const REPLY_HEAD: usize = 16;
/// The game waits this long for an answer.
const REPLY_WAIT_SECONDS: i64 = 5;

struct Reply {
    mapping: isize,
    base: *mut u8,
    capacity: usize,
    requests: u32,
    /// The time that the game waited for answers, in ticks.
    waited: i64,
}

impl Reply {
    unsafe fn create(name: &str, capacity: usize) -> Option<Reply> {
        let mut text = format!("{name}.reply").into_bytes();
        text.push(0);
        let mapping = CreateFileMappingA(-1, core::ptr::null(), 4, 0, (capacity + REPLY_HEAD) as u32, text.as_ptr());
        if mapping == 0 {
            return None;
        }
        let base = MapViewOfFile(mapping, 0xF001F, 0, 0, 0);
        if base.is_null() {
            CloseHandle(mapping);
            return None;
        }
        core::ptr::write_bytes(base, 0, REPLY_HEAD);
        Some(Reply { mapping, base, capacity, requests: 0, waited: 0 })
    }

    unsafe fn close(&mut self) {
        UnmapViewOfFile(self.base);
        CloseHandle(self.mapping);
    }
}

/// Asks the host of the stream for the pixels of a rendered texture, or of the back buffer
/// with `texture` 0. Gives the width, the height and the pixels: 4 bytes each, blue first.
/// `None` without a stream, or when the host has no answer in time.
pub unsafe fn read_pixels(texture: u32) -> Option<(u32, u32, &'static [u8])> {
    if !recording() {
        return None;
    }
    let s = state()?;
    let request = {
        let reply = s.reply.as_mut()?;
        reply.requests += 1;
        reply.requests
    };
    write(tag::READ_PIXELS, &[&words(&[request, texture])]);
    // The write can end the stream.
    let reply = state()?.reply.as_mut()?;
    let start = ticks();
    while (reply.base as *const u32).read_volatile() != request {
        if ticks() - start > REPLY_WAIT_SECONDS * ticks_per_second() {
            crate::log!("stream: no answer to the pixel read {request} of the texture {texture:#x}");
            return None;
        }
        Sleep(0);
    }
    reply.waited += ticks() - start;
    let (width, height) = ((reply.base.add(4) as *const u32).read_volatile(), (reply.base.add(8) as *const u32).read_volatile());
    let size = width as usize * height as usize * 4;
    if size == 0 || size > reply.capacity {
        return None;
    }
    Some((width, height, core::slice::from_raw_parts(reply.base.add(REPLY_HEAD), size)))
}

enum Sink {
    File(BufWriter<File>),
    Ring(Ring),
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
    /// The frames of the record. 0 for a stream: it has no end.
    frames: u32,
    /// At the end of the warm-up the plugin becomes the host of the render link.
    own_host: bool,
    out: Option<Sink>,
    reply: Option<Reply>,
    /// The geometry data and the skin instances of the draws of the warm-up.
    geometry: HashSet<(u32, u32)>,
    /// The buffers whose bytes are in the capture.
    known_buffers: HashSet<u32>,
    /// The image surfaces: surfaces in system memory, where the game changes pixels.
    image_surfaces: HashSet<u32>,
    /// The object of the draws that follow, and the objects to drop the buffers of after this
    /// frame: a draw used a buffer that is not in the capture.
    object: (u32, u32),
    late: HashSet<(u32, u32)>,
    dropped_late: HashSet<(u32, u32)>,
    records: u64,
    bytes: u64,
    frames_done: u32,
    /// The time of the game thread in this module during the record, in ticks.
    spent: i64,
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
    begin(Sink::File(out), warmup, frames.max(1));
    crate::log!("capture: start, {warmup} frames of warm-up, {frames} frames of record, into {path}");
    Ok(())
}

/// Starts a stream into a ring of `megabytes` in shared memory with the name `name`. It
/// goes on until `stop`, or until the host does not read for some seconds.
///
/// With `own_host`, the plugin becomes the host of the render link at the end of the warm-up:
/// from then on no call of the game gets to a D3D device, and the stream is the only picture.
pub unsafe fn start_stream(name: &str, megabytes: u32, warmup: u32, own_host: bool) -> Result<(), String> {
    if active() {
        return Err("a capture is in progress".into());
    }
    begin(Sink::Ring(Ring::create(name, megabytes)?), warmup, 0);
    if let Some(s) = state() {
        s.own_host = own_host;
        // Room for the back buffer, and for a rendered texture of 1024 x 1024.
        let renderer = crate::engine::renderer();
        let size: [u32; 2] = if renderer != 0 { rd(renderer, BACK_BUFFER_SIZE) } else { [0, 0] };
        s.reply = Reply::create(name, (size[0] as usize * size[1] as usize * 4).max(1024 * 1024 * 4));
    }
    crate::log!("stream: start, {warmup} frames of warm-up, a ring of {megabytes} MB with the name {name}, own host {own_host}");
    Ok(())
}

unsafe fn begin(out: Sink, warmup: u32, frames: u32) {
    *core::ptr::addr_of_mut!(STATE) = Some(State {
        phase: Phase::Warmup(warmup.max(1)),
        frames,
        own_host: false,
        out: Some(out),
        reply: None,
        geometry: HashSet::new(),
        known_buffers: HashSet::new(),
        image_surfaces: HashSet::new(),
        object: (0, 0),
        late: HashSet::new(),
        dropped_late: HashSet::new(),
        records: 0,
        bytes: 8,
        frames_done: 0,
        spent: 0,
        failed: false,
    });
    // The recorder on the buffers gives their bytes at each unlock.
    crate::resource::enable(true);
    ACTIVE.store(true, Ordering::Relaxed);
}

/// Ends the capture or the stream.
pub unsafe fn stop() {
    let Some(s) = state() else { return };
    if s.phase == Phase::Off {
        return;
    }
    RECORDING.store(false, Ordering::Relaxed);
    ACTIVE.store(false, Ordering::Relaxed);
    s.phase = Phase::Off;
    match s.out.take() {
        Some(Sink::File(mut out)) => s.failed |= out.flush().is_err(),
        Some(Sink::Ring(mut ring)) => {
            crate::log!("stream: the game waited {:.1} ms in all for room in the ring", ring.waited as f64 * 1000.0 / ticks_per_second() as f64);
            ring.close();
            if let Some(mut reply) = s.reply.take() {
                crate::log!("stream: {} pixel reads, the game waited {:.1} ms in all for them", reply.requests, reply.waited as f64 * 1000.0 / ticks_per_second() as f64);
                reply.close();
            }
        }
        None => {}
    }
    #[cfg(not(test))]
    {
        if crate::own_host::active() {
            crate::log!("{}", crate::own_host::report());
        }
    }
    crate::log!(
        "capture: end, {} frames, {} records, {} bytes, {:.3} ms of the game thread for each frame{}",
        s.frames_done,
        s.records,
        s.bytes,
        s.spent as f64 * 1000.0 / ticks_per_second() as f64 / s.frames_done.max(1) as f64,
        if s.failed { ", with a write error" } else { "" }
    );
}

/// Asks the host of the stream for a picture file of the frame in progress.
pub unsafe fn shot(path: &str) {
    write(tag::SHOT, &[path.as_bytes()]);
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
            format!(
                "{phase} {} {} frames {} ms_per_frame {:.3}{}",
                s.records,
                s.bytes,
                s.frames_done,
                s.spent as f64 * 1000.0 / ticks_per_second() as f64 / s.frames_done.max(1) as f64,
                if s.failed { " FAILED" } else { "" }
            )
        }
    }
}

unsafe fn write(tag: u32, parts: &[&[u8]]) {
    if !recording() {
        return;
    }
    let Some(s) = state() else { return };
    let start = ticks();
    let length: usize = parts.iter().map(|p| p.len()).sum();
    match s.out.as_mut() {
        Some(Sink::File(out)) => {
            let mut ok = out.write_all(&tag.to_le_bytes()).is_ok() && out.write_all(&(length as u32).to_le_bytes()).is_ok();
            for part in parts {
                ok = ok && out.write_all(part).is_ok();
            }
            s.failed |= !ok;
        }
        Some(Sink::Ring(ring)) => {
            if !ring.push(tag, parts) {
                crate::log!("stream: the host did not read for {RING_WAIT_SECONDS} s; the stream ends");
                s.failed = true;
                stop();
                return;
            }
        }
        None => return,
    }
    s.records += 1;
    s.bytes += 8 + length as u64;
    s.spent += ticks() - start;
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
    if let Some(s) = state() {
        s.object = (draw.geometry_data, draw.skin_instance);
        if !recording() {
            s.geometry.insert(s.object);
            return;
        }
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
    if recording() {
        if let Some(s) = state() {
            let known = s.known_buffers.contains(&packet.vertex_buffer) && s.known_buffers.contains(&packet.index_buffer);
            if !known && !s.dropped_late.contains(&s.object) {
                s.late.insert(s.object);
            }
        }
    }
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
    if index == CALL_CREATE_IMAGE_SURFACE && handle != 0 {
        if let Some(s) = state() {
            s.image_surfaces.insert(handle);
        }
    }
    if recording() {
        let mut head = vec![index, result, handle, args.len() as u32];
        head.extend_from_slice(args);
        // A copy from an image surface: the pixels are those that the game wrote, for
        // example the character image after the game changed its alpha. The record has
        // width, height, format, size, the texture of the destination when the own host
        // knows it (or 0), and the pixels.
        let from_image = index == CALL_COPY_PIXELS && (result as i32) >= 0 && !args.is_empty() && state().is_some_and(|s| s.image_surfaces.contains(&args[0]));
        let pixels = if from_image { crate::link_client::surface_pixels(args[0]) } else { None };
        if let Some((width, height, format, pixels)) = pixels {
            let texture = if args.len() > 3 { destination_texture(args[3]) } else { 0 };
            write(tag::CALL, &[&words(&head), &words(&[width, height, format, pixels.len() as u32, texture]), &pixels]);
        } else {
            let data: &[u8] = if index == CALL_VIEWPORT && !args.is_empty() { bytes_at(args[0], 24) } else { &[] };
            write(tag::CALL, &[&words(&head), data]);
        }
    }
    if index == CALL_PRESENT {
        frame_end();
    }
}

#[cfg(not(test))]
unsafe fn destination_texture(surface: u32) -> u32 {
    if crate::own_host::active() {
        crate::own_host::texture_of(surface)
    } else {
        0
    }
}

#[cfg(test)]
unsafe fn destination_texture(_surface: u32) -> u32 {
    0
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

/// The bytes that the game wrote into a level of a texture.
pub unsafe fn texture_update(handle: usize, level: u32, width: u32, height: u32, format: u32, bytes: &[u8]) {
    write(tag::TEXTURE_UPDATE, &[&words(&[handle as u32, level, width, height, format, bytes.len() as u32]), bytes]);
}

/// The bytes that the game wrote into a vertex or index buffer, at its unlock.
pub unsafe fn buffer_data(handle: usize, offset: u32, bytes: &[u8]) {
    if let Some(s) = state() {
        s.known_buffers.insert(handle as u32);
    }
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
    let textures = crate::ports::textures::purge_source_textures();
    let rendered = crate::ports::rendered::remake_rendered_textures(renderer);
    // All the buffers, not only those of the objects in view: the game can draw an object
    // into a rendered texture one time and not again.
    let (shapes, partitions) = crate::ports::geometry::purge_all_geometry(renderer);
    let objects: Vec<(u32, u32)> = s.geometry.iter().copied().collect();
    let skins = drop_buffers(renderer, &objects);
    crate::log!(
        "capture: the renderer dropped {textures} source textures, the buffers of {shapes} shapes and {partitions} skin partitions (and of {} objects in view, {skins} with a skin), and made {rendered} rendered textures again",
        objects.len()
    );
}

/// Makes the renderer drop the buffers of these objects: geometry data and skin instance.
/// Returns the number of skins.
unsafe fn drop_buffers(renderer: usize, objects: &[(u32, u32)]) -> u32 {
    let mut skins = 0;
    for &(geometry_data, skin_instance) in objects {
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
    skins
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
                #[cfg(not(test))]
                {
                    if s.own_host {
                        crate::own_host::take_over();
                    }
                }
                // The record starts before the drop: the calls that make the rendered textures
                // again are a part of the stream.
                s.phase = Phase::Record(s.frames);
                RECORDING.store(true, Ordering::Relaxed);
                let renderer = crate::engine::renderer();
                let size: [u32; 2] = if renderer != 0 { rd(renderer, BACK_BUFFER_SIZE) } else { [0, 0] };
                let window: u32 = if renderer != 0 { rd(renderer, DEVICE_WINDOW) } else { 0 };
                write(tag::HEADER, &[&words(&[size[0], size[1], window])]);
                // The render states that are not in a state packet, as they are now.
                for (state, value) in crate::link_client::render_states() {
                    write(tag::COMMAND, &[&words(&[crate::link::COMMAND_RENDER_STATE, state, value, 0])]);
                }
                if let Some(s) = state() {
                    purge(s);
                }
            }
        }
        Phase::Record(left) => {
            write(tag::FRAME_END, &[]);
            let Some(s) = state() else { return };
            if s.phase == Phase::Off {
                // The write ended the stream.
                return;
            }
            s.frames_done += 1;
            if !s.late.is_empty() {
                let objects: Vec<(u32, u32)> = s.late.drain().collect();
                drop_buffers(crate::engine::renderer(), &objects);
                s.dropped_late.extend(objects);
            }
            if s.frames == 0 {
                return;
            }
            if left > 1 {
                s.phase = Phase::Record(left - 1);
                return;
            }
            stop();
        }
    }
}

/// `NiDX8Renderer::d3dPresentParameters`: the width and the height of the back buffer.
const BACK_BUFFER_SIZE: usize = 0x23C;
/// `NiDX8Renderer::d3dPresentParameters.hDeviceWindow`
const DEVICE_WINDOW: usize = 0x254;
