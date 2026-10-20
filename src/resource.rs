//! The recorder on vertex and index buffers. It records the data that the game writes into a
//! locked buffer.
//!
//! The device recorder finds a buffer when the game creates or binds it. It replaces the
//! vtable pointer of the buffer, as `device::attach` does for the device. Then `Unlock` puts
//! the locked bytes into the device log as a separate record. Thus two runs must fill a
//! buffer with the same data.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::device;

/// Log slots past the device methods.
pub const SLOT_BUFFER_UNLOCK: usize = 100;

// IDirect3DVertexBuffer8 and IDirect3DIndexBuffer8 have the same vtable layout.
const LOCK: usize = 11;
const UNLOCK: usize = 12;
const GET_DESC: usize = 13;
const TABLE_LEN: usize = 32;
/// Buffers larger than this go into the log as a hash.
const MAX_PAYLOAD: usize = 1 << 20;

static ENABLED: AtomicBool = AtomicBool::new(false);

struct Class {
    original: usize,
    /// The array has one word before the table for the type information pointer, as in
    /// `device`.
    table: Box<[usize; TABLE_LEN + 1]>,
}

struct Locked {
    object: usize,
    data: usize,
    offset: u32,
    size: u32,
    flags: u32,
}

struct State {
    classes: Vec<Class>,
    locked: Vec<Locked>,
}

static mut STATE: State = State { classes: Vec::new(), locked: Vec::new() };

unsafe fn state() -> &'static mut State {
    &mut *core::ptr::addr_of_mut!(STATE)
}

pub fn enable(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

unsafe fn class_of(object: usize) -> Option<&'static Class> {
    let vtable = *(object as *const usize);
    state().classes.iter().find(|c| c.table.as_ptr().add(1) as usize == vtable)
}

/// Puts the recorder on a vertex or index buffer. It does nothing for a buffer that has the
/// recorder.
pub unsafe fn wrap(object: usize) {
    if object == 0 || !ENABLED.load(Ordering::Relaxed) || class_of(object).is_some() {
        return;
    }
    let vtable = *(object as *const usize);
    let s = state();
    let index = match s.classes.iter().position(|c| c.original == vtable) {
        Some(index) => index,
        None => {
            let mut table = Box::new([0usize; TABLE_LEN + 1]);
            core::ptr::copy_nonoverlapping((vtable - 4) as *const usize, table.as_mut_ptr(), TABLE_LEN + 1);
            table[1 + LOCK] = lock_wrapper as *const () as usize;
            table[1 + UNLOCK] = unlock_wrapper as *const () as usize;
            s.classes.push(Class { original: vtable, table });
            s.classes.len() - 1
        }
    };
    *(object as *mut usize) = s.classes[index].table.as_ptr().add(1) as usize;
}

unsafe fn original(object: usize, slot: usize) -> usize {
    // A wrapped object always has a class.
    let class = class_of(object).unwrap_unchecked();
    *((class.original + slot * 4) as *const usize)
}

unsafe fn buffer_size(object: usize) -> u32 {
    // D3DVERTEXBUFFER_DESC and D3DINDEXBUFFER_DESC: Format, Type, Usage, Pool, Size, (FVF)
    let mut desc = [0u32; 6];
    let get_desc: unsafe extern "system" fn(usize, *mut u32) -> u32 = core::mem::transmute(original(object, GET_DESC));
    get_desc(object, desc.as_mut_ptr());
    desc[4]
}

unsafe extern "system" fn lock_wrapper(this: usize, offset: u32, size: u32, data: *mut usize, flags: u32) -> u32 {
    let _lock = crate::sync::device_lock();
    crate::sync::off_thread("resource", "buffer Lock");
    let lock: unsafe extern "system" fn(usize, u32, u32, *mut usize, u32) -> u32 = core::mem::transmute(original(this, LOCK));
    let result = lock(this, offset, size, data, flags);
    if (result as i32) >= 0 && (device::recording() || crate::capture::active()) {
        // Size 0 locks the whole buffer.
        let size = if size == 0 { buffer_size(this).saturating_sub(offset) } else { size };
        let s = state();
        s.locked.retain(|l| l.object != this);
        s.locked.push(Locked { object: this, data: *data, offset, size, flags });
    }
    result
}

unsafe extern "system" fn unlock_wrapper(this: usize) -> u32 {
    let _lock = crate::sync::device_lock();
    crate::sync::off_thread("resource", "buffer Unlock");
    let s = state();
    if let Some(index) = s.locked.iter().position(|l| l.object == this) {
        let locked = s.locked.swap_remove(index);
        #[cfg(feature = "rig")]
        if device::recording() && locked.data != 0 {
            let bytes = core::slice::from_raw_parts(locked.data as *const u8, locked.size as usize);
            let args = [this as u32, locked.offset, locked.size, locked.flags];
            if bytes.len() <= MAX_PAYLOAD {
                device::log_extra(SLOT_BUFFER_UNLOCK, &args, bytes);
            } else {
                device::log_extra(SLOT_BUFFER_UNLOCK, &args, &crate::mem::fnv1a(bytes).to_le_bytes());
            }
        }
    }
    let unlock: unsafe extern "system" fn(usize) -> u32 = core::mem::transmute(original(this, UNLOCK));
    unlock(this)
}
