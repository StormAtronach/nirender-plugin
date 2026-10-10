//! Modes of a port and the shadow comparison.
//!
//! In shadow mode a call runs two times from the same start state. The Rust function runs
//! first on a dry device. Then the engine function runs for real. The device calls, the
//! return value and the memory that the function can write must be equal. The result of the
//! engine function stays.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};

use crate::device;
use crate::win;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Mode {
    Off = 0,
    Shadow = 1,
    Live = 2,
}

impl Mode {
    pub fn parse(text: &str) -> Option<Mode> {
        match text {
            "off" => Some(Mode::Off),
            "shadow" => Some(Mode::Shadow),
            "live" => Some(Mode::Live),
            _ => None,
        }
    }
}

/// One port.
pub struct PortState {
    pub name: &'static str,
    pub addr: usize,
    pub stack_args: usize,
    /// The caller removes the stack arguments.
    pub cdecl: bool,
    pub detour: fn() -> usize,
    /// The address to call for the engine function. 0 until the detour is in place.
    pub orig: AtomicUsize,
    pub mode: AtomicU8,
    /// Another mod changed this function in a way that the port does not serve. The function
    /// stays game code in this install. The mode cannot leave "off".
    pub locked: AtomicBool,
    /// A function to call after each call of the port, in every mode. 0 means none.
    pub after: AtomicUsize,
    /// Calls that ran the Rust function for real.
    pub live_calls: AtomicU64,
    /// The number of runs of the Rust function: live, in a comparison, or as a call from
    /// another port.
    pub rust_runs: AtomicU64,
    pub shadow_calls: AtomicU64,
    /// Calls in shadow mode that the shadow test skipped.
    pub skipped: AtomicU64,
    pub mismatches: AtomicU64,
    pub reported: AtomicU32,
}

impl PortState {
    pub const fn new_cdecl(name: &'static str, addr: usize, stack_args: usize, detour: fn() -> usize) -> PortState {
        let mut state = PortState::new(name, addr, stack_args, detour);
        state.cdecl = true;
        state
    }

    pub const fn new(name: &'static str, addr: usize, stack_args: usize, detour: fn() -> usize) -> PortState {
        PortState {
            name,
            addr,
            stack_args,
            cdecl: false,
            detour,
            orig: AtomicUsize::new(0),
            mode: AtomicU8::new(0),
            locked: AtomicBool::new(false),
            after: AtomicUsize::new(0),
            live_calls: AtomicU64::new(0),
            rust_runs: AtomicU64::new(0),
            shadow_calls: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
            mismatches: AtomicU64::new(0),
            reported: AtomicU32::new(0),
        }
    }

    pub fn mode(&self) -> Mode {
        match self.mode.load(Ordering::Relaxed) {
            1 => Mode::Shadow,
            2 => Mode::Live,
            _ => Mode::Off,
        }
    }

    pub fn set_mode(&self, mode: Mode) {
        let mode = if self.locked.load(Ordering::Relaxed) { Mode::Off } else { mode };
        self.mode.store(mode as u8, Ordering::Relaxed);
    }

    pub fn locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    pub fn installed(&self) -> bool {
        self.orig.load(Ordering::Relaxed) != 0
    }
}

/// Counts one run of the Rust function of a port. Each port function calls it first.
#[inline(always)]
pub fn touch(state: &PortState) {
    state.rust_runs.fetch_add(1, Ordering::Relaxed);
}

/// What a function leaves in EAX.
pub trait Eax: Copy {
    fn to_eax(self) -> u32;
    /// The bits of EAX that carry the value.
    const MASK: u32;
}
impl Eax for () {
    fn to_eax(self) -> u32 {
        0
    }
    const MASK: u32 = 0;
}
impl Eax for u32 {
    fn to_eax(self) -> u32 {
        self
    }
    const MASK: u32 = !0;
}
impl Eax for usize {
    fn to_eax(self) -> u32 {
        self as u32
    }
    const MASK: u32 = !0;
}
impl Eax for u8 {
    fn to_eax(self) -> u32 {
        self as u32
    }
    const MASK: u32 = 0xFF;
}
impl Eax for bool {
    fn to_eax(self) -> u32 {
        self as u32
    }
    const MASK: u32 = 0xFF;
}

const LEG_NONE: u8 = 0;
const LEG_RUST: u8 = 1;
const LEG_ENGINE: u8 = 2;

static LEG: AtomicU8 = AtomicU8::new(LEG_NONE);
static MAIN_THREAD: AtomicU32 = AtomicU32::new(0);
/// Not 0 while a frame comparison runs. Shadow mode does not compare calls then.
static SUSPENDED: AtomicU8 = AtomicU8::new(0);

pub fn set_main_thread() {
    MAIN_THREAD.store(unsafe { win::GetCurrentThreadId() }, Ordering::Relaxed);
}

pub fn suspend_shadow(on: bool) {
    SUSPENDED.store(on as u8, Ordering::Relaxed);
}

/// The memory regions that a function can write. The port gives them for each call.
pub struct Regions {
    list: [(usize, usize); 8],
    len: usize,
    skip: bool,
}

impl Regions {
    pub const fn new() -> Regions {
        Regions { list: [(0, 0); 8], len: 0, skip: false }
    }
    pub fn add(&mut self, addr: usize, size: usize) {
        if addr != 0 && size != 0 && self.len < self.list.len() {
            self.list[self.len] = (addr, size);
            self.len += 1;
        }
    }
    /// This call allocates or frees memory. The shadow test can compare it only with the
    /// allocator hooks.
    pub fn allocates(&mut self) {
        if !crate::alloc::active() {
            self.skip = true;
        }
    }
    /// The shadow test cannot compare this call. A call that creates a device resource is one
    /// example. The engine function runs alone.
    pub fn skip(&mut self) {
        self.skip = true;
    }
    pub fn as_slice(&self) -> &[(usize, usize)] {
        &self.list[..self.len]
    }
}

struct Scratch {
    before: Vec<u8>,
    after_rust: Vec<u8>,
    log_rust: Vec<u32>,
    log_engine: Vec<u32>,
    blocks_rust: Vec<u8>,
}

static mut SCRATCH: Scratch = Scratch { before: Vec::new(), after_rust: Vec::new(), log_rust: Vec::new(), log_engine: Vec::new(), blocks_rust: Vec::new() };

unsafe fn save(regions: &[(usize, usize)], into: &mut Vec<u8>) {
    into.clear();
    for &(addr, size) in regions {
        into.extend_from_slice(core::slice::from_raw_parts(addr as *const u8, size));
    }
}

unsafe fn restore(regions: &[(usize, usize)], from: &[u8]) {
    let mut at = 0;
    for &(addr, size) in regions {
        core::ptr::copy_nonoverlapping(from.as_ptr().add(at), addr as *mut u8, size);
        at += size;
    }
}

/// The differences between the saved bytes and the memory now, as text. The text is empty
/// when they are equal.
unsafe fn memory_differences(regions: &[(usize, usize)], rust: &[u8]) -> String {
    let mut text = String::new();
    let mut at = 0;
    let mut shown = 0;
    for (index, &(addr, size)) in regions.iter().enumerate() {
        let engine = core::slice::from_raw_parts(addr as *const u8, size);
        let rust_part = &rust[at..at + size];
        at += size;
        if engine == rust_part {
            continue;
        }
        let mut offset = 0;
        while offset < size && shown < 12 {
            let end = (offset + 4).min(size);
            if engine[offset..end] != rust_part[offset..end] {
                text.push_str(&format!(
                    " region{}+{:#x}: rust {:02x?} engine {:02x?};",
                    index,
                    offset & !3,
                    &rust_part[offset..end],
                    &engine[offset..end]
                ));
                shown += 1;
            }
            offset = end;
        }
    }
    text
}

/// Runs a call of a port in the mode of the port.
///
/// `rust` and `engine` return what the function leaves in EAX. `compare_eax` tells if that
/// value has a meaning.
pub unsafe fn dispatch<R: Eax>(
    state: &PortState,
    compare_eax: bool,
    regions: impl FnOnce(&mut Regions),
    rust: impl FnOnce() -> R,
    engine: impl FnOnce() -> u32,
) -> u32 {
    let eax = dispatch_inner(state, compare_eax, regions, rust, engine);
    let after = state.after.load(Ordering::Relaxed);
    if after != 0 {
        core::mem::transmute::<usize, unsafe fn()>(after)();
    }
    eax
}

unsafe fn dispatch_inner<R: Eax>(
    state: &PortState,
    compare_eax: bool,
    regions: impl FnOnce(&mut Regions),
    rust: impl FnOnce() -> R,
    engine: impl FnOnce() -> u32,
) -> u32 {
    if win::GetCurrentThreadId() != MAIN_THREAD.load(Ordering::Relaxed) {
        return engine();
    }
    let mode = state.mode();
    match LEG.load(Ordering::Relaxed) {
        // Inside the Rust run of a comparison every function that has a port runs as Rust. A
        // port that is locked or in mode off is the exception.
        LEG_RUST => {
            return if mode == Mode::Off { engine() } else { rust().to_eax() };
        }
        LEG_ENGINE => return engine(),
        _ => {}
    }
    match mode {
        Mode::Off => engine(),
        Mode::Live => {
            state.live_calls.fetch_add(1, Ordering::Relaxed);
            rust().to_eax()
        }
        Mode::Shadow => {
            if SUSPENDED.load(Ordering::Relaxed) != 0 || device::recording() {
                return engine();
            }
            let mut set = Regions::new();
            regions(&mut set);
            if set.skip {
                state.skipped.fetch_add(1, Ordering::Relaxed);
                return engine();
            }
            let regions = set.as_slice();
            let scratch = &mut *core::ptr::addr_of_mut!(SCRATCH);

            save(regions, &mut scratch.before);
            let track = crate::alloc::active();
            if track {
                crate::alloc::begin_rust();
            }
            LEG.store(LEG_RUST, Ordering::Relaxed);
            device::begin(true);
            let eax_rust = rust().to_eax();
            device::end(&mut scratch.log_rust);
            save(regions, &mut scratch.after_rust);
            restore(regions, &scratch.before);
            if track {
                crate::alloc::begin_engine(&mut scratch.blocks_rust);
            }

            LEG.store(LEG_ENGINE, Ordering::Relaxed);
            device::begin_replay();
            let eax_engine = engine();
            device::end(&mut scratch.log_engine);
            let resources = device::end_replay();
            LEG.store(LEG_NONE, Ordering::Relaxed);
            let heap = if track { crate::alloc::end(&scratch.blocks_rust) } else { String::new() };

            state.shadow_calls.fetch_add(1, Ordering::Relaxed);
            let calls = device::first_difference(&scratch.log_rust, &scratch.log_engine);
            let memory = memory_differences(regions, &scratch.after_rust);
            let eax_differs = compare_eax && (eax_rust ^ eax_engine) & R::MASK != 0;
            if calls.is_some() || !memory.is_empty() || !heap.is_empty() || !resources.is_empty() || eax_differs {
                state.mismatches.fetch_add(1, Ordering::Relaxed);
                if state.reported.fetch_add(1, Ordering::Relaxed) < 8 {
                    crate::log!(
                        "MISMATCH {} (call {}): {}{}{}{}{}",
                        state.name,
                        state.shadow_calls.load(Ordering::Relaxed),
                        calls.map(|c| format!("device {c};")).unwrap_or_default(),
                        if memory.is_empty() { String::new() } else { format!(" memory{memory}") },
                        if heap.is_empty() { String::new() } else { format!(" heap{heap}") },
                        if resources.is_empty() { String::new() } else { format!(" resources{resources}") },
                        if eax_differs { format!(" eax rust {eax_rust:#x} engine {eax_engine:#x}") } else { String::new() },
                    );
                }
            }
            eax_engine
        }
    }
}
