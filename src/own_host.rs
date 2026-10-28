//! The plugin as the host of its own render link.
//!
//! The plugin is the host of the render link: each call of the link ends here, and not on a
//! Direct3D device. The game makes no call that gets to a D3D device: the commands go only
//! into the stream of `capture`, and a renderer in another process draws them.
//!
//! The game holds pointers to its buffers, textures and surfaces and calls their methods.
//! Other code in the process does that too, for example the skinning code of MGE XE. So each
//! resource that this host makes is an object with a table of methods in the layout of
//! Direct3D 8, and with memory of its own for a lock. A call of the link on a resource goes
//! through the table of the object, so an object that Direct3D made before `take_over`
//! works too.
//!
//! The objects and their counts change only under the device lock ([`crate::sync`]).
//!
//! The pixels of a target are in the other process. For a copy from a rendered texture or
//! from the back buffer into a surface, and for a lock of the back buffer, this host asks
//! the host program of the stream for them. Without a stream such a read gives zeros.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::sync::{DeviceLock, LockCell};
use crate::link::*;

static ACTIVE: AtomicBool = AtomicBool::new(false);

#[inline]
pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

const OK: u32 = 0;
const FAIL: u32 = 0x8000_4005;
const NO_INTERFACE: u32 = 0x8000_4002;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    VertexBuffer,
    IndexBuffer,
    Texture,
    /// A level of a texture. `owner` is the texture.
    LevelSurface,
    /// A surface with memory of its own: an image surface, a depth buffer, the back buffer.
    Surface,
}

/// A resource of this host. The first word is the method table, as in a COM object.
#[repr(C)]
struct Object {
    table: *const usize,
    references: u32,
    kind: Kind,
    /// The handle of the object in the stream ([`serial_of`]).
    serial: u32,
    /// A buffer: its length. A texture or a surface: its width and height.
    length: u32,
    width: u32,
    height: u32,
    levels: u32,
    usage: u32,
    format: u32,
    pool: u32,
    /// A level surface: its texture and its level.
    owner: *mut Object,
    level: u32,
    /// The memory of a buffer or of a surface. A texture has one block for each level, made at
    /// the first lock of the level.
    memory: Vec<u8>,
    level_memory: Vec<Vec<u8>>,
}

/// The bytes of a level with this width and height, and the bytes of one of its rows.
fn level_size(format: u32, width: u32, height: u32) -> (usize, u32) {
    let (width, height) = (width.max(1), height.max(1));
    let block = match format {
        0x3154_5844 => 8,
        0x3254_5844..=0x3554_5844 | crate::d3d::D3DFMT_BC7 => 16,
        _ => 0,
    };
    if block != 0 {
        let across = width.div_ceil(4);
        return ((across * height.div_ceil(4) * block) as usize, across * block);
    }
    // D3DFMT_R8G8B8 20, the 16-bit formats 23 to 26 and 30, D3DFMT_A8 28 and the 8-bit ones.
    let bytes = match format {
        20 => 3,
        23..=26 | 30 | 40 | 51 | 80 => 2,
        28 | 41 | 50 => 1,
        _ => 4,
    };
    ((width * height * bytes) as usize, width * bytes)
}

/// The resources that are alive, for a snapshot. Under the device lock.
static LIVE: LockCell<Vec<usize>> = LockCell::new(Vec::new());

/// Writes each resource that is alive into the capture: its creation and its bytes. For a
/// capture file that starts in the middle of a stream. A texture whose pixels came from the
/// pixel data of the game (the port of the texture loader writes them, with no lock) has no
/// memory here and is not written: the caller makes the game load it again. A rendered
/// texture is not written: the caller makes the game make it again.
pub unsafe fn snapshot() {
    let lock = crate::sync::device_lock();
    let live: Vec<usize> = LIVE.get(&lock).clone();
    let (back, depth) = (BACK_BUFFER.load(Ordering::Relaxed) as usize, DEPTH_BUFFER.load(Ordering::Relaxed) as usize);
    // The textures before their level surfaces.
    let order = |kind: &Kind| match kind {
        Kind::Texture => 0,
        Kind::LevelSurface => 1,
        _ => 2,
    };
    let mut sorted = live;
    sorted.sort_by_key(|&o| order(&(*(o as *const Object)).kind));
    let mut count = [0u32; 4];
    for object in sorted {
        let o = &*(object as *const Object);
        let handle = object as u32;
        let address = &handle as *const u32 as u32;
        let kept_texture = |t: &Object| t.usage & 1 == 0 && t.level_memory.iter().any(|m| !m.is_empty());
        match o.kind {
            Kind::Texture if !kept_texture(o) => {}
            Kind::LevelSurface if o.owner.is_null() || !kept_texture(&*o.owner) => {}
            Kind::Texture => {
                crate::capture::call(CALL_CREATE_TEXTURE, &[o.width, o.height, o.levels, o.usage, o.format, o.pool, address], 0);
                for (level, memory) in o.level_memory.iter().enumerate() {
                    if !memory.is_empty() {
                        crate::capture::texture_update(object, level as u32, (o.width >> level).max(1), (o.height >> level).max(1), o.format, memory);
                    }
                }
                count[0] += 1;
            }
            Kind::LevelSurface => {
                crate::capture::call(CALL_TEXTURE_LEVEL, &[o.owner as u32, o.level, address], 0);
                count[1] += 1;
            }
            Kind::VertexBuffer | Kind::IndexBuffer => {
                let index = if o.kind == Kind::VertexBuffer { CALL_CREATE_VERTEX_BUFFER } else { CALL_CREATE_INDEX_BUFFER };
                crate::capture::call(index, &[o.length, o.usage, o.format, o.pool, address], 0);
                count[2] += 1;
            }
            Kind::Surface => {
                if object == back || object == depth {
                    continue;
                }
                crate::capture::call(CALL_CREATE_IMAGE_SURFACE, &[o.width, o.height, o.format, address], 0);
                count[3] += 1;
            }
        }
    }
    crate::log!("own host: snapshot of {} textures, {} level surfaces, {} buffers, {} surfaces", count[0], count[1], count[2], count[3]);
}

/// The CPU copies of the locked texture levels that the objects alive hold (risk 42): the
/// levels with a copy, the textures they belong to, and their bytes. For the log line at a
/// mark ([`crate::capture::mark`]).
pub fn level_copies() -> LevelCopies {
    let lock = crate::sync::device_lock();
    // SAFETY: an object in LIVE is alive until its release takes it out, which needs the
    // device lock that this function holds.
    let copies = LevelCopies::of(LIVE.get(&lock).iter().map(|&o| unsafe { (*(o as *const Object)).level_memory.as_slice() }));
    copies
}

/// The sums of [`level_copies`].
#[derive(Debug, Default, PartialEq)]
pub struct LevelCopies {
    pub levels: u32,
    pub textures: u32,
    pub bytes: u64,
}

impl LevelCopies {
    fn of<'a>(objects: impl Iterator<Item = &'a [Vec<u8>]>) -> Self {
        let mut sums = Self::default();
        for levels in objects {
            let held = levels.iter().filter(|memory| !memory.is_empty());
            let (count, bytes) = held.fold((0, 0), |(count, bytes), memory| (count + 1, bytes + memory.len() as u64));
            if count > 0 {
                sums.levels += count;
                sums.textures += 1;
                sums.bytes += bytes;
            }
        }
        sums
    }
}

/// Writes the content of one texture that is alive into the capture, as it is now, by its
/// serial ([`serial_of`]). For a capture file that holds between its frames.
pub unsafe fn write_texture(serial: u32) {
    let lock = crate::sync::device_lock();
    let Some(handle) = LIVE.get(&lock).iter().copied().find(|&o| (*(o as *const Object)).serial == serial) else { return };
    write_resource(handle);
}

/// Writes the content of one buffer or texture that is alive into the capture, as it is
/// now.
unsafe fn write_resource(handle: usize) {
    let lock = crate::sync::device_lock();
    if !LIVE.get(&lock).contains(&handle) {
        return;
    }
    let o = &*(handle as *const Object);
    match o.kind {
        // The bytes of a buffer are not in the stream: the helper draws from the records.
        Kind::VertexBuffer | Kind::IndexBuffer => {}
        Kind::Texture => {
            for (level, memory) in o.level_memory.iter().enumerate() {
                if !memory.is_empty() {
                    crate::capture::texture_update(handle, level as u32, (o.width >> level).max(1), (o.height >> level).max(1), o.format, memory);
                }
            }
        }
        _ => {}
    }
}

unsafe fn new_object(lock: &DeviceLock, kind: Kind, table: &'static Table) -> *mut Object {
    let object = Box::into_raw(Box::new(Object {
        table: table.methods(),
        references: 1,
        kind,
        serial: NEXT_SERIAL.fetch_add(1, Ordering::Relaxed),
        length: 0,
        width: 0,
        height: 0,
        levels: 1,
        usage: 0,
        format: 0,
        pool: 0,
        owner: core::ptr::null_mut(),
        level: 0,
        memory: Vec::new(),
        level_memory: Vec::new(),
    }));
    LIVE.get(&lock).push(object as usize);
    object
}

/// The serial of the next object: 1 for the first, never 0 (no texture) and never used
/// again in a session.
static NEXT_SERIAL: AtomicU32 = AtomicU32::new(1);

/// The handle of an object of the game's device in the stream: the serial of a texture or a
/// surface of this host, made with the object and never given to another. The game keeps the
/// address (it is the COM pointer of the object), and an address comes back for a later
/// object: a handle of the stream must not, or a draw that names the old texture draws the
/// new one. A buffer keeps its address (its records name the buffer as long as it lives,
/// and no draw names it after). Any other value (0, an object of another host) stays as it is.
pub unsafe fn serial_of(address: u32) -> u32 {
    match own(address) {
        Some(o) if !matches!(o.kind, Kind::VertexBuffer | Kind::IndexBuffer) => o.serial,
        _ => address,
    }
}

/// A handle that a device keeps from a Set call (a stage texture, a stream, the indices, a
/// target), with the serial of its object at that call. The device holds no count on it, so
/// the object can be released and its address taken by a later object: [`Kept::live`] gives
/// the handle only while the same object lives.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Kept {
    pub handle: u32,
    serial: u32,
}

impl Kept {
    /// No handle.
    pub const NONE: Kept = Kept { handle: 0, serial: 0 };

    /// A handle that the game sets now: its object is alive (0 for a value that is not an
    /// object of this host).
    pub unsafe fn of(handle: u32) -> Kept {
        Kept { handle, serial: own(handle).map_or(0, |o| o.serial) }
    }

    /// The handle while the object of the Set call is alive, else 0.
    pub unsafe fn live(self) -> u32 {
        if self.handle == 0 || self.serial == 0 {
            return 0;
        }
        let lock = crate::sync::device_lock();
        let alive = LIVE.get(&lock).contains(&(self.handle as usize)) && (*(self.handle as usize as *const Object)).serial == self.serial;
        if alive { self.handle } else { 0 }
    }
}

/// The textures freed since the last drain (`take_freed_textures`): their serials. Under the
/// device lock.
static FREED_TEXTURES: LockCell<Vec<u32>> = LockCell::new(Vec::new());

/// The textures freed since the last call, by serial: for the main thread, which forgets the
/// materials that name them (`material_stream::drain_freed_textures`).
pub unsafe fn take_freed_textures() -> Vec<u32> {
    let lock = crate::sync::device_lock();
    let freed = core::mem::take(&mut *FREED_TEXTURES.get(&lock));
    freed
}

// ---- the methods

unsafe extern "system" fn query_interface(_this: *mut Object, _id: usize, out: *mut usize) -> u32 {
    if !out.is_null() {
        *out = 0;
    }
    NO_INTERFACE
}

unsafe extern "system" fn add_reference(this: *mut Object) -> u32 {
    let _lock = crate::sync::device_lock();
    crate::sync::off_thread("resource", "AddRef");
    (*this).references += 1;
    (*this).references
}

unsafe extern "system" fn release(this: *mut Object) -> u32 {
    let lock = crate::sync::device_lock();
    crate::sync::off_thread("resource", "Release");
    (*this).references -= 1;
    let left = (*this).references;
    if left == 0 {
        let owner = (*this).owner;
        let live = &mut *LIVE.get(&lock);
        if let Some(at) = live.iter().position(|&o| o == this as usize) {
            live.swap_remove(at);
        }
        // A new texture can get the address.
        forget_read(&lock, this as u32);
        crate::map_fog::forget(&lock, this as u32);
        // The helper keeps a texture and a level surface under the handle until this record.
        if matches!((*this).kind, Kind::Texture | Kind::LevelSurface) {
            crate::capture::texture_gone(this as u32);
        }
        if (*this).kind == Kind::Texture {
            FREED_TEXTURES.get(&lock).push((*this).serial);
        }
        drop(Box::from_raw(this));
        if !owner.is_null() {
            release(owner);
        }
    }
    left
}

// Methods that this host does not need: they take their arguments and say that all is well.
unsafe extern "system" fn nothing0(_this: *mut Object) -> u32 {
    OK
}
unsafe extern "system" fn nothing1(_this: *mut Object, _a: u32) -> u32 {
    OK
}
unsafe extern "system" fn nothing3(_this: *mut Object, _a: u32, _b: u32, _c: u32) -> u32 {
    OK
}
unsafe extern "system" fn nothing4(_this: *mut Object, _a: u32, _b: u32, _c: u32, _d: u32) -> u32 {
    OK
}
unsafe extern "system" fn fail1(_this: *mut Object, _a: u32) -> u32 {
    FAIL
}
unsafe extern "system" fn fail2(_this: *mut Object, _a: u32, _b: u32) -> u32 {
    FAIL
}

unsafe extern "system" fn buffer_lock(this: *mut Object, offset: u32, _size: u32, data: *mut usize, _flags: u32) -> u32 {
    let _lock = crate::sync::device_lock();
    crate::sync::off_thread("resource", "buffer Lock (own)");
    let object = &mut *this;
    if offset > object.length || data.is_null() {
        return FAIL;
    }
    *data = object.memory.as_mut_ptr() as usize + offset as usize;
    OK
}

/// `IDirect3DVertexBuffer8::GetDesc` and `IDirect3DIndexBuffer8::GetDesc`: format, type,
/// usage, pool, size, and for a vertex buffer the vertex format.
unsafe extern "system" fn buffer_description(this: *mut Object, description: *mut u32) -> u32 {
    let object = &*this;
    let vertex = object.kind == Kind::VertexBuffer;
    // D3DFMT_VERTEXDATA is 100. D3DRTYPE_VERTEXBUFFER is 6 and D3DRTYPE_INDEXBUFFER is 7.
    let words = [if vertex { 100 } else { object.format }, if vertex { 6 } else { 7 }, object.usage, object.pool, object.length];
    core::ptr::copy_nonoverlapping(words.as_ptr(), description, 5);
    if vertex {
        *description.add(5) = object.format;
    }
    OK
}

unsafe extern "system" fn texture_level_count(this: *mut Object) -> u32 {
    (*this).levels
}

/// A D3DSURFACE_DESC: format, type, usage, pool, size, multisample type, width, height.
unsafe fn write_surface_description(object: &Object, level: u32, description: *mut u32) {
    let (width, height) = ((object.width >> level).max(1), (object.height >> level).max(1));
    let words = [object.format, 1, object.usage, object.pool, level_size(object.format, width, height).0 as u32, 0, width, height];
    core::ptr::copy_nonoverlapping(words.as_ptr(), description, 8);
}

unsafe extern "system" fn texture_level_description(this: *mut Object, level: u32, description: *mut u32) -> u32 {
    if level >= (*this).levels {
        return FAIL;
    }
    write_surface_description(&*this, level, description);
    OK
}

unsafe extern "system" fn texture_surface_level(this: *mut Object, level: u32, out: *mut usize) -> u32 {
    let lock = crate::sync::device_lock();
    crate::sync::off_thread("resource", "texture GetSurfaceLevel");
    if level >= (*this).levels || out.is_null() {
        return FAIL;
    }
    let surface = new_object(&lock, Kind::LevelSurface, &SURFACE_TABLE);
    (*surface).owner = this;
    (*surface).level = level;
    (*surface).width = ((*this).width >> level).max(1);
    (*surface).height = ((*this).height >> level).max(1);
    (*surface).format = (*this).format;
    (*surface).usage = (*this).usage;
    (*surface).pool = (*this).pool;
    add_reference(this);
    *out = surface as *const () as usize;
    OK
}

/// Gives the memory of a level of a texture: a D3DLOCKED_RECT is the bytes of a row and the
/// address.
unsafe fn lock_level(texture: *mut Object, level: u32, locked: *mut u32) -> u32 {
    let object = &mut *texture;
    if level >= object.levels || locked.is_null() {
        return FAIL;
    }
    if object.level_memory.len() < object.levels as usize {
        object.level_memory.resize(object.levels as usize, Vec::new());
    }
    let (size, row) = level_size(object.format, object.width >> level, object.height >> level);
    let memory = &mut object.level_memory[level as usize];
    if memory.len() != size {
        memory.resize(size, 0);
    }
    *locked = row;
    *locked.add(1) = memory.as_mut_ptr() as u32;
    OK
}

/// The game wrote a level of a texture: its bytes go into the stream.
unsafe fn unlock_level(texture: *mut Object, level: u32) -> u32 {
    let object = &*texture;
    if let Some(memory) = object.level_memory.get(level as usize) {
        if !memory.is_empty() {
            crate::capture::texture_update(texture as usize, level, (object.width >> level).max(1), (object.height >> level).max(1), object.format, memory);
        }
    }
    OK
}

unsafe extern "system" fn texture_lock(this: *mut Object, level: u32, locked: *mut u32, _rectangle: usize, _flags: u32) -> u32 {
    let _lock = crate::sync::device_lock();
    crate::sync::off_thread("resource", "texture LockRect");
    lock_level(this, level, locked)
}

unsafe extern "system" fn texture_unlock(this: *mut Object, level: u32) -> u32 {
    let _lock = crate::sync::device_lock();
    crate::sync::off_thread("resource", "texture UnlockRect");
    unlock_level(this, level)
}

unsafe extern "system" fn surface_description(this: *mut Object, description: *mut u32) -> u32 {
    write_surface_description(&*this, 0, description);
    OK
}

/// The object of this host at this address, or [`None`] for an object of Direct3D.
unsafe fn own(object: u32) -> Option<&'static mut Object> {
    if object == 0 {
        return None;
    }
    let object = object as usize as *mut Object;
    let table = (*object).table;
    [&BUFFER_TABLE, &TEXTURE_TABLE, &SURFACE_TABLE].iter().any(|t| t.methods() == table).then(|| &mut *object)
}

/// The target that a surface is, as the stream names it: the texture of a level 0 surface
/// of a rendered texture, or 0 for the back buffer. D3DUSAGE_RENDERTARGET is 1.
unsafe fn target_of(surface: &Object) -> Option<u32> {
    if surface as *const Object as u32 == BACK_BUFFER.load(Ordering::Relaxed) {
        return Some(0);
    }
    (surface.kind == Kind::LevelSurface && surface.level == 0 && (*surface.owner).usage & 1 != 0).then_some(surface.owner as u32)
}

/// The rendered texture that a surface of this host is level 0 of, or 0. This host makes a
/// new surface object for each `GetSurfaceLevel`, so the address of a surface does not name
/// the texture.
/// The width and the height of a texture of this host, or [`None`] for another object.
pub unsafe fn texture_size(handle: u32) -> Option<(u32, u32)> {
    own(handle).filter(|object| object.kind == Kind::Texture).map(|object| (object.width, object.height))
}

pub unsafe fn texture_of(surface: u32) -> u32 {
    own(surface).and_then(|surface| target_of(surface)).unwrap_or(0)
}

/// The memory of a surface with 4 bytes for each pixel, made when it is not there.
unsafe fn pixel_memory(surface: &mut Object) -> Option<&'static mut Vec<u8>> {
    // D3DFMT_A8R8G8B8 is 21 and D3DFMT_X8R8G8B8 is 22.
    if !(surface.format == 21 || surface.format == 22) {
        return None;
    }
    let size = (surface.width.max(1) * surface.height.max(1) * 4) as usize;
    let memory: &'static mut Vec<u8> = if surface.kind == Kind::LevelSurface {
        let texture = &mut *surface.owner;
        if texture.level_memory.len() < texture.levels as usize {
            texture.level_memory.resize(texture.levels as usize, Vec::new());
        }
        &mut *(&mut texture.level_memory[surface.level as usize] as *mut Vec<u8>)
    } else {
        &mut *(&mut surface.memory as *mut Vec<u8>)
    };
    if memory.len() != size {
        memory.resize(size, 0);
    }
    Some(memory)
}

/// Gets the pixels of a target from the host program of the stream into the memory of a
/// surface of the same size.
///
/// A rendered texture that is not the target of the draws now gives the pixels of its last
/// read again when nothing drew into it after that read ([`READ_CACHE`]): the fog of war of the
/// map menu reads the same textures again and again (records entry 55).
unsafe fn read_target(lock: &DeviceLock, target: u32, into: &mut Object) -> bool {
    let (into_width, into_height) = (into.width, into.height);
    let Some(memory) = pixel_memory(into) else { return false };
    let cacheable = target != 0 && read_cache_on() && texture_of(CURRENT_TARGET.load(Ordering::Relaxed)) != target;
    if cacheable {
        if let Some(kept) = READ_CACHE.get(lock).get(&target) {
            if kept.len() == memory.len() {
                memory.copy_from_slice(kept);
                *CACHED_READS.get(lock) += 1;
                return true;
            }
        }
    }
    let start = crate::capture::ticks();
    let read = crate::capture::read_pixels(target);
    {
        // The reads by the size of the target (the fog of war, a map tile, the character
        // image, the back buffer): the count and the wait, for the report.
        let waited = crate::capture::ticks() - start;
        let mut reads = READS_BY_SIZE.get(&lock);
        let entry = reads.entry((into_width, into_height, target == 0)).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += waited;
    }
    let Some((width, height, pixels)) = read else { return false };
    if (width, height) != (into_width, into_height) {
        return false;
    }
    memory.copy_from_slice(pixels);
    *PIXEL_READS.get(&lock) += 1;
    if cacheable {
        READ_CACHE.get(lock).insert(target, pixels.to_vec());
    }
    true
}

/// The pixels of the last read of each rendered texture, by its handle, while nothing drew
/// into the texture after the read. A texture leaves the map when it becomes the target of
/// the draws or stops being it, when pixels are copied into it, and when it is freed. The
/// current target gives and keeps no read.
static READ_CACHE: LockCell<std::collections::BTreeMap<u32, Vec<u8>>> = LockCell::new(std::collections::BTreeMap::new());
static CACHED_READS: LockCell<u32> = LockCell::new(0);

/// `NIRENDER_READ_CACHE=0` turns the cache of the reads off, for an A/B.
fn read_cache_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| crate::switches::on("NIRENDER_READ_CACHE") != Some(false))
}

/// Something can draw into `target` (a texture handle, or 0): its read is not kept.
fn forget_read(lock: &DeviceLock, target: u32) {
    if target != 0 {
        READ_CACHE.get(lock).remove(&target);
    }
}

/// `IDirect3DDevice8::CopyRects` for all of a surface.
unsafe fn copy_pixels(lock: &DeviceLock, source: u32, destination: u32) -> u32 {
    let (Some(source), Some(destination)) = (own(source), own(destination)) else { return OK };
    if let Some(target) = target_of(destination) {
        // Into a target: its kept read is old now.
        forget_read(lock, target);
        crate::map_fog::forget(lock, target);
    }
    if let Some(target) = target_of(source) {
        // Out of a target: the pixels come from the host program.
        read_target(&lock, target, destination);
    } else if target_of(destination).is_none() && (source.width, source.height, source.format) == (destination.width, destination.height, destination.format) {
        // Between two surfaces with memory here.
        if let (Some(from), Some(to)) = (pixel_memory(source), pixel_memory(destination)) {
            to.copy_from_slice(from);
        }
    }
    // Into a target: the stream has the call with the pixels of the source.
    OK
}

static PIXEL_READS: LockCell<u32> = LockCell::new(0);
/// The reads by (width, height, from the back buffer): the count and the wait in ticks.
static READS_BY_SIZE: LockCell<std::collections::BTreeMap<(u32, u32, bool), (u32, i64)>> = LockCell::new(std::collections::BTreeMap::new());

unsafe extern "system" fn surface_lock(this: *mut Object, locked: *mut u32, _rectangle: usize, _flags: u32) -> u32 {
    let lock = crate::sync::device_lock();
    crate::sync::off_thread("resource", "surface LockRect");
    let object = &mut *this;
    if object.kind == Kind::LevelSurface {
        return lock_level(object.owner, object.level, locked);
    }
    if locked.is_null() {
        return FAIL;
    }
    if this as u32 == BACK_BUFFER.load(Ordering::Relaxed) {
        // The game reads the picture of the last frame: a screenshot.
        read_target(&lock, 0, object);
    }
    let (size, row) = level_size(object.format, object.width, object.height);
    if object.memory.len() != size {
        object.memory.resize(size, 0);
    }
    *locked = row;
    *locked.add(1) = object.memory.as_mut_ptr() as u32;
    OK
}

unsafe extern "system" fn surface_unlock(this: *mut Object) -> u32 {
    let _lock = crate::sync::device_lock();
    crate::sync::off_thread("resource", "surface UnlockRect");
    let object = &*this;
    if object.kind == Kind::LevelSurface {
        return unlock_level(object.owner, object.level);
    }
    OK
}

// ---- the method tables

/// The tables have room for the copy that the buffer recorder makes: 32 methods and one word
/// before them.
const TABLE_WORDS: usize = 34;

struct Table {
    words: core::cell::UnsafeCell<[usize; TABLE_WORDS]>,
    made: AtomicBool,
    fill: unsafe fn(&mut [usize]),
}

unsafe impl Sync for Table {}

impl Table {
    const fn new(fill: unsafe fn(&mut [usize])) -> Table {
        Table { words: core::cell::UnsafeCell::new([0; TABLE_WORDS]), made: AtomicBool::new(false), fill }
    }

    /// The address of the first method.
    unsafe fn methods(&self) -> *const usize {
        let words = &mut *self.words.get();
        if !self.made.swap(true, Ordering::Relaxed) {
            // A method that the table does not name: the call must not jump to 0.
            words.fill(nothing0 as *const () as usize);
            (self.fill)(&mut words[1..]);
        }
        words.as_ptr().add(1)
    }
}

unsafe fn fill_unknown(methods: &mut [usize]) {
    methods[0] = query_interface as *const () as usize;
    methods[1] = add_reference as *const () as usize;
    methods[2] = release as *const () as usize;
    // GetDevice, SetPrivateData, GetPrivateData, FreePrivateData.
    methods[3] = fail1 as *const () as usize;
    methods[4] = nothing4 as *const () as usize;
    methods[5] = nothing3 as *const () as usize;
    methods[6] = nothing1 as *const () as usize;
}

/// `IDirect3DVertexBuffer8` and `IDirect3DIndexBuffer8`.
unsafe fn fill_buffer(methods: &mut [usize]) {
    fill_unknown(methods);
    // SetPriority, GetPriority, PreLoad, GetType.
    methods[7] = nothing1 as *const () as usize;
    methods[11] = buffer_lock as *const () as usize;
    methods[13] = buffer_description as *const () as usize;
}

/// `IDirect3DTexture8`.
unsafe fn fill_texture(methods: &mut [usize]) {
    fill_unknown(methods);
    // SetPriority, GetPriority, PreLoad, GetType, SetLOD, GetLOD.
    methods[7] = nothing1 as *const () as usize;
    methods[11] = nothing1 as *const () as usize;
    methods[13] = texture_level_count as *const () as usize;
    methods[14] = texture_level_description as *const () as usize;
    methods[15] = texture_surface_level as *const () as usize;
    methods[16] = texture_lock as *const () as usize;
    methods[17] = texture_unlock as *const () as usize;
    // AddDirtyRect.
    methods[18] = nothing1 as *const () as usize;
}

/// `IDirect3DSurface8`.
unsafe fn fill_surface(methods: &mut [usize]) {
    fill_unknown(methods);
    // GetContainer.
    methods[7] = fail2 as *const () as usize;
    methods[8] = surface_description as *const () as usize;
    methods[9] = surface_lock as *const () as usize;
    methods[10] = surface_unlock as *const () as usize;
}

static BUFFER_TABLE: Table = Table::new(fill_buffer);
static TEXTURE_TABLE: Table = Table::new(fill_texture);
static SURFACE_TABLE: Table = Table::new(fill_surface);

/// The address of method `index` of an object, from its table. The object can be of this
/// host or of Direct3D.
unsafe fn method(object: u32, index: usize) -> usize {
    let table: usize = *(object as usize as *const usize);
    *((table + index * 4) as *const usize)
}

// ---- the functions of the host table

/// The origin of the scene in progress, from its packet: the bits of three f64, and whether
/// the scene has one. `camera_space` of `camera_space.rs` asks for it at each draw.
static SCENE_ORIGIN: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
static SCENE_RELATIVE: AtomicBool = AtomicBool::new(false);

unsafe extern "C" fn scene_begin(scene: *const SceneV1) {
    if scene.is_null() {
        SCENE_RELATIVE.store(false, Ordering::Relaxed);
        return;
    }
    let scene = &*scene;
    for i in 0..3 {
        SCENE_ORIGIN[i].store(scene.camera_origin[i].to_bits(), Ordering::Relaxed);
    }
    SCENE_RELATIVE.store(scene.flags & SCENE_HAS_CAMERA_ORIGIN != 0, Ordering::Relaxed);
}
unsafe extern "C" fn scene_end() {}
unsafe extern "C" fn frame_event(_kind: u32, _a: u32, _b: u32) {}
unsafe extern "C" fn draw(_draw: *const DrawV1) {}
unsafe extern "C" fn draw_state(_state: *const DrawStateV1) {}
unsafe extern "C" fn command(_command: u32, _a: u32, _b: u32, _c: u32) {}

unsafe extern "C" fn read_counters(out: *mut CountersV1, _reset: u32) {
    if !out.is_null() {
        let size = (*out).struct_size as usize;
        core::ptr::write_bytes(out as *mut u8, 0, size.min(core::mem::size_of::<CountersV1>()));
        (*out).struct_size = size as u32;
    }
}

unsafe extern "C" fn use_facts(mask: u32) -> u32 {
    mask
}

unsafe extern "C" fn use_kinds(mask: u32) -> u32 {
    mask
}

unsafe extern "C" fn world_space() -> u32 {
    0
}

/// The scene in progress is in camera space when its packet had an origin. Then the draws
/// of the game get world matrices relative to that origin (`camera_space::own`), and the
/// records of the stream say so.
unsafe extern "C" fn camera_space(origin: *mut f64) -> u32 {
    if !SCENE_RELATIVE.load(Ordering::Relaxed) {
        return 0;
    }
    if !origin.is_null() {
        for i in 0..3 {
            *origin.add(i) = f64::from_bits(SCENE_ORIGIN[i].load(Ordering::Relaxed));
        }
    }
    1
}

unsafe extern "C" fn texture_create(texture: *const TextureV1) -> u32 {
    let lock = crate::sync::device_lock();
    crate::sync::off_thread("resource", "host texture_create");
    let texture = &*texture;
    let object = new_object(&lock, Kind::Texture, &TEXTURE_TABLE);
    (*object).width = texture.width;
    (*object).height = texture.height;
    (*object).levels = texture.levels.max(1);
    (*object).format = texture.d3d_format;
    object as u32
}

static BACK_BUFFER: AtomicU32 = AtomicU32::new(0);
static DEPTH_BUFFER: AtomicU32 = AtomicU32::new(0);

/// The surface that stands for the back buffer or for its depth buffer, with one more
/// reference.
unsafe fn target_surface(slot: &AtomicU32, format: u32) -> u32 {
    let lock = crate::sync::device_lock();
    let mut surface = slot.load(Ordering::Relaxed);
    if surface == 0 {
        let object = new_object(&lock, Kind::Surface, &SURFACE_TABLE);
        let renderer = crate::engine::renderer();
        let size: [u32; 2] = if crate::own_device::present() {
            let (width, height) = crate::own_device::size();
            [width, height]
        } else if renderer != 0 {
            crate::mem::rd(renderer, crate::gen::layouts::NiDX8Renderer::d3dPresentParameters)
        } else {
            [640, 480]
        };
        (*object).width = size[0];
        (*object).height = size[1];
        (*object).format = format;
        surface = object as u32;
        slot.store(surface, Ordering::Relaxed);
    }
    add_reference(surface as usize as *mut Object);
    surface
}

unsafe fn put(out: u32, value: u32) -> u32 {
    if out == 0 {
        return FAIL;
    }
    *(out as usize as *mut u32) = value;
    OK
}

unsafe extern "C" fn call(index: u32, arguments: *const u32) -> u32 {
    let lock = crate::sync::device_lock();
    crate::sync::off_thread("resource", "host call");
    let a = |i: usize| *arguments.add(i);
    match index {
        CALL_CREATE_VERTEX_BUFFER | CALL_CREATE_INDEX_BUFFER => {
            // Length, usage, vertex format or index format, pool, address for the handle.
            let vertex = index == CALL_CREATE_VERTEX_BUFFER;
            let object = new_object(&lock, if vertex { Kind::VertexBuffer } else { Kind::IndexBuffer }, &BUFFER_TABLE);
            (*object).length = a(0);
            (*object).usage = a(1);
            (*object).format = a(2);
            (*object).pool = a(3);
            (*object).memory = vec![0; a(0) as usize];
            put(a(4), object as u32)
        }
        CALL_CREATE_TEXTURE => {
            // Width, height, levels, usage, format, pool, address for the handle.
            let object = new_object(&lock, Kind::Texture, &TEXTURE_TABLE);
            (*object).width = a(0);
            (*object).height = a(1);
            (*object).levels = a(2).max(1);
            (*object).usage = a(3);
            (*object).format = a(4);
            (*object).pool = a(5);
            put(a(6), object as u32)
        }
        CALL_CREATE_DEPTH_BUFFER | CALL_CREATE_IMAGE_SURFACE => {
            // Width, height, format, then the address for the handle: after one more word
            // for a depth buffer.
            let object = new_object(&lock, Kind::Surface, &SURFACE_TABLE);
            (*object).width = a(0);
            (*object).height = a(1);
            (*object).format = a(2);
            put(a(if index == CALL_CREATE_DEPTH_BUFFER { 4 } else { 3 }), object as u32)
        }
        // D3DFMT_X8R8G8B8 is 22 and D3DFMT_D24S8 is 75.
        CALL_GET_RENDER_TARGET => put(a(0), target_surface(&BACK_BUFFER, 22)),
        CALL_GET_DEPTH_BUFFER => put(a(0), target_surface(&DEPTH_BUFFER, 75)),
        // The calls on a resource go through its table.
        CALL_ADD_REF => core::mem::transmute::<usize, unsafe extern "system" fn(u32) -> u32>(method(a(0), 1))(a(0)),
        CALL_RELEASE => core::mem::transmute::<usize, unsafe extern "system" fn(u32) -> u32>(method(a(0), 2))(a(0)),
        CALL_BUFFER_LOCK => {
            core::mem::transmute::<usize, unsafe extern "system" fn(u32, u32, u32, u32, u32) -> u32>(method(a(0), 11))(a(0), a(1), a(2), a(3), a(4))
        }
        CALL_BUFFER_UNLOCK => core::mem::transmute::<usize, unsafe extern "system" fn(u32) -> u32>(method(a(0), 12))(a(0)),
        CALL_BUFFER_DESCRIPTION => core::mem::transmute::<usize, unsafe extern "system" fn(u32, u32) -> u32>(method(a(0), 13))(a(0), a(1)),
        CALL_TEXTURE_LEVEL => core::mem::transmute::<usize, unsafe extern "system" fn(u32, u32, u32) -> u32>(method(a(0), 15))(a(0), a(1), a(2)),
        CALL_SURFACE_DESCRIPTION => core::mem::transmute::<usize, unsafe extern "system" fn(u32, u32) -> u32>(method(a(0), 8))(a(0), a(1)),
        CALL_SURFACE_LOCK => {
            core::mem::transmute::<usize, unsafe extern "system" fn(u32, u32, u32, u32) -> u32>(method(a(0), 9))(a(0), a(1), a(2), a(3))
        }
        CALL_SURFACE_UNLOCK => core::mem::transmute::<usize, unsafe extern "system" fn(u32) -> u32>(method(a(0), 10))(a(0)),
        CALL_CUBE_FACE => FAIL,
        // Source surface, rectangles, their number, destination surface, points.
        CALL_COPY_PIXELS => copy_pixels(&lock, a(0), a(3)),
        // The colour surface and the depth surface.
        CALL_RENDER_TARGET => {
            // The old and the new target can get draws: their kept reads are old. (A test of
            // 2026-10-21 that dropped a read only after a draw or a clear kept the same 41 of
            // 100 reads of a crossing run: records entry 55.)
            forget_read(&lock, texture_of(CURRENT_TARGET.load(Ordering::Relaxed)));
            forget_read(&lock, texture_of(a(0)));
            // The device holds a reference to its colour and depth surfaces, as
            // `IDirect3DDevice8::SetRenderTarget` does: a surface set here lives until another
            // one replaces it. The new ones first: the same surface again stays alive.
            add_reference_of(a(0));
            add_reference_of(a(1));
            release_of(CURRENT_TARGET.swap(a(0), Ordering::Relaxed));
            release_of(CURRENT_DEPTH.swap(a(1), Ordering::Relaxed));
            OK
        }
        // The frame, the scene, the target: nothing to do here. The stream has the call.
        _ => OK,
    }
}

static TABLE: HostV1 = HostV1 {
    struct_size: core::mem::size_of::<HostV1>() as u32,
    version: VERSION,
    capabilities: 0xFF,
    reserved: 0,
    scene_begin,
    scene_end,
    frame_event,
    draw,
    read_counters,
    use_facts,
    draw_state,
    texture_create,
    world_space,
    camera_space,
    command,
    call,
    use_kinds,
};

/// The device calls of other code that this host took and did not do, by the method.
static TAKEN: LockCell<[u32; 100]> = LockCell::new([0; 100]);

/// A device call of other code that has no call of the link. It must not get to the device
/// of MGE XE: it can have an object of this host as an argument.
pub unsafe fn take(slot: usize) {
    let lock = crate::sync::device_lock();
    if let Some(count) = TAKEN.get(&lock).get_mut(slot) {
        *count += 1;
    };
}

/// The colour surface that the game draws into now: the last one of a call to set the
/// render target. 0 before the first call. The device holds a reference to it.
static CURRENT_TARGET: AtomicU32 = AtomicU32::new(0);
/// The depth surface of the last call to set the render target. The device holds a reference
/// to it.
static CURRENT_DEPTH: AtomicU32 = AtomicU32::new(0);

/// `AddRef` of a resource through its table (an object of this host or of Direct3D), when
/// there is one.
unsafe fn add_reference_of(object: u32) {
    if object != 0 {
        core::mem::transmute::<usize, unsafe extern "system" fn(u32) -> u32>(method(object, 1))(object);
    }
}

/// `Release` of a resource through its table, when there is one.
unsafe fn release_of(object: u32) {
    if object != 0 {
        core::mem::transmute::<usize, unsafe extern "system" fn(u32) -> u32>(method(object, 2))(object);
    }
}

/// True when the game draws into the back buffer now, and not into a texture.
pub fn target_is_back_buffer() -> bool {
    let target = CURRENT_TARGET.load(Ordering::Relaxed);
    target == 0 || target == BACK_BUFFER.load(Ordering::Relaxed)
}

/// The table of this host, for a link without MGE XE.
pub fn table() -> &'static HostV1 {
    &TABLE
}

/// The device of the game is the one of the plugin: this host is the only one, from the
/// start. The link gets the table when the ports connect.
pub unsafe fn activate() {
    ACTIVE.store(true, Ordering::Relaxed);
}

/// A call of the link from the device of the plugin, before the ports use the link or for a
/// call that no port made. The stream gets the call.
pub unsafe fn direct_call(index: u32, arguments: &[u32]) -> u32 {
    let result = call(index, arguments.as_ptr());
    crate::capture::call(index, arguments, result);
    result
}

/// The plugin is the host of the render link from now on.
pub unsafe fn take_over() {
    crate::link_client::set_host(&TABLE);
    ACTIVE.store(true, Ordering::Relaxed);
    crate::log!("own host: the plugin answers the calls of the render link from now on");
}

/// One line with the device methods that this host took, for the log.
pub unsafe fn report() -> String {
    let lock = crate::sync::device_lock();
    let taken = &*TAKEN.get(&lock);
    let mut text = String::new();
    for (slot, count) in taken.iter().enumerate() {
        if *count != 0 {
            text.push_str(&format!(" {}:{}", crate::device::NAMES.get(slot).copied().unwrap_or("?"), count));
        }
    }
    if text.is_empty() {
        text.push_str(" none");
    }
    let per_second = crate::capture::ticks_per_second() as f64;
    let mut sizes = String::new();
    for ((width, height, back), (count, ticks)) in READS_BY_SIZE.get(&lock).iter() {
        sizes.push_str(&format!(" {width}x{height}{}: {count} reads, {:.1} ms;", if *back { " (back buffer)" } else { "" }, *ticks as f64 * 1000.0 / per_second));
    }
    format!(
        "own host: {} pixel reads from the host program, by target size:{sizes} {} reads from the kept pixels{}; device calls without a call of the link, taken and not done:{text}",
        *PIXEL_READS.get(&lock),
        *CACHED_READS.get(&lock),
        if read_cache_on() { "" } else { " (NIRENDER_READ_CACHE=0)" }
    )
}

#[cfg(test)]
mod tests {
    use super::LevelCopies;

    #[test]
    fn the_level_copies_count_only_the_levels_that_hold_bytes() {
        let texture = vec![vec![0u8; 64], Vec::new(), vec![0u8; 16]];
        let never_locked: Vec<Vec<u8>> = vec![Vec::new(); 3];
        let other = vec![vec![0u8; 4]];
        let objects: [&[Vec<u8>]; 4] = [&texture, &never_locked, &[], &other];
        assert_eq!(LevelCopies::of(objects.into_iter()), LevelCopies { levels: 3, textures: 2, bytes: 84 });
    }
}
