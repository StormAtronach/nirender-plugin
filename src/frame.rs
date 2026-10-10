//! The frame comparison: the device calls of one frame with the engine functions against the
//! calls of the next frame with the Rust functions.
//!
//! It needs a scene that does not change between the frames. The control run records two
//! frames with the engine functions. The live run records three: engine, Rust, engine. When
//! the two engine frames differ the scene changed (an animated texture moved on, for one) and
//! the run says RETRY; a FAIL means the Rust frame differs from two equal engine frames.

use core::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;

use crate::device;
use crate::ports;
use crate::shadow::{self, Mode};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Engine functions in both frames.
    Control,
    /// Engine functions, then Rust functions.
    Live,
}

const IDLE: u8 = 0;
const ARMED: u8 = 1;
const FIRST: u8 = 2;
const SECOND: u8 = 3;
const THIRD: u8 = 4;

static PHASE: AtomicU8 = AtomicU8::new(IDLE);
static KIND_LIVE: AtomicU8 = AtomicU8::new(0);
static RESULT: Mutex<Option<String>> = Mutex::new(None);

struct Work {
    first: Vec<u32>,
    second: Vec<u32>,
    third: Vec<u32>,
    modes: Vec<Mode>,
    live_calls_before: u64,
}
static mut WORK: Work = Work { first: Vec::new(), second: Vec::new(), third: Vec::new(), modes: Vec::new(), live_calls_before: 0 };

/// Starts a comparison at the next Present. False when one is running.
pub fn request(kind: Kind) -> bool {
    if PHASE.load(Ordering::Relaxed) != IDLE {
        return false;
    }
    *RESULT.lock().unwrap() = None;
    KIND_LIVE.store((kind == Kind::Live) as u8, Ordering::Relaxed);
    PHASE.store(ARMED, Ordering::Relaxed);
    true
}

pub fn take_result() -> Option<String> {
    RESULT.lock().unwrap().take()
}

fn total_live_calls() -> u64 {
    ports::all().map(|p| p.live_calls.load(Ordering::Relaxed)).sum()
}

fn set_all(mode: Mode) {
    for port in ports::all() {
        if port.installed() {
            port.set_mode(mode);
        }
    }
}

/// The comparison of two frame logs as one line of text. Starts with PASS or FAIL.
pub fn compare(first: &[u32], second: &[u32], first_name: &str, second_name: &str) -> String {
    let counts = format!(
        "{} {} calls, {} {} calls",
        first_name,
        device::count_records(first),
        second_name,
        device::count_records(second)
    );
    // `first_difference` names its sides for the shadow test: its first log is the Rust one.
    match device::first_difference(second, first) {
        None => format!("PASS {counts}, identical"),
        Some(difference) => format!("FAIL {counts}; {difference}"),
    }
}

/// Runs at each Present of the engine, before the call goes to the device.
pub unsafe fn on_present() {
    let work = &mut *core::ptr::addr_of_mut!(WORK);
    match PHASE.load(Ordering::Relaxed) {
        ARMED => {
            work.modes = ports::all().map(|p| p.mode()).collect();
            shadow::suspend_shadow(true);
            set_all(Mode::Off);
            device::begin(false);
            PHASE.store(FIRST, Ordering::Relaxed);
        }
        FIRST => {
            device::end(&mut work.first);
            if KIND_LIVE.load(Ordering::Relaxed) != 0 {
                set_all(Mode::Live);
            }
            work.live_calls_before = total_live_calls();
            device::begin(false);
            PHASE.store(SECOND, Ordering::Relaxed);
        }
        SECOND => {
            device::end(&mut work.second);
            if KIND_LIVE.load(Ordering::Relaxed) != 0 {
                // A third frame with the engine functions again.
                work.live_calls_before = total_live_calls() - work.live_calls_before;
                set_all(Mode::Off);
                device::begin(false);
                PHASE.store(THIRD, Ordering::Relaxed);
                return;
            }
            finish(work, compare(&work.first, &work.second, "engine", "engine"), "control");
        }
        THIRD => {
            device::end(&mut work.third);
            let ran = work.live_calls_before;
            let mut text = if work.first != work.third {
                format!("RETRY the scene changed between the two engine frames; {}", compare(&work.first, &work.third, "engine", "engine"))
            } else {
                compare(&work.first, &work.second, "engine", "rust")
            };
            text.push_str(&format!("; {ran} calls ran as Rust"));
            if ran == 0 {
                text = format!("FAIL no Rust function ran; {text}");
            }
            finish(work, text, "live");
        }
        _ => {}
    }
}

unsafe fn finish(work: &Work, text: String, kind: &str) {
    for (port, mode) in ports::all().zip(work.modes.iter()) {
        port.set_mode(*mode);
    }
    shadow::suspend_shadow(false);
    crate::log!("frame test ({kind}): {text}");
    *RESULT.lock().unwrap() = Some(text);
    PHASE.store(IDLE, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_reports_pass_and_the_first_difference() {
        let mut a = Vec::new();
        device::append_record(&mut a, 50, &[7, 1], &[]);
        device::append_record(&mut a, 71, &[4, 0, 10, 0, 6], &[]);
        let mut b = a.clone();
        assert!(compare(&a, &b, "engine", "rust").starts_with("PASS engine 2 calls, rust 2 calls"));
        let last = b.len() - 2;
        b[last] = 5; // the primitive count of the draw
        let text = compare(&a, &b, "engine", "rust");
        assert!(text.starts_with("FAIL"), "{text}");
        assert!(text.contains("call 1: rust DrawIndexedPrimitive(0x4, 0x0, 0xa, 0x0, 0x5) | engine DrawIndexedPrimitive(0x4, 0x0, 0xa, 0x0, 0x6)"), "{text}");
    }
}
