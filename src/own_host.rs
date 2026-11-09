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
    /// the lock of the level and freed at its unlock, once its bytes are in the stream
    /// ([`unlock_level`]).
    memory: Vec<u8>,
    level_memory: Vec<Vec<u8>>,
    /// The levels whose block an unlock freed (bit `level`), until the next lock of the level.
    freed_levels: u32,
    /// The game locked a level of this texture again after its block was freed: a texture that
    /// changes (a new revision of its pixel data). Its unlocks keep their blocks from then on,
    /// so that each change reuses them.
    dynamic: bool,
    /// A texture of the link (`texture_create`): a source texture of the engine, which a capture
    /// file reads from its own pixel data (`capture::name_texture`). False for a texture of a
    /// device call.
    from_link: bool,
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
/// memory here and is not written: the capture reads it for the file when a material names it
/// (`capture::name_texture`). A rendered texture is written with no bytes: its picture is in the
/// helper (`rendered_pictures`).
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
    // The textures that an unlock freed a level of (risk 42): from the link, and from a device
    // call. The file reads a texture of the link at its first naming (`capture::name_texture`);
    // one of a device call has no other way into the file.
    let mut freed = [0u32; 2];
    for object in sorted {
        let o = &*(object as *const Object);
        let handle = object as u32;
        let address = &handle as *const u32 as u32;
        let in_file = |t: &Object| in_snapshot(t);
        match o.kind {
            Kind::Texture if !in_file(o) => {
                if o.freed_levels != 0 && !is_rendered(o) {
                    freed[usize::from(!o.from_link)] += 1;
                }
            }
            Kind::LevelSurface if o.owner.is_null() || !in_file(&*o.owner) => {}
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
    crate::log!(
        "own host: {} textures of the link with freed levels come into the file at their naming; {} of a device call with freed levels are not in it",
        freed[0],
        freed[1]
    );
}

/// Whether the snapshot of a capture file writes a texture: a rendered one always (its picture
/// comes from the helper), another one when it holds the bytes of a level and no unlock freed a
/// level of it. A texture with a freed level comes into the file whole by its naming
/// (`capture::name_texture`), not in part from here: a create record here would mark it as in
/// the file, and the naming would not read it.
fn in_snapshot(texture: &Object) -> bool {
    is_rendered(texture) || (texture.freed_levels == 0 && texture.level_memory.iter().any(|memory| !memory.is_empty()))
}

/// D3DUSAGE_RENDERTARGET: the game draws into the texture.
fn is_rendered(texture: &Object) -> bool {
    texture.usage & 1 != 0
}

/// The picture of a rendered texture, for a capture file.
pub struct RenderedPicture {
    texture: u32,
    format: u32,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

/// Reads the picture of each rendered texture that is alive from the helper. The game draws a
/// rendered texture only when it needs a new picture (the fog of war of a map tile, a texture
/// of a mod), so a file's frames need not hold it. A texture with no answer goes into the file
/// with no picture.
///
/// The reads stop at a time budget: the engine sets no limit on the map's tiles
/// (`MapController` 0x41F680), and a press of the mark must not stall the game for long. The
/// largest textures go first. A texture not read is blank in a replay only.
///
/// # Safety
///
/// With the own host in use, before a capture file opens: a read is a record of the stream, and
/// the caller does not hold the state open. Each read waits for the helper.
pub unsafe fn rendered_pictures() -> Vec<RenderedPicture> {
    let lock = crate::sync::device_lock();
    let mut textures: Vec<usize> = LIVE.get(&lock).iter().copied().filter(|&o| (*(o as *const Object)).kind == Kind::Texture && is_rendered(&*(o as *const Object))).collect();
    textures.sort_by_key(|&o| std::cmp::Reverse((*(o as *const Object)).width * (*(o as *const Object)).height));
    let mut sizes = std::collections::BTreeMap::<(u32, u32), u32>::new();
    for &o in &textures {
        let o = &*(o as *const Object);
        *sizes.entry((o.width, o.height)).or_default() += 1;
    }
    let sizes: Vec<String> = sizes.iter().map(|((width, height), count)| format!("{count} of {width} x {height}")).collect();

    let start = crate::capture::ticks();
    let budget = crate::switches::number("NIRENDER_CAPTURE_READS_MS", READS_BUDGET_MS) as i64 * crate::capture::ticks_per_second() / 1000;
    let count = textures.len();
    let (pictures, ended) = read_within(textures, budget, || crate::capture::ticks() - start, |object| {
        let (width, height, pixels) = crate::capture::read_pixels(object as u32)?;
        Some(RenderedPicture { texture: object as u32, format: (*(object as *const Object)).format, width, height, pixels: pixels.to_vec() })
    });

    // A file that holds does not count this frame in its worst frame: this line has the time.
    let spent = (crate::capture::ticks() - start) as f64 * 1000.0 / crate::capture::ticks_per_second() as f64;
    let end = if ended { ", the budget ended the reads" } else { "" };
    crate::log!("capture: {count} rendered textures ({}), {} with their pictures read in {spent:.1} ms{end}", sizes.join(", "), pictures.len());
    pictures
}

/// The time for the reads of [`rendered_pictures`], in milliseconds: about 300 textures of a fog
/// tile, a map of 17 x 17 tiles. `NIRENDER_CAPTURE_READS_MS` sets another, for a test.
const READS_BUDGET_MS: u32 = 500;

/// Reads the items in their order while `elapsed` is under `budget`, looked at before each read.
/// Gives what the reads gave, and true when the budget left items unread.
fn read_within<T, R>(items: Vec<T>, budget: i64, elapsed: impl Fn() -> i64, mut read: impl FnMut(T) -> Option<R>) -> (Vec<R>, bool) {
    let mut done = Vec::new();
    for item in items {
        if elapsed() >= budget {
            return (done, true);
        }
        done.extend(read(item));
    }
    (done, false)
}

/// Writes a picture of [`rendered_pictures`] into the capture file. After the snapshot, which
/// makes its texture.
///
/// # Safety
///
/// Under the device lock since [`rendered_pictures`], so that the picture's texture is alive.
pub unsafe fn write_picture(picture: &RenderedPicture) {
    crate::capture::rendered_picture(picture.texture, picture.width, picture.height, picture.format, &picture.pixels);
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

/// The textures that keep their level copies because they change (the field `dynamic` of an
/// object), and the bytes of those copies. For the log line at a mark.
pub fn dynamic_copies() -> (u32, u64) {
    let lock = crate::sync::device_lock();
    let mut sums = (0u32, 0u64);
    for &object in LIVE.get(&lock).iter() {
        // SAFETY: an object in LIVE is alive until its release takes it out, which needs the
        // device lock that this function holds.
        let o = unsafe { &*(object as *const Object) };
        if o.dynamic {
            sums.0 += 1;
            sums.1 += o.level_memory.iter().map(|memory| memory.len() as u64).sum::<u64>();
        }
    }
    sums
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
        freed_levels: 0,
        dynamic: false,
        from_link: false,
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

/// The serial of any object of this host, for the state summary: a buffer made again at the
/// same address shows there. Any other value stays as it is.
///
/// # Safety
///
/// As for [`serial_of`]: `address` is 0, a live object of this host, or another host's.
pub unsafe fn object_serial(address: u32) -> u32 {
    own(address).map_or(address, |o| o.serial)
}

/// The back buffer and the depth buffer, as the stream names them.
///
/// # Safety
///
/// The plugin runs in the game, and the two objects live as long as the device.
pub unsafe fn target_serials() -> [u32; 2] {
    [serial_of(BACK_BUFFER.load(Ordering::Relaxed)), serial_of(DEPTH_BUFFER.load(Ordering::Relaxed))]
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
    // A lock of a freed level gives zeros: the engine's locker writes the whole level
    // (`NiDX8SourceTextureData::CopyDataToSurface` 0x6BFF40). The texture changes, so it keeps
    // its blocks from now on (`Object::dynamic`).
    let bit = level_bit(level);
    if object.freed_levels & bit != 0 {
        object.freed_levels &= !bit;
        LEVEL_FREES.locks_after_free.fetch_add(1, Ordering::Relaxed);
        if !object.dynamic {
            object.dynamic = true;
            LEVEL_FREES.made_dynamic.fetch_add(1, Ordering::Relaxed);
        }
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

/// The bit of `level` in [`Object::freed_levels`]; a level past 31 has none (a texture of
/// Direct3D 8 has at most 13 levels).
fn level_bit(level: u32) -> u32 {
    1u32.checked_shl(level).unwrap_or(0)
}

/// The game wrote a level of a texture: its bytes go into the stream, and then its block is
/// freed (risk 42). A texture that the game locked again after a free keeps its blocks
/// ([`Object::dynamic`]): a texture that changes reuses them, with no new block for each
/// change. While a capture file is open no block is freed: a file that holds writes its
/// changed textures later from these blocks (`capture::tee_frames`).
unsafe fn unlock_level(texture: *mut Object, level: u32) -> u32 {
    unlock_level_into(texture, level, crate::capture::file_open(), |width, height, format, bytes| {
        crate::capture::texture_update(texture as usize, level, width, height, format, bytes)
    })
}

/// [`unlock_level`] with the open file and the stream as arguments, for a test: `send` gets the
/// level's width, height, format and bytes.
unsafe fn unlock_level_into(texture: *mut Object, level: u32, file_open: bool, send: impl FnOnce(u32, u32, u32, &[u8])) -> u32 {
    let object = &mut *texture;
    let (width, height, format) = ((object.width >> level).max(1), (object.height >> level).max(1), object.format);
    let keep = file_open || object.dynamic;
    let Some(memory) = object.level_memory.get_mut(level as usize) else { return OK };
    if memory.is_empty() {
        return OK;
    }
    send(width, height, format, memory);
    if !keep {
        LEVEL_FREES.levels.fetch_add(1, Ordering::Relaxed);
        LEVEL_FREES.bytes.fetch_add(memory.len() as u64, Ordering::Relaxed);
        *memory = Vec::new();
        object.freed_levels |= level_bit(level);
    }
    OK
}

/// The counts of the frees at unlock since the start (risk 42), for the mark line.
pub struct LevelFrees {
    pub levels: AtomicU64,
    pub bytes: AtomicU64,
    /// Locks of a level whose block was freed: the locker gets zeros.
    pub locks_after_free: AtomicU64,
    /// The textures that such a lock made dynamic.
    pub made_dynamic: AtomicU64,
    /// Copies out of a level surface between two surfaces of this host, and the ones of them
    /// out of a freed level.
    pub copies_from_level: AtomicU64,
    pub copies_from_freed: AtomicU64,
}

pub static LEVEL_FREES: LevelFrees = LevelFrees {
    levels: AtomicU64::new(0),
    bytes: AtomicU64::new(0),
    locks_after_free: AtomicU64::new(0),
    made_dynamic: AtomicU64::new(0),
    copies_from_level: AtomicU64::new(0),
    copies_from_freed: AtomicU64::new(0),
};

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
    let Some(memory) = pixel_memory(into) else {
        *FAILED_READS.get(lock) += 1;
        return false;
    };
    let cacheable = target != 0 && texture_of(CURRENT_TARGET.load(Ordering::Relaxed)) != target;
    if cacheable {
        if let Some(kept) = READ_CACHE.get(lock).get(&target) {
            if kept.len() == memory.len() {
                memory.copy_from_slice(kept);
                *CACHED_READS.get(lock) += 1;
                return true;
            }
        }
    }
    // A read of the back buffer (a lock of it, or a copy out of it): the main view's list goes
    // before the read's record.
    if target == 0 {
        if let Some(t) = crate::main_thread::MainThread::here() {
            crate::instance_stream::send_shown_list(&t);
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
    let Some((width, height, pixels)) = read else {
        *FAILED_READS.get(lock) += 1;
        return false;
    };
    if (width, height) != (into_width, into_height) {
        *FAILED_READS.get(lock) += 1;
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
        // Between two surfaces with memory here. Out of a level surface, the count shows a
        // reader of a level after its unlock (risk 42): no engine caller is known \[I\].
        if source.kind == Kind::LevelSurface {
            LEVEL_FREES.copies_from_level.fetch_add(1, Ordering::Relaxed);
            if (*source.owner).freed_levels & level_bit(source.level) != 0 {
                LEVEL_FREES.copies_from_freed.fetch_add(1, Ordering::Relaxed);
            }
        }
        if let (Some(from), Some(to)) = (pixel_memory(source), pixel_memory(destination)) {
            to.copy_from_slice(from);
        }
    }
    // Into a target: the stream has the call with the pixels of the source.
    OK
}

static PIXEL_READS: LockCell<u32> = LockCell::new(0);
/// The reads that gave no pixels, gave pixels of another size, or had no pixel memory to write
/// into: the game's memory stays as it was, and the game's call still returns success.
static FAILED_READS: LockCell<u32> = LockCell::new(0);
/// The reads by (width, height, from the back buffer): the count and the wait in ticks.
static READS_BY_SIZE: LockCell<std::collections::BTreeMap<(u32, u32, bool), (u32, i64)>> = LockCell::new(std::collections::BTreeMap::new());

/// The offset of the first byte of a locked rectangle that starts at `left` and `top`, in a surface
/// with rows of `row` bytes and pixels of `pixel` bytes: [`None`] when it starts outside the surface.
/// A block format has no pixels to count (`pixel` 0), and it locks whole.
fn lock_offset(left: u32, top: u32, width: u32, height: u32, row: u32, pixel: u32) -> Option<usize> {
    if pixel == 0 {
        return Some(0);
    }
    (left < width && top < height).then(|| top as usize * row as usize + left as usize * pixel as usize)
}

unsafe extern "system" fn surface_lock(this: *mut Object, locked: *mut u32, rectangle: usize, _flags: u32) -> u32 {
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
    // A RECT: left, top, right, bottom. `NiDX8Renderer::TakeScreenshot` gives a width and a height as
    // the last two, so only the first two count here.
    let (left, top) = if rectangle != 0 { (crate::mem::rd::<u32>(rectangle, 0), crate::mem::rd::<u32>(rectangle, 4)) } else { (0, 0) };
    let pixel = if object.format >= 0x100 { 0 } else { row / object.width.max(1) };
    let Some(offset) = lock_offset(left, top, object.width, object.height, row, pixel) else {
        return FAIL;
    };
    *locked = row;
    *locked.add(1) = object.memory.as_mut_ptr().add(offset) as u32;
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
    (*object).from_link = true;
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
/// The same calls by the method and by the code that made the call (its return address).
static TAKEN_BY: LockCell<std::collections::BTreeMap<(usize, usize), u32>> = LockCell::new(std::collections::BTreeMap::new());
/// The lines of the log for a method and a caller seen for the first time, at most.
const TAKEN_LINES: usize = 32;

/// The return address of the call of `NiDX8LightManager::RemoveLight` to `LightEnable`.
const REMOVE_LIGHT_CALL: usize = 0x6BB5FE;
/// The return addresses of the calls of the cutscene player's `setupD3DState` and
/// `restoreD3DState` to `SetPixelShader`.
const CUTSCENE_SHADER_CALLS: [usize; 2] = [0x64EFBE, 0x64F245];

/// True for a device call of other code that changes nothing on this host, so it is not a taken
/// call. There is no device light and no pixel shader here:
/// - `LightEnable(index, false)` from `NiDX8LightManager::RemoveLight` on the cell worker. The
///   helper hears of the destroyed light from `worker_events`.
/// - `SetPixelShader(0)` from the cutscene player.
///
/// The same form from another caller stays a taken call, so that an unported path shows. This
/// only stops the count: the answer of a taken call is the same.
pub fn nothing_to_do(slot: usize, args: &[u32], caller: usize) -> bool {
    use crate::d3d::{DEVICE_LIGHT_ENABLE, DEVICE_SET_PIXEL_SHADER};
    match slot * 4 {
        DEVICE_LIGHT_ENABLE => caller == REMOVE_LIGHT_CALL && args.get(1) == Some(&0),
        DEVICE_SET_PIXEL_SHADER => CUTSCENE_SHADER_CALLS.contains(&caller) && args.first() == Some(&0),
        _ => false,
    }
}

/// A device call of other code that has no call of the link. It must not get to the device
/// of MGE XE: it can have an object of this host as an argument.
pub unsafe fn take(slot: usize) {
    let lock = crate::sync::device_lock();
    if let Some(count) = TAKEN.get(&lock).get_mut(slot) {
        *count += 1;
    };
    let mut by = TAKEN_BY.get(&lock);
    let count = by.entry((slot, crate::device::caller())).or_insert(0);
    *count += 1;
    let first = *count == 1;
    if first && by.len() <= TAKEN_LINES {
        crate::log!("own host: a device call taken and not done: {} from {}", crate::device::NAMES.get(slot).copied().unwrap_or("?"), module_and_offset(crate::device::caller()));
    }
}

/// An address as the module that holds it and the offset in it, or the address alone for code
/// outside every module.
fn module_and_offset(address: usize) -> String {
    // GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT
    const FROM_ADDRESS: u32 = 4 | 2;
    let mut module: *mut core::ffi::c_void = core::ptr::null_mut();
    // SAFETY: the two calls read no memory of the address; `module` and `name` are ours, and the
    // size is the size of `name`.
    unsafe {
        if crate::win::GetModuleHandleExW(FROM_ADDRESS, address as *const core::ffi::c_void, &mut module) == 0 {
            return format!("{address:#x}");
        }
        let mut name = [0u16; 260];
        let length = crate::win::GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) as usize;
        let path = String::from_utf16_lossy(&name[..length]);
        let file = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string();
        format!("{file}+{:#x}", address - module as usize)
    }
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

/// True when the helper has the target the game draws into now: the back buffer, or level 0
/// of a rendered texture of this host (the helper makes each such texture at its creation).
pub fn target_is_known() -> bool {
    // SAFETY: CURRENT_TARGET is 0 or a surface of this host that the host keeps a reference
    // to while it is the target (`SetRenderTarget` takes it, the next one releases it).
    target_is_back_buffer() || unsafe { texture_of(CURRENT_TARGET.load(Ordering::Relaxed)) } != 0
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
            let callers: Vec<String> = TAKEN_BY.get(&lock).iter().filter(|((method, _), _)| *method == slot).map(|((_, caller), count)| format!("{} {count}", module_and_offset(*caller))).collect();
            text.push_str(&format!(" ({})", callers.join(", ")));
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
        "own host: {} pixel reads from the host program, {} failed, by target size:{sizes} {} reads from the kept pixels; device calls without a call of the link, taken and not done:{text}",
        *PIXEL_READS.get(&lock),
        *FAILED_READS.get(&lock),
        *CACHED_READS.get(&lock),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        copy_pixels, in_snapshot, level_bit, lock_level, lock_offset, new_object, nothing_to_do, read_within, release, surface_lock, unlock_level_into, Kind, LevelCopies, Object,
        BACK_BUFFER, CUTSCENE_SHADER_CALLS, LEVEL_FREES, OK, REMOVE_LIGHT_CALL, SURFACE_TABLE, TEXTURE_TABLE,
    };
    use crate::d3d::{DEVICE_LIGHT_ENABLE, DEVICE_SET_LIGHT, DEVICE_SET_PIXEL_SHADER};
    use crate::instance_stream::{test_list_one, test_shown_state};
    use core::sync::atomic::Ordering;

    /// A surface of this host of 4 x 4 pixels of 4 bytes, for the tests of the reads.
    unsafe fn test_surface() -> *mut Object {
        let lock = crate::sync::device_lock();
        let surface = new_object(&lock, Kind::Surface, &SURFACE_TABLE);
        (*surface).width = 4;
        (*surface).height = 4;
        (*surface).format = 21;
        surface
    }

    /// A read of the back buffer (`read`, with the back buffer and another surface) sends the main
    /// view's list of a world scene before the read's record. The next read sends nothing: no
    /// scene of the list began since the first.
    unsafe fn the_list_goes_at(read: impl Fn(*mut Object, *mut Object)) {
        let _turn = crate::main_thread::TestTurn::take();
        let before = crate::shadow::main_thread_for_test();
        let mt = crate::main_thread::MainThread::claim();
        crate::capture::test_record_deletes(true);
        let (back, other) = (test_surface(), test_surface());
        let kept = BACK_BUFFER.swap(back as u32, Ordering::Relaxed);
        test_list_one(&mt, 61);
        let reads = test_shown_state(&mt).2;
        assert_eq!(test_shown_state(&mt), (vec![61], true, reads));
        read(back, other);
        assert_eq!(test_shown_state(&mt), (vec![], false, reads + 1));
        read(back, other);
        assert_eq!(test_shown_state(&mt), (vec![], false, reads + 1));
        BACK_BUFFER.store(kept, Ordering::Relaxed);
        release(back);
        release(other);
        crate::capture::test_record_deletes(false);
        crate::shadow::restore_main_thread(before);
    }

    #[test]
    fn the_list_goes_at_a_back_buffer_lock_and_starts_again() {
        unsafe {
            the_list_goes_at(|back, _| {
                let mut locked = [0u32; 2];
                assert_eq!(surface_lock(back, locked.as_mut_ptr(), 0, 0), OK);
            });
        }
    }

    #[test]
    fn the_list_goes_at_a_copy_out_of_the_back_buffer() {
        unsafe {
            the_list_goes_at(|back, other| {
                let lock = crate::sync::device_lock();
                assert_eq!(copy_pixels(&lock, back as u32, other as u32), OK);
            });
        }
    }

    #[test]
    fn the_reads_stop_at_the_budget_and_say_so() {
        // Each look at the clock is one tick later: the budget of 3 ends the reads at the fourth.
        let clock = std::cell::Cell::new(0);
        let elapsed = || {
            let now = clock.get();
            clock.set(now + 1);
            now
        };
        let (read, ended) = read_within(vec![1, 2, 3, 4, 5], 3, elapsed, |item| Some(item * 10));
        assert_eq!((read, ended), (vec![10, 20, 30], true));
        // All read in time: not ended. An item with no answer is no result and no stop.
        let (read, ended) = read_within(vec![1, 2, 3], 100, || 0, |item| (item != 2).then_some(item));
        assert_eq!((read, ended), (vec![1, 3], false));
        // A budget of 0 reads nothing, and an empty list is not ended by it.
        assert_eq!(read_within(vec![1], 0, || 0, Some), (vec![], true));
        assert_eq!(read_within(Vec::<i32>::new(), 0, || 0, Some), (vec![], false));
    }

    #[test]
    fn only_a_call_that_changes_nothing_is_not_a_taken_call() {
        let light = DEVICE_LIGHT_ENABLE / 4;
        let shader = DEVICE_SET_PIXEL_SHADER / 4;
        let [setup, restore] = CUTSCENE_SHADER_CALLS;
        // The index of the light does not matter; its state does.
        assert!(nothing_to_do(light, &[7, 0], REMOVE_LIGHT_CALL));
        assert!(!nothing_to_do(light, &[7, 1], REMOVE_LIGHT_CALL));
        assert!(nothing_to_do(shader, &[0], setup));
        assert!(nothing_to_do(shader, &[0], restore));
        assert!(!nothing_to_do(shader, &[0x1234], restore));
        // The same form from another caller: `updateLights` at 0x6BB794, the water code at 0x65D2AB.
        assert!(!nothing_to_do(light, &[7, 0], 0x6BB79A));
        assert!(!nothing_to_do(shader, &[0], 0x65D2B1));
        // The caller of the other method.
        assert!(!nothing_to_do(light, &[7, 0], setup));
        assert!(!nothing_to_do(shader, &[0], REMOVE_LIGHT_CALL));
        // Another method with the same arguments, and a call with too few.
        assert!(!nothing_to_do(DEVICE_SET_LIGHT / 4, &[7, 0], REMOVE_LIGHT_CALL));
        assert!(!nothing_to_do(light, &[7], REMOVE_LIGHT_CALL));
        assert!(!nothing_to_do(shader, &[], setup));
    }

    #[test]
    fn a_locked_rectangle_starts_at_its_left_and_top() {
        // A frame of 1920 x 1080 with 4 bytes for each pixel.
        assert_eq!(lock_offset(0, 0, 1920, 1080, 7680, 4), Some(0));
        assert_eq!(lock_offset(100, 200, 1920, 1080, 7680, 4), Some(200 * 7680 + 400));
        assert_eq!(lock_offset(1919, 1079, 1920, 1080, 7680, 4), Some(1079 * 7680 + 7676));
        assert_eq!(lock_offset(1920, 0, 1920, 1080, 7680, 4), None);
        assert_eq!(lock_offset(0, 1080, 1920, 1080, 7680, 4), None);
        // A block format locks whole.
        assert_eq!(lock_offset(100, 200, 64, 64, 256, 0), Some(0));
    }

    /// A texture of this host of 8 x 8 pixels of 4 bytes with two levels, from the link.
    unsafe fn test_texture() -> *mut Object {
        let lock = crate::sync::device_lock();
        let texture = new_object(&lock, Kind::Texture, &TEXTURE_TABLE);
        (*texture).width = 8;
        (*texture).height = 8;
        (*texture).levels = 2;
        (*texture).format = 21;
        (*texture).from_link = true;
        texture
    }

    /// Locks a level of `texture`, fills it with `value`, and gives the bytes the lock gave
    /// before the fill.
    unsafe fn lock_and_fill(texture: *mut Object, level: u32, value: u8) -> Vec<u8> {
        let mut locked = [0u32; 2];
        assert_eq!(lock_level(texture, level, locked.as_mut_ptr()), OK);
        let block = (&mut (*texture).level_memory)[level as usize].as_mut_slice();
        assert_eq!(block.as_ptr() as u32, locked[1]);
        let before = block.to_vec();
        block.fill(value);
        before
    }

    /// Unlocks a level of `texture`, with a capture file open or not, and gives the bytes that
    /// went into the stream.
    unsafe fn unlock(texture: *mut Object, level: u32, file_open: bool) -> Vec<u8> {
        let mut sent = Vec::new();
        assert_eq!(unlock_level_into(texture, level, file_open, |_, _, _, bytes| sent = bytes.to_vec()), OK);
        sent
    }

    /// Risk 42: an unlock frees the level's block once its bytes are in the stream; the next
    /// lock gives a block of zeros of the level's size, and is counted.
    #[test]
    fn an_unlock_frees_the_level_and_a_lock_after_it_gives_zeros() {
        unsafe {
            let texture = test_texture();
            lock_and_fill(texture, 0, 7);
            lock_and_fill(texture, 1, 9);
            let frees = LEVEL_FREES.levels.load(Ordering::Relaxed);
            assert_eq!(unlock(texture, 0, false), vec![7u8; 8 * 8 * 4], "the stream gets the bytes before the free");
            assert!((&(*texture).level_memory)[0].is_empty() && (&(*texture).level_memory)[0].capacity() == 0);
            assert_eq!((*texture).freed_levels, level_bit(0));
            // The other level is still locked: it keeps its bytes.
            assert_eq!((&(*texture).level_memory)[1], vec![9u8; 4 * 4 * 4]);
            assert!(LEVEL_FREES.levels.load(Ordering::Relaxed) > frees);
            assert_eq!(LevelCopies::of([(*texture).level_memory.as_slice()].into_iter()), LevelCopies { levels: 1, textures: 1, bytes: 64 });

            let locks = LEVEL_FREES.locks_after_free.load(Ordering::Relaxed);
            assert_eq!(lock_and_fill(texture, 0, 3), vec![0u8; 8 * 8 * 4]);
            assert_eq!((*texture).freed_levels, 0);
            assert!(LEVEL_FREES.locks_after_free.load(Ordering::Relaxed) > locks);
            release(texture);
        }
    }

    /// A texture that the game locks again after a free changes: from then on its unlocks keep
    /// the blocks, and each later lock gets the same block with its bytes, with no new one.
    #[test]
    fn a_texture_locked_again_keeps_its_blocks() {
        unsafe {
            let texture = test_texture();
            lock_and_fill(texture, 0, 1);
            unlock(texture, 0, false);
            assert!((&(*texture).level_memory)[0].is_empty() && !(*texture).dynamic, "the first unlock frees");
            lock_and_fill(texture, 0, 2);
            assert!((*texture).dynamic, "the lock of a freed level makes the texture dynamic");
            unlock(texture, 0, false);
            assert_eq!((&(*texture).level_memory)[0], vec![2u8; 8 * 8 * 4], "the second unlock keeps");
            let block = (&(*texture).level_memory)[0].as_ptr();
            assert_eq!(lock_and_fill(texture, 0, 3), vec![2u8; 8 * 8 * 4]);
            assert_eq!((&(*texture).level_memory)[0].as_ptr(), block, "the third lock reuses the block");
            unlock(texture, 0, false);
            // Another level of a dynamic texture keeps its block at its first unlock too.
            lock_and_fill(texture, 1, 4);
            unlock(texture, 1, false);
            assert_eq!((&(*texture).level_memory)[1], vec![4u8; 4 * 4 * 4]);
            release(texture);
        }
    }

    /// While a file is open a re-lock finds no freed level, so the texture stays not dynamic.
    /// After the file closes, its next unlock frees the level, and the lock after that makes the
    /// texture dynamic.
    #[test]
    fn a_texture_becomes_dynamic_only_after_a_free_outside_a_file() {
        unsafe {
            let texture = test_texture();
            lock_and_fill(texture, 0, 1);
            unlock(texture, 0, true);
            lock_and_fill(texture, 0, 2);
            unlock(texture, 0, true);
            assert!(!(*texture).dynamic && !(&(*texture).level_memory)[0].is_empty(), "a file is open: kept, not dynamic");
            lock_and_fill(texture, 0, 3);
            unlock(texture, 0, false);
            assert!(!(*texture).dynamic && (&(*texture).level_memory)[0].is_empty(), "the file closed: the unlock frees");
            lock_and_fill(texture, 0, 4);
            assert!((*texture).dynamic, "the lock after the free makes it dynamic");
            release(texture);
        }
    }

    /// While a capture file is open, an unlock keeps the block: the file writes its changed
    /// textures from it later.
    #[test]
    fn an_unlock_keeps_the_level_while_a_file_is_open() {
        unsafe {
            let texture = test_texture();
            lock_and_fill(texture, 0, 5);
            assert_eq!(unlock(texture, 0, true), vec![5u8; 8 * 8 * 4]);
            assert_eq!((&(*texture).level_memory)[0], vec![5u8; 8 * 8 * 4]);
            assert_eq!((*texture).freed_levels, 0);
            // A lock again gives the kept bytes.
            assert_eq!(lock_and_fill(texture, 0, 6), vec![5u8; 8 * 8 * 4]);
            release(texture);
        }
    }

    /// The snapshot of a file writes a texture with kept bytes, and leaves a texture with a
    /// freed level to its naming, also when another level still holds bytes.
    #[test]
    fn the_snapshot_leaves_a_texture_with_a_freed_level_to_its_naming() {
        unsafe {
            let texture = test_texture();
            assert!(!in_snapshot(&*texture), "a texture never locked");
            lock_and_fill(texture, 0, 1);
            unlock(texture, 0, true);
            assert!(in_snapshot(&*texture), "a texture with kept bytes");
            lock_and_fill(texture, 1, 2);
            unlock(texture, 1, false);
            assert!(!in_snapshot(&*texture), "level 0 still holds bytes, level 1 was freed");
            // A rendered texture is in the snapshot always: its picture comes from the helper.
            (*texture).usage = 1;
            assert!(in_snapshot(&*texture));
            release(texture);
        }
    }

    /// The bit of a level past the 32 bits is none, not a panic.
    #[test]
    fn a_level_has_its_bit() {
        assert_eq!((level_bit(0), level_bit(12), level_bit(31), level_bit(32)), (1, 1 << 12, 1 << 31, 0));
    }

    #[test]
    fn the_level_copies_count_only_the_levels_that_hold_bytes() {
        let texture = vec![vec![0u8; 64], Vec::new(), vec![0u8; 16]];
        let never_locked: Vec<Vec<u8>> = vec![Vec::new(); 3];
        let other = vec![vec![0u8; 4]];
        let objects: [&[Vec<u8>]; 4] = [&texture, &never_locked, &[], &other];
        assert_eq!(LevelCopies::of(objects.into_iter()), LevelCopies { levels: 3, textures: 2, bytes: 84 });
    }
}
