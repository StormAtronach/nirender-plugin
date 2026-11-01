//! The watchdog of the frame thread. A step of the frame thread (the feed of one record of
//! the stream, with the work of the frame's end when the record ends a frame) that does not
//! finish in [`STALL_SECONDS`] is a stall: a call of the driver that does not return, a wait
//! that nothing ends. The watchdog thread then says so in the log, writes numidium.dmp with
//! the context of the frame thread and ends the program with the exit code [`EXIT_STALLED`]
//! (`crash::stall_dump_and_end`); the plugin finds the code at the end of its frame and runs
//! the failure path. While the extension loads ([`crate::Extension::loading`]) a step may take
//! [`LOADING_SECONDS`]: a load of the distant land can hold a frame for seconds. The helper
//! writes its allowance of the moment into the ring head ([`allowance`],
//! `stream_format::RING_ALLOWANCE`), so that the plugin's own limit is always later. A step
//! that the helper knows is long (the GPU objects of a load, the pipelines) has the same limit
//! while it lasts ([`LongStep`]): a real hang inside it is still caught, at that limit.
//! `MGE_XE_WATCHDOG=0` leaves the watchdog off, for a test of the plugin's own limit. A
//! refusal disarms it before its box ([`disarm`], `gpu::refuse`): the box waits for the
//! player inside a step, and the program ends on purpose after it.
//!
//! The frame thread does two atomic writes for each step and reads no clock; the watchdog
//! thread looks every [`LOOK_MS`] ms whether a step is in progress and whether the count of
//! the steps moved.

use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use crate::stream_format::{EXIT_STALLED, LOADING_SECONDS, STALL_SECONDS};

/// The time between two looks of the watchdog thread.
const LOOK_MS: u64 = 250;

/// The steps that began, and the ends of a long part inside a step, counted: the watchdog's clock
/// of a step restarts when the count moves. A step in progress; the extension loads.
static STEPS: AtomicU64 = AtomicU64::new(0);
static IN_STEP: AtomicBool = AtomicBool::new(false);
static LOADING: AtomicBool = AtomicBool::new(false);
/// The long steps in progress ([`LongStep`]).
static LONG_STEPS: AtomicU32 = AtomicU32::new(0);
/// The word `RING_ALLOWANCE` of the ring head, or null without a ring (a replay, the probe). The
/// head stays mapped for the process, as for `live::ENDING_WORD`.
static ALLOWANCE_WORD: AtomicPtr<AtomicU32> = AtomicPtr::new(std::ptr::null_mut());
/// The frames started: until then the word stays 0, and the plugin gives the helper its first
/// limit (`stream_format::FIRST_RECORD_SECONDS`).
static FRAMES_STARTED: AtomicBool = AtomicBool::new(false);
/// The word holds a long value that [`allowance`] no longer gives: the next step lowers it.
static LOWER_WORD: AtomicBool = AtomicBool::new(false);
/// A refusal is in progress: the watchdog never ends the program after it.
static DISARMED: AtomicBool = AtomicBool::new(false);

/// Disarms the watchdog for good: the program ends on purpose (`gpu::refuse`).
pub(crate) fn disarm() {
    DISARMED.store(true, Ordering::Release);
}

/// A step of the frame thread: it ends when the guard drops.
pub(crate) struct Step;

impl Step {
    pub(crate) fn begin() -> Step {
        STEPS.fetch_add(1, Ordering::Relaxed);
        IN_STEP.store(true, Ordering::Release);
        // The read position moved for this step's record: the plugin's silence starts again.
        if LOWER_WORD.load(Ordering::Relaxed) {
            LOWER_WORD.store(false, Ordering::Relaxed);
            write_word(allowance());
        }
        Step
    }
}

impl Drop for Step {
    fn drop(&mut self) {
        IN_STEP.store(false, Ordering::Release);
    }
}

/// Tells the watchdog whether the extension loads, from the frame thread. The end of a load
/// restarts the clock of the step: the time of the load does not count against the short limit.
pub(crate) fn loading(on: bool) {
    if LOADING.swap(on, Ordering::Relaxed) && !on {
        STEPS.fetch_add(1, Ordering::Relaxed);
    }
}

/// A step of the frame thread has the long limit: the extension loads, or a long step runs.
fn long_limit() -> bool {
    LOADING.load(Ordering::Relaxed) || LONG_STEPS.load(Ordering::Relaxed) > 0
}

/// The seconds a step of the frame thread may take now: the word that the helper writes into
/// the ring head for the plugin.
pub(crate) fn allowance() -> u32 {
    if long_limit() { LOADING_SECONDS as u32 } else { STALL_SECONDS as u32 }
}

/// The ring is open: `word` is its word `RING_ALLOWANCE`, mapped for the process.
pub(crate) fn set_allowance_word(word: *mut AtomicU32) {
    ALLOWANCE_WORD.store(word, Ordering::Release);
}

/// The frames start: from now the plugin gives a step the allowance and 5 s more.
pub(crate) fn frames_start() {
    FRAMES_STARTED.store(true, Ordering::Relaxed);
    publish();
}

/// Writes `seconds` into the word, once the frames started and when the ring is open; gives the
/// value the word had.
fn write_word(seconds: u32) -> u32 {
    let word = ALLOWANCE_WORD.load(Ordering::Acquire);
    if word.is_null() || !FRAMES_STARTED.load(Ordering::Relaxed) {
        return 0;
    }
    // SAFETY: a word of the ring head, 4-aligned and mapped for the process, or a null pointer
    // (`set_allowance_word`), tested above.
    unsafe { (*word).swap(seconds, Ordering::AcqRel) }
}

/// Writes [`allowance`] into the word for the plugin: the one writer of the word, so that a
/// write at a frame's end inside a long step cannot put the short value back. A rise is written
/// now. A fall waits for the next step ([`Step::begin`]): the plugin counts its silence from
/// the last read, and the long part is in that silence, so the short limit may come only after
/// the next read. From the frame thread only: nothing orders the reading and the writing of two
/// threads.
pub(crate) fn publish() {
    let now = allowance();
    let word = ALLOWANCE_WORD.load(Ordering::Acquire);
    // SAFETY: as in `write_word`; the word is read only when it is not null.
    if !word.is_null() && now < unsafe { (*word).load(Ordering::Acquire) } {
        LOWER_WORD.store(true, Ordering::Relaxed);
    } else {
        LOWER_WORD.store(false, Ordering::Relaxed);
        write_word(now);
    }
}

/// A step that the helper knows is long: the GPU objects of a load, the making of pipelines. The
/// watchdog gives the long limit until the last guard drops, then restarts the step's clock, so
/// that the time inside the guard does not count against the short limit. The plugin has the long
/// limit until the helper's next read ([`publish`]). Guards can nest. Make it the first statement
/// of the long part, on the frame thread: a raise at the start of a step reaches the plugin at
/// its next look, inside the short limit.
pub struct LongStep(PhantomData<*const ()>);

impl LongStep {
    pub fn begin() -> LongStep {
        LONG_STEPS.fetch_add(1, Ordering::Relaxed);
        publish();
        LongStep(PhantomData)
    }
}

impl Drop for LongStep {
    fn drop(&mut self) {
        if LONG_STEPS.fetch_sub(1, Ordering::Relaxed) == 1 {
            STEPS.fetch_add(1, Ordering::Relaxed);
        }
        publish();
    }
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
            while !DISARMED.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(LOOK_MS));
                let look = Look { steps: STEPS.load(Ordering::Relaxed), in_step: IN_STEP.load(Ordering::Acquire), loading: long_limit() };
                // A refusal that came during the look ends the program itself.
                match watch.look(look) {
                    Some(seconds) if !DISARMED.load(Ordering::Acquire) => stalled(frame_thread, seconds),
                    _ => {}
                }
            }
        })
        .expect("the watchdog thread");
}

/// The frame thread stalled: the exit announced to the plugin, the log line, then the dump
/// and the end of the program.
/// The line is printed before the frame thread is suspended: a thread suspended inside a
/// print or an allocation holds its lock, and a print after it would wait for ever.
fn stalled(frame_thread: u32, seconds: u64) -> ! {
    use std::io::Write;
    // First the exit for the plugin: a dump that hangs is then ended with the code 30, not 31.
    crate::live::announce_exit(EXIT_STALLED);
    println!("stalled for {seconds} s: the frame thread made no progress in a step; a minidump, then the program ends with the exit code {EXIT_STALLED}");
    let _ = std::io::stdout().flush();
    crate::crash::stall_dump_and_end(frame_thread, EXIT_STALLED)
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

    /// The one test that uses the statics of the long steps: tests run at the same time.
    #[test]
    fn a_long_step_raises_the_allowance_and_the_word_until_the_last_guard_drops() {
        let word: &'static AtomicU32 = Box::leak(Box::new(AtomicU32::new(0)));
        let read = || word.load(Ordering::Acquire);
        let (short, long) = (STALL_SECONDS as u32, LOADING_SECONDS as u32);
        // No ring: the guard changes the watchdog only, and nothing fails.
        {
            let _step = LongStep::begin();
            assert_eq!(allowance(), long);
        }
        assert_eq!(allowance(), short);
        set_allowance_word(word as *const AtomicU32 as *mut AtomicU32);
        // Before the frames the word stays 0: the plugin's first limit stands.
        {
            let _step = LongStep::begin();
            assert_eq!((allowance(), read()), (long, 0));
        }
        assert_eq!(read(), 0);
        frames_start();
        assert_eq!(read(), short);
        // One guard, and two nested: the last drop puts both back.
        let outer = LongStep::begin();
        assert_eq!((allowance(), read()), (long, long));
        let inner = LongStep::begin();
        assert_eq!((allowance(), read()), (long, long));
        drop(inner);
        assert_eq!((allowance(), read()), (long, long));
        // A write at a frame's end inside the guard cannot put the short value back.
        publish();
        assert_eq!(read(), long);
        // The last drop: the limit of the watchdog is short, and the clock of the step restarts
        // (one more step counted; the inner drop above counted none). The word falls only at the
        // next step, after the read position moved.
        let steps = STEPS.load(Ordering::Relaxed);
        drop(outer);
        assert_eq!((allowance(), read()), (short, long));
        assert_eq!(STEPS.load(Ordering::Relaxed), steps + 1);
        // A frame's end before that next read leaves it long.
        publish();
        assert_eq!(read(), long);
        drop(Step::begin());
        assert_eq!(read(), short);
        // A second step writes nothing more, and a rise after a pending fall is written at once.
        drop(Step::begin());
        assert_eq!(read(), short);
        drop(LongStep::begin());
        assert_eq!(read(), long);
        let again = LongStep::begin();
        assert_eq!(read(), long);
        // The next step inside a guard keeps the long value.
        drop(Step::begin());
        assert_eq!(read(), long);
        drop(again);
        drop(Step::begin());
        assert_eq!(read(), short);
        // The end of a guard while the extension loads keeps the long value and the clock of
        // the load; the end of the load restarts the clock, and the word falls at the next step.
        loading(true);
        publish();
        let steps = STEPS.load(Ordering::Relaxed);
        assert_eq!(read(), long);
        drop(LongStep::begin());
        assert_eq!((allowance(), read()), (long, long));
        assert_eq!(STEPS.load(Ordering::Relaxed), steps + 1);
        let steps = STEPS.load(Ordering::Relaxed);
        loading(false);
        publish();
        assert_eq!((allowance(), read()), (short, long));
        assert_eq!(STEPS.load(Ordering::Relaxed), steps + 1);
        drop(Step::begin());
        assert_eq!(read(), short);
        set_allowance_word(std::ptr::null_mut());
        FRAMES_STARTED.store(false, Ordering::Relaxed);
        LOWER_WORD.store(false, Ordering::Relaxed);
    }

    /// The Reviewer's sequence: a long part of 6 s in a step, then a look after its end. The count
    /// that moved at the drop restarts the clock, so this is no stall, and the short limit
    /// counts from the drop.
    #[test]
    fn the_time_inside_a_long_part_does_not_count_after_it() {
        let mut watch = Watch::default();
        let inside = Look { steps: 1, in_step: true, loading: true };
        assert_eq!(looks(&mut watch, 25, inside), None);
        let after = Look { steps: 2, in_step: true, loading: false };
        assert_eq!(looks(&mut watch, 20, after), None);
        // A real hang after the long part: the 21st look after the drop reaches the 5 s.
        assert_eq!(watch.look(after), Some(5));
    }
}
