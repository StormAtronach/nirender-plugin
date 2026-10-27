//! The numbers of the game process for the debug overlay of the host program: the memory
//! in use and the address space. The game is a 32-bit process, so the address space and
//! its largest free block are what a player runs out of. See `capture::tag::OVERLAY`.

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);

/// True when the overlay shows.
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Turns the overlay on or off; without a value, to the other state. Returns the state.
pub fn set(value: Option<bool>) -> bool {
    let now = value.unwrap_or(!on());
    ON.store(now, Ordering::Relaxed);
    now
}

#[repr(C)]
#[derive(Default)]
struct ProcessMemory {
    size: u32,
    page_faults: u32,
    peak_working_set: usize,
    working_set: usize,
    quota: [usize; 4],
    page_file: usize,
    peak_page_file: usize,
    private: usize,
}

#[repr(C)]
#[derive(Default)]
struct MemoryStatus {
    length: u32,
    load: u32,
    total_physical: u64,
    available_physical: u64,
    total_page_file: u64,
    available_page_file: u64,
    total_virtual: u64,
    available_virtual: u64,
    available_extended: u64,
}

#[repr(C)]
struct Region {
    base: usize,
    allocation_base: usize,
    allocation_protect: u32,
    size: usize,
    state: u32,
    protect: u32,
    kind: u32,
}

const MEM_FREE: u32 = 0x10000;

extern "system" {
    fn K32GetProcessMemoryInfo(process: *mut c_void, counters: *mut ProcessMemory, size: u32) -> i32;
    fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
    fn VirtualQuery(address: *const c_void, region: *mut Region, length: usize) -> usize;
}

/// The largest free block of the address space of the process.
unsafe fn largest_free(total: u64) -> u64 {
    let mut largest = 0usize;
    let mut address = 0x1_0000usize;
    let mut region: Region = core::mem::zeroed();
    while (address as u64) < total && VirtualQuery(address as *const c_void, &mut region, core::mem::size_of::<Region>()) != 0 {
        if region.state == MEM_FREE {
            largest = largest.max(region.size);
        }
        match region.base.checked_add(region.size) {
            Some(next) if next > address => address = next,
            _ => break,
        }
    }
    largest as u64
}

/// The body of an `OVERLAY` record: the state, then the numbers of the process, then the
/// time of the game thread for each frame in ms (in the stream, in the ports of the
/// renderer, in waits for the host program).
///
/// # Safety
///
/// No condition: the function reads no engine memory. It is `unsafe` because of the calls of the
/// Windows functions.
pub unsafe fn body(stream_ms: f32, ports_ms: f32, wait_ms: f32) -> [u8; 64] {
    let mut memory = ProcessMemory { size: core::mem::size_of::<ProcessMemory>() as u32, ..Default::default() };
    K32GetProcessMemoryInfo(crate::win::GetCurrentProcess(), &mut memory, memory.size);
    let mut status = MemoryStatus { length: core::mem::size_of::<MemoryStatus>() as u32, ..Default::default() };
    GlobalMemoryStatusEx(&mut status);
    let mut out = [0u8; 64];
    out[0..4].copy_from_slice(&(on() as u32).to_le_bytes());
    out[8..16].copy_from_slice(&(memory.working_set as u64).to_le_bytes());
    out[16..24].copy_from_slice(&(memory.private as u64).to_le_bytes());
    out[24..32].copy_from_slice(&(status.total_virtual - status.available_virtual).to_le_bytes());
    out[32..40].copy_from_slice(&status.total_virtual.to_le_bytes());
    out[40..48].copy_from_slice(&largest_free(status.total_virtual).to_le_bytes());
    out[48..52].copy_from_slice(&stream_ms.to_le_bytes());
    out[52..56].copy_from_slice(&ports_ms.to_le_bytes());
    out[56..60].copy_from_slice(&wait_ms.to_le_bytes());
    out
}
