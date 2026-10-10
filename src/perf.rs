//! Frame times, for the comparison of the game code with the Rust code.
//!
//! A frame time is the time between two `Present` calls of the game. It comes from the high
//! resolution counter of Windows.

use core::sync::atomic::{AtomicBool, Ordering};

#[link(name = "kernel32")]
extern "system" {
    fn QueryPerformanceCounter(count: *mut i64) -> i32;
    fn QueryPerformanceFrequency(frequency: *mut i64) -> i32;
}

static COLLECTING: AtomicBool = AtomicBool::new(false);

struct State {
    last: i64,
    /// Frame times in counter ticks.
    ticks: Vec<i64>,
}

static mut STATE: State = State { last: 0, ticks: Vec::new() };

/// Starts a measurement. The first frame time is the one after the next `Present`.
pub unsafe fn start() {
    let state = &mut *core::ptr::addr_of_mut!(STATE);
    state.ticks.clear();
    state.ticks.reserve(1 << 16);
    state.last = 0;
    COLLECTING.store(true, Ordering::Relaxed);
}

/// The device recorder calls this at each `Present` of the game.
#[inline]
pub unsafe fn on_present() {
    if !COLLECTING.load(Ordering::Relaxed) {
        return;
    }
    let state = &mut *core::ptr::addr_of_mut!(STATE);
    let mut now = 0i64;
    QueryPerformanceCounter(&mut now);
    if state.last != 0 {
        state.ticks.push(now - state.last);
    }
    state.last = now;
}

/// Ends the measurement. Returns the frame times in microseconds, in the order of the frames.
pub unsafe fn stop() -> Vec<f64> {
    COLLECTING.store(false, Ordering::Relaxed);
    let state = &mut *core::ptr::addr_of_mut!(STATE);
    let mut frequency = 1i64;
    QueryPerformanceFrequency(&mut frequency);
    state.ticks.iter().map(|&t| t as f64 * 1_000_000.0 / frequency as f64).collect()
}
