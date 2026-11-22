//! The numbers of the game process for the debug overlay of the host program: the memory
//! in use and the address space. The game is a 32-bit process, so the address space and
//! its largest free block are what a player runs out of. See `capture::tag::OVERLAY`.

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicI64, Ordering};

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

/// The largest free block of the address space of the process now, for a log line (risk 41).
/// A walk of the address space: for a mark, not for each frame.
pub fn largest_free_now() -> u64 {
    let mut status = MemoryStatus {
        length: core::mem::size_of::<MemoryStatus>() as u32,
        ..Default::default()
    };
    // SAFETY: the calls write into the own structures; the walk reads no memory of the process.
    unsafe {
        GlobalMemoryStatusEx(&mut status);
        largest_free(status.total_virtual)
    }
}

/// The seconds between two memory lines of the log ([`memory_tick`]).
pub const MEMORY_LINE_SECONDS: i64 = 60;

/// The ticks of the performance counter at the first memory line and at the last one; 0
/// before the first.
static MEMORY_FIRST: AtomicI64 = AtomicI64::new(0);
static MEMORY_LAST: AtomicI64 = AtomicI64::new(0);

/// At the end of a recorded frame (`capture`): a memory line in the log at the first frame,
/// then each [`MEMORY_LINE_SECONDS`]. Risk 41: the address space of the game fragments over
/// long play, and the marks of the stream come only from a script. `now` and `per_second`:
/// the ticks of the performance counter. The walk of the address space and the line run on a
/// short thread of their own: the walk took 6.4 to 6.9 ms after 2 minutes of play (verdict 632,
/// the run 20261123-034741-hob-w64-live), a dropped frame each minute on the game thread under
/// the device lock. Without a thread, no line.
pub fn memory_tick(now: i64, per_second: i64) {
    let last = MEMORY_LAST.load(Ordering::Relaxed);
    if !memory_line_due(now, last, per_second) {
        return;
    }
    if last == 0 {
        MEMORY_FIRST.store(now, Ordering::Relaxed);
    }
    MEMORY_LAST.store(now, Ordering::Relaxed);
    let seconds = (now - MEMORY_FIRST.load(Ordering::Relaxed)) / per_second.max(1);
    let _ = std::thread::Builder::new()
        .name("nirender memory line".into())
        .spawn(move || crate::log!("{}", memory_line(seconds, &memory_now())));
}

/// True when a memory line is due: at the first frame (`last` 0), then when
/// [`MEMORY_LINE_SECONDS`] have passed since the last line.
fn memory_line_due(now: i64, last: i64, per_second: i64) -> bool {
    last == 0 || now - last >= MEMORY_LINE_SECONDS * per_second
}

/// The numbers of a memory line, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct MemoryNow {
    largest_free: u64,
    /// The address space in use, and all of it.
    used: u64,
    total: u64,
    /// The private bytes of the process.
    private: u64,
    /// The time of the walk for `largest_free`, in microseconds, on the thread of the line
    /// ([`memory_tick`]).
    walk_us: u64,
}

/// The numbers of the process now.
fn memory_now() -> MemoryNow {
    let mut memory = ProcessMemory {
        size: core::mem::size_of::<ProcessMemory>() as u32,
        ..Default::default()
    };
    let mut status = MemoryStatus {
        length: core::mem::size_of::<MemoryStatus>() as u32,
        ..Default::default()
    };
    // SAFETY: the calls write into the own structures; the walk reads no memory of the process.
    unsafe {
        K32GetProcessMemoryInfo(crate::win::GetCurrentProcess(), &mut memory, memory.size);
        GlobalMemoryStatusEx(&mut status);
        let walk = std::time::Instant::now();
        let largest_free = largest_free(status.total_virtual);
        MemoryNow {
            largest_free,
            used: status.total_virtual - status.available_virtual,
            total: status.total_virtual,
            private: memory.private as u64,
            walk_us: walk.elapsed().as_micros() as u64,
        }
    }
}

/// The memory line: `seconds` since the first one. MB are 10^6 bytes, as in the mark line.
fn memory_line(seconds: i64, memory: &MemoryNow) -> String {
    let mb = |bytes: u64| bytes as f64 / 1e6;
    format!(
        "memory at {seconds} s: largest free block {:.1} MB; address space {:.1} MB of {:.1} MB in use; private {:.1} MB; walk {:.1} ms",
        mb(memory.largest_free),
        mb(memory.used),
        mb(memory.total),
        mb(memory.private),
        memory.walk_us as f64 / 1000.0
    )
}

/// The largest free block of the address space of the process.
unsafe fn largest_free(total: u64) -> u64 {
    let mut largest = 0usize;
    let mut address = 0x1_0000usize;
    let mut region: Region = core::mem::zeroed();
    while (address as u64) < total
        && VirtualQuery(address as *const c_void, &mut region, core::mem::size_of::<Region>()) != 0
    {
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
    let mut memory = ProcessMemory {
        size: core::mem::size_of::<ProcessMemory>() as u32,
        ..Default::default()
    };
    K32GetProcessMemoryInfo(crate::win::GetCurrentProcess(), &mut memory, memory.size);
    let mut status = MemoryStatus {
        length: core::mem::size_of::<MemoryStatus>() as u32,
        ..Default::default()
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_memory_line_comes_at_the_first_frame_then_each_minute() {
        let per_second = 10_000_000;
        assert!(memory_line_due(5, 0, per_second), "the first frame");
        let last = 1_000;
        assert!(!memory_line_due(last + 59 * per_second, last, per_second));
        assert!(memory_line_due(last + 60 * per_second, last, per_second));
        assert!(memory_line_due(last + 600 * per_second, last, per_second));
    }

    #[test]
    fn the_memory_line_has_the_largest_free_block_and_the_address_space() {
        let memory = MemoryNow {
            largest_free: 2_131_200_000,
            used: 1_830_400_000,
            total: 4_294_836_224,
            private: 1_500_300_000,
            walk_us: 1_240,
        };
        assert_eq!(
            memory_line(120, &memory),
            "memory at 120 s: largest free block 2131.2 MB; address space 1830.4 MB of 4294.8 MB in use; private 1500.3 MB; walk 1.2 ms"
        );
        // The walk of this process: a free block, and no more than the address space.
        let now = memory_now();
        assert!(now.largest_free > 0 && now.largest_free <= now.total, "{now:?}");
        assert!(now.used > 0 && now.used <= now.total, "{now:?}");
        assert!(now.walk_us < 5_000_000, "{now:?}");
    }

    /// The frame's call only starts the thread; the thread writes the line into the log. The
    /// only test of `memory_tick`: its state is the program's.
    #[test]
    fn the_memory_line_comes_from_its_own_thread() {
        let per_second = 10_000_000;
        // The first line, at 0 s; a call before the minute writes none.
        memory_tick(1, per_second);
        memory_tick(2, per_second);
        let log = std::env::temp_dir().join("nirender-test-log").join("nirender.log");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let lines = loop {
            let text = std::fs::read_to_string(&log).unwrap_or_default();
            let lines = text.lines().filter(|line| line.starts_with("memory at ")).count();
            if lines > 0 || std::time::Instant::now() >= deadline {
                break lines;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        assert_eq!(lines, 1, "the memory lines in {}", log.display());
    }
}
