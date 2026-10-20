//! The lock of the device, and the census of the calls from other threads.
//!
//! The game uses more than one thread for its renderer. A second thread loads the cells
//! around the player and makes their textures (`game_threadLoadCells` 0x48FC90), and the
//! loading screen has a thread too (`LoadScreenManager::startMovie` 0x4588CB). The game asks
//! Direct3D for a device that takes calls from more than one thread
//! (D3DCREATE_MULTITHREADED): Direct3D then does one call at a time, of the device and of
//! each resource.
//!
//! The plugin gives the game the same. Each call that comes into the plugin from the game
//! holds `device_lock`: a call of the device, a method of a resource of `own_host`, a
//! function of the host table, the recorder on a buffer, and each write into the stream. A
//! thread can take the lock again when it has it.
//!
//! A port runs its Rust code only on the main thread (`shadow::dispatch`). On another thread
//! the game code runs, and it calls the device and the resources. The census has a count for
//! each kind of call that came from another thread, and the log gets a line at the first one.

use std::collections::BTreeMap;
use std::sync::{Mutex, Once};

#[repr(C)]
struct CriticalSection([usize; 6]);

extern "system" {
    fn InitializeCriticalSection(section: *mut CriticalSection);
    fn EnterCriticalSection(section: *mut CriticalSection);
    fn LeaveCriticalSection(section: *mut CriticalSection);
}

static mut SECTION: CriticalSection = CriticalSection([0; 6]);
static SECTION_MADE: Once = Once::new();

/// The lock is free again when this value goes away.
pub struct DeviceLock;

pub fn device_lock() -> DeviceLock {
    unsafe {
        SECTION_MADE.call_once(|| InitializeCriticalSection(core::ptr::addr_of_mut!(SECTION)));
        EnterCriticalSection(core::ptr::addr_of_mut!(SECTION));
    }
    DeviceLock
}

impl Drop for DeviceLock {
    fn drop(&mut self) {
        unsafe { LeaveCriticalSection(core::ptr::addr_of_mut!(SECTION)) };
    }
}

/// True when this thread holds the device lock now (`CRITICAL_SECTION::OwningThread`, the
/// fourth word, is the owner's thread id; the section is made at its first use).
pub fn device_lock_held_here() -> bool {
    if !SECTION_MADE.is_completed() {
        return false;
    }
    let owner = unsafe { (*core::ptr::addr_of!(SECTION)).0[3] };
    owner != 0 && owner == unsafe { crate::win::GetCurrentThreadId() } as usize
}

/// An engine critical section held: the lock is free again when this value goes away.
pub struct EngineLock(*mut CriticalSection);

/// Holds the engine's critical section at `section` (a `CRITICAL_SECTION` inside an engine
/// object, initialized by the engine) while the value lives. The engine's own code takes the
/// same lock around the structure the plugin reads (the light manager's list and map,
/// `NiDX8LightManager+0x50`), so a walk under it cannot see a half-changed structure (the
/// audit of the cell worker, Tier 1 finding 6).
pub unsafe fn engine_lock(section: usize) -> EngineLock {
    let section = section as *mut CriticalSection;
    EnterCriticalSection(section);
    EngineLock(section)
}

impl Drop for EngineLock {
    fn drop(&mut self) {
        unsafe { LeaveCriticalSection(self.0) };
    }
}

static CENSUS: Mutex<BTreeMap<(&'static str, &'static str), u32>> = Mutex::new(BTreeMap::new());

/// Counts a call when it is not on the main thread. `group` is the kind of the entry
/// ("device", "resource", "port") and `name` is the method or the function.
#[inline]
pub fn off_thread(group: &'static str, name: &'static str) {
    if crate::shadow::on_main_thread() {
        return;
    }
    let first = match CENSUS.lock() {
        Ok(mut census) => {
            let count = census.entry((group, name)).or_insert(0);
            *count += 1;
            *count == 1
        }
        Err(_) => false,
    };
    if first {
        crate::log!("threads: the first call of {group} {name} from another thread (thread {})", unsafe { crate::win::GetCurrentThreadId() });
    }
}

/// The census as text: one line for each kind of call, with its count.
pub fn census_text() -> String {
    let Ok(census) = CENSUS.lock() else { return String::new() };
    if census.is_empty() {
        return "no call from another thread".to_string();
    }
    census.iter().map(|((group, name), count)| format!("{group} {name} {count}")).collect::<Vec<_>>().join("\n")
}
