//! Heap allocations inside a shadow comparison.
//!
//! A function that allocates gets a new address in each of its two runs. Thus every pointer
//! that it stores is different. So the engine run gets the blocks that the Rust run
//! allocated, in the same order. `operator new` in the engine run returns the recorded block
//! and does not allocate a new one. The code fills a block with a pattern before each run.
//! Thus the bytes that neither run writes are equal. The bytes that only one run writes are
//! not equal.
//!
//! The code records a free in the Rust run and does not do it. The engine run does the real
//! free.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::hook::{self, Cleanup};

/// `operator new(size_t)` is a jump through the import table.
const OPERATOR_NEW: usize = 0x727692;
const OPERATOR_NEW_HEAD: [u8; 6] = [0xFF, 0x25, 0x58, 0x62, 0x74, 0x00];
/// `operator delete(void*)` is `push [esp+4]; call free; pop ecx; ret`.
const OPERATOR_DELETE: usize = 0x727530;
const OPERATOR_DELETE_HEAD: [u8; 9] = [0xFF, 0x74, 0x24, 0x04, 0xE8, 0xF9, 0x01, 0x00, 0x00];

const FILL: u8 = 0xCD;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Leg {
    None,
    Rust,
    Engine,
}

static ACTIVE: AtomicBool = AtomicBool::new(false);
static ORIG_NEW: AtomicUsize = AtomicUsize::new(0);
static ORIG_DELETE: AtomicUsize = AtomicUsize::new(0);
/// The thread that the comparisons run on. Other threads allocate during a comparison. Their
/// allocations are not part of it.
static THREAD: AtomicUsize = AtomicUsize::new(0);

unsafe fn leg() -> Leg {
    let s = state();
    if s.leg != Leg::None && crate::win::GetCurrentThreadId() as usize != THREAD.load(Ordering::Relaxed) {
        return Leg::None;
    }
    s.leg
}

struct State {
    leg: Leg,
    /// The address and the size of each block that the Rust run allocated.
    blocks: Vec<(usize, usize)>,
    /// The number of these blocks that the engine run took.
    taken: usize,
    freed_rust: Vec<usize>,
    freed_engine: Vec<usize>,
    /// The engine run asked for a block that the Rust run did not allocate, or for another
    /// size.
    mismatch: bool,
}

static mut STATE: State =
    State { leg: Leg::None, blocks: Vec::new(), taken: 0, freed_rust: Vec::new(), freed_engine: Vec::new(), mismatch: false };

unsafe fn state() -> &'static mut State {
    &mut *core::ptr::addr_of_mut!(STATE)
}

/// True when the allocator hooks are in place. Then the shadow test can compare a call that
/// allocates.
pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

unsafe fn real_new(size: usize) -> usize {
    core::mem::transmute::<usize, unsafe extern "C" fn(usize) -> usize>(ORIG_NEW.load(Ordering::Relaxed))(size)
}

unsafe fn real_delete(ptr: usize) {
    core::mem::transmute::<usize, unsafe extern "C" fn(usize)>(ORIG_DELETE.load(Ordering::Relaxed))(ptr)
}

unsafe extern "C" fn new_detour(size: usize) -> usize {
    let s = state();
    match leg() {
        Leg::None => real_new(size),
        Leg::Rust => {
            let ptr = real_new(size);
            if ptr != 0 {
                core::ptr::write_bytes(ptr as *mut u8, FILL, size);
                s.blocks.push((ptr, size));
            }
            ptr
        }
        Leg::Engine => match s.blocks.get(s.taken) {
            Some(&(ptr, block_size)) if block_size == size => {
                s.taken += 1;
                ptr
            }
            _ => {
                s.mismatch = true;
                real_new(size)
            }
        },
    }
}

unsafe extern "C" fn delete_detour(ptr: usize) {
    let s = state();
    match leg() {
        Leg::None => real_delete(ptr),
        Leg::Rust => s.freed_rust.push(ptr),
        Leg::Engine => {
            s.freed_engine.push(ptr);
            real_delete(ptr)
        }
    }
}

/// Puts the hooks on the allocator of the game. Without them `active()` stays false.
pub unsafe fn install() -> Result<(), String> {
    if active() {
        return Ok(());
    }
    let new = hook::install_raw(OPERATOR_NEW, &OPERATOR_NEW_HEAD, &[], new_detour as *const () as usize, 1, Cleanup::Caller)
        .map_err(|e| format!("operator new: {e}"))?;
    ORIG_NEW.store(new, Ordering::Relaxed);
    let delete = hook::install_raw(OPERATOR_DELETE, &OPERATOR_DELETE_HEAD, &[5], delete_detour as *const () as usize, 1, Cleanup::Caller)
        .map_err(|e| format!("operator delete: {e}"))?;
    ORIG_DELETE.store(delete, Ordering::Relaxed);
    THREAD.store(crate::win::GetCurrentThreadId() as usize, Ordering::Relaxed);
    ACTIVE.store(true, Ordering::Relaxed);
    Ok(())
}

/// Start of the Rust run of a comparison.
pub unsafe fn begin_rust() {
    let s = state();
    s.blocks.clear();
    s.freed_rust.clear();
    s.freed_engine.clear();
    s.taken = 0;
    s.mismatch = false;
    s.leg = Leg::Rust;
}

/// The code calls this between the runs. It copies the content that the Rust run left in
/// its blocks into `saved`. Then it fills the blocks again for the engine run.
pub unsafe fn begin_engine(saved: &mut Vec<u8>) {
    let s = state();
    saved.clear();
    for &(ptr, size) in &s.blocks {
        saved.extend_from_slice(core::slice::from_raw_parts(ptr as *const u8, size));
        core::ptr::write_bytes(ptr as *mut u8, FILL, size);
    }
    s.leg = Leg::Engine;
}

/// End of the engine run. Returns the differences as text. The text is empty when the two
/// runs allocated, wrote and freed the same.
pub unsafe fn end(saved: &[u8]) -> String {
    let s = state();
    s.leg = Leg::None;
    let mut text = String::new();
    if s.mismatch {
        text.push_str(" the engine allocated a block the rust run did not;");
    }
    if s.taken < s.blocks.len() {
        text.push_str(&format!(" rust allocated {} blocks, the engine {};", s.blocks.len(), s.taken));
    }
    let mut at = 0;
    for (index, &(ptr, size)) in s.blocks.iter().enumerate() {
        let rust = &saved[at..at + size];
        at += size;
        if index >= s.taken {
            // The engine run did not take the block. The engine run did not free it.
            real_delete(ptr);
            continue;
        }
        if s.freed_engine.contains(&ptr) {
            continue; // The same call freed the block again, so the content is gone.
        }
        let engine = core::slice::from_raw_parts(ptr as *const u8, size);
        if let Some(offset) = (0..size).find(|&i| rust[i] != engine[i]) {
            let end = (offset + 8).min(size);
            text.push_str(&format!(
                " new block {} ({} bytes) +{:#x}: rust {:02x?} engine {:02x?};",
                index,
                size,
                offset,
                &rust[offset..end],
                &engine[offset..end]
            ));
        }
    }
    if s.freed_rust != s.freed_engine {
        text.push_str(&format!(" rust freed {} blocks, the engine {} (or in another order);", s.freed_rust.len(), s.freed_engine.len()));
    }
    text
}
