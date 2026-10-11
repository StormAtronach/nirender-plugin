//! The plugin as the host of its own render link.
//!
//! The host of the render link is MGE XE at first: each call of the link ends on its D3D9
//! device. After `take_over` the plugin answers the calls itself. Then the game makes no
//! call that gets to a D3D device: the commands go only into the stream of `capture`, and a
//! renderer in another process draws them.
//!
//! The game holds pointers to its buffers, textures and surfaces and calls their methods.
//! Other code in the process does that too, for example the skinning code of MGE XE. So each
//! resource that this host makes is an object with a table of methods in the layout of
//! Direct3D 8, and with memory of its own for a lock. A call of the link on a resource goes
//! through the table of the object: an object that MGE XE made before the change works too.
//!
//! What this host does not do: a pixel read gives the bytes that the game wrote or zeros,
//! and a copy between surfaces does nothing.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

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
        0x3254_5844..=0x3554_5844 => 16,
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

unsafe fn new_object(kind: Kind, table: &'static Table) -> *mut Object {
    Box::into_raw(Box::new(Object {
        table: table.methods(),
        references: 1,
        kind,
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
    }))
}

// ---- the methods

unsafe extern "system" fn query_interface(_this: *mut Object, _id: usize, out: *mut usize) -> u32 {
    if !out.is_null() {
        *out = 0;
    }
    NO_INTERFACE
}

unsafe extern "system" fn add_reference(this: *mut Object) -> u32 {
    (*this).references += 1;
    (*this).references
}

unsafe extern "system" fn release(this: *mut Object) -> u32 {
    (*this).references -= 1;
    let left = (*this).references;
    if left == 0 {
        let owner = (*this).owner;
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
    if level >= (*this).levels || out.is_null() {
        return FAIL;
    }
    let surface = new_object(Kind::LevelSurface, &SURFACE_TABLE);
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
    lock_level(this, level, locked)
}

unsafe extern "system" fn texture_unlock(this: *mut Object, level: u32) -> u32 {
    unlock_level(this, level)
}

unsafe extern "system" fn surface_description(this: *mut Object, description: *mut u32) -> u32 {
    write_surface_description(&*this, 0, description);
    OK
}

unsafe extern "system" fn surface_lock(this: *mut Object, locked: *mut u32, _rectangle: usize, _flags: u32) -> u32 {
    let object = &mut *this;
    if object.kind == Kind::LevelSurface {
        return lock_level(object.owner, object.level, locked);
    }
    if locked.is_null() {
        return FAIL;
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

/// Calls method `index` of an object through its table. The object can be of this host or
/// of Direct3D.
unsafe fn method(object: u32, index: usize) -> usize {
    let table: usize = *(object as usize as *const usize);
    *((table + index * 4) as *const usize)
}

// ---- the functions of the host table

unsafe extern "C" fn scene_begin(_scene: *const SceneV1) {}
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

unsafe extern "C" fn camera_space(_origin: *mut f64) -> u32 {
    0
}

unsafe extern "C" fn texture_create(texture: *const TextureV1) -> u32 {
    let texture = &*texture;
    let object = new_object(Kind::Texture, &TEXTURE_TABLE);
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
    let mut surface = slot.load(Ordering::Relaxed);
    if surface == 0 {
        let object = new_object(Kind::Surface, &SURFACE_TABLE);
        let renderer = crate::engine::renderer();
        let size: [u32; 2] = if renderer != 0 { crate::mem::rd(renderer, 0x23C) } else { [640, 480] };
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
    let a = |i: usize| *arguments.add(i);
    match index {
        CALL_CREATE_VERTEX_BUFFER | CALL_CREATE_INDEX_BUFFER => {
            // Length, usage, vertex format or index format, pool, address for the handle.
            let vertex = index == CALL_CREATE_VERTEX_BUFFER;
            let object = new_object(if vertex { Kind::VertexBuffer } else { Kind::IndexBuffer }, &BUFFER_TABLE);
            (*object).length = a(0);
            (*object).usage = a(1);
            (*object).format = a(2);
            (*object).pool = a(3);
            (*object).memory = vec![0; a(0) as usize];
            put(a(4), object as u32)
        }
        CALL_CREATE_TEXTURE => {
            // Width, height, levels, usage, format, pool, address for the handle.
            let object = new_object(Kind::Texture, &TEXTURE_TABLE);
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
            let object = new_object(Kind::Surface, &SURFACE_TABLE);
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
        // The frame, the scene, the target, a copy of pixels: nothing to do here. The stream
        // has the call.
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
static mut TAKEN: [u32; 100] = [0; 100];

/// A device call of other code that has no call of the link. It must not get to the device
/// of MGE XE: it can have an object of this host as an argument.
pub unsafe fn take(slot: usize) {
    if let Some(count) = (*core::ptr::addr_of_mut!(TAKEN)).get_mut(slot) {
        *count += 1;
    }
}

/// The plugin is the host of the render link from now on.
pub unsafe fn take_over() {
    crate::link_client::set_host(&TABLE);
    ACTIVE.store(true, Ordering::Relaxed);
    crate::log!("own host: the plugin answers the calls of the render link from now on");
}

/// One line with the device methods that this host took, for the log.
pub unsafe fn report() -> String {
    let taken = &*core::ptr::addr_of!(TAKEN);
    let mut text = String::new();
    for (slot, count) in taken.iter().enumerate() {
        if *count != 0 {
            text.push_str(&format!(" {}:{}", crate::device::NAMES.get(slot).copied().unwrap_or("?"), count));
        }
    }
    if text.is_empty() {
        text.push_str(" none");
    }
    format!("own host: device calls without a call of the link, taken and not done:{text}")
}
