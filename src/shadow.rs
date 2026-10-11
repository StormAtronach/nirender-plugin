//! Modes of a port and the shadow comparison.
//!
//! In shadow mode a call runs two times from the same start state. The Rust function runs
//! first on a dry device. Then the engine function runs for real. The device calls, the
//! return value and the memory that the function can write must be equal. The result of the
//! engine function stays.

use core::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};

use crate::device;
use crate::hook::Cleanup;
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

/// A count of calls. Only the main thread adds to it, so the add has no bus lock: a locked
/// 64-bit add is a slow loop on a 32-bit processor, and the ports count on every call.
pub struct Counter {
    low: AtomicU32,
    high: AtomicU32,
}

impl Counter {
    pub const fn new() -> Counter {
        Counter { low: AtomicU32::new(0), high: AtomicU32::new(0) }
    }

    #[inline(always)]
    pub fn add(&self) {
        let low = self.low.load(Ordering::Relaxed).wrapping_add(1);
        self.low.store(low, Ordering::Relaxed);
        if low == 0 {
            self.high.store(self.high.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
        }
    }

    pub fn get(&self) -> u64 {
        (self.high.load(Ordering::Relaxed) as u64) << 32 | self.low.load(Ordering::Relaxed) as u64
    }
}

/// A sum of time stamps of the processor. Only the main thread adds to it, as `Counter`.
pub struct Stamps {
    low: AtomicU32,
    high: AtomicU32,
}

impl Stamps {
    pub const fn new() -> Stamps {
        Stamps { low: AtomicU32::new(0), high: AtomicU32::new(0) }
    }

    #[inline(always)]
    pub fn add(&self, delta: u64) {
        let sum = self.get().wrapping_add(delta);
        self.low.store(sum as u32, Ordering::Relaxed);
        self.high.store((sum >> 32) as u32, Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        (self.high.load(Ordering::Relaxed) as u64) << 32 | self.low.load(Ordering::Relaxed) as u64
    }

    /// Returns the sum and sets it to zero.
    pub fn take(&self) -> u64 {
        let sum = self.get();
        self.low.store(0, Ordering::Relaxed);
        self.high.store(0, Ordering::Relaxed);
        sum
    }
}

/// The time stamp counter of the processor: much cheaper than the performance counter,
/// for a measurement on every call of a port.
#[inline(always)]
pub fn stamp() -> u64 {
    unsafe { core::arch::x86::_rdtsc() }
}

/// The performance counter and the time stamp at the start, for the rate of the stamps.
static STAMP_START_TICKS: AtomicI64 = AtomicI64::new(0);
static STAMP_START: AtomicU64 = AtomicU64::new(0);

/// Notes the start for `stamps_to_ms`.
pub fn calibrate_stamps() {
    STAMP_START_TICKS.store(crate::capture::ticks(), Ordering::Relaxed);
    STAMP_START.store(stamp(), Ordering::Relaxed);
}

/// Milliseconds for a number of time stamps. The rate of the stamps is that over the time
/// since `calibrate_stamps`.
pub fn stamps_to_ms(stamps: u64) -> f64 {
    let seconds = (crate::capture::ticks() - STAMP_START_TICKS.load(Ordering::Relaxed)) as f64 / crate::capture::ticks_per_second() as f64;
    let rate = stamp().wrapping_sub(STAMP_START.load(Ordering::Relaxed)) as f64 / seconds.max(1e-6);
    stamps as f64 * 1000.0 / rate.max(1.0)
}

/// One port.
pub struct PortState {
    pub name: &'static str,
    pub addr: usize,
    pub stack_args: usize,
    pub cleanup: Cleanup,
    pub detour: fn() -> usize,
    /// The address to call for the engine function. 0 until the detour is in place.
    pub orig: AtomicUsize,
    pub mode: AtomicU8,
    /// Another mod changed this function in a way that the port does not serve. The function
    /// stays game code in this install. The mode cannot leave "off".
    pub locked: AtomicBool,
    /// A function to call after each call of the port, in every mode. 0 means none.
    pub after: AtomicUsize,
    /// A function to call before each call of the port, in every mode, with the arguments
    /// of the call. 0 means none. See `run_before`.
    pub before: AtomicUsize,
    /// Calls that ran the Rust function for real.
    pub live_calls: Counter,
    /// The time of the main thread in those calls, with the ports they call, in time
    /// stamps of the processor. `take` resets it for a report of one step.
    pub live_stamps: Stamps,
    /// The number of runs of the Rust function: live, in a comparison, or as a call from
    /// another port.
    pub rust_runs: Counter,
    pub shadow_calls: Counter,
    /// Calls in shadow mode that the shadow test skipped.
    pub skipped: Counter,
    pub mismatches: Counter,
}

impl PortState {
    pub const fn new(name: &'static str, addr: usize, stack_args: usize, cleanup: Cleanup, detour: fn() -> usize) -> PortState {
        PortState {
            name,
            addr,
            stack_args,
            cleanup,
            detour,
            orig: AtomicUsize::new(0),
            mode: AtomicU8::new(0),
            locked: AtomicBool::new(false),
            after: AtomicUsize::new(0),
            before: AtomicUsize::new(0),
            live_calls: Counter::new(),
            live_stamps: Stamps::new(),
            rust_runs: Counter::new(),
            shadow_calls: Counter::new(),
            skipped: Counter::new(),
            mismatches: Counter::new(),
        }
    }

    pub fn mode(&self) -> Mode {
        match self.mode.load(Ordering::Relaxed) {
            x if x == Mode::Shadow as u8 => Mode::Shadow,
            x if x == Mode::Live as u8 => Mode::Live,
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

    /// Makes the function stay game code in this install.
    pub fn lock(&self) {
        self.locked.store(true, Ordering::Relaxed);
        self.set_mode(Mode::Off);
    }

    pub fn installed(&self) -> bool {
        self.orig.load(Ordering::Relaxed) != 0
    }
}

/// Counts one run of the Rust function of a port. Each port function calls it first.
#[inline(always)]
pub fn touch(state: &PortState) {
    state.rust_runs.add();
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
/// A float result is on the x87 stack, not in EAX. The comparison uses its bits.
impl Eax for f32 {
    fn to_eax(self) -> u32 {
        self.to_bits()
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

/// The run of a comparison that is in progress.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Leg {
    /// No comparison runs.
    None = 0,
    Rust = 1,
    Engine = 2,
}

static LEG: AtomicU8 = AtomicU8::new(Leg::None as u8);
static MAIN_THREAD: AtomicU32 = AtomicU32::new(0);
/// True while a frame comparison runs. Shadow mode does not compare calls then.
static SUSPENDED: AtomicBool = AtomicBool::new(false);

pub fn set_main_thread() {
    MAIN_THREAD.store(unsafe { win::GetCurrentThreadId() }, Ordering::Relaxed);
}

pub fn on_main_thread() -> bool {
    unsafe { win::GetCurrentThreadId() == MAIN_THREAD.load(Ordering::Relaxed) }
}

/// The run of a comparison that the calling thread is in. A comparison runs on the main
/// thread only. Other threads are never in one.
pub fn leg() -> Leg {
    match LEG.load(Ordering::Relaxed) {
        x if x == Leg::Rust as u8 && on_main_thread() => Leg::Rust,
        x if x == Leg::Engine as u8 && on_main_thread() => Leg::Engine,
        _ => Leg::None,
    }
}

fn set_leg(leg: Leg) {
    LEG.store(leg as u8, Ordering::Relaxed);
}

pub fn suspend_shadow(on: bool) {
    SUSPENDED.store(on, Ordering::Relaxed);
}

/// The memory regions that a function can write. The port gives them for each call.
pub struct Regions {
    list: Vec<(usize, usize)>,
    skip: bool,
}

impl Regions {
    pub const fn new() -> Regions {
        Regions { list: Vec::new(), skip: false }
    }
    /// Adds a block of memory. The list has no limit. A null address or a size of 0 adds
    /// nothing.
    pub fn add(&mut self, addr: usize, size: usize) {
        if addr != 0 && size != 0 {
            self.list.push((addr, size));
        }
    }
    /// This call allocates or frees memory. The shadow test can compare it only with the
    /// allocator hooks.
    pub fn allocates(&mut self) {
        if !crate::alloc::active() {
            self.skip = true;
        }
    }
    /// The shadow test cannot compare this call. A call that does a `Present` is one example.
    /// The engine function runs alone.
    pub fn skip(&mut self) {
        self.skip = true;
    }
    pub fn as_slice(&self) -> &[(usize, usize)] {
        &self.list
    }
}

struct Scratch {
    before: Vec<u8>,
    after_rust: Vec<u8>,
    log_rust: Vec<u32>,
    log_engine: Vec<u32>,
    blocks_rust: Vec<u8>,
    /// The list of the last comparison. The next one uses its memory again.
    regions: Vec<(usize, usize)>,
}

static mut SCRATCH: Scratch = Scratch {
    before: Vec::new(),
    after_rust: Vec::new(),
    log_rust: Vec::new(),
    log_engine: Vec::new(),
    blocks_rust: Vec::new(),
    regions: Vec::new(),
};

/// The number of mismatches of one port that go into the log with their details.
const REPORTED_MISMATCHES: u64 = 8;

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

/// True when the memory now is not the same as the saved bytes.
unsafe fn regions_differ(regions: &[(usize, usize)], rust: &[u8]) -> bool {
    let mut at = 0;
    for &(addr, size) in regions {
        if core::slice::from_raw_parts(addr as *const u8, size) != &rust[at..at + size] {
            return true;
        }
        at += size;
    }
    false
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

/// An argument of a port as one machine word, for a `before` function.
pub trait Word: Copy {
    fn word(self) -> usize;
}
impl Word for usize {
    fn word(self) -> usize {
        self
    }
}
impl Word for u32 {
    fn word(self) -> usize {
        self as usize
    }
}
impl Word for u8 {
    fn word(self) -> usize {
        self as usize
    }
}
impl Word for f32 {
    fn word(self) -> usize {
        self.to_bits() as usize
    }
}

/// The type of a `before` function. `args` has the arguments of the call. The first one of
/// a `__thiscall` function is `this`.
pub type BeforeFn = unsafe fn(args: &[usize]);

/// Calls the `before` function of a port. The function does not run in the Rust run of a
/// comparison and not on other threads: the engine run of the comparison makes the call
/// that counts.
pub unsafe fn run_before(before: usize, args: &[usize]) {
    if on_main_thread() && LEG.load(Ordering::Relaxed) != Leg::Rust as u8 {
        core::mem::transmute::<usize, BeforeFn>(before)(args);
    }
}

/// The calls of ports in live mode that are in progress, and the time in the outermost
/// ones. Only the main thread changes them.
static PORT_DEPTH: AtomicU32 = AtomicU32::new(0);
static PORT_STAMPS: Stamps = Stamps::new();

/// The time of the main thread in the ports in live mode since the start, in ms. A port
/// that a port calls is not counted again.
pub fn port_ms() -> f64 {
    stamps_to_ms(PORT_STAMPS.get())
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
    if !on_main_thread() {
        crate::sync::off_thread("port", state.name);
        return engine();
    }
    let mode = state.mode();
    match LEG.load(Ordering::Relaxed) {
        // Inside the Rust run of a comparison every function that has a port runs as Rust. A
        // port that is locked or in mode off is the exception.
        x if x == Leg::Rust as u8 => {
            return if mode == Mode::Off { engine() } else { rust().to_eax() };
        }
        x if x == Leg::Engine as u8 => return engine(),
        _ => {}
    }
    match mode {
        Mode::Off => engine(),
        Mode::Live => {
            state.live_calls.add();
            let depth = PORT_DEPTH.load(Ordering::Relaxed);
            let start = stamp();
            PORT_DEPTH.store(depth + 1, Ordering::Relaxed);
            let eax = rust().to_eax();
            PORT_DEPTH.store(depth, Ordering::Relaxed);
            let spent = stamp().wrapping_sub(start);
            state.live_stamps.add(spent);
            if depth == 0 {
                PORT_STAMPS.add(spent);
            }
            eax
        }
        Mode::Shadow => compare(state, compare_eax, regions, rust, engine),
    }
}

/// One call in shadow mode: the Rust run, the engine run, and the comparison of the two.
#[inline(never)]
unsafe fn compare<R: Eax>(
    state: &PortState,
    compare_eax: bool,
    regions: impl FnOnce(&mut Regions),
    rust: impl FnOnce() -> R,
    engine: impl FnOnce() -> u32,
) -> u32 {
    if SUSPENDED.load(Ordering::Relaxed) || device::recording() {
        return engine();
    }
    crate::install::attach_device_early();
    let scratch = &mut *core::ptr::addr_of_mut!(SCRATCH);
    let mut set = Regions { list: core::mem::take(&mut scratch.regions), skip: false };
    set.list.clear();
    regions(&mut set);
    if set.skip {
        scratch.regions = set.list;
        state.skipped.add();
        return engine();
    }
    let regions = set.as_slice();

    save(regions, &mut scratch.before);
    let track = crate::alloc::active();
    if track {
        crate::alloc::begin_rust();
    }
    set_leg(Leg::Rust);
    device::begin_rust_run();
    let eax_rust = rust().to_eax();
    device::end(&mut scratch.log_rust);
    save(regions, &mut scratch.after_rust);
    restore(regions, &scratch.before);
    if track {
        crate::alloc::begin_engine(&mut scratch.blocks_rust);
    }

    set_leg(Leg::Engine);
    device::begin_engine_run();
    let eax_engine = engine();
    device::end(&mut scratch.log_engine);
    let resources = device::end_engine_run();
    set_leg(Leg::None);
    let heap = if track { crate::alloc::end(&scratch.blocks_rust) } else { String::new() };

    state.shadow_calls.add();
    let eax_differs = compare_eax && (eax_rust ^ eax_engine) & R::MASK != 0;
    let memory_differs = regions_differ(regions, &scratch.after_rust);
    let calls_differ = scratch.log_rust != scratch.log_engine;
    if calls_differ || memory_differs || !heap.is_empty() || !resources.is_empty() || eax_differs {
        state.mismatches.add();
        // The text of a difference costs time. Only the first ones go into the log.
        if state.mismatches.get() <= REPORTED_MISMATCHES {
            let calls = device::first_difference(&scratch.log_rust, &scratch.log_engine);
            let memory = memory_differences(regions, &scratch.after_rust);
            crate::log!(
                "MISMATCH {} (call {}): {}{}{}{}{}",
                state.name,
                state.shadow_calls.get(),
                calls.map(|c| format!("device {c};")).unwrap_or_default(),
                if memory.is_empty() { String::new() } else { format!(" memory{memory}") },
                if heap.is_empty() { String::new() } else { format!(" heap{heap}") },
                if resources.is_empty() { String::new() } else { format!(" resources{resources}") },
                if eax_differs { format!(" eax rust {eax_rust:#x} engine {eax_engine:#x}") } else { String::new() },
            );
        }
    }
    scratch.regions = set.list;
    eax_engine
}
