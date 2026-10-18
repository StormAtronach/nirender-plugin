//! A test instrument: every block the engine frees is filled with a pattern first
//! (`NIRENDER_FILL_FREED=1`; off by default). A read of freed memory then fails at the read,
//! with the pattern in the registers, instead of working by chance until the block is used
//! again. It is the proof the lifetime rules ask for (the cell worker audit, 2026-10-18): a
//! crossing glide with the fill on and no crash means no kept pointer was read after its
//! object died, on either thread.
//!
//! The engine's `free` (0x727732) and `operator delete` (0x727530, which calls `free`) both
//! go through one import slot, `msvcrt!free` at 0x746278 (MWSE's `stdafx.h` names the
//! functions; the IDA database the slot). The slot is pointed at `fill_free`, which asks the
//! same CRT for the block's size (`_msize`), writes the pattern, and calls the original.
//! Every thread's frees pass through; the function keeps no state but counters.
//!
//! The pattern is 0xDD: a pointer read from a freed block is 0xDDDDDDDD, outside every mapped
//! page of a 32-bit game, so the next read faults.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

const IMPORT_FREE: usize = 0x746278;
const PATTERN: u8 = 0xDD;
/// A block larger than this is left alone (a wrong size from a foreign allocator).
const LARGEST: usize = 64 << 20;

static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static MSIZE: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

type Free = unsafe extern "C" fn(*mut u8);
type Msize = unsafe extern "C" fn(*mut u8) -> usize;

unsafe extern "C" fn fill_free(block: *mut u8) {
    let original: Free = core::mem::transmute(ORIGINAL.load(Ordering::Relaxed));
    if !block.is_null() {
        let msize: Msize = core::mem::transmute(MSIZE.load(Ordering::Relaxed));
        let size = msize(block);
        if size > 0 && size <= LARGEST {
            core::ptr::write_bytes(block, PATTERN, size);
            FREES.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(size as u64, Ordering::Relaxed);
        }
    }
    original(block);
}

pub unsafe fn install() {
    if !std::env::var("NIRENDER_FILL_FREED").is_ok_and(|v| v == "1") {
        return;
    }
    let name: Vec<u16> = "msvcrt.dll\0".encode_utf16().collect();
    let module = crate::win::GetModuleHandleW(name.as_ptr());
    let msize = if module.is_null() { core::ptr::null_mut() } else { crate::win::GetProcAddress(module, b"_msize\0".as_ptr()) };
    if msize.is_null() {
        crate::log!("freed fill: msvcrt.dll has no _msize; the fill is off");
        return;
    }
    let original: usize = crate::mem::rd(IMPORT_FREE, 0);
    if original == 0 || original == fill_free as *const () as usize {
        return;
    }
    MSIZE.store(msize as usize, Ordering::Relaxed);
    ORIGINAL.store(original, Ordering::Relaxed);
    if !crate::win::patch(IMPORT_FREE, &(fill_free as *const () as u32).to_le_bytes()) {
        crate::log!("freed fill: the import slot of free could not be written; the fill is off");
        return;
    }
    crate::log!("freed fill: every block the engine frees is filled with {PATTERN:#04x} first (NIRENDER_FILL_FREED=1); a read of freed memory faults at the read");
}

pub fn report() -> String {
    if ORIGINAL.load(Ordering::Relaxed) == 0 {
        return String::from("freed fill: off");
    }
    format!("freed fill: {} blocks filled, {:.1} MB in all", FREES.load(Ordering::Relaxed), BYTES.load(Ordering::Relaxed) as f64 / 1048576.0)
}
