//! The watchdog of the frame thread. A step of the frame thread (the feed of one record of
//! the stream, with the work of the frame's end when the record ends a frame) that does not
//! finish in [`STALL_SECONDS`] is a stall: a call of the driver that does not return, a wait
//! that nothing ends. The watchdog thread then writes numidium.dmp with the context of the
//! frame thread (`crash::stall_dump`), says so in the log and ends the program with the exit
//! code [`EXIT_STALLED`]; the plugin finds the code at the end of its frame and runs the
//! failure path. While the extension loads ([`crate::Extension::loading`]) a step may take
//! [`LOADING_SECONDS`]: a load of the distant land can hold a frame for seconds.
//! `MGE_XE_WATCHDOG=0` leaves the watchdog off, for a test of the plugin's own limit.
//!
//! The frame thread does two atomic writes for each step and reads no clock; the watchdog
//! thread looks every [`LOOK_MS`] ms whether a step is in progress and whether the count of
//! the steps moved.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use crate::stream_format::{EXIT_STALLED, STALL_SECONDS};

/// The seconds a step may take while the extension loads.
pub const LOADING_SECONDS: u64 = 30;
/// The time between two looks of the watchdog thread.
const LOOK_MS: u64 = 250;

/// The steps that began, counted; a step in progress; the extension loads.
static STEPS: AtomicU64 = AtomicU64::new(0);
static IN_STEP: AtomicBool = AtomicBool::new(false);
static LOADING: AtomicBool = AtomicBool::new(false);

/// A step of the frame thread: it ends when the guard drops.
pub(crate) struct Step;

impl Step {
    pub(crate) fn begin() -> Step {
        STEPS.fetch_add(1, Ordering::Relaxed);
        IN_STEP.store(true, Ordering::Release);
        Step
    }
}

impl Drop for Step {
    fn drop(&mut self) {
        IN_STEP.store(false, Ordering::Release);
    }
}

/// Tells the watchdog whether the extension loads, from the frame thread.
pub(crate) fn loading(on: bool) {
    LOADING.store(on, Ordering::Relaxed);
}

/// What the watchdog saw at one look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Look {
    steps: u64,
    in_step: bool,
    loading: bool,
}

/// The time the same step has been in progress, over the looks: 0 when no step is in
/// progress or the count moved.
#[derive(Default)]
struct Watch {
    last_steps: u64,
    stalled_ms: u64,
}

impl Watch {
    /// The seconds of the stall when the limit is passed, else `None`.
    fn look(&mut self, look: Look) -> Option<u64> {
        if !look.in_step || look.steps != self.last_steps {
            self.last_steps = look.steps;
            self.stalled_ms = 0;
            return None;
        }
        self.stalled_ms += LOOK_MS;
        let limit = if look.loading { LOADING_SECONDS } else { STALL_SECONDS };
        (self.stalled_ms >= limit * 1000).then_some(self.stalled_ms / 1000)
    }
}

/// Starts the watchdog thread for the frame thread, which is the caller.
pub(crate) fn start() {
    // SAFETY: a plain read of the id of this thread.
    let frame_thread = unsafe { GetCurrentThreadId() };
    std::thread::Builder::new()
        .name("watchdog".into())
        .spawn(move || {
            let mut watch = Watch::default();
            loop {
                std::thread::sleep(Duration::from_millis(LOOK_MS));
                let look = Look { steps: STEPS.load(Ordering::Relaxed), in_step: IN_STEP.load(Ordering::Acquire), loading: LOADING.load(Ordering::Relaxed) };
                if let Some(seconds) = watch.look(look) {
                    stalled(frame_thread, seconds);
                }
            }
        })
        .expect("the watchdog thread");
}

/// The frame thread stalled: the dump, the log line, the end of the program.
fn stalled(frame_thread: u32, seconds: u64) -> ! {
    use std::io::Write;
    println!("stalled for {seconds} s: the frame thread made no progress in a step; the program ends with the exit code {EXIT_STALLED}");
    match crate::crash::stall_dump(frame_thread) {
        Ok(path) => println!("minidump: {}", path.display()),
        Err(problem) => println!("no minidump: {problem}"),
    }
    let _ = std::io::stdout().flush();
    std::process::exit(EXIT_STALLED as i32)
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentThreadId() -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn looks(watch: &mut Watch, count: usize, look: Look) -> Option<u64> {
        (0..count).map(|_| watch.look(look)).last().flatten()
    }

    #[test]
    fn a_step_that_stays_for_the_limit_is_a_stall() {
        let mut watch = Watch::default();
        let stuck = Look { steps: 7, in_step: true, loading: false };
        // The first look sees the step; the next 19 looks are under 5 s; the 21st reaches it.
        assert_eq!(looks(&mut watch, 20, stuck), None);
        assert_eq!(watch.look(stuck), Some(5));
    }

    #[test]
    fn steps_that_move_or_end_are_no_stall() {
        let mut watch = Watch::default();
        for steps in 0..100 {
            assert_eq!(watch.look(Look { steps, in_step: true, loading: false }), None);
        }
        // Between the steps nothing is in progress: a long wait for the game is no stall.
        assert_eq!(looks(&mut watch, 100, Look { steps: 100, in_step: false, loading: false }), None);
    }

    #[test]
    fn a_load_in_progress_gives_the_step_30_s() {
        let mut watch = Watch::default();
        let loading = Look { steps: 1, in_step: true, loading: true };
        assert_eq!(looks(&mut watch, 120, loading), None);
        assert_eq!(watch.look(loading), Some(30));
    }
}
