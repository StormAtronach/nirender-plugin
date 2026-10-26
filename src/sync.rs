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
    fn TryEnterCriticalSection(section: *mut CriticalSection) -> i32;
    fn LeaveCriticalSection(section: *mut CriticalSection);
}

/// The waits for the device lock while another thread held it: the count, the sum and the
/// longest, in ticks of the performance counter. Index 0 is the main thread, index 1 every
/// other thread (the cell worker, the loading screen's thread).
static LOCK_WAITS: [core::sync::atomic::AtomicU64; 2] = [core::sync::atomic::AtomicU64::new(0), core::sync::atomic::AtomicU64::new(0)];
static LOCK_WAIT_TICKS: [core::sync::atomic::AtomicU64; 2] = [core::sync::atomic::AtomicU64::new(0), core::sync::atomic::AtomicU64::new(0)];
static LOCK_WAIT_LONGEST: [core::sync::atomic::AtomicU64; 2] = [core::sync::atomic::AtomicU64::new(0), core::sync::atomic::AtomicU64::new(0)];

/// The report line of the waits for the device lock.
pub fn lock_wait_report() -> String {
    use core::sync::atomic::Ordering::Relaxed;
    let per_second = crate::capture::ticks_per_second() as f64;
    let part = |i: usize| {
        format!(
            "{} waits, {:.1} ms in all, the longest {:.2} ms",
            LOCK_WAITS[i].load(Relaxed),
            LOCK_WAIT_TICKS[i].load(Relaxed) as f64 * 1000.0 / per_second,
            LOCK_WAIT_LONGEST[i].load(Relaxed) as f64 * 1000.0 / per_second
        )
    };
    format!("device lock: the main thread {}; the other threads {}", part(0), part(1))
}

static mut SECTION: CriticalSection = CriticalSection([0; 6]);
static SECTION_MADE: Once = Once::new();

/// The lock is free again when this value goes away.
pub struct DeviceLock;

/// The device lock: one device call at a time, from any thread, as with a Direct3D device
/// for more than one thread. It also guards the state of the capture, the own device and
/// the own host.
///
/// The rule: a thread that holds this lock never waits for another game thread. The cell
/// worker takes the lock for its device calls (a texture of a cell it loads), and it stops
/// for a stall only between two references. A stall of the worker under this lock is a
/// deadlock (records entry 50). The stalls are MGE XE's logic of the frame and the
/// capture's purge; both run in `link_client::after_present`, which checks
/// `device_lock_held_here` first.
pub fn device_lock() -> DeviceLock {
    unsafe {
        SECTION_MADE.call_once(|| InitializeCriticalSection(core::ptr::addr_of_mut!(SECTION)));
        // A free lock costs no clock read; a held one is timed (`lock_wait_report`).
        if TryEnterCriticalSection(core::ptr::addr_of_mut!(SECTION)) == 0 {
            let start = crate::capture::ticks();
            EnterCriticalSection(core::ptr::addr_of_mut!(SECTION));
            let waited = (crate::capture::ticks() - start).max(0) as u64;
            let i = usize::from(!crate::shadow::on_main_thread());
            use core::sync::atomic::Ordering::Relaxed;
            LOCK_WAITS[i].fetch_add(1, Relaxed);
            LOCK_WAIT_TICKS[i].fetch_add(waited, Relaxed);
            LOCK_WAIT_LONGEST[i].fetch_max(waited, Relaxed);
        }
    }
    DeviceLock
}

impl Drop for DeviceLock {
    fn drop(&mut self) {
        unsafe { LeaveCriticalSection(core::ptr::addr_of_mut!(SECTION)) };
    }
}

/// State that only a holder of the device lock may touch, from any thread: the capture, the
/// own device, the own host. `get` needs the guard. The rule, as for
/// `main_thread::MainCell`: one open at a time; the guard of `get`
/// ([`crate::checked::Open`]) finds a second open while the first is live. The section is
/// reentrant, so a function that cannot take its caller's guard takes the lock again.
pub struct LockCell<T>(crate::checked::Checked<T>);

// The cell is a static shared between threads; the lock orders the opens.
unsafe impl<T> Sync for LockCell<T> {}

impl<T> LockCell<T> {
    pub const fn new(value: T) -> LockCell<T> {
        LockCell(crate::checked::Checked::new(value))
    }

    /// The state, for a holder of the lock, until the guard ends. One open at a time (see
    /// the type).
    #[track_caller]
    #[inline(always)]
    pub fn get<'a>(&'a self, _: &'a DeviceLock) -> crate::checked::Open<'a, T> {
        // The guard says the lock is held: no other thread opens the state.
        unsafe { self.0.open() }
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
/// `NiDX8LightManager+0x50`), so a walk under it cannot see a half-changed structure.
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
