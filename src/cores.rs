//! The scheduling of the game on a hybrid processor: its main thread goes to the cores of
//! the highest efficiency class. In some runs the scheduler gave the thread an efficiency
//! core and the ports took twice their time. A processor with one class of cores gets no
//! change. The priority of the process stays as the user or Windows set it.
//! `MGE_XE_CORES=0` in the environment leaves the scheduler alone. The helper program does
//! the same for itself (`cores` in the renderer crate).

#[repr(C)]
struct CpuSetInformation {
    size: u32,
    kind: u32,
    id: u32,
    group: u16,
    logical_processor_index: u8,
    core_index: u8,
    last_level_cache_index: u8,
    numa_node_index: u8,
    efficiency_class: u8,
    all_flags: u8,
    reserved: u32,
    allocation_tag: u64,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentThread() -> *mut core::ffi::c_void;
    fn GetSystemCpuSetInformation(information: *mut CpuSetInformation, length: u32, returned: *mut u32, process: *mut core::ffi::c_void, flags: u32) -> i32;
    fn SetThreadSelectedCpuSets(thread: *mut core::ffi::c_void, ids: *const u32, count: u32) -> i32;
}

/// Returns one line for the log with what was done. Called on the main thread of the game.
pub fn prefer_performance_cores() -> String {
    if std::env::var("MGE_XE_CORES").is_ok_and(|v| v == "0") {
        return "cores: left to the scheduler (MGE_XE_CORES=0)".into();
    }
    // SAFETY: calls of the Windows API with valid arguments; the buffer is sized by the
    // first call.
    unsafe {
        let process = crate::win::GetCurrentProcess();
        let mut needed = 0u32;
        GetSystemCpuSetInformation(core::ptr::null_mut(), 0, &mut needed, process, 0);
        let count = needed as usize / core::mem::size_of::<CpuSetInformation>();
        let mut sets: Vec<CpuSetInformation> = Vec::with_capacity(count.max(1));
        if GetSystemCpuSetInformation(sets.as_mut_ptr(), needed, &mut needed, process, 0) != 0 {
            sets.set_len(needed as usize / core::mem::size_of::<CpuSetInformation>());
        }
        let best = sets.iter().map(|s| s.efficiency_class).max().unwrap_or(0);
        let classes = sets.iter().map(|s| s.efficiency_class).min().unwrap_or(0) != best;
        let ids: Vec<u32> = sets.iter().filter(|s| s.efficiency_class == best).map(|s| s.id).collect();
        let pinned = classes && !ids.is_empty() && SetThreadSelectedCpuSets(GetCurrentThread(), ids.as_ptr(), ids.len() as u32) != 0;
        format!(
            "cores: {} of {} logical processors in the highest efficiency class{}",
            ids.len(),
            sets.len(),
            if pinned { ", the main thread on them" } else if classes { ", not pinned" } else { ", one class, not pinned" }
        )
    }
}
