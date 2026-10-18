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
    DrawV1, SceneV1, TextureV1, CALL_COPY_PIXELS, CALL_CREATE_DEPTH_BUFFER, CALL_CREATE_IMAGE_SURFACE, CALL_CREATE_INDEX_BUFFER,
    CALL_CREATE_TEXTURE, CALL_CREATE_VERTEX_BUFFER, CALL_CUBE_FACE, CALL_GET_DEPTH_BUFFER, CALL_GET_RENDER_TARGET, CALL_PRESENT,
    CALL_TEXTURE_LEVEL, CALL_VIEWPORT, COMMAND_TRANSFORM,
};
use crate::mem::rd;

pub const MAGIC: &[u8; 8] = b"NIRCAP01";

/// The tags of the records.
pub mod tag {
    /// handle, flags, width, height, levels, D3D format, pixel format, pixel format tag, bytes
    /// for each pixel; then levels + 1 offsets, the widths and the heights of the levels; then
    /// the pixels.
    pub const TEXTURE: u32 = 1;
    /// handle, offset, size; then the bytes that the game wrote into a buffer of the device.
    /// Not written since the helper draws from the records (step R7b); a capture from
    /// before has it.
    pub const BUFFER_DATA: u32 = 2;
    /// A `MgeLinkSceneV1`.
    pub const SCENE_BEGIN: u32 = 3;
    pub const SCENE_END: u32 = 4;
    /// kind, a, b
    pub const FRAME_EVENT: u32 = 5;
    /// The object of the draws that follow: kind, class, flags, geometry, then its name.
    pub const OBJECT: u32 = 6;
    /// 7 was a `MgeLinkDrawStateV1` with its lights, the state of a draw of the device. Not
    /// written since step R7b; a capture from before has it.
    ///
    /// command, a, b, c; then the data at an address: 16 floats for a transform. Only a
    /// transform is written (see `command`).
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
    /// A draw with its vertices in the call, `DrawPrimitiveUP`: the texture of stage 0, the
    /// vertex format, the bytes of a vertex, the primitive type, the number of primitives;
    /// then the vertices. The films of the game are such draws, in screen coordinates.
    pub const DRAW_UP: u32 = 15;
    /// Data of another program for the part of it that draws: a number that the two parts
    /// agree on, then the bytes. MGE XE sends the state of the game that its features need
    /// (fog distances, weather) to its helper in this way. The record is in the stream at
    /// the place of the call, so the data is for the frame in progress.
    pub const EXTENSION: u32 = 16;
    /// A text from the scenario. The host program prints its counts since the last mark
    /// with the text, so its numbers are those of one step of the scenario.
    pub const MARK: u32 = 17;
    /// The geometry of a shape, packed by the plugin and keyed by the geometry data: handle,
    /// unique id, revision, vertex format, stride, vertices, index entries, strip count;
    /// then the strip lengths (u16, padded to 4 bytes), the vertices, the indexes. See
    /// `geometry_stream`.
    pub const GEOMETRY: u32 = 18;
    /// The material of a shape as facts: the handle (a hash of the words), then the words.
    /// See `material_stream`.
    pub const MATERIAL: u32 = 19;
    /// The draw of a shape: geometry handle, material handle, flags, light count, the scene
    /// ambient, the world matrix (16 floats), the 14 colour values of the material, the
    /// bone count; then the keys of the lights (4 bytes each), then the matrix of each
    /// bone. With the flag `EFFECTS`, before the bones come the stages of the texture
    /// effects of the draw: for the environment map and then for the fog map, the
    /// coordinate index with its generation, the transform flags and the texture matrix
    /// (18 words each; zeros for an effect the draw does not have). With the flag `FOG`,
    /// after the effects come the fog of the draw: the colour, the start, the end and the
    /// range fog flag (4 words). With the flag `BOUND` (0x400, since 2026-10-17) the last 16
    /// bytes of the record are the world bound of the shape (the centre and the radius),
    /// after everything else. A record with the flag `PARTICLES` is the draw of a
    /// particle system: the geometry handle is the key
    /// of the particle data, and after the bones come the count, the radius, the camera
    /// right and up axes in the model space (3 floats each), then for each particle its
    /// position (3 floats), its size and its colour (20 bytes). The helper makes the
    /// quads. A record with the flag `LINES` is the draw of lines: the helper draws its
    /// geometry as a line list. See `shape_stream`.
    pub const SHAPE: u32 = 20;
    /// A light of the light manager: its key (the index of the light), then its
    /// `D3DLIGHT8` (26 words). Sent when the light is new or changed. See `light_stream`.
    pub const LIGHT: u32 = 21;
    /// The debug overlay of the host program: 1 or 0 for on or off, a word of zero, then
    /// the numbers of the game process as 64-bit values (the working set, the private
    /// bytes, the address space in use, the address space in all, its largest free block),
    /// then the time of the game thread for each frame in ms as floats (in the stream, in
    /// the ports of the renderer, in waits for the host program). Sent when the state
    /// changes, and one time in a second while the overlay is on. See `overlay`.
    pub const OVERLAY: u32 = 22;
    /// A retained instance (`instance_stream`): id, class, flags, geometry handle, material
    /// handle, the rows 0 to 2 of its world matrix (12 floats), the exact translation (3
    /// doubles), the stored translation (3 floats), the 14 colours of its material; then its
    /// name. The same id again replaces the instance.
    pub const INSTANCE: u32 = 23;
    /// A draw of a retained instance, in the place of a shape record: id, ambient, flags,
    /// the fog (4 words) when the flags have `FOG`, the number of lights, then their keys;
    /// with the flag `SKINNED` then the number of bones and the palette (16 floats each).
    pub const INSTANCE_DRAW: u32 = 24;
    /// The instance is deleted: id.
    pub const INSTANCE_DELETE: u32 = 25;
    /// The lights of an instance the helper draws from its table (mode gpu): id, the number
    /// of lights, their keys. Sent when the lights of the instance change.
    pub const INSTANCE_LIGHTS: u32 = 26;
    /// The visibility of such an instance: id, 1 when the engine draws it, 0 when it does
    /// not although it is in the view of the camera.
    pub const INSTANCE_STATE: u32 = 27;
    /// The values of the frame for the draws of the instances the helper draws from its
    /// table: the ambient light (`D3DRS_AMBIENT`), flags (32: the fog follows), the fog of
    /// a draw (4 words, see `FOG` of the shape record). One in a frame, at the first such
    /// shape the engine reaches.
    pub const RETAINED_SCENE: u32 = 28;
    /// The bones of a skinned instance the helper draws from its table (mode gpu), in each
    /// frame that the table walk works it in the view: id, the number of bones, the world
    /// bound of the shape (the centre and the radius), then for each bone the matrix of its
    /// rotation and scale (9 floats, by columns) and its exact translation in the world (3
    /// doubles), as `camera_space::skin_bones` makes them.
    pub const INSTANCE_BONES: u32 = 29;
}

// ---- the ring in shared memory

extern "system" {
    fn CreateFileMappingA(file: isize, security: *const u8, protect: u32, size_high: u32, size_low: u32, name: *const u8) -> isize;
    fn MapViewOfFile(mapping: isize, access: u32, offset_high: u32, offset_low: u32, bytes: usize) -> *mut u8;
    fn UnmapViewOfFile(address: *const u8) -> i32;
    fn CloseHandle(handle: isize) -> i32;
    fn CreateEventA(security: *const u8, manual: i32, initial: i32, name: *const u8) -> isize;
    fn SetEvent(event: isize) -> i32;
    fn QueryPerformanceCounter(count: *mut i64) -> i32;
    fn QueryPerformanceFrequency(frequency: *mut i64) -> i32;
    fn Sleep(milliseconds: u32);
}

#[link(name = "user32")]
extern "system" {
    fn PeekMessageA(message: *mut [u32; 7], window: usize, first: u32, last: u32, flags: u32) -> i32;
}

/// One step of a wait for the host program. The thread answers the window messages that
/// other threads sent to its windows, and then gives its time away.
///
/// The host program makes its window inside the window of the game, and Windows then sends
/// messages to the thread of the game and waits for the answers. A game thread that only
/// sleeps while it waits for the host program stops the host program: each one then waits
/// for the other. `PeekMessage` answers the sent messages; with these flags it takes no
/// message out of the queue of the game.
unsafe fn wait_step() {
    // PM_NOREMOVE with PM_QS_SENDMESSAGE: QS_SENDMESSAGE (0x40) in the high word.
    let mut message = [0u32; 7];
    PeekMessageA(&mut message, 0, 0, 0, 0x0040_0000);
    Sleep(0);
}

/// The game loads its data: from its start to the "initialized" event of MWSE.
static LOADING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
/// The time of the last frame while the game loads, and the longest time between two.
static LOAD_LAST: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
static LOAD_LONGEST: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// A frame while the game loads. The game does not read the messages of its window while
/// it loads, and after 5 seconds Windows puts a copy of the window in its place ("Not
/// Responding"), which hides the picture of the host program. A look at the queue tells
/// Windows that the thread is alive. It takes no message out of the queue.
unsafe fn loading_tick() {
    if !LOADING.load(Ordering::Relaxed) {
        return;
    }
    // PM_NOREMOVE.
    let mut message = [0u32; 7];
    PeekMessageA(&mut message, 0, 0, 0, 0);
    let now = ticks();
    let last = LOAD_LAST.swap(now, Ordering::Relaxed);
    if last != 0 {
        LOAD_LONGEST.fetch_max(now - last, Ordering::Relaxed);
    }
}

/// The game has its data. From now on Windows finds a game that hangs, as without this
/// plugin. Windows shows its copy of the window when the time between two looks is more
/// than 5 seconds: the log has the longest time.
pub fn loading_done() {
    if LOADING.swap(false, Ordering::Relaxed) {
        let longest = LOAD_LONGEST.load(Ordering::Relaxed) as f64 / ticks_per_second() as f64;
        crate::log!("load: the longest time between two frames was {longest:.1} s (Windows shows \"Not Responding\" after 5 s)");
    }
}

pub(crate) fn ticks() -> i64 {
    let mut count = 0;
    unsafe { QueryPerformanceCounter(&mut count) };
    count
}

pub(crate) fn ticks_per_second() -> i64 {
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
/// The classes of the game that the helper draws itself (one bit for each `CLASS_*`),
/// written by the helper at each frame, read by the plugin (`helper_takes`).
pub const RING_TAKES: usize = 32;
/// The helper waits for a record (1 while it does): the game sets the event of the ring
/// after a write only then (plan M5).
const RING_WAITING: usize = 36;
/// The reach of the helper's shadow map in units of the world (the bits of an f32; 0
/// without shadows), for the animation policy of the plugin (`animation`).
pub const RING_SHADOW_REACH: usize = 40;
/// The game waits this long for room in the ring. Then it ends the stream: the host is gone.
const RING_WAIT_SECONDS: i64 = 10;

struct Ring {
    mapping: isize,
    base: *mut u8,
    capacity: u64,
    /// The event the helper waits on for a record: `<name>-records`, set after a write
    /// while the helper says it waits (`RING_WAITING`); the times it was set.
    event: isize,
    wakes: u64,
    /// The time that the game waited for room, in ticks.
    waited: i64,
    /// The bytes that the game wrote up to the end of the frame before the last one, and the
    /// time that the game waited for the host program to draw that frame, in ticks.
    frame_before: u64,
    paced: i64,
    /// The bytes that the host program had read at the end of the last frame.
    read_before: u64,
    /// The host program did not read at all for the time limit of a wait. The game does not
    /// wait again before the host program reads.
    host_late: bool,
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
        // An event that resets itself: one wait takes one set.
        let mut event_name = format!("{name}-records").into_bytes();
        event_name.push(0);
        let event = CreateEventA(core::ptr::null(), 0, 0, event_name.as_ptr());
        Ok(Ring { mapping, base, capacity, event, wakes: 0, waited: 0, frame_before: 0, read_before: 0, paced: 0, host_late: false })
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
                wait_step();
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
        self.counter(RING_WRITTEN).store(at, Ordering::SeqCst);
        // The helper waits for a record: wake it. The store above and this load are
        // SeqCst, as the helper's store of the word and its load of the count are, so
        // that one of the two sides sees the other's store.
        if self.event != 0 && (*(self.base.add(RING_WAITING) as *const core::sync::atomic::AtomicU32)).load(Ordering::SeqCst) != 0 {
            SetEvent(self.event);
            self.wakes += 1;
        }
        true
    }

    /// The end of a frame. The game waits here until the host program has read the frame
    /// before this one: without the wait the game runs ahead when it is faster than the host
    /// program, the ring fills with frames, and the picture is late by all of them. The mouse
    /// pointer of a menu then follows the hand late.
    ///
    /// The wait ends only when the host program does not read at all for a short time: it
    /// is not there yet, it is gone, or it is in one long step. The game then runs without
    /// a wait until the host program reads again. A host program that reads, also a slow
    /// one, keeps the game at its rate: after a long step the game waits until the host
    /// program has read the frames that the game made in that time.
    unsafe fn pace(&mut self) {
        let start = ticks();
        let limit = ticks_per_second() / 20;
        let mut read = self.counter(RING_READ).load(Ordering::Acquire);
        if !(self.host_late && read == self.read_before) {
            self.host_late = false;
            let mut moved = start;
            while read < self.frame_before {
                wait_step();
                let now = self.counter(RING_READ).load(Ordering::Acquire);
                if now != read {
                    read = now;
                    moved = ticks();
                } else if ticks() - moved > limit {
                    self.host_late = true;
                    break;
                }
            }
            self.paced += ticks() - start;
        }
        self.read_before = read;
        self.frame_before = self.counter(RING_WRITTEN).load(Ordering::Relaxed);
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
    let _lock = crate::sync::device_lock();
    if !recording() {
        return None;
    }
    let s = state()?;
    let request = {
        let reply = s.reply.as_mut()?;
        reply.requests += 1;
        reply.requests
    };
    write(tag::READ_PIXELS, &[words(&[request, texture])]);
    // The write can end the stream.
    let reply = state()?.reply.as_mut()?;
    let start = ticks();
    while (reply.base as *const u32).read_volatile() != request {
        if ticks() - start > REPLY_WAIT_SECONDS * ticks_per_second() {
            crate::log!("stream: no answer to the pixel read {request} of the texture {texture:#x}");
            return None;
        }
        wait_step();
    }
    reply.waited += ticks() - start;
    let (width, height) = ((reply.base.add(4) as *const u32).read_volatile(), (reply.base.add(8) as *const u32).read_volatile());
    let size = width as usize * height as usize * 4;
    if size == 0 || size > reply.capacity {
        return None;
    }
    Some((width, height, core::slice::from_raw_parts(reply.base.add(REPLY_HEAD), size)))
}

/// The message memory of a stream: shared memory with the name of the ring and `.messages`.
/// The host writes messages of its extension into it (`Extension::messages` of the crate):
/// records of the number, the length and the bytes, which can go around the end of the data.
/// The head: `MESSAGES_MAGIC`, the number of data bytes, the bytes that the host wrote (u64),
/// the bytes that the game read (u64). The plugin reads them at the end of each frame and
/// gives each one to MGE XE (`link_client::stream_message`).
pub const MESSAGES_MAGIC: u32 = 0x4753_4D31;
const MESSAGES_HEAD: usize = 32;
const MESSAGES_BYTES: usize = 64 * 1024;

struct Messages {
    mapping: isize,
    base: *mut u8,
}

impl Messages {
    unsafe fn create(name: &str) -> Option<Messages> {
        let mut text = format!("{name}.messages").into_bytes();
        text.push(0);
        let mapping = CreateFileMappingA(-1, core::ptr::null(), 4, 0, (MESSAGES_BYTES + MESSAGES_HEAD) as u32, text.as_ptr());
        if mapping == 0 {
            return None;
        }
        let base = MapViewOfFile(mapping, 0xF001F, 0, 0, 0);
        if base.is_null() {
            CloseHandle(mapping);
            return None;
        }
        core::ptr::write_bytes(base, 0, MESSAGES_HEAD);
        (base.add(4) as *mut u32).write(MESSAGES_BYTES as u32);
        (base as *mut u32).write_volatile(MESSAGES_MAGIC);
        Some(Messages { mapping, base })
    }

    unsafe fn counter(&self, offset: usize) -> &core::sync::atomic::AtomicU64 {
        &*(self.base.add(offset) as *const core::sync::atomic::AtomicU64)
    }

    /// The messages that the host wrote since the last call.
    unsafe fn take(&mut self) -> Vec<(u32, Vec<u8>)> {
        let written = self.counter(8).load(Ordering::Acquire);
        let mut read = self.counter(16).load(Ordering::Relaxed);
        let mut out = Vec::new();
        let byte = |at: u64| self.base.add(MESSAGES_HEAD + (at % MESSAGES_BYTES as u64) as usize).read();
        let word = |at: u64| u32::from_le_bytes([byte(at), byte(at + 1), byte(at + 2), byte(at + 3)]);
        while written - read >= 8 {
            let (id, length) = (word(read), word(read + 4) as u64);
            if written - read < 8 + length {
                break;
            }
            out.push((id, (0..length).map(|n| byte(read + 8 + n)).collect()));
            read += 8 + length;
        }
        self.counter(16).store(read, Ordering::Release);
        out
    }

    unsafe fn close(&mut self) {
        UnmapViewOfFile(self.base);
        CloseHandle(self.mapping);
    }
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
    /// The stream started with the device of the plugin: each buffer got the recorder when
    /// the game made it, and each buffer of a draw is in the stream.
    direct: bool,
    out: Option<Sink>,
    reply: Option<Reply>,
    messages: Option<Messages>,
    /// A file that gets a copy of the records of a direct stream, with the number of frames
    /// that are left for it. While `tee_only`, the records go into the file only: the
    /// snapshot of the resources at its start.
    tee: Option<(BufWriter<File>, u32)>,
    /// The file from the stream stays open when its frames are done: it takes the resource
    /// records only, until `tee_frames` asks for frames again. See `tee`.
    tee_hold: bool,
    /// The buffers and textures whose content changed while the file holds: their content
    /// goes into the file once, when frames are asked for.
    tee_dirty: HashSet<u32>,
    /// The geometry and material records that came while the file held: the plugin sends
    /// them again at the next frames.
    tee_dirty_geometry: HashSet<u32>,
    tee_dirty_material: HashSet<u32>,
    tee_only: bool,
    /// The words of the header record of a direct stream, for the file.
    header: [u32; 3],
    /// The geometry data and the skin instances of the draws of the warm-up.
    geometry: HashSet<(u32, u32)>,
    /// The image surfaces: surfaces in system memory, where the game changes pixels.
    image_surfaces: HashSet<u32>,
    /// The object of the draws that follow.
    object: (u32, u32),
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
    let _lock = crate::sync::device_lock();
    if active() && state().is_some_and(|s| s.direct && s.tee.is_none()) {
        return tee(path, frames);
    }
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

/// Starts a capture file from a direct stream: the file gets the header, the render states
/// and all resources of the own host as they are now, and then a copy of each record of
/// the stream for `frames` frames. The file plays alone, as a capture of the other kind.
///
/// With 0 frames the file holds: it takes only the records that make or change resources,
/// until `tee_frames` asks for frames, and it ends with `tee_end`. A tour of a test level
/// records two frames at each stop into one file, and the textures and the geometry that
/// the game makes between the stops are in the file once.
unsafe fn tee(path: &str, frames: u32) -> Result<(), String> {
    let file = File::create(path).map_err(|error| format!("{path}: {error}"))?;
    let mut out = BufWriter::with_capacity(1 << 20, file);
    out.write_all(MAGIC).map_err(|error| error.to_string())?;
    let Some(s) = state() else { return Err("no stream".into()) };
    s.tee = Some((out, frames.max(1)));
    s.tee_hold = frames == 0;
    s.tee_only = true;
    let header = s.header;
    write(tag::HEADER, &[words(&header)]);
    write_kept();
    #[cfg(not(test))]
    crate::own_host::snapshot();
    crate::geometry_stream::reset();
    crate::material_stream::reset();
    crate::light_stream::reset();
    crate::instance_stream::reset();
    if let Some(s) = state() {
        s.tee_only = false;
        if s.tee_hold {
            if let Some((_, left)) = s.tee.as_mut() {
                *left = 0;
            }
        }
        // The source textures, the rendered textures and the geometry of the game go into
        // the file as the game makes them again.
        purge(s);
    }
    crate::log!("capture: a file from the stream, {frames} frames, into {path}");
    Ok(())
}

/// Asks a file from the stream that holds for `frames` frames more. The render states go
/// into the file first, as they are now: a frame that was not recorded can have changed
/// them. Returns false without such a file.
pub unsafe fn tee_frames(frames: u32) -> bool {
    let _lock = crate::sync::device_lock();
    let Some(s) = state() else { return false };
    if !s.tee_hold || s.tee.is_none() {
        return false;
    }
    if let Some((_, left)) = s.tee.as_mut() {
        *left = frames.max(1);
    }
    // The content of the buffers and textures that changed while the file held, as it is
    // now, once each.
    let dirty: Vec<u32> = state().map(|s| s.tee_dirty.drain().collect()).unwrap_or_default();
    #[cfg(not(test))]
    for handle in dirty {
        crate::own_host::write_resource(handle as usize);
    }
    #[cfg(test)]
    drop(dirty);
    // The geometry and material records of the hold: forgotten, so that the next draw of
    // each sends it again, into the frames.
    let geometry: Vec<u32> = state().map(|s| s.tee_dirty_geometry.drain().collect()).unwrap_or_default();
    crate::geometry_stream::forget(&geometry);
    let material: Vec<u32> = state().map(|s| s.tee_dirty_material.drain().collect()).unwrap_or_default();
    crate::material_stream::forget(&material);
    // The lights change in most frames: all of them again, into the frames.
    crate::light_stream::reset();
    crate::instance_stream::reset();
    true
}

/// Ends a file from the stream that holds.
pub unsafe fn tee_end() {
    let _lock = crate::sync::device_lock();
    if let Some(s) = state() {
        s.tee_hold = false;
        s.tee_dirty.clear();
        s.tee_dirty_geometry.clear();
        s.tee_dirty_material.clear();
        end_tee(s);
    }
}

/// A record that makes a resource, or changes one in a way that has no content: a file that
/// holds takes these between its frames, so that the resources of a recorded frame are in
/// the file. The content of buffers and textures comes once, at the frames. The visibility
/// and the deletion of a retained instance count too: the file has the instance from an
/// earlier frame, and a release sends again only the instances the engine displays (a
/// replay drew the far level of a level-of-detail node at the near stops of the NIF tour,
/// 2026-10-16).
/// Engine-rules R14 (the skill wgpu-renderer-dev).
fn resource_record(tag: u32) -> bool {
    matches!(tag, tag::TEXTURE | tag::CALL | tag::INSTANCE_STATE | tag::INSTANCE_DELETE)
}

/// Ends the capture file of a direct stream.
unsafe fn end_tee(s: &mut State) {
    if let Some((mut out, _)) = s.tee.take() {
        let ok = out.flush().is_ok();
        crate::log!("capture: the file from the stream is complete, ok {ok}");
    }
}

/// Starts a stream into a ring of `megabytes` in shared memory with the name `name`. It
/// goes on until `stop`, or until the host does not read for some seconds.
///
/// With `own_host`, the plugin becomes the host of the render link at the end of the warm-up:
/// from then on no call of the game gets to a D3D device, and the stream is the only picture.
pub unsafe fn start_stream(name: &str, megabytes: u32, warmup: u32, own_host: bool) -> Result<(), String> {
    let _lock = crate::sync::device_lock();
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

/// Starts a stream that has all of the life of the device: the device is the one of the
/// plugin, and the stream starts when the game makes it. No warm-up: each resource that the
/// game makes after this is in the stream.
pub unsafe fn start_direct(name: &str, megabytes: u32, width: u32, height: u32, window: u32) -> Result<(), String> {
    let _lock = crate::sync::device_lock();
    if active() {
        return Err("a capture is in progress".into());
    }
    begin(Sink::Ring(Ring::create(name, megabytes)?), 1, 0);
    if let Some(s) = state() {
        s.direct = true;
        s.phase = Phase::Record(0);
        s.reply = Reply::create(name, (width as usize * height as usize * 4).max(1024 * 1024 * 4));
        s.messages = Messages::create(name);
    }
    RECORDING.store(true, Ordering::Relaxed);
    if let Some(s) = state() {
        s.header = [width, height, window];
    }
    write(tag::HEADER, &[words(&[width, height, window])]);
    write_kept();
    crate::log!("stream: start with the device, a ring of {megabytes} MB with the name {name}");
    Ok(())
}

unsafe fn begin(out: Sink, warmup: u32, frames: u32) {
    *core::ptr::addr_of_mut!(STATE) = Some(State {
        phase: Phase::Warmup(warmup.max(1)),
        frames,
        own_host: false,
        direct: false,
        out: Some(out),
        reply: None,
        messages: None,
        tee: None,
        tee_hold: false,
        tee_dirty: HashSet::new(),
        tee_dirty_geometry: HashSet::new(),
        tee_dirty_material: HashSet::new(),
        tee_only: false,
        header: [0; 3],
        geometry: HashSet::new(),
        image_surfaces: HashSet::new(),
        object: (0, 0),
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
    let _lock = crate::sync::device_lock();
    let Some(s) = state() else { return };
    if s.phase == Phase::Off {
        return;
    }
    RECORDING.store(false, Ordering::Relaxed);
    ACTIVE.store(false, Ordering::Relaxed);
    s.phase = Phase::Off;
    end_tee(s);
    match s.out.take() {
        Some(Sink::File(mut out)) => s.failed |= out.flush().is_err(),
        Some(Sink::Ring(mut ring)) => {
            crate::log!(
                "stream: the game waited {:.1} ms in all for room in the ring, and {:.1} ms in all for the host program at the ends of the frames; it woke the host program {} times",
                ring.waited as f64 * 1000.0 / ticks_per_second() as f64,
                ring.paced as f64 * 1000.0 / ticks_per_second() as f64,
                ring.wakes
            );
            ring.close();
            if let Some(mut reply) = s.reply.take() {
                crate::log!("stream: {} pixel reads, the game waited {:.1} ms in all for them", reply.requests, reply.waited as f64 * 1000.0 / ticks_per_second() as f64);
                reply.close();
            }
            if let Some(mut messages) = s.messages.take() {
                messages.close();
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
    crate::log!("threads: {}", crate::sync::census_text().replace('\n', "; "));
    crate::log!("{}", crate::geometry_stream::report());
    crate::log!("{}", crate::material_stream::report());
    crate::log!("{}", crate::light_stream::report());
    crate::log!("{}", crate::shape_stream::report());
    crate::log!("{}", crate::instance_stream::report());
    crate::log!(
        "capture: end, {} frames, {} records, {} bytes, {:.3} ms of the game thread for each frame{}",
        s.frames_done,
        s.records,
        s.bytes,
        s.spent as f64 * 1000.0 / ticks_per_second() as f64 / s.frames_done.max(1) as f64,
        if s.failed { ", with a write error" } else { "" }
    );
}

/// A draw with its vertices in the call.
pub unsafe fn draw_up(texture: u32, vertex_format: u32, stride: u32, primitive_type: u32, primitives: u32, vertices: &[u8]) {
    write(tag::DRAW_UP, &[words(&[texture, vertex_format, stride, primitive_type, primitives]), vertices]);
}

/// The bit of the number of an extension record that marks a state, not an event: the
/// stream keeps the last record of each such number and writes it again at the start of a
/// stream and of a capture file, so they have the state that was set before them. The record
/// has the number without the bit.
pub const EXTENSION_KEPT: u32 = 0x8000_0000;

/// The last record of each number with `EXTENSION_KEPT`, without the bit.
static mut KEPT: Vec<(u32, Vec<u8>)> = Vec::new();

/// Data of another program for its part in the renderer. See `tag::EXTENSION`.
pub unsafe fn extension(id: u32, bytes: &[u8]) {
    let _lock = crate::sync::device_lock();
    let id = if id & EXTENSION_KEPT != 0 {
        let id = id & !EXTENSION_KEPT;
        let kept = &mut *core::ptr::addr_of_mut!(KEPT);
        kept.retain(|(known, _)| *known != id);
        kept.push((id, bytes.to_vec()));
        id
    } else {
        id
    };
    write(tag::EXTENSION, &[words(&[id]), bytes]);
}

/// Writes the kept extension records again, after the header of a stream or of a file.
unsafe fn write_kept() {
    let kept = (*core::ptr::addr_of!(KEPT)).clone();
    for (id, bytes) in kept {
        write(tag::EXTENSION, &[words(&[id]), &bytes]);
    }
}

/// Asks the host of the stream for a picture file of the frame in progress.
pub unsafe fn shot(path: &str) {
    write(tag::SHOT, &[path.as_bytes()]);
}

/// The totals at the last `OVERLAY` record: the time of the record in ticks, the frames
/// since it, and the ticks in the stream, the ms in the ports and the ticks in waits.
static mut OVERLAY_LAST: (i64, u32, i64, f64, i64) = (0, 0, 0, 0.0, 0);

/// Turns the debug overlay of the host program on or off (to the other state without a
/// value) and tells the host program. Returns the state.
pub unsafe fn overlay(value: Option<bool>) -> bool {
    let _lock = crate::sync::device_lock();
    let now = crate::overlay::set(value);
    overlay_record();
    now
}

/// The classes of the game that the helper draws itself (one bit for each `CLASS_*`), as
/// the helper wrote them into the head of the ring at its last frame. 0 without a ring
/// (a file capture, no stream): the engine then draws every class.
pub unsafe fn helper_takes() -> u32 {
    let Some(s) = state() else { return 0 };
    match s.out.as_ref() {
        Some(Sink::Ring(ring)) => (*(ring.base.add(RING_TAKES) as *const core::sync::atomic::AtomicU32)).load(Ordering::Relaxed),
        _ => 0,
    }
}

/// The reach of the helper's shadow map, as the helper wrote it into the head of the ring
/// at its last frame; 0 without a ring or without shadows.
pub unsafe fn helper_shadow_reach() -> f32 {
    let Some(s) = state() else { return 0.0 };
    match s.out.as_ref() {
        Some(Sink::Ring(ring)) => f32::from_bits((*(ring.base.add(RING_SHADOW_REACH) as *const core::sync::atomic::AtomicU32)).load(Ordering::Relaxed)),
        _ => 0.0,
    }
}

/// Writes an `OVERLAY` record with the numbers since the last one.
unsafe fn overlay_record() {
    let Some(s) = state() else { return };
    let waits = match s.out.as_ref() {
        Some(Sink::Ring(ring)) => ring.waited + ring.paced,
        _ => 0,
    };
    let (spent, ports) = (s.spent, crate::shadow::port_ms());
    let last = &mut *core::ptr::addr_of_mut!(OVERLAY_LAST);
    let frames = last.1.max(1) as f64;
    let ms = |ticks: i64| (ticks as f64 * 1000.0 / ticks_per_second() as f64 / frames) as f32;
    let body = crate::overlay::body(ms(spent - last.2), ((ports - last.3) / frames) as f32, ms(waits - last.4));
    *last = (ticks(), 0, spent, ports, waits);
    write(tag::OVERLAY, &[&body]);
}

/// At the end of a frame: the record of the overlay, one time in a second while it is on.
unsafe fn overlay_tick() {
    let last = &mut *core::ptr::addr_of_mut!(OVERLAY_LAST);
    last.1 += 1;
    if crate::overlay::on() && ticks() - last.0 >= ticks_per_second() {
        overlay_record();
    }
}

/// Puts a mark with a text into the stream. See `tag::MARK`.
pub unsafe fn mark(text: &str) {
    write(tag::MARK, &[text.as_bytes()]);
}

/// One line about the capture, for the scenario: the phase, the records and the bytes,
/// then the time of the game thread since the start, in ms: in this module (`stream`), in
/// the ports of the renderer (`ports`, outermost calls, with the stream time in them),
/// in waits for room in the ring (`room`) and in waits for the host program at the ends
/// of the frames (`pace`). The difference of two lines gives the numbers of one step.
pub unsafe fn status() -> String {
    let _lock = crate::sync::device_lock();
    match state() {
        None => "off 0 0".into(),
        Some(s) => {
            let phase = match s.phase {
                Phase::Off => "done",
                Phase::Warmup(_) => "warmup",
                Phase::Record(_) => "record",
            };
            let ms = |ticks: i64| ticks as f64 * 1000.0 / ticks_per_second() as f64;
            let (room, pace) = match s.out.as_ref() {
                Some(Sink::Ring(ring)) => (ring.waited, ring.paced),
                _ => (0, 0),
            };
            format!(
                "{phase} {} {} frames {} ms_per_frame {:.3} ms stream {:.1} ports {:.1} room {:.1} pace {:.1}{}",
                s.records,
                s.bytes,
                s.frames_done,
                ms(s.spent) / s.frames_done.max(1) as f64,
                ms(s.spent),
                crate::shadow::port_ms(),
                ms(room),
                ms(pace),
                if s.failed { " FAILED" } else { "" }
            )
        }
    }
}

unsafe fn write(tag: u32, parts: &[&[u8]]) {
    let _lock = crate::sync::device_lock();
    if !recording() {
        return;
    }
    let Some(s) = state() else { return };
    let start = ticks();
    let length: usize = parts.iter().map(|p| p.len()).sum();
    let hold = s.tee_hold;
    // A record of a resource while the file holds: its handle is the first word. The record
    // of the end of a frame has no parts.
    if hold && s.tee.as_ref().is_some_and(|tee| tee.1 == 0) && parts.first().is_some_and(|p| p.len() >= 4) {
        let handle = u32::from_le_bytes([parts[0][0], parts[0][1], parts[0][2], parts[0][3]]);
        match tag {
            tag::BUFFER_DATA | tag::TEXTURE_UPDATE => {
                s.tee_dirty.insert(handle);
            }
            tag::GEOMETRY => {
                s.tee_dirty_geometry.insert(handle);
            }
            tag::MATERIAL => {
                s.tee_dirty_material.insert(handle);
            }
            _ => {}
        }
    }
    if let Some((out, _)) = s.tee.as_mut().filter(|tee| !hold || tee.1 > 0 || resource_record(tag)) {
        let mut ok = out.write_all(&tag.to_le_bytes()).is_ok() && out.write_all(&(length as u32).to_le_bytes()).is_ok();
        for part in parts {
            ok = ok && out.write_all(part).is_ok();
        }
        if !ok {
            crate::log!("capture: the file from the stream failed");
            s.tee = None;
        }
    }
    if s.tee_only {
        return;
    }
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

// The stream is little endian, as the only target of the plugin (i686) is: the memory of a
// value is its stream bytes.
const _: () = assert!(cfg!(target_endian = "little"), "the stream writes the memory of its values as little-endian bytes");

/// The bytes of `values` in the stream: the memory of the values, without a copy.
fn words(values: &[u32]) -> &[u8] {
    // SAFETY: a u32 slice is valid as bytes of 4 times its length, for its lifetime.
    unsafe { core::slice::from_raw_parts(values.as_ptr() as *const u8, core::mem::size_of_val(values)) }
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
    write(tag::FRAME_EVENT, &[words(&[kind, a, b])]);
}

/// The object packet of the draws that follow.
pub unsafe fn object(draw: &DrawV1, geometry_handle: u32, material_handle: u32) {
    let _lock = crate::sync::device_lock();
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
    if geometry_handle != 0 || material_handle != 0 {
        let flags = draw.flags | if geometry_handle != 0 { OBJECT_HAS_GEOMETRY } else { 0 } | if material_handle != 0 { OBJECT_HAS_MATERIAL } else { 0 };
        write(tag::OBJECT, &[words(&[draw.kind, draw.object_class, flags, draw.geometry, geometry_handle, material_handle]), &name[..name.len().min(96)]]);
    } else {
        write(tag::OBJECT, &[words(&[draw.kind, draw.object_class, draw.flags, draw.geometry]), &name[..name.len().min(96)]]);
    }
}

/// The flags in the object record that say two more words follow: the geometry handle and
/// the material handle (each 0 when its flag is off).
pub const OBJECT_HAS_GEOMETRY: u32 = 0x100;
pub const OBJECT_HAS_MATERIAL: u32 = 0x200;

/// A shape record. See `tag::SHAPE`. `tail` and `bytes` follow the lights: the bones of a
/// skinned shape, or the particle words and the particles.
pub unsafe fn shape(head: &[u32], lights: &[u32], tail: &[u32], bytes: &[u8], bound: &[u32]) {
    let _lock = crate::sync::device_lock();
    write(tag::SHAPE, &[words(head), words(lights), words(tail), bytes, words(bound)]);
}

/// An instance record. See `tag::INSTANCE`.
pub unsafe fn instance(head: &[u32], name: &[u8]) {
    let _lock = crate::sync::device_lock();
    write(tag::INSTANCE, &[words(head), name]);
}

/// A draw of an instance. See `tag::INSTANCE_DRAW`.
pub unsafe fn instance_draw(id: u32, ambient: u32, fog: Option<[u32; 4]>, lights: &[u32], bones: &[u8], bone_count: u32) {
    let _lock = crate::sync::device_lock();
    let skinned = bone_count != 0;
    let flags = if fog.is_some() { crate::shape_stream::FOG } else { 0 } | if skinned { crate::shape_stream::SKINNED } else { 0 };
    let count = [lights.len() as u32];
    let bone_word = [bone_count];
    let palette: &[u8] = if skinned { bones } else { &[] };
    write(tag::INSTANCE_DRAW, &[words(&[id, ambient, flags]), words(fog.as_ref().map_or(&[], |f| &f[..])), words(&count), words(lights), words(if skinned { &bone_word } else { &[] }), palette]);
}

/// The end of an instance. See `tag::INSTANCE_DELETE`.
pub unsafe fn instance_delete(id: u32) {
    let _lock = crate::sync::device_lock();
    write(tag::INSTANCE_DELETE, &[words(&[id])]);
}

/// The lights of an instance. See `tag::INSTANCE_LIGHTS`.
pub unsafe fn instance_lights(id: u32, lights: &[u32]) {
    let _lock = crate::sync::device_lock();
    write(tag::INSTANCE_LIGHTS, &[words(&[id, lights.len() as u32]), words(lights)]);
}

/// The bones of an instance: `bones` holds `count` bones of `camera_space::BONE_BYTES`. See
/// `tag::INSTANCE_BONES`.
pub unsafe fn instance_bones(id: u32, count: u32, bound: &[f32; 4], bones: &[u8]) {
    let _lock = crate::sync::device_lock();
    write(tag::INSTANCE_BONES, &[words(&[id, count]), words(&bound.map(f32::to_bits)), bones]);
}

/// The visibility of an instance. See `tag::INSTANCE_STATE`.
pub unsafe fn instance_state(id: u32, visible: bool) {
    let _lock = crate::sync::device_lock();
    write(tag::INSTANCE_STATE, &[words(&[id, visible as u32])]);
}

/// The values of the frame for the retained draws. See `tag::RETAINED_SCENE`.
pub unsafe fn retained_scene(ambient: u32, fog: Option<[u32; 4]>) {
    let _lock = crate::sync::device_lock();
    let flags = if fog.is_some() { crate::shape_stream::FOG } else { 0 };
    write(tag::RETAINED_SCENE, &[words(&[ambient, flags]), words(fog.as_ref().map_or(&[], |f| &f[..]))]);
}

/// A light record. See `tag::LIGHT`.
pub unsafe fn light(key: u32, values: &[u32; 26]) {
    let _lock = crate::sync::device_lock();
    write(tag::LIGHT, &[words(&[key]), words(values)]);
}

/// A material record. See `tag::MATERIAL`.
pub unsafe fn material(handle: u32, record: &[u32]) {
    let _lock = crate::sync::device_lock();
    write(tag::MATERIAL, &[words(&[handle]), words(record)]);
}

/// A geometry record. See `tag::GEOMETRY`.
pub unsafe fn geometry(head: &[u32], lengths: &[u8], vertices: &[u8], indexes: &[u8]) {
    let _lock = crate::sync::device_lock();
    write(tag::GEOMETRY, &[words(head), lengths, vertices, indexes]);
}

/// A state command of the device. Only a transform goes into the stream, with its matrix:
/// the helper takes the view and the projection from it. A render state, a stage state and
/// a draw of the device do not: the helper draws from the shape records, and each record
/// has the state of its draw.
pub unsafe fn command(command: u32, a: u32, b: u32, c: u32) {
    if !recording() || command != COMMAND_TRANSFORM {
        return;
    }
    write(tag::COMMAND, &[words(&[command, a, b, c]), bytes_at(b, 64)]);
}

/// A call of the link, after the host did it.
pub unsafe fn call(index: u32, args: &[u32], result: u32) {
    let _lock = crate::sync::device_lock();
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
            write(tag::CALL, &[words(&head), words(&[width, height, format, pixels.len() as u32, texture]), &pixels]);
        } else {
            let data: &[u8] = if index == CALL_VIEWPORT && !args.is_empty() { bytes_at(args[0], 24) } else { &[] };
            write(tag::CALL, &[words(&head), data]);
        }
    }
    if index == CALL_PRESENT {
        frame_end();
        #[cfg(not(test))]
        crate::link_client::stream_present();
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
    write(tag::TEXTURE, &[words(&head), words(offsets), words(widths), words(heights), pixels]);
}

/// The bytes that the game wrote into a level of a texture.
pub unsafe fn texture_update(handle: usize, level: u32, width: u32, height: u32, format: u32, bytes: &[u8]) {
    write(tag::TEXTURE_UPDATE, &[words(&[handle as u32, level, width, height, format, bytes.len() as u32]), bytes]);
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
    let (textures, kept) = crate::ports::textures::purge_source_textures();
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
    if kept != 0 {
        crate::log!("capture: {kept} source textures kept their data: the game cannot make them again (no file name, no pixel data); a capture file does not have them");
    }
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
    loading_tick();
    #[cfg(not(test))]
    crate::own_device::check_host();
    // The messages of the host for MGE XE.
    if let Some(messages) = state().and_then(|s| s.messages.as_mut()) {
        for (id, bytes) in messages.take() {
            #[cfg(not(test))]
            crate::link_client::stream_message(id, &bytes);
            #[cfg(test)]
            let _ = (id, bytes);
        }
    }
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
                write(tag::HEADER, &[words(&[size[0], size[1], window])]);
                write_kept();
                if let Some(s) = state() {
                    purge(s);
                }
            }
        }
        Phase::Record(left) => {
            overlay_tick();
            crate::instance_stream::frame_tick();
            // The times of the frame, for the age of the picture in the debug overlay: the
            // start of the frame of the game (the end of the wait of the frame before) and
            // its end, in ticks of the performance counter. The host program has the same
            // counter.
            static FRAME_START: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
            let end = ticks();
            let start = match FRAME_START.load(Ordering::Relaxed) {
                0 => end,
                start => start,
            };
            write(tag::FRAME_END, &[&start.to_le_bytes(), &end.to_le_bytes()]);
            FRAME_START.store(ticks(), Ordering::Relaxed);
            let Some(s) = state() else { return };
            if s.phase == Phase::Off {
                // The write ended the stream.
                return;
            }
            let hold = s.tee_hold;
            if let Some((_, left)) = s.tee.as_mut().filter(|tee| tee.1 > 0) {
                *left -= 1;
                if *left == 0 && !hold {
                    end_tee(s);
                }
            }
            if let Some(Sink::Ring(ring)) = s.out.as_mut() {
                ring.pace();
            }
            s.frames_done += 1;
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

#[cfg(test)]
mod tests {
    /// The words of a record are the same bytes as before the view without a copy: each value
    /// as its little-endian bytes.
    #[test]
    fn the_words_are_their_little_endian_bytes() {
        let values = [0u32, 1, 0x0102_0304, u32::MAX, 0xDEAD_BEEF];
        let expected: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(super::words(&values), &expected[..]);
        assert!(super::words(&[]).is_empty());
    }
}
