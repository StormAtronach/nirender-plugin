//! The pacing of the game to the helper, as `bevy_framepace` paces a Bevy app, across the two
//! programs. At the start of each frame of the game (MWSE's native event 8, before
//! `WorldController::mainLoopBeforeInput` takes the time of the frame and reads the input) the
//! game waits until the helper will need the frame, less the game's own work for a frame and a
//! margin. The input of the frame is then as late as the helper allows, and the frame does not
//! wait in the ring.
//!
//! The facts: the helper writes into the head of the ring when it showed a frame, how far it
//! had read then, and its busy time for a frame (`Ring::shown` of the helper); the game measures
//! its own work. The margin follows the measured lead of each frame: the time the helper was
//! free for the frame less the end of the frame. A positive lead is a frame that waited in the
//! ring (lag), a negative one a helper that waited for the frame (frames lost).
//! `NIRENDER_PACE=0` turns the pacing off.

use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

use crate::capture::{ticks, ticks_per_second, wait_step};
use crate::stream_format::{RING_BUSY, RING_DONE_AT, RING_DONE_TIME, RING_HEAD, RING_NEXT_START, SHOWN_EVENT};

extern "system" {
    fn MapViewOfFile(mapping: isize, access: u32, offset_high: u32, offset_low: u32, bytes: usize) -> *mut u8;
    fn UnmapViewOfFile(address: *const u8) -> i32;
    fn CloseHandle(handle: isize) -> i32;
    fn CreateEventA(security: *const u8, manual: i32, initial: i32, name: *const u8) -> isize;
    fn CreateWaitableTimerExW(security: *const u8, name: *const u16, flags: u32, access: u32) -> isize;
    fn SetWaitableTimer(timer: isize, due: *const i64, period: i32, routine: usize, argument: usize, resume: i32) -> i32;
    fn MsgWaitForMultipleObjects(count: u32, handles: *const isize, all: i32, milliseconds: u32, wake: u32) -> u32;
}

/// The lead that the margin steers to, in microseconds: the frame complete this long before the
/// helper is free for it. `NIRENDER_PACE_LEAD_US` changes it, for a test.
const LEAD_US: i64 = 200;

fn lead_us() -> i64 {
    static LEAD: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *LEAD.get_or_init(|| crate::switches::number("NIRENDER_PACE_LEAD_US", LEAD_US))
}
/// The part of the error of a lead that the margin takes at each frame.
const GAIN: f64 = 0.1;

struct Pacer {
    /// A view of the head of the ring of its own: the stream can close the ring meanwhile.
    head: *mut u8,
    /// A timer of high resolution for the waits (0 without one: the wait then spins).
    timer: isize,
    /// The event `<ring>-shown`, which the helper sets when it has shown a frame (0 without
    /// one: the wait for a frame shown then spins).
    shown: isize,
    /// The bytes written up to the end of the frame before the last one, and of the last one.
    before: u64,
    last: u64,
    /// The end of the last frame and its start after its wait, in ticks.
    end: i64,
    started: i64,
    /// The game's work for a frame, a mean in ticks, and the margin in ticks.
    work: f64,
    margin: f64,
    /// The sum of the leads and their count, for the report.
    leads: f64,
    frames: u64,
}

// SAFETY: the view and the timer belong to the pacer; the mutex orders the threads.
unsafe impl Send for Pacer {}

static PACER: Mutex<Option<Pacer>> = Mutex::new(None);
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// The time that the game waited at the starts of the frames, in ticks.
static WAITED: AtomicI64 = AtomicI64::new(0);

/// Listens to MWSE's start of a frame. A refusal leaves the game without the pacing: its wait
/// at Present stays.
pub unsafe fn install() {
    if INSTALLED.load(Ordering::Relaxed) {
        return;
    }
    if crate::switches::on("NIRENDER_PACE") == Some(false) {
        crate::log!("pacing: NIRENDER_PACE=0, no pacing of the game to the helper");
        return;
    }
    use crate::mwse_native::{subscribe, NATIVE_EVENT_FRAME_STARTED};
    if let Err(error) = subscribe(NATIVE_EVENT_FRAME_STARTED, started) {
        crate::log!("pacing: {error}: no pacing of the game to the helper");
        return;
    }
    INSTALLED.store(true, Ordering::Relaxed);
    crate::log!("pacing: the game waits for the helper at the start of each frame (MWSE event 8), lead {} us", lead_us());
}

/// A stream to the helper starts: the pacer maps the head of its ring `name`.
pub unsafe fn start(mapping: isize, name: &str) {
    if !INSTALLED.load(Ordering::Relaxed) {
        return;
    }
    // FILE_MAP_ALL_ACCESS
    let head = MapViewOfFile(mapping, 0xF001F, 0, 0, RING_HEAD);
    if head.is_null() {
        crate::log!("pacing: no view of the head of the ring, no pacing");
        return;
    }
    // CREATE_WAITABLE_TIMER_HIGH_RESOLUTION (2), TIMER_ALL_ACCESS.
    let timer = CreateWaitableTimerExW(core::ptr::null(), core::ptr::null(), 2, 0x1F_0003);
    // An event that resets itself: one wait takes one set.
    let mut event_name = format!("{name}{SHOWN_EVENT}").into_bytes();
    event_name.push(0);
    let shown = CreateEventA(core::ptr::null(), 0, 0, event_name.as_ptr());
    let pacer = Pacer { head, timer, shown, before: 0, last: 0, end: 0, started: 0, work: 0.0, margin: 0.0, leads: 0.0, frames: 0 };
    if let Ok(mut guard) = PACER.lock() {
        *guard = Some(pacer);
    }
}

/// The stream ends: the pacer reports and lets the view go.
pub unsafe fn stop() {
    let Some(pacer) = PACER.lock().ok().and_then(|mut guard| guard.take()) else { return };
    let ms = |ticks: f64| ticks * 1000.0 / ticks_per_second() as f64;
    crate::log!(
        "pacing: {} frames paced, the mean lead {:.2} ms, the margin at the end {:.2} ms, the game's work {:.2} ms for a frame, {:.1} ms waited in all",
        pacer.frames,
        ms(pacer.leads / pacer.frames.max(1) as f64),
        ms(pacer.margin),
        ms(pacer.work),
        ms(WAITED.load(Ordering::Relaxed) as f64)
    );
    UnmapViewOfFile(pacer.head);
    if pacer.timer != 0 {
        CloseHandle(pacer.timer);
    }
    if pacer.shown != 0 {
        CloseHandle(pacer.shown);
    }
}

/// The end of a frame of the stream (its Present): the bytes written up to it and its time.
/// The game writes no frame until the start of the next one (`RING_NEXT_START`).
pub fn frame_end(written: u64, end: i64) {
    if let Some(pacer) = PACER.lock().ok().as_mut().and_then(|guard| guard.as_mut()) {
        pacer.before = pacer.last;
        pacer.last = written;
        pacer.end = end;
        unsafe { pacer.announce(i64::MAX) };
    }
}

/// The time that the game waited at the starts of the frames, in ticks.
pub fn waited() -> i64 {
    WAITED.load(Ordering::Relaxed)
}

/// MWSE_NATIVE_EVENT_FRAME_STARTED, on the main thread.
unsafe extern "C" fn started(_user: *mut c_void, _packet: *const c_void) {
    let Ok(mut guard) = PACER.lock() else { return };
    if let Some(pacer) = guard.as_mut() {
        pacer.pace();
    }
}

impl Pacer {
    unsafe fn word(&self, offset: usize) -> u64 {
        (*(self.head.add(offset) as *const AtomicU64)).load(Ordering::Acquire)
    }

    unsafe fn pace(&mut self) {
        let second = ticks_per_second();
        let start = ticks();
        if self.started != 0 && self.end > self.started {
            let work = (self.end - self.started) as f64;
            self.work = if self.work == 0.0 { work } else { self.work + (work - self.work) * 0.1 };
        }
        // No frame before, or a helper before its first frame shown.
        if self.before == 0 || self.word(RING_DONE_AT) == 0 {
            return self.begin(start);
        }
        // The helper shows the frame before the last one; a helper that does not in 50 ms is
        // late, and the frame goes without a wait.
        while self.word(RING_DONE_AT) < self.before {
            if ticks() - start > second / 20 {
                return self.begin(start);
            }
            // Asleep until the helper sets the event, or a message or input comes for the game's
            // windows; 1 ms at most. A set before the wait stays set for it.
            if self.shown != 0 {
                MsgWaitForMultipleObjects(1, &self.shown, 0, 1, crate::capture::WAIT_WAKE_MASK);
            }
            wait_step();
        }
        let shown = self.word(RING_DONE_TIME) as i64;
        let busy = (*(self.head.add(RING_BUSY) as *const AtomicU32)).load(Ordering::Acquire) as i64 * second / 1_000_000;
        // The helper was free for the last frame when it had shown the one before.
        let lead = (shown - self.end) as f64;
        self.leads += lead;
        self.frames += 1;
        let want = (lead_us() * second / 1_000_000) as f64;
        self.margin = (self.margin + (want - lead) * GAIN).clamp(0.0, busy as f64);
        // The helper takes the last frame when it is free and has it, and needs the next one
        // `busy` later.
        let needed = shown.max(self.end) + busy;
        let target = (needed - self.work as i64 - self.margin as i64).min(ticks() + second / 10);
        // The helper sleeps while the game does, and waits awake from the start of the frame.
        self.announce(target);
        self.wait_until(target);
        self.begin(start);
    }

    /// The start of the game's next frame, for the helper's wait for the records
    /// ([`RING_NEXT_START`]): the time the wait at the start of the frame ends, the time the
    /// frame started, or `i64::MAX` from the end of a frame to the start of the next.
    unsafe fn announce(&self, start: i64) {
        (*(self.head.add(RING_NEXT_START) as *const AtomicI64)).store(start, Ordering::Release);
    }

    /// The frame starts: its input is read next.
    fn begin(&mut self, start: i64) {
        let now = ticks();
        WAITED.fetch_add(now - start, Ordering::Relaxed);
        self.started = now;
        unsafe { self.announce(now) };
        crate::capture::FRAME_START.store(now, Ordering::Relaxed);
    }

    /// Waits until `target` (ticks). More than 1 ms before it, on the timer, which a message or
    /// input for the game's windows also wakes; the rest spins.
    unsafe fn wait_until(&self, target: i64) {
        let second = ticks_per_second();
        loop {
            let left = target - ticks();
            if left <= 0 {
                return;
            }
            if self.timer != 0 && left > second / 1000 {
                // In 100 ns units; negative: from now. Wake 0.5 ms early.
                let due = -((left - second / 2000) * 10_000_000 / second);
                SetWaitableTimer(self.timer, &due, 0, 0, 0, 0);
                MsgWaitForMultipleObjects(1, &self.timer, 0, 0xFFFF_FFFF, crate::capture::WAIT_WAKE_MASK);
            }
            wait_step();
        }
    }
}
